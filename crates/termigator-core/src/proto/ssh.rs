//! ssh and mosh command lines, guards, keepalive, ControlMaster, secrets via env (scp, konsole
//! and xdotool have their own modules).
//!
//! [`build_ssh_command`] turns a connection mapping into the exact shell line the app
//! builds (argument order, quoting through [`crate::shell::join`], error texts). The pieces
//! scp reuses ([`conn_port`], [`identity_flags`], [`normalize_proxy_jump_hops`],
//! [`ssh_target_guard`], [`quote_repr`]) are public. Secrets never reach argv: `sshpass -e` reads
//! `$SSHPASS` ([`ssh_password_env`] refuses sealed `enc:` tokens), clipboard copies are masked
//! ([`clipboard_ssh_command`]).
//!
//! Anything that touches the machine is injectable through [`Env`] (binary lookup like
//! `shutil.which`, the ControlPath directory, `$HOME`), so tests are deterministic.
//! Golden fixtures: `compat/golden/ssh_*.json`.
//!
//! Known limits (none occur with sane input):
//! - `~user` in `identity_file` is not expanded (only `~` and `~/...`, like `paths.rs`).
//! - Digits for ports are ASCII only.
//! - `str()` of a nested list/mapping inside a value is `<container>`, not a full repr.
//! - [`quote_repr`] knows the common non-printable characters, not the whole Unicode database.

use std::path::Path;

use regex::Regex;

use crate::config::doc::Value;
use crate::config::model::{proxy_jump_value, strip_space, truthy, value_int, value_str};
use crate::paths::private_runtime_dir;
use crate::proto::askpass;
use crate::shell::{join, quote, split};
use crate::text::osc7::tracking_remote_command;
use crate::vault::{SEALED_PREFIX, Secret, VaultError};

/// Keys fully owned by the connection dialog; other keys are preserved on save.
pub const SSH_DIALOG_MANAGED_KEYS: &[&str] = &[
    "label",
    "host",
    "user",
    "password",
    "identity_file",
    "port",
    "extra_ssh_args",
    "terminal",
    "group",
    "proxy_jump",
    "jumphost",
    "jump_host",
    "forward_agent",
    "x11_forwarding",
    "local_forwards",
    "remote_forwards",
    "dynamic_forward",
    "remote_command",
    "strict_host_key_checking",
    "use_mosh",
    "auto_reconnect",
    "tags",
    "credential",
    "x11_trusted",
    "files_only",
    "compression",
    "request_tty",
    "remote_locale",
    "ssh_keepalive",
    "startup_commands",
    "font_family",
    "font_size",
    "color_scheme",
];

/// A `shutil.which` stand-in: name to resolved path.
pub type WhichFn = Box<dyn Fn(&str) -> Option<String>>;

/// What the builders ask the machine: `shutil.which`, the ControlPath directory, `$HOME`.
pub struct Env {
    /// `shutil.which(name)`: the resolved path or `None`.
    pub which: WhichFn,
    /// Private directory for ControlMaster sockets ([`ssh_control_dir`]).
    pub control_dir: Option<String>,
    /// Home directory for `~` in `identity_file`.
    pub home: Option<String>,
}

impl Env {
    /// The real machine: `$PATH`, [`private_runtime_dir`], `$HOME`.
    pub fn system() -> Env {
        Env {
            which: Box::new(|name| which(name, std::env::var("PATH").ok().as_deref())),
            // Win32-OpenSSH has no ControlMaster.
            control_dir: if cfg!(windows) {
                None
            } else {
                ssh_control_dir()
            },
            home: std::env::var("HOME")
                .ok()
                .or_else(|| std::env::home_dir().map(|p| p.to_string_lossy().into_owned())),
        }
    }
}

/// `shutil.which(cmd)` for an executable regular file, searching `path` (None: `/bin:/usr/bin`).
pub fn which(cmd: &str, path: Option<&str>) -> Option<String> {
    let usable = |p: &Path| crate::platform::is_executable(p);
    if cmd.contains('/') {
        return usable(Path::new(cmd)).then(|| cmd.to_owned());
    }
    let path = path.unwrap_or("/bin:/usr/bin");
    // Windows: `;`-separated PATH and PATHEXT extensions (`ssh` -> `ssh.exe`).
    let (sep, names) = if cfg!(windows) {
        let ext = std::env::var("PATHEXT").unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".to_owned());
        (';', crate::platform::pathext_names(cmd, &ext))
    } else {
        (':', vec![cmd.to_owned()])
    };
    let mut seen = Vec::new();
    for dir in path.split(sep) {
        if seen.contains(&dir) {
            continue;
        }
        seen.push(dir);
        for name in &names {
            let full = Path::new(dir).join(name);
            if usable(&full) {
                return Some(full.to_string_lossy().into_owned());
            }
        }
    }
    None
}

/// Quotes a string in single quotes (double quotes when it holds `'` but no `"`), escaping
/// backslashes, control characters and the quote.
pub fn quote_repr(s: &str) -> String {
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
            c if printable(c) => out.push(c),
            c if (c as u32) < 0x100 => out.push_str(&format!("\\x{:02x}", c as u32)),
            c if (c as u32) < 0x10000 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push_str(&format!("\\U{:08x}", c as u32)),
        }
    }
    out.push(quote);
    out
}

