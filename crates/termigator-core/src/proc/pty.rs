//! PTY sessions: spawn with an explicit environment, read/write the master, resize, wait, kill
//! (the session semantics sit on top: `bash -lc` vs login shell, secrets, TERM).
//!
//! [`Pty::spawn`] opens a pty pair through portable-pty, starts `argv` on the slave as a session
//! leader with the pty as its controlling terminal, and keeps the master plus the
//! [`std::process::Child`]. The child gets exactly the environment in [`Spawn::env`] (the
//! environment of this process is neither read nor changed), except that portable-pty always sets
//! `SHELL`: from `env` when it names an executable, otherwise from the password database.
//!
//! The GUI does terminal I/O on [`Pty::master_fd`] itself (ARCHITECTURE.md §4.1, ADR-3): the bridge
//! duplicates the fd, so dropping a `Pty` hangs up the child only once the bridge has let go too.
//!
//! Signals go only to our own child and never after it was reaped: once [`Pty::try_wait`]
//! or [`Pty::wait`] has returned a status, the PID may belong to an unrelated process and
//! [`Pty::kill`] returns [`PtyError::AlreadyExited`] without signalling. Until then the PID cannot
//! be reused, because only this `Child` reaps it. portable-pty's own `ChildKiller` is not used:
//! it sends SIGHUP to the PID without that check.
//!
//! Dropping a `Pty` neither kills nor reaps the child; owners call [`Pty::kill`] and then wait,
//! or [`Pty::close`], which does both for a live child and nothing for a reaped one.
//!
//! # Production sessions
//!
//! [`Pty::spawn_session`] starts a [`SessionSpec`]: `<shell> -l` or `bash -lc <cmd>`
//! ([`ShellCommand`]) with the environment built by `session_env` (process env
//! without `TERM`, then `TERM`, `TERMIGATOR_PTY=1`, `COLORTERM=truecolor`, then the extra env).
//! Extra env values are [`Secret`]s: they are revealed only while the child is spawned, never
//! appear in argv, and `Debug` of the spec masks them.
//!
//! [`SessionSink`] consumes the output bytes the GUI thread taps (ARCHITECTURE.md 4.1) and does
//! the per-session side work synchronously: one-shot distro sniff, OSC 7 working directory,
//! optional session log and asciicast recording. Per chunk it only appends to already open files.
//! Text is scanned as Latin-1 (one char per byte), as the terminal's received-data strings.
//! Carrying a log over a restart is `start_log` on the new sink with the old
//! [`SessionSink::log_path`].

use std::{
    collections::BTreeMap,
    ffi::OsString,
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    path::{Path, PathBuf},
    process::ExitStatus,
    time::{Instant, SystemTime, UNIX_EPOCH},
};

use portable_pty::{CommandBuilder, MasterPty, PtySize, native_pty_system};

use crate::{
    platform, shell,
    text::{
        asciicast::CastRecorder,
        distro::detect_distro,
        osc7::Osc7Scanner,
        session_log::{Clock, LogFormatter},
    },
    vault::Secret,
};

/// Terminal size in character cells.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Size {
    pub cols: u16,
    pub rows: u16,
}

/// What to start on a new pty.
#[derive(Clone, Debug)]
pub struct Spawn {
    /// Program and arguments; `argv[0]` without a `/` is looked up in `env`'s `PATH`.
    pub argv: Vec<OsString>,
    /// The child's complete environment (plus `SHELL`, see the module docs).
    pub env: BTreeMap<OsString, OsString>,
    /// Working directory; must exist (portable-pty would silently fall back to `$HOME`).
    pub cwd: PathBuf,
    pub size: Size,
}

