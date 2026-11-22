//! Shared helpers for the tests with real binaries (`tests/live_*.rs`).
//!
//! A test that needs a missing program is skipped with a `skipped: <test>: ...` line on stderr
//! (`cargo test -- --nocapture` shows it). With `TERMIGATOR_REQUIRE_LIVE` set (CI, where the
//! container has every package) a missing program fails the test instead, so a broken image
//! cannot turn the live suite into silent skips.
#![allow(dead_code)] // each test binary uses its own subset

use std::{
    ffi::OsString,
    fs::{self, File},
    io::Read,
    net::TcpStream,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex, MutexGuard, PoisonError,
        atomic::{AtomicU32, Ordering},
    },
    thread::JoinHandle,
    time::{Duration, Instant},
};

use termigator_core::{
    config::doc::Value,
    proc::pty::{Pty, SessionSpec, ShellCommand, Size},
    vault::Secret,
};

/// A config mapping from `(key, value)` pairs.
pub fn map(pairs: &[(&str, Value)]) -> Value {
    Value::Map(
        pairs
            .iter()
            .map(|(k, v)| (Value::Str((*k).to_owned()), v.clone()))
            .collect(),
    )
}

/// A config string.
pub fn s(t: &str) -> Value {
    Value::Str(t.to_owned())
}

/// `TERMIGATOR_REQUIRE_LIVE` is set: missing programs are failures, not skips.
pub fn required() -> bool {
    std::env::var_os("TERMIGATOR_REQUIRE_LIVE").is_some_and(|v| !v.is_empty())
}

/// Reports a skipped test (or panics under `TERMIGATOR_REQUIRE_LIVE`).
pub fn skip(test: &str, why: &str) {
    assert!(
        !required(),
        "{test}: {why} (TERMIGATOR_REQUIRE_LIVE is set, so this is a failure)"
    );
    eprintln!("skipped: {test}: {why}");
}

/// An executable `bin` on `$PATH` (`shutil.which`).
pub fn which(bin: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|d| d.join(bin))
        .find(|p| fs::metadata(p).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0))
}

/// `which` as the builders' `Which` (`name -> installed?`).
pub fn installed(name: &str) -> bool {
    which(name).is_some()
}

/// True when every program in `bins` is on `$PATH`; else the test is skipped (see [`skip`]).
pub fn have(test: &str, bins: &[&str]) -> bool {
    let missing: Vec<&str> = bins
        .iter()
        .copied()
        .filter(|b| which(b).is_none())
        .collect();
    if missing.is_empty() {
        return true;
    }
    skip(test, &format!("needs {} (not on PATH)", missing.join(", ")));
    false
}

/// The OpenSSH `sftp-server` (`TERMIGATOR_TEST_SFTP_SERVER` first).
pub fn sftp_server() -> Option<PathBuf> {
    std::env::var_os("TERMIGATOR_TEST_SFTP_SERVER")
        .map(PathBuf::from)
        .into_iter()
        .chain(
            [
                "/usr/lib/ssh/sftp-server",
                "/usr/lib/openssh/sftp-server",
                "/usr/libexec/openssh/sftp-server",
                "/usr/libexec/sftp-server",
            ]
            .map(PathBuf::from),
        )
        .find(|p| p.is_file())
}

/// Writing an executable while a sibling test forks can give ETXTBSY (the child holds the
/// write descriptor until it execs): the live tests of one binary run one at a time.
static SERIAL: Mutex<()> = Mutex::new(());

pub fn serial() -> MutexGuard<'static, ()> {
    SERIAL.lock().unwrap_or_else(PoisonError::into_inner)
}

