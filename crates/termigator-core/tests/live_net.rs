//! Network tools, SSH gateway, Telnet and serial sessions with real binaries: `nc` and bash
//! `/dev/tcp`, socat, sshpass, busybox telnetd + telnet, socat + screen (also picocom, the client
//! the builder prefers). Sessions run in the production pty ([`live::Term`]) instead of a
//! `QTermShellTab`.
// Unix-only for now: uses modes, symlinks, sockets or sh stubs.
#![cfg(unix)]

mod live;

use std::{
    ffi::OsString,
    fs::OpenOptions,
    io::{ErrorKind, Read, Write},
    net::TcpListener,
    os::unix::fs::{OpenOptionsExt, PermissionsExt},
    process::{Command, Stdio},
    time::Duration,
};

use live::{Daemon, Term, have, map, port_open, s, wait_until};
use termigator_core::{
    config::doc::Value,
    proc::tunnel::free_local_port,
    proto::{
        gateway::build_via_gateway,
        nettools::tool_argv,
        serial::build_serial_command,
        ssh::{self, Env},
        telnet::build_telnet_command,
    },
    shell,
};

const SECS: Duration = Duration::from_secs(10);

#[test]
fn port_check_really_checks() {
    let _g = live::serial();
    for prog in [Some("nc"), None] {
        if let Some(p) = prog
            && !have("port_check_really_checks[nc]", &[p])
        {
            continue;
        }
        let srv = TcpListener::bind("127.0.0.1:0").expect("bind");
        let port = srv.local_addr().expect("addr").port();
        let only = |n: &str| Some(n) == prog;
        let argv = tool_argv("port", "127.0.0.1", i64::from(port), &only).expect("argv");
        let run = || {
            Command::new("bash")
                .arg("-c")
                .arg(shell::join(&argv))
                .output()
                .expect("bash")
        };
        let r = run();
        let (out, err) = (
            String::from_utf8_lossy(&r.stdout),
            String::from_utf8_lossy(&r.stderr),
        );
        assert!(
            r.status.success() && (out.contains("open") || err.contains("succeeded")),
            "{prog:?} open port: {:?} {out:?} {err:?}",
            r.status
        );
        drop(srv);
        let r = run();
        assert!(
            String::from_utf8_lossy(&r.stdout).contains("closed") || !r.status.success(),
            "{prog:?} closed port: {r:?}"
        );
    }
}

/// Stand-in for ssh: honours `-L 127.0.0.1:P:host:port` with a real forwarder.
const STUB_SSH: &str = r#"#!/usr/bin/env bash
while [ $# -gt 0 ]; do
  case "$1" in
    -L) spec="$2"; shift 2 ;;
    *) shift ;;
  esac
done
IFS=: read -r bind lport thost tport <<<"$spec"
exec socat TCP-LISTEN:"$lport",bind="$bind",reuseaddr,fork TCP:"$thost":"$tport"
"#;

fn gw(password: Option<&str>) -> Value {
    let mut pairs = vec![
        ("label", s("bastion")),
        ("host", s("bastion.example")),
        ("user", s("jump")),
        ("local_forwards", Value::Seq(vec![s("1:x:1")])),
        ("remote_command", s("tmux")),
        ("x11_forwarding", Value::Bool(true)),
    ];
    if let Some(p) = password {
        pairs.push(("password", s(p)));
    }
    map(&pairs)
}