/// `str.isprintable` for one character (controls, separators, format and private-use characters
/// that matter in practice are not printable).
fn printable(c: char) -> bool {
    let u = c as u32;
    if c == ' ' {
        return true;
    }
    !(c.is_control()
        || c.is_whitespace()
        || matches!(u, 0xAD | 0x600..=0x605 | 0x61C | 0x6DD | 0x70F | 0x180E
            | 0x200B..=0x200F | 0x202A..=0x202E | 0x2060..=0x2064 | 0x2066..=0x206F
            | 0xE000..=0xF8FF | 0xFEFF | 0xFFF9..=0xFFFB | 0xE0001 | 0xE0020..=0xE007F
            | 0xF0000..=0x10FFFF))
}

/// Splits on line boundaries (`\n`, `\r\n`, `\r`, VT, FF, FS/GS/RS, NEL, LS/PS); a trailing
/// break adds no empty line.
fn split_lines(s: &str) -> Vec<&str> {
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

/// `str(conn.get(key) or "")`.
fn text(v: Option<&Value>) -> String {
    match v {
        Some(x) if truthy(x) => value_str(x),
        _ => String::new(),
    }
}

/// `str(conn.get(key) or "").strip()`.
fn stripped(v: Option<&Value>) -> String {
    strip_space(&text(v)).to_owned()
}

fn is_true(v: Option<&Value>) -> bool {
    matches!(v, Some(Value::Bool(true)))
}

/// `" ".join(cmd)`, used by the substring checks done on the command so far.
fn joined(cmd: &[String]) -> String {
    cmd.join(" ")
}

/// Directory for OpenSSH ControlMaster sockets (private, short); `None` without one.
pub fn ssh_control_dir() -> Option<String> {
    private_runtime_dir(&[]).map(|d| d.to_string_lossy().into_owned())
}

/// `-o` flags sharing one authenticated connection per destination; empty without a directory.
pub fn ssh_control_opts(control_dir: Option<&str>) -> Vec<String> {
    let Some(d) = control_dir else {
        return Vec::new();
    };
    [
        "-o",
        "ControlMaster=auto",
        "-o",
        &format!("ControlPath={d}/cm-%C"),
        "-o",
        "ControlPersist=60",
    ]
    .map(str::to_owned)
    .to_vec()
}

/// `-o` flags that make ssh give up on a dead connection after about `dead_after_seconds`
/// (the budget split three ways, interval at least 1).
pub fn keepalive_ssh_opts(dead_after_seconds: i64) -> Vec<String> {
    let count_max = 3;
    // round(n / 3) for n > 0: the fraction is never exactly one half.
    let interval = if dead_after_seconds <= 0 {
        1
    } else {
        ((dead_after_seconds + 1) / 3).max(1)
    };
    vec![
        "-o".to_owned(),
        format!("ServerAliveInterval={interval}"),
        "-o".to_owned(),
        format!("ServerAliveCountMax={count_max}"),
    ]
}

/// local_forwards / remote_forwards from YAML: a list, a multiline string or one string.
pub fn forward_specs_from_yaml(val: Option<&Value>) -> Vec<String> {
    let keep = |s: &str| {
        let t = strip_space(s);
        (!t.is_empty()).then(|| t.to_owned())
    };
    match val {
        None | Some(Value::Null) => Vec::new(),
        Some(Value::Seq(items)) => items.iter().filter_map(|x| keep(&value_str(x))).collect(),
        Some(v) => {
            let s = strip_space(&value_str(v)).to_owned();
            if s.is_empty() {
                Vec::new()
            } else if s.contains('\n') {
                split_lines(&s).into_iter().filter_map(keep).collect()
            } else {
                vec![s]
            }
        }
    }
}

/// Guard 1: a value that becomes an ssh option argument must not be empty or start with `-`.
pub fn ssh_arg_injection_guard(s: &str) -> Option<&'static str> {
    let t = strip_space(s);
    if t.is_empty() {
        return Some("empty value");
    }
    if t.starts_with('-') {
        return Some("must not start with '-' (pass options via Extra ssh arguments if needed)");
    }
    None
}

/// Guard 2: `user@host` is positional, so neither part may look like an option, hold
/// whitespace or control characters; the host has no `@`.
pub fn ssh_target_guard(user: &str, host: &str) -> Option<String> {
    for (name, value) in [("user", user), ("host", host)] {
        if let Some(err) = ssh_arg_injection_guard(value) {
            return Some(format!("{name} {}: {err}", quote_repr(value)));
        }
        if value
            .chars()
            .any(|c| c.is_whitespace() || (c as u32) < 32 || c as u32 == 127)
        {
            return Some(format!(
                "{name} {}: must not contain spaces or control characters",
                quote_repr(value)
            ));
        }
    }
    if host.contains('@') {
        return Some(format!(
            "host {}: must not contain '@' (put the user name in the User field)",
            quote_repr(host)
        ));
    }
    None
}

/// The jump host as the connection dialog shows it (comma-separated).
pub fn format_proxy_jump_for_ui(conn: &Value) -> String {
    match proxy_jump_value(conn) {
        None | Some(Value::Null) => String::new(),
        Some(Value::Seq(items)) => items
            .iter()
            .map(|x| strip_space(&value_str(x)).to_owned())
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join(","),
        Some(v) => strip_space(&value_str(v)).to_owned(),
    }
}

