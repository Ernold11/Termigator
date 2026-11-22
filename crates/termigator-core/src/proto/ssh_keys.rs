//! ssh-keygen / ssh-add / ssh-copy-id command lines and output parsers .
//!
//! [`list_keys`] lists the key pairs of `~/.ssh` (`x` + `x.pub`, through `ssh-keygen -l -f`,
//! parsed by [`parse_keygen_line`]); [`agent_fingerprints`] asks `ssh-add -l` which are loaded
//! ([`parse_agent_list`]). [`keygen_argv`] never overwrites a file and refuses a name starting
//! with `-` (guard 14); [`copy_id_argv`] applies the same `user@host` guards as ssh and scp
//! (guards 1 and 2) and sends a saved password through `sshpass -e`, never through argv.
//! Every step that asks for a passphrase or password runs in a terminal tab, so Termigator never
//! sees those secrets. The process-running helpers use the killable runner with a 10 s timeout;
//! live behaviour is covered by the live tests, the pure parts by golden fixtures
//! (`compat/golden/tools_ssh_keys.json`).
//!
//! Known defect: a list `proxy_jump` is passed to `-o ProxyJump=` as
//! the text of the list (`['a', 'b']`) and the port is not validated. Differences: `~user` is
//! looked up through the passwd database like `expanduser`; `..` as a key name is reported as
//! "Choose a file name" instead of "already exists".

use std::collections::BTreeSet;
use std::ffi::OsString;
use std::path::{Component, Path, PathBuf};
use std::sync::LazyLock;
use std::time::Duration;

use regex::Regex;

use crate::config::doc::Value;
use crate::config::model::{proxy_jump_value, strip_space, truthy, value_str};
use crate::proc::runner::{Command, spawn};
use crate::proto::ssh::{
    password_via_askpass, quote_repr, ssh_arg_injection_guard, ssh_target_guard,
};
use crate::proto::vnc::{Which, text_stripped};

/// Key type to its extra `ssh-keygen` arguments; ed25519 first (the default and modern choice).
pub const KEY_TYPES: [(&str, &[&str]); 3] = [
    ("ed25519", &[]),
    ("rsa", &["-b", "4096"]),
    ("ecdsa", &["-b", "521"]),
];

const TIMEOUT: Duration = Duration::from_secs(10);

#[allow(clippy::unwrap_used)] // constant pattern, checked by the tests
static LINE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^(\d+)\s+(\S+)\s+(.*?)\s*\(([A-Z0-9-]+)\)\s*$").unwrap());

/// The fields of one `ssh-keygen -l` line: `256 SHA256:... comment (ED25519)`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyLine {
    pub bits: u32,
    /// `SHA256:...`
    pub fingerprint: String,
    /// As printed (`no comment` included).
    pub comment: String,
    /// `ED25519`, `RSA`, ...
    pub key_type: String,
}

/// A key pair found in the SSH directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyInfo {
    /// The private key.
    pub path: PathBuf,
    pub pub_path: PathBuf,
    pub bits: u32,
    pub fingerprint: String,
    /// Empty when the key has none (`no comment`).
    pub comment: String,
    pub key_type: String,
}

/// Parses the output of `ssh-keygen -l -f key.pub` (surrounding whitespace is ignored).
pub fn parse_keygen_line(output: &str) -> Option<KeyLine> {
    let m = LINE.captures(strip_space(output))?;
    Some(KeyLine {
        bits: m[1].parse().ok()?,
        fingerprint: m[2].to_owned(),
        comment: m[3].to_owned(),
        key_type: m[4].to_owned(),
    })
}

/// A `Command` with only what the ssh tools need from the environment.
fn tool_command(argv: &[&str], vars: &[&str]) -> Command {
    let mut c = Command::new(argv);
    for v in vars {
        if let Some(val) = std::env::var_os(v) {
            c.env.insert(OsString::from(v), val);
        }
    }
    c.timeout = Some(TIMEOUT);
    c
}

/// Runs `ssh-keygen -l -f pub` (`None`: not runnable, timed out, failed or unparsable).
pub fn fingerprint(pub_path: &Path) -> Option<KeyLine> {
    let p = pub_path.to_str()?;
    let out = spawn(tool_command(&["ssh-keygen", "-l", "-f", p], &["PATH"]))
        .ok()?
        .wait()?;
    if out.timed_out || out.code() != 0 {
        return None;
    }
    parse_keygen_line(&String::from_utf8_lossy(&out.stdout))
}

