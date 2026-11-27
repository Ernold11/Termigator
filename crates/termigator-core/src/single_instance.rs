//! Control socket: `termigator --toggle` / `--show` steer the running window.
//!
//! Wire protocol (stable across versions): the client sends one line
//! `toggle|show [TOKEN]\n` (TOKEN = the Wayland `XDG_ACTIVATION_TOKEN`), the server answers
//! `ok\n` or `err\n` and closes. The socket is `$TERMIGATOR_CONTROL_SOCKET` or
//! `control.sock` in the user's private runtime directory (`paths::private_runtime_dir`); with
//! none there is no control (never a socket in a shared place).
//!
//! * Client: [`send_command`] / [`send`] (2 s timeout) and [`socket_alive`].
//! * Server: [`ControlServer::start`] binds the socket (a stale file left by a crash is replaced,
//!   a live one is never stolen), accepts on a background thread and hands every valid line to
//!   the application as a [`Request`] through [`ControlServer::requests`]; the application thread
//!   decides the reply with [`Request::respond`] (no answer within 1.5 s, or a dropped request,
//!   means `err`). Dropping the server stops the thread and removes the socket file.
//!   At most 8 clients are served at once; more are closed at once. Only the holder of an
//!   exclusive `flock` on `<path>.lock` checks and replaces a stale socket file.
//! * [`claim`]: the single-instance start (`window.single_instance`). Become the server, or hand
//!   `show` to the Termigator that is; only the `flock` holder is ever the one, so two starts at
//!   once cannot both win, and a crashed one leaves nothing that blocks (the kernel drops the lock,
//!   the stale socket is replaced).
//! * [`split_remote_args`] picks `--toggle` / `--show` out of the command line.

use std::ffi::OsString;
use std::fs;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use crate::paths::{EnvSource, Layout, private_runtime_dir_in};
use crate::platform::{self, IpcListener, IpcStream};

const CLIENT_TIMEOUT: Duration = Duration::from_secs(2);
const READ_TIMEOUT: Duration = Duration::from_secs(1);
const REPLY_WAIT: Duration = Duration::from_millis(1500);
const POLL: Duration = Duration::from_millis(25);
/// A control line is a command and a token; anything longer is not one.
const MAX_LINE: u64 = 4096;
/// Connections served at once; more are closed at once (the client sees a dead server).
const MAX_CONNECTIONS: usize = 8;
/// Pause between [`claim`] attempts while the other Termigator is still starting.
const CLAIM_RETRY: Duration = Duration::from_millis(100);
/// How long a second start waits for the running Termigator to answer: enough for a slow first
/// start (config, QML) to begin serving, short enough that a hung one does not hold the user up.
pub const HAND_OVER_WAIT: Duration = Duration::from_secs(10);

/// A control command.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Command {
    Toggle,
    Show,
}

impl Command {
    pub fn as_str(self) -> &'static str {
        match self {
            Command::Toggle => "toggle",
            Command::Show => "show",
        }
    }

    fn parse(s: &str) -> Option<Command> {
        match s {
            "toggle" => Some(Command::Toggle),
            "show" => Some(Command::Show),
            _ => None,
        }
    }
}

/// `send` got a command that is neither `toggle` nor `show`.
#[derive(Debug, thiserror::Error)]
#[error("unknown control command: {0:?}")]
pub struct InvalidCommand(pub String);

/// The control socket path; `None` when there is no private runtime directory.
pub fn control_socket_path(env: &dyn EnvSource) -> Option<PathBuf> {
    if let Some(p) = env
        .var("TERMIGATOR_CONTROL_SOCKET")
        .filter(|p| !p.is_empty())
    {
        return Some(PathBuf::from(p));
    }
    let dir = private_runtime_dir_in(
        Layout::ACTIVE,
        env,
        &std::env::temp_dir(),
        platform::current_uid(),
        &[],
    )?;
    Some(dir.join("control.sock"))
}

/// `b"toggle TOKEN\n"` -> `(Toggle, "TOKEN")`; `None` for anything else (bad UTF-8 included).
pub fn parse_line(line: &[u8]) -> Option<(Command, String)> {
    let text = String::from_utf8_lossy(line);
    let mut parts = text.split_whitespace();
    let command = Command::parse(parts.next()?)?;
    let token = parts.next().unwrap_or("");
    if parts.next().is_some() {
        return None;
    }
    Some((command, token.to_owned()))
}

