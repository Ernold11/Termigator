//! Supervised `ssh -N` tunnel processes and the gateway local port.
//!
//! The process half of the tunnel manager plus `gateway.free_local_port`.
//! Built on [`super::runner`], so the child gets its own process group and stop is `SIGTERM` to
//! the group, `SIGKILL` after [`runner::KILL_GRACE`], with the leader reaped only afterwards.
//!
//! * [`Tunnel::start`] never blocks. Events arrive on [`Tunnel::events`]: `Starting` at once,
//!   `Running` once the process survived the settle time ([`SETTLE`], 1.5 s), then exactly one
//!   terminal event: `Stopped` after [`Tunnel::stop`] (or drop), otherwise `Failed`. A process
//!   that exits before the settle time never reports `Running`.
//! * [`Tunnel::stop`] only raises a flag; the supervisor thread does the signalling, so
//!   stopping never blocks the GUI thread.
//! * The child environment is the current process environment overlaid with the caller's map
//!   (like `QProcessEnvironment`), so secrets such as `SSH_ASKPASS` variables travel
//!   in the environment only, never in argv.
//! * Output is merged for the failure text from merged channels: the last 20 non-empty
//!   lines (stderr, then stdout), minus ssh's `Warning: Permanently added` notices.
//! * The runner does not expose the pid, so [`TunnelEvent::Running`] carries none.

use std::{
    collections::BTreeMap,
    ffi::OsString,
    io,
    net::TcpListener,
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, RecvTimeoutError},
    },
    thread,
    time::Duration,
};

use super::runner::{self, CancelHandle, Command, Outcome};

/// Time a process must stay alive before the tunnel counts as running.
pub const SETTLE: Duration = Duration::from_millis(1500);
const TAIL_LINES: usize = 20;

/// Status changes of one tunnel, in order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TunnelEvent {
    Starting,
    Running,
    /// The process exited (or could not start) without [`Tunnel::stop`].
    Failed {
        /// Exit code (`128 + signal` if killed); `None` when the process never started.
        code: Option<i32>,
        /// Last lines of output, newline-joined (for a spawn failure, the error text).
        stderr_tail: String,
    },
    Stopped,
}

impl TunnelEvent {
    /// The status detail shown for `Failed`: ssh's last line, else its exit code.
    pub fn detail(&self) -> Option<String> {
        let Self::Failed { code, stderr_tail } = self else {
            return None;
        };
        Some(match stderr_tail.lines().last() {
            Some(l) => l.to_owned(),
            None => format!("ssh exited with code {}", code.unwrap_or(-1)),
        })
    }
}

/// A started tunnel; dropping it stops the process.
#[derive(Debug)]
pub struct Tunnel {
    stopping: Arc<AtomicBool>,
    cancel: Option<CancelHandle>,
    rx: Receiver<TunnelEvent>,
}

/// The current environment overlaid with `extra`.
pub fn with_system_env(extra: &BTreeMap<OsString, OsString>) -> BTreeMap<OsString, OsString> {
    let mut env: BTreeMap<_, _> = std::env::vars_os().collect();
    env.extend(extra.iter().map(|(k, v)| (k.clone(), v.clone())));
    env
}

/// Linux: `argv` behind util-linux `setpriv --pdeathsig TERM`, so the tunnel ends with Termigator
/// even when it crashes (without it a killed app leaves `ssh -N` holding the ports).
/// The signal follows the thread that started the child: [`Tunnel::start`] runs on the caller's
/// thread, which in the app is the GUI thread. Unchanged when setpriv or the program is missing,
/// so a missing ssh is still reported as such.
fn with_parent_death_signal(argv: &[String]) -> Vec<String> {
    let found = |p: &str| {
        if Path::new(p).is_absolute() {
            crate::platform::is_executable(Path::new(p))
        } else {
            crate::platform::find_in_path(p).is_some()
        }
    };
    match argv.first() {
        Some(prog) if cfg!(target_os = "linux") && found(prog) && found("setpriv") => {
            ["setpriv", "--pdeathsig", "TERM", "--"]
                .into_iter()
                .map(str::to_owned)
                .chain(argv.iter().cloned())
                .collect()
        }
        _ => argv.to_vec(),
    }
}

impl Tunnel {
    /// Start `argv` (from `proto::tunnels::tunnel_argv`) with the default [`SETTLE`].
    pub fn start(argv: &[String], env: &BTreeMap<OsString, OsString>) -> Self {
        Self::start_with_settle(argv, env, SETTLE)
    }

