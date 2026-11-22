//! Killable command with timeout and byte output.
//!
//! One argv runs on worker threads, the
//! GUI thread never blocks, and exactly one [`Outcome`] arrives on a channel.
//!
//! * The child gets exactly [`Command::env`] (the environment is cleared first) and runs in its
//!   own process group, so cancel and timeout reach grandchildren too (`ssh` helpers, `sleep`).
//! * Cancel/timeout send `SIGTERM` to the group, then `SIGKILL` after [`KILL_GRACE`]. The leader
//!   is only reaped after the last signal, so a recycled pid can never be signalled.
//! * stdout/stderr are raw bytes (ADR-12). Each stream keeps at most
//!   [`Command::max_output`] bytes (default [`DEFAULT_MAX_OUTPUT`], 32 MiB); the rest is read
//!   and discarded so the child never blocks on a full pipe, and the matching `*_truncated`
//!   flag is set. The cap only protects against runaway output.
//! * Empty [`Command::stdin`] means `/dev/null`; otherwise the bytes are written
//!   and the pipe closed.
//! * A grandchild that keeps stdout/stderr open after the leader
//!   exits delays completion until it closes them; set a timeout to bound that.
//! * A spawn failure (missing binary) is an `Err` from [`spawn`], not an outcome.

use std::{
    collections::BTreeMap,
    ffi::OsString,
    io::{Read, Write},
    process::{Child, ExitStatus, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

use crate::platform;

/// Default per-stream capture cap: 32 MiB.
pub const DEFAULT_MAX_OUTPUT: usize = 32 * 1024 * 1024;
/// Time between `SIGTERM` and `SIGKILL`.
pub const KILL_GRACE: Duration = Duration::from_millis(500);
const TICK: Duration = Duration::from_millis(5);

#[derive(Debug, thiserror::Error)]
pub enum RunnerError {
    #[error("empty argv")]
    EmptyArgv,
    #[error("cannot start {program:?}: {source}")]
    Spawn {
        program: OsString,
        #[source]
        source: std::io::Error,
    },
}

/// What to run. Nothing is inherited: the caller supplies the whole environment.
#[derive(Debug, Clone)]
pub struct Command {
    pub argv: Vec<OsString>,
    pub env: BTreeMap<OsString, OsString>,
    pub stdin: Vec<u8>,
    pub timeout: Option<Duration>,
    pub max_output: usize,
}

impl Command {
    pub fn new<I, S>(argv: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<OsString>,
    {
        Self {
            argv: argv.into_iter().map(Into::into).collect(),
            env: BTreeMap::new(),
            stdin: Vec::new(),
            timeout: None,
            max_output: DEFAULT_MAX_OUTPUT,
        }
    }
}

/// Final result, delivered once.
#[derive(Debug)]
pub struct Outcome {
    /// Exit code / terminating signal of the process-group leader.
    pub status: ExitStatus,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub stdout_truncated: bool,
    pub stderr_truncated: bool,
    /// The timeout fired first and the group was killed.
    pub timed_out: bool,
    /// [`CancelHandle::cancel`] came first and the group was killed.
    pub cancelled: bool,
}

impl Outcome {
    /// Exit code, or `128 + signal` like a shell.
    pub fn code(&self) -> i32 {
        self.status
            .code()
            .or_else(|| platform::exit_signal(&self.status).map(|s| 128 + s))
            .unwrap_or(-1)
    }
}

/// Cloneable, thread-safe; cancelling a finished command is a no-op.
#[derive(Debug, Clone, Default)]
pub struct CancelHandle(Arc<AtomicBool>);

impl CancelHandle {
    pub fn cancel(&self) {
        self.0.store(true, Ordering::SeqCst);
    }
}

/// A started command.
#[derive(Debug)]
pub struct Running {
    cancel: CancelHandle,
    rx: Receiver<Outcome>,
}

impl Running {
    pub fn cancel_handle(&self) -> CancelHandle {
        self.cancel.clone()
    }

    /// The channel that receives the single [`Outcome`].
    pub fn receiver(&self) -> &Receiver<Outcome> {
        &self.rx
    }

    /// Block until done (tests, non-GUI callers). `None` only if the supervisor thread died.
    pub fn wait(self) -> Option<Outcome> {
        self.rx.recv().ok()
    }
}

/// Start `cmd`; the spawn itself happens here so a missing binary is reported immediately.
static SPAWN_HOOK: std::sync::OnceLock<fn(&std::process::Child)> = std::sync::OnceLock::new();

/// Called with every process [`spawn`] starts, right after it started. The app uses it on Windows
/// to put tunnels and transfers into a kill-on-close Job Object, which needs the
/// WinAPI this crate does not call. Set once; later calls are ignored.
pub fn set_spawn_hook(hook: fn(&std::process::Child)) {
    let _ = SPAWN_HOOK.set(hook);
}

pub fn spawn(mut cmd: Command) -> Result<Running, RunnerError> {
    let program = cmd.argv.first().ok_or(RunnerError::EmptyArgv)?.clone();
    let mut c = std::process::Command::new(&program);
    c.args(&cmd.argv[1..])
        .env_clear()
        .envs(&cmd.env)
        .stdin(if cmd.stdin.is_empty() {
            Stdio::null()
        } else {
            Stdio::piped()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    platform::background_process(&mut c);
    // Win32-OpenSSH never exits while its stdout is a pipe made by std, nor while its stderr is
    // one after a refused login, so on Windows both go to private temp files read after the
    // child exits.
    // shortcut: no live output on Windows; upgrade if a streaming consumer of `spawn` appears.
    #[cfg(windows)]
    let out_files = {
        static N: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let file = |stream: &str| {
            let p = std::env::temp_dir().join(format!(
                "termigator-{stream}-{}-{}",
                std::process::id(),
                N.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::File::create(&p)
                .map(|f| (Stdio::from(f), p))
                .map_err(|source| RunnerError::Spawn {
                    program: program.clone(),
                    source,
                })
        };
        let (out, out_path) = file("out")?;
        let (err, err_path) = match file("err") {
            Ok(v) => v,
            Err(e) => {
                let _ = std::fs::remove_file(&out_path);
                return Err(e);
            }
        };
        c.stdout(out).stderr(err);
        Some((out_path, err_path))
    };
    #[cfg(not(windows))]
    let out_files: Option<(std::path::PathBuf, std::path::PathBuf)> = None;
    let spawned = c.spawn();
    // shortcut: std::process::Command frees its own copy of the env unwiped, upgrade if std ever
    // offers a zeroizing env.
    drop(c);
    crate::vault::wipe_env(&mut cmd.env);
    let mut child = spawned.map_err(|source| RunnerError::Spawn { program, source })?;
    if let Some(hook) = SPAWN_HOOK.get() {
        hook(&child);
    }

    let cap = cmd.max_output;
    let out = child.stdout.take().map(|r| capture(r, cap));
    let err = child.stderr.take().map(|r| capture(r, cap));
    if let Some(mut w) = child.stdin.take() {
        let data = cmd.stdin;
        // A child that exits without reading gives EPIPE; that is not an error here.
        thread::spawn(move || {
            let _ = w.write_all(&data);
        });
    }

    let cancel = CancelHandle::default();
    let (tx, rx) = mpsc::channel();
    let flag = cancel.clone();
    let timeout = cmd.timeout;
    thread::spawn(move || {
        let _ = tx.send(supervise(child, out, out_files, cap, err, &flag, timeout));
    });
    Ok(Running { cancel, rx })
}

type Captured = (Vec<u8>, bool);

fn capture<R: Read + Send + 'static>(mut r: R, cap: usize) -> JoinHandle<Captured> {
    thread::spawn(move || {
        let (mut data, mut truncated) = (Vec::new(), false);
        let mut buf = [0u8; 8192];
        while let Ok(n) = r.read(&mut buf) {
            if n == 0 {
                break;
            }
            let room = cap.saturating_sub(data.len());
            data.extend_from_slice(&buf[..n.min(room)]);
            truncated |= n > room;
        }
        (data, truncated)
    })
}

fn supervise(
    mut child: Child,
    out: Option<JoinHandle<Captured>>,
    out_files: Option<(std::path::PathBuf, std::path::PathBuf)>,
    cap: usize,
    err: Option<JoinHandle<Captured>>,
    cancel: &CancelHandle,
    timeout: Option<Duration>,
) -> Outcome {
    let pid = child.id();
    let deadline = timeout.map(|t| Instant::now() + t);
    let (mut timed_out, mut cancelled) = (false, false);
    let mut term_at: Option<Instant> = None;
    let mut killed = false;
    let finished =
        |h: &Option<JoinHandle<Captured>>| h.as_ref().is_none_or(JoinHandle::is_finished);

    // The leader stays unreaped until the loop ends, so signalling its group is always safe.
    while !(platform::has_exited(&mut child) && finished(&out) && finished(&err)) {
        if let Some(t) = term_at {
            if !killed && t.elapsed() >= KILL_GRACE {
                platform::group_kill(pid);
                killed = true;
            }
        } else {
            let by_cancel = cancel.0.load(Ordering::SeqCst);
            let by_time = deadline.is_some_and(|d| Instant::now() >= d);
            if by_cancel || by_time {
                cancelled = by_cancel;
                timed_out = !by_cancel;
                platform::group_terminate(pid);
                term_at = Some(Instant::now());
            }
        }
        thread::sleep(TICK);
    }
    let status = child
        .wait()
        .unwrap_or_else(|_| platform::exit_status_success());
    let join = |h: Option<JoinHandle<Captured>>| h.and_then(|h| h.join().ok()).unwrap_or_default();
    let read = |path: &std::path::Path| {
        let data = std::fs::read(path).unwrap_or_default();
        let _ = std::fs::remove_file(path);
        let truncated = data.len() > cap;
        (data.into_iter().take(cap).collect::<Vec<u8>>(), truncated)
    };
    let ((stdout, stdout_truncated), (stderr, stderr_truncated)) = match out_files {
        Some((out_path, err_path)) => (read(&out_path), read(&err_path)),
        None => (join(out), join(err)),
    };
    Outcome {
        status,
        stdout,
        stderr,
        stdout_truncated,
        stderr_truncated,
        timed_out,
        cancelled,
    }
}