/// The tab command goes through the tunnel, and closing the tab takes the tunnel down — also
/// behind sshpass, which runs ssh on a pty of its own.
fn gateway_lifecycle(password: Option<&str>) {
    let test = format!("live_gateway_lifecycle[{password:?}]");
    let mut need = vec!["socat", "bash"];
    if password.is_some() {
        need.push("sshpass");
    }
    if !have(&test, &need) {
        return;
    }
    let dir = live::scratch("gw");
    let bin = dir.join("bin");
    std::fs::create_dir(&bin).expect("bin");
    live::script(&bin.join("ssh"), STUB_SSH);
    let path = live::path_with(&bin);
    let path_s = path.to_str().expect("utf-8 PATH").to_owned();
    let env = Env {
        which: Box::new(move |n| ssh::which(n, Some(&path_s))),
        control_dir: None,
        home: None,
    };
    let tport = free_local_port().expect("port");
    let _echo = Daemon::spawn(
        Command::new("socat")
            .arg(format!("TCP-LISTEN:{tport},bind=127.0.0.1,reuseaddr,fork"))
            .arg("EXEC:cat"),
    );
    assert!(wait_until(SECS, || port_open(tport)), "echo server");
    let lport = free_local_port().expect("port");
    let viewer = |c: &Value| -> Result<String, String> {
        let Some(Value::Int(p)) = c.get("port") else {
            return Err(format!("no port in {c:?}"));
        };
        Ok(format!(
            "exec 3<>/dev/tcp/127.0.0.1/{p}; echo PING >&3; read -r l <&3; echo GOT:$l; sleep 60"
        ))
    };
    let conn = map(&[
        ("protocol", s("vnc")),
        ("host", s("127.0.0.1")),
        ("port", Value::Int(i64::from(tport))),
    ]);
    let launch = map(&[("ssh_keepalive", Value::Bool(false))]);
    let cmd = build_via_gateway(&conn, &gw(password), &viewer, &launch, lport, &env)
        .expect("gateway command");
    if password.is_some() {
        assert!(cmd.starts_with("sshpass -e "), "{cmd}");
    }
    let extra: Vec<(&str, &str)> = password.map(|p| ("SSHPASS", p)).into_iter().collect();
    let mut tab = Term::bash(&cmd, &[("PATH", &path)], &extra);
    tab.expect("GOT:PING", SECS); // through the tunnel
    assert!(port_open(lport));
    tab.close();
    assert!(
        wait_until(Duration::from_secs(5), || !port_open(lport)),
        "the gateway outlived its tab"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn live_gateway_lifecycle() {
    let _g = live::serial();
    gateway_lifecycle(None);
}

#[test]
fn live_gateway_lifecycle_behind_sshpass() {
    let _g = live::serial();
    gateway_lifecycle(Some("pw"));
}

#[test]
fn live_telnet_session() {
    let _g = live::serial();
    if !have("live_telnet_session", &["busybox", "telnet"]) {
        return;
    }
    let port = free_local_port().expect("port");
    let _srv = Daemon::spawn(
        Command::new("busybox")
            .args(["telnetd", "-F", "-p", &port.to_string(), "-b", "127.0.0.1"])
            .args(["-l", "/bin/sh"])
            .stdout(Stdio::null())
            .stderr(Stdio::null()),
    );
    assert!(
        wait_until(SECS, || port_open(port)),
        "telnetd never listened"
    );
    let conn = map(&[
        ("host", s("127.0.0.1")),
        ("port", Value::Int(i64::from(port))),
    ]);
    let cmd = build_telnet_command(&conn, &live::installed).expect("telnet command");
    let mut tab = Term::bash(&cmd, &[], &[]);
    // `$ ` for a user, `# ` for root (the CI container).
    assert!(
        wait_until(SECS, || {
            let t = tab.text();
            t.contains("$ ") || t.contains("# ")
        }),
        "no shell prompt over telnet:\n{}",
        tab.text()
    );
    tab.send("echo HI$((40+2))\n");
    tab.expect("HI42", SECS);
    tab.close();
}

/// Kills the screen sessions whose sockets are in `screendir` (`<pid>.<tty>.<host>`).
fn kill_screens(screendir: &std::path::Path) {
    for e in std::fs::read_dir(screendir).into_iter().flatten().flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        if let Some(Ok(pid)) = name.split('.').next().map(str::parse::<i32>) {
            let _ = nix::sys::signal::kill(
                nix::unistd::Pid::from_raw(pid),
                nix::sys::signal::Signal::SIGTERM,
            );
        }
    }
}

/// A live serial session with `screen` as the client.
#[test]
fn live_serial_session() {
    let _g = live::serial();
    let test = "live_serial_session[screen]";
    if !have(test, &["socat", "screen"]) {
        return;
    }
    serial_session(test, "screen");
}

/// The same live session with picocom, the first client the builder picks.
#[test]
fn live_serial_session_picocom() {
    let _g = live::serial();
    let test = "live_serial_session[picocom]";
    if have(test, &["socat", "picocom"]) {
        serial_session(test, "picocom");
    }
}

/// A socat pty pair stands in for the device: what the device sends shows in the console, and
/// what is typed reaches the device.
fn serial_session(test: &str, client: &str) {
    let dir = live::scratch("serial");
    let (dev, peer) = (dir.join("ttyDEV"), dir.join("ttyPEER"));
    let _pair = Daemon::spawn(
        Command::new("socat")
            .arg(format!("pty,raw,echo=0,link={}", dev.display()))
            .arg(format!("pty,raw,echo=0,link={}", peer.display()))
            .stdout(Stdio::null())
            .stderr(Stdio::null()),
    );
    assert!(
        wait_until(SECS, || dev.exists() && peer.exists()),
        "socat pty pair"
    );
    let conn = map(&[
        ("device", s(dev.to_str().expect("utf-8"))),
        ("baud", Value::Int(115_200)),
    ]);
    let launch = map(&[("serial_client", s(client))]);
    let cmd = build_serial_command(&conn, &launch, &live::installed).expect("serial command");
    assert_eq!(shell::split(&cmd).expect("split")[0], client, "{test}");
    // A private socket directory (screen insists on mode 700) keeps the sessions findable.
    let screendir = dir.join("screens");
    std::fs::create_dir(&screendir).expect("screendir");
    std::fs::set_permissions(&screendir, std::fs::Permissions::from_mode(0o700)).expect("chmod");
    let screendir_os = OsString::from(&screendir);
    let mut tab = Term::bash(&cmd, &[("SCREENDIR", &screendir_os)], &[]);
    std::thread::sleep(Duration::from_millis(800));
    let mut fd = OpenOptions::new()
        .read(true)
        .write(true)
        .custom_flags((nix::fcntl::OFlag::O_NOCTTY | nix::fcntl::OFlag::O_NONBLOCK).bits())
        .open(&peer)
        .expect("open peer");
    fd.write_all(b"FROM-DEVICE\r\n").expect("device writes");
    tab.expect("FROM-DEVICE", SECS); // the console shows what the device sends
    // picocom needs a moment between reading the device and taking keyboard input.
    std::thread::sleep(Duration::from_millis(200));
    tab.send("typed"); // and what is typed reaches the device
    let mut got = Vec::new();
    let reached = wait_until(Duration::from_secs(5), || {
        let mut buf = [0u8; 100];
        match fd.read(&mut buf) {
            Ok(n) => got.extend_from_slice(&buf[..n]),
            Err(e) if e.kind() == ErrorKind::WouldBlock => {}
            Err(e) => panic!("read peer: {e}"),
        }
        got.windows(5).any(|w| w == b"typed")
    });
    assert!(
        reached,
        "{test}: device got {:?}",
        String::from_utf8_lossy(&got)
    );
    tab.close();
    kill_screens(&screendir);
    let _ = std::fs::remove_dir_all(&dir);
}