/// Jump hops from `proxy_jump` (falling back to `jumphost` / `jump_host`), guarded.
pub fn normalize_proxy_jump_hops(conn: &Value) -> Result<Vec<String>, String> {
    let primary = conn.get("proxy_jump");
    let blank = match primary {
        None | Some(Value::Null) => true,
        Some(Value::Str(s)) => strip_space(s).is_empty(),
        Some(_) => false,
    };
    let raw = if blank {
        match conn.get("jumphost").filter(|v| truthy(v)) {
            Some(v) => Some(v),
            None => conn.get("jump_host"),
        }
    } else {
        primary
    };
    let clean = |s: &str| {
        let t = strip_space(s);
        (!t.is_empty()).then(|| t.to_owned())
    };
    let hops: Vec<String> = match raw {
        None | Some(Value::Null) => return Ok(Vec::new()),
        Some(Value::Seq(items)) => items.iter().filter_map(|x| clean(&value_str(x))).collect(),
        Some(v) => value_str(v).split(',').filter_map(clean).collect(),
    };
    for h in &hops {
        if let Some(err) = ssh_arg_injection_guard(h) {
            return Err(format!("proxy_jump / jumphost {}: {err}", quote_repr(h)));
        }
        // A hop is `[user@]host[:port]` and never holds whitespace; with it,
        // `x -oProxyCommand=…` can turn into an ssh option once mosh's `--ssh=` command (or
        // the jump command ssh builds from `-J`) is split into words again.
        if h.chars()
            .any(|c| c.is_whitespace() || (c as u32) < 32 || c as u32 == 127)
        {
            return Err(format!(
                "proxy_jump / jumphost {}: must not contain spaces or control characters",
                quote_repr(h)
            ));
        }
    }
    Ok(hops)
}

/// Guard 3: `^[A-Za-z][A-Za-z0-9_.@-]{0,63}$`.
fn valid_locale(s: &str) -> bool {
    let mut chars = s.chars();
    chars.next().is_some_and(|c| c.is_ascii_alphabetic())
        && s.len() <= 64
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '@' | '-'))
}

fn append_devops_flags(conn: &Value, cmd: &mut Vec<String>) -> Result<(), String> {
    let hops = normalize_proxy_jump_hops(conn)?;
    if !hops.is_empty() {
        cmd.extend(["-J".to_owned(), hops.join(",")]);
    }
    for (key, flag) in [("local_forwards", "-L"), ("remote_forwards", "-R")] {
        for spec in forward_specs_from_yaml(conn.get(key)) {
            if let Some(e) = ssh_arg_injection_guard(&spec) {
                return Err(format!("{key} {}: {e}", quote_repr(&spec)));
            }
            cmd.extend([flag.to_owned(), spec]);
        }
    }

    if let Some(df) = conn.get("dynamic_forward")
        && !matches!(df, Value::Null | Value::Bool(_))
    {
        let ds = strip_space(&value_str(df)).to_owned();
        if !ds.is_empty() && !["none", "false", "no", "off"].contains(&ds.to_lowercase().as_str()) {
            let bind = if ds.bytes().all(|b| b.is_ascii_digit()) {
                match ds.parse::<u64>() {
                    Ok(p @ 1..=65535) => format!("127.0.0.1:{p}"),
                    _ => return Err("dynamic_forward: port must be 1–65535.".to_owned()),
                }
            } else {
                if let Some(e) = ssh_arg_injection_guard(&ds) {
                    return Err(format!("dynamic_forward {}: {e}", quote_repr(&ds)));
                }
                ds
            };
            cmd.extend(["-D".to_owned(), bind]);
        }
    }

    if is_true(conn.get("forward_agent")) && !joined(cmd).contains("ForwardAgent") {
        cmd.extend(["-o".to_owned(), "ForwardAgent=yes".to_owned()]);
    }
    if is_true(conn.get("x11_forwarding")) && !cmd.iter().any(|a| a == "-X" || a == "-Y") {
        // -Y (trusted) skips the X SECURITY extension; opt-in per entry.
        let flag = if is_true(conn.get("x11_trusted")) {
            "-Y"
        } else {
            "-X"
        };
        cmd.push(flag.to_owned());
    }
    if is_true(conn.get("compression"))
        && !cmd.iter().any(|a| a == "-C")
        && !joined(cmd).contains("Compression")
    {
        // -o form: OpenSSH 10 reports -C as "compression UNKNOWN" in `ssh -G`.
        cmd.extend(["-o".to_owned(), "Compression=yes".to_owned()]);
    }

    let loc = stripped(conn.get("remote_locale"));
    if !loc.is_empty() {
        if !valid_locale(&loc) {
            return Err(format!(
                "remote_locale {}: use a locale name such as en_US.UTF-8 or pl_PL.UTF-8.",
                quote_repr(&loc)
            ));
        }
        // The value travels in the spawn env (ssh_session_env); SendEnv makes ssh pass it.
        cmd.extend(["-o".to_owned(), "SendEnv=LANG LC_ALL".to_owned()]);
    }

    if let Some(shk) = conn.get("strict_host_key_checking") {
        let val = strip_space(&value_str(shk)).to_owned();
        if !matches!(shk, Value::Null) && !val.is_empty() {
            if !["yes", "no", "ask", "accept-new", "off"].contains(&val.as_str()) {
                return Err(format!(
                    "strict_host_key_checking: use one of accept-new, ask, no, off, yes (got {}).",
                    quote_repr(&val)
                ));
            }
            if !joined(cmd).contains("StrictHostKeyChecking") {
                cmd.extend(["-o".to_owned(), format!("StrictHostKeyChecking={val}")]);
            }
        }
    }
    Ok(())
}

