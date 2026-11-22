//! `~/.ssh/config` import.
//!
//! Every concrete `Host` alias (no `*`/`?`/`!`/`,`) becomes one connection. Values are
//! resolved the way ssh does it: blocks are read top to bottom, the **first** value for a
//! keyword wins (except the multi keywords: IdentityFile, the forwards, CertificateFile,
//! SendEnv, SetEnv), and every block whose pattern list matches the alias contributes
//! (`Host *` defaults included). `Include` is followed (globs, paths relative to the
//! config's directory, depth <= 16); `Match` blocks cannot be evaluated offline: `Match all`
//! counts as `Host *`, any other is skipped and counted.
//!
//! Mapped to entry fields: HostName (`%h` = alias), User (default: the local user), Port,
//! IdentityFile (only an existing file, as ssh ignores missing ones), ProxyJump,
//! Local/Remote/DynamicForward, ForwardAgent, ForwardX11, RemoteCommand,
//! StrictHostKeyChecking. Tokens `%% %h %n %r %p %d %u` are expanded in HostName and
//! IdentityFile. Any other option set in a block naming the alias itself is kept as
//! `-o Key=Value` in `extra_ssh_args` (canonical casing for known keywords). Options of
//! wildcard blocks are not copied: ssh still reads the config at connect time.
//!
//! Security note: `ProxyCommand` and `LocalCommand`
//! are copied into `extra_ssh_args` like any other option, so importing a foreign config
//! stores commands that ssh will later run locally. The caller should warn about it.
//!
//! Host patterns use `fnmatch` rules (so `[abc]` is a character class, unlike in ssh).
//!
//! Limits on hostile input: files over
//! [`MAX_FILE_BYTES`](super::putty::MAX_FILE_BYTES) are unreadable; at most
//! [`MAX_INCLUDED_FILES`] files / [`MAX_TOTAL_BYTES`] bytes are read, at most [`MAX_BLOCKS`]
//! blocks and [`MAX_ALIASES`] aliases kept, patterns and aliases over
//! [`MAX_PATTERN_CHARS`] never match (all reported through `truncated`); a malformed
//! character range in a pattern never panics; `~user` is not expanded; a port beyond `i64` or
//! written with non-ASCII digits
//! counts as 22.
//!
//! Golden: `compat/golden/ssh_config.json`, tests in `tests/import_ssh_config.rs`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use super::moba::{row, splitlines, sv};
use super::putty::read_limited;
use crate::config::doc::Value;
use crate::config::model::{strip_space, value_int};
use crate::proto::ssh::expand_user;
use crate::shell;

/// Group of imported hosts.
pub const DEFAULT_GROUP: &str = "ssh_config";
/// Deepest `Include` nesting that is followed.
pub const MAX_INCLUDE_DEPTH: usize = 16;
/// Files read per import (the config and everything it includes).
pub const MAX_INCLUDED_FILES: usize = 1024;
/// Bytes read per import over all files.
pub const MAX_TOTAL_BYTES: u64 = 64 * 1024 * 1024;
/// `Host`/`Match` blocks kept.
pub const MAX_BLOCKS: usize = 5_000;
/// Aliases imported.
pub const MAX_ALIASES: usize = 1_000;
/// Longest pattern or alias that is matched.
pub const MAX_PATTERN_CHARS: usize = 4_096;
/// Entries one glob level may produce.
const MAX_GLOB_ENTRIES: usize = 4_096;

/// Keywords whose every occurrence counts.
const MULTI: &[&str] = &[
    "identityfile",
    "localforward",
    "remoteforward",
    "dynamicforward",
    "certificatefile",
    "sendenv",
    "setenv",
];

