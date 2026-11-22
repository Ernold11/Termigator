//! MobaXterm INI import.
//!
//! - [`decode_moba_bytes`]: UTF-16 with BOM (strict), UTF-8 with BOM (lossy), strict UTF-8,
//!   else Windows-1250 with undefined bytes replaced by U+FFFD (code page cp1250).
//! - [`parse_moba_bookmarks`]: the SSH bookmarks of `[Bookmarks]` / `[Bookmarks_*]` sections
//!   (what the CLI imports); [`parse_moba_export`]: SSH plus RDP, VNC, Telnet and SFTP rows
//!   and a count of the skipped session types (the GUI path). The session type decides, not
//!   the icon; `SubRep` becomes the group; entries without a user get `LastUsername`, else
//!   the local user (both injectable: functions take `local_user`, see [`local_user`]).
//! - [`merge_moba_connections`]: appends rows whose [`connection_key`] is new.
//!
//! Known defect kept 1:1: the key is `(host, user, port)` without the protocol, so a
//! second RDP/VNC/Telnet entry for the same host and user is dropped.
//!
//! By design: malformed UTF-16 is an error; `\d` in the
//! bookmark head is ASCII only; ports and session types beyond 64 bits are not supported.
//! Golden fixtures: `compat/golden/moba_*.json`.

use std::collections::HashSet;

use crate::config::doc::Value;
use crate::config::model::{strip_space, truthy, value_int, value_str};

pub const MOBA_SSH: u64 = 0;
pub const MOBA_TELNET: u64 = 1;
pub const MOBA_RDP: u64 = 4;
pub const MOBA_VNC: u64 = 5;
pub const MOBA_SFTP: u64 = 7;

/// Malformed UTF-16 (odd length or lone surrogate) after a UTF-16 BOM.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("invalid UTF-16 data")]
pub struct DecodeError;

/// Windows-1250 bytes the cp1250 code page leaves undefined (WHATWG maps them to C1).
const CP1250_UNDEFINED: [u8; 5] = [0x81, 0x83, 0x88, 0x90, 0x98];

/// `decode_moba_bytes`: BOM first, then strict UTF-8, then Windows-1250.
pub fn decode_moba_bytes(data: &[u8]) -> Result<String, DecodeError> {
    if let Some(rest) = data.strip_prefix(&[0xff, 0xfe]) {
        return encoding_rs::UTF_16LE
            .decode_without_bom_handling_and_without_replacement(rest)
            .map(|c| c.into_owned())
            .ok_or(DecodeError);
    }
    if let Some(rest) = data.strip_prefix(&[0xfe, 0xff]) {
        return encoding_rs::UTF_16BE
            .decode_without_bom_handling_and_without_replacement(rest)
            .map(|c| c.into_owned())
            .ok_or(DecodeError);
    }
    if let Some(rest) = data.strip_prefix(&[0xef, 0xbb, 0xbf]) {
        return Ok(String::from_utf8_lossy(rest).into_owned());
    }
    if let Ok(s) = std::str::from_utf8(data) {
        return Ok(s.to_owned());
    }
    // Single-byte code page: splitting at the undefined bytes cannot change the rest.
    let mut out = String::new();
    for (i, chunk) in data.split(|b| CP1250_UNDEFINED.contains(b)).enumerate() {
        if i > 0 {
            out.push('\u{fffd}');
        }
        out.push_str(
            &encoding_rs::WINDOWS_1250
                .decode_without_bom_handling(chunk)
                .0,
        );
    }
    Ok(out)
}

/// The Linux account importing the file: `LOGNAME`, `USER`,
/// `LNAME`, `USERNAME`, then the passwd entry; `""` if none.
pub fn local_user() -> String {
    for name in ["LOGNAME", "USER", "LNAME", "USERNAME"] {
        if let Some(v) = std::env::var_os(name).and_then(|v| v.into_string().ok())
            && !v.is_empty()
        {
            return v;
        }
    }
    crate::platform::current_user_name().unwrap_or_default()
}