/// Validated port from a connection (`None` when unset or the default 22).
pub fn conn_port(conn: &Value) -> Result<Option<u16>, String> {
    let Some(port) = conn.get("port") else {
        return Ok(None);
    };
    if matches!(port, Value::Null) || strip_space(&value_str(port)).is_empty() {
        return Ok(None);
    }
    let p = value_int(port).ok_or("Invalid port.")?;
    if !(1..=65535).contains(&p) {
        return Err("Port must be between 1 and 65535.".to_owned());
    }
    Ok(u16::try_from(p).ok().filter(|&p| p != 22))
}

/// `os.path.expanduser` for `~` and `~/...` (`~user` is left alone).
pub fn expand_user(path: &str, home: Option<&str>) -> String {
    expand_user_with(path, home, cfg!(windows))
}

/// [`expand_user`] with `backslash` = also accept `~\` (Windows paths).
pub fn expand_user_with(path: &str, home: Option<&str>, backslash: bool) -> String {
    let Some(rest) = path.strip_prefix('~') else {
        return path.to_owned();
    };
    if !(rest.is_empty() || rest.starts_with('/') || (backslash && rest.starts_with('\\'))) {
        return path.to_owned();
    }
    let Some(home) = home else {
        return path.to_owned();
    };
    let home = if backslash {
        home.trim_end_matches(['/', '\\'])
    } else {
        home.trim_end_matches('/')
    };
    let out = format!("{home}{rest}");
    if out.is_empty() { "/".to_owned() } else { out }
}

/// What a key file holds, judged by its content (a name ending in `.pub` proves nothing).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyKind {
    Private,
    Public,
    /// A PuTTY `.ppk`; OpenSSH cannot read it.
    Putty,
    Unknown,
}

/// Classifies the first 4 KiB of `path`. An unreadable file is [`KeyKind::Unknown`].
pub fn classify_key_file(path: &Path) -> KeyKind {
    use std::io::Read;
    let mut head = Vec::new();
    let read = std::fs::File::open(path).and_then(|f| f.take(4096).read_to_end(&mut head));
    if read.is_err() {
        return KeyKind::Unknown;
    }
    let text = String::from_utf8_lossy(&head);
    let first = text
        .trim_start_matches('\u{feff}')
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("");
    if first.starts_with("PuTTY-User-Key-File-") {
        KeyKind::Putty
    } else if first.starts_with("-----BEGIN") && first.contains("PRIVATE KEY") {
        KeyKind::Private
    } else if first.starts_with("-----BEGIN PUBLIC KEY")
        || first.starts_with("---- BEGIN SSH2 PUBLIC KEY")
        || ["ssh-", "ecdsa-sha2-", "sk-ssh-", "sk-ecdsa-"]
            .iter()
            .any(|p| first.starts_with(p))
    {
        KeyKind::Public
    } else {
        KeyKind::Unknown
    }
}

/// A warning (the connection still goes ahead) when `identity_file` does not look like a private
/// key OpenSSH can use. `None` when it is fine, unset or not a readable file.
pub fn identity_warning(conn: &Value, home: Option<&str>) -> Option<String> {
    let identity = stripped(conn.get("identity_file"));
    if identity.is_empty() {
        return None;
    }
    let key_path = expand_user(&identity, home);
    let path = Path::new(&key_path);
    if !path.is_file() {
        return None;
    }
    match classify_key_file(path) {
        KeyKind::Public => Some(format!(
            "The SSH key {} looks like a public key, not a private one; the login will probably fail.",
            quote_repr(&key_path)
        )),
        KeyKind::Putty => Some(format!(
            "The SSH key {} is a PuTTY .ppk file, which OpenSSH cannot read; convert it with puttygen.",
            quote_repr(&key_path)
        )),
        KeyKind::Private if crate::platform::key_too_open(path) => Some(format!(
            "The SSH key {} can be read by other users, so OpenSSH would refuse it; Termigator \
                 passes ssh a private copy. To fix the file itself: {}.",
            quote_repr(&key_path),
            if cfg!(windows) {
                "Properties → Security, leave only your account"
            } else {
                "chmod 600 on it"
            }
        )),
        KeyKind::Private | KeyKind::Unknown => None,
    }
}

/// For a private key OpenSSH would refuse as readable by others ([`crate::platform::key_too_open`]): a
/// copy only this user can read, in the private runtime directory, so the login works like it
/// does in clients with their own ssh (MobaXterm, PuTTY). The user's file is left as it is. A
/// `<key>-cert.pub` next to it is copied too, as ssh looks for it beside the `-i` path. `None`
/// when the key is fine or no copy can be made (ssh then reports the problem itself).
fn private_key_copy(path: &Path) -> Option<String> {
    use std::hash::{Hash, Hasher};
    if classify_key_file(path) != KeyKind::Private || !crate::platform::key_too_open(path) {
        return None;
    }
    let bytes = std::fs::read(path).ok()?;
    let dir = private_runtime_dir(&["keys"])?;
    let mut h = std::collections::hash_map::DefaultHasher::new();
    path.hash(&mut h);
    let name = path.file_name()?.to_string_lossy();
    let copy = dir.join(format!("{name}-{:016x}", h.finish()));
    write_private_copy(&copy, &bytes).ok()?;
    let cert = |p: &Path| {
        let mut s = p.as_os_str().to_owned();
        s.push("-cert.pub");
        std::path::PathBuf::from(s)
    };
    if let Ok(c) = std::fs::read(cert(path)) {
        let _ = write_private_copy(&cert(&copy), &c);
    }
    Some(copy.to_string_lossy().into_owned())
}