#[derive(Debug, thiserror::Error)]
pub enum PtyError {
    #[error("empty argv")]
    EmptyArgv,
    #[error("working directory {0:?} is not a directory")]
    BadCwd(PathBuf),
    #[error("cannot open a pty: {0}")]
    Open(#[source] Box<dyn std::error::Error + Send + Sync>),
    #[error("cannot start {program:?}: {source}")]
    Spawn {
        program: OsString,
        #[source]
        source: Box<dyn std::error::Error + Send + Sync>,
    },
    #[error("cannot resize the pty: {0}")]
    Resize(#[source] Box<dyn std::error::Error + Send + Sync>),
    #[error("the process already exited and was reaped")]
    AlreadyExited,
    #[error(transparent)]
    Io(#[from] io::Error),
}

/// A process on a pty, owned together with the master side.
pub struct Pty {
    master: Box<dyn MasterPty + Send>,
    io: platform::PtyIo,
    child: platform::PtyChild,
    status: Option<ExitStatus>,
}

impl std::fmt::Debug for Pty {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Pty")
            .field("pid", &self.child.id())
            .field("status", &self.status)
            .finish_non_exhaustive()
    }
}

impl Pty {
    pub fn spawn(spec: &Spawn) -> Result<Self, PtyError> {
        let program = spec.argv.first().ok_or(PtyError::EmptyArgv)?.clone();
        if !spec.cwd.is_dir() {
            return Err(PtyError::BadCwd(spec.cwd.clone()));
        }
        let pair = native_pty_system()
            .openpty(PtySize {
                rows: spec.size.rows,
                cols: spec.size.cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|e| PtyError::Open(e.into()))?;

        // shortcut: portable-pty's CommandBuilder keeps its own unwiped copy of the env until it is
        // dropped below, upgrade if portable-pty ever accepts a zeroizing env.
        let mut cmd = CommandBuilder::from_argv(spec.argv.clone());
        cmd.env_clear();
        for (key, value) in &spec.env {
            cmd.env(key, value);
        }
        cmd.cwd(&spec.cwd);
        let child = pair.slave.spawn_command(cmd).map_err(|e| PtyError::Spawn {
            program: program.clone(),
            source: e.into(),
        })?;
        // Our copy of the slave must go, or reads never see EIO when the child exits.
        drop(pair.slave);
        let child = platform::PtyChild::from_portable(child).map_err(|e| PtyError::Spawn {
            program,
            source: e.into(),
        })?;
        let io = platform::PtyIo::open(&*pair.master).map_err(|e| PtyError::Open(e.into()))?;
        Ok(Self {
            master: pair.master,
            io,
            child,
            status: None,
        })
    }

    /// The master side, for the GUI's notifier. Valid while `self` lives; the bridge dups it.
    #[cfg(unix)] // Windows feeds the bridge from a reader thread instead (feed mode)
    pub fn master_fd(&self) -> std::os::fd::BorrowedFd<'_> {
        self.io.as_fd()
    }

    pub fn pid(&self) -> u32 {
        self.child.id()
    }

    /// Reads output; `Ok(0)` once the child side is closed (the kernel reports that as EIO).
    /// For use without the GUI: once the bridge reads the fd, it owns the output (and may have
    /// made the fd non-blocking).
    pub fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.io.read(buf)
    }

    /// Writes input. Like [`Write::write`] it may write less than `data`, and it returns
    /// `WouldBlock` when the bridge has made the master non-blocking.
    pub fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        self.io.write(data)
    }

    pub fn write_all(&mut self, data: &[u8]) -> io::Result<()> {
        self.io.write_all(data)
    }

    /// A reader for the output and the writer for the input, cloned from the master: the I/O of
    /// feed mode (a thread reads, the GUI is fed). The writer can be taken once; the master and
    /// the child stay with `self`.
    #[allow(clippy::type_complexity)]
    pub fn take_streams(
        &mut self,
    ) -> Result<(Box<dyn Read + Send>, Box<dyn Write + Send>), PtyError> {
        let reader = self
            .master
            .try_clone_reader()
            .map_err(|e| PtyError::Open(e.into()))?;
        let writer = self
            .master
            .take_writer()
            .map_err(|e| PtyError::Open(e.into()))?;
        Ok((reader, writer))
    }

