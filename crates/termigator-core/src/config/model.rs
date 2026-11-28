//! Typed accessors over a config [`Value`] with loose coercion, the defaults table
//! and the legacy alias tables.
//!
//! - [`default_config`]: the full `default_config()` table (`save.rs` re-exports it).
//! - [`truthy`], [`value_str`], [`value_int`], [`strip_space`]: truthiness, text form, integer
//!   parsing and whitespace stripping of a [`Value`]. `value_int` returns `None` for `None`,
//!   containers, unparsable text, NaN and values outside `i64`; `value_str` is
//!   exact for scalars except floats in exponent form, and containers give a placeholder
//!   that never equals an alias (shortcut: nothing in the config compares a list's text).
//! - [`connection_protocol`], [`normalize_terminal_mode`], [`default_terminal_from_cfg`],
//!   [`terminal_mode_for_entry`], [`launch_embedded_int`]: the config helpers.
//! - [`proxy_jump_value`] / [`proxy_jump`]: `proxy_jump` or its legacy aliases `jumphost`,
//!   `jump_host` (first truthy wins).
//!
//! On a malformed section (`window: "x"`) the accessors read it as empty.
//! Golden fixtures: `compat/golden/model_*.json`.

use super::doc::Value;

pub const TERMINAL_EMBEDDED: &str = "embedded";
pub const TERMINAL_EXTERNAL: &str = "external";

pub const PROTOCOL_SSH: &str = "ssh";
pub const PROTOCOL_RDP: &str = "rdp";
pub const PROTOCOL_VNC: &str = "vnc";
pub const PROTOCOL_TELNET: &str = "telnet";
pub const PROTOCOL_SERIAL: &str = "serial";

/// Default scrollback of an embedded terminal (`launch.embedded_pyte_history`). The emulator
/// keeps every line at the full terminal width (about 8 bytes per column), so 10 000 lines cost
/// about 7 MB at 80 columns and 16 MB at 200.
pub const DEFAULT_SCROLLBACK_LINES: i64 = 10_000;

/// The legacy spellings of the jump-host key, in priority order.
pub const PROXY_JUMP_KEYS: [&str; 3] = ["proxy_jump", "jumphost", "jump_host"];

pub(super) fn s(text: &str) -> Value {
    Value::Str(text.to_owned())
}

fn map(pairs: Vec<(&str, Value)>) -> Value {
    Value::Map(pairs.into_iter().map(|(k, v)| (s(k), v)).collect())
}

/// The full defaults table: the data merge/strip compare against.
pub fn default_config() -> Value {
    use Value::{Bool, Int, Null};
    map(vec![
        (
            "window",
            map(vec![
                ("width", Int(900)),
                ("height", Int(0)),
                ("min_height", Int(560)),
                ("list_width", Int(0)),
                ("always_on_top", Bool(false)),
                ("follow_konsole", Bool(false)),
                ("follow_poll_ms", Int(500)),
                ("default_terminal", s("embedded")),
                ("tray_icon", Bool(true)),
                ("show_recent", Bool(true)),
                ("confirm_close", Bool(true)),
                ("confirm_close_tab", Bool(true)),
                ("environment_frame", Bool(false)),
                ("notify_on_disconnect", Bool(false)),
                ("check_updates", Bool(true)),
                ("session_restore", Bool(false)),
                ("toolbar", Bool(true)),
            ]),
        ),
        (
            "launch",
            map(vec![
                ("template", s("konsole -e bash -lc {cmd_quoted}")),
                ("shell", s("")),
                ("editor", s("")),
                ("confirm_multiline_paste", Bool(true)),
                ("copy_on_select", Bool(false)),
                ("paste_on_right_click", Bool(false)),
                ("paste_on_middle_click", Bool(true)),
                ("allow_clipboard_write", Bool(true)),
                ("tab_attention_marks", Bool(true)),
                ("session_log_auto", Bool(false)),
                (
                    "session_log_template",
                    s("~/termigator-logs/{label}/{date}_{time}.log"),
                ),
                ("session_log_strip_ansi", Bool(true)),
                ("session_log_timestamps", Bool(false)),
                ("term", s("xterm-256color")),
                ("embedded_pyte_history", Int(DEFAULT_SCROLLBACK_LINES)),
                ("scrollback_unlimited", Bool(false)),
                ("blinking_cursor", Bool(false)),
                ("bracketed_paste", Bool(true)),
                ("shell_executable", Null),
                ("ssh_keepalive", Bool(true)),
                ("ssh_keepalive_seconds", Int(15)),
                ("ssh_control_master", Bool(true)),
                ("rdp_client", Null),
                ("vnc_client", Null),
                ("serial_client", Null),
                ("probe_timeout", Int(5)),
                ("font_family", s("")),
                ("font_size", Int(0)),
                ("color_scheme", s("")),
            ]),
        ),
        ("environments", crate::config::environments::default_value()),
        ("connections", Value::Seq(Vec::new())),
        ("favorites", Value::Seq(Vec::new())),
    ])
}

/// Truthiness: null, `false`, zero and empty text/lists/maps are false.
pub fn truthy(v: &Value) -> bool {
    match v {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Int(i) => *i != 0,
        Value::Float(f) => *f != 0.0,
        Value::Str(t) => !t.is_empty(),
        Value::Timestamp(_) => true,
        Value::Seq(x) => !x.is_empty(),
        Value::Map(x) => !x.is_empty(),
    }
}