/// Removes the private key copies of [`private_key_copy`] (the app calls it when it quits), so no
/// copy of a private key outlives the session on disk (`%TEMP%` on Windows, `/tmp` without
/// `$XDG_RUNTIME_DIR`).
pub fn remove_private_key_copies() {
    if let Some(dir) = private_runtime_dir(&["keys"]) {
        let _ = std::fs::remove_dir_all(dir);
    }
}

/// `bytes` into `path` (owner only), skipped when it already holds them.
fn write_private_copy(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    if std::fs::read(path).is_ok_and(|old| old == bytes) {
        return Ok(());
    }
    let mut f = crate::platform::private_file(
        std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true),
    )
    .open(path)?;
    f.write_all(bytes)?;
    crate::platform::set_private_file(path)
}

/// `-i <key> -o IdentitiesOnly=yes` for a validated `identity_file` (empty when unset).
pub fn identity_flags(conn: &Value, home: Option<&str>) -> Result<Vec<String>, String> {
    let identity = stripped(conn.get("identity_file"));
    if identity.is_empty() {
        return Ok(Vec::new());
    }
    let key_path = expand_user(&identity, home);
    let path = Path::new(&key_path);
    if !path.is_file() {
        return Err(format!(
            "SSH key file not found (not a regular file): {} → {}. \
             Use an absolute path or ~/.ssh/… and the private key, not .pub.",
            quote_repr(&identity),
            quote_repr(&key_path)
        ));
    }
    if !crate::platform::can_read(path) {
        return Err(format!(
            "SSH key file is not readable: {}",
            quote_repr(&key_path)
        ));
    }
    if path.metadata().is_ok_and(|m| m.len() == 0) {
        return Err(format!("SSH key file is empty: {}", quote_repr(&key_path)));
    }
    let key_path = private_key_copy(path).unwrap_or(key_path);
    // Only this identity, so ssh does not try many agent keys first ("Too many failures").
    Ok(vec![
        "-i".to_owned(),
        key_path,
        "-o".to_owned(),
        "IdentitiesOnly=yes".to_owned(),
    ])
}

fn requests_tty_already(cmd: &[String]) -> bool {
    cmd.iter()
        .any(|a| matches!(a.as_str(), "-t" | "-tt" | "-T") || a.contains("RequestTTY"))
}

const NO_SSHPASS: &str = "Password is set but sshpass is not in PATH \
                          (e.g. sudo apt install sshpass). Use an SSH key instead.";

/// The shell line for a connection (ssh, or mosh with `use_mosh`), on the real machine.
pub fn build_ssh_command(conn: &Value, launch: &Value) -> Result<String, String> {
    build_ssh_command_with(conn, launch, &Env::system())
}

