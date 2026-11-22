//! Export: no Qt.
//!
//! * [`export_ssh_config`] — the SSH entries as an OpenSSH client config (to `Include` from
//!   `~/.ssh/config`). Passwords are never written; `import::ssh_config` reads the result back.
//! * [`build_backup_zip`] — the config files in one deflate zip with an optional `README.txt`.
//!
//! Text is byte-identical to the golden fixtures (`export_alias.json`, `export_config.json`).
//! Limits: a list/map value prints as `<container>` (config values are
//! scalars or lists of strings in practice). The backup zip is created
//! with mode 0600 from the start instead of being `chmod`ed afterwards.

use crate::platform;
use std::collections::HashSet;
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::Path;

use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipWriter};

use crate::config::doc::Value;
use crate::config::model::{proxy_jump_value, strip_space, truthy, value_str};
use crate::proto::ssh::forward_specs_from_yaml;
use crate::shell;

/// Local wall-clock time for the header comment, injected.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Stamp {
    pub year: i32,
    pub month: u32,
    pub day: u32,
    pub hour: u32,
    pub minute: u32,
}

/// Result of [`export_ssh_config`]; `skipped`: other protocols and entries without a host.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SshExport {
    pub text: String,
    pub exported: usize,
    pub skipped: usize,
}

/// ssh_config keywords Termigator manages itself — not copied from extra args into the file.
const SKIP_OPTS: [&str; 3] = ["controlmaster", "controlpath", "controlpersist"];

fn is_space(c: char) -> bool {
    c.is_whitespace() || ('\x1c'..='\x1f').contains(&c)
}

/// A `Host` alias from an entry label: no spaces / wildcards, unique (case-insensitively).
pub fn ssh_alias(label: &str, taken: &mut HashSet<String>) -> String {
    let mut base = String::new();
    let mut in_bad = false;
    for c in strip_space(label).chars() {
        if c.is_ascii_alphanumeric() || "._@+-".contains(c) {
            base.push(c);
            in_bad = false;
        } else if !in_bad {
            base.push('-');
            in_bad = true;
        }
    }
    let mut base = base.trim_matches(['-', '.']).to_owned();
    if base.is_empty() {
        base = "host".to_owned();
    }
    let mut alias = base.clone();
    let mut n = 2;
    while taken.contains(&alias.to_lowercase()) {
        alias = format!("{base}-{n}");
        n += 1;
    }
    taken.insert(alias.to_lowercase());
    alias
}

/// A value is one line of the file; a line break would start a new directive.
fn one_line(value: &str) -> String {
    value
        .chars()
        .map(|c| if matches!(c, '\n' | '\r') { ' ' } else { c })
        .collect()
}

fn quote(value: &str) -> String {
    if value.chars().any(is_space) {
        format!("\"{value}\"")
    } else {
        value.to_owned()
    }
}

/// `str(v)` for a present, non-null value.
fn text(v: Option<&Value>) -> Option<String> {
    v.filter(|v| !matches!(v, Value::Null)).map(value_str)
}

fn tr(v: Option<&Value>) -> bool {
    v.is_some_and(truthy)
}

fn is_true(v: Option<&Value>) -> bool {
    matches!(v, Some(Value::Bool(true)))
}

type Opts = Vec<(String, String)>;