/// Mapped to entry fields (or managed by Termigator itself): never copied as `-o`.
const NOT_COPIED: &[&str] = &[
    "host",
    "match",
    "include",
    "hostname",
    "user",
    "port",
    "identityfile",
    "proxyjump",
    "localforward",
    "remoteforward",
    "dynamicforward",
    "forwardagent",
    "forwardx11",
    "remotecommand",
    "stricthostkeychecking",
    "identitiesonly",
    // ControlMaster is Termigator's (the terminal, Files panel and scp share it).
    "controlmaster",
    "controlpath",
    "controlpersist",
    // keepalive comes from launch.ssh_keepalive_seconds
    "serveraliveinterval",
    "serveralivecountmax",
];

/// Canonical spelling of the `-o` options (ssh accepts any case).
const CANON: &[&str] = &[
    "AddKeysToAgent",
    "AddressFamily",
    "BatchMode",
    "BindAddress",
    "BindInterface",
    "CanonicalDomains",
    "CanonicalizeHostname",
    "CertificateFile",
    "CheckHostIP",
    "Ciphers",
    "ClearAllForwardings",
    "Compression",
    "ConnectionAttempts",
    "ConnectTimeout",
    "EscapeChar",
    "ExitOnForwardFailure",
    "FingerprintHash",
    "ForwardX11Trusted",
    "GatewayPorts",
    "GlobalKnownHostsFile",
    "GSSAPIAuthentication",
    "GSSAPIDelegateCredentials",
    "HashKnownHosts",
    "HostbasedAuthentication",
    "HostKeyAlgorithms",
    "HostKeyAlias",
    "IdentityAgent",
    "IPQoS",
    "KbdInteractiveAuthentication",
    "KexAlgorithms",
    "LogLevel",
    "MACs",
    "NumberOfPasswordPrompts",
    "PasswordAuthentication",
    "PermitLocalCommand",
    "LocalCommand",
    "PKCS11Provider",
    "PreferredAuthentications",
    "ProxyCommand",
    "ProxyUseFdpass",
    "PubkeyAcceptedAlgorithms",
    "PubkeyAcceptedKeyTypes",
    "PubkeyAuthentication",
    "RekeyLimit",
    "RequestTTY",
    "SendEnv",
    "SetEnv",
    "StreamLocalBindUnlink",
    "TCPKeepAlive",
    "Tunnel",
    "TunnelDevice",
    "UpdateHostKeys",
    "UserKnownHostsFile",
    "VerifyHostKeyDNS",
    "VisualHostKey",
];

const STRICT_VALUES: &[&str] = &["yes", "no", "ask", "accept-new", "off"];

/// Result of an import run.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SshConfigImport {
    /// One entry (map) per concrete alias, in order of appearance.
    pub connections: Vec<Value>,
    /// `Match` blocks other than `Match all` (not evaluated).
    pub skipped_match_blocks: usize,
    /// Files that could not be read (the config itself or an include).
    pub unreadable: Vec<String>,
    /// A resource limit (see the module doc) cut the import short.
    pub truncated: bool,
}

/// `~/.ssh/config`.
pub fn default_ssh_config_path(home: &Path) -> PathBuf {
    home.join(".ssh").join("config")
}

fn is_space(c: char) -> bool {
    c.is_whitespace() || ('\x1c'..='\x1f').contains(&c)
}

// ----------------------------------------------------------------------------- fnmatch

enum Tok {
    Star,
    Any,
    Lit(char),
    Class {
        negate: bool,
        set: Vec<(char, char)>,
    },
}

impl Tok {
    fn one(&self, c: char) -> bool {
        match self {
            Tok::Star => false,
            Tok::Any => true,
            Tok::Lit(l) => *l == c,
            Tok::Class { negate, set } => {
                set.iter().any(|(lo, hi)| (*lo..=*hi).contains(&c)) != *negate
            }
        }
    }
}

