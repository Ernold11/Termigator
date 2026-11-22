//! PuTTY, `.reg` and SuperPuTTY import.
//!
//! Three sources, SSH sessions only, each yielding Termigator connection rows:
//! - [`parse_putty_sessions_dir`] / [`parse_putty_session_file`]: Linux `~/.putty/sessions/*`
//!   (`Key=Value` files, URL-encoded names; `Default%20Settings` is skipped),
//! - [`parse_putty_reg`]: a `.reg` export (`"k"="v"` with `\\` unescaped, `dword:hex`),
//! - [`parse_superputty_xml`]: `sessions.xml` (`SessionData` attributes; the `SessionId`
//!   folders become the group). Malformed XML gives no rows.
//!
//! [`parse_putty_export`] picks `.reg` or XML for a file (UTF-16 with BOM is decoded, invalid
//! data replaced). `.ppk` keys are dropped; `PortForwardings` become `local_forwards`,
//! `remote_forwards` and `dynamic_forward`; `ProxyMethod` 6 becomes `proxy_jump`. Rows are
//! merged with `import::moba::merge_moba_connections`. Key paths use `local_user` like the
//! MobaXterm importer (injectable instead of reading the OS user).
//!
//! The input is untrusted: nothing here panics, XML is read as a stream (no recursion, no
//! entity expansion beyond the predefined ones) and files are read at most [`MAX_FILE_BYTES`].
//!
//! Differences by design: files over [`MAX_FILE_BYTES`] are skipped; numbers beyond 64 bits are
//! not supported; the XML reader is a little more lenient than expat (text and undeclared
//! prefixes are not validated) but rejects unclosed, mismatched, duplicate-attribute and
//! multi-root documents. Golden fixtures: `compat/golden/putty_*.json`.

use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};

use percent_encoding::percent_decode_str;
use quick_xml::events::{BytesStart, Event};

use super::moba::{identity_local, row, splitlines, sv, truncate_chars};
use crate::config::doc::Value;
use crate::config::model::{strip_space, truthy, value_int, value_str};

/// Largest session / export file read (bigger ones are skipped or give no rows).
pub const MAX_FILE_BYTES: u64 = 16 * 1024 * 1024;

/// PuTTY's `ProxyMethod` "SSH to proxy and use port forwarding" (0.77+): a jump host.
const PROXY_SSH: i64 = 6;

/// `~/.putty/sessions` (`$HOME`, else the passwd home); `None` if there is no home.
pub fn default_sessions_dir() -> Option<PathBuf> {
    let home = std::env::var_os("HOME")
        .filter(|h| !h.is_empty())
        .map(PathBuf::from)
        .or_else(crate::platform::current_user_home)?;
    Some(home.join(".putty").join("sessions"))
}

fn unquote(s: &str) -> String {
    percent_decode_str(s).decode_utf8_lossy().into_owned()
}

/// `_identity_from_putty`: a key path as a Linux one, `None` for empty or `.ppk`.
fn identity_from_putty(raw: &str, local_user: &str) -> Option<String> {
    let p = strip_space(raw).trim_matches('"');
    if p.is_empty() || p.to_lowercase().ends_with(".ppk") {
        return None;
    }
    identity_local(Some(p), local_user)
}

/// `_row`: PuTTY allows `user@host` in the host field.
fn make_row(
    label: &str,
    host: &str,
    user: &str,
    port: Option<i64>,
    identity: Option<String>,
    group: &str,
) -> Vec<(&'static str, Value)> {
    let (mut host, mut user) = (host.to_owned(), user.to_owned());
    if user.is_empty()
        && let Some((u, h)) = host.split_once('@')
        && !u.is_empty()
        && !h.is_empty()
        && !h.contains('\n')
    {
        (user, host) = (strip_space(u).to_owned(), strip_space(h).to_owned());
    }
    let label = truncate_chars(label, 120);
    let mut out = vec![(
        "label",
        sv(if label.is_empty() {
            host.clone()
        } else {
            label
        }),
    )];
    out.push(("host", sv(host)));
    if !user.is_empty() {
        out.push(("user", sv(user)));
    }
    if let Some(p) = port.filter(|p| *p != 22) {
        out.push(("port", Value::Int(p)));
    }
    if let Some(i) = identity {
        out.push(("identity_file", sv(i)));
    }
    if !group.is_empty() {
        out.push(("group", sv(truncate_chars(group, 80))));
    }
    out
}

type Kv = HashMap<String, Value>;

/// `str(kv.get(key) or "")`.
fn text_of(kv: &Kv, key: &str) -> String {
    kv.get(key)
        .filter(|v| truthy(v))
        .map(value_str)
        .unwrap_or_default()
}

