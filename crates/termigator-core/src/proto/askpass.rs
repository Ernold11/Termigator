//! SSH password logins without `sshpass`, the platform-neutral half.
//!
//! OpenSSH runs `$SSH_ASKPASS <prompt>` and reads the answer from its stdout. Termigator points it
//! at its own executable (`--askpass`), which asks the running instance for the secret over a
//! private channel (a Unix socket, AF_UNIX on Windows too) and prints it. This module holds what does not
//! depend on the channel: telling the prompts apart, deciding what may be answered with a saved
//! password, and the environment that makes ssh use the helper. The secret itself never appears
//! in argv or in the environment of the child; the environment carries only the channel name.

use std::fs;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use crate::platform::{self, IpcListener, IpcStream};
use crate::vault::Secret;

/// Variable carrying the channel (socket / pipe path) the helper connects to.
pub const CHANNEL_ENV: &str = "TERMIGATOR_ASKPASS_CHANNEL";
/// Variable carrying the one-time token the helper must present on that channel.
pub const TOKEN_ENV: &str = "TERMIGATOR_ASKPASS_TOKEN";

const IO_TIMEOUT: Duration = Duration::from_secs(2);
const POLL: Duration = Duration::from_millis(25);
/// A request is a token and one prompt; anything longer is not one.
const MAX_LINE: u64 = 4096;

/// What ssh is asking for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PromptKind {
    /// `user@host's password:`, `Password:`, keyboard-interactive password prompts.
    Password,
    /// `Enter passphrase for key …` — a key's passphrase, not the login password.
    Passphrase,
    /// `Are you sure you want to continue connecting (yes/no…)?` and other yes/no questions.
    Confirm,
    /// Anything else (OTP, verification codes, …).
    Other,
}

/// Classifies the prompt text ssh passes as `argv[1]`.
pub fn classify(prompt: &str) -> PromptKind {
    let p = prompt.trim().to_lowercase();
    if p.contains("(yes/no") || p.contains("[fingerprint]") {
        PromptKind::Confirm
    } else if p.contains("passphrase") || p.contains("pin for") {
        PromptKind::Passphrase
    } else if p.contains("verification code") || p.contains("one-time") || p.contains("otp") {
        PromptKind::Other
    } else if p.ends_with("password:") || p.ends_with("password: ") || p.contains("'s password") {
        PromptKind::Password
    } else {
        PromptKind::Other
    }
}

/// Whose password a saved secret is. Through a jump host every ssh in the chain inherits the
/// helper, so only a prompt that names the target may get it — never the jump host's.
/// An empty `user` matches any user on `host`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target {
    pub user: String,
    pub host: String,
}

impl Target {
    /// `user@host's password:` (password authentication) or `(user@host) Password:`
    /// (keyboard-interactive, OpenSSH 8.4+) for this target; a bare `Password:` names nobody.
    fn named_in(&self, prompt: &str) -> bool {
        let p = prompt.trim();
        let who = if let Some(rest) = p.strip_prefix('(') {
            rest.split_once(')').map(|(who, _)| who)
        } else {
            // ASCII lowering keeps the byte offsets of `p`.
            p.to_ascii_lowercase()
                .find("'s password")
                .map(|end| &p[..end])
        };
        let Some((user, host)) = who.and_then(|w| w.rsplit_once('@')) else {
            return false;
        };
        host.eq_ignore_ascii_case(&self.host)
            && (self.user.is_empty() || user.eq_ignore_ascii_case(&self.user))
    }
}

/// The bytes to print for `prompt`, or `None` when the saved password must not answer it (the
/// helper then exits non-zero and ssh falls back to its own failure path instead of leaking the
/// password into a passphrase, OTP or host-key question, or to a jump host when `target` is set).
pub fn answer(prompt: &str, secret: &Secret, target: Option<&Target>) -> Option<Vec<u8>> {
    if classify(prompt) != PromptKind::Password {
        return None;
    }
    if target.is_some_and(|t| !t.named_in(prompt)) {
        return None;
    }
    let mut out = secret.expose().to_vec();
    out.push(b'\n');
    Some(out)
}

/// Environment that makes ssh call `exe --askpass`-style helper `exe` for every prompt, even
/// with a terminal attached (`SSH_ASKPASS_REQUIRE=force`), plus the channel the helper dials.
pub fn env(exe: &str, channel: &str, token: &str) -> Vec<(String, String)> {
    vec![
        ("SSH_ASKPASS".to_owned(), exe.to_owned()),
        ("SSH_ASKPASS_REQUIRE".to_owned(), "force".to_owned()),
        (CHANNEL_ENV.to_owned(), channel.to_owned()),
        (TOKEN_ENV.to_owned(), token.to_owned()),
    ]
}