/// Splits at line boundaries (`\n`, `\r\n`, `\r` and the other Unicode separators).
pub(crate) fn splitlines(text: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut start = 0;
    let mut chars = text.char_indices().peekable();
    while let Some((i, c)) = chars.next() {
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
            out.push(&text[start..i]);
            let mut end = i + c.len_utf8();
            if c == '\r' && chars.peek().is_some_and(|&(_, n)| n == '\n') {
                chars.next();
                end += 1;
            }
            start = end;
        }
    }
    if start < text.len() {
        out.push(&text[start..]);
    }
    out
}

pub(crate) fn truncate_chars(s: &str, n: usize) -> String {
    s.chars().take(n).collect()
}

pub(crate) fn sv(s: impl Into<String>) -> Value {
    Value::Str(s.into())
}

pub(crate) fn row(pairs: Vec<(&str, Value)>) -> Value {
    Value::Map(pairs.into_iter().map(|(k, v)| (sv(k), v)).collect())
}

/// `LastUsername=` of the first line that starts with it (any section); `None` if empty.
fn misc_username(text: &str) -> Option<String> {
    splitlines(text).into_iter().find_map(|line| {
        strip_space(line)
            .strip_prefix("LastUsername=")
            .map(|v| Some(strip_space(v).to_owned()).filter(|v| !v.is_empty()))
    })?
}

/// `#<icon>#<type>%…` → the session type.
fn session_type(value: &str) -> Option<u64> {
    let rest = value.strip_prefix('#')?;
    let icon_end = rest.find(|c: char| !c.is_ascii_digit())?;
    let rest = rest[icon_end..]
        .strip_prefix('#')
        .filter(|_| icon_end > 0)?;
    let ty_end = rest.find(|c: char| !c.is_ascii_digit())?;
    if ty_end == 0 || !rest[ty_end..].starts_with('%') {
        return None;
    }
    Some(rest[..ty_end].parse().unwrap_or(u64::MAX))
}

fn type_name(kind: u64) -> String {
    let names = [
        "SSH", "Telnet", "Rsh", "XDMCP", "RDP", "VNC", "FTP", "SFTP", "Serial", "File", "Shell",
        "Browser", "Mosh", "S3", "WSL",
    ];
    usize::try_from(kind)
        .ok()
        .and_then(|i| names.get(i))
        .map_or_else(|| format!("type {kind}"), |n| (*n).to_owned())
}

struct SshBookmark {
    host: String,
    user: String,
    port: String,
    identity: Option<String>,
}

fn parse_ssh_value(rest: &str) -> Option<SshBookmark> {
    if session_type(rest) != Some(MOBA_SSH) {
        return None;
    }
    let parts: Vec<&str> = rest.split('%').collect();
    if parts.len() < 4 {
        return None;
    }
    let mut host = strip_space(parts[1]).to_owned();
    let port = strip_space(parts[2]);
    let port = if port.is_empty() { "22" } else { port }.to_owned();
    let mut user = strip_space(parts[3]).to_owned();
    if host.is_empty() {
        return None;
    }
    if user.is_empty()
        && let Some((u, h)) = host.split_once('@')
        && !u.is_empty()
        && !h.is_empty()
    {
        (user, host) = (strip_space(u).to_owned(), strip_space(h).to_owned());
    }
    const MARKERS: [&str; 9] = [
        ".ssh",
        "id_rsa",
        "id_ed25519",
        "_MyDocuments_",
        "_ProfileDir_",
        ".pem",
        "PRIV",
        "priv",
        ":\\",
    ];
    // Any `_Token_` MobaXterm expands itself (`_DesktopDir_`, `_AppDataDir_`, `_CurrentDrive_`, …)
    // and the key formats it keeps there mark a key path too.
    let is_key_path = |p: &str| {
        MARKERS.iter().any(|m| p.contains(m))
            || starts_with_moba_token(p).is_some()
            || p.ends_with(".ppk")
            || p.ends_with(".key")
    };
    let mut identity = None;
    for p in &parts[4..] {
        let p = strip_space(p);
        if p.is_empty() || ["-1", "0", "1", "22"].contains(&p) {
            continue;
        }
        if p.contains("MobaFont") || p.contains("Std_Colors") || p.starts_with('#') {
            break;
        }
        if p == "Interactive shell" {
            continue;
        }
        // Every marker has a non-digit, so a digits-only check is never needed.
        if is_key_path(p) && p.chars().count() > 4 {
            identity = Some(p.to_owned());
            break;
        }
    }
    Some(SshBookmark {
        host,
        user,
        port,
        identity,
    })
}