/// The `[...]` body `p[i..j]` as `fnmatch` translation reads it.
fn class_tok(p: &[char], i: usize, j: usize) -> Tok {
    let stuff = &p[i..j];
    let mut chunks: Vec<Vec<char>> = Vec::new();
    if stuff.contains(&'-') {
        let (mut start, mut k) = (i, if p[i] == '!' { i + 2 } else { i + 1 });
        while let Some(at) = (k..j).find(|&x| p[x] == '-') {
            chunks.push(p[start..at].to_vec());
            start = at + 1;
            k = at + 3;
        }
        if start < j {
            chunks.push(p[start..j].to_vec());
        } else if let Some(last) = chunks.last_mut() {
            last.push('-');
        }
        // Drop reversed ranges together with their end points.
        for k in (1..chunks.len()).rev() {
            let (a, b) = (chunks[k - 1].last().copied(), chunks[k].first().copied());
            if let (Some(a), Some(b)) = (a, b)
                && a > b
            {
                let tail: Vec<char> = chunks[k][1..].to_vec();
                chunks[k - 1].pop();
                chunks[k - 1].extend(tail);
                chunks.remove(k);
            }
        }
    } else {
        chunks.push(stuff.to_vec());
    }
    if chunks.len() == 1 && chunks[0] == ['!'] {
        return Tok::Lit('!');
    }
    let negate = chunks.first().and_then(|c| c.first()) == Some(&'!');
    if negate {
        chunks[0].remove(0);
    }
    let mut set: Vec<(char, char)> = chunks.iter().flatten().map(|&c| (c, c)).collect();
    for w in chunks.windows(2) {
        if let (Some(&lo), Some(&hi)) = (w[0].last(), w[1].first())
            && lo <= hi
        {
            set.push((lo, hi));
        }
    }
    Tok::Class { negate, set }
}

fn compile(pat: &str) -> Vec<Tok> {
    let p: Vec<char> = pat.chars().collect();
    let mut toks = Vec::new();
    let mut i = 0;
    while i < p.len() {
        let c = p[i];
        i += 1;
        match c {
            '*' => {
                if !matches!(toks.last(), Some(Tok::Star)) {
                    toks.push(Tok::Star);
                }
            }
            '?' => toks.push(Tok::Any),
            '[' => {
                let mut j = i;
                if p.get(j) == Some(&'!') {
                    j += 1;
                }
                if p.get(j) == Some(&']') {
                    j += 1;
                }
                while j < p.len() && p[j] != ']' {
                    j += 1;
                }
                if j >= p.len() {
                    toks.push(Tok::Lit('['));
                } else {
                    toks.push(class_tok(&p, i, j));
                    i = j + 1;
                }
            }
            c => toks.push(Tok::Lit(c)),
        }
    }
    toks
}

/// Case-sensitive `fnmatch` match.
fn fnmatchcase(name: &str, pat: &str) -> bool {
    if name.chars().count() > MAX_PATTERN_CHARS || pat.chars().count() > MAX_PATTERN_CHARS {
        return false;
    }
    if !pat.contains(['*', '?', '[']) {
        return name == pat;
    }
    let toks = compile(pat);
    let s: Vec<char> = name.chars().collect();
    let (mut ti, mut si) = (0, 0);
    let mut star: Option<(usize, usize)> = None;
    while si < s.len() {
        match toks.get(ti) {
            Some(Tok::Star) => {
                star = Some((ti, si));
                ti += 1;
                continue;
            }
            Some(t) if t.one(s[si]) => {
                ti += 1;
                si += 1;
                continue;
            }
            _ => {}
        }
        match star {
            Some((st, ss)) => {
                ti = st + 1;
                si = ss + 1;
                star = Some((st, ss + 1));
            }
            None => return false,
        }
    }
    while matches!(toks.get(ti), Some(Tok::Star)) {
        ti += 1;
    }
    ti == toks.len()
}

// ------------------------------------------------------------------------------- glob

fn has_magic(s: &str) -> bool {
    s.contains(['*', '?', '['])
}

