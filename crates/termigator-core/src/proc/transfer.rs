//! scp under a PTY with live progress, and sftp `reget` resume.
//!
//! [`start`] runs one [`TransferJob`] on a worker thread and reports through the channel of the
//! returned [`TransferHandle`]: any number of [`Event::Progress`], then exactly one of
//! [`Event::Done`], [`Event::Failed`] or [`Event::Cancelled`]. scp draws its meter only when it
//! is the foreground process of a controlling terminal, so the job runs on a [`Pty`] of
//! 24 x [`PTY_COLUMNS`] cells (the pty child is a session leader, so its pid is the process
//! group id). The worker blocks in `read` on the master; it wakes when the output ends (the
//! kernel's EIO once every holder of the slave is gone).
//!
//! The job's environment is this process's environment plus `job.env`.
//!
//! [`start_resume`] is the sftp path for an interrupted download (scp cannot resume): the caller
//! builds the sftp argv and batch text from a path chosen here (see
//! [`crate::proto::scp::sftp_resume_argv_with`]); the worker writes the batch text with mode 0600
//! into [`private_runtime_dir`] and removes the file when the job ends, however it ends.
//! A path with a line break never reaches the batch text: `sftp_resume_argv_with` refuses it.
//!
//! Final status: cancelled -> `Cancelled` (even if the process managed to exit 0);
//! exit 0 -> `Done`; otherwise `Failed` with the last non-meter line scp printed, or `exit <rc>`
//! (`-<signal>` when killed). A program that cannot be started fails with the OS error.
//!
//! Cancelling sends `SIGTERM` to the whole process group (scp and the ssh it spawned), then
//! `SIGKILL` follows after [`KILL_GRACE`], the same policy as [`crate::proc::runner`]. The
//! group is only signalled while the leader is unreaped, so the pid cannot belong to anyone else
//!
//!
//! The sftp batch file is made with `O_EXCL` in the private runtime dir, not in `$TMPDIR`; the
//! worker reaps the child itself, so a cancelled job reports `Cancelled`, not `-15`.

use std::{
    collections::BTreeMap,
    ffi::OsString,
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex, MutexGuard, PoisonError,
        mpsc::{self, Receiver, Sender},
    },
    thread,
    time::{SystemTime, UNIX_EPOCH},
};

use crate::{
    config::model::strip_space,
    paths::private_runtime_dir,
    platform,
    proc::{
        pty::{Pty, Size, Spawn},
        runner::KILL_GRACE,
    },
    remote::transfers::{PTY_COLUMNS, TransferJob, parse_progress, split_updates},
};

const PTY_ROWS: u16 = 24;

/// What a worker reports. `Done`, `Failed` and `Cancelled` are final.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    Progress {
        percent: u8,
        size: String,
        speed: String,
        eta: String,
    },
    Done,
    Failed {
        message: String,
    },
    Cancelled,
}

/// Argv (program first) and batch text for [`start_resume`].
pub type ResumePlan = (Vec<String>, String);

#[derive(Default)]
struct State {
    /// The leader's pid while it is unreaped.
    pid: Option<u32>,
    cancelled: bool,
}

/// A running (or finished) transfer.
pub struct TransferHandle {
    shared: Arc<Mutex<State>>,
    events: Receiver<Event>,
}

impl TransferHandle {
    /// The events of this job; disconnects after the final one.
    pub fn events(&self) -> &Receiver<Event> {
        &self.events
    }

    /// Stops the job: `SIGTERM` to its process group, `SIGKILL` after [`KILL_GRACE`]. Safe to
    /// call at any time, also before the process exists and after it ended.
    pub fn cancel(&self) {
        let mut st = lock(&self.shared);
        st.cancelled = true;
        terminate(&self.shared, &st);
    }
}

fn lock(shared: &Mutex<State>) -> MutexGuard<'_, State> {
    shared.lock().unwrap_or_else(PoisonError::into_inner)
}

/// TERM now, KILL later, only while the leader is unreaped (`pid` is `Some`).
fn terminate(shared: &Arc<Mutex<State>>, st: &State) {
    let Some(pid) = st.pid else { return };
    platform::group_terminate(pid);
    let shared = Arc::clone(shared);
    thread::spawn(move || {
        thread::sleep(KILL_GRACE);
        if let Some(pid) = lock(&shared).pid {
            platform::group_kill(pid);
        }
    });
}