    /// Sets the window size (TIOCSWINSZ); the child gets SIGWINCH.
    pub fn resize(&self, size: Size) -> Result<(), PtyError> {
        self.master
            .resize(PtySize {
                rows: size.rows,
                cols: size.cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|e| PtyError::Resize(e.into()))
    }

    pub fn size(&self) -> Result<Size, PtyError> {
        let size = self
            .master
            .get_size()
            .map_err(|e| PtyError::Resize(e.into()))?;
        Ok(Size {
            cols: size.cols,
            rows: size.rows,
        })
    }

    /// Reaps the child if it exited; the status is kept and returned again on later calls.
    pub fn try_wait(&mut self) -> Result<Option<ExitStatus>, PtyError> {
        if self.status.is_none() {
            self.status = self.child.try_wait()?;
        }
        Ok(self.status)
    }

    /// Blocks until the child exits.
    pub fn wait(&mut self) -> Result<ExitStatus, PtyError> {
        if let Some(status) = self.status {
            return Ok(status);
        }
        let status = self.child.wait()?;
        self.status = Some(status);
        Ok(status)
    }

    /// Sends SIGKILL to our child, unless it has already exited (then nothing is signalled).
    pub fn kill(&mut self) -> Result<(), PtyError> {
        if self.try_wait()?.is_some() {
            return Err(PtyError::AlreadyExited);
        }
        // Not reaped yet, so the PID is still our child (alive or a zombie).
        Ok(self.child.kill()?)
    }

    /// Close semantics: kills the child only if it is still ours and
    /// alive, then reaps it. After an exit this signals nothing. Returns the exit status.
    pub fn close(&mut self) -> Result<ExitStatus, PtyError> {
        match self.kill() {
            Ok(()) | Err(PtyError::AlreadyExited) => {}
            Err(e) => return Err(e),
        }
        self.wait()
    }

    /// Starts a production session; `process_env` is the environment to inherit (injected so
    /// tests do not depend on the real one, callers pass `std::env::vars_os()`).
    pub fn spawn_session(
        spec: &SessionSpec,
        process_env: impl IntoIterator<Item = (OsString, OsString)>,
    ) -> Result<Self, PtyError> {
        let env = session_env(process_env, spec.term.as_deref(), &spec.extra_env);
        let bash = which_in(&env, "bash").unwrap_or_else(|| PathBuf::from("/bin/bash"));
        let mut spawn = Spawn {
            argv: spec.command.argv(&bash),
            env,
            cwd: spec.cwd.clone(),
            size: spec.size,
        };
        let pty = Self::spawn(&spawn);
        crate::vault::wipe_env(&mut spawn.env);
        pty
    }
}

/// What a session runs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ShellCommand {
    /// An executable run as itself: `<exe> -l`.
    LoginShell(String),
    /// A command line for `bash -lc <cmd>`.
    BashLc(String),
    /// A program and its arguments, run as they are (no shell; what Windows uses).
    Argv(Vec<String>),
}

impl ShellCommand {
    /// Exactly `<absolute executable> -l` runs as itself,
    /// any other non-empty text goes through `bash -lc`; empty text is `LoginShell(default_shell)`.
    pub fn from_command_line(cmd: &str, default_shell: &str) -> Self {
        let cmd = cmd.trim();
        if cmd.is_empty() {
            return Self::LoginShell(default_shell.to_owned());
        }
        if let Ok(parts) = shell::split(cmd)
            && let [exe, flag] = parts.as_slice()
            && flag == "-l"
            && Path::new(exe).is_absolute()
            && crate::platform::can_exec(Path::new(exe))
        {
            return Self::LoginShell(exe.clone());
        }
        // No bash on Windows: the line was built by `shell::join`, so split it back into argv.
        #[cfg(windows)]
        if let Ok(parts) = shell::split(cmd) {
            return Self::Argv(parts);
        }
        Self::BashLc(cmd.to_owned())
    }

    fn argv(&self, bash: &Path) -> Vec<OsString> {
        match self {
            Self::LoginShell(exe) => vec![exe.into(), "-l".into()],
            Self::BashLc(cmd) => vec![bash.into(), "-lc".into(), cmd.into()],
            Self::Argv(argv) => argv.iter().map(OsString::from).collect(),
        }
    }
}