/// [`build_ssh_command`] with an explicit [`Env`].
pub fn build_ssh_command_with(conn: &Value, launch: &Value, env: &Env) -> Result<String, String> {
    let host = stripped(conn.get("host"));
    let user = stripped(conn.get("user"));
    if host.is_empty() || user.is_empty() {
        return Err("Host or user is missing.".to_owned());
    }
    if let Some(e) = ssh_target_guard(&user, &host) {
        return Err(e);
    }

    let mut cmd = vec!["ssh".to_owned()];
    if let Some(p) = conn_port(conn)? {
        cmd.extend(["-p".to_owned(), p.to_string()]);
    }
    cmd.extend(identity_flags(conn, env.home.as_deref())?);

    // Per-connection ssh_keepalive overrides the launch default.
    let use_keepalive = match conn.get("ssh_keepalive") {
        Some(Value::Bool(b)) => *b,
        _ => launch.get("ssh_keepalive").is_none_or(truthy),
    };
    if use_keepalive && !joined(&cmd).contains("ServerAliveInterval") {
        let dead_after = launch
            .get("ssh_keepalive_seconds")
            .filter(|v| truthy(v))
            .and_then(value_int)
            .unwrap_or(15);
        cmd.extend(keepalive_ssh_opts(dead_after));
    }

    if let Some(extra) = conn.get("extra_ssh_args").filter(|v| truthy(v)) {
        match extra {
            Value::Seq(items) => cmd.extend(items.iter().map(value_str)),
            other => cmd.extend(
                split(&value_str(other))
                    .map_err(|_| "Could not parse extra_ssh_args.".to_owned())?,
            ),
        }
    }

    append_devops_flags(conn, &mut cmd)?;

    let target = format!("{user}@{host}");
    let mut rc = stripped(conn.get("remote_command"));
    if !rc.is_empty() && split(&rc).is_err() {
        // Only to reject unbalanced quotes early, with a clear message.
        return Err("Could not parse remote_command (shell quoting).".to_owned());
    }
    let has_password = !text(conn.get("password")).is_empty();

    if is_true(conn.get("use_mosh")) {
        if has_password {
            return Err(
                "Mosh entries cannot use a saved password (no sshpass path) — use an SSH key."
                    .to_owned(),
            );
        }
        if (env.which)("mosh").is_none() {
            return Err(
                "use_mosh is set but mosh is not in PATH (e.g. sudo pacman -S mosh).".to_owned(),
            );
        }
        let mut mosh = vec!["mosh".to_owned()];
        if cmd.len() > 1 {
            // mosh runs this ssh line only for the handshake; -L/-R/-D do not outlive it.
            mosh.push(format!("--ssh={}", join(&cmd)));
        }
        mosh.push(target);
        if !rc.is_empty() {
            // mosh-server execs without a shell: wrap so pipes, || and quoting work.
            mosh.extend(["--", "sh", "-c"].map(str::to_owned));
            mosh.push(rc);
        }
        return Ok(join(&mosh));
    }

    // `launch.track_remote_folder` (set by the terminal's callers, never by the golden configs): an
    // entry without a command of its own starts its shell through the OSC 7 bootstrap.
    if rc.is_empty() && launch.get("track_remote_folder").is_some_and(truthy) {
        rc = tracking_remote_command();
    }

    // The terminal session becomes the ControlMaster the Files panel piggybacks on.
    if launch.get("ssh_control_master").is_some_and(truthy) {
        let j = joined(&cmd);
        if !j.contains("ControlMaster") && !j.contains("ControlPath") {
            cmd.extend(ssh_control_opts(env.control_dir.as_deref()));
        }
    }

    if (!rc.is_empty() || is_true(conn.get("request_tty"))) && !requests_tty_already(&cmd) {
        // With a command ssh allocates no TTY; this is an interactive tab, so force one.
        cmd.push("-t".to_owned());
    }
    cmd.push(target);
    if !rc.is_empty() {
        // One argument, exactly as typed (the remote shell re-parses it).
        cmd.push(rc);
    }

    if has_password {
        // Windows has no sshpass, and through a jump host sshpass would answer the jump host's
        // prompt: ssh asks the askpass helper instead (see `ssh_password_env`).
        if !password_via_askpass(conn) {
            if (env.which)("sshpass").is_none() {
                return Err(NO_SSHPASS.to_owned());
            }
            // -e reads $SSHPASS from the child environment; -p would show in `ps`.
            cmd.splice(0..0, ["sshpass".to_owned(), "-e".to_owned()]);
        }
    }
    Ok(join(&cmd))
}

/// Environment for spawning a built command: `SSHPASS` for `sshpass -e`. A sealed token is
/// refused, never sent (the connection must be revealed first).
pub fn ssh_password_env(conn: &Value) -> Result<Vec<(String, Secret)>, VaultError> {
    let password = text(conn.get("password"));
    if password.starts_with(SEALED_PREFIX) {
        return Err(VaultError::Vault(
            "Password is still encrypted — unlock the vault first.".to_owned(),
        ));
    }
    if password.is_empty() {
        return Ok(Vec::new());
    }
    let password = Secret::new(password.into_bytes());
    if password_via_askpass(conn) {
        return askpass_env(&password, conn);
    }
    Ok(vec![("SSHPASS".to_owned(), password)])
}

/// Whether a saved password goes to ssh through the askpass helper rather than `sshpass -e`:
/// always on Windows (no sshpass), and with a jump host everywhere — sshpass answers the first
/// password prompt, which is the jump host's, so the target's password would reach it.
/// The helper answers only the target's own prompt there.
pub fn password_via_askpass(conn: &Value) -> bool {
    cfg!(windows) || normalize_proxy_jump_hops(conn).is_ok_and(|hops| !hops.is_empty())
}

/// `(major, minor)` of the local OpenSSH (`ssh -V`), read once; `None` when unknown.
fn local_openssh_version() -> Option<(u32, u32)> {
    static V: std::sync::OnceLock<Option<(u32, u32)>> = std::sync::OnceLock::new();
    *V.get_or_init(|| {
        let ssh = crate::platform::find_in_path("ssh")?;
        let mut cmd = std::process::Command::new(ssh);
        cmd.arg("-V");
        crate::platform::background_process(&mut cmd);
        let out = cmd.output().ok()?;
        // ssh -V writes to stderr.
        parse_openssh_version(&String::from_utf8_lossy(&out.stderr))
    })
}

/// `OpenSSH_9.6p1 Ubuntu…` or `OpenSSH_for_Windows_9.5p1, LibreSSL…` → `(9, 6)` / `(9, 5)`.
pub fn parse_openssh_version(text: &str) -> Option<(u32, u32)> {
    let rest = text.split("OpenSSH_").nth(1)?;
    let rest = rest.strip_prefix("for_Windows_").unwrap_or(rest);
    let (major, rest) = rest.split_once('.')?;
    let minor: String = rest.chars().take_while(char::is_ascii_digit).collect();
    Some((major.parse().ok()?, minor.parse().ok()?))
}

