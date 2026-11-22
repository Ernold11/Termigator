//! telnet command line.
//!
//! Same contract as `vnc`: `Ok(command)` or `Err(message)`. Telnet is clear
//! text, so a stored password is refused. Client: `telnet`, else `busybox telnet`, decided by the
//! injected `which`. Guards (ARCHITECTURE.md §7, guard 7): host and user must not start with `-`
//! nor contain whitespace. The port is written only when it differs from 23.
//!
//! Golden: `compat/golden/protocols_telnet.json`, tests in
//! `tests/proto_telnet.rs`.

use crate::config::doc::Value;
use crate::shell;

use super::vnc::{Which, is_space, parse_port, quote_repr, text, text_stripped};

pub const TELNET_DEFAULT_PORT: i64 = 23;

fn guard(name: &str, value: &str) -> Result<(), String> {
    if value.starts_with('-') {
        return Err(format!(
            "{name} {}: must not start with '-'",
            quote_repr(value)
        ));
    }
    if value.chars().any(is_space) {
        return Err(format!(
            "{name} {}: must not contain spaces",
            quote_repr(value)
        ));
    }
    Ok(())
}

/// `["telnet"]`, `["busybox", "telnet"]` or `None` when neither is installed.
pub fn telnet_client_argv(which: Which) -> Option<Vec<String>> {
    if which("telnet") {
        return Some(vec!["telnet".to_owned()]);
    }
    which("busybox").then(|| vec!["busybox".to_owned(), "telnet".to_owned()])
}

/// `telnet [-l user] host [port]`, shell-quoted.
pub fn build_telnet_command(conn: &Value, which: Which) -> Result<String, String> {
    let host = text_stripped(conn.get("host"));
    if host.is_empty() {
        return Err("Host is missing.".to_owned());
    }
    guard("host", &host)?;
    let port = parse_port(conn, TELNET_DEFAULT_PORT)?;
    if !text(conn.get("password")).is_empty() {
        return Err(
            "Telnet entries do not store a password (it would travel in clear text anyway)."
                .to_owned(),
        );
    }
    let mut argv = telnet_client_argv(which)
        .ok_or_else(|| "telnet is not installed (e.g. sudo apt install telnet).".to_owned())?;
    let user = text_stripped(conn.get("user"));
    if !user.is_empty() {
        guard("user", &user)?;
        argv.extend(["-l".to_owned(), user]); // sent as USER over NEW-ENVIRON; most devices still prompt
    }
    argv.push(host);
    if port != TELNET_DEFAULT_PORT {
        argv.push(port.to_string());
    }
    Ok(shell::join(&argv))
}