/// A fresh scratch directory (short path: Unix sockets live in it).
pub fn scratch(tag: &str) -> PathBuf {
    static N: AtomicU32 = AtomicU32::new(0);
    let d = std::env::temp_dir().join(format!(
        "termigator-v3-{}-{}-{tag}",
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = fs::remove_dir_all(&d);
    fs::create_dir_all(&d).expect("scratch dir");
    d
}

/// An executable script at `path`.
///
/// This process never holds a write descriptor to the executable: `fs::write` + `chmod` on the
/// script itself leaves a window in which a sibling test thread's `fork()` child inherits that
/// descriptor, and an `execve` of the script in the meantime fails with ETXTBSY ("Text file
/// busy"; seen as `cannot start 'ssh'` in `proc_tunnel`). So the body goes to a plain
/// (non-executable) file and a child process (`install`) creates the executable as a new inode
/// that no fork of ours can have open for writing.
pub fn script(path: &Path, body: &str) {
    static N: AtomicU32 = AtomicU32::new(0);
    let tmp = path.with_extension(format!(
        "tmp{}-{}",
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    ));
    fs::write(&tmp, body).expect("write script body");
    let out = std::process::Command::new("install")
        .args(["-m", "755"])
        .arg(&tmp)
        .arg(path)
        .output()
        .expect("run install");
    let _ = fs::remove_file(&tmp);
    assert!(
        out.status.success(),
        "install {path:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// `dir` in front of the current `$PATH`.
pub fn path_with(dir: &Path) -> OsString {
    let mut p = OsString::from(dir);
    p.push(":");
    p.push(std::env::var_os("PATH").unwrap_or_default());
    p
}

/// Polls `f` every 50 ms until it holds or `timeout` passes.
pub fn wait_until(timeout: Duration, mut f: impl FnMut() -> bool) -> bool {
    let t0 = Instant::now();
    loop {
        if f() {
            return true;
        }
        if t0.elapsed() > timeout {
            return false;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// Something listens on `127.0.0.1:port`.
pub fn port_open(port: u16) -> bool {
    TcpStream::connect_timeout(&([127, 0, 0, 1], port).into(), Duration::from_millis(300)).is_ok()
}

/// The process `pid` exists and is not a zombie.
pub fn alive(pid: &str) -> bool {
    fs::read_to_string(format!("/proc/{pid}/stat")).is_ok_and(|s| !s.contains(") Z "))
}

pub const SIZE: Size = Size {
    cols: 100,
    rows: 30,
};

/// A production terminal session ([`Pty::spawn_session`]) whose output a thread collects —
/// the stand-in for the GUI's terminal tab.
pub struct Term {
    pub pty: Pty,
    chunks: Arc<Mutex<Vec<Vec<u8>>>>,
    taken: usize,
    reader: Option<JoinHandle<()>>,
}

impl Term {
    /// Starts `command` with `process_env` as the inherited environment and `extra` as the
    /// session's secret environment (SSHPASS etc.).
    pub fn start(
        command: ShellCommand,
        process_env: Vec<(OsString, OsString)>,
        extra: &[(&str, &str)],
        term: Option<&str>,
    ) -> Term {
        let spec = SessionSpec {
            command,
            extra_env: extra
                .iter()
                .map(|(k, v)| ((*k).to_owned(), Secret::new(v.as_bytes().to_vec())))
                .collect(),
            term: term.map(str::to_owned),
            size: SIZE,
            cwd: std::env::temp_dir(),
        };
        let pty = Pty::spawn_session(&spec, process_env).expect("spawn session");
        let fd = pty.master_fd().try_clone_to_owned().expect("dup master");
        let mut file = File::from(fd);
        let chunks = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&chunks);
        let reader = std::thread::spawn(move || {
            let mut buf = [0u8; 4096];
            // EIO (every slave side closed) or EOF ends the session's output.
            while let Ok(n) = file.read(&mut buf) {
                if n == 0 {
                    break;
                }
                sink.lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .push(buf[..n].to_vec());
            }
        });
        Term {
            pty,
            chunks,
            taken: 0,
            reader: Some(reader),
        }
    }

    /// A `bash -lc <cmd>` session with the current environment plus `env`.
    pub fn bash(cmd: &str, env: &[(&str, &OsString)], extra: &[(&str, &str)]) -> Term {
        let mut process_env: Vec<(OsString, OsString)> = std::env::vars_os()
            .filter(|(k, _)| !env.iter().any(|(e, _)| k.as_os_str() == *e))
            .collect();
        process_env.extend(env.iter().map(|(k, v)| (OsString::from(*k), (*v).clone())));
        Term::start(
            ShellCommand::BashLc(cmd.to_owned()),
            process_env,
            extra,
            None,
        )
    }

    /// Everything read so far, lossily decoded.
    pub fn text(&self) -> String {
        let chunks = self.chunks.lock().unwrap_or_else(PoisonError::into_inner);
        String::from_utf8_lossy(&chunks.concat()).into_owned()
    }

    /// The chunks read since the previous call, in order.
    pub fn new_chunks(&mut self) -> Vec<Vec<u8>> {
        let chunks = self.chunks.lock().unwrap_or_else(PoisonError::into_inner);
        let fresh = chunks[self.taken..].to_vec();
        self.taken = chunks.len();
        fresh
    }

    /// Waits until the output contains `needle`; the panic message carries the output.
    pub fn expect(&self, needle: &str, timeout: Duration) {
        assert!(
            wait_until(timeout, || self.text().contains(needle)),
            "{needle:?} never appeared; output so far:\n{}",
            self.text()
        );
    }

    /// Types `text` (bytes as given; `\n` is Enter).
    pub fn send(&mut self, text: &str) {
        self.pty.write_all(text.as_bytes()).expect("write to pty");
    }

    /// Closes the tab: [`Pty::close`] (kill + reap), then the reader drains.
    pub fn close(&mut self) {
        let _ = self.pty.close();
        if let Some(r) = self.reader.take() {
            // Background children may still hold the slave; do not wait for them forever.
            let t0 = Instant::now();
            while !r.is_finished() && t0.elapsed() < Duration::from_secs(5) {
                std::thread::sleep(Duration::from_millis(20));
            }
            if r.is_finished() {
                let _ = r.join();
            }
        }
    }
}

impl Drop for Term {
    fn drop(&mut self) {
        let _ = self.pty.close();
    }
}

/// A child process killed (and reaped) when dropped — servers the tests start.
pub struct Daemon(pub std::process::Child);

impl Daemon {
    pub fn spawn(cmd: &mut std::process::Command) -> Daemon {
        Daemon(cmd.spawn().expect("spawn helper process"))
    }

    pub fn pid(&self) -> u32 {
        self.0.id()
    }
}

impl Drop for Daemon {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
