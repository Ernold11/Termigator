//! SSH gateway for RDP / VNC entries (the local port is a parameter, since the free-port
//! lookup is done elsewhere).
//!
//! An RDP or VNC entry with `ssh_gateway: <label of an SSH entry>` is reached through that SSH
//! host. [`build_via_gateway`] returns a shell script: it starts
//! `ssh -N -L 127.0.0.1:<port>:<host>:<port>` in the background, a `trap` stops it when the
//! viewer exits (and the closing pty's SIGHUP does when the tab is closed), `/dev/tcp` polling
//! waits for the local port, then the viewer runs against `127.0.0.1:<port>`. For FreeRDP 3 a
//! `/server-name:<real host>` flag keeps the certificate check on the real name.
//!
//! The hop keeps only [`GATEWAY_KEYS`] of its SSH entry (no forwards, remote command, X11, TTY
//! or mosh) and never uses ControlMaster; `BatchMode=yes` is added unless a password
//! (`sshpass -e`) is used. ssh errors get the `SSH gateway: ` prefix.
//!
//! Golden: `compat/golden/gateway_*.json`, tests in
//! `tests/proto_gateway.rs`.

use crate::config::doc::Value;
use crate::config::model::{truthy, value_int, value_str};
use crate::proto::ssh::{Env, build_ssh_command_with};
use crate::proto::vnc::{text, text_stripped};
use crate::shell;

/// What the gateway hop keeps from its SSH entry.
pub const GATEWAY_KEYS: [&str; 12] = [
    "host",
    "user",
    "port",
    "identity_file",
    "extra_ssh_args",
    "proxy_jump",
    "jumphost",
    "jump_host",
    "jump_identity_file",
    "strict_host_key_checking",
    "password",
    "compression",
];

/// Polls of 0.2 s each before giving up on the tunnel.
const WAIT_TRIES: u32 = 100;

fn default_port(proto: &str) -> i64 {
    match proto {
        "rdp" => 3389,
        "vnc" => 5900,
        _ => 0,
    }
}

/// The SSH entry named `label` (the first one, like the sidebar shows them). Entries whose
/// `protocol` is not exactly `ssh` (case-insensitive, default `ssh`) never match.
pub fn find_gateway<'a>(conns: &'a [Value], label: Option<&Value>) -> Option<&'a Value> {
    let label = text_stripped(label);
    conns.iter().find(|c| {
        matches!(c, Value::Map(_))
            && match c.get("protocol") {
                Some(p) if truthy(p) => value_str(p).to_lowercase() == "ssh",
                _ => true,
            }
            && text_stripped(c.get("label")) == label
    })
}

/// `launch` as a mapping with `key` set (a non-mapping launch counts as empty).
fn with_entry(base: &Value, key: &str, value: Value) -> Value {
    let mut pairs = match base {
        Value::Map(p) => p.clone(),
        _ => Vec::new(),
    };
    match pairs
        .iter_mut()
        .find(|(k, _)| matches!(k, Value::Str(s) if s == key))
    {
        Some((_, v)) => *v = value,
        None => pairs.push((Value::Str(key.to_owned()), value)),
    }
    Value::Map(pairs)
}

/// `[sshpass -e] ssh [-o BatchMode=yes] -N -o ExitOnForwardFailure=yes -L 127.0.0.1:P:host:port … gw`.
pub fn gateway_tunnel_argv(
    target_host: &str,
    target_port: i64,
    gw: &Value,
    local_port: u16,
    launch: &Value,
    env: &Env,
) -> Result<Vec<String>, String> {
    let mut hop = Value::Map(
        GATEWAY_KEYS
            .iter()
            .filter_map(|k| gw.get(k).map(|v| (Value::Str((*k).to_owned()), v.clone())))
            .collect(),
    );
    hop = with_entry(
        &hop,
        "local_forwards",
        Value::Seq(vec![Value::Str(format!(
            "127.0.0.1:{local_port}:{target_host}:{target_port}"
        ))]),
    );
    // Its own connection, gone with the tab.
    let lo = with_entry(launch, "ssh_control_master", Value::Bool(false));
    let line = build_ssh_command_with(&hop, &lo, env).map_err(|e| format!("SSH gateway: {e}"))?;
    let mut argv = shell::split(&line).map_err(|e| format!("SSH gateway: {e}"))?;
    let i = argv
        .iter()
        .position(|a| a == "ssh")
        .ok_or_else(|| "SSH gateway: no ssh in the command.".to_owned())?;
    argv.splice(
        i + 1..i + 1,
        ["-N", "-o", "ExitOnForwardFailure=yes"].map(str::to_owned),
    );
    if text(hop.get("password")).is_empty() {
        // No TTY to answer a prompt on.
        argv.splice(i + 1..i + 1, ["-o", "BatchMode=yes"].map(str::to_owned));
    }
    Ok(argv)
}

