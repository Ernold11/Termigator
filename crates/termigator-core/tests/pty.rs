//! `proc::pty` without a GUI: exact bytes, exit status, resize, kill after reap, environment.
// Unix-only for now: uses modes, symlinks, sockets or sh stubs.
#![cfg(unix)]
#![allow(clippy::unwrap_used)]

use std::{
    collections::BTreeMap,
    ffi::OsString,
    os::unix::process::ExitStatusExt,
    path::PathBuf,
    time::{Duration, Instant},
};

use termigator_core::proc::pty::{Pty, PtyError, Size, Spawn};

fn spec(argv: &[&str]) -> Spawn {
    Spawn {
        argv: argv.iter().map(OsString::from).collect(),
        env: BTreeMap::from([("PATH".into(), "/usr/bin:/bin".into())]),
        cwd: PathBuf::from("/"),
        size: Size { cols: 80, rows: 24 },
    }
}

/// Everything the child writes until it closes the pty.
fn read_to_end(pty: &mut Pty) -> Vec<u8> {
    let mut out = Vec::new();
    let mut buf = [0; 4096];
    loop {
        match pty.read(&mut buf).unwrap() {
            0 => return out,
            n => out.extend_from_slice(&buf[..n]),
        }
    }
}

fn read_until(pty: &mut Pty, needle: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut buf = [0; 4096];
    while !out.windows(needle.len()).any(|w| w == needle) {
        let n = pty.read(&mut buf).unwrap();
        assert!(n > 0, "EOF before {needle:?}, got {out:?}");
        out.extend_from_slice(&buf[..n]);
    }
    out
}

#[test]
fn raw_bytes_and_exit_status() {
    // The pty's ONLCR turns \n into \r\n; \xe9 alone is invalid UTF-8 and must survive as is.
    let mut pty = Pty::spawn(&spec(&["sh", "-c", r"printf 'a\351b\n'; exit 3"])).unwrap();
    assert_eq!(read_to_end(&mut pty), b"a\xe9b\r\n");
    let status = pty.wait().unwrap();
    assert_eq!(status.code(), Some(3));
    assert_eq!(pty.try_wait().unwrap(), Some(status));
}

#[test]
fn kill_after_reap_sends_nothing() {
    let mut pty = Pty::spawn(&spec(&["sh", "-c", "exit 0"])).unwrap();
    pty.wait().unwrap();
    assert!(matches!(pty.kill(), Err(PtyError::AlreadyExited)));
}

#[test]
fn kill_running_child() {
    let mut pty = Pty::spawn(&spec(&["sleep", "30"])).unwrap();
    assert_eq!(pty.try_wait().unwrap(), None);
    pty.kill().unwrap();
    assert_eq!(pty.wait().unwrap().signal(), Some(9));
    assert!(matches!(pty.kill(), Err(PtyError::AlreadyExited)));
}

#[test]
fn try_wait_sees_exit_quickly() {
    let mut pty = Pty::spawn(&spec(&["sh", "-c", "exit 7"])).unwrap();
    read_to_end(&mut pty);
    let start = Instant::now();
    let status = loop {
        if let Some(status) = pty.try_wait().unwrap() {
            break status;
        }
        assert!(start.elapsed() < Duration::from_secs(1), "no exit status");
        std::thread::sleep(Duration::from_millis(5));
    };
    assert_eq!(status.code(), Some(7));
}

#[test]
fn resize_is_visible_to_the_child() {
    let mut pty = Pty::spawn(&spec(&["sh", "-c", "stty size; read x; stty size"])).unwrap();
    read_until(&mut pty, b"24 80");
    pty.resize(Size {
        cols: 132,
        rows: 43,
    })
    .unwrap();
    assert_eq!(
        pty.size().unwrap(),
        Size {
            cols: 132,
            rows: 43
        }
    );
    pty.write_all(b"\n").unwrap();
    read_until(&mut pty, b"43 132");
    assert!(pty.wait().unwrap().success());
}

#[test]
fn environment_is_exactly_the_given_map() {
    let mut s = spec(&["env"]);
    s.env.insert("TERMIGATOR_TEST".into(), "v1".into());
    s.env.insert("SHELL".into(), "/bin/sh".into());
    let mut pty = Pty::spawn(&s).unwrap();
    let out = String::from_utf8(read_to_end(&mut pty)).unwrap();
    let mut lines: Vec<&str> = out.lines().map(|l| l.trim_end_matches('\r')).collect();
    lines.sort_unstable();
    assert_eq!(
        lines,
        ["PATH=/usr/bin:/bin", "SHELL=/bin/sh", "TERMIGATOR_TEST=v1"]
    );
}