/// True when this process was started by ssh as the askpass helper (the channel is in the
/// environment; ssh passes only the prompt, so there is no flag to look for).
pub fn is_helper_invocation(channel_env: Option<&str>) -> bool {
    channel_env.is_some_and(|c| !c.is_empty())
}

/// Serves one secret on a private channel. Every request is `<token>\t<prompt>\n`; the reply is
/// the answer to that prompt, or nothing (the client then exits non-zero). Dropping the server
/// stops it and removes the socket file.
pub struct Server {
    path: PathBuf,
    secrets: Secrets,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

/// A token the server answers, with its secret and, through a jump host, whose it is.
struct Entry {
    token: Secret,
    secret: Secret,
    target: Option<Target>,
}

type Secrets = Arc<Mutex<Vec<Entry>>>;

impl Server {
    /// Binds `path` and serves nothing until a secret is [`add`](Server::add)ed.
    pub fn start(path: &Path) -> io::Result<Server> {
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            platform::private_dir_create(parent, true)?;
        }
        let listener = IpcListener::bind(path)?;
        let setup = platform::set_private_file(path).and_then(|()| listener.set_nonblocking(true));
        if let Err(e) = setup {
            let _ = fs::remove_file(path);
            return Err(e);
        }
        let stop = Arc::new(AtomicBool::new(false));
        let secrets: Secrets = Arc::default();
        let thread = {
            let stop = Arc::clone(&stop);
            let secrets = Arc::clone(&secrets);
            thread::Builder::new()
                .name("termigator-askpass".into())
                .spawn(move || serve(&listener, &stop, &secrets))?
        };
        Ok(Server {
            path: path.to_owned(),
            secrets,
            stop,
            thread: Some(thread),
        })
    }

    /// Lets `token` fetch `secret` for the prompts of `target` (any password prompt for `None`).
    pub fn add(&self, token: &str, secret: Secret, target: Option<Target>) {
        if let Ok(mut all) = self.secrets.lock() {
            all.push(Entry {
                token: Secret::new(token.as_bytes().to_vec()),
                secret,
                target,
            });
        }
    }

    /// The token for `secret` and `target`, a new random one the first time.
    pub fn register(&self, secret: &Secret, target: Option<&Target>) -> io::Result<String> {
        let mut all = self
            .secrets
            .lock()
            .map_err(|_| io::Error::other("poisoned"))?;
        if let Some(e) = all
            .iter()
            .find(|e| e.secret == *secret && e.target.as_ref() == target)
        {
            return Ok(String::from_utf8_lossy(e.token.expose()).into_owned());
        }
        let mut raw = [0u8; 16];
        getrandom::fill(&mut raw).map_err(io::Error::other)?;
        let token: String = raw.iter().map(|b| format!("{b:02x}")).collect();
        all.push(Entry {
            token: Secret::new(token.as_bytes().to_vec()),
            secret: Secret::new(secret.expose().to_vec()),
            target: target.cloned(),
        });
        Ok(token)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
        let _ = fs::remove_file(&self.path);
    }
}

fn serve(listener: &IpcListener, stop: &AtomicBool, secrets: &Secrets) {
    while !stop.load(Ordering::SeqCst) {
        match listener.accept() {
            Ok((stream, _)) => {
                let _ = handle(stream, secrets);
            }
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => thread::sleep(POLL),
            Err(_) => thread::sleep(POLL),
        }
    }
}

fn handle(mut stream: IpcStream, secrets: &Secrets) -> io::Result<()> {
    stream.set_nonblocking(false)?;
    stream.set_read_timeout(Some(IO_TIMEOUT))?;
    stream.set_write_timeout(Some(IO_TIMEOUT))?;
    let mut line = Vec::new();
    BufReader::new((&stream).take(MAX_LINE)).read_until(b'\n', &mut line)?;
    let line = String::from_utf8_lossy(&line);
    let Some((given, prompt)) = line.trim_end_matches(['\r', '\n']).split_once('\t') else {
        return Ok(());
    };
    let given = Secret::new(given.as_bytes().to_vec());
    let reply = secrets
        .lock()
        .ok()
        .and_then(|all| {
            all.iter()
                .find(|e| e.token == given)
                .map(|e| answer(prompt, &e.secret, e.target.as_ref()))
        })
        .flatten();
    // One line, `+answer` or `-` (refused): the helper stops at the newline instead of waiting
    // for the end of the stream, which Windows' AF_UNIX sometimes does not deliver.
    let mut out = Vec::with_capacity(reply.as_ref().map_or(2, |r| r.len() + 1));
    match reply {
        Some(reply) => {
            out.push(b'+');
            out.extend_from_slice(&reply);
        }
        None => out.extend_from_slice(b"-\n"),
    }
    stream.write_all(&out)?;
    stream.shutdown(std::net::Shutdown::Write)
}