/// `glob(pattern)` (no recursion; hidden names only for patterns starting
/// with `.`), unsorted, capped.
/// Length of the root prefix of `pattern`: `/` everywhere, `X:/` (a drive) when `windows`; 0 for
/// a relative pattern.
fn root_len(pattern: &str, windows: bool) -> usize {
    if pattern.starts_with('/') {
        return 1;
    }
    let b = pattern.as_bytes();
    if windows && b.len() >= 3 && b[0].is_ascii_alphabetic() && b[1] == b':' && b[2] == b'/' {
        return 3;
    }
    0
}

fn glob_paths(pattern: &str) -> Vec<PathBuf> {
    if !has_magic(pattern) {
        let p = Path::new(pattern);
        return if p.symlink_metadata().is_ok() {
            vec![p.to_path_buf()]
        } else {
            Vec::new()
        };
    }
    let root = root_len(pattern, cfg!(windows));
    let mut cur: Vec<PathBuf> = vec![PathBuf::from(&pattern[..root])];
    for comp in pattern[root..].split('/').filter(|c| !c.is_empty()) {
        let mut next = Vec::new();
        for base in &cur {
            if !has_magic(comp) {
                let p = base.join(comp);
                if p.symlink_metadata().is_ok() {
                    next.push(p);
                }
                continue;
            }
            let dir = if base.as_os_str().is_empty() {
                Path::new(".")
            } else {
                base.as_path()
            };
            let Ok(rd) = std::fs::read_dir(dir) else {
                continue;
            };
            for entry in rd.flatten() {
                let name = entry.file_name().to_string_lossy().into_owned();
                if (name.starts_with('.') && !comp.starts_with('.')) || !fnmatchcase(&name, comp) {
                    continue;
                }
                next.push(base.join(&name));
                if next.len() >= MAX_GLOB_ENTRIES {
                    break;
                }
            }
        }
        next.truncate(MAX_GLOB_ENTRIES);
        cur = next;
    }
    cur
}

// ------------------------------------------------------------------------ line reading

struct Reader<'a> {
    home: &'a str,
    base_dir: &'a Path,
    unreadable: Vec<String>,
    files: usize,
    bytes: u64,
    truncated: bool,
}

fn split_line(line: &str) -> Option<(String, String)> {
    let s = strip_space(line);
    if s.is_empty() || s.starts_with('#') {
        return None;
    }
    // "Key value", "Key=value", "Key = value"
    let end = s
        .char_indices()
        .find(|&(_, c)| is_space(c) || c == '=')
        .map_or(s.len(), |(i, _)| i);
    let (key, rest) = (&s[..end], strip_space(&s[end..]));
    let rest = match rest.strip_prefix('=') {
        Some(r) => strip_space(r),
        None => rest,
    };
    (!key.is_empty()).then(|| (key.to_lowercase(), rest.to_owned()))
}

fn unquote(value: &str) -> &str {
    let v = strip_space(value);
    match v.strip_prefix('"').and_then(|r| r.strip_suffix('"')) {
        Some(inner) if v.len() >= 2 => inner,
        _ => v,
    }
}

/// `shlex.split`, or whitespace splitting when the quoting is broken.
fn words(value: &str) -> Vec<String> {
    shell::split(value).unwrap_or_else(|_| {
        value
            .split(is_space)
            .filter(|w| !w.is_empty())
            .map(str::to_owned)
            .collect()
    })
}

