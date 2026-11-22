//! Remote listing over raw bytes: the `ssh <host> <cmd>` argv for listing, mkdir, rename, delete
//! and chmod, and the parser of the listing output, without the
//! scp/sftp builders ([`crate::proto::scp`]) and the OSC 7 scanner (terminal).
//!
//! Names are `Vec<u8>` end to end (ADR-12): names are never decoded for
//! operations, so no byte is lost or altered on the way back to the remote shell. Only
//! [`display_name`] decodes (lossily) for showing a name.
//!
//! ## Quoting of byte names
//! [`quote_bytes`] is `shlex.quote` over bytes: an empty name is `''`, a name made only of ASCII
//! `[A-Za-z0-9_@%+=:,./-]` stays as is, anything else (every non-ASCII byte included) is wrapped in
//! single quotes with `'` written as `'"'"'`. The remote shell receives the original bytes inside
//! single quotes; for valid UTF-8 the result equals [`crate::shell::quote`]. The remote command is one argv element and is not
//! re-encoded, so `caf\xe9` reaches the remote shell as the byte `0xe9`.
//!
//! ## Listing output
//! The command prints [`LS_MARKER`], `pwd`, a format tag (`F` = GNU `find -printf` NUL records,
//! `L` = `ls -lAn` fallback) and the entries; everything before the marker (`.bashrc` noise) is
//! ignored. The `F` records keep names containing newlines or ` -> `; the `L` fallback is
//! line-based (the `splitlines` separators below included).
//!
//! Golden fixtures: `compat/golden/listing*.json` (bytes as `{"$bytes": hex}`), tests in
//! `tests/remote_listing.rs`.
//!
//! Limits (none occur with real `find`/`ls` output):
//! - In the `ls -lAn` fallback the token classes of the line regex are ASCII: Unicode digits or
//!   non-ASCII whitespace (NBSP, U+2003, ...) between the numeric/date columns are not recognised
//!   (the line is skipped, like any exotic line). Names may contain anything.
//! - `size` is `i64` and ASCII-digit only; an unparsable size is `0`.
//! - `mode`, `mtime` and `owner` are kept as lossily decoded `String`s (only names and link
//!   targets are bytes).

use crate::platform;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use regex::bytes::Regex;
use sha1::{Digest, Sha1};

use crate::config::doc::Value;
use crate::config::model::{truthy, value_int, value_str};
use crate::proto::ssh::{Env, build_ssh_command_with, quote_repr};
use crate::shell::split;

/// Everything before this line is login-shell noise (an `echo` in `.bashrc`).
pub const LS_MARKER: &str = "@@TERMIGATOR-LS@@";

/// GNU find, one NUL-terminated field each: symbolic mode, size, sortable mtime, owner:group,
/// name, link target.
const FIND_FMT: &str = r"%M\0%s\0%TY-%Tm-%Td %TH:%TM\0%u:%g\0%P\0%l\0";
const FIND_FIELDS: usize = 6;

/// The subset of a connection a file operation needs: endpoint, auth, route. No
/// `remote_command`, forwards or mosh: those belong to the terminal.
const BROWSE_KEYS: [&str; 10] = [
    "host",
    "user",
    "port",
    "password",
    "identity_file",
    "proxy_jump",
    "jumphost",
    "jump_host",
    "strict_host_key_checking",
    "ssh_keepalive",
];

/// A remote directory entry. Names and link targets are the exact bytes the host sent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteEntry {
    pub name: Vec<u8>,
    pub is_dir: bool,
    pub is_link: bool,
    pub size: i64,
    /// Symbolic mode, e.g. `-rwxr-x---`.
    pub mode: String,
    pub mtime: String,
    pub link_target: Vec<u8>,
    /// `user:group` (names via find, numeric ids via the ls fallback).
    pub owner: String,
}

/// `str(v)` of a mapping value; absent (`None`) is `"None"`.
fn str_or_none(v: Option<&Value>) -> String {
    v.map_or_else(|| "None".to_owned(), value_str)
}

