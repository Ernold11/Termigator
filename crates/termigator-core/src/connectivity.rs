//! Probe endpoint selection (first hop) and DNS/ping/TCP/banner probe.
//!
//! [`endpoint_for`] names the TCP endpoint a connection really talks to: the entry's own
//! `host:port` (defaults 22 / 3389 / 5900 / 23), `-p` / `-J` from `extra_ssh_args` when the
//! entry does not set them, and the first `proxy_jump` hop when there is one (the host is then
//! only reachable from the bastion, so the bastion is probed and [`endpoint_for`] says so in
//! `via_note`). [`probe_connection`] runs DNS, an informational ICMP ping, a TCP connect to
//! every resolved address and the server greeting (`SSH-` / `RFB`) as data ([`ProbeStep`],
//! [`ProbeReport`]); the TCP connect decides `ok`. Serial entries check the device node and
//! its permissions instead. Nothing authenticates and nothing is sent.
//!
//! Everything blocks: call it from a worker thread. A ping
//! host starting with `-` is refused. Ports outside 0..=65535 that come
//! from `-p` / the jump hop are reported as a failed connect, and OS error
//! texts come from `strerror` (resolver errors from Rust's `getaddrinfo` wrapper).
//! Golden: `compat/golden/connectivity_endpoint.json`,
//! `connectivity_first_hop.json`; tests in `tests/connectivity.rs`.

use std::io::Read;
use std::net::{SocketAddr, TcpStream, ToSocketAddrs};
use std::time::{Duration, Instant};

use crate::platform::{errno_text, find_in_path};
use regex::Regex;

use crate::config::doc::Value;
use crate::config::model::{
    PROTOCOL_RDP, PROTOCOL_SERIAL, PROTOCOL_SSH, PROTOCOL_TELNET, PROTOCOL_VNC,
    connection_protocol, proxy_jump_value, strip_space, truthy, value_int, value_str,
};
use crate::proc::runner::{self, Command};
use crate::proto::serial::serial_access_problem;
use crate::proto::vnc::{parse_port, quote_repr, text, text_stripped};
use crate::shell;

pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(5);

/// `(host, port, via_note, error)` of [`endpoint_for`]. The port is `i64` because a `-p` value
/// is not range-checked.
pub type Endpoint = (String, i64, String, Option<String>);

fn default_port(proto: &str) -> i64 {
    match proto {
        PROTOCOL_RDP => 3389,
        PROTOCOL_VNC => 5900,
        PROTOCOL_TELNET => 23,
        _ => 22,
    }
}

/// One check in the chain. `ok == None` is inconclusive (e.g. ICMP filtered) — informational.
#[derive(Debug, Clone, PartialEq)]
pub struct ProbeStep {
    pub name: String,
    pub ok: Option<bool>,
    pub detail: String,
    pub ms: Option<f64>,
}