/// A session to start with [`Pty::spawn_session`].
#[derive(Clone, Debug)]
pub struct SessionSpec {
    pub command: ShellCommand,
    /// Extra environment (SSHPASS etc.); values are secrets and masked in `Debug`.
    pub extra_env: Vec<(String, Secret)>,
    /// `TERM` override; `None` keeps the process `TERM`, else `xterm-256color`.
    pub term: Option<String>,
    pub size: Size,
    pub cwd: PathBuf,
}

/// `process_env` minus `TERM` and the extra keys, then `TERM`,
/// `TERMIGATOR_PTY`, `COLORTERM`, then the extra env.
fn session_env(
    process_env: impl IntoIterator<Item = (OsString, OsString)>,
    term: Option<&str>,
    extra: &[(String, Secret)],
) -> BTreeMap<OsString, OsString> {
    let mut env = BTreeMap::new();
    let mut proc_term = None;
    for (k, v) in process_env {
        if k == "TERM" {
            proc_term = Some(v);
        } else if !extra.iter().any(|(e, _)| k.as_os_str() == e.as_str()) {
            env.insert(k, v);
        }
    }
    let term = match term {
        Some(t) if !t.is_empty() => OsString::from(t),
        _ => proc_term
            .filter(|t| !t.is_empty())
            .unwrap_or_else(|| "xterm-256color".into()),
    };
    env.insert("TERM".into(), term);
    env.insert("TERMIGATOR_PTY".into(), "1".into());
    env.insert("COLORTERM".into(), "truecolor".into());
    for (k, v) in extra {
        env.insert(k.into(), platform::os_string_from_vec(v.expose().to_vec()));
    }
    env
}

/// `shutil.which` over the `PATH` of `env`.
fn which_in(env: &BTreeMap<OsString, OsString>, name: &str) -> Option<PathBuf> {
    let path = env.get(std::ffi::OsStr::new("PATH"))?;
    std::env::split_paths(path)
        .map(|d| d.join(name))
        .find(|p| crate::platform::is_executable(p))
}

/// What a [`SessionSink`] found in a chunk.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SinkEvent {
    /// Canonical distro id; emitted at most once per sink.
    Distro(&'static str),
    /// Last working directory reported by OSC 7 in the chunk.
    Cwd(String),
}

/// Distro sniff window, in chars.
const SNIFF_LIMIT: usize = 65536;

/// Consumes the tapped output of one session.
pub struct SessionSink {
    epoch: Instant,
    sniff: String,
    distro_done: bool,
    osc7: Osc7Scanner,
    log: Option<(File, LogFormatter, PathBuf)>,
    cast: Option<(CastRecorder<File>, PathBuf)>,
}

impl Default for SessionSink {
    fn default() -> Self {
        Self::new()
    }
}

/// `YYYY-MM-DDTHH:MM:SSZ` for the current UTC time (days-from-civil, Hinnant).
fn utc_stamp() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(0));
    let (days, rem) = (secs.div_euclid(86400), secs.rem_euclid(86400));
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60
    )
}

fn create_parent(path: &Path) -> io::Result<()> {
    match path.parent() {
        Some(p) if !p.as_os_str().is_empty() => fs::create_dir_all(p),
        _ => Ok(()),
    }
}

impl SessionSink {
    pub fn new() -> Self {
        Self {
            epoch: Instant::now(),
            sniff: String::new(),
            distro_done: false,
            osc7: Osc7Scanner::new(),
            log: None,
            cast: None,
        }
    }

    fn mono(&self) -> f64 {
        self.epoch.elapsed().as_secs_f64()
    }

