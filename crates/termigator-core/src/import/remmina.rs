//! Remmina profile import (`*.remmina`).
//!
//! A profile is an INI file with one `[remmina]` section. Mapped: `name`, `group` (`a/b`
//! nests; default `Remmina`), `server` (`host[:port]`), `username`, and per protocol
//!
//! * **SSH / SFTP**: `ssh_privatekey` (an existing file only), `ssh_compression`,
//!   `ssh_forward_x11`, `ssh_stricthostkeycheck`; an SSH tunnel becomes `proxy_jump`.
//!   SFTP profiles open as a file browser (`files_only`).
//! * **RDP**: `domain`, `sharefolder`, `sound=local`, `multimon`, `disableclipboard` and
//!   the RD Gateway (`gateway_server` / `_username` / `_domain`).
//! * **VNC**: `server=host:N` with N < 100 is a display number (port 5900 + N).
//!
//! An SSH tunnel on an RDP / VNC profile maps to `ssh_gateway` only when an SSH entry for
//! that tunnel host already exists (the gateway is chosen by entry name); otherwise a note
//! is returned. Passwords are never imported. Other protocols are counted in `skipped`.
//!
//! The INI reader (no interpolation, duplicate keys allowed) accepts `=`/`:` delimiters,
//! `#`/`;` full-line comments, indented continuation lines, `[DEFAULT]` inherited by every section
//! and lower-cased option names; any parse error makes the file `unreadable`, as does a missing
//! `[remmina]` section.
//!
//! Limits, all on hostile input only: a file over
//! [`MAX_FILE_BYTES`](super::putty::MAX_FILE_BYTES) is unreadable; at most
//! [`MAX_PROFILES`] files are read (a note says so); a port made of non-ASCII digits or
//! larger than `i64` counts as absent; `~user`
//! in `ssh_privatekey` is not expanded.
//!
//! Golden: `compat/golden/remmina.json`, tests in
//! `tests/import_remmina.rs`.

use crate::platform;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use super::moba::{row, sv};
use super::putty::read_limited;
use crate::config::doc::Value;
use crate::config::model::{strip_space, truthy, value_str};
use crate::proto::ssh::expand_user;

/// Group of profiles that have none.
pub const DEFAULT_GROUP: &str = "Remmina";
/// First display-number port of VNC.
pub const VNC_DEFAULT_PORT: i64 = 5900;
/// Profiles read per call; the rest are reported in `notes`.
pub const MAX_PROFILES: usize = 10_000;

/// Result of an import run.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RemminaImport {
    /// Entries (maps) in file-name order.
    pub connections: Vec<Value>,
    /// Unsupported protocol (upper case) -> number of profiles, in first-seen order.
    pub skipped: Vec<(String, usize)>,
    /// User-facing remarks (skipped profiles, tunnels that need a gateway entry).
    pub notes: Vec<String>,
    /// Files that could not be read or parsed.
    pub unreadable: Vec<String>,
}

/// Where Remmina keeps profiles: `$XDG_DATA_HOME/remmina` (default `~/.local/share`), then
/// the old `~/.remmina`; only existing directories.
pub fn remmina_dirs(xdg_data_home: Option<&str>, home: &Path) -> Vec<PathBuf> {
    let data = match xdg_data_home.filter(|s| !s.is_empty()) {
        Some(d) => PathBuf::from(d),
        None => home.join(".local").join("share"),
    };
    [data.join("remmina"), home.join(".remmina")]
        .into_iter()
        .filter(|d| d.is_dir())
        .collect()
}

fn is_space(c: char) -> bool {
    c.is_whitespace() || ('\x1c'..='\x1f').contains(&c)
}

type Options = HashMap<String, Vec<String>>;

/// The options one profile can see: `[remmina]` over `[DEFAULT]`.
struct Section {
    own: HashMap<String, String>,
    defaults: HashMap<String, String>,
}

impl Section {
    /// `str(sec.get(key, ""))`.
    fn get(&self, key: &str) -> &str {
        self.own
            .get(key)
            .or_else(|| self.defaults.get(key))
            .map_or("", String::as_str)
    }
}

fn joined(opts: Options) -> HashMap<String, String> {
    opts.into_iter()
        .map(|(k, lines)| (k, lines.join("\n").trim_end_matches(is_space).to_owned()))
        .collect()
}

#[derive(Clone, Copy, PartialEq)]
enum Where {
    Nowhere,
    Default,
    Remmina,
    Other,
}