/// `str(v or default)`.
fn str_or(v: Option<&Value>, default: &str) -> String {
    match v {
        Some(x) if truthy(x) => value_str(x),
        _ => default.to_owned(),
    }
}

/// `shlex.quote` over bytes (see the module doc).
pub fn quote_bytes(s: &[u8]) -> Vec<u8> {
    if s.is_empty() {
        return b"''".to_vec();
    }
    if s.iter()
        .all(|b| b.is_ascii_alphanumeric() || b"_@%+=:,./-".contains(b))
    {
        return s.to_vec();
    }
    let mut out = vec![b'\''];
    for &b in s {
        if b == b'\'' {
            out.extend_from_slice(b"'\"'\"'");
        } else {
            out.push(b);
        }
    }
    out.push(b'\'');
    out
}

/// `bytes.rstrip(b"/")`.
fn rstrip_slashes(b: &[u8]) -> &[u8] {
    let n = b.iter().rposition(|c| *c != b'/').map_or(0, |i| i + 1);
    &b[..n]
}

/// Shell word for `cd`. A typed `~` / `~/dir` was quoted as-is, and the shell never expands
/// a quoted tilde, so it is rewritten to `"$HOME"`.
fn cd_target(path: &[u8]) -> Vec<u8> {
    if path == b"~" {
        return b"\"$HOME\"".to_vec();
    }
    if let Some(rest) = path.strip_prefix(b"~/") {
        return if rest.is_empty() {
            b"\"$HOME\"".to_vec()
        } else {
            [b"\"$HOME\"/".as_slice(), &quote_bytes(rest)].concat()
        };
    }
    quote_bytes(if path.is_empty() { b"." } else { path })
}

/// The Files panel's copy of a connection: only the endpoint/auth/route keys, plus
/// `extra_ssh_args` minus the flags meant for the terminal (forwards, `-N -f -t -tt -T`).
pub fn browse_conn(conn: &Value) -> Value {
    let mut out: Vec<(Value, Value)> = BROWSE_KEYS
        .iter()
        .filter_map(|k| {
            conn.get(k)
                .map(|v| (Value::Str((*k).to_owned()), v.clone()))
        })
        .collect();
    let extra = ssh_extra_for_files(conn);
    if !extra.is_empty() {
        out.push((
            Value::Str("extra_ssh_args".to_owned()),
            Value::Seq(extra.into_iter().map(Value::Str).collect()),
        ));
    }
    Value::Map(out)
}

fn extra_args_list(conn: &Value) -> Vec<String> {
    match conn.get("extra_ssh_args") {
        Some(Value::Seq(items)) if !items.is_empty() => items.iter().map(value_str).collect(),
        Some(raw) if truthy(raw) && !matches!(raw, Value::Seq(_)) => {
            split(&value_str(raw)).unwrap_or_default()
        }
        _ => Vec::new(),
    }
}

fn ssh_extra_for_files(conn: &Value) -> Vec<String> {
    const DROP_FLAGS: [&str; 5] = ["-N", "-f", "-t", "-tt", "-T"];
    const DROP_WITH_VALUE: [&str; 3] = ["-L", "-R", "-D"];
    let args = extra_args_list(conn);
    let mut out = Vec::new();
    let mut i = 0;
    while i < args.len() {
        let a = args[i].as_str();
        if DROP_FLAGS.contains(&a) {
            i += 1;
        } else if DROP_WITH_VALUE.contains(&a) {
            i += 2;
        } else if DROP_WITH_VALUE
            .iter()
            .any(|f| a.starts_with(f) && a.len() > 2)
        {
            i += 1;
        } else {
            out.push(a.to_owned());
            i += 1;
        }
    }
    out
}

/// The app's `launch` settings as the panel follows them: keepalive and ControlMaster
/// default to on.
fn launch_for_files(launch: &Value) -> Value {
    let pick = |k: &str, default: Value| {
        (
            Value::Str(k.to_owned()),
            launch.get(k).cloned().unwrap_or(default),
        )
    };
    Value::Map(vec![
        pick("ssh_keepalive", Value::Bool(true)),
        pick("ssh_keepalive_seconds", Value::Int(15)),
        pick("ssh_control_master", Value::Bool(true)),
    ])
}