/// Key pairs (`x` + `x.pub`) in `ssh_dir`, sorted by file name, fingerprints from `ssh-keygen`.
pub fn list_keys(ssh_dir: &Path) -> Vec<KeyInfo> {
    list_keys_with(ssh_dir, &fingerprint)
}

/// [`list_keys`] with an injectable fingerprint reader.
pub fn list_keys_with(ssh_dir: &Path, fp: &dyn Fn(&Path) -> Option<KeyLine>) -> Vec<KeyInfo> {
    let Ok(rd) = std::fs::read_dir(ssh_dir) else {
        return Vec::new();
    };
    let mut pubs: Vec<(String, PathBuf)> = rd
        .filter_map(Result::ok)
        .filter_map(|e| {
            let name = e.file_name().into_string().ok()?;
            (name.len() > 4 && name.ends_with(".pub")).then(|| (name, e.path()))
        })
        .collect();
    pubs.sort();
    let mut out = Vec::new();
    for (name, pub_path) in pubs {
        let path = ssh_dir.join(&name[..name.len() - 4]);
        if !path.is_file() {
            continue;
        }
        let Some(k) = fp(&pub_path) else { continue };
        let comment = if k.comment == "no comment" {
            String::new()
        } else {
            k.comment
        };
        out.push(KeyInfo {
            path,
            pub_path,
            bits: k.bits,
            fingerprint: k.fingerprint,
            comment,
            key_type: k.key_type,
        });
    }
    out
}

/// Fingerprints of the `ssh-add -l` output; `None` when no agent is reachable (exit code 2),
/// an empty set for "The agent has no identities." (exit code 1).
pub fn parse_agent_list(code: i32, stdout: &str) -> Option<BTreeSet<String>> {
    match code {
        2 => None,
        1 => Some(BTreeSet::new()),
        _ => Some(
            stdout
                .lines()
                .filter_map(|l| l.split_whitespace().nth(1))
                .filter(|p| p.starts_with("SHA256:") || p.starts_with("MD5:"))
                .map(str::to_owned)
                .collect(),
        ),
    }
}

/// Fingerprints loaded in the agent; `None` when there is no agent or `ssh-add` is missing.
pub fn agent_fingerprints() -> Option<BTreeSet<String>> {
    std::env::var_os("SSH_AUTH_SOCK").filter(|s| !s.is_empty())?;
    if !crate::proto::vnc::which_in_path("ssh-add") {
        return None;
    }
    let out = spawn(tool_command(&["ssh-add", "-l"], &["PATH", "SSH_AUTH_SOCK"]))
        .ok()?
        .wait()?;
    if out.timed_out {
        return None;
    }
    parse_agent_list(out.code(), &String::from_utf8_lossy(&out.stdout))
}

/// Expands `~` and `~/...` (with `home`) and `~user` (passwd lookup).
fn expanduser(path: &str, home: Option<&str>) -> String {
    if !path.starts_with('~') {
        return path.to_owned();
    }
    let end = path.find('/').unwrap_or(path.len());
    let user_home = if end == 1 {
        match home {
            Some(h) => Some(h.to_owned()),
            None => crate::platform::current_user_home().map(|d| d.to_string_lossy().into_owned()),
        }
    } else {
        crate::platform::user_home_by_name(&path[1..end]).map(|d| d.to_string_lossy().into_owned())
    };
    match user_home {
        None => path.to_owned(),
        Some(h) => {
            let joined = format!("{}{}", h.trim_end_matches('/'), &path[end..]);
            if joined.is_empty() {
                "/".to_owned()
            } else {
                joined
            }
        }
    }
}

/// `pathlib.Path` normalisation: `.` components and repeated or trailing slashes disappear.
pub(crate) fn normalize_path(p: &Path) -> PathBuf {
    let cleaned: PathBuf = p
        .components()
        .filter(|c| !matches!(c, Component::CurDir))
        .collect();
    if cleaned.as_os_str().is_empty() {
        PathBuf::from(".")
    } else {
        cleaned
    }
}

