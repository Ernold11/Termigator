//! Single-instance lock and hand-over (no Qt).
// Unix-only for now: uses modes, symlinks, sockets or sh stubs.
#![cfg(unix)]

use std::ffi::OsString;
use std::io::{Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

use termigator_core::single_instance::*;

/// A forked child briefly holds every open fd; spawning must not overlap the dead-socket test.
static SPAWN: Mutex<()> = Mutex::new(());

/// Short temp dir (sun_path is 108 bytes), removed on drop.
struct Dir(PathBuf);

impl Dir {
    fn new() -> Dir {
        use std::sync::atomic::{AtomicU32, Ordering};
        static N: AtomicU32 = AtomicU32::new(0);
        let p = std::env::temp_dir().join(format!(
            "ei{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::SeqCst)
        ));
        std::fs::create_dir_all(&p).expect("mkdir");
        Dir(p)
    }
}

impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn sock(name: &str) -> (Dir, PathBuf) {
    let d = Dir::new();
    let p = d.0.join(name);
    (d, p)
}

/// Receive `n` requests, answer each with `ok`, return what they said.
fn answer(server: &ControlServer, n: usize, ok: bool) -> Vec<(Command, String)> {
    (0..n)
        .map(|_| {
            let r = server
                .requests()
                .recv_timeout(Duration::from_secs(5))
                .expect("request");
            let seen = (r.command, r.token.clone());
            r.respond(ok);
            seen
        })
        .collect()
}

#[test]
fn parse_table() {
    assert_eq!(
        parse_line(b"toggle\n"),
        Some((Command::Toggle, String::new()))
    );
    assert_eq!(
        parse_line(b"show tok-123\n"),
        Some((Command::Show, "tok-123".into()))
    );
    for bad in [&b""[..], b"quit\n", b"toggle a b\n", b"\xff\xfe"] {
        assert_eq!(parse_line(bad), None);
    }
    assert!(send(Path::new("/nonexistent"), "rm -rf", "").is_err());
}

#[test]
fn socket_path_from_env_override() {
    let env = |k: &str| (k == "TERMIGATOR_CONTROL_SOCKET").then(|| OsString::from("/x/y.sock"));
    assert_eq!(control_socket_path(&env), Some(PathBuf::from("/x/y.sock")));
}

#[test]
fn argv_split() {
    let a = |v: &[&str]| v.iter().map(OsString::from).collect::<Vec<_>>();
    let (c, rest) = split_remote_args(a(&["--show", "x", "--toggle", "y"]));
    assert_eq!(c, Some(Command::Show));
    assert_eq!(rest, a(&["x", "y"]));
    assert_eq!(split_remote_args(a(&["x"])), (None, a(&["x"])));
}

#[test]
fn toggle_and_show_round_trip() {
    let (_d, p) = sock("c.sock");
    let server = ControlServer::start(&p).expect("start").expect("bound");
    assert!(socket_alive(&p));
    let mode = std::fs::metadata(&p).expect("md").permissions().mode();
    assert_eq!(mode & 0o777, 0o600);
    let p2 = p.clone();
    let t = std::thread::spawn(move || {
        (
            send(&p2, "toggle", "tok-1"),
            send(&p2, "show", ""),
            send(&p2, "toggle", ""),
        )
    });
    let seen = answer(&server, 3, true);
    let (a, b, c) = t.join().expect("join");
    assert!(a.expect("a") && b.expect("b") && c.expect("c"));
    assert_eq!(seen[0], (Command::Toggle, "tok-1".into()));
    assert_eq!(seen[1], (Command::Show, String::new()));
}

#[test]
fn receiver_can_refuse_and_garbage_gets_err() {
    let (_d, p) = sock("c.sock");
    let server = ControlServer::start(&p).expect("start").expect("bound");
    let p2 = p.clone();
    let t = std::thread::spawn(move || send(&p2, "show", ""));
    answer(&server, 1, false);
    assert!(!t.join().expect("join").expect("valid"));
    // A line that does not parse never reaches the application.
    let mut s = UnixStream::connect(&p).expect("connect");
    s.write_all(b"quit now\n").expect("write");
    let mut r = String::new();
    s.read_to_string(&mut r).expect("read");
    assert_eq!(r, "err\n");
    assert!(server.requests().try_recv().is_err());
}

#[test]
fn nothing_running_means_not_delivered() {
    let (_d, p) = sock("none.sock");
    assert!(!send(&p, "show", "").expect("valid") && !socket_alive(&p));
}

#[test]
fn stale_socket_is_replaced_and_live_one_is_not_stolen() {
    let _g = SPAWN.lock().unwrap_or_else(|e| e.into_inner());
    let (_d, p) = sock("c.sock");
    let first = ControlServer::start(&p).expect("start").expect("bound");
    assert!(
        ControlServer::start(&p).expect("second").is_none(),
        "live socket stolen"
    );
    assert!(socket_alive(&p));
    drop(first);
    assert!(!p.exists(), "socket removed on drop");
    drop(UnixListener::bind(&p).expect("dead")); // a crash leaves the socket file behind
    assert!(p.exists(), "file gone");
    assert!(!socket_alive(&p), "still alive");
    let third = ControlServer::start(&p).expect("third").expect("replaced");
    assert!(socket_alive(&p));
    drop(third);
    assert!(!p.exists());
}

#[test]
fn regular_file_is_not_clobbered() {
    let (_d, p) = sock("c.sock");
    std::fs::write(&p, b"precious").expect("write");
    assert!(ControlServer::start(&p).is_err());
    assert_eq!(std::fs::read(&p).expect("read"), b"precious");
}

/// One `show` through the server, answered `ok` if it arrives within 500 ms.
fn round_trip(server: &ControlServer, p: &Path) -> bool {
    std::thread::scope(|s| {
        let client = s.spawn(|| send(p, "show", "t").expect("valid"));
        if let Ok(r) = server.requests().recv_timeout(Duration::from_millis(500)) {
            r.respond(true);
        }
        client.join().expect("client")
    })
}

#[test]
fn at_most_eight_connections_are_served_at_once() {
    let (_d, p) = sock("cap.sock");
    let server = ControlServer::start(&p).expect("start").expect("bound");
    // Twelve silent clients: a served one gets `err` when its handler's read times out, one over
    // the cap is closed without a reply.
    let slow: Vec<UnixStream> = (0..12)
        .map(|_| UnixStream::connect(&p).expect("connect"))
        .collect();
    let replies: Vec<Vec<u8>> = slow
        .into_iter()
        .map(|mut c| {
            c.set_read_timeout(Some(Duration::from_secs(5)))
                .expect("timeout");
            let mut got = Vec::new();
            c.read_to_end(&mut got).expect("read");
            got
        })
        .collect();
    assert_eq!(
        replies.iter().filter(|r| r.as_slice() == b"err\n").count(),
        8
    );
    assert_eq!(replies.iter().filter(|r| r.is_empty()).count(), 4);
    // The handlers are gone, so a normal client is served again.
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while !round_trip(&server, &p) {
        assert!(std::time::Instant::now() < deadline, "never served again");
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[test]
fn concurrent_starts_over_a_stale_socket_yield_one_server() {
    let _g = SPAWN.lock().unwrap_or_else(|e| e.into_inner());
    let (_d, p) = sock("race.sock");
    for _ in 0..20 {
        drop(UnixListener::bind(&p).expect("dead")); // a stale socket file
        let barrier = std::sync::Barrier::new(2);
        let started: Vec<Option<ControlServer>> = std::thread::scope(|s| {
            let starts: Vec<_> = (0..2)
                .map(|_| {
                    s.spawn(|| {
                        barrier.wait();
                        ControlServer::start(&p).expect("start")
                    })
                })
                .collect();
            starts
                .into_iter()
                .map(|t| t.join().expect("join"))
                .collect()
        });
        let servers: Vec<_> = started.into_iter().flatten().collect();
        assert_eq!(servers.len(), 1, "exactly one server");
        assert!(socket_alive(&p));
        drop(servers);
        assert!(!p.exists());
    }
}