/// Runs `job` (its `argv` and `env`) on a worker thread.
pub fn start(job: TransferJob) -> TransferHandle {
    spawn_worker(job, None)
}

/// Resumes an interrupted download with sftp. `build` gets the path of the (not yet existing)
/// batch file and returns the sftp argv and the batch text; an `Err` becomes `Failed`. `job`
/// supplies the environment; its `argv` is ignored.
pub fn start_resume<F>(job: TransferJob, build: F) -> TransferHandle
where
    F: FnOnce(&str) -> Result<ResumePlan, String> + Send + 'static,
{
    spawn_worker(job, Some(Box::new(build)))
}

type Build = Box<dyn FnOnce(&str) -> Result<ResumePlan, String> + Send>;

fn spawn_worker(job: TransferJob, build: Option<Build>) -> TransferHandle {
    let shared = Arc::new(Mutex::new(State::default()));
    let (tx, rx) = mpsc::channel();
    let worker_shared = Arc::clone(&shared);
    let worker_tx = tx.clone();
    let name = format!("transfer-{}", job.id);
    let spawned = thread::Builder::new().name(name).spawn(move || {
        let last = run(job, build, &worker_shared, &worker_tx);
        let _ = worker_tx.send(last);
    });
    if let Err(e) = spawned {
        let _ = tx.send(Event::Failed {
            message: e.to_string(),
        });
    }
    TransferHandle { shared, events: rx }
}

/// Removes the batch file when dropped.
struct BatchFile(PathBuf);

impl Drop for BatchFile {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

fn failed(message: impl Into<String>) -> Event {
    Event::Failed {
        message: message.into(),
    }
}

fn run(
    job: TransferJob,
    build: Option<Build>,
    shared: &Arc<Mutex<State>>,
    tx: &Sender<Event>,
) -> Event {
    let mut argv: Vec<OsString> = job
        .argv
        .into_iter()
        .map(platform::os_string_from_vec)
        .collect();
    let mut batch_guard = None;
    if let Some(build) = build {
        let Some(dir) = private_runtime_dir(&[]) else {
            return failed("Cannot create a private directory for the sftp batch file.");
        };
        let path = batch_path(&dir, job.id);
        let Some(path_str) = path.to_str() else {
            return failed("The sftp batch file path is not valid UTF-8.");
        };
        let (plan_argv, text) = match build(path_str) {
            Ok(plan) => plan,
            Err(e) => return failed(e),
        };
        let guard = batch_guard.insert(BatchFile(path.clone()));
        if let Err(e) = write_private(&guard.0, &text) {
            return failed(e.to_string());
        }
        argv = plan_argv.into_iter().map(OsString::from).collect();
    }

    let mut env: BTreeMap<OsString, OsString> = std::env::vars_os().collect();
    env.extend(job.env.into_iter().map(|(k, v)| {
        (
            OsString::from(k),
            platform::os_string_from_vec(v.expose().to_vec()),
        )
    }));
    let mut spec = Spawn {
        argv,
        env,
        cwd: std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/")),
        size: Size {
            cols: PTY_COLUMNS,
            rows: PTY_ROWS,
        },
    };
    let spawned = Pty::spawn(&spec);
    crate::vault::wipe_env(&mut spec.env);
    let mut pty = match spawned {
        Ok(p) => p,
        Err(e) => return failed(e.to_string()),
    };
    {
        let mut st = lock(shared);
        st.pid = Some(pty.pid());
        if st.cancelled {
            terminate(shared, &st); // cancelled before the process existed
        }
    }

    let mut out = match Output::new(&mut pty) {
        Ok(o) => o,
        Err(e) => return failed(e.to_string()),
    };
    let mut pending = String::new();
    let mut last_text = String::new();
    let mut buf = [0u8; 4096];
    loop {
        let n = out.read(&mut pty, &mut buf);
        if n == 0 {
            break; // the pty closed: the job is over
        }
        let chunk = String::from_utf8_lossy(&buf[..n]);
        let (lines, rest) = split_updates(&pending, &chunk);
        pending = rest;
        for line in lines {
            match parse_progress(&line) {
                Some(p) => {
                    let _ = tx.send(Event::Progress {
                        percent: p.percent,
                        size: p.size,
                        speed: p.rate,
                        eta: p.eta,
                    });
                }
                None => last_text = strip_space(&line).to_owned(),
            }
        }
    }
    // From here no signal may reach the pid: it is reaped next.
    let cancelled = {
        let mut st = lock(shared);
        st.pid = None;
        st.cancelled
    };
    let status = pty.wait();
    drop(batch_guard);
    if cancelled {
        return Event::Cancelled;
    }
    if !strip_space(&pending).is_empty() && parse_progress(&pending).is_none() {
        last_text = strip_space(&pending).to_owned();
    }
    match status {
        Ok(s) if s.success() => Event::Done,
        Ok(s) => {
            let rc = s
                .code()
                .or_else(|| platform::exit_signal(&s).map(|n| -n))
                .unwrap_or(-1);
            failed(if last_text.is_empty() {
                format!("exit {rc}")
            } else {
                last_text
            })
        }
        Err(e) => failed(e.to_string()),
    }
}

/// The job's output. Unix reads the pty itself (EIO once the child is gone). A ConPTY never reports
/// the child's exit as EOF, so on Windows a thread reads and the exit is noticed through `try_wait`.
#[cfg(unix)]
struct Output;

#[cfg(unix)]
impl Output {
    fn new(_pty: &mut Pty) -> std::io::Result<Self> {
        Ok(Self)
    }