/// `_flag`: a non-zero integer.
fn flag(kv: &Kv, key: &str) -> bool {
    kv.get(key)
        .and_then(|v| value_int(&Value::Str(value_str(v))))
        .is_some_and(|n| n != 0)
}

/// `_putty_extras`: tunnels, jump host, remote command, X11 and agent forwarding.
fn putty_extras(kv: &Kv) -> Vec<(&'static str, Value)> {
    let (mut dynamic, mut local, mut remote) = (None, Vec::new(), Vec::new());
    for item in text_of(kv, "PortForwardings").split(',') {
        let mut item = strip_space(item);
        // IPv4/IPv6-only marker before the type letter.
        if let Some(rest) = item.strip_prefix(['4', '6']) {
            item = rest;
        }
        let mut chars = item.chars();
        let kind = chars.next().map(|c| c.to_ascii_uppercase());
        let spec = chars.as_str();
        let (src, dst) = spec.split_once('=').unwrap_or((spec, ""));
        match kind {
            Some('L') if !src.is_empty() && !dst.is_empty() => {
                local.push(sv(format!("{src}:{dst}")));
            }
            Some('R') if !src.is_empty() && !dst.is_empty() => {
                remote.push(sv(format!("{src}:{dst}")));
            }
            Some('D') if !src.is_empty() && dynamic.is_none() => dynamic = Some(sv(src)),
            _ => {}
        }
    }
    let mut out = Vec::new();
    if let Some(d) = dynamic {
        out.push(("dynamic_forward", d));
    }
    if !local.is_empty() {
        out.push(("local_forwards", Value::Seq(local)));
    }
    if !remote.is_empty() {
        out.push(("remote_forwards", Value::Seq(remote)));
    }
    let rc = text_of(kv, "RemoteCommand");
    if !strip_space(&rc).is_empty() {
        out.push(("remote_command", sv(strip_space(&rc))));
    }
    if flag(kv, "X11Forward") {
        out.push(("x11_forwarding", Value::Bool(true)));
    }
    if flag(kv, "AgentFwd") {
        out.push(("forward_agent", Value::Bool(true)));
    }
    let method = value_int(&Value::Str(text_of_or(kv, "ProxyMethod", "0"))).unwrap_or(0);
    let proxy = text_of(kv, "ProxyHost");
    let proxy = strip_space(&proxy);
    if method == PROXY_SSH && !proxy.is_empty() {
        let user = text_of(kv, "ProxyUsername");
        let user = strip_space(&user);
        let hop = if user.is_empty() {
            proxy.to_owned()
        } else {
            format!("{user}@{proxy}")
        };
        let port = value_int(&Value::Str(text_of_or(kv, "ProxyPort", "22"))).unwrap_or(22);
        out.push((
            "proxy_jump",
            sv(if port == 22 {
                hop
            } else {
                format!("{hop}:{port}")
            }),
        ));
    }
    out
}

/// `str(kv.get(key) or default)`.
fn text_of_or(kv: &Kv, key: &str, default: &str) -> String {
    let t = text_of(kv, key);
    if t.is_empty() { default.to_owned() } else { t }
}

/// One `~/.putty/sessions/<name>` file (`Key=Value` lines) as a row; `None` if not SSH or
/// without a host. `name` is the URL-encoded file name.
pub fn parse_putty_session_file(name: &str, text: &str, local_user: &str) -> Option<Value> {
    let mut kv = Kv::new();
    for line in splitlines(text) {
        if let Some((k, v)) = line.split_once('=') {
            kv.insert(strip_space(k).to_owned(), sv(strip_space(v)));
        }
    }
    let get = |k: &str| match kv.get(k) {
        Some(Value::Str(s)) => Some(s.as_str()),
        _ => None,
    };
    if get("Protocol").unwrap_or("ssh").to_lowercase() != "ssh" {
        return None;
    }
    let host = get("HostName").unwrap_or_default();
    if host.is_empty() {
        return None;
    }
    let port = value_int(&Value::Str(get("PortNumber").unwrap_or("22").to_owned()));
    let mut r = make_row(
        &unquote(name),
        host,
        get("UserName").unwrap_or_default(),
        port,
        identity_from_putty(get("PublicKeyFile").unwrap_or_default(), local_user),
        "",
    );
    r.extend(putty_extras(&kv));
    Some(row(r))
}

/// Up to [`MAX_FILE_BYTES`] of a file, `None` if unreadable or larger.
pub(crate) fn read_limited(path: &Path) -> Option<Vec<u8>> {
    let mut buf = Vec::new();
    std::fs::File::open(path)
        .ok()?
        .take(MAX_FILE_BYTES + 1)
        .read_to_end(&mut buf)
        .ok()?;
    (buf.len() as u64 <= MAX_FILE_BYTES).then_some(buf)
}

