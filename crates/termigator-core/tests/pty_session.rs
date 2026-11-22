//! Production PTY sessions without a GUI: env, command forms, secrets, sink, close.
// Unix-only for now: uses modes, symlinks, sockets or sh stubs.
#![cfg(unix)]
#![allow(clippy::unwrap_used)]

use std::{
    ffi::OsString,
    fs,
    path::{Path, PathBuf},
    process::id,
};

use termigator_core::{
    proc::pty::{Pty, SessionSink, SessionSpec, ShellCommand, SinkEvent, Size, save_output},
    text::session_log::{Clock, LocalTime},
    vault::Secret,
};

const SIZE: Size = Size { cols: 80, rows: 24 };

fn proc_env() -> Vec<(OsString, OsString)> {
    vec![
        ("PATH".into(), "/usr/bin:/bin".into()),
        ("TERM".into(), "dumb".into()),
        ("SSHPASS".into(), "from-process".into()),
        ("KEEP".into(), "kept".into()),
    ]
}

fn spec(command: ShellCommand, term: Option<&str>) -> SessionSpec {
    SessionSpec {
        command,
        extra_env: vec![("SSHPASS".into(), Secret::new(b"hunter2-secret".to_vec()))],
        term: term.map(str::to_owned),
        size: SIZE,
        cwd: PathBuf::from("/"),
    }
}

fn read_to_end(pty: &mut Pty) -> String {
    let mut out = Vec::new();
    let mut buf = [0; 4096];
    loop {
        match pty.read(&mut buf).unwrap() {
            0 => return String::from_utf8_lossy(&out).into_owned(),
            n => out.extend_from_slice(&buf[..n]),
        }
    }
}

/// `/proc/<pid>/<file>` once the child has exec'd (before that it still shows this process).
fn proc_file(pid: u32, file: &str) -> Vec<u8> {
    let me = fs::read(format!("/proc/{}/{file}", id())).unwrap();
    for _ in 0..200 {
        let v = fs::read(format!("/proc/{pid}/{file}")).unwrap();
        if !v.is_empty() && v != me {
            return v;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    panic!("child never exec'd");
}

fn tmp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("termigator-pty-session-{}-{name}", id()));
    let _ = fs::remove_dir_all(&d);
    d
}

fn clock() -> Clock {
    Box::new(|| LocalTime {
        year: 2026,
        month: 1,
        day: 2,
        hour: 3,
        minute: 4,
        second: 5,
    })
}

#[test]
fn env_is_exact() {
    let cmd = ShellCommand::BashLc(
        "printf '%s|%s|%s|%s|%s' \"$TERM\" \"$TERMIGATOR_PTY\" \"$COLORTERM\" \"$SSHPASS\" \"$KEEP\""
            .into(),
    );
    let mut pty = Pty::spawn_session(&spec(cmd.clone(), None), proc_env()).unwrap();
    assert_eq!(
        read_to_end(&mut pty),
        "dumb|1|truecolor|hunter2-secret|kept"
    );
    pty.close().unwrap();
    // an explicit TERM overrides the process one
    let mut pty = Pty::spawn_session(&spec(cmd.clone(), Some("xterm-kitty")), proc_env()).unwrap();
    assert!(read_to_end(&mut pty).starts_with("xterm-kitty|1|truecolor|"));
    pty.close().unwrap();
    // no TERM anywhere: the default
    let env: Vec<_> = proc_env()
        .into_iter()
        .filter(|(k, _)| k != "TERM")
        .collect();
    let mut pty = Pty::spawn_session(&spec(cmd, None), env).unwrap();
    assert!(read_to_end(&mut pty).starts_with("xterm-256color|"));
    pty.close().unwrap();
}

#[test]
fn secret_in_child_env_but_not_argv_or_debug() {
    let s = spec(ShellCommand::BashLc("sleep 30".into()), None);
    assert!(!format!("{s:?}").contains("hunter2"));
    let mut pty = Pty::spawn_session(&s, proc_env()).unwrap();
    let pid = pty.pid();
    // bash may not have exec'd sleep yet, but the environment is inherited either way
    let environ = proc_file(pid, "environ");
    assert!(
        String::from_utf8_lossy(&environ).contains("SSHPASS=hunter2-secret"),
        "{environ:?}"
    );
    let cmdline = proc_file(pid, "cmdline");
    assert!(!String::from_utf8_lossy(&cmdline).contains("hunter2"));
    assert!(!format!("{pty:?}").contains("hunter2"));
    pty.close().unwrap();
}

#[test]
fn login_shell_vs_bash_lc() {
    let mut a = Pty::spawn_session(
        &spec(ShellCommand::LoginShell("/bin/bash".into()), None),
        proc_env(),
    )
    .unwrap();
    let cmdline = proc_file(a.pid(), "cmdline");
    assert_eq!(cmdline, b"/bin/bash\0-l\0");
    a.close().unwrap();
    let mut b = Pty::spawn_session(
        &spec(ShellCommand::BashLc("sleep 30; true".into()), None),
        proc_env(),
    )
    .unwrap();
    let cmdline = proc_file(b.pid(), "cmdline");
    let text = String::from_utf8_lossy(&cmdline).into_owned();
    assert!(text.ends_with("bash\0-lc\0sleep 30; true\0"), "{text:?}");
    b.close().unwrap();
}

