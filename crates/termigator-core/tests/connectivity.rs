//! Connectivity endpoint selection against the golden fixtures (compat/golden/connectivity_*.json);
//! the probes run against throwaway listeners on 127.0.0.1 (no external network).
mod common;

use std::io::Write;
use std::net::TcpListener;
use std::thread;
use std::time::Duration;

use serde_json::Value as J;
use termigator_core::config::doc::Value;
use termigator_core::connectivity::{
    endpoint_for, first_hop, icmp_ping, probe_connection, resolve_host, tcp_probe,
};

fn from_golden(v: &J) -> Value {
    if let Some((tag, p)) = common::tagged(v) {
        let s = || p.as_str().unwrap_or_default().to_owned();
        return match tag {
            "$int" => Value::Int(s().parse().expect("i64")),
            "$float" => Value::Float(s().parse().expect("f64")),
            "$map" => Value::Map(
                p.as_array()
                    .into_iter()
                    .flatten()
                    .map(|kv| (from_golden(&kv[0]), from_golden(&kv[1])))
                    .collect(),
            ),
            other => panic!("unknown tag {other}"),
        };
    }
    match v {
        J::Null => Value::Null,
        J::Bool(b) => Value::Bool(*b),
        J::String(s) => Value::Str(s.clone()),
        J::Array(a) => Value::Seq(a.iter().map(from_golden).collect()),
        other => panic!("not a golden value: {other}"),
    }
}

fn arg(case: &J) -> Value {
    from_golden(&case["input"][0])
}

fn conn(pairs: &[(&str, Value)]) -> Value {
    Value::Map(
        pairs
            .iter()
            .map(|(k, v)| (Value::Str((*k).to_owned()), v.clone()))
            .collect(),
    )
}

fn s(t: &str) -> Value {
    Value::Str(t.to_owned())
}

const T: Duration = Duration::from_secs(2);

fn field_str(e: &Value, k: &str) -> Option<String> {
    match e.get(k) {
        Some(Value::Str(t)) => Some(t.clone()),
        _ => None,
    }
}

fn field_int(e: &Value, k: &str) -> Option<i64> {
    match e.get(k) {
        Some(Value::Int(i)) => Some(*i),
        _ => None,
    }
}

#[test]
fn golden_endpoint_for() {
    let cases = common::golden("connectivity_endpoint");
    assert!(cases.len() > 200);
    for c in &cases {
        let (host, port, note, err) = endpoint_for(&arg(c));
        let e = from_golden(&c["expect"]);
        let name = common::name(c);
        assert_eq!(Some(host), field_str(&e, "host"), "{name}: host");
        assert_eq!(Some(port), field_int(&e, "port"), "{name}: port");
        assert_eq!(Some(note), field_str(&e, "via_note"), "{name}: via_note");
        assert_eq!(err, field_str(&e, "error"), "{name}: error");
    }
}

#[test]
fn golden_first_hop() {
    let cases = common::golden("connectivity_first_hop");
    assert!(cases.len() > 30);
    for c in &cases {
        let (host, port) = first_hop(&arg(c));
        let e = from_golden(&c["expect"]);
        let name = common::name(c);
        assert_eq!(Some(host), field_str(&e, "host"), "{name}: host");
        assert_eq!(port, field_int(&e, "port"), "{name}: port");
    }
}

#[test]
fn endpoint_messages() {
    assert_eq!(
        endpoint_for(&conn(&[])).3.as_deref(),
        Some("Host is missing.")
    );
    let bad = conn(&[("host", s("h")), ("port", s("abc"))]);
    assert_eq!(endpoint_for(&bad).3.as_deref(), Some("Invalid port."));
    let big = conn(&[("host", s("h")), ("port", Value::Int(70000))]);
    assert!(endpoint_for(&big).3.is_some());
    let jump = conn(&[
        ("host", s("10.0.0.9")),
        ("port", Value::Int(2222)),
        ("proxy_jump", s("admin@bastion.example:22,second")),
    ]);
    let (host, port, note, err) = endpoint_for(&jump);
    assert_eq!((host.as_str(), port, err), ("bastion.example", 22, None));
    assert!(note.contains("bastion.example"));
}