/// `configparser` read: `Some(section)` when the text parses and has `[remmina]`.
fn parse_ini(text: &str) -> Option<Section> {
    let text = text.replace("\r\n", "\n").replace('\r', "\n");
    let mut lines: Vec<&str> = text.split('\n').collect();
    if lines.last() == Some(&"") {
        lines.pop();
    }
    let mut defaults = Options::new();
    let mut remmina: Option<Options> = None;
    let mut other = Options::new();
    let mut at = Where::Nowhere;
    let mut optname = String::new();
    let mut indent_level = 0usize;
    for line in lines {
        let stripped = strip_space(line);
        if stripped.starts_with(['#', ';']) {
            continue;
        }
        let cur = match at {
            Where::Nowhere => None,
            Where::Default => Some(&mut defaults),
            Where::Remmina => remmina.as_mut(),
            Where::Other => Some(&mut other),
        };
        if stripped.is_empty() {
            if let Some(cur) = cur
                && !optname.is_empty()
                && let Some(v) = cur.get_mut(&optname)
            {
                v.push(String::new());
            }
            continue;
        }
        let indent = line.chars().take_while(|c| is_space(*c)).count();
        if let Some(cur) = cur
            && !optname.is_empty()
            && indent > indent_level
        {
            if let Some(v) = cur.get_mut(&optname) {
                v.push(stripped.to_owned());
            }
            continue;
        }
        indent_level = indent;
        // SECTCRE `\[(?P<header>.+)\]` matched at the start: the header runs to the last `]`.
        let header = stripped
            .strip_prefix('[')
            .and_then(|r| r.rfind(']').map(|i| &r[..i]))
            .filter(|h| !h.is_empty());
        if let Some(name) = header {
            at = match name {
                "remmina" => {
                    remmina.get_or_insert_with(Options::new);
                    Where::Remmina
                }
                "DEFAULT" => Where::Default,
                _ => {
                    other.clear();
                    Where::Other
                }
            };
            optname.clear();
            continue;
        }
        let cur = match at {
            Where::Nowhere => return None,
            Where::Default => &mut defaults,
            Where::Remmina => remmina.get_or_insert_with(Options::new),
            Where::Other => &mut other,
        };
        let (name, val) = stripped.split_once(['=', ':'])?;
        let name = name.trim_end_matches(is_space);
        if name.is_empty() {
            return None;
        }
        optname = name.to_lowercase();
        cur.insert(optname.clone(), vec![strip_space(val).to_owned()]);
    }
    Some(Section {
        own: joined(remmina?),
        defaults: joined(defaults),
    })
}

/// `host[:port]` / `[v6]:port`; the port only when it is plain ASCII digits.
fn split_server(server: &str) -> (String, Option<i64>) {
    let s = strip_space(server);
    let (host, port) = if let Some(r) = s.strip_prefix('[') {
        let (h, rest) = r.split_once(']').unwrap_or((r, ""));
        (h, rest.strip_prefix(':').unwrap_or(""))
    } else if s.matches(':').count() == 1 {
        s.split_once(':').unwrap_or((s, ""))
    } else {
        (s, "")
    };
    let port = if !port.is_empty() && port.bytes().all(|b| b.is_ascii_digit()) {
        port.parse().ok()
    } else {
        None
    };
    (strip_space(host).to_owned(), port)
}

fn on(sec: &Section, key: &str) -> bool {
    strip_space(sec.get(key)) == "1"
}

/// `(user, host[:port])` of an enabled SSH tunnel.
fn tunnel(sec: &Section) -> Option<(String, String)> {
    if !on(sec, "ssh_tunnel_enabled") {
        return None;
    }
    let server = strip_space(sec.get("ssh_tunnel_server"));
    (!server.is_empty()).then(|| {
        (
            strip_space(sec.get("ssh_tunnel_username")).to_owned(),
            server.to_owned(),
        )
    })
}

/// `str(v or "")` of a mapping field.
fn text_of(c: &Value, key: &str) -> String {
    c.get(key)
        .filter(|v| truthy(v))
        .map(value_str)
        .unwrap_or_default()
}

/// The existing SSH entry that is the tunnel's host (and user, when given).
fn find_gateway<'a>(existing: &'a [Value], user: &str, host: &str) -> Option<&'a Value> {
    existing.iter().find(|c| {
        matches!(c, Value::Map(_))
            && c.get("protocol")
                .filter(|v| truthy(v))
                .map_or_else(|| "ssh".to_owned(), value_str)
                .to_lowercase()
                == "ssh"
            && strip_space(&text_of(c, "host")) == host
            && (user.is_empty() || strip_space(&text_of(c, "user")) == user)
    })
}