    /// Processes one chunk of raw output.
    pub fn feed(&mut self, data: &[u8]) -> Vec<SinkEvent> {
        let mut events = Vec::new();
        let text: String = data.iter().map(|&b| char::from(b)).collect();
        if !self.distro_done {
            self.sniff.push_str(&text);
            let n = self.sniff.chars().count();
            if n > SNIFF_LIMIT {
                self.sniff = self.sniff.chars().skip(n - SNIFF_LIMIT).collect();
                self.distro_done = true;
            }
            if let Some(id) = detect_distro(&self.sniff) {
                self.distro_done = true;
                events.push(SinkEvent::Distro(id));
            }
            if self.distro_done {
                self.sniff = String::new();
            }
        }
        if let Some(dir) = self.osc7.feed(&text).pop() {
            events.push(SinkEvent::Cwd(dir));
        }
        if let Some((file, fmt, _)) = &mut self.log
            && file.write_all(&fmt.feed(data)).is_err()
        {
            self.log = None; // a failed write stops the log
        }
        let mono = self.mono();
        if let Some((cast, _)) = &mut self.cast
            && cast.output(data, mono).is_err()
        {
            self.cast = None;
        }
        events
    }

    /// Starts (or restarts) appending to `path`: parent directories are created and the header
    /// `### Termigator session log start <UTC> ###` is written. Replaces a running log.
    pub fn start_log(
        &mut self,
        path: &Path,
        strip_ansi: bool,
        timestamps: bool,
        clock: Clock,
    ) -> io::Result<()> {
        self.stop_log();
        create_parent(path)?;
        // 0600: a log holds whatever the terminal printed.
        let mut file =
            platform::private_file(OpenOptions::new().append(true).create(true)).open(path)?;
        // Windows: a new file only inherits its folder's ACL; best effort, as a log on
        // a file system without ACLs must still work.
        let _ = platform::set_private_file(path);
        let header = format!("\n### Termigator session log start {} ###\n", utc_stamp());
        file.write_all(header.as_bytes())?;
        let fmt = LogFormatter::new(strip_ansi, timestamps, clock);
        self.log = Some((file, fmt, path.to_owned()));
        Ok(())
    }

    pub fn stop_log(&mut self) {
        self.log = None;
    }

    /// The running log's path, for carrying it over to the next session.
    pub fn log_path(&self) -> Option<&Path> {
        self.log.as_ref().map(|(_, _, p)| p.as_path())
    }

    /// Starts an asciicast v2 recording at `path` (truncating); `env` is the process env
    /// (only `TERM` and `SHELL` are written).
    pub fn start_recording(
        &mut self,
        path: &Path,
        size: Size,
        title: &str,
        env: &[(String, String)],
    ) -> io::Result<()> {
        self.stop_recording()?;
        create_parent(path)?;
        let file =
            platform::private_file(OpenOptions::new().write(true).create(true).truncate(true))
                .open(path)?;
        let _ = platform::set_private_file(path); // see start_log
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0.0, |d| d.as_secs_f64());
        let rec = CastRecorder::new(
            file,
            i64::from(size.cols),
            i64::from(size.rows),
            title,
            env,
            now,
            self.mono(),
        )?;
        self.cast = Some((rec, path.to_owned()));
        Ok(())
    }

    /// Finishes the recording (flushing a truncated trailing character).
    pub fn stop_recording(&mut self) -> io::Result<()> {
        let mono = self.mono();
        match self.cast.take() {
            Some((cast, _)) => cast.close(mono).map(drop),
            None => Ok(()),
        }
    }

    pub fn recording_path(&self) -> Option<&Path> {
        self.cast.as_ref().map(|(_, p)| p.as_path())
    }

    /// Call when the terminal size changes; recorded only while recording and if it changed.
    pub fn resized(&mut self, size: Size) {
        let mono = self.mono();
        if let Some((cast, _)) = &mut self.cast
            && cast
                .resize(i64::from(size.cols), i64::from(size.rows), mono)
                .is_err()
        {
            self.cast = None;
        }
    }

    /// Closes log and recording (session close).
    pub fn close(&mut self) {
        self.stop_log();
        // Nowhere to report a failing final flush; the events were already written unbuffered.
        let _ = self.stop_recording();
    }
}

/// "Save terminal output": writes `bytes` to `path`, truncating.
pub fn save_output(path: &Path, bytes: &[u8]) -> io::Result<()> {
    use std::io::Write;
    // 0600 like the session logs: the output can hold typed passwords.
    let mut f = platform::private_file(
        fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true),
    )
    .open(path)?;
    let _ = platform::set_private_file(path); // see start_log
    f.write_all(bytes)
}
