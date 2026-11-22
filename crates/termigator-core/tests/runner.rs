//! `proc::runner`: bytes, status, timeout/cancel on the whole process group, stdin, env, cap.
// Unix-only for now: uses modes, symlinks, sockets or sh stubs.
#![cfg(unix)]
#![allow(clippy::unwrap_used)]

use std::{
    os::unix::process::ExitStatusExt,
    time::{Duration, Instant},
};

use termigator_core::proc::runner::{Command, Outcome, RunnerError, spawn};

fn sh(script: &str) -> Command {
    let mut c = Command::new(["/bin/sh", "-c", script]);
    c.env.insert("PATH".into(), "/usr/bin:/bin".into());
    c
}

fn run(c: Command) -> Outcome {
    spawn(c).unwrap().wait().unwrap()
}

fn alive(pid: &str) -> bool {
    std::fs::read_to_string(format!("/proc/{pid}/stat")).is_ok_and(|s| !s.contains(") Z "))
}

#[test]
fn bytes_on_both_streams_incl_non_utf8() {
    let o = run(sh(r"printf '\377\376ok'; printf '\200err' >&2"));
    assert!(o.status.success());
    assert_eq!(o.stdout, b"\xff\xfeok");
    assert_eq!(o.stderr, b"\x80err");
    assert!(!o.timed_out && !o.cancelled && !o.stdout_truncated);
}

#[test]
fn nonzero_exit() {
    let o = run(sh("exit 7"));
    assert_eq!(o.status.code(), Some(7));
    assert_eq!(o.code(), 7);
}

#[test]
fn timeout_kills_the_whole_group() {
    let mut c = sh("sleep 60 & echo $!; wait");
    c.timeout = Some(Duration::from_millis(300));
    let t = Instant::now();
    let o = run(c);
    assert!(o.timed_out && !o.cancelled);
    assert!(t.elapsed() < Duration::from_secs(5));
    assert!(o.status.signal().is_some());
    let child = String::from_utf8(o.stdout).unwrap().trim().to_string();
    assert!(!alive(&child), "grandchild {child} survived");
}

#[test]
fn term_ignoring_child_is_killed_after_grace() {
    let mut c = sh("trap '' TERM; while :; do sleep 1; done");
    c.timeout = Some(Duration::from_millis(200));
    let o = run(c);
    assert!(o.timed_out);
    assert_eq!(o.status.signal(), Some(9));
}

#[test]
fn cancel_from_another_thread() {
    let r = spawn(sh("sleep 60")).unwrap();
    let h = r.cancel_handle();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(100));
        h.cancel();
    });
    let o = r.wait().unwrap();
    assert!(o.cancelled && !o.timed_out);
}

#[test]
fn cancel_after_finish_is_harmless() {
    let r = spawn(sh("true")).unwrap();
    let h = r.cancel_handle();
    let o = r.wait().unwrap();
    h.cancel();
    assert!(o.status.success() && !o.cancelled);
}

#[test]
fn stdin_is_delivered() {
    let mut c = sh("cat");
    c.stdin = b"\x00in\xffput".to_vec();
    assert_eq!(run(c).stdout, b"\x00in\xffput");
}

#[test]
fn no_stdin_is_devnull() {
    assert!(run(sh("cat")).stdout.is_empty());
}

#[test]
fn env_is_exactly_what_was_given() {
    let mut c = Command::new(["/usr/bin/env"]);
    c.env.insert("A".into(), "1".into());
    assert_eq!(run(c).stdout, b"A=1\n");
}

#[test]
fn missing_binary_is_an_error() {
    let e = spawn(Command::new(["/nonexistent/termigator-nope"])).unwrap_err();
    assert!(matches!(e, RunnerError::Spawn { .. }));
    let empty = Command::new(Vec::<String>::new());
    assert!(matches!(spawn(empty), Err(RunnerError::EmptyArgv)));
}

#[test]
fn output_cap_truncates_without_blocking_the_child() {
    let mut c = sh("head -c 1000000 /dev/zero; head -c 10 /dev/zero >&2");
    c.max_output = 4096;
    let o = run(c);
    assert!(o.status.success());
    assert_eq!(o.stdout.len(), 4096);
    assert!(o.stdout_truncated);
    assert_eq!(o.stderr.len(), 10);
    assert!(!o.stderr_truncated);
}