/// An argv whose elements are raw bytes (the remote command may hold non-UTF-8 names).
pub type Argv = Vec<Vec<u8>>;

/// Argv builders for one connection: `conn` and `launch` as in the app's connection model, `env` for binary
/// lookup, the ControlPath directory and `$HOME` ([`Env::system`] on the real machine).
#[derive(Clone, Copy)]
pub struct RemoteOps<'a> {
    pub conn: &'a Value,
    pub launch: &'a Value,
    pub env: &'a Env,
}

impl RemoteOps<'_> {
    /// argv running `remote_cmd` on the connection's host, sharing the session's ControlMaster
    /// socket. The command is one argument; the remote shell does the word splitting, so paths
    /// must go through [`quote_bytes`].
    pub fn exec_argv(&self, remote_cmd: &[u8]) -> Result<Argv, String> {
        let line = build_ssh_command_with(
            &browse_conn(self.conn),
            &launch_for_files(self.launch),
            self.env,
        )
        .map_err(|e| {
            if e.is_empty() {
                "Could not build ssh command.".to_owned()
            } else {
                e
            }
        })?;
        const SHAPE: &str = "Unexpected ssh command shape.";
        let mut argv = split(&line).map_err(|_| SHAPE.to_owned())?;
        let i = argv.iter().position(|a| a == "ssh").ok_or(SHAPE)? + 1;
        let mut opts = vec!["-o".to_owned(), "ConnectTimeout=10".to_owned()];
        if str_or(self.conn.get("password"), "").is_empty() {
            // A background op must never hang on a prompt; with a saved password sshpass
            // answers it, so BatchMode stays off there.
            opts.extend(["-o".to_owned(), "BatchMode=yes".to_owned()]);
        }
        argv.splice(i..i, opts);
        let mut out: Argv = argv.into_iter().map(String::into_bytes).collect();
        out.push(remote_cmd.to_vec());
        Ok(out)
    }

    /// List `path` (`""`/`.` = login home). The output (parse with [`parse_ls_output`]) is the
    /// marker, the resolved `pwd`, a format tag and the entries; it runs under `sh` so a
    /// csh/fish login shell does not matter, and unsets `TIME_STYLE` for the `ls` fallback.
    pub fn list_dir_argv(&self, path: &[u8]) -> Result<Argv, String> {
        let script = [
            b"cd -- ".as_slice(),
            &cd_target(path),
            b" || exit 1; echo ",
            LS_MARKER.as_bytes(),
            b"; pwd; if find . -maxdepth 0 -printf '' >/dev/null 2>&1; then echo F; ",
            b"find . -mindepth 1 -maxdepth 1 -printf '",
            FIND_FMT.as_bytes(),
            b"'; else echo L; env -u TIME_STYLE LC_ALL=C ls -lAn; fi",
        ]
        .concat();
        self.exec_sh(&script)
    }

    fn exec_sh(&self, script: &[u8]) -> Result<Argv, String> {
        self.exec_argv(&[b"exec sh -c ".as_slice(), &quote_bytes(script)].concat())
    }

    /// `<size> <mtime seconds>` of a remote file (GNU / BusyBox `stat`), the base an
    /// edited copy is compared with before its save goes back.
    pub fn stat_argv(&self, path: &[u8]) -> Result<Argv, String> {
        self.exec_argv(&[b"stat -c '%s %Y' -- ".as_slice(), &quote_bytes(path)].concat())
    }

    pub fn mkdir_argv(&self, path: &[u8]) -> Result<Argv, String> {
        self.exec_argv(&[b"mkdir -p -- ".as_slice(), &quote_bytes(path)].concat())
    }

    /// Never clobbers; a POSIX existence check refuses with a readable message on stderr.
    pub fn rename_argv(&self, old: &[u8], new: &[u8]) -> Result<Argv, String> {
        let (o, n) = (quote_bytes(old), quote_bytes(new));
        let base = new.rsplit(|b| *b == b'/').next().unwrap_or(new);
        let msg = quote_bytes(&[b"'".as_slice(), base, b"' already exists"].concat());
        let script = [
            b"if [ -e ".as_slice(),
            &n,
            b" ] || [ -L ",
            &n,
            b" ]; then echo ",
            &msg,
            b" >&2; exit 17; fi; mv -- ",
            &o,
            b" ",
            &n,
        ]
        .concat();
        self.exec_sh(&script)
    }

    pub fn delete_argv(&self, path: &[u8]) -> Result<Argv, String> {
        self.delete_many_argv(&[path])
    }

    /// One `rm -rf` for a multi-selection. Guard 11: `""`, `/`, `.`, `..` (also with trailing
    /// slashes) are refused.
    pub fn delete_many_argv<P: AsRef<[u8]>>(&self, paths: &[P]) -> Result<Argv, String> {
        if paths.is_empty() {
            return Err("Nothing to delete.".to_owned());
        }
        let placeholder = |p: &P| {
            let t = rstrip_slashes(p.as_ref());
            t.is_empty() || t == b"." || t == b".."
        };
        if paths.iter().any(placeholder) {
            return Err("Refusing to delete the root or a relative placeholder path.".to_owned());
        }
        self.exec_argv(&words(b"rm -rf --", paths))
    }

    pub fn chmod_argv<P: AsRef<[u8]>>(
        &self,
        paths: &[P],
        mode: i64,
        recursive: bool,
    ) -> Result<Argv, String> {
        if paths.is_empty() {
            return Err("Nothing to change.".to_owned());
        }
        if !(0..=0o7777).contains(&mode) {
            return Err("Invalid mode.".to_owned());
        }
        let head = format!("chmod {}{mode:04o} --", if recursive { "-R " } else { "" });
        self.exec_argv(&words(head.as_bytes(), paths))
    }
}