    fn read(&mut self, pty: &mut Pty, buf: &mut [u8]) -> usize {
        pty.read(buf).unwrap_or(0)
    }
}

#[cfg(windows)]
struct Output {
    rx: Receiver<Vec<u8>>,
    input: Box<dyn Write + Send>, // dropping the writer would close the child's input early
}

#[cfg(windows)]
impl Output {
    fn new(pty: &mut Pty) -> std::io::Result<Self> {
        use std::io::Read;
        let (mut reader, input) = pty.take_streams().map_err(std::io::Error::other)?;
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            let mut b = [0u8; 4096];
            while let Ok(n) = reader.read(&mut b) {
                if n == 0 || tx.send(b[..n].to_vec()).is_err() {
                    break;
                }
            }
        });
        Ok(Self { rx, input })
    }

    fn read(&mut self, pty: &mut Pty, buf: &mut [u8]) -> usize {
        use std::{sync::mpsc::RecvTimeoutError, time::Duration};
        let mut wait = Duration::from_millis(100);
        loop {
            match self.rx.recv_timeout(wait) {
                Ok(mut v) => {
                    // ConPTY asks where the cursor is (`ESC[6n`) and stops the child until a host
                    // answers; there is no terminal here, so say "row 1, column 1" and hide the query.
                    const CPR: &[u8] = b"\x1b[6n";
                    while let Some(i) = v.windows(CPR.len()).position(|w| w == CPR) {
                        v.drain(i..i + CPR.len());
                        let _ = self.input.write_all(b"\x1b[1;1R");
                        let _ = self.input.flush();
                    }
                    if v.is_empty() {
                        continue;
                    }
                    buf[..v.len()].copy_from_slice(&v);
                    return v.len();
                }
                Err(RecvTimeoutError::Disconnected) => return 0,
                // Exited: one last short wait for output still in flight, then the job is over.
                Err(RecvTimeoutError::Timeout) if matches!(pty.try_wait(), Ok(Some(_))) => {
                    if wait == Duration::from_millis(100) {
                        wait = Duration::from_millis(300);
                    } else {
                        return 0;
                    }
                }
                Err(RecvTimeoutError::Timeout) => {}
            }
        }
    }
}

fn batch_path(dir: &Path, job_id: u64) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.subsec_nanos());
    dir.join(format!(
        "termigator-resume-{}-{job_id}-{nanos}.sftp",
        std::process::id()
    ))
}

fn write_private(path: &Path, text: &str) -> std::io::Result<()> {
    let mut f =
        platform::private_file(OpenOptions::new().write(true).create_new(true)).open(path)?;
    f.write_all(text.as_bytes())
}