impl Reader<'_> {
    /// Lines of `path` with every `Include` expanded in place (as ssh reads them).
    fn read_lines(&mut self, path: &Path, depth: usize, out: &mut Vec<String>) {
        if self.files >= MAX_INCLUDED_FILES {
            self.truncated = true;
            return;
        }
        self.files += 1;
        let Some(raw) = read_limited(path) else {
            self.unreadable.push(path.to_string_lossy().into_owned());
            return;
        };
        self.bytes += raw.len() as u64;
        if self.bytes > MAX_TOTAL_BYTES {
            self.truncated = true;
            return;
        }
        let text = String::from_utf8_lossy(&raw);
        for line in splitlines(&text) {
            let kv = split_line(line);
            if let Some((key, value)) = &kv
                && key == "include"
                && depth < MAX_INCLUDE_DEPTH
            {
                for pat in words(value) {
                    let pat = expand_user(&pat, Some(self.home));
                    // Windows ssh_config paths may use either separator.
                    let pat = if cfg!(windows) {
                        pat.replace('\\', "/")
                    } else {
                        pat
                    };
                    let pat = if root_len(&pat, cfg!(windows)) > 0 {
                        pat
                    } else {
                        self.base_dir.join(&pat).to_string_lossy().into_owned()
                    };
                    let mut found = glob_paths(&pat);
                    found.sort();
                    for inc in found {
                        if inc.is_file() {
                            self.read_lines(&inc, depth + 1, out);
                        }
                    }
                }
                continue;
            }
            out.push(line.to_owned());
        }
    }
}

// ------------------------------------------------------------------------------ blocks

struct Block {
    /// `Host` patterns; `["*"]` for the implicit top and `Match all`.
    patterns: Vec<String>,
    /// (keyword lower-case, raw value)
    options: Vec<(String, String)>,
    /// An unevaluable `Match` block.
    skipped: bool,
    /// Lower-cased, stripped, comma-split patterns as (negated, pattern).
    pats: Vec<(bool, String)>,
    /// Lower-cased comma-split names (unstripped), for the "names this alias" test.
    names: Vec<String>,
}

impl Block {
    fn new(patterns: Vec<String>, skipped: bool) -> Self {
        let pats = patterns
            .iter()
            .flat_map(|raw| raw.split(','))
            .map(|p| strip_space(p).to_lowercase())
            .filter(|p| !p.is_empty())
            .map(|p| match p.strip_prefix('!') {
                Some(n) => (true, n.to_owned()),
                None => (false, p),
            })
            .collect();
        let names = patterns
            .iter()
            .flat_map(|raw| raw.split(','))
            .map(str::to_lowercase)
            .collect();
        Block {
            patterns,
            options: Vec::new(),
            skipped,
            pats,
            names,
        }
    }
}

fn blocks(lines: &[String], truncated: &mut bool) -> Vec<Block> {
    let star = || Block::new(vec!["*".to_owned()], false);
    let mut out = vec![star()];
    for line in lines {
        let Some((key, value)) = split_line(line) else {
            continue;
        };
        let new = match key.as_str() {
            "host" => Block::new(words(&value), false),
            "match" => {
                let crit: Vec<String> = words(&value).iter().map(|w| w.to_lowercase()).collect();
                if crit == ["all"] {
                    star()
                } else {
                    Block::new(Vec::new(), true)
                }
            }
            _ => {
                if let Some(last) = out.last_mut() {
                    last.options.push((key, value));
                }
                continue;
            }
        };
        if out.len() >= MAX_BLOCKS {
            *truncated = true;
            break;
        }
        out.push(new);
    }
    out
}

/// ssh pattern-list semantics: any positive match, and no negated (`!`) match.
/// `name` is the lower-cased alias.
fn matches(b: &Block, name: &str) -> bool {
    let mut hit = false;
    for (neg, pat) in &b.pats {
        if *neg {
            if fnmatchcase(name, pat) {
                return false;
            }
        } else if fnmatchcase(name, pat) {
            hit = true;
        }
    }
    hit
}

fn is_concrete(pattern: &str) -> bool {
    !pattern.is_empty()
        && !pattern.contains(['*', '?', '!', ','])
        && pattern.chars().count() <= MAX_PATTERN_CHARS
}