/// `head` followed by every path, quoted, each after a space.
fn words<P: AsRef<[u8]>>(head: &[u8], paths: &[P]) -> Vec<u8> {
    let mut cmd = head.to_vec();
    for p in paths {
        cmd.push(b' ');
        cmd.extend(quote_bytes(p.as_ref()));
    }
    cmd
}

/// `-rwxr-x---` gives `0o750`; setuid / setgid / sticky (`s`/`S`, `t`/`T`) are folded in.
pub fn mode_bits(mode: &str) -> u32 {
    let c: Vec<char> = mode.chars().collect();
    if c.len() < 10 {
        return 0;
    }
    let mut bits = 0;
    for (i, ch) in c[1..10].iter().enumerate() {
        if !matches!(ch, '-' | 'S' | 'T') {
            bits |= 1 << (8 - i);
        }
    }
    for (pos, bit, chars) in [(3, 0o4000, "sS"), (6, 0o2000, "sS"), (9, 0o1000, "tT")] {
        if chars.contains(c[pos]) {
            bits |= bit;
        }
    }
    bits
}

/// `1536` gives `1.5 K`; bytes are whole numbers.
pub fn human_size(n: i64) -> String {
    let mut f = n as f64;
    for unit in ["B", "K", "M", "G", "T"] {
        if f < 1024.0 || unit == "T" {
            return if unit == "B" {
                format!("{} B", f as i64)
            } else {
                format!("{f:.1} {unit}")
            };
        }
        f /= 1024.0;
    }
    format!("{n} B")
}

/// The name for showing; undecodable bytes become U+FFFD (Qt would silently drop them).
pub fn display_name(name: &[u8]) -> String {
    String::from_utf8_lossy(name).into_owned()
}

/// Stable per-host key for remembered Files-panel state (bookmarks, last folder).
pub fn host_key(conn: &Value) -> String {
    format!(
        "{}@{}:{}",
        str_or(conn.get("user"), ""),
        str_or(conn.get("host"), ""),
        str_or(conn.get("port"), "22")
    )
}