/// The normalised POSIX path and its last component for the part after `X:/`.
fn posix_path(rest: &str) -> (String, String) {
    let lead = rest.chars().take_while(|&c| c == '/').count();
    let comps: Vec<&str> = rest
        .split('/')
        .filter(|c| !c.is_empty() && *c != ".")
        .collect();
    let root = match lead {
        0 => "",
        2 => "//",
        _ => "/",
    };
    if comps.is_empty() {
        return (
            if root.is_empty() { "." } else { root }.to_owned(),
            String::new(),
        );
    }
    let name = comps.last().map(|c| (*c).to_owned()).unwrap_or_default();
    (format!("{root}{}", comps.join("/")), name)
}

/// A key path from a Windows program for this machine: a Windows path on Windows
/// ([`super::windows::moba_identity_to_windows`]), else [`moba_identity_to_linux`].
pub(crate) fn identity_local(path: Option<&str>, local_user: &str) -> Option<String> {
    match super::windows::win_dirs() {
        Some(d) => super::windows::moba_identity_to_windows(path, &d),
        None => moba_identity_to_linux(path, local_user),
    }
}

/// The `_Token_` a MobaXterm path starts with (`_DesktopDir_` in `_DesktopDir_\\k.ppk`) and the
/// rest of the path after it (a `:` right after the token, as in `_CurrentDrive_:\\k`, is dropped).
pub(crate) fn starts_with_moba_token(path: &str) -> Option<(&str, &str)> {
    let inner = path.strip_prefix('_')?;
    let end = inner.find('_')?;
    if end == 0 || !inner[..end].chars().all(|c| c.is_ascii_alphanumeric()) {
        return None;
    }
    let token_len = end + 2;
    let rest = path[token_len..]
        .strip_prefix(':')
        .unwrap_or(&path[token_len..]);
    Some((&path[..token_len], rest))
}

/// `moba_identity_to_linux`: a Windows key path as a Linux one.
pub fn moba_identity_to_linux(path: Option<&str>, linux_user: &str) -> Option<String> {
    let path = path.filter(|p| !p.is_empty())?;
    let p = strip_space(&path.replace('\\', "/")).to_owned();
    if let Some(tail) = p.strip_prefix("_MyDocuments_/MobaXterm/home/") {
        return Some(format!(
            "~/{}",
            tail.trim_start_matches('/').replace("//", "/")
        ));
    }
    if let Some(tail) = p.strip_prefix("_ProfileDir_/Documents/") {
        return Some(format!(
            "~/Documents/{}",
            tail.trim_start_matches('/').replace("//", "/")
        ));
    }
    // The other folders MobaXterm expands: the profile is the home folder, the desktop is
    // `~/Desktop`, a drive root is `~` like any `X:/` path below. Unknown tokens stay as written.
    if let Some((token, rest)) = starts_with_moba_token(&p)
        && let Some(tail) = rest.strip_prefix('/')
    {
        let tail = tail.trim_start_matches('/').replace("//", "/");
        match token {
            "_DesktopDir_" => return Some(format!("~/Desktop/{tail}")),
            "_ProfileDir_" | "_CurrentDrive_" if !tail.is_empty() => {
                return Some(format!("~/{tail}"));
            }
            _ => {}
        }
    }
    let drive_rest = {
        let mut it = p.chars();
        match (it.next(), it.next()) {
            (Some(d), Some(':')) if d.is_ascii_alphabetic() => p[2..].strip_prefix('/'),
            _ => None,
        }
    };
    let Some(rest) = drive_rest.filter(|r| !r.contains('\n')) else {
        return Some(p);
    };
    let key = rest
        .strip_prefix("home/")
        .and_then(|r| r.split_once('/'))
        .filter(|(u, _)| !u.is_empty())
        .and_then(|(u, tail)| Some((u, tail.strip_prefix(".ssh/")?)))
        .filter(|(_, k)| !k.is_empty());
    if let Some((u, keyfile)) = key {
        return Some(if u == linux_user {
            format!("~/.ssh/{keyfile}")
        } else {
            format!("/home/{u}/.ssh/{keyfile}")
        });
    }
    let (posix, name) = posix_path(rest);
    let lower = rest.to_lowercase();
    if lower.contains("backup") && lower.contains("documents") {
        return Some(format!("~/Documents/{name}"));
    }
    Some(format!("~/{posix}"))
}