fn expand_tokens(value: &str, alias: &str, host: &str, user: &str, port: i64, ctx: &Ctx) -> String {
    let mut out = String::new();
    let mut chars = value.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '%'
            && let Some(&n) = chars.peek()
        {
            let rep = match n {
                '%' => Some("%".to_owned()),
                'h' => Some(host.to_owned()),
                'n' => Some(alias.to_owned()),
                'r' => Some(user.to_owned()),
                'p' => Some(port.to_string()),
                'd' => Some(ctx.home.to_owned()),
                'u' => Some(ctx.user.to_owned()),
                _ => None,
            };
            if let Some(rep) = rep {
                out.push_str(&rep);
                chars.next();
                continue;
            }
        }
        out.push(c);
    }
    out
}

/// What tokens and defaults need to know about the importing user.
struct Ctx<'a> {
    home: &'a str,
    user: &'a str,
}

/// `LocalForward 8080 db:5432` -> `-L` spec `8080:db:5432`.
fn forward_spec(value: &str) -> String {
    words(value).join(":")
}

fn is_yes(value: &str) -> bool {
    matches!(unquote(value).to_lowercase().as_str(), "yes" | "true")
}

fn resolve(alias: &str, blocks: &[Block], ctx: &Ctx) -> Value {
    let alias_lc = alias.to_lowercase();
    let mut first: HashMap<&str, &str> = HashMap::new();
    let mut multi: HashMap<&str, Vec<&str>> = HashMap::new();
    let mut own: Vec<(&str, &str)> = Vec::new(); // options of blocks naming this alias exactly
    for b in blocks {
        if b.skipped || !matches(b, &alias_lc) {
            continue;
        }
        let exact = b.names.contains(&alias_lc);
        for (key, value) in &b.options {
            if MULTI.contains(&key.as_str()) {
                multi.entry(key).or_default().push(value);
            } else if !first.contains_key(key.as_str()) {
                first.insert(key, value);
            } else {
                continue;
            }
            if exact {
                own.push((key, value));
            }
        }
    }
    let get = |k: &str, default: &'static str| first.get(k).copied().unwrap_or(default);

    let host = expand_tokens(unquote(get("hostname", "%h")), alias, alias, "", 22, ctx);
    let user = match unquote(get("user", "")) {
        "" => ctx.user.to_owned(),
        u => u.to_owned(),
    };
    let port = value_int(&Value::Str(unquote(get("port", "22")).to_owned())).unwrap_or(22);
    let host = if host.is_empty() {
        alias.to_owned()
    } else {
        host
    };
    let mut r: Vec<(&str, Value)> = vec![
        ("label", sv(alias)),
        ("host", sv(host.clone())),
        ("user", sv(user.clone())),
        ("group", sv(DEFAULT_GROUP)),
    ];
    if port != 22 && 0 < port && port < 65536 {
        r.push(("port", Value::Int(port)));
    }

    for ident in multi.get("identityfile").into_iter().flatten() {
        let p = expand_tokens(unquote(ident), alias, &host, &user, port, ctx);
        if p.to_lowercase() == "none" {
            break;
        }
        if Path::new(&expand_user(&p, Some(ctx.home))).is_file() {
            r.push(("identity_file", sv(p)));
            break;
        }
    }

    let pj = unquote(get("proxyjump", ""));
    if !pj.is_empty() && pj.to_lowercase() != "none" {
        r.push(("proxy_jump", sv(pj)));
    }
    for (key, name) in [
        ("localforward", "local_forwards"),
        ("remoteforward", "remote_forwards"),
    ] {
        let specs: Vec<Value> = multi
            .get(key)
            .into_iter()
            .flatten()
            .filter(|v| !strip_space(v).is_empty())
            .map(|v| sv(forward_spec(v)))
            .collect();
        if !specs.is_empty() {
            r.push((name, Value::Seq(specs)));
        }
    }
    if let Some(d) = multi.get("dynamicforward").and_then(|d| d.first()) {
        r.push(("dynamic_forward", sv(unquote(d))));
    }
    if is_yes(get("forwardagent", "")) {
        r.push(("forward_agent", Value::Bool(true)));
    }
    if is_yes(get("forwardx11", "")) {
        r.push(("x11_forwarding", Value::Bool(true)));
    }
    let rc = unquote(get("remotecommand", ""));
    if !rc.is_empty() && rc.to_lowercase() != "none" {
        r.push(("remote_command", sv(rc)));
    }
    let shk = unquote(get("stricthostkeychecking", "")).to_lowercase();
    if STRICT_VALUES.contains(&shk.as_str()) {
        r.push(("strict_host_key_checking", sv(shk)));
    }

    let mut extra: Vec<Value> = Vec::new();
    for (key, value) in own {
        if NOT_COPIED.contains(&key) {
            continue;
        }
        let name = CANON
            .iter()
            .find(|c| c.to_lowercase() == key)
            .copied()
            .unwrap_or(key);
        extra.push(sv("-o"));
        extra.push(sv(format!("{name}={}", unquote(value))));
    }
    if !extra.is_empty() {
        r.push(("extra_ssh_args", Value::Seq(extra)));
    }
    row(r)
}