/// `-o Key=Value` / `-p`, `-i`, `-J`, `-C`, `-A` from extra_ssh_args as config lines; flags
/// with no config form are returned to be listed in a comment.
fn extra_options(conn: &Value) -> (Opts, Vec<String>) {
    let raw = conn.get("extra_ssh_args");
    let args: Vec<String> = match raw {
        Some(Value::Seq(items)) => items.iter().map(value_str).collect(),
        _ => {
            let s = if tr(raw) {
                raw.map(value_str).unwrap_or_default()
            } else {
                String::new()
            };
            match shell::split(&s) {
                Ok(a) => a,
                Err(_) => return (Vec::new(), vec![s]),
            }
        }
    };
    let mut out = Vec::new();
    let mut left = Vec::new();
    let mut i = 0;
    while i < args.len() {
        let a = args[i].as_str();
        let nxt = args.get(i + 1).map(String::as_str);
        let glued = a.starts_with("-o") && a.chars().count() > 2;
        if (a == "-o" && nxt.is_some()) || glued {
            let opt = if a == "-o" {
                nxt.unwrap_or("")
            } else {
                &a[2..]
            };
            i += if a == "-o" { 2 } else { 1 };
            let (key, val) = match opt.split_once('=') {
                Some(kv) => kv,
                None => opt.split_once(' ').unwrap_or((opt, "")),
            };
            let key = strip_space(key);
            if !SKIP_OPTS.contains(&key.to_lowercase().as_str()) {
                out.push((key.to_owned(), strip_space(val).to_owned()));
            }
            continue;
        }
        let simple = match a {
            "-p" => Some("Port"),
            "-i" => Some("IdentityFile"),
            "-J" => Some("ProxyJump"),
            _ => None,
        };
        if let (Some(key), Some(n)) = (simple, nxt) {
            out.push((key.to_owned(), n.to_owned()));
            i += 2;
            continue;
        }
        match a {
            "-C" => out.push(("Compression".to_owned(), "yes".to_owned())),
            "-A" => out.push(("ForwardAgent".to_owned(), "yes".to_owned())),
            _ => left.push(a.to_owned()),
        }
        i += 1;
    }
    (out, left)
}

/// `-L` spec `[bind:]port:host:hostport` → `[bind:]port host:hostport`.
fn forward_to_config(spec: &str) -> String {
    let parts: Vec<&str> = spec.rsplitn(3, ':').collect();
    match parts[..] {
        [hostport, host, port] => format!("{port} {host}:{hostport}"),
        _ => spec.to_owned(),
    }
}

fn host_block(conn: &Value, alias: &str) -> Vec<String> {
    let mut lines = vec![format!("Host {alias}")];
    // words=true: the value is several arguments (LocalForward 8080 host:80) — quoting it
    // whole would make it one, which ssh rejects.
    let put = |lines: &mut Vec<String>, key: &str, value: Option<String>, words: bool| {
        if let Some(v) = value {
            let v = one_line(&v);
            let v = strip_space(&v);
            if !v.is_empty() {
                let v = if words { v.to_owned() } else { quote(v) };
                lines.push(format!("    {key} {v}"));
            }
        }
    };
    let get = |k: &str| conn.get(k);

    put(&mut lines, "HostName", text(get("host")), false);
    put(&mut lines, "User", text(get("user")), false);
    let default_port = match get("port") {
        None | Some(Value::Null) => true,
        Some(Value::Str(s)) => s.is_empty() || s == "22",
        Some(Value::Int(22)) => true,
        Some(Value::Float(f)) => *f == 22.0,
        _ => false,
    };
    if !default_port {
        put(&mut lines, "Port", text(get("port")), false);
    }
    if tr(get("identity_file")) {
        put(
            &mut lines,
            "IdentityFile",
            text(get("identity_file")),
            false,
        );
        put(&mut lines, "IdentitiesOnly", Some("yes".to_owned()), false);
    }
    let pj = match proxy_jump_value(conn) {
        Some(Value::Seq(items)) => Some(items.iter().map(value_str).collect::<Vec<_>>().join(",")),
        other => text(other),
    };
    put(&mut lines, "ProxyJump", pj, false);
    for spec in forward_specs_from_yaml(get("local_forwards")) {
        put(
            &mut lines,
            "LocalForward",
            Some(forward_to_config(&spec)),
            true,
        );
    }
    for spec in forward_specs_from_yaml(get("remote_forwards")) {
        put(
            &mut lines,
            "RemoteForward",
            Some(forward_to_config(&spec)),
            true,
        );
    }
    let df = get("dynamic_forward");
    let df_off = match df {
        None | Some(Value::Null | Value::Bool(false)) => true,
        Some(Value::Str(s)) => s.is_empty(),
        Some(Value::Int(0)) => true,
        Some(Value::Float(f)) => *f == 0.0,
        _ => false,
    };
    if !df_off {
        let s = text(df).unwrap_or_default();
        if !["none", "off", "no", "false"].contains(&s.to_lowercase().as_str()) {
            put(&mut lines, "DynamicForward", Some(s), false);
        }
    }
    let yes = || Some("yes".to_owned());
    if is_true(get("forward_agent")) {
        put(&mut lines, "ForwardAgent", yes(), false);
    }
    if is_true(get("x11_forwarding")) {
        put(&mut lines, "ForwardX11", yes(), false);
        if is_true(get("x11_trusted")) {
            put(&mut lines, "ForwardX11Trusted", yes(), false);
        }
    }
    if is_true(get("compression")) {
        put(&mut lines, "Compression", yes(), false);
    }
    if is_true(get("request_tty")) || tr(get("remote_command")) {
        put(&mut lines, "RequestTTY", yes(), false);
    }
    put(
        &mut lines,
        "RemoteCommand",
        text(get("remote_command")),
        false,
    );
    put(
        &mut lines,
        "StrictHostKeyChecking",
        text(get("strict_host_key_checking")),
        false,
    );
    if tr(get("remote_locale")) {
        let l = text(get("remote_locale")).unwrap_or_default();
        put(
            &mut lines,
            "SetEnv",
            Some(format!("LANG={l} LC_ALL={l}")),
            true,
        );
    }
    let (opts, left) = extra_options(conn);
    let have: HashSet<String> = lines[1..]
        .iter()
        .filter_map(|l| l.split_whitespace().next())
        .map(str::to_lowercase)
        .collect();
    for (key, val) in opts {
        if !have.contains(&key.to_lowercase()) {
            put(&mut lines, &key, Some(val), false);
        }
    }
    if !left.is_empty() {
        lines.push(format!(
            "    # not expressible here, from extra ssh arguments: {}",
            one_line(&left.join(" "))
        ));
    }
    lines
}