// --- probes -------------------------------------------------------------------

/// A listener that accepts one connection, sends `banner` (if any) and closes.
fn listener(banner: Option<&'static [u8]>) -> u16 {
    let srv = TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = srv.local_addr().expect("addr").port();
    thread::spawn(move || {
        if let Ok((mut c, _)) = srv.accept()
            && let Some(b) = banner
        {
            let _ = c.write_all(b);
        }
    });
    port
}

fn free_port() -> u16 {
    let srv = TcpListener::bind("127.0.0.1:0").expect("bind");
    srv.local_addr().expect("addr").port()
}

fn ssh_conn(port: u16) -> Value {
    conn(&[
        ("host", s("127.0.0.1")),
        ("port", Value::Int(i64::from(port))),
    ])
}

fn names(r: &termigator_core::connectivity::ProbeReport) -> Vec<&str> {
    r.steps.iter().map(|x| x.name.as_str()).collect()
}

#[test]
fn resolve_host_literal_and_unknown() {
    let (addrs, _ms, err) = resolve_host("127.0.0.1");
    assert!(err.is_none());
    assert_eq!(addrs, ["127.0.0.1"]);
    let (_a, _ms, err) = resolve_host("no-such-host.invalid");
    assert!(err.is_some());
}

#[test]
fn tcp_probe_open_port_and_banner() {
    let port = listener(None);
    let (ok, ms, banner, err) = tcp_probe("127.0.0.1", i64::from(port), T, false);
    assert!(ok && err.is_none() && banner.is_empty() && ms >= 0.0);

    let port = listener(Some(b"SSH-2.0-OpenSSH_9.6\r\n"));
    let (ok, _ms, banner, err) = tcp_probe("127.0.0.1", i64::from(port), T, true);
    assert!(ok && err.is_none());
    assert!(banner.starts_with("SSH-2.0-OpenSSH_9.6"));
}

// Windows reports a closed loopback port only after ~2 s of SYN retries, so the probe times out first.
#[cfg(unix)]
#[test]
fn tcp_probe_closed_port() {
    let (ok, _ms, _banner, err) = tcp_probe("127.0.0.1", i64::from(free_port()), T, false);
    assert!(!ok);
    assert_eq!(err.as_deref(), Some("Connection refused"));
}

#[test]
fn probe_reachable_with_ssh_banner() {
    let port = listener(Some(b"SSH-2.0-test\r\n"));
    let rep = probe_connection(&ssh_conn(port), T, false);
    assert!(rep.ok);
    assert_eq!(rep.endpoint(), format!("127.0.0.1:{port}"));
    assert_eq!(names(&rep), ["DNS", &format!("TCP {port}"), "SSH banner"]);
    let last = rep.steps.last().expect("step");
    assert_eq!(
        (last.ok, last.detail.as_str()),
        (Some(true), "SSH-2.0-test")
    );
    assert!(
        rep.summary
            .starts_with(&format!("127.0.0.1:{port} is reachable ("))
    );
}

#[test]
fn probe_open_port_without_ssh_greeting_is_inconclusive() {
    // The listener accepts and closes without sending: the banner read ends at once.
    let port = listener(None);
    let rep = probe_connection(&ssh_conn(port), T, false);
    assert!(rep.ok); // the port decides reachability
    let last = rep.steps.last().expect("step");
    assert_eq!((last.name.as_str(), last.ok), ("SSH banner", None));
    assert!(last.detail.contains("sent no SSH greeting"));
}

#[test]
fn probe_silent_server_times_out_the_banner_read() {
    let srv = TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = srv.local_addr().expect("addr").port();
    let hold = thread::spawn(move || {
        let conn = srv.accept();
        thread::sleep(Duration::from_secs(1)); // keep the socket open, send nothing
        drop(conn);
    });
    let rep = probe_connection(&ssh_conn(port), Duration::from_millis(300), false);
    assert!(rep.ok);
    assert_eq!(rep.steps.last().map(|x| x.ok), Some(None));
    let _ = hold.join();
}