/// `parse_putty_sessions_dir`: every regular file of `dir` in name order except
/// `Default%20Settings`; unreadable or oversized files are skipped, a missing dir gives `[]`.
pub fn parse_putty_sessions_dir(dir: &Path, local_user: &str) -> Vec<Value> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut names: Vec<std::ffi::OsString> = entries.flatten().map(|e| e.file_name()).collect();
    names.sort();
    names
        .iter()
        .filter(|n| *n != "Default%20Settings")
        .filter_map(|n| {
            let path = dir.join(n);
            if !std::fs::metadata(&path).ok()?.is_file() {
                return None;
            }
            let data = read_limited(&path)?;
            parse_putty_session_file(
                &n.to_string_lossy(),
                &String::from_utf8_lossy(&data),
                local_user,
            )
        })
        .collect()
}

/// `[HKEY_…\PuTTY\Sessions\<name>]` (case-insensitive) → the still URL-encoded name.
fn reg_section(line: &str) -> Option<&str> {
    let head = line.get(..6).filter(|h| h.eq_ignore_ascii_case("[HKEY_"))?;
    let body = trim_space_end(&line[head.len()..]).strip_suffix(']')?;
    // The prefix may not contain `]`; the last marker with a name after it wins.
    let region = body.find(']').unwrap_or(body.len());
    const MARK: &[u8] = b"\\PuTTY\\Sessions\\";
    let bytes = body.as_bytes();
    (0..=region.saturating_sub(MARK.len()))
        .rev()
        .find(|&i| {
            bytes
                .get(i..i + MARK.len())
                .is_some_and(|w| w.eq_ignore_ascii_case(MARK))
                && i + MARK.len() <= region
                && i + MARK.len() < body.len()
        })
        .map(|i| &body[i + MARK.len()..])
}

fn trim_space_end(s: &str) -> &str {
    s.trim_end_matches(|c: char| c.is_whitespace() || ('\x1c'..='\x1f').contains(&c))
}

/// `"key"="value"` → `(key, raw value)`.
fn reg_string(line: &str) -> Option<(&str, &str)> {
    let rest = line.strip_prefix('"')?;
    let k = rest.find('"').filter(|k| *k > 0)?;
    let value = trim_space_end(rest[k + 1..].strip_prefix("=\"")?).strip_suffix('"')?;
    Some((&rest[..k], value))
}

/// `"key"=dword:hex` (1–8 hex digits) → `(key, value)`.
fn reg_dword(line: &str) -> Option<(&str, i64)> {
    let rest = line.strip_prefix('"')?;
    let k = rest.find('"').filter(|k| *k > 0)?;
    let hex = trim_space_end(rest[k + 1..].strip_prefix("=dword:")?);
    if hex.is_empty() || hex.len() > 8 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    Some((&rest[..k], i64::from_str_radix(hex, 16).ok()?))
}

fn reg_row(name: &str, kv: &Kv, local_user: &str) -> Option<Value> {
    let protocol = if kv.contains_key("Protocol") {
        value_str(&kv["Protocol"])
    } else {
        "ssh".to_owned()
    };
    if protocol.to_lowercase() != "ssh" {
        return None;
    }
    let host = text_of(kv, "HostName");
    let host = strip_space(&host);
    if host.is_empty() {
        return None;
    }
    let port = match kv.get("PortNumber") {
        Some(Value::Int(p)) => Some(*p),
        _ => None,
    };
    let user = text_of(kv, "UserName");
    let mut r = make_row(
        &unquote(name),
        host,
        strip_space(&user),
        port,
        identity_from_putty(&text_of(kv, "PublicKeyFile"), local_user),
        "",
    );
    r.extend(putty_extras(kv));
    Some(row(r))
}

/// `parse_putty_reg`: a Windows `.reg` export of PuTTY sessions as rows.
pub fn parse_putty_reg(text: &str, local_user: &str) -> Vec<Value> {
    let mut out = Vec::new();
    let mut current: Option<(String, Kv)> = None;
    let mut flush = |cur: &mut Option<(String, Kv)>| {
        if let Some((name, kv)) = cur.take()
            && let Some(r) = reg_row(&name, &kv, local_user)
        {
            out.push(r);
        }
    };
    for line in splitlines(text) {
        let line = line.trim_matches(['\u{feff}', '\r', '\n']);
        if let Some(name) = reg_section(line) {
            flush(&mut current);
            current = Some((name.to_owned(), Kv::new()));
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') {
            flush(&mut current);
            continue;
        }
        let Some((_, kv)) = current.as_mut() else {
            continue;
        };
        if let Some((k, v)) = reg_string(line) {
            kv.insert(k.to_owned(), sv(v.replace("\\\\", "\\")));
        } else if let Some((k, n)) = reg_dword(line) {
            kv.insert(k.to_owned(), Value::Int(n));
        }
    }
    flush(&mut current);
    out
}