/// `ssh-keygen -t TYPE [-b N] -f PATH -C COMMENT` — the passphrase is asked in the terminal.
/// `home` is the value of `$HOME` for a leading `~`.
pub fn keygen_argv(
    key_type: &str,
    path: &str,
    comment: &str,
    home: Option<&str>,
) -> Result<Vec<String>, String> {
    let Some((_, extra)) = KEY_TYPES.iter().find(|(k, _)| *k == key_type) else {
        return Err(format!("Unknown key type: {key_type}"));
    };
    let p = normalize_path(Path::new(&expanduser(strip_space(path), home)));
    let name = p
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    if strip_space(path).is_empty() || name.is_empty() || name.starts_with('-') {
        return Err("Choose a file name for the key.".to_owned());
    }
    if p.exists() || p.with_file_name(format!("{name}.pub")).exists() {
        return Err(format!(
            "{} already exists — pick another name (an existing key is never overwritten).",
            p.display()
        ));
    }
    let parent = match p.parent() {
        Some(d) if !d.as_os_str().is_empty() => d,
        _ => Path::new("."),
    };
    if !parent.is_dir() {
        return Err(format!("Folder {} does not exist.", parent.display()));
    }
    let mut argv: Vec<String> = ["ssh-keygen", "-t", key_type]
        .map(str::to_owned)
        .into_iter()
        .chain(extra.iter().map(|s| (*s).to_owned()))
        .collect();
    argv.extend([
        "-f".to_owned(),
        p.to_string_lossy().into_owned(),
        "-C".to_owned(),
        strip_space(comment).to_owned(),
    ]);
    Ok(argv)
}

/// `ssh-add KEY`.
pub fn add_to_agent_argv(key: &Path) -> Vec<String> {
    vec!["ssh-add".to_owned(), key.to_string_lossy().into_owned()]
}

/// The text of the jump host: a list of texts shows as `['a', 'b']`.
fn jump_text(v: &Value) -> String {
    match v {
        Value::Seq(items) if items.iter().all(|i| matches!(i, Value::Str(_))) => {
            let shown: Vec<String> = items.iter().map(|i| quote_repr(&value_str(i))).collect();
            format!("[{}]", shown.join(", "))
        }
        other => value_str(other),
    }
}

/// `[sshpass -e] ssh-copy-id -i key.pub [-p port] [-o ProxyJump=...] user@host`.
pub fn copy_id_argv(
    pub_path: &Path,
    conn: &Value,
    with_password: bool,
    which: Which,
) -> Result<Vec<String>, String> {
    let host = text_stripped(conn.get("host"));
    let user = text_stripped(conn.get("user"));
    if host.is_empty() {
        return Err("The entry has no host.".to_owned());
    }
    // The same user@host check as ssh / scp: no option in disguise.
    let err = if user.is_empty() {
        ssh_arg_injection_guard(&host).map(str::to_owned)
    } else {
        ssh_target_guard(&user, &host)
    };
    if let Some(e) = err {
        return Err(e);
    }
    if !which("ssh-copy-id") {
        return Err("ssh-copy-id is not installed (package openssh-client).".to_owned());
    }
    let mut argv = vec![
        "ssh-copy-id".to_owned(),
        "-i".to_owned(),
        pub_path.to_string_lossy().into_owned(),
    ];
    let port = text_stripped(conn.get("port"));
    if !port.is_empty() && port != "22" {
        argv.extend(["-p".to_owned(), port]);
    }
    let jump = proxy_jump_value(conn)
        .filter(|v| truthy(v))
        .map(|v| strip_space(&jump_text(v)).to_owned())
        .unwrap_or_default();
    if !jump.is_empty() {
        argv.extend(["-o".to_owned(), format!("ProxyJump={jump}")]);
    }
    argv.push(if user.is_empty() {
        host
    } else {
        format!("{user}@{host}")
    });
    // Windows, or a jump host: ssh asks the askpass helper instead (`ssh_password_env`).
    if with_password && !password_via_askpass(conn) {
        if !which("sshpass") {
            return Err(
                "sshpass is needed to use the saved password (or remove it and type it)."
                    .to_owned(),
            );
        }
        argv.splice(0..0, ["sshpass".to_owned(), "-e".to_owned()]);
    }
    Ok(argv)
}