#[test]
fn command_line_detection() {
    let d = "/bin/zsh";
    assert_eq!(
        ShellCommand::from_command_line("", d),
        ShellCommand::LoginShell(d.into())
    );
    assert_eq!(
        ShellCommand::from_command_line("/bin/bash -l", d),
        ShellCommand::LoginShell("/bin/bash".into())
    );
    assert_eq!(
        ShellCommand::from_command_line("ssh host", d),
        ShellCommand::BashLc("ssh host".into())
    );
    assert_eq!(
        ShellCommand::from_command_line("/nonexistent/sh -l", d),
        ShellCommand::BashLc("/nonexistent/sh -l".into())
    );
}

#[test]
fn close_kills_live_and_is_noop_after_exit() {
    let mut live = Pty::spawn_session(
        &spec(ShellCommand::BashLc("sleep 30".into()), None),
        proc_env(),
    )
    .unwrap();
    let status = live.close().unwrap();
    assert!(!status.success());
    // again: already reaped, nothing is signalled, same status
    assert_eq!(live.close().unwrap(), status);

    let mut done = Pty::spawn_session(
        &spec(ShellCommand::BashLc("exit 3".into()), None),
        proc_env(),
    )
    .unwrap();
    read_to_end(&mut done);
    assert_eq!(done.wait().unwrap().code(), Some(3));
    assert_eq!(done.close().unwrap().code(), Some(3));
}

fn events(sink: &mut SessionSink, chunks: &[&[u8]]) -> Vec<SinkEvent> {
    chunks.iter().flat_map(|c| sink.feed(c)).collect()
}

#[test]
fn distro_detected_once() {
    let mut sink = SessionSink::new();
    let ev = events(
        &mut sink,
        &[
            b"$ cat /etc/os-release\r\nNAME=\"Debian GNU/Linux\"\r\nI",
            b"D=debian\r\n",
            b"ID=debian\r\n",
        ],
    );
    assert_eq!(ev, vec![SinkEvent::Distro("debian")]);
}

#[test]
fn osc7_split_across_chunks() {
    let mut sink = SessionSink::new();
    let ev = events(
        &mut sink,
        &[
            b"x\x1b]7;file://h",
            b"ost/home/a%20b\x07$ ",
            b"\x1b]7;file://h/tmp\x1b",
            b"\\",
        ],
    );
    assert_eq!(
        ev,
        vec![
            SinkEvent::Cwd("/home/a b".into()),
            SinkEvent::Cwd("/tmp".into())
        ]
    );
}

#[test]
fn log_header_strip_and_carry() {
    let dir = tmp("log");
    let path = dir.join("sub/s.log");
    let mut a = SessionSink::new();
    a.start_log(&path, true, false, clock()).unwrap();
    // A log / recording is private to the user.
    assert_eq!(
        std::os::unix::fs::PermissionsExt::mode(&std::fs::metadata(&path).unwrap().permissions())
            & 0o777,
        0o600
    );
    a.feed(b"\x1b[31mred\x1b[0m\r\nline2\r\n");
    assert_eq!(a.log_path(), Some(path.as_path()));
    let carried = a.log_path().map(Path::to_path_buf).unwrap();
    a.close();
    assert_eq!(a.log_path(), None);

    let mut b = SessionSink::new();
    b.start_log(&carried, true, true, clock()).unwrap();
    b.feed(b"again\n");
    b.close();
    let text = fs::read_to_string(&path).unwrap();
    let lines: Vec<&str> = text.split('\n').collect();
    assert_eq!(lines[0], "");
    assert!(lines[1].starts_with("### Termigator session log start 20"));
    assert!(lines[1].ends_with("Z ###"));
    assert_eq!(&lines[2..4], ["red", "line2"]);
    assert_eq!(lines[4], "");
    assert!(lines[5].starts_with("### Termigator session log start"));
    assert_eq!(lines[6], "[2026-01-02 03:04:05] again");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn asciicast_header_events_and_resize() {
    let dir = tmp("cast");
    let path = dir.join("r.cast");
    let mut sink = SessionSink::new();
    let env = vec![
        ("TERM".to_owned(), "xterm".to_owned()),
        ("SHELL".to_owned(), "/bin/zsh".to_owned()),
        ("SSHPASS".to_owned(), "nope".to_owned()),
    ];
    sink.start_recording(&path, SIZE, "T", &env).unwrap();
    // A log / recording is private to the user.
    assert_eq!(
        std::os::unix::fs::PermissionsExt::mode(&std::fs::metadata(&path).unwrap().permissions())
            & 0o777,
        0o600
    );
    assert_eq!(sink.recording_path(), Some(path.as_path()));
    sink.feed("héllo".as_bytes());
    sink.resized(Size {
        cols: 100,
        rows: 30,
    });
    sink.resized(Size {
        cols: 100,
        rows: 30,
    }); // unchanged: nothing
    sink.close();
    let text = fs::read_to_string(&path).unwrap();
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), 3, "{text}");
    assert!(lines[0].starts_with(r#"{"version": 2, "width": 80, "height": 24, "timestamp": "#));
    assert!(lines[0].ends_with(r#""title": "T", "env": {"TERM": "xterm", "SHELL": "/bin/zsh"}}"#));
    assert!(!text.contains("nope"));
    assert!(lines[1].ends_with(r#", "o", "héllo"]"#));
    assert!(lines[2].ends_with(r#", "r", "100x30"]"#));
    assert_eq!(sink.recording_path(), None);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn save_output_truncates() {
    let dir = tmp("save");
    fs::create_dir_all(&dir).unwrap();
    let p = dir.join("o.txt");
    save_output(&p, b"long old content").unwrap();
    save_output(&p, b"new").unwrap();
    assert_eq!(fs::read(&p).unwrap(), b"new");
    let _ = fs::remove_dir_all(&dir);
}