/// Whether an ssh run failed because the server wanted a password (or key) it did not get:
/// the background commands of Files run with `BatchMode`, so without a saved password and
/// before the terminal's own login has created the shared connection they end like this.
pub fn is_auth_failure(rc: i32, stderr: &[u8]) -> bool {
    if rc != 255 {
        return false;
    }
    let text = String::from_utf8_lossy(stderr).to_lowercase();
    text.contains("permission denied") || text.contains("authentication failed")
}

/// `conn` with `password` set (a password typed into Files for this session).
pub fn with_typed_password(conn: &Value, password: &str) -> Value {
    let mut out = conn.clone();
    if let Value::Map(pairs) = &mut out {
        pairs.retain(|(k, _)| !matches!(k, Value::Str(s) if s == "password"));
        pairs.push((
            Value::Str("password".to_owned()),
            Value::Str(password.to_owned()),
        ));
    }
    out
}

fn default_edit_root() -> PathBuf {
    let base = std::env::var_os("XDG_CACHE_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::home_dir().map(|h| h.join(".cache")))
        .unwrap_or_else(|| PathBuf::from(".cache"));
    base.join("termigator").join("edit")
}

/// At start: working copies of earlier runs are gone with their sessions, so they are deleted
/// (their content came from servers).
pub fn clear_edit_cache() {
    let _ = std::fs::remove_dir_all(default_edit_root());
}

/// Local working copy for "Edit": one stable folder per host + remote path under
/// `$XDG_CACHE_HOME/termigator/edit` (or `cache_root`), named after the remote file.
pub fn edit_cache_path(conn: &Value, remote_path: &[u8], cache_root: Option<&Path>) -> PathBuf {
    let root = cache_root.map_or_else(default_edit_root, Path::to_path_buf);
    let key = [
        str_or_none(conn.get("user")).as_bytes(),
        b"@",
        str_or_none(conn.get("host")).as_bytes(),
        b":",
        str_or(conn.get("port"), "22").as_bytes(),
        b":",
        remote_path,
    ]
    .concat();
    let digest: String = Sha1::digest(&key)
        .iter()
        .take(6)
        .map(|b| format!("{b:02x}"))
        .collect();
    let trimmed = rstrip_slashes(remote_path);
    let base = trimmed.rsplit(|b| *b == b'/').next().unwrap_or(trimmed);
    // `.` / `..` from a hostile server would climb out of the digest folder.
    let name = if base.is_empty() || base == b"." || base == b".." {
        b"file".to_vec()
    } else {
        base.to_vec()
    };
    root.join(digest).join(platform::os_string_from_vec(name))
}

/// `posixpath.join(parent or "/", name)`.
pub fn remote_join(parent: &[u8], name: &[u8]) -> Vec<u8> {
    if name.starts_with(b"/") {
        return name.to_vec();
    }
    let mut path = if parent.is_empty() {
        b"/".to_vec()
    } else {
        parent.to_vec()
    };
    if !path.ends_with(b"/") {
        path.push(b'/');
    }
    path.extend_from_slice(name);
    path
}

/// `posixpath.dirname(path.rstrip("/")) or "/"`.
pub fn remote_parent(path: &[u8]) -> Vec<u8> {
    let p = rstrip_slashes(if path.is_empty() { b"/" } else { path });
    let head = &p[..p.iter().rposition(|b| *b == b'/').map_or(0, |i| i + 1)];
    let head = if head.iter().all(|b| *b == b'/') {
        head
    } else {
        rstrip_slashes(head)
    };
    if head.is_empty() {
        b"/".to_vec()
    } else {
        head.to_vec()
    }
}

// ---------------------------------------------------------------------------------------------
// Parsing

/// Quoted repr of the first 80 characters of the bytes (invalid bytes escaped), for error messages.
fn repr80(b: &[u8]) -> String {
    let mut items: Vec<Result<char, u8>> = Vec::new();
    let mut rest = b;
    while !rest.is_empty() {
        match std::str::from_utf8(rest) {
            Ok(s) => {
                items.extend(s.chars().map(Ok));
                break;
            }
            Err(e) => {
                let (good, bad) = rest.split_at(e.valid_up_to());
                items.extend(
                    std::str::from_utf8(good)
                        .into_iter()
                        .flat_map(str::chars)
                        .map(Ok),
                );
                let n = e.error_len().unwrap_or(bad.len());
                items.extend(bad[..n].iter().map(|b| Err(*b)));
                rest = &bad[n..];
            }
        }
    }
    items.truncate(80);
    let has = |q: char| items.contains(&Ok(q));
    let quote = if has('\'') && !has('"') { '"' } else { '\'' };
    let mut out = String::from(quote);
    for it in items {
        match it {
            Ok(c) if c == quote => {
                out.push('\\');
                out.push(c);
            }
            Ok(c) => {
                let one = quote_repr(&c.to_string());
                out.push_str(&one[1..one.len() - 1]);
            }
            Err(byte) => out.push_str(&format!("\\udc{byte:02x}")),
        }
    }
    out.push(quote);
    out
}

fn is_space(c: char) -> bool {
    c.is_whitespace() || ('\x1c'..='\x1f').contains(&c)
}

fn first_char(b: &[u8]) -> Option<char> {
    let head = &b[..b.len().min(4)];
    let s = match std::str::from_utf8(head) {
        Ok(s) => s,
        Err(e) => std::str::from_utf8(&head[..e.valid_up_to()]).ok()?,
    };
    s.chars().next()
}

fn last_char(b: &[u8]) -> Option<char> {
    (1..=b.len().min(4)).find_map(|k| std::str::from_utf8(&b[b.len() - k..]).ok()?.chars().next())
}

/// Strips leading/trailing Unicode whitespace over bytes (invalid bytes are never whitespace).
fn strip_space_bytes(mut b: &[u8]) -> &[u8] {
    while let Some(c) = first_char(b).filter(|c| is_space(*c)) {
        b = &b[c.len_utf8()..];
    }
    while let Some(c) = last_char(b).filter(|c| is_space(*c)) {
        b = &b[..b.len() - c.len_utf8()];
    }
    b
}

/// `splitlines` over bytes (`\n \r \r\n \v \f \x1c-\x1e`, U+0085, U+2028, U+2029;
/// a lead byte always starts a new character, so scanning bytes finds exactly those).
fn splitlines(b: &[u8]) -> Vec<&[u8]> {
    let mut out = Vec::new();
    let (mut start, mut i) = (0, 0);
    while i < b.len() {
        let sep = match b[i] {
            b'\r' if b.get(i + 1) == Some(&b'\n') => 2,
            b'\n' | b'\r' | 0x0b | 0x0c | 0x1c..=0x1e => 1,
            0xc2 if b.get(i + 1) == Some(&0x85) => 2,
            0xe2 if b.get(i + 1) == Some(&0x80) && matches!(b.get(i + 2), Some(0xa8 | 0xa9)) => 3,
            _ => 0,
        };
        if sep == 0 {
            i += 1;
        } else {
            out.push(&b[start..i]);
            i += sep;
            start = i;
        }
    }
    if start < b.len() {
        out.push(&b[start..]);
    }
    out
}

fn find_sub(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

/// `bytes.partition(b"\n")`: the part before and after the first newline (`rest` empty if none).
fn partition_nl(b: &[u8]) -> (&[u8], &[u8]) {
    match b.iter().position(|c| *c == b'\n') {
        Some(i) => (&b[..i], &b[i + 1..]),
        None => (b, &[]),
    }
}

fn lossy(b: &[u8]) -> String {
    String::from_utf8_lossy(b).into_owned()
}

/// `int(s)` of a size field; anything unparsable is 0.
fn parse_size(b: &[u8]) -> i64 {
    std::str::from_utf8(b)
        .ok()
        .and_then(|s| value_int(&Value::Str(s.to_owned())))
        .unwrap_or(0)
}

fn parse_find_records(body: &[u8]) -> Vec<RemoteEntry> {
    let fields: Vec<&[u8]> = body.split(|b| *b == 0).collect();
    let mut entries = Vec::new();
    for rec in fields.as_chunks::<FIND_FIELDS>().0 {
        let (mode, size, mtime, owner, name, target) =
            (rec[0], rec[1], rec[2], rec[3], rec[4], rec[5]);
        if mode.is_empty() || name.is_empty() {
            continue;
        }
        let kind = mode[0];
        entries.push(RemoteEntry {
            name: name.to_vec(),
            is_dir: kind == b'd',
            is_link: kind == b'l',
            size: parse_size(size),
            mode: lossy(mode),
            mtime: lossy(mtime),
            link_target: if kind == b'l' {
                target.to_vec()
            } else {
                Vec::new()
            },
            owner: lossy(owner),
        });
    }
    entries
}

/// `LC_ALL=C ls -lAn`: mode links uid gid size  month day time|year  name[ -> target]. Sizes of
/// device nodes appear as "major, minor" (stored as 0). `\s` is the ASCII whitespace set plus the
/// `\x1c-\x1f` separators.
static LS_LINE_RE: LazyLock<Regex> = LazyLock::new(|| {
    let (s, n) = (r"[\s\x1c-\x1f]", r"[^\s\x1c-\x1f]");
    let pat = format!(
        r"(?-u)^([bcdlps-][rwxsStT-]{{9}}[+.@]?){s}+\d+{s}+({n}+){s}+({n}+){s}+(\d+|\d+,{s}*\d+){s}+({n}+{s}+{n}+{s}+{n}+){s}(.+)$"
    );
    Regex::new(&pat).unwrap_or_else(|e| unreachable!("static regex: {e}"))
});

/// `(resolved_path, entries)` from the output of [`RemoteOps::list_dir_argv`]; output without the
/// marker is read as the older plain `pwd` + `ls -lAn` form. Errors carry the messages shown to the user.
pub fn parse_ls_output(text: &[u8]) -> Result<(Vec<u8>, Vec<RemoteEntry>), String> {
    let unexpected =
        |cwd: &[u8]| format!("Unexpected response (no absolute path): {}", repr80(cwd));
    let joined;
    let mut text = text;
    if find_sub(text, LS_MARKER.as_bytes()).is_some() {
        let key = [LS_MARKER.as_bytes(), b"\n"].concat();
        let after = find_sub(text, &key).map_or(text, |i| &text[i + key.len()..]);
        let (cwd, rest) = partition_nl(after);
        let (tag, body) = partition_nl(rest);
        if !cwd.starts_with(b"/") {
            return Err(unexpected(cwd));
        }
        if tag == b"F" {
            return Ok((cwd.to_vec(), parse_find_records(body)));
        }
        joined = [cwd, b"\n", body].concat(); // "L": ls -lAn, parsed below
        text = &joined;
    }
    let mut lines = splitlines(text).into_iter().peekable();
    while lines.next_if(|l| strip_space_bytes(l).is_empty()).is_some() {}
    let Some(first) = lines.next() else {
        return Err("Empty response from the remote host.".to_owned());
    };
    let cwd = strip_space_bytes(first);
    if !cwd.starts_with(b"/") {
        return Err(unexpected(cwd));
    }
    let mut entries = Vec::new();
    for ln in lines {
        if strip_space_bytes(ln).is_empty() || ln.starts_with(b"total ") {
            continue;
        }
        let Some(m) = LS_LINE_RE.captures(ln) else {
            continue; // exotic line (odd locale): skip rather than fail the listing
        };
        let mode = &m[1];
        let is_link = mode[0] == b'l';
        let mut name = m[6].to_vec();
        let mut link_target = Vec::new();
        if is_link && let Some(i) = find_sub(&name, b" -> ") {
            link_target = name[i + 4..].to_vec();
            name.truncate(i);
        }
        entries.push(RemoteEntry {
            name,
            is_dir: mode[0] == b'd',
            is_link,
            size: parse_size(&m[4]),
            mode: lossy(mode),
            mtime: lossy(&m[5]),
            link_target,
            owner: format!("{}:{}", lossy(&m[2]), lossy(&m[3])),
        });
    }
    Ok((cwd.to_vec(), entries))
}