type Fields = Vec<(&'static str, Value)>;

/// One profile -> (entry, protocol name); the entry is `None` for unsupported protocols.
fn profile(
    sec: &Section,
    existing: &[Value],
    notes: &mut Vec<String>,
    home: &str,
) -> (Option<Value>, String) {
    let proto = strip_space(sec.get("protocol")).to_uppercase();
    let (host, port) = split_server(sec.get("server"));
    let label = match strip_space(sec.get("name")) {
        "" => host.clone(),
        n => n.to_owned(),
    };
    if host.is_empty() {
        notes.push(format!(
            "\u{201c}{label}\u{201d}: no server \u{2014} skipped."
        ));
        return (None, String::new());
    }
    let group = sec
        .get("group")
        .split('/')
        .map(strip_space)
        .filter(|p| !p.is_empty())
        .collect::<Vec<_>>()
        .join("/");
    let group = if group.is_empty() {
        DEFAULT_GROUP.to_owned()
    } else {
        group
    };
    let user = strip_space(sec.get("username"));
    let mut r: Fields = vec![
        ("label", sv(label.clone())),
        ("host", sv(host)),
        ("group", sv(group)),
    ];
    if !user.is_empty() {
        r.push(("user", sv(user)));
    }
    let nonzero_port = port.filter(|p| *p != 0);
    let tun = tunnel(sec);

    if proto == "SSH" || proto == "SFTP" {
        if let Some(p) = nonzero_port {
            r.push(("port", Value::Int(p)));
        }
        let key = expand_user(strip_space(sec.get("ssh_privatekey")), Some(home));
        if !key.is_empty() && Path::new(&key).is_file() {
            r.push(("identity_file", sv(key)));
        }
        if on(sec, "ssh_compression") {
            r.push(("compression", Value::Bool(true)));
        }
        if on(sec, "ssh_forward_x11") {
            r.push(("x11_forwarding", Value::Bool(true)));
        }
        if on(sec, "ssh_stricthostkeycheck") {
            r.push(("strict_host_key_checking", sv("yes")));
        }
        if proto == "SFTP" {
            r.push(("files_only", Value::Bool(true)));
        }
        if let Some((tu, th)) = tun {
            let jump = if tu.is_empty() {
                th
            } else {
                format!("{tu}@{th}")
            };
            r.push(("proxy_jump", sv(jump)));
        }
        return (Some(row(r)), proto);
    }

    if proto == "RDP" {
        r.push(("protocol", sv("rdp")));
        if let Some(p) = nonzero_port {
            r.push(("port", Value::Int(p)));
        }
        for (src, dst) in [
            ("domain", "domain"),
            ("gateway_server", "gateway"),
            ("gateway_username", "gateway_user"),
            ("gateway_domain", "gateway_domain"),
        ] {
            let val = strip_space(sec.get(src));
            if !val.is_empty() {
                r.push((dst, sv(val)));
            }
        }
        let share = strip_space(sec.get("sharefolder"));
        if !share.is_empty() {
            r.push(("shared_folders", Value::Seq(vec![sv(share)])));
        }
        if strip_space(sec.get("sound")) == "local" {
            r.push(("sound", Value::Bool(true)));
        }
        if on(sec, "multimon") {
            r.push(("multimon", Value::Bool(true)));
        }
        if on(sec, "disableclipboard") {
            r.push(("clipboard", Value::Bool(false)));
        }
    } else if proto == "VNC" {
        r.push(("protocol", sv("vnc")));
        r.retain(|(k, _)| *k != "user"); // plain VNC has no user name
        if let Some(p) = port {
            let p = if p < 100 { p + VNC_DEFAULT_PORT } else { p };
            r.push(("port", Value::Int(p)));
        }
    } else {
        let name = if proto.is_empty() {
            "unknown".to_owned()
        } else {
            proto
        };
        return (None, name);
    }

    if let Some((tu, th)) = tun {
        let (thost, _) = split_server(&th);
        let gateway = find_gateway(existing, &tu, &thost)
            .map(|g| text_of(g, "label"))
            .filter(|l| !l.is_empty());
        match gateway {
            Some(l) => r.push(("ssh_gateway", sv(l))),
            None => notes.push(format!(
                "\u{201c}{label}\u{201d}: goes through an SSH tunnel via {th} \u{2014} add that \
                 host as an SSH entry and pick it as the SSH gateway."
            )),
        }
    }
    (Some(row(r)), proto)
}

/// Every `*.remmina` file in `paths` (files or directories), sorted by name per directory.
/// `existing` are the entries SSH tunnels are matched against; `home` expands `~` in key paths.
pub fn parse_remmina_profiles<P: AsRef<Path>>(
    paths: &[P],
    existing: &[Value],
    home: &Path,
) -> RemminaImport {
    let mut res = RemminaImport::default();
    let mut files: Vec<PathBuf> = Vec::new();
    for p in paths {
        let p = p.as_ref();
        if p.is_dir() {
            let mut names: Vec<_> = std::fs::read_dir(p)
                .map(|rd| {
                    rd.flatten()
                        .map(|e| e.file_name())
                        .filter(|n| platform::os_str_bytes(n).ends_with(b".remmina"))
                        .collect()
                })
                .unwrap_or_default();
            names.sort();
            files.extend(names.into_iter().map(|n| p.join(n)));
        } else {
            files.push(p.to_path_buf());
        }
    }
    if files.len() > MAX_PROFILES {
        files.truncate(MAX_PROFILES);
        res.notes.push(format!(
            "Only the first {MAX_PROFILES} profile files were read."
        ));
    }
    let home = home.to_string_lossy();
    for f in files {
        let parsed = read_limited(&f).and_then(|raw| parse_ini(&String::from_utf8_lossy(&raw)));
        let Some(sec) = parsed else {
            res.unreadable.push(f.to_string_lossy().into_owned());
            continue;
        };
        let (entry, proto) = profile(&sec, existing, &mut res.notes, &home);
        if let Some(entry) = entry {
            res.connections.push(entry);
        } else if !proto.is_empty() {
            match res.skipped.iter_mut().find(|(p, _)| *p == proto) {
                Some((_, n)) => *n += 1,
                None => res.skipped.push((proto, 1)),
            }
        }
    }
    res
}