impl ProbeStep {
    fn new(name: impl Into<String>, ok: Option<bool>, detail: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            ok,
            detail: detail.into(),
            ms: None,
        }
    }

    fn timed(mut self, ms: Option<f64>) -> Self {
        self.ms = ms;
        self
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ProbeReport {
    pub ok: bool,
    pub host: String,
    pub port: i64,
    pub protocol: &'static str,
    pub steps: Vec<ProbeStep>,
    pub summary: String,
    /// Set when the probed endpoint is a bastion rather than the connection's own host.
    pub via_note: String,
}

impl ProbeReport {
    /// `host:port`, with `[host]:port` for IPv6.
    pub fn endpoint(&self) -> String {
        if self.host.contains(':') {
            format!("[{}]:{}", self.host, self.port)
        } else {
            format!("{}:{}", self.host, self.port)
        }
    }
}

/// First `proxy_jump` hop as `(host, port)` — drops `user@`; the port is kept.
pub fn first_hop(raw: &Value) -> (String, Option<i64>) {
    let hops: Vec<String> = match raw {
        Value::Seq(items) => items
            .iter()
            .map(|x| strip_space(&value_str(x)).to_owned())
            .filter(|s| !s.is_empty())
            .collect(),
        other => text(Some(other))
            .split(',')
            .map(|p| strip_space(p).to_owned())
            .filter(|s| !s.is_empty())
            .collect(),
    };
    let Some(first) = hops.first() else {
        return (String::new(), None);
    };
    let hop = first.rsplit('@').next().unwrap_or_default();
    let (host, port_s) = if let Some(rest) = hop.strip_prefix('[') {
        // [2001:db8::1]:22
        let (h, tail) = rest.split_once(']').unwrap_or((rest, ""));
        (h, tail.trim_start_matches(':'))
    } else if hop.matches(':').count() == 1 {
        hop.split_once(':').unwrap_or((hop, ""))
    } else {
        (hop, "") // bare IPv6 or plain host
    };
    let port = if port_s.is_empty() {
        None
    } else {
        value_int(&Value::Str(port_s.to_owned()))
    };
    (host.to_owned(), port)
}

/// `-p PORT` / `-J HOP` in `extra_ssh_args` — ssh honours them, so the probe must too.
fn from_extra_args(conn: &Value) -> (Option<i64>, String) {
    let args: Vec<String> = match conn.get("extra_ssh_args") {
        Some(Value::Seq(items)) => items.iter().map(value_str).collect(),
        raw => shell::split(&text(raw)).unwrap_or_default(),
    };
    let digits = |s: &str| !s.is_empty() && s.chars().all(|c| c.is_ascii_digit());
    let (mut port, mut jump) = (None, String::new());
    for (i, a) in args.iter().enumerate() {
        let nxt = args.get(i + 1).map_or("", String::as_str);
        if a == "-p" && digits(nxt) {
            port = nxt.parse().ok().or(port);
        } else if a.starts_with("-p") && digits(&a[2..]) {
            port = a[2..].parse().ok().or(port);
        } else if a == "-J" && !nxt.is_empty() {
            jump = nxt.to_owned();
        } else if a.starts_with("-J") && a.len() > 2 {
            jump = a[2..].to_owned();
        }
    }
    (port, jump)
}

/// The TCP endpoint a probe should target: `(host, port, via_note, error)`.
pub fn endpoint_for(conn: &Value) -> Endpoint {
    let fail = |e: &str| (String::new(), 0, String::new(), Some(e.to_owned()));
    let proto = connection_protocol(conn);
    let default = default_port(proto);

    let host = text_stripped(conn.get("host"));
    if host.is_empty() {
        return fail("Host is missing.");
    }
    let mut host = host.trim_matches(['[', ']']).to_owned(); // bracketed IPv6 as in ssh targets

    let (extra_port, extra_jump) = if proto == PROTOCOL_SSH {
        from_extra_args(conn)
    } else {
        (None, String::new())
    };
    let own_default = extra_port.filter(|p| *p != 0).unwrap_or(default);
    let mut port = match parse_port(conn, own_default) {
        Ok(p) => p,
        Err(e) => return fail(&e),
    };

    let mut via_note = String::new();
    if proto == PROTOCOL_SSH {
        let raw = proxy_jump_value(conn)
            .filter(|v| truthy(v))
            .cloned()
            .unwrap_or(Value::Str(extra_jump));
        let (hop, hop_port) = first_hop(&raw);
        if !hop.is_empty() {
            let hop_port = hop_port.filter(|p| *p != 0).unwrap_or(22);
            via_note = format!(
                "{host}:{port} sits behind the bastion {hop}:{hop_port} and cannot be reached \
                 from here — probing the bastion instead."
            );
            (host, port) = (hop, hop_port);
        }
    }
    (host, port, via_note, None)
}

fn ms_since(t0: Instant) -> f64 {
    t0.elapsed().as_secs_f64() * 1000.0
}

/// `(addresses, elapsed_ms, error)`. A literal IP resolves to itself.
pub fn resolve_host(host: &str) -> (Vec<String>, f64, Option<String>) {
    let t0 = Instant::now();
    match (host, 0).to_socket_addrs() {
        Err(e) => (Vec::new(), ms_since(t0), Some(e.to_string())),
        Ok(infos) => {
            let mut addrs: Vec<String> = Vec::new();
            for a in infos {
                let a = a.ip().to_string();
                if !addrs.contains(&a) {
                    addrs.push(a);
                }
            }
            (addrs, ms_since(t0), None)
        }
    }
}

/// The OS message without Rust's " (os error N)" suffix.
fn os_message(e: &std::io::Error) -> String {
    match e.raw_os_error() {
        Some(code) => errno_text(code),
        None if e.kind() == std::io::ErrorKind::TimedOut => "timed out".to_owned(),
        None => e.to_string(),
    }
}

/// `(ok, elapsed_ms, banner, error)` for a plain TCP connect to `host:port`.
///
/// Tries every address the name resolves to (as returned) so a host with a dead AAAA record
/// is not reported as down. The socket is closed right away; nothing is sent, so this leaves
/// no login attempt in the server's logs.
pub fn tcp_probe(
    host: &str,
    port: i64,
    timeout: Duration,
    read_banner: bool,
) -> (bool, f64, String, Option<String>) {
    let Ok(port16) = u16::try_from(port) else {
        return (
            false,
            0.0,
            String::new(),
            Some(format!("port {port} out of range")),
        );
    };
    let addrs: Vec<SocketAddr> = match (host, port16).to_socket_addrs() {
        Ok(a) => a.collect(),
        Err(e) => {
            let err = format!("could not resolve {}: {e}", quote_repr(host));
            return (false, 0.0, String::new(), Some(err));
        }
    };
    let floor = Duration::from_millis(1); // a zero timeout is an error for the std calls
    let mut last_err = "no address to connect to".to_owned();
    for addr in addrs {
        let t0 = Instant::now();
        let mut sock = match TcpStream::connect_timeout(&addr, timeout.max(floor)) {
            Ok(s) => s,
            Err(e) => {
                last_err = os_message(&e);
                continue;
            }
        };
        let ms = ms_since(t0);
        let mut banner = String::new();
        if read_banner {
            let wait = timeout.min(Duration::from_secs(2)).max(floor);
            let mut buf = [0u8; 128];
            if sock.set_read_timeout(Some(wait)).is_ok()
                && let Ok(n) = sock.read(&mut buf)
            {
                banner = strip_space(&String::from_utf8_lossy(&buf[..n])).to_owned();
            }
        }
        return (true, ms, banner, None);
    }
    (false, 0.0, String::new(), Some(last_err))
}

fn ping_rtt(stdout: &str) -> Option<f64> {
    let re = Regex::new(r"time[=<]\s*([\d.]+)\s*ms").ok()?;
    re.captures(stdout)?.get(1)?.as_str().parse().ok()
}

/// `(ok, rtt_ms, detail)` from one `ping -c 1`. Purely informational: firewalls routinely drop
/// ICMP while the service port is wide open, so a failure never decides the verdict.
pub fn icmp_ping(host: &str, timeout: Duration) -> (bool, Option<f64>, String) {
    // A name starting with `-` would be read as a ping option (imported configs are untrusted).
    if host.starts_with('-') {
        return (
            false,
            None,
            format!("refusing to ping {}", quote_repr(host)),
        );
    }
    let Some(exe) = find_in_path("ping") else {
        return (false, None, "ping is not in PATH".to_owned());
    };
    let wait = (timeout.as_secs_f64().round_ties_even() as u64).max(1);
    let mut cmd = Command::new([
        exe.into_os_string(),
        "-n".into(),
        "-c".into(),
        "1".into(),
        "-W".into(),
        wait.to_string().into(),
        host.into(),
    ]);
    cmd.timeout = Some(Duration::from_secs(wait + 2));
    let out = runner::spawn(cmd).ok().and_then(runner::Running::wait);
    match out {
        Some(o) if !o.timed_out && !o.cancelled => {
            if o.code() != 0 {
                return (
                    false,
                    None,
                    "no ICMP reply (often filtered — not conclusive)".to_owned(),
                );
            }
            let rtt = ping_rtt(&String::from_utf8_lossy(&o.stdout));
            (true, rtt, "host replies to ICMP".to_owned())
        }
        _ => (false, None, "no ICMP reply".to_owned()),
    }
}

/// The first line of a non-empty, stripped text.
fn first_line(s: &str) -> &str {
    let end = s
        .find(|c: char| {
            matches!(
                c,
                '\n' | '\r' | '\x0b' | '\x0c' | '\x1c'
                    ..='\x1e' | '\u{85}' | '\u{2028}' | '\u{2029}'
            )
        })
        .unwrap_or(s.len());
    &s[..end]
}

fn head(s: &str, n: usize) -> String {
    s.chars().take(n).collect()
}

/// The server greeting as a step: SSH announces `SSH-2.0-…`, VNC `RFB 003.008`. RDP stays
/// silent until the client speaks, so it has no banner step.
fn banner_step(protocol: &str, banner: &str) -> Option<ProbeStep> {
    match protocol {
        PROTOCOL_SSH if banner.starts_with("SSH-") => Some(ProbeStep::new(
            "SSH banner",
            Some(true),
            head(first_line(banner), 80),
        )),
        PROTOCOL_SSH => Some(ProbeStep::new(
            "SSH banner",
            None,
            if banner.is_empty() {
                "port is open but sent no SSH greeting — is this really an SSH server?".to_owned()
            } else {
                format!("unexpected greeting: {}", quote_repr(&head(banner, 60)))
            },
        )),
        PROTOCOL_VNC if banner.starts_with("RFB") => Some(ProbeStep::new(
            "VNC handshake",
            Some(true),
            head(first_line(banner), 80),
        )),
        PROTOCOL_VNC => Some(ProbeStep::new(
            "VNC handshake",
            None,
            "port is open but sent no RFB greeting — is this really a VNC server?",
        )),
        _ => None,
    }
}

/// No network to probe — check the device node and the permission to open it.
fn probe_serial(conn: &Value) -> ProbeReport {
    let dev = text(conn.get("device"));
    let problem = if dev.is_empty() {
        Some("No device set.".to_owned())
    } else {
        serial_access_problem(&dev)
    };
    let ok = problem.is_none();
    let detail = problem
        .clone()
        .unwrap_or_else(|| "present, read/write allowed".to_owned());
    ProbeReport {
        ok,
        summary: problem.unwrap_or_else(|| format!("{dev} is ready.")),
        host: dev,
        port: 0,
        protocol: PROTOCOL_SERIAL,
        steps: vec![ProbeStep::new("Device", Some(ok), detail)],
        via_note: String::new(),
    }
}

/// Run the full check chain for a connection. The TCP connect decides `ok`.
pub fn probe_connection(conn: &Value, timeout: Duration, with_ping: bool) -> ProbeReport {
    let proto = connection_protocol(conn);
    if proto == PROTOCOL_SERIAL {
        return probe_serial(conn);
    }
    let (host, port, via_note, err) = endpoint_for(conn);
    if let Some(err) = err {
        return ProbeReport {
            ok: false,
            host,
            port,
            protocol: proto,
            steps: vec![ProbeStep::new("Endpoint", Some(false), err.clone())],
            summary: err,
            via_note: String::new(),
        };
    }
    let mut report = ProbeReport {
        ok: false,
        host,
        port,
        protocol: proto,
        steps: Vec::new(),
        summary: String::new(),
        via_note,
    };
    let host = report.host.clone();

    let (addrs, dns_ms, dns_err) = resolve_host(&host);
    if let Some(e) = dns_err {
        let detail = format!("cannot resolve {}: {e}", quote_repr(&host));
        report
            .steps
            .push(ProbeStep::new("DNS", Some(false), detail).timed(Some(dns_ms)));
        report.summary = format!(
            "Host {} does not resolve — check the name or your DNS.",
            quote_repr(&host)
        );
        return report;
    }
    let shown = addrs.iter().take(4).cloned().collect::<Vec<_>>().join(", ");
    let more = if addrs.len() > 4 { "…" } else { "" };
    report
        .steps
        .push(ProbeStep::new("DNS", Some(true), format!("{shown}{more}")).timed(Some(dns_ms)));

    if with_ping {
        let (ok, rtt, detail) = icmp_ping(&host, timeout);
        report
            .steps
            .push(ProbeStep::new("Ping (ICMP)", ok.then_some(true), detail).timed(rtt));
    }

    let want_banner = proto == PROTOCOL_SSH || proto == PROTOCOL_VNC;
    let (tcp_ok, tcp_ms, banner, tcp_err) = tcp_probe(&host, port, timeout, want_banner);
    if !tcp_ok {
        let why = tcp_err.unwrap_or_else(|| "connection failed".to_owned());
        report.steps.push(ProbeStep::new(
            format!("TCP {port}"),
            Some(false),
            why.clone(),
        ));
        report.summary = format!("Cannot reach {} — {why}.", report.endpoint());
        return report;
    }
    report.steps.push(
        ProbeStep::new(format!("TCP {port}"), Some(true), "port is open").timed(Some(tcp_ms)),
    );
    report.steps.extend(banner_step(proto, &banner));

    report.ok = true;
    report.summary = format!("{} is reachable ({tcp_ms:.0} ms).", report.endpoint());
    report
}