/// Connections for every concrete `Host` alias in `path` (usually `~/.ssh/config`).
/// `home` is `~` (for `Include`, `IdentityFile` and `%d`), `local_user` the account used
/// when a host has no `User` (and for `%u`).
pub fn parse_ssh_config(path: &Path, home: &Path, local_user: &str) -> SshConfigImport {
    let home_s = home.to_string_lossy();
    let base_dir = path.parent().unwrap_or_else(|| Path::new("."));
    let mut reader = Reader {
        home: &home_s,
        base_dir,
        unreadable: Vec::new(),
        files: 0,
        bytes: 0,
        truncated: false,
    };
    let mut lines = Vec::new();
    reader.read_lines(path, 0, &mut lines);
    let mut truncated = reader.truncated;
    let blocks = blocks(&lines, &mut truncated);
    let ctx = Ctx {
        home: &home_s,
        user: local_user,
    };
    let mut aliases: Vec<&str> = Vec::new();
    'collect: for b in blocks.iter().filter(|b| !b.skipped) {
        for pat in b.patterns.iter().flat_map(|raw| raw.split(',')) {
            let pat = strip_space(pat);
            if is_concrete(pat)
                && !aliases
                    .iter()
                    .any(|a| a.to_lowercase() == pat.to_lowercase())
            {
                if aliases.len() >= MAX_ALIASES {
                    truncated = true;
                    break 'collect;
                }
                aliases.push(pat);
            }
        }
    }
    SshConfigImport {
        connections: aliases.iter().map(|a| resolve(a, &blocks, &ctx)).collect(),
        skipped_match_blocks: blocks.iter().filter(|b| b.skipped).count(),
        unreadable: reader.unreadable,
        truncated,
    }
}

/// ssh options that run a local program or load a library. From the user's own `~/.ssh/config`
/// they are what ssh runs anyway; from a file someone else wrote, connecting would run them.
pub const COMMAND_OPTIONS: &[&str] = &[
    "ProxyCommand",
    "LocalCommand",
    "PermitLocalCommand",
    "KnownHostsCommand",
    "PKCS11Provider",
    "SecurityKeyProvider",
];

/// Entry fields that hand the server something of this computer: the SSH agent (it can log in
/// elsewhere as you while connected) and the X display. Off for a file someone else wrote.
const FORWARD_FIELDS: &[(&str, &str)] = &[
    ("forward_agent", "ForwardAgent"),
    ("x11_forwarding", "ForwardX11"),
];