/// Deliver *command* to the instance listening on *path*. `Ok(false)`: none listening (or it
/// refused); `Err`: *command* is not a control command.
pub fn send(path: &Path, command: &str, token: &str) -> Result<bool, InvalidCommand> {
    let command = Command::parse(command).ok_or_else(|| InvalidCommand(command.to_owned()))?;
    Ok(send_io(path, command, token).unwrap_or(false))
}

/// [`send`] to the socket [`control_socket_path`] names; `Ok(false)` when there is none.
pub fn send_command(
    env: &dyn EnvSource,
    command: &str,
    token: &str,
) -> Result<bool, InvalidCommand> {
    match control_socket_path(env) {
        Some(p) => send(&p, command, token),
        None => Command::parse(command)
            .map(|_| false)
            .ok_or_else(|| InvalidCommand(command.to_owned())),
    }
}

fn send_io(path: &Path, command: Command, token: &str) -> io::Result<bool> {
    let mut s = IpcStream::connect(path)?;
    s.set_read_timeout(Some(CLIENT_TIMEOUT))?;
    s.set_write_timeout(Some(CLIENT_TIMEOUT))?;
    let line = format!("{} {token}", command.as_str());
    s.write_all(format!("{}\n", line.trim()).as_bytes())?;
    let mut reply = [0u8; 16];
    let n = s.read(&mut reply)?;
    Ok(reply[..n].starts_with(b"ok"))
}

/// Is a running Termigator listening on *path* (vs a socket file left by a crash)? A local
/// connect succeeds or fails at once, so no timeout is needed.
pub fn socket_alive(path: &Path) -> bool {
    IpcStream::connect(path).is_ok()
}

/// What [`claim`] found.
#[derive(Debug)]
pub enum Claim {
    /// No other Termigator: this process is the one. Keep the server for its whole life.
    Primary(ControlServer),
    /// The running Termigator took the command: this process is not needed.
    HandedOver,
    /// Another Termigator holds the lock but did not answer within the wait (hung, or stuck in
    /// its start). This process must not become a second one.
    NoAnswer,
}

/// The single-instance start on *path*: [`ControlServer::start`], else deliver *command* to the
/// server there, retried until *wait* runs out while the other process is still starting (lock
/// held, socket not bound or not served yet). A retry may deliver *command* twice, so it should be
/// one that does no harm then ([`Command::Show`]). `Err`: no server could be set up at *path*
/// (e.g. something else than a socket there).
pub fn claim(path: &Path, command: Command, token: &str, wait: Duration) -> io::Result<Claim> {
    let deadline = Instant::now() + wait;
    loop {
        if let Some(server) = ControlServer::start(path)? {
            return Ok(Claim::Primary(server));
        }
        if send_io(path, command, token).unwrap_or(false) {
            return Ok(Claim::HandedOver);
        }
        if Instant::now() >= deadline {
            return Ok(Claim::NoAnswer);
        }
        thread::sleep(CLAIM_RETRY);
    }
}

/// `--toggle` / `--show` from the command line (without the program name): the first one found
/// and the arguments with every occurrence removed.
pub fn split_remote_args<I>(args: I) -> (Option<Command>, Vec<OsString>)
where
    I: IntoIterator<Item = OsString>,
{
    let mut remote = None;
    let mut rest = Vec::new();
    for a in args {
        let c = match a.to_str() {
            Some("--toggle") => Some(Command::Toggle),
            Some("--show") => Some(Command::Show),
            _ => None,
        };
        match c {
            Some(c) => remote = remote.or(Some(c)),
            None => rest.push(a),
        }
    }
    (remote, rest)
}

/// A valid command from a client, waiting for the application's verdict.
#[derive(Debug)]
pub struct Request {
    pub command: Command,
    pub token: String,
    reply: SyncSender<bool>,
}

impl Request {
    /// Answer the client: `ok` when handled, `err` otherwise.
    pub fn respond(self, ok: bool) {
        let _ = self.reply.send(ok);
    }
}

