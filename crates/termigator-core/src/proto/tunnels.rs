//! Saved tunnels (`ssh -N` with only `-L`/`-R`/`-D`) definitions and command lines.
//!
//! [`parse_forwards`] turns the `L 8080:db:5432` / `R 9000:localhost:22` / `D 1080` lines into
//! `(kind, spec)` pairs (guard 1: a spec never starts with `-`; a bare SOCKS port is bound to
//! `127.0.0.1`). [`normalize_tunnels`] cleans the `tunnels` list of `config.yaml`.
//! [`tunnel_argv`] builds `[sshpass -e] ssh -N -o ExitOnForwardFailure=yes [-o BatchMode=yes] -D …
//! -L … -R … user@host` on top of [`build_ssh_command_with`] from the referenced SSH entry;
//! the entry's own forwards, remote command, mosh and TTY settings are dropped, the ControlMaster
//! is off. A tunnel's own `identity_file` replaces the entry's key (blank: the entry's key, else
//! ssh's default keys); it goes to ssh like the entry's (`-i … -o IdentitiesOnly=yes`), since
//! OpenSSH takes a key file only on the command line or in a config file. Spawning and
//! supervising the process is done elsewhere.
//!
//! Limits (none occur with sane input): `D` ports are recognised with
//! ASCII digits only; a non-list, non-string `forwards` value counts as "no forwards".
//! Golden fixtures: `compat/golden/tools_tunnels.json`.

use std::collections::HashSet;

use crate::config::doc::Value;
use crate::config::model::{strip_space, value_str};
use crate::proto::ssh::{Env, build_ssh_command_with, quote_repr, ssh_arg_injection_guard};
use crate::proto::vnc::{is_space, text, text_stripped};
use crate::shell;

/// The keys of the SSH entry a tunnel borrows (everything else is dropped).
const HOP_KEYS: [&str; 12] = [
    "host",
    "user",
    "port",
    "identity_file",
    "extra_ssh_args",
    "proxy_jump",
    "jumphost",
    "jump_host",
    "jump_identity_file",
    "strict_host_key_checking",
    "password",
    "compression",
];

/// One normalized entry of `config.yaml` -> `tunnels`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tunnel {
    pub name: String,
    /// Label of the SSH entry the tunnel goes through.
    pub via: String,
    pub forwards: Vec<String>,
    pub autostart: bool,
    /// The tunnel's own private key; "" = the key of the SSH entry (or ssh's defaults).
    pub identity_file: String,
}

/// Splits at any line separator, like `str.splitlines`.
fn splitlines(s: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut start = 0;
    let mut it = s.char_indices().peekable();
    while let Some((i, c)) = it.next() {
        if matches!(
            c,
            '\n' | '\r'
                | '\x0b'
                | '\x0c'
                | '\x1c'
                | '\x1d'
                | '\x1e'
                | '\u{85}'
                | '\u{2028}'
                | '\u{2029}'
        ) {
            out.push(&s[start..i]);
            let mut end = i + c.len_utf8();
            if c == '\r'
                && let Some(&(j, '\n')) = it.peek()
            {
                it.next();
                end = j + 1;
            }
            start = end;
        }
    }
    if start < s.len() {
        out.push(&s[start..]);
    }
    out
}

/// The forward lines of a `forwards` value: a list (one item per line) or a text (one line each).
fn lines_of(v: Option<&Value>) -> Vec<String> {
    match v {
        Some(Value::Seq(items)) => items.iter().map(value_str).collect(),
        Some(Value::Str(s)) => splitlines(s).into_iter().map(str::to_owned).collect(),
        _ => Vec::new(),
    }
}

/// `["L 8080:db:5432", "D 1080"]` (or one text, a line each) to `[('L', spec), ...]`; blank lines
/// and `#` comments are skipped. `Err` is the user-facing message.
pub fn parse_forwards(lines: Option<&Value>) -> Result<Vec<(char, String)>, String> {
    let mut out = Vec::new();
    for raw in lines_of(lines) {
        let line = strip_space(&raw);
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let bad = || {
            format!(
                "forward {}: use 'L 8080:host:80', 'R 9000:localhost:22' or 'D 1080'.",
                quote_repr(line)
            )
        };
        // ^\s*([LRD])\s+(\S+)\s*$ on the stripped line.
        let mut chars = line.chars();
        let kind = chars
            .next()
            .map(|c| c.to_ascii_uppercase())
            .filter(|c| matches!(c, 'L' | 'R' | 'D'))
            .ok_or_else(bad)?;
        let rest = chars.as_str();
        let spec = rest.trim_start_matches(is_space);
        if spec.len() == rest.len() || spec.is_empty() || spec.chars().any(is_space) {
            return Err(bad());
        }
        if let Some(err) = ssh_arg_injection_guard(spec) {
            return Err(format!("forward {}: {err}", quote_repr(line)));
        }
        let spec = if kind == 'D' && spec.chars().all(|c| c.is_ascii_digit()) {
            format!("127.0.0.1:{spec}") // a bare SOCKS port stays on this machine
        } else {
            spec.to_owned()
        };
        out.push((kind, spec));
    }
    if out.is_empty() {
        return Err("A tunnel needs at least one forward.".to_owned());
    }
    Ok(out)
}