/// Text form of `v` (`True`/`False`, null → `"None"`). See the module doc for the limits.
pub fn value_str(v: &Value) -> String {
    match v {
        Value::Null => "None".to_owned(),
        Value::Bool(true) => "True".to_owned(),
        Value::Bool(false) => "False".to_owned(),
        Value::Int(i) => i.to_string(),
        Value::Float(f) if f.is_nan() => "nan".to_owned(),
        Value::Float(f) if f.is_infinite() => (if *f < 0.0 { "-inf" } else { "inf" }).to_owned(),
        Value::Float(f) => format!("{f:?}"),
        Value::Str(t) | Value::Timestamp(t) => t.clone(),
        Value::Seq(_) | Value::Map(_) => "<container>".to_owned(),
    }
}

/// Strips leading/trailing Unicode whitespace (also the `\x1c`-`\x1f` separators Rust's `trim` keeps).
pub fn strip_space(t: &str) -> &str {
    t.trim_matches(|c: char| c.is_whitespace() || ('\x1c'..='\x1f').contains(&c))
}

/// Integer value: bool, int, float (truncated), text (`" 7 "`, `"+5"`, `"1_0"`).
pub fn value_int(v: &Value) -> Option<i64> {
    match v {
        Value::Bool(b) => Some(i64::from(*b)),
        Value::Int(i) => Some(*i),
        Value::Float(f) => (f.is_finite() && f.abs() < 9.2e18).then(|| f.trunc() as i64),
        Value::Str(t) => {
            let t = strip_space(t);
            let (sign, digits) = match t.strip_prefix(['-', '+']) {
                Some(rest) => (&t[..1], rest),
                None => ("", t),
            };
            let bad = digits.is_empty()
                || digits.starts_with('_')
                || digits.ends_with('_')
                || digits.contains("__")
                || !digits.chars().all(|c| c.is_ascii_digit() || c == '_');
            if bad {
                return None;
            }
            format!("{}{}", sign.replace('+', ""), digits.replace('_', ""))
                .parse()
                .ok()
        }
        _ => None,
    }
}

/// `(cfg.get(name) or {})`: the section when it is a non-empty mapping, else an empty one.
pub fn section<'a>(cfg: &'a Value, name: &str) -> &'a Value {
    static EMPTY: Value = Value::Map(Vec::new());
    cfg.get(name)
        .filter(|v| matches!(v, Value::Map(_)))
        .unwrap_or(&EMPTY)
}

/// Protocol of a connection: `rdp` / `vnc` / `telnet` / `serial` by `protocol` (aliases
/// included), else `ssh`.
pub fn connection_protocol(entry: &Value) -> &'static str {
    let Some(p) = entry.get("protocol").filter(|p| truthy(p)) else {
        return PROTOCOL_SSH;
    };
    match strip_space(&value_str(p)).to_lowercase().as_str() {
        "rdp" | "freerdp" | "xfreerdp" | "rdesktop" => PROTOCOL_RDP,
        "vnc" | "vncviewer" | "tigervnc" => PROTOCOL_VNC,
        "telnet" => PROTOCOL_TELNET,
        "serial" | "com" | "tty" => PROTOCOL_SERIAL,
        _ => PROTOCOL_SSH,
    }
}

/// `embedded` or `external`; `None`, unknown values and everything else are `embedded`.
pub fn normalize_terminal_mode(value: Option<&Value>) -> &'static str {
    let t = match value {
        None | Some(Value::Null) => String::new(),
        Some(v) => value_str(v),
    };
    match strip_space(&t).to_lowercase().as_str() {
        "external" | "konsole" | "window" | "out" => TERMINAL_EXTERNAL,
        _ => TERMINAL_EMBEDDED,
    }
}

/// `window.default_terminal`; always `embedded` on Windows, which has no external terminal
/// (launch.template runs through setsid(1) and a POSIX shell line).
pub fn default_terminal_from_cfg(cfg: &Value) -> &'static str {
    if cfg!(windows) {
        return TERMINAL_EMBEDDED;
    }
    normalize_terminal_mode(section(cfg, "window").get("default_terminal"))
}

/// The entry's own `terminal` when set (not `None`, not blank), else the config default; always
/// `embedded` on Windows (see [`default_terminal_from_cfg`]).
pub fn terminal_mode_for_entry(entry: Option<&Value>, cfg: &Value) -> &'static str {
    if cfg!(windows) {
        return TERMINAL_EMBEDDED;
    }
    let own = entry
        .and_then(|e| e.get("terminal"))
        .filter(|t| !matches!(t, Value::Null) && !strip_space(&value_str(t)).is_empty());
    match own {
        Some(t) => normalize_terminal_mode(Some(t)),
        None => default_terminal_from_cfg(cfg),
    }
}

/// First of `keys` present in the `launch` mapping `lo` that `int()` accepts, else `default`.
pub fn launch_embedded_int(lo: &Value, keys: &[&str], default: i64) -> i64 {
    keys.iter()
        .filter_map(|k| lo.get(k))
        .find_map(value_int)
        .unwrap_or(default)
}

/// `conn.get("proxy_jump") or conn.get("jumphost") or conn.get("jump_host")`: the first
/// truthy one, else the last lookup (possibly a falsy value) — `None` when absent.
pub fn proxy_jump_value(conn: &Value) -> Option<&Value> {
    let [a, b, c] = PROXY_JUMP_KEYS;
    [a, b]
        .iter()
        .filter_map(|k| conn.get(k))
        .find(|v| truthy(v))
        .or_else(|| conn.get(c))
}

/// `str(<proxy_jump or jumphost or jump_host or "">).strip()`.
pub fn proxy_jump(conn: &Value) -> String {
    proxy_jump_value(conn)
        .filter(|v| truthy(v))
        .map(|v| strip_space(&value_str(v)).to_owned())
        .unwrap_or_default()
}
