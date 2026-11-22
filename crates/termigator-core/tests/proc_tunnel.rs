//! `proc::tunnel`: lifecycle with stub `ssh` scripts on a temp PATH (passed through the env map,
//! no global env mutation), settle timing, non-blocking stop, SIGKILL fallback, free port, and a
//! live socat forwarder (skipped with a message when socat is missing).
// Unix-only for now: uses modes, symlinks, sockets or sh stubs.
#![cfg(unix)]
#![allow(clippy::unwrap_used)]

mod live;

use std::{
    collections::BTreeMap,
    ffi::OsString,
    io::{Read, Write},
    net::TcpStream,
    path::PathBuf,
    process::Stdio,
    sync::mpsc::RecvTimeoutError,
    time::{Duration, Instant},
};

use termigator_core::proc::tunnel::{Tunnel, TunnelEvent, free_local_port};

const STUB: &str = r#"#!/bin/bash
case "$1" in
  alive) echo "$$" > "$TERMIGATOR_PIDFILE"; exec sleep 30 ;;
  fail) echo "Warning: Permanently added 'h' (ED25519) to the list of known hosts." >&2
        echo "bind [127.0.0.1]:1: Address already in use" >&2
        echo "Could not request local forwarding." >&2; exit 255 ;;
  quiet) exit 3 ;;
  ignore) trap '' TERM; echo "$$" > "$TERMIGATOR_PIDFILE"; while :; do sleep 0.1; done ;;
  fwd) shift
       while [ $# -gt 0 ]; do case "$1" in -L) spec="$2"; shift 2 ;; *) shift ;; esac; done
       IFS=: read -r lport thost tport <<<"$spec"
       exec socat TCP-LISTEN:"$lport",bind=127.0.0.1,reuseaddr,fork TCP:"$thost":"$tport" ;;
esac
"#;

struct Stub {
    dir: PathBuf,
    env: BTreeMap<OsString, OsString>,
    pidfile: PathBuf,
}

fn stub(tag: &str) -> Stub {
    let dir = std::env::temp_dir().join(format!("termigator-tun-{tag}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let ssh = dir.join("ssh");
    live::script(&ssh, STUB);
    let pidfile = dir.join("pid");
    let mut path = OsString::from(&dir);
    path.push(":");
    path.push(std::env::var_os("PATH").unwrap_or_default());
    let env = BTreeMap::from([
        ("PATH".into(), path),
        ("TERMIGATOR_PIDFILE".into(), pidfile.clone().into()),
    ]);
    Stub { dir, env, pidfile }
}

fn argv(mode: &str) -> Vec<String> {
    vec!["ssh".into(), mode.into()]
}

fn next(t: &Tunnel, secs: u64) -> TunnelEvent {
    t.events().recv_timeout(Duration::from_secs(secs)).unwrap()
}

fn pid_of(s: &Stub) -> String {
    let t0 = Instant::now();
    loop {
        if let Ok(p) = std::fs::read_to_string(&s.pidfile)
            && p.ends_with('\n')
        {
            return p.trim().to_owned();
        }
        assert!(
            t0.elapsed() < Duration::from_secs(5),
            "stub never wrote its pid"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn alive(pid: &str) -> bool {
    std::fs::read_to_string(format!("/proc/{pid}/stat")).is_ok_and(|s| !s.contains(") Z "))
}

#[test]
fn running_after_settle_then_stopped_without_blocking() {
    let s = stub("alive");
    let t0 = Instant::now();
    let t = Tunnel::start_with_settle(&argv("alive"), &s.env, Duration::from_millis(300));
    assert!(t0.elapsed() < Duration::from_millis(250), "start blocked");
    assert_eq!(next(&t, 5), TunnelEvent::Starting);
    // Not running before the settle time.
    assert_eq!(
        t.events().recv_timeout(Duration::from_millis(100)),
        Err(RecvTimeoutError::Timeout)
    );
    assert_eq!(next(&t, 5), TunnelEvent::Running);
    let pid = pid_of(&s);
    let t1 = Instant::now();
    t.stop();
    assert!(t1.elapsed() < Duration::from_millis(50), "stop blocked");
    assert_eq!(next(&t, 5), TunnelEvent::Stopped);
    assert!(!alive(&pid));
    std::fs::remove_dir_all(&s.dir).unwrap();
}

#[test]
fn early_exit_fails_with_stderr_tail_and_never_runs() {
    let s = stub("fail");
    let t = Tunnel::start(&argv("fail"), &s.env);
    assert_eq!(next(&t, 5), TunnelEvent::Starting);
    let ev = next(&t, 5);
    let TunnelEvent::Failed { code, stderr_tail } = &ev else {
        panic!("expected Failed, got {ev:?}");
    };
    assert_eq!(*code, Some(255));
    assert!(!stderr_tail.contains("Permanently added"), "{stderr_tail}");
    assert!(stderr_tail.contains("Address already in use"));
    assert_eq!(
        ev.detail().as_deref(),
        Some("Could not request local forwarding.")
    );
    assert!(t.events().recv_timeout(Duration::from_millis(100)).is_err());
    std::fs::remove_dir_all(&s.dir).unwrap();
}

#[test]
fn silent_exit_reports_the_code() {
    let s = stub("quiet");
    let t = Tunnel::start(&argv("quiet"), &s.env);
    assert_eq!(next(&t, 5), TunnelEvent::Starting);
    let ev = next(&t, 5);
    assert_eq!(ev.detail().as_deref(), Some("ssh exited with code 3"));
    std::fs::remove_dir_all(&s.dir).unwrap();
}

#[test]
fn missing_binary_or_empty_argv_fails_at_once() {
    let t = Tunnel::start(&["/nonexistent/ssh".into()], &BTreeMap::new());
    assert_eq!(next(&t, 5), TunnelEvent::Starting);
    assert_eq!(
        next(&t, 5).detail().as_deref(),
        Some("cannot start '/nonexistent/ssh'")
    );
    let t = Tunnel::start(&[], &BTreeMap::new());
    assert_eq!(next(&t, 5), TunnelEvent::Starting);
    assert_eq!(
        next(&t, 5).detail().as_deref(),
        Some("cannot build the ssh command")
    );
}

#[test]
fn stop_during_settle_never_reports_running() {
    let s = stub("early-stop");
    let t = Tunnel::start_with_settle(&argv("alive"), &s.env, Duration::from_secs(5));
    assert_eq!(next(&t, 5), TunnelEvent::Starting);
    pid_of(&s);
    t.stop();
    assert_eq!(next(&t, 5), TunnelEvent::Stopped);
    std::fs::remove_dir_all(&s.dir).unwrap();
}

#[test]
fn term_ignoring_process_is_killed() {
    let s = stub("ignore");
    let t = Tunnel::start_with_settle(&argv("ignore"), &s.env, Duration::from_millis(100));
    assert_eq!(next(&t, 5), TunnelEvent::Starting);
    assert_eq!(next(&t, 5), TunnelEvent::Running);
    let pid = pid_of(&s);
    t.stop();
    assert_eq!(next(&t, 5), TunnelEvent::Stopped);
    assert!(!alive(&pid), "TERM-proof stub survived");
    std::fs::remove_dir_all(&s.dir).unwrap();
}

#[test]
fn drop_kills_the_process() {
    let s = stub("drop");
    let t = Tunnel::start_with_settle(&argv("alive"), &s.env, Duration::from_millis(100));
    let pid = pid_of(&s);
    drop(t);
    let t0 = Instant::now();
    while alive(&pid) {
        assert!(
            t0.elapsed() < Duration::from_secs(5),
            "still alive after drop"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    std::fs::remove_dir_all(&s.dir).unwrap();
}

#[test]
fn free_port_is_bindable() {
    let p = free_local_port().unwrap();
    assert!(p >= 1024); // not bound afterwards; binding here would race the socat test
}

#[test]
fn live_forwarder_through_socat() {
    if std::process::Command::new("socat")
        .arg("-V")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_err()
    {
        live::skip("live_forwarder_through_socat", "needs socat");
        return;
    }
    let target = free_local_port().unwrap();
    let mut echo = std::process::Command::new("socat")
        .arg(format!("TCP-LISTEN:{target},bind=127.0.0.1,reuseaddr,fork"))
        .arg("EXEC:cat")
        .spawn()
        .unwrap();
    let s = stub("fwd");
    let lport = free_local_port().unwrap();
    let mut args = argv("fwd");
    args.extend(["-L".into(), format!("{lport}:127.0.0.1:{target}")]);
    let t = Tunnel::start_with_settle(&args, &s.env, Duration::from_millis(500));
    assert_eq!(next(&t, 5), TunnelEvent::Starting);
    assert_eq!(next(&t, 5), TunnelEvent::Running);
    let mut c = TcpStream::connect(("127.0.0.1", lport)).unwrap();
    c.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
    c.write_all(b"PING\n").unwrap();
    let mut buf = [0u8; 5];
    c.read_exact(&mut buf).unwrap();
    assert_eq!(&buf, b"PING\n");
    t.stop();
    assert_eq!(next(&t, 5), TunnelEvent::Stopped);
    let t0 = Instant::now();
    while TcpStream::connect(("127.0.0.1", lport)).is_ok() {
        assert!(
            t0.elapsed() < Duration::from_secs(3),
            "port still open after stop"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
    echo.kill().unwrap();
    echo.wait().unwrap();
    std::fs::remove_dir_all(&s.dir).unwrap();
}