#[test]
fn working_directory_and_errors() {
    let mut s = spec(&["pwd"]);
    s.cwd = PathBuf::from("/usr");
    let mut pty = Pty::spawn(&s).unwrap();
    assert_eq!(read_to_end(&mut pty), b"/usr\r\n");

    s.cwd = PathBuf::from("/nonexistent-termigator");
    assert!(matches!(Pty::spawn(&s), Err(PtyError::BadCwd(_))));
    assert!(matches!(Pty::spawn(&spec(&[])), Err(PtyError::EmptyArgv)));
    assert!(matches!(
        Pty::spawn(&spec(&["termigator-no-such-program"])),
        Err(PtyError::Spawn { .. })
    ));
}

/// Fields 2.. of `/proc/<pid>/stat` once the child has exec'd `sleep`.
fn exec_stat(pid: u32) -> Vec<String> {
    // Up to 10 s: a busy CI runner can take a while to exec.
    for _ in 0..1000 {
        let cmdline = std::fs::read(format!("/proc/{pid}/cmdline")).unwrap();
        if cmdline.starts_with(b"sleep\0") {
            let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).unwrap();
            let (_, rest) = stat.rsplit_once(") ").unwrap();
            return rest.split(' ').map(str::to_owned).collect();
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    panic!("child never exec'd");
}

/// `SigIgn:` or `SigBlk:` of `/proc/<pid>/status` as a mask.
fn sig_mask(pid: u32, field: &str) -> u64 {
    let status = std::fs::read_to_string(format!("/proc/{pid}/status")).unwrap();
    let line = status.lines().find_map(|l| l.strip_prefix(field)).unwrap();
    u64::from_str_radix(line.trim(), 16).unwrap()
}

/// The child starts as a session leader on the pty and inherits neither our descriptors (even
/// without CLOEXEC) nor ignored or blocked signals: a shell started from a launcher that ignores
/// SIGHUP must still die when its tab closes. The setup is process-wide, so it runs in a child.
#[test]
fn child_starts_clean() {
    use nix::sys::signal::{SigHandler, SigSet, SigmaskHow, Signal, sigprocmask};
    const CHILD: &str = "TERMIGATOR_TEST_PTY_CLEAN_CHILD";
    // Not SIGCHLD: a parent that ignores it cannot wait for the child at all.
    const RESET: [Signal; 5] = [
        Signal::SIGHUP,
        Signal::SIGINT,
        Signal::SIGQUIT,
        Signal::SIGTERM,
        Signal::SIGALRM,
    ];
    if std::env::var_os(CHILD).is_none() {
        let out = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "child_starts_clean", "--nocapture"])
            .env(CHILD, "1")
            .output()
            .unwrap();
        let log = String::from_utf8_lossy(&out.stdout);
        // The child's own assertion message is on its stderr.
        let err = String::from_utf8_lossy(&out.stderr);
        assert!(
            out.status.success() && log.contains("child clean"),
            "{log}\n{err}"
        );
        return;
    }
    for sig in RESET {
        // SAFETY: SIG_IGN installs no handler code; this child process does nothing else.
        unsafe { nix::sys::signal::signal(sig, SigHandler::SigIgn) }.unwrap();
    }
    let mut blocked = SigSet::empty();
    blocked.add(Signal::SIGUSR1);
    sigprocmask(SigmaskHow::SIG_BLOCK, Some(&blocked), None).unwrap();
    let inheritable = std::fs::File::open("/dev/null").unwrap();
    rustix::io::fcntl_setfd(&inheritable, rustix::io::FdFlags::empty()).unwrap();

    let mut pty = Pty::spawn(&spec(&["sleep", "30"])).unwrap();
    let pid = pty.pid();
    let stat = exec_stat(pid);
    // state ppid pgrp session tty_nr
    assert_eq!(stat[2], pid.to_string(), "process group");
    assert_eq!(stat[3], pid.to_string(), "session");
    assert_ne!(stat[4], "0", "controlling terminal");
    let mut fds: Vec<String> = std::fs::read_dir(format!("/proc/{pid}/fd"))
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    fds.sort_unstable();
    assert_eq!(fds, ["0", "1", "2"]);
    let reset = RESET.iter().fold(0u64, |m, s| m | 1 << (*s as i32 - 1));
    assert_eq!(sig_mask(pid, "SigIgn:") & reset, 0, "ignored signals");
    assert_eq!(sig_mask(pid, "SigBlk:"), 0, "blocked signals");
    pty.close().unwrap();
    drop(inheritable);
    println!("child clean");
}