/// XML attribute value as expat gives it: line ends and tabs become spaces, predefined and
/// character entities are resolved; anything else is an error.
fn attr_value(raw: &str) -> Option<String> {
    let norm = raw.replace("\r\n", " ").replace(['\r', '\n', '\t'], " ");
    quick_xml::escape::unescape(&norm)
        .ok()
        .map(std::borrow::Cow::into_owned)
}

fn session_row(e: &BytesStart<'_>) -> Result<Option<Value>, ()> {
    if !e.local_name().as_ref().ends_with("SessionData") {
        return Ok(None);
    }
    let mut a: HashMap<String, String> = HashMap::new();
    for attr in e.attributes() {
        let attr = attr.map_err(|_| ())?;
        let key = attr.key.as_ref().to_owned();
        a.insert(key, attr_value(&attr.value).ok_or(())?);
    }
    let get = |k: &str| a.get(k).map_or("", String::as_str);
    let proto = get("Proto");
    let proto = if proto.is_empty() { "SSH" } else { proto };
    if !["SSH", "SSH2"].contains(&proto.to_uppercase().as_str()) {
        return Ok(None);
    }
    let host = strip_space(get("Host"));
    if host.is_empty() {
        return Ok(None);
    }
    let session_id = get("SessionId").replace('\\', "/");
    let session_id = session_id.trim_matches('/');
    let name = strip_space(get("SessionName"));
    let label = if !name.is_empty() {
        name
    } else if session_id.is_empty() {
        host
    } else {
        session_id.rsplit('/').next().unwrap_or(host)
    };
    let group = session_id.rsplit_once('/').map_or("", |(g, _)| g);
    let port = get("Port");
    let port = value_int(&Value::Str(
        if port.is_empty() { "22" } else { port }.to_owned(),
    ));
    Ok(Some(row(make_row(
        label,
        host,
        strip_space(get("Username")),
        port,
        None,
        group,
    ))))
}

/// `parse_superputty_xml`: SuperPuTTY `sessions.xml` as rows (folders → groups); `[]` for
/// malformed XML.
pub fn parse_superputty_xml(text: &str) -> Vec<Value> {
    xml_rows(text.trim_start_matches('\u{feff}')).unwrap_or_default()
}

fn xml_rows(text: &str) -> Result<Vec<Value>, ()> {
    let mut reader = quick_xml::Reader::from_str(text);
    let (mut out, mut depth, mut roots) = (Vec::new(), 0usize, 0usize);
    loop {
        match reader.read_event().map_err(|_| ())? {
            Event::Start(e) => {
                if depth == 0 {
                    roots += 1;
                }
                depth += 1;
                out.extend(session_row(&e)?);
            }
            Event::Empty(e) => {
                if depth == 0 {
                    roots += 1;
                }
                out.extend(session_row(&e)?);
            }
            Event::End(_) => depth = depth.checked_sub(1).ok_or(())?,
            Event::Text(t)
                if depth == 0 && !t.as_ref().bytes().all(|b| b.is_ascii_whitespace()) =>
            {
                return Err(());
            }
            Event::Eof => break,
            _ => {}
        }
        if roots > 1 {
            return Err(());
        }
    }
    if depth != 0 || roots == 0 {
        return Err(());
    }
    Ok(out)
}

/// `read_text_guess`: UTF-16 when the file starts with a BOM (invalid data replaced), else
/// UTF-8 with replacement.
pub fn decode_text_guess(raw: &[u8]) -> String {
    if let Some(rest) = raw.strip_prefix(&[0xff, 0xfe]) {
        return encoding_rs::UTF_16LE
            .decode_without_bom_handling(rest)
            .0
            .into_owned();
    }
    if let Some(rest) = raw.strip_prefix(&[0xfe, 0xff]) {
        return encoding_rs::UTF_16BE
            .decode_without_bom_handling(rest)
            .0
            .into_owned();
    }
    String::from_utf8_lossy(raw).into_owned()
}

/// `parse_putty_export`: a picked file as `.reg` (by extension or header) or SuperPuTTY XML;
/// unreadable, oversized or unrecognised files give `[]`.
pub fn parse_putty_export(path: &Path, local_user: &str) -> Vec<Value> {
    let Some(raw) = read_limited(path) else {
        return Vec::new();
    };
    let text = decode_text_guess(&raw);
    let head: String = text.chars().take(200).collect();
    let is_reg = path
        .extension()
        .is_some_and(|e| e.to_string_lossy().to_lowercase() == "reg");
    if is_reg || head.contains("Windows Registry Editor") {
        parse_putty_reg(&text, local_user)
    } else if head.contains('<') {
        parse_superputty_xml(&text)
    } else {
        Vec::new()
    }
}