fn subrep_to_group(subrep: &str) -> String {
    let g = strip_space(&subrep.replace('\\', "/"))
        .trim_matches('/')
        .to_owned();
    truncate_chars(&g, 80)
}

fn is_bookmarks_section(line: &str) -> Option<bool> {
    let s = strip_space(line);
    (s.starts_with('[') && s.ends_with(']')).then(|| {
        let sec = &s[1..s.len() - 1];
        sec == "Bookmarks" || sec.starts_with("Bookmarks_")
    })
}

/// `(label, value, group)` of every `key=value` line of the bookmark sections.
fn bookmark_lines(text: &str) -> Vec<(String, String, String)> {
    let mut out = Vec::new();
    let mut in_bookmarks = false;
    let mut group = String::new();
    for line in splitlines(text.trim_start_matches('\u{feff}')) {
        if let Some(b) = is_bookmarks_section(line) {
            in_bookmarks = b;
            group.clear();
            continue;
        }
        let Some((label, rest)) = line.split_once('=') else {
            continue;
        };
        if !in_bookmarks {
            continue;
        }
        let (label, rest) = (strip_space(label), strip_space(rest));
        if label.to_lowercase() == "subrep" {
            group = subrep_to_group(rest);
            continue;
        }
        out.push((label.to_owned(), rest.to_owned(), group.clone()));
    }
    out
}

/// `parse_moba_bookmarks`: Termigator connection rows of the SSH bookmarks.
pub fn parse_moba_bookmarks(text: &str, local_user: &str) -> Vec<Value> {
    let default_user = misc_username(text.trim_start_matches('\u{feff}')).unwrap_or_default();
    let mut out = Vec::new();
    for (label, rest, group) in bookmark_lines(text) {
        if label.to_lowercase() == "imgnum" {
            continue;
        }
        let Some(b) = parse_ssh_value(&rest) else {
            continue;
        };
        // No per-entry user: Moba's last login, else the local user.
        let user = [b.user.as_str(), default_user.as_str(), local_user]
            .into_iter()
            .find(|u| !u.is_empty())
            .unwrap_or_default()
            .to_owned();
        if user.is_empty() {
            continue;
        }
        let port = value_int(&text_value(&b.port)).unwrap_or(22);
        let mut r = vec![
            ("label", sv(truncate_chars(&label, 120))),
            ("host", sv(b.host)),
            ("user", sv(user)),
        ];
        if port != 22 {
            r.push(("port", Value::Int(port)));
        }
        if let Some(i) = identity_local(b.identity.as_deref(), local_user)
            && !i.is_empty()
        {
            r.push(("identity_file", sv(i)));
        }
        if !group.is_empty() {
            r.push(("group", sv(group)));
        }
        out.push(row(r));
    }
    out
}

fn text_value(s: &str) -> Value {
    Value::Str(s.to_owned())
}

