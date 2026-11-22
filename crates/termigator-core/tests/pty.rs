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
