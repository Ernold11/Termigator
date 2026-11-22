//! Launcher shared by the offscreen checks: the real binary in smoke-test mode
//! (`TERMIGATOR_SMOKE_TEST`) with a private config dir and control socket. `tests/smoke.rs` runs the
//! shell's `SmokeCheck.qml`; an area's test passes `--smoke-check=<area>/<File>.qml` so the window
//! runs that file from `qml/` instead (same `required property var shell`), and uses
//! [`run_check`] when the check quits by itself.

#![allow(dead_code)] // each test binary uses its own part

use std::fs::{self, File};
use std::path::PathBuf;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::thread;
use std::time::{Duration, Instant};

/// An executable script at `path`, created without this process ever holding a write descriptor
/// to it: `fs::write` + `chmod` on the script leaves a window in which a sibling thread's
/// `fork()` child inherits the descriptor and an `execve` of the script fails with ETXTBSY. The
/// body goes to a plain file and a child process (`install`) makes the executable inode.
pub fn write_executable(path: &std::path::Path, body: &str) {
    use std::sync::atomic::{AtomicU32, Ordering};
    static N: AtomicU32 = AtomicU32::new(0);
    let tmp = path.with_extension(format!(
        "tmp{}-{}",
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    ));
    fs::write(&tmp, body).expect("write script body");
    let out = Command::new("install")
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

pub struct Launched {
    pub child: Child,
    pub config: PathBuf,
    pub socket: PathBuf,
    pub shot: PathBuf,
    pub log_path: PathBuf,
}

impl Launched {
    pub fn log(&self) -> String {
        fs::read_to_string(&self.log_path).unwrap_or_default()
    }

    /// Waits up to `secs` for the process to quit; `tick` runs every 100 ms with the log so far.
    pub fn wait(&mut self, secs: u64, mut tick: impl FnMut(&str)) -> ExitStatus {
        let deadline = Instant::now() + Duration::from_secs(secs);
        loop {
            if let Some(status) = self.child.try_wait().expect("wait for termigator") {
                return status;
            }
            if Instant::now() > deadline {
                let _ = self.child.kill();
                panic!(
                    "termigator did not quit within {secs} s in smoke-test mode, stderr:\n{}",
                    self.log()
                );
            }
            tick(&self.log());
            thread::sleep(Duration::from_millis(100));
        }
    }
}

/// Starts termigator offscreen with `config.yaml` = `config_yaml` in a fresh dir named `name`.
pub fn launch(name: &str, config_yaml: &str, args: &[&str]) -> Launched {
    launch_env(name, config_yaml, args, &[])
}

/// [`launch`] with extra environment variables for the process (e.g. a stub `ssh` first in `PATH`).
pub fn launch_env(name: &str, config_yaml: &str, args: &[&str], env: &[(&str, &str)]) -> Launched {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("termigator-{name}"));
    let _ = fs::remove_dir_all(&dir);
    let config = dir.join("config");
    fs::create_dir_all(&config).expect("config dir");
    fs::write(config.join("config.yaml"), config_yaml).expect("config.yaml");
    // A Unix socket path must fit in sun_path (108 bytes); the target dir may be deeper.
    let socket =
        std::env::temp_dir().join(format!("termigator-{name}-{}.sock", std::process::id()));
    let shot = dir.join("shot.png");
    let log_path = dir.join("stderr.log");
    let child = Command::new(env!("CARGO_BIN_EXE_termigator"))
        .args(args)
        .env("QT_QPA_PLATFORM", "offscreen")
        .env("QT_QPA_PLATFORMTHEME", "") // the smokes assume the plain Qt theme, not the developer's KDE one
        .env("QT_QUICK_BACKEND", "software")
        // Qt logs to journald when stderr is not a terminal; the check reads stderr.
        .env("QT_FORCE_STDERR_LOGGING", "1")
        .env("TERMIGATOR_SMOKE_TEST", &shot)
        .env("TERMIGATOR_CONFIG_DIR", &config)
        .env("TERMIGATOR_CONTROL_SOCKET", &socket)
        .envs(env.iter().copied())
        .stdout(Stdio::null())
        .stderr(File::create(&log_path).expect("create smoke log"))
        .spawn()
        .expect("spawn termigator");
    Launched {
        child,
        config,
        socket,
        shot,
        log_path,
    }
}

/// Runs `qml/<check>` (e.g. `"sidebar/SidebarSmoke.qml"`), which must end with `Qt.exit(0)`
/// (`Qt.exit(1)` after a `console.error` on failure). Returns the stderr log; panics on failure.
pub fn run_check(name: &str, config_yaml: &str, check: &str) -> String {
    let mut run = launch(name, config_yaml, &[&format!("--smoke-check={check}")]);
    let status = run.wait(60, |_| {});
    let log = run.log();
    assert!(status.success(), "exit {status}, stderr:\n{log}");
    log
}