/// The SSH entries as ssh_config text. Mosh / file-only flags and passwords have no
/// ssh_config form and are left out.
pub fn export_ssh_config(conns: &[Value], now: Stamp) -> SshExport {
    let mut out = vec![
        format!(
            "# Termigator export, {:04}-{:02}-{:02} {:02}:{:02} — SSH entries as an OpenSSH client config.",
            now.year, now.month, now.day, now.hour, now.minute
        ),
        format!(
            "# Use it from {} with:  Include <path of this file>",
            if cfg!(windows) {
                "%USERPROFILE%\\.ssh\\config"
            } else {
                "~/.ssh/config"
            }
        ),
        "# Saved passwords are never exported.".to_owned(),
        String::new(),
    ];
    let mut taken = HashSet::new();
    let (mut exported, mut skipped) = (0, 0);
    for c in conns {
        if !matches!(c, Value::Map(_)) {
            continue;
        }
        let proto = if tr(c.get("protocol")) {
            text(c.get("protocol")).unwrap_or_default().to_lowercase()
        } else {
            "ssh".to_owned()
        };
        let host = if tr(c.get("host")) {
            text(c.get("host")).unwrap_or_default()
        } else {
            String::new()
        };
        if proto != "ssh" || strip_space(&host).is_empty() {
            skipped += 1;
            continue;
        }
        let label = if tr(c.get("label")) {
            c.get("label")
        } else {
            c.get("host")
        };
        let alias = ssh_alias(&text(label).unwrap_or_default(), &mut taken);
        let grp = if tr(c.get("group")) {
            text(c.get("group")).unwrap_or_default()
        } else {
            String::new()
        };
        let grp = one_line(&grp);
        let grp = strip_space(&grp);
        if !grp.is_empty() {
            out.push(format!("# group: {grp}"));
        }
        out.extend(host_block(c, &alias));
        out.push(String::new());
        exported += 1;
    }
    SshExport {
        text: out.join("\n"),
        exported,
        skipped,
    }
}

/// Zip `files` (`name in the zip → path on disk`; missing ones are skipped) into `target`
/// (created with mode 0600). Returns the names written.
pub fn build_backup_zip(
    target: &Path,
    files: &[(&str, &Path)],
    note: &str,
) -> io::Result<Vec<String>> {
    let file = platform::private_file(OpenOptions::new().write(true).create(true).truncate(true))
        .open(target)?;
    // An existing target keeps its old mode on open; the backup may hold passwords.
    platform::set_private_file(target)?;
    let mut zw = ZipWriter::new(file);
    let opts = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
    let mut written = Vec::new();
    for (name, path) in files {
        if path.is_file() {
            zw.start_file(*name, opts)?;
            zw.write_all(&fs::read(path)?)?;
            written.push((*name).to_owned());
        }
    }
    if !note.is_empty() {
        zw.start_file("README.txt", opts)?;
        zw.write_all(note.as_bytes())?;
        written.push("README.txt".to_owned());
    }
    zw.finish()?;
    Ok(written)
}