/// Removes the [`COMMAND_OPTIONS`] from each entry's `extra_ssh_args` (pairs `-o`, `Key=value`)
/// and the [`FORWARD_FIELDS`], and returns `label: Key, Key` for every entry that had any.
pub fn strip_command_options(connections: &mut [Value]) -> Vec<String> {
    let mut report = Vec::new();
    for conn in connections.iter_mut() {
        let label = conn.get("label").map(value_str_lossy).unwrap_or_default();
        let mut dropped = Vec::new();
        if let Value::Map(fields) = conn {
            fields.retain(|(k, v)| {
                let hit = FORWARD_FIELDS
                    .iter()
                    .find(|(f, _)| matches!(k, Value::Str(s) if s == f) && *v == Value::Bool(true));
                if let Some((_, name)) = hit {
                    dropped.push(*name);
                }
                hit.is_none()
            });
        }
        let Some(Value::Seq(args)) = conn.get_mut("extra_ssh_args") else {
            if !dropped.is_empty() {
                report.push(format!("{label}: {}", dropped.join(", ")));
            }
            continue;
        };
        let mut kept = Vec::with_capacity(args.len());
        let mut it = std::mem::take(args).into_iter().peekable();
        while let Some(a) = it.next() {
            let opt = match (&a, it.peek()) {
                (Value::Str(o), Some(Value::Str(kv))) if o == "-o" => kv.split('=').next(),
                _ => None,
            };
            match opt.and_then(|k| COMMAND_OPTIONS.iter().find(|c| c.eq_ignore_ascii_case(k))) {
                Some(name) => {
                    dropped.push(*name);
                    it.next();
                }
                None => kept.push(a),
            }
        }
        *args = kept;
        if !dropped.is_empty() {
            report.push(format!("{label}: {}", dropped.join(", ")));
        }
    }
    report
}

fn value_str_lossy(v: &Value) -> String {
    match v {
        Value::Str(s) => s.clone(),
        Value::Int(i) => i.to_string(),
        _ => String::new(),
    }
}

#[cfg(test)]
mod command_option_tests {
    use super::*;

    #[test]
    fn command_options_are_removed_and_reported() {
        let s = |x: &str| Value::Str(x.to_owned());
        let args = |xs: &[&str]| Value::Seq(xs.iter().map(|x| s(x)).collect());
        let mut conns = vec![
            Value::Map(vec![
                (s("label"), s("web")),
                (
                    s("extra_ssh_args"),
                    args(&[
                        "-o",
                        "ProxyCommand=nc %h %p",
                        "-o",
                        "Ciphers=aes256-gcm@openssh.com",
                        "-o",
                        "localcommand=id",
                    ]),
                ),
            ]),
            Value::Map(vec![
                (s("label"), s("db")),
                (s("extra_ssh_args"), args(&["-o", "MACs=x"])),
            ]),
            Value::Map(vec![(s("label"), s("bare"))]),
            Value::Map(vec![
                (s("label"), s("agent")),
                (s("forward_agent"), Value::Bool(true)),
                (s("x11_forwarding"), Value::Bool(true)),
            ]),
        ];
        let report = strip_command_options(&mut conns);
        assert_eq!(
            report,
            [
                "web: ProxyCommand, LocalCommand",
                "agent: ForwardAgent, ForwardX11"
            ]
        );
        assert_eq!(conns[3], Value::Map(vec![(s("label"), s("agent"))]));
        assert_eq!(
            conns[0].get("extra_ssh_args"),
            Some(&args(&["-o", "Ciphers=aes256-gcm@openssh.com"]))
        );
        assert_eq!(
            conns[1].get("extra_ssh_args"),
            Some(&args(&["-o", "MACs=x"]))
        );
    }
}

#[cfg(test)]
mod windows_path_tests {
    use super::*;

    #[test]
    fn drive_roots_count_only_on_windows() {
        assert_eq!(root_len("/etc/ssh", false), 1);
        assert_eq!(root_len("/etc/ssh", true), 1);
        assert_eq!(root_len("C:/Users/x/.ssh/*.conf", true), 3);
        assert_eq!(root_len("c:/x", true), 3);
        // On Unix `C:/x` is an ordinary relative name.
        assert_eq!(root_len("C:/Users/x", false), 0);
        assert_eq!(root_len("C:x", true), 0);
        assert_eq!(root_len("conf.d/*", true), 0);
    }
}