/// `SSH_ASKPASS` pointing at this executable plus the channel and token it dials; through a jump
/// host the token answers only `user@host` of the entry itself.
fn askpass_env(password: &Secret, conn: &Value) -> Result<Vec<(String, Secret)>, VaultError> {
    let unavailable = || {
        VaultError::Vault(
            "Saved passwords are not available here: the password helper could not start."
                .to_owned(),
        )
    };
    let jumps = normalize_proxy_jump_hops(conn).is_ok_and(|hops| !hops.is_empty());
    // Before 8.4 keyboard-interactive prompts carry no `(user@host)`, so a jump host could word
    // its prompt like the target's and get the password.
    if jumps && local_openssh_version().is_some_and(|v| v < (8, 4)) {
        return Err(VaultError::Vault(
            "A saved password through a jump host needs OpenSSH 8.4 or newer here — update \
             OpenSSH or use an SSH key."
                .to_owned(),
        ));
    }
    let target = jumps.then(|| askpass::Target {
        user: strip_space(&text(conn.get("user"))).to_owned(),
        host: strip_space(&text(conn.get("host"))).to_owned(),
    });
    let exe = std::env::current_exe().map_err(|_| unavailable())?;
    let env = askpass::env_for(password, target.as_ref(), &exe.to_string_lossy())
        .ok_or_else(unavailable)?;
    Ok(env
        .into_iter()
        .map(|(k, v)| (k, Secret::new(v.into_bytes())))
        .collect())
}

/// Non-secret per-entry environment: `remote_locale` as `LANG` / `LC_ALL`.
pub fn ssh_session_env(conn: &Value) -> Vec<(String, String)> {
    let loc = stripped(conn.get("remote_locale"));
    if loc.is_empty() || !valid_locale(&loc) {
        return Vec::new();
    }
    vec![("LANG".to_owned(), loc.clone()), ("LC_ALL".to_owned(), loc)]
}

/// X11 forwarding without a local display (`display` = `$DISPLAY`): a warning for the user.
pub fn x11_display_warning(conn: &Value, display: Option<&str>) -> Option<String> {
    (is_true(conn.get("x11_forwarding")) && strip_space(display.unwrap_or("")).is_empty()).then(
        || {
            "X11 forwarding is on, but DISPLAY is not set here — graphical programs on \
         the server will not be able to open windows (no local X server / XWayland)."
                .to_owned()
        },
    )
}

/// Clipboard variant of a command line: a masked `SSHPASS=` prefix instead of the password.
pub fn clipboard_ssh_command(cmd: &str, conn: &Value) -> String {
    if text(conn.get("password")).is_empty() {
        cmd.to_owned()
    } else {
        format!("SSHPASS='****' {cmd}")
    }
}

/// Local interactive shell as `<absolute exe> -l`: `launch.shell_executable`, then
/// `launch.shell`, then `$SHELL` (`shell_env`), then bash, then `/bin/sh`.
pub fn default_interactive_shell(
    cfg: &Value,
    shell_env: &str,
    which: &dyn Fn(&str) -> Option<String>,
) -> String {
    let empty = Value::Null;
    let lo = cfg.get("launch").filter(|v| truthy(v)).unwrap_or(&empty);
    let se = stripped(lo.get("shell_executable"));
    if !se.is_empty() && se.starts_with('/') && crate::platform::can_exec(Path::new(&se)) {
        return format!("{} -l", quote(&se));
    }
    for name in [
        stripped(lo.get("shell")),
        shell_env.to_owned(),
        "bash".to_owned(),
    ] {
        if !name.is_empty()
            && let Some(exe) = which(&name)
        {
            return format!("{} -l", quote(&exe));
        }
    }
    "/bin/sh -l".to_owned()
}

/// Listed in /etc/shells but not interactive shells to offer in the "+" menu.
const NOT_SHELLS: &[&str] = &["rbash", "screen", "tmux", "nologin", "false", "git-shell"];

/// Login shells from the text of /etc/shells: executable, one per name.
pub fn available_shells_from(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut seen: Vec<&str> = Vec::new();
    for line in split_lines(text) {
        let path = strip_space(line.split('#').next().unwrap_or(""));
        let name = path.rsplit('/').next().unwrap_or("");
        if !path.starts_with('/') || seen.contains(&name) || NOT_SHELLS.contains(&name) {
            continue;
        }
        let p = Path::new(path);
        if crate::platform::is_executable(p) {
            seen.push(name);
            out.push(path.to_owned());
        }
    }
    out
}

/// [`available_shells_from`] the file at `shells_file` (usually /etc/shells); `[]` if unreadable.
pub fn available_shells(shells_file: &str) -> Vec<String> {
    std::fs::read(shells_file)
        .map(|b| available_shells_from(&String::from_utf8_lossy(&b)))
        .unwrap_or_default()
}

/// Result of [`parse_quick_ssh_target`]; `host` is bare (no brackets).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuickTarget {
    pub user: String,
    pub host: String,
    pub port: Option<u16>,
}