static BROKER: OnceLock<Option<Server>> = OnceLock::new();

/// Where this process's secret server listens: a private socket in the runtime directory
/// (an AF_UNIX socket file on Windows as well).
fn broker_path() -> Option<PathBuf> {
    let mut nonce = [0u8; 8];
    getrandom::fill(&mut nonce).ok()?;
    let name = channel_name(
        &std::env::var("USERNAME")
            .or_else(|_| std::env::var("USER"))
            .unwrap_or_default(),
        u64::from_le_bytes(nonce),
    );
    // %TEMP% is already per-user on Windows; the socket is an AF_UNIX file there too.
    crate::paths::private_runtime_dir(&["askpass"]).map(|d| d.join(name))
}

/// The process-wide secret server, started on first use. `None` when it cannot start (no private
/// directory, no socket support); the caller then reports that password logins are unavailable.
pub fn broker() -> Option<&'static Server> {
    BROKER
        .get_or_init(|| Server::start(&broker_path()?).ok())
        .as_ref()
}

/// Environment that lets ssh log in with `password` through the helper `exe` (only at `target`
/// when set), or `None` when the broker is unavailable.
pub fn env_for(
    password: &Secret,
    target: Option<&Target>,
    exe: &str,
) -> Option<Vec<(String, String)>> {
    let server = broker()?;
    let token = server.register(password, target).ok()?;
    Some(env(exe, &server.path().to_string_lossy(), &token))
}

/// The helper side: asks the channel `path` for the answer to `prompt`. `Ok(None)` = refused.
pub fn fetch(path: &Path, token: &str, prompt: &str) -> io::Result<Option<Vec<u8>>> {
    let mut stream = IpcStream::connect(path)?;
    stream.set_read_timeout(Some(IO_TIMEOUT))?;
    stream.set_write_timeout(Some(IO_TIMEOUT))?;
    let prompt = prompt.replace(['\n', '\r', '\t'], " ");
    stream.write_all(format!("{token}\t{prompt}\n").as_bytes())?;
    let mut line = Vec::new();
    BufReader::new((&stream).take(MAX_LINE)).read_until(b'\n', &mut line)?;
    Ok(match line.split_first() {
        Some((b'+', answer)) => Some(answer.to_vec()),
        _ => None,
    })
}