/// The tab command for `conn` through the SSH entry `gw`. `build_client` is the entry's own
/// builder (RDP / VNC), called with the address rewritten to the tunnel; `local_port` is a free
/// local port chosen by the caller.
pub fn build_via_gateway(
    conn: &Value,
    gw: &Value,
    build_client: &dyn Fn(&Value) -> Result<String, String>,
    launch: &Value,
    local_port: u16,
    env: &Env,
) -> Result<String, String> {
    let proto = text(conn.get("protocol")).to_lowercase();
    let host = text_stripped(conn.get("host"));
    if host.is_empty() {
        return Err("Host is missing.".to_owned());
    }
    let port = match conn.get("port").filter(|v| truthy(v)) {
        Some(v) => value_int(v).ok_or_else(|| "Invalid port.".to_owned())?,
        None => default_port(&proto),
    };
    let tunnel = gateway_tunnel_argv(&host, port, gw, local_port, launch, env)?;
    let mut local = with_entry(conn, "host", Value::Str("127.0.0.1".to_owned()));
    local = with_entry(&local, "port", Value::Int(i64::from(local_port)));
    if let Value::Map(pairs) = &mut local {
        pairs.retain(|(k, _)| !matches!(k, Value::Str(s) if s == "ssh_gateway"));
    }
    let mut client = build_client(&local)?;
    // An RDP password pipe comes first.
    let exe = client
        .splitn(2, "| ")
        .last()
        .and_then(|s| s.split(' ').next())
        .unwrap_or("");
    if proto == "rdp" && exe.contains("freerdp3") {
        // FreeRDP 3: check the certificate against the real name, not 127.0.0.1 (FreeRDP 2
        // may not know /server-name, then the usual prompt shows 127.0.0.1).
        client = client.replacen(
            "/v:127.0.0.1",
            &format!(
                "/v:127.0.0.1 {}",
                shell::quote(&format!("/server-name:{host}"))
            ),
            1,
        );
    }
    let name = match gw.get("label").filter(|v| truthy(v)) {
        Some(v) => value_str(v),
        None => match gw.get("host").filter(|v| truthy(v)) {
            Some(v) => value_str(v),
            None => "gateway".to_owned(),
        },
    };
    let gw_name = shell::quote(&name);
    let tunnel = shell::join(&tunnel);
    Ok(format!(
        "{tunnel} & TERMIGATOR_GW=$!\n\
         trap 'kill $TERMIGATOR_GW 2>/dev/null' EXIT HUP TERM INT\n\
         for _ in $(seq {WAIT_TRIES}); do\n\
         \x20 (exec 3<>/dev/tcp/127.0.0.1/{local_port}) 2>/dev/null && break\n\
         \x20 kill -0 $TERMIGATOR_GW 2>/dev/null || {{ echo \"SSH gateway \"{gw_name}\" failed (see above).\"; exit 1; }}\n\
         \x20 sleep 0.2\n\
         done\n\
         (exec 3<>/dev/tcp/127.0.0.1/{local_port}) 2>/dev/null || {{ echo 'SSH gateway: the tunnel did not open in time.'; exit 1; }}\n\
         echo \"via SSH gateway \"{gw_name}\" → 127.0.0.1:{local_port}\"\n\
         {client}\n"
    ))
}