#[test]
fn probe_unexpected_greeting_is_quoted() {
    let port = listener(Some(b"HTTP/1.1 400\r\n"));
    let rep = probe_connection(&ssh_conn(port), T, false);
    let last = rep.steps.last().expect("step");
    assert_eq!(last.ok, None);
    assert_eq!(last.detail, "unexpected greeting: 'HTTP/1.1 400'");
}

#[test]
fn probe_vnc_handshake() {
    let port = listener(Some(b"RFB 003.008\n"));
    let c = conn(&[
        ("host", s("127.0.0.1")),
        ("port", Value::Int(i64::from(port))),
        ("protocol", s("vnc")),
    ]);
    let rep = probe_connection(&c, T, false);
    assert!(rep.ok);
    let last = rep.steps.last().expect("step");
    assert_eq!((last.name.as_str(), last.ok), ("VNC handshake", Some(true)));
    assert_eq!(last.detail, "RFB 003.008");
}

#[test]
fn probe_refused_port_fails() {
    let rep = probe_connection(&ssh_conn(free_port()), T, false);
    assert!(!rep.ok);
    assert!(rep.summary.contains("Cannot reach"));
    assert_eq!(rep.steps.last().map(|x| x.ok), Some(Some(false)));
}

#[test]
fn probe_unresolvable_host_stops_at_dns() {
    let rep = probe_connection(&conn(&[("host", s("no-such-host.invalid"))]), T, false);
    assert!(!rep.ok);
    assert_eq!(names(&rep), ["DNS"]);
    assert!(rep.summary.contains("does not resolve"));
}

#[test]
fn probe_rdp_has_no_banner_step() {
    let port = listener(None);
    let c = conn(&[
        ("host", s("127.0.0.1")),
        ("port", Value::Int(i64::from(port))),
        ("protocol", s("rdp")),
    ]);
    let rep = probe_connection(&c, T, false);
    assert!(rep.ok);
    assert_eq!(names(&rep), ["DNS", &format!("TCP {port}")]);
}

#[test]
fn probe_reports_invalid_entry() {
    let rep = probe_connection(&conn(&[("host", s(""))]), Duration::from_secs(1), false);
    assert!(!rep.ok);
    assert_eq!(rep.summary, "Host is missing.");
}

// `/dev/null` and `/dev/tty*` paths: Unix only (Windows serial ports are COMn).
#[cfg(unix)]
#[test]
fn probe_serial_checks_the_device() {
    let none = probe_connection(&conn(&[("protocol", s("serial"))]), T, false);
    assert!(!none.ok);
    assert_eq!(none.summary, "No device set.");
    let missing = conn(&[
        ("protocol", s("serial")),
        ("device", s("/dev/termigator-no-such-tty")),
    ]);
    let rep = probe_connection(&missing, T, false);
    assert!(!rep.ok);
    assert!(rep.summary.contains("does not exist"));
    let dev_null = conn(&[("protocol", s("serial")), ("device", s("/dev/null"))]);
    let rep = probe_connection(&dev_null, T, false);
    assert!(rep.ok);
    assert_eq!(rep.summary, "/dev/null is ready.");
    assert_eq!(rep.steps[0].detail, "present, read/write allowed");
}

#[test]
fn probe_with_ping_adds_an_informational_step() {
    let port = listener(None);
    let rep = probe_connection(&ssh_conn(port), T, true);
    assert!(rep.ok); // ping outcome never decides the verdict
    assert_eq!(names(&rep)[..2], ["DNS", "Ping (ICMP)"]);
    assert_ne!(rep.steps[1].ok, Some(false));
}

#[test]
fn ping_refuses_a_host_that_looks_like_an_option() {
    let (ok, rtt, detail) = icmp_ping("-f", Duration::from_secs(1));
    assert!(!ok && rtt.is_none(), "{detail}");
    assert!(detail.contains("refusing"), "{detail}");
}