/// Channel name for one connection: unique per user and per attempt so another local process
/// cannot guess it, e.g. `termigator-askpass-<user>-<nonce>`.
pub fn channel_name(user: &str, nonce: u64) -> String {
    let safe: String = user
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    format!("termigator-askpass-{safe}-{nonce:016x}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prompts_are_told_apart() {
        for p in [
            "user@host's password: ",
            "Password:",
            "(user@host) Password: ",
        ] {
            assert_eq!(classify(p), PromptKind::Password, "{p}");
        }
        assert_eq!(
            classify("Enter passphrase for key '/home/u/.ssh/id_ed25519': "),
            PromptKind::Passphrase
        );
        assert_eq!(
            classify("Are you sure you want to continue connecting (yes/no/[fingerprint])? "),
            PromptKind::Confirm
        );
        assert_eq!(classify("Verification code: "), PromptKind::Other);
        assert_eq!(classify(""), PromptKind::Other);
    }

    #[test]
    fn only_password_prompts_get_the_secret() {
        let s = Secret::new(b"hunter2".to_vec());
        assert_eq!(
            answer("u@h's password: ", &s, None).as_deref(),
            Some(&b"hunter2\n"[..])
        );
        assert!(answer("Enter passphrase for key 'k': ", &s, None).is_none());
        assert!(answer("Continue connecting (yes/no)? ", &s, None).is_none());
        assert!(answer("Verification code: ", &s, None).is_none());
    }

    #[test]
    fn through_a_jump_host_only_the_target_gets_the_password() {
        let s = Secret::new(b"hunter2".to_vec());
        let t = Target {
            user: "u".to_owned(),
            host: "Target.example".to_owned(),
        };
        for p in [
            "u@target.example's password: ",
            "U@TARGET.EXAMPLE's password: ",
            "(u@target.example) Password: ",
        ] {
            assert!(answer(p, &s, Some(&t)).is_some(), "{p}");
        }
        for p in [
            "u@bastion's password: ",
            "admin@target.example's password: ",
            "(u@bastion) Password: ",
            "xu@target.example.evil's password: ",
            "Password: ",
            "(u@target.example) Enter passphrase for key 'k': ",
        ] {
            assert!(answer(p, &s, Some(&t)).is_none(), "{p}");
        }
        let any_user = Target {
            user: String::new(),
            host: "target.example".to_owned(),
        };
        assert!(answer("me@target.example's password: ", &s, Some(&any_user)).is_some());
        assert!(answer("me@bastion's password: ", &s, Some(&any_user)).is_none());
    }

    #[test]
    fn env_names_the_helper_and_never_holds_the_secret() {
        let e = env("C:\\tg\\termigator.exe", "termigator-askpass-u-1", "tok");
        assert!(e.contains(&(
            "SSH_ASKPASS".to_owned(),
            "C:\\tg\\termigator.exe".to_owned()
        )));
        assert!(e.contains(&("SSH_ASKPASS_REQUIRE".to_owned(), "force".to_owned())));
        assert!(e.iter().any(|(k, _)| k == CHANNEL_ENV));
        assert!(e.contains(&(TOKEN_ENV.to_owned(), "tok".to_owned())));
        assert!(e.iter().all(|(k, _)| k != "SSHPASS"));
    }

    #[test]
    fn helper_is_recognised_by_the_channel() {
        assert!(is_helper_invocation(Some("x")));
        assert!(!is_helper_invocation(Some("")));
        assert!(!is_helper_invocation(None));
    }

    #[test]
    fn env_for_registers_one_token_per_password() {
        let (a, b) = (Secret::new(b"a".to_vec()), Secret::new(b"b".to_vec()));
        let Some(first) = env_for(&a, None, "/x/termigator") else {
            return; // no private runtime directory on this machine
        };
        let get = |e: &[(String, String)], k: &str| {
            e.iter().find(|(n, _)| n == k).map(|(_, v)| v.clone())
        };
        let again = env_for(&a, None, "/x/termigator").expect("again");
        let other = env_for(&b, None, "/x/termigator").expect("other");
        assert_eq!(get(&first, TOKEN_ENV), get(&again, TOKEN_ENV));
        assert_ne!(get(&first, TOKEN_ENV), get(&other, TOKEN_ENV));
        let path = PathBuf::from(get(&first, CHANNEL_ENV).expect("channel"));
        let token = get(&other, TOKEN_ENV).expect("token");
        assert_eq!(
            fetch(&path, &token, "Password: ")
                .expect("fetch")
                .as_deref(),
            Some(&b"b\n"[..])
        );
    }

    #[test]
    fn server_answers_password_prompts_for_the_right_token_only() {
        let dir = std::env::temp_dir().join(format!("tg-askpass-{}", std::process::id()));
        let path = dir.join("ask.sock");
        let server = Server::start(&path).expect("start");
        server.add("tok", Secret::new(b"hunter2".to_vec()), None);
        let jump = Target {
            user: "u".to_owned(),
            host: "h".to_owned(),
        };
        server.add("jtok", Secret::new(b"j".to_vec()), Some(jump));
        assert!(
            fetch(&path, "jtok", "u@h's password: ")
                .expect("fetch")
                .is_some()
        );
        assert!(
            fetch(&path, "jtok", "u@bastion's password: ")
                .expect("fetch")
                .is_none()
        );
        let got = fetch(&path, "tok", "u@h's password: ").expect("fetch");
        assert_eq!(got.as_deref(), Some(&b"hunter2\n"[..]));
        // Second prompt on the same channel (ssh retries after a wrong password).
        assert!(fetch(&path, "tok", "Password:").expect("fetch").is_some());
        assert!(fetch(&path, "bad", "Password:").expect("fetch").is_none());
        assert!(
            fetch(&path, "tok", "Enter passphrase for key 'k': ")
                .expect("fetch")
                .is_none()
        );
        assert!(
            fetch(&path, "tok", "Continue (yes/no)? ")
                .expect("fetch")
                .is_none()
        );
        drop(server);
        assert!(!path.exists(), "socket file left behind");
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn channel_names_are_safe_and_unique() {
        let a = channel_name("Domain\\User name", 1);
        assert_eq!(a, "termigator-askpass-Domain_User_name-0000000000000001");
        assert_ne!(a, channel_name("Domain\\User name", 2));
    }
}