    pub fn start_with_settle(
        argv: &[String],
        env: &BTreeMap<OsString, OsString>,
        settle: Duration,
    ) -> Self {
        let (tx, rx) = mpsc::channel();
        let stopping = Arc::new(AtomicBool::new(false));
        let _ = tx.send(TunnelEvent::Starting);
        let argv = with_parent_death_signal(argv);
        let mut cmd = Command::new(argv.iter().map(String::as_str));
        cmd.env = with_system_env(env);
        let running = match runner::spawn(cmd) {
            Ok(r) => r,
            Err(e) => {
                let stderr_tail = match &e {
                    runner::RunnerError::EmptyArgv => "cannot build the ssh command".to_owned(),
                    runner::RunnerError::Spawn { program, .. } => {
                        format!("cannot start '{}'", program.to_string_lossy())
                    }
                };
                let _ = tx.send(TunnelEvent::Failed {
                    code: None,
                    stderr_tail,
                });
                return Self {
                    stopping,
                    cancel: None,
                    rx,
                };
            }
        };
        let cancel = running.cancel_handle();
        let flag = Arc::clone(&stopping);
        thread::spawn(move || {
            let outcome = match running.receiver().recv_timeout(settle) {
                Ok(o) => Some(o),
                Err(RecvTimeoutError::Timeout) => {
                    if !flag.load(Ordering::SeqCst) {
                        let _ = tx.send(TunnelEvent::Running);
                    }
                    running.receiver().recv().ok()
                }
                Err(RecvTimeoutError::Disconnected) => None,
            };
            let _ = tx.send(finish(outcome, flag.load(Ordering::SeqCst)));
        });
        Self {
            stopping,
            cancel: Some(cancel),
            rx,
        }
    }

    /// Status events; ends after the terminal event.
    pub fn events(&self) -> &Receiver<TunnelEvent> {
        &self.rx
    }

    /// Ask the tunnel to end; returns at once. `Stopped` arrives on [`Self::events`] once the
    /// process group is gone (`SIGKILL` follows `SIGTERM` after the runner's grace).
    pub fn stop(&self) {
        self.stopping.store(true, Ordering::SeqCst);
        if let Some(c) = &self.cancel {
            c.cancel();
        }
    }
}

impl Drop for Tunnel {
    fn drop(&mut self) {
        self.stop();
    }
}

fn finish(outcome: Option<Outcome>, stopping: bool) -> TunnelEvent {
    if stopping {
        return TunnelEvent::Stopped;
    }
    let Some(o) = outcome else {
        return TunnelEvent::Failed {
            code: None,
            stderr_tail: "tunnel supervisor failed".to_owned(),
        };
    };
    let text = format!(
        "{}\n{}",
        String::from_utf8_lossy(&o.stderr),
        String::from_utf8_lossy(&o.stdout)
    )
    .replace('\r', "\n");
    let lines: Vec<&str> = text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect();
    let tail = &lines[lines.len().saturating_sub(TAIL_LINES)..];
    let stderr_tail = tail
        .iter()
        .filter(|l| !l.starts_with("Warning: Permanently added"))
        .copied()
        .collect::<Vec<_>>()
        .join("\n");
    TunnelEvent::Failed {
        code: Some(o.code()),
        stderr_tail,
    }
}

/// A free TCP port on 127.0.0.1 (bind port 0, read it back, release). Another process could
/// grab it before use.
pub fn free_local_port() -> io::Result<u16> {
    Ok(TcpListener::bind(("127.0.0.1", 0))?.local_addr()?.port())
}

#[cfg(all(test, target_os = "linux"))]
mod parent_death_tests {
    use super::*;

    #[test]
    fn tunnels_die_with_the_app_when_setpriv_is_there() {
        let argv = |xs: &[&str]| xs.iter().map(|x| (*x).to_owned()).collect::<Vec<_>>();
        if crate::platform::find_in_path("setpriv").is_some() {
            assert_eq!(
                with_parent_death_signal(&argv(&["sh", "-c", "x"])),
                argv(&["setpriv", "--pdeathsig", "TERM", "--", "sh", "-c", "x"])
            );
        }
        // A missing program keeps its own "cannot start" error.
        let missing = argv(&["no-such-ssh-xyz", "-N"]);
        assert_eq!(with_parent_death_signal(&missing), missing);
    }
}
