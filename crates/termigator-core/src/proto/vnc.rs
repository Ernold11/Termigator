//! VNC viewer command line.
//!
//! Contract: `Ok(command)` or `Err(user-facing message)`, texts are fixed. Stored
//! passwords are refused (the viewer prompts itself). The first character of the host must not be
//! `/`, `-` or `+` (ARCHITECTURE.md §7, guard 5). Client resolution: `launch.vnc_client`, else the
//! first of `vncviewer`, `xtigervncviewer`, `gvncviewer` for which `which` says yes, else the first
//! candidate. `which` is injected so tests do not depend on the machine; production code passes
//! [`which_in_path`].
//!
//! This module also holds the helpers shared with `telnet` and `serial` (`repr`-style quoting of a
//! string, text-or-empty, port parsing, `which`). Known limit of `quote_repr`: invisible
//! "format" characters (e.g. U+200B) are not escaped.
//!
//! Golden: `compat/golden/protocols_vnc.json`, tests in
//! `tests/proto_vnc.rs`.

use crate::config::doc::Value;
use crate::config::model::{strip_space, truthy, value_int, value_str};
use crate::shell;

pub const VNC_DEFAULT_PORT: i64 = 5900;

/// VNC viewer binaries, in preference order.
pub const VNC_CLIENT_CANDIDATES: [&str; 3] = ["vncviewer", "xtigervncviewer", "gvncviewer"];

/// Injectable `shutil.which`: is `name` an executable in PATH?
pub type Which<'a> = &'a dyn Fn(&str) -> bool;

/// Whether a bare command name is found in `PATH`.
pub fn which_in_path(name: &str) -> bool {
    crate::platform::find_in_path(name).is_some()
}

/// Text of a truthy value, else the empty string.
pub(crate) fn text(v: Option<&Value>) -> String {
    match v {
        Some(x) if truthy(x) => value_str(x),
        _ => String::new(),
    }
}

/// As [`text`], stripped.
pub(crate) fn text_stripped(v: Option<&Value>) -> String {
    strip_space(&text(v)).to_owned()
}

/// Quoted like a string `repr`: single quotes, or double quotes when it holds `'` but no `"`.
pub(crate) fn quote_repr(s: &str) -> String {
    let quote = if s.contains('\'') && !s.contains('"') {
        '"'
    } else {
        '\''
    };
    let mut out = String::from(quote);
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c == quote => {
                out.push('\\');
                out.push(c);
            }
            c if c.is_control() || (!c.is_ascii() && c.is_whitespace()) => {
                let n = u32::from(c);
                if n < 0x100 {
                    out.push_str(&format!("\\x{n:02x}"));
                } else if n < 0x10000 {
                    out.push_str(&format!("\\u{n:04x}"));
                } else {
                    out.push_str(&format!("\\U{n:08x}"));
                }
            }
            c => out.push(c),
        }
    }
    out.push(quote);
    out
}

/// Whether one character is whitespace (Unicode whitespace plus `\x1c`-`\x1f`).
pub(crate) fn is_space(c: char) -> bool {
    c.is_whitespace() || ('\x1c'..='\x1f').contains(&c)
}

/// The `port` entry (default when absent or blank); range is 1..=65535.
pub(crate) fn parse_port(conn: &Value, default: i64) -> Result<i64, String> {
    let Some(raw) = conn.get("port").filter(|v| **v != Value::Null) else {
        return Ok(default);
    };
    if strip_space(&value_str(raw)).is_empty() {
        return Ok(default);
    }
    let port = value_int(raw).ok_or_else(|| "Invalid port.".to_owned())?;
    if !(1..=65535).contains(&port) {
        return Err("Port must be between 1 and 65535.".to_owned());
    }
    Ok(port)
}

/// VNC viewer executable: `launch.vnc_client` if set, else the first candidate present.
pub fn resolve_vnc_client(launch: &Value, which: Which) -> String {
    let over = text_stripped(launch.get("vnc_client"));
    if !over.is_empty() {
        return over;
    }
    VNC_CLIENT_CANDIDATES
        .iter()
        .find(|n| which(n))
        .unwrap_or(&VNC_CLIENT_CANDIDATES[0])
        .to_string()
}

fn value_guard(s: &str) -> Option<&'static str> {
    let t = strip_space(s);
    let Some(first) = t.chars().next() else {
        return Some("empty value");
    };
    if "/-+".contains(first) {
        return Some("must not start with '/', '-' or '+' (pass options via Extra VNC arguments)");
    }
    None
}

/// `vncviewer [extra…] host::port`, shell-quoted.
pub fn build_vnc_command(conn: &Value, launch: &Value, which: Which) -> Result<String, String> {
    let host = text_stripped(conn.get("host"));
    if host.is_empty() {
        return Err("Host is missing.".to_owned());
    }
    if let Some(g) = value_guard(&host) {
        return Err(format!("host {}: {g}", quote_repr(&host)));
    }
    let port = parse_port(conn, VNC_DEFAULT_PORT)?;
    let mut argv = vec![resolve_vnc_client(launch, which)];
    match conn.get("extra_vnc_args") {
        Some(Value::Seq(items)) => argv.extend(items.iter().map(value_str)),
        Some(x) if truthy(x) => argv.extend(
            shell::split(&value_str(x))
                .map_err(|_| "Could not parse extra_vnc_args.".to_owned())?,
        ),
        _ => {}
    }
    // host::port is the TigerVNC syntax for a raw TCP port (single colon = display number).
    argv.push(format!("{host}::{port}"));
    if !text(conn.get("password")).is_empty() {
        return Err(
            "VNC entries do not support a stored password — the viewer prompts for it. \
                    Edit the entry and save it to remove the password."
                .to_owned(),
        );
    }
    Ok(shell::join(&argv))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repr_quoting_rules() {
        assert_eq!(quote_repr("a"), "'a'");
        assert_eq!(quote_repr("it's"), "\"it's\"");
        assert_eq!(quote_repr("a'\"b"), "'a\\'\"b'");
        assert_eq!(quote_repr("a\tb\x01"), "'a\\tb\\x01'");
    }

    #[test]
    fn which_in_path_finds_sh_not_nonsense() {
        assert!(which_in_path(if cfg!(windows) { "cmd" } else { "sh" }));
        assert!(!which_in_path("no-such-binary-termigator"));
    }
}