/// An RDP, VNC, Telnet or SFTP bookmark; `None` for anything else or a malformed one.
fn remote_desktop_row(label: &str, rest: &str, group: &str, local_user: &str) -> Option<Value> {
    let kind = session_type(rest)?;
    let parts: Vec<&str> = if rest.matches('#').count() >= 2 {
        rest.split('#')
            .nth(2)
            .unwrap_or_default()
            .split('%')
            .collect()
    } else {
        Vec::new()
    };
    if ![MOBA_RDP, MOBA_VNC, MOBA_TELNET, MOBA_SFTP].contains(&kind) || parts.len() < 3 {
        return None;
    }
    let host = strip_space(parts[1]);
    if host.is_empty() {
        return None;
    }
    let (protocol, default_port) = match kind {
        MOBA_RDP => ("rdp", 3389),
        MOBA_VNC => ("vnc", 5900),
        MOBA_TELNET => ("telnet", 23),
        _ => ("ssh", 22),
    };
    let mut r = vec![("label", sv(truncate_chars(label, 120)))];
    if kind == MOBA_SFTP {
        // An SSH entry that opens as a file browser.
        r.push(("host", sv(host)));
        r.push(("files_only", Value::Bool(true)));
    } else {
        r.push(("protocol", sv(protocol)));
        r.push(("host", sv(host)));
    }
    let port_text = strip_space(parts[2]);
    let port = if port_text.is_empty() {
        default_port
    } else {
        value_int(&text_value(port_text)).unwrap_or(default_port)
    };
    if port != default_port && 0 < port && port < 65536 {
        r.push(("port", Value::Int(port)));
    }
    if kind == MOBA_SFTP {
        let user = parts.get(3).map_or("", |u| strip_space(u));
        r.push(("user", sv(if user.is_empty() { local_user } else { user })));
    }
    if kind == MOBA_RDP && parts.len() > 3 {
        let mut user = strip_space(parts[3]).to_owned();
        if let Some((domain, name)) = user.split_once('\\')
            && !domain.is_empty()
            && !name.is_empty()
        {
            r.push(("domain", sv(domain)));
            user = name.to_owned();
        }
        if !user.is_empty() {
            r.push(("user", sv(user)));
        }
    }
    if !group.is_empty() {
        r.push(("group", sv(group)));
    }
    Some(row(r))
}

/// `parse_moba_export`: SSH rows, then RDP / VNC / Telnet / SFTP rows, and the number of
/// bookmarks of every other session type, in order of first appearance.
pub fn parse_moba_export(text: &str, local_user: &str) -> (Vec<Value>, Vec<(String, u32)>) {
    let mut rows = parse_moba_bookmarks(text, local_user);
    let mut skipped: Vec<(String, u32)> = Vec::new();
    for (label, rest, group) in bookmark_lines(text) {
        let Some(kind) = session_type(&rest) else {
            continue;
        };
        if kind == MOBA_SSH {
            continue;
        }
        if let Some(r) = remote_desktop_row(&label, &rest, &group, local_user) {
            rows.push(r);
            continue;
        }
        let name = type_name(kind);
        match skipped.iter_mut().find(|(n, _)| *n == name) {
            Some((_, c)) => *c += 1,
            None => skipped.push((name, 1)),
        }
    }
    (rows, skipped)
}

fn field_text(c: &Value, key: &str) -> String {
    c.get(key)
        .filter(|v| truthy(v))
        .map(value_str)
        .unwrap_or_default()
}

/// `connection_key`: `(host, user, port)`, lower-cased and stripped; a missing or
/// non-numeric port is 22.
pub fn connection_key(c: &Value) -> (String, String, i64) {
    let port = field_text(c, "port");
    let port = strip_space(&port);
    let port = if port.is_empty() {
        22
    } else {
        value_int(&text_value(port)).unwrap_or(22)
    };
    (
        strip_space(&field_text(c, "host")).to_lowercase(),
        strip_space(&field_text(c, "user")).to_lowercase(),
        port,
    )
}

/// `merge_moba_connections`: *parsed* rows appended to the mappings of *existing*, skipping
/// duplicate keys; new rows get `terminal` if they have none. Returns `(merged, added)`.
pub fn merge_moba_connections(
    existing: &[Value],
    parsed: &[Value],
    default_terminal: &str,
) -> (Vec<Value>, usize) {
    let mut merged: Vec<Value> = existing
        .iter()
        .filter(|x| matches!(x, Value::Map(_)))
        .cloned()
        .collect();
    let mut seen: HashSet<_> = merged.iter().map(connection_key).collect();
    let mut added = 0;
    for r in parsed {
        let key = connection_key(r);
        if !seen.insert(key) {
            continue;
        }
        let mut r = r.clone();
        if let Value::Map(pairs) = &mut r
            && r_lacks_terminal(pairs)
        {
            pairs.push((sv("terminal"), sv(default_terminal)));
        }
        merged.push(r);
        added += 1;
    }
    (merged, added)
}

fn r_lacks_terminal(pairs: &[(Value, Value)]) -> bool {
    !pairs
        .iter()
        .any(|(k, _)| matches!(k, Value::Str(s) if s == "terminal"))
}