/// The listening side. Dropping it stops the accept thread and removes the socket file.
#[derive(Debug)]
pub struct ControlServer {
    path: PathBuf,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
    requests: Receiver<Request>,
    /// `flock` on `<path>.lock`, held while the server lives: only its holder may replace a stale
    /// socket, so two starts cannot remove each other's fresh one.
    _lock: fs::File,
}

impl ControlServer {
    /// Bind *path* (private parent directory created if missing, socket mode 0600).
    /// `Ok(None)`: another Termigator answers there (or holds `<path>.lock`) and keeps the shortcut. A stale socket file is
    /// replaced; any other kind of file at *path* is an error and is left alone.
    pub fn start(path: &Path) -> io::Result<Option<ControlServer>> {
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            platform::private_dir_create(parent, true)?;
        }
        let mut lock_path = path.as_os_str().to_owned();
        lock_path.push(".lock");
        let lock = platform::private_file(
            fs::OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(false),
        )
        .open(lock_path)?;
        // Another Termigator is starting or running on this path.
        if !platform::try_lock_exclusive(&lock)? {
            return Ok(None);
        }
        if let Ok(md) = fs::symlink_metadata(path) {
            if !platform::is_ipc_endpoint(&md) {
                return Err(io::Error::new(
                    io::ErrorKind::AlreadyExists,
                    format!("{} exists and is not a socket", path.display()),
                ));
            }
            if socket_alive(path) {
                return Ok(None);
            }
            fs::remove_file(path)?;
        }
        let listener = IpcListener::bind(path)?;
        let setup = platform::set_private_file(path).and_then(|()| listener.set_nonblocking(true));
        if let Err(e) = setup {
            let _ = fs::remove_file(path);
            return Err(e);
        }
        let stop = Arc::new(AtomicBool::new(false));
        let (tx, requests) = mpsc::channel();
        let thread = {
            let stop = Arc::clone(&stop);
            thread::Builder::new()
                .name("termigator-control".into())
                .spawn(move || accept_loop(&listener, &stop, &tx))?
        };
        Ok(Some(ControlServer {
            path: path.to_owned(),
            stop,
            thread: Some(thread),
            requests,
            _lock: lock,
        }))
    }

    /// Valid commands, in arrival order; answer each with [`Request::respond`].
    pub fn requests(&self) -> &Receiver<Request> {
        &self.requests
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for ControlServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
        let _ = fs::remove_file(&self.path);
    }
}

/// Decrements the live-connection count when its handler ends (or never starts).
struct Slot(Arc<AtomicUsize>);

impl Drop for Slot {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

fn accept_loop(listener: &IpcListener, stop: &AtomicBool, tx: &mpsc::Sender<Request>) {
    let live = Arc::new(AtomicUsize::new(0));
    while !stop.load(Ordering::SeqCst) {
        match listener.accept() {
            // Over the cap: dropping `conn` closes it without blocking this loop.
            Ok(_) if live.load(Ordering::SeqCst) >= MAX_CONNECTIONS => {}
            Ok((conn, _)) => {
                live.fetch_add(1, Ordering::SeqCst);
                let slot = Slot(Arc::clone(&live));
                let tx = tx.clone();
                // One thread per connection: a slow client cannot hold up the others.
                let _ = thread::Builder::new()
                    .name("termigator-control-conn".into())
                    .spawn(move || {
                        let _slot = slot;
                        let _ = serve(conn, &tx);
                    });
            }
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(_) => thread::sleep(POLL),
        }
    }
}

fn serve(mut conn: IpcStream, tx: &mpsc::Sender<Request>) -> io::Result<()> {
    conn.set_nonblocking(false)?;
    conn.set_read_timeout(Some(READ_TIMEOUT))?;
    conn.set_write_timeout(Some(READ_TIMEOUT))?;
    let mut line = Vec::new();
    let got_line = BufReader::new((&conn).take(MAX_LINE))
        .read_until(b'\n', &mut line)
        .is_ok()
        && line.ends_with(b"\n");
    let ok = match parse_line(&line).filter(|_| got_line) {
        Some((command, token)) => {
            let (reply, answer) = mpsc::sync_channel(1);
            let req = Request {
                command,
                token,
                reply,
            };
            tx.send(req).is_ok() && answer.recv_timeout(REPLY_WAIT).unwrap_or(false)
        }
        None => false,
    };
    conn.write_all(if ok { b"ok\n" } else { b"err\n" })
}