/// The `tunnels` list of the config: unnamed and repeated names dropped, fields cleaned.
pub fn normalize_tunnels(raw: &Value) -> Vec<Tunnel> {
    let Value::Seq(items) = raw else {
        return Vec::new();
    };
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for t in items {
        if !matches!(t, Value::Map(_)) {
            continue;
        }
        let name = text_stripped(t.get("name"));
        if name.is_empty() || !seen.insert(name.clone()) {
            continue;
        }
        let forwards: Vec<String> = match t.get("forwards") {
            Some(Value::Seq(items)) => items.iter().map(value_str).collect(),
            other => splitlines(&text(other))
                .into_iter()
                .map(str::to_owned)
                .collect(),
        };
        out.push(Tunnel {
            name,
            via: text_stripped(t.get("via")),
            forwards: forwards
                .iter()
                .map(|x| strip_space(x).to_owned())
                .filter(|x| !x.is_empty())
                .collect(),
            autostart: t.get("autostart") == Some(&Value::Bool(true)),
            identity_file: text_stripped(t.get("identity_file")),
        });
    }
    out
}

fn strs(items: impl IntoIterator<Item = String>) -> Value {
    Value::Seq(items.into_iter().map(Value::Str).collect())
}

/// `[sshpass -e] ssh -N -o ExitOnForwardFailure=yes [-o BatchMode=yes] -L ... user@host` for
/// `tunnel` (a mapping with `via`, `forwards` and optionally `identity_file`) through the SSH
/// entry `via`.
///
/// ExitOnForwardFailure makes a busy local port a visible failure; BatchMode stops a key prompt
/// from hanging (there is no terminal to answer it). `launch` is the `launch` config section.
pub fn tunnel_argv(
    tunnel: &Value,
    via: Option<&Value>,
    launch: &Value,
    env: &Env,
) -> Result<Vec<String>, String> {
    let Some(via) = via else {
        let shown = match tunnel.get("via") {
            Some(Value::Str(s)) => quote_repr(s),
            Some(other) => value_str(other),
            None => "None".to_owned(),
        };
        return Err(format!(
            "SSH entry {shown} not found — pick one of the saved SSH entries."
        ));
    };
    let fwds = parse_forwards(tunnel.get("forwards"))?;
    let mut hop: Vec<(Value, Value)> = HOP_KEYS
        .iter()
        .filter_map(|k| via.get(k).map(|v| (Value::Str((*k).to_owned()), v.clone())))
        .collect();
    let own_key = text_stripped(tunnel.get("identity_file"));
    if !own_key.is_empty() {
        hop.retain(|(k, _)| !matches!(k, Value::Str(s) if s == "identity_file"));
        hop.push((Value::Str("identity_file".to_owned()), Value::Str(own_key)));
    }
    for (key, kind) in [("local_forwards", 'L'), ("remote_forwards", 'R')] {
        let specs = fwds
            .iter()
            .filter(|(k, _)| *k == kind)
            .map(|(_, s)| s.clone());
        hop.push((Value::Str(key.to_owned()), strs(specs)));
    }
    let mut lo: Vec<(Value, Value)> = match launch {
        Value::Map(pairs) => pairs
            .iter()
            .filter(|(k, _)| !matches!(k, Value::Str(s) if s == "ssh_control_master"))
            .cloned()
            .collect(),
        _ => Vec::new(),
    };
    lo.push((
        Value::Str("ssh_control_master".to_owned()),
        Value::Bool(false),
    ));
    let hop = Value::Map(hop);
    let line = build_ssh_command_with(&hop, &Value::Map(lo), env)?;
    let mut argv = shell::split(&line).map_err(|e| e.to_string())?;
    let i = argv
        .iter()
        .position(|a| a == "ssh")
        .ok_or_else(|| "ssh is missing from the tunnel command.".to_owned())?;
    let mut extra: Vec<String> = ["-N", "-o", "ExitOnForwardFailure=yes"]
        .map(str::to_owned)
        .into();
    for (kind, spec) in &fwds {
        if *kind == 'D' {
            extra.extend(["-D".to_owned(), spec.clone()]);
        }
    }
    if text(hop.get("password")).is_empty() {
        extra.extend(["-o".to_owned(), "BatchMode=yes".to_owned()]);
    }
    argv.splice(i + 1..i + 1, extra);
    Ok(argv)
}

/// `L 8080:db:5432, D 127.0.0.1:1080` for the tunnels list, or the parse error.
pub fn forwards_summary(tunnel: &Value) -> String {
    match parse_forwards(tunnel.get("forwards")) {
        Err(e) => e,
        Ok(f) => f
            .iter()
            .map(|(k, s)| format!("{k} {s}"))
            .collect::<Vec<_>>()
            .join(", "),
    }
}
