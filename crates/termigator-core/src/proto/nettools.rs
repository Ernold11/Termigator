//! Network tools for a saved host: ping, trace route, DNS lookup, port check, port scan.
//!
//! Each tool picks the best program installed (`mtr` over `traceroute` over `tracepath`, `dig`
//! over `host` over `getent`, `nc` over bash's `/dev/tcp`); the lookup is injectable
//! ([`Which`]). [`tool_argv`] refuses a host that is not a plain name or IP literal (guard 8:
//! `^[A-Za-z0-9_.:%\-]+$`, never a leading `-`). [`tool_for_entry`] aims a tool at the endpoint
//! the connection test probes ([`endpoint_for`]: the first hop behind a jump host, with a note
//! echoed first) and joins the argv with [`shell::join`]; nothing goes through a shell string
//! built from the entry. Running the line in a terminal tab is the UI's job.
//!
//! As with a regex `$`, one trailing newline after the host is accepted (it is quoted by
//! the join and cannot start an option). Golden fixtures:
//! `compat/golden/tools_nettools.json`.

use crate::config::doc::Value;
use crate::connectivity::endpoint_for;
use crate::proto::ssh::quote_repr;
use crate::proto::vnc::Which;
use crate::shell;

/// `(key, menu label)`.
pub const TOOLS: [(&str, &str); 5] = [
    ("ping", "Ping"),
    ("trace", "Trace route"),
    ("dns", "DNS lookup"),
    ("port", "Check port"),
    ("scan", "Scan common ports"),
];

fn host_ok(host: &str) -> bool {
    let h = host.strip_suffix('\n').unwrap_or(host);
    !h.is_empty()
        && !host.starts_with('-')
        && h.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | ':' | '%' | '-'))
}

fn argv(parts: &[&str]) -> Vec<String> {
    parts.iter().map(|s| (*s).to_owned()).collect()
}

/// argv for `tool` against `host:port`, or the reason it cannot run.
pub fn tool_argv(tool: &str, host: &str, port: i64, which: Which) -> Result<Vec<String>, String> {
    if host.is_empty() || !host_ok(host) {
        return Err(format!("Not a host name: {}", quote_repr(host)));
    }
    match tool {
        "ping" if which("ping") => Ok(argv(&["ping", host])),
        "ping" => Err("ping is not installed.".to_owned()),
        "trace" => {
            if which("mtr") {
                Ok(argv(&["mtr", "--curses", host]))
            } else if which("traceroute") {
                Ok(argv(&["traceroute", host]))
            } else if which("tracepath") {
                Ok(argv(&["tracepath", host]))
            } else {
                Err("Install mtr, traceroute or tracepath.".to_owned())
            }
        }
        "dns" => Ok(if which("dig") {
            argv(&[
                "dig",
                "+noall",
                "+answer",
                "+authority",
                host,
                "A",
                host,
                "AAAA",
            ])
        } else if which("host") {
            argv(&["host", host])
        } else {
            argv(&["getent", "ahosts", host])
        }),
        "port" => {
            let port = port.to_string();
            if which("nc") {
                return Ok(argv(&["nc", "-zv", "-w", "5", host, &port]));
            }
            // bash's /dev/tcp: no extra package needed; host / port are argv, not code.
            let script = "timeout 5 bash -c 'exec 3<>/dev/tcp/$1/$2' _ \"$1\" \"$2\"\
                          \x20&& echo \"$1 port $2: open\" || echo \"$1 port $2: closed or filtered\"";
            Ok(argv(&["bash", "-c", script, "_", host, &port]))
        }
        "scan" if which("nmap") => Ok(argv(&["nmap", "-Pn", "--top-ports", "100", host])),
        "scan" => Err("Install nmap to scan ports.".to_owned()),
        _ => Err(format!("Unknown tool: {tool}")),
    }
}

/// [`tool_argv`] for Windows: `ping -t`, `tracert`, `nslookup`, PowerShell `Test-NetConnection`.
/// Port scan has no Windows tool yet. `host` is validated like on Unix, so it cannot start an
/// option or carry shell syntax; none of these programs is run through a shell string.
pub fn tool_argv_windows(tool: &str, host: &str, port: i64) -> Result<Vec<String>, String> {
    if host.is_empty() || !host_ok(host) {
        return Err(format!("Not a host name: {}", quote_repr(host)));
    }
    match tool {
        "ping" => Ok(argv(&["ping", "-t", host])),
        "trace" => Ok(argv(&["tracert", host])),
        "dns" => Ok(argv(&["nslookup", host])),
        "port" => Ok(argv(&[
            "powershell",
            "-NoProfile",
            "-Command",
            "Test-NetConnection",
            "-ComputerName",
            host,
            "-Port",
            &port.to_string(),
        ])),
        "scan" => Err("Port scan is not available on Windows.".to_owned()),
        _ => Err(format!("Unknown tool: {tool}")),
    }
}

/// `(command line, tab title)` for `tool` against a saved entry, or the error message.
pub fn tool_for_entry(tool: &str, conn: &Value, which: Which) -> Result<(String, String), String> {
    let (host, port, via_note, err) = endpoint_for(conn);
    if let Some(e) = err.filter(|e| !e.is_empty()) {
        return Err(e);
    }
    let argv = tool_argv(tool, &host, port, which)?;
    let label = TOOLS
        .iter()
        .find(|(k, _)| *k == tool)
        .map_or(tool, |(_, l)| l);
    let mut title = format!("{label}: {host}");
    if tool == "port" {
        title.push_str(&format!(":{port}"));
    }
    let mut cmd = shell::join(&argv);
    if !via_note.is_empty() {
        cmd = format!("echo {}; echo; {cmd}", shell::quote(&via_note));
    }
    Ok((cmd, title))
}