/// Parse `user@host[:port]` for Quick SSH. IPv6: `user@[2001:db8::1]:port` (brackets with a
/// port) or bare `user@::1` (no port).
pub fn parse_quick_ssh_target(line: &str) -> Result<QuickTarget, String> {
    let s = strip_space(line);
    if s.is_empty() {
        return Err("Enter user@host.".to_owned());
    }
    let Some((user, rest)) = s.split_once('@') else {
        return Err("Use user@host (e.g. root@server or ubuntu@10.0.0.5:2222).".to_owned());
    };
    let invalid = || {
        "Invalid format. Examples: root@192.168.1.5, admin@bastion.example:2222, \
         root@[2001:db8::1]:2222"
            .to_owned()
    };
    let has_space = |t: &str| {
        t.chars()
            .any(|c| c.is_whitespace() || ('\x1c'..='\x1f').contains(&c))
    };
    if user.is_empty() || rest.is_empty() || has_space(user) || has_space(rest) {
        return Err(invalid());
    }
    // IPv6 address, optionally with a %zone (interface name after the hex part).
    const V6: &str = r"[0-9A-Fa-f:.]+(?:%[A-Za-z0-9._-]+)?";
    let re = |p: String| Regex::new(&p).map_err(|e| e.to_string());
    let (host, port_s): (String, Option<String>) =
        if let Some(m) = re(format!(r"^\[({V6})\](?::([0-9]+))?$"))?.captures(rest) {
            (m[1].to_owned(), m.get(2).map(|p| p.as_str().to_owned()))
        } else if rest.matches(':').count() >= 2 {
            // Bare IPv6: everything is the address, a port is impossible.
            if !re(format!("^{V6}$"))?.is_match(rest) {
                return Err(invalid());
            }
            (rest.to_owned(), None)
        } else {
            let m = re(r"^([^:\[\]]+)(?::([0-9]+))?$".to_owned())?
                .captures(rest)
                .ok_or_else(invalid)?;
            (m[1].to_owned(), m.get(2).map(|p| p.as_str().to_owned()))
        };
    let port = match port_s {
        None => None,
        Some(p) => match p.parse::<u64>() {
            Ok(n @ 1..=65535) => u16::try_from(n).ok(),
            _ => return Err("Port must be between 1 and 65535.".to_owned()),
        },
    };
    Ok(QuickTarget {
        user: user.to_owned(),
        host,
        port,
    })
}

#[cfg(test)]
mod expand_user_tests {
    use super::*;

    #[test]
    fn backslash_after_tilde_is_a_windows_only_separator() {
        let home = Some(r"C:\Users\x\");
        assert_eq!(
            expand_user_with(r"~\.ssh\id_ed25519", home, true),
            r"C:\Users\x\.ssh\id_ed25519"
        );
        assert_eq!(
            expand_user_with("~/.ssh/id", home, true),
            r"C:\Users\x/.ssh/id"
        );
        // Unix: `~\x` is not a home path and the home keeps its backslashes.
        assert_eq!(expand_user_with(r"~\x", home, false), r"~\x");
        assert_eq!(
            expand_user_with("~/a", Some("/home/u/"), false),
            "/home/u/a"
        );
        assert_eq!(expand_user_with("~", Some("/home/u"), false), "/home/u");
    }
}

#[cfg(all(test, windows))]
mod key_copy_windows_tests {
    use super::*;
    use std::path::PathBuf;
    use std::process::{Command, Stdio};

    fn s(x: &str) -> Value {
        Value::Str(x.to_owned())
    }

    /// The user's case: the key's ACL lets the Users group read it, so Win32-OpenSSH ignores it
    /// ("Bad permissions"), while MobaXterm logs in. `TG_KEY_E2E=<authorized key>` also logs in
    /// to the local sshd with the copy.
    #[test]
    fn a_key_others_can_read_reaches_ssh_as_a_private_copy() {
        let dir = std::env::temp_dir().join(format!("tg-keycopy-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("dir");
        let key = dir.join("loose");
        match std::env::var_os("TG_KEY_E2E") {
            Some(k) => {
                std::fs::copy(k, &key).expect("copy key");
            }
            None => {
                let made = Command::new("ssh-keygen")
                    .args(["-q", "-t", "ed25519", "-N", "", "-f"])
                    .arg(&key)
                    .stdin(Stdio::null())
                    .output()
                    .expect("ssh-keygen");
                assert!(made.status.success(), "{made:?}");
            }
        }
        let grant = Command::new("icacls")
            .arg(&key)
            .args(["/grant", "*S-1-5-32-545:(R)"])
            .output()
            .expect("icacls");
        assert!(grant.status.success());
        assert!(crate::platform::key_too_open(&key));

        let c = Value::Map(vec![
            (s("host"), s("127.0.0.1")),
            (s("identity_file"), s(&key.to_string_lossy())),
        ]);
        let flags = identity_flags(&c, None).expect("flags");
        let copy = PathBuf::from(&flags[1]);
        assert_ne!(copy, key);
        let check = Command::new("ssh-keygen")
            .args(["-y", "-P", "", "-f"])
            .arg(&copy)
            .stdin(Stdio::null())
            .output()
            .expect("ssh-keygen -y");
        assert!(
            check.status.success(),
            "OpenSSH still refuses the copy: {}",
            String::from_utf8_lossy(&check.stderr)
        );
        assert!(identity_warning(&c, None).is_some_and(|w| w.contains("private copy")));
        if std::env::var_os("TG_KEY_E2E").is_some() {
            // Exit status only: Win32-OpenSSH may not exit while its output is a pipe.
            let login = Command::new("ssh")
                .args(&flags)
                .args(["-o", "BatchMode=yes", "-o", "StrictHostKeyChecking=no"])
                .arg(format!(
                    "{}@127.0.0.1",
                    std::env::var("USERNAME").expect("user")
                ))
                .arg("exit")
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .expect("ssh");
            assert!(login.success(), "login with the copy failed: {login:?}");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
