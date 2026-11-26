//! "Add to Hosts…" on a tab: the tab's connection as a new entry or as an update of the one it
//! duplicates.

use super::*;
use crate::config::model::value_int;
use crate::proto::rdp::RDP_DEFAULT_PORT;
use crate::proto::telnet::TELNET_DEFAULT_PORT;
use crate::proto::vnc::VNC_DEFAULT_PORT;
use crate::vault::is_sealed;

/// Keys holding secrets, dropped at any depth (gateway hops, tunnels). `password_env` names a
/// variable and `credential` a shared credential; both stay.
const SECRET_KEYS: [&str; 2] = ["password", "passphrase"];

/// The connection without its secrets: [`SECRET_KEYS`] and sealed vault tokens, at any depth.
pub fn without_secrets(v: &Value) -> Value {
    match v {
        Value::Map(pairs) => Value::Map(
            pairs
                .iter()
                .filter(|(k, x)| {
                    !is_sealed(x)
                        && !matches!(k, Value::Str(s) if SECRET_KEYS.contains(&s.as_str()))
                })
                .map(|(k, x)| (k.clone(), without_secrets(x)))
                .collect(),
        ),
        Value::Seq(items) => Value::Seq(
            items
                .iter()
                .filter(|x| !is_sealed(x))
                .map(without_secrets)
                .collect(),
        ),
        other => other.clone(),
    }
}

/// What makes two entries the same connection: protocol, host (case-folded), port (unset = the
/// protocol's default) and user; a serial line by its device.
fn target(c: &Value) -> Option<(&'static str, String, i64, String)> {
    let proto = connection_protocol(c);
    if proto == PROTOCOL_SERIAL {
        let device = strip_space(&text(c, "device")).to_owned();
        return (!device.is_empty()).then(|| (proto, device, 0, String::new()));
    }
    let host = casefold(strip_space(&text(c, "host")));
    if host.is_empty() {
        return None;
    }
    let default = match proto {
        PROTOCOL_RDP => RDP_DEFAULT_PORT,
        PROTOCOL_VNC => VNC_DEFAULT_PORT,
        PROTOCOL_TELNET => TELNET_DEFAULT_PORT,
        _ => 22,
    };
    let port = c.get("port").and_then(value_int).unwrap_or(default);
    let user = strip_space(&text(c, "user")).to_owned();
    Some((proto, host, port, user))
}

/// The saved connection `entry` would duplicate: the first in config order with the same label
/// (case-folded) or the same target (protocol, host, port, user; serial: device).
pub fn matching_connection(cfg: &Value, entry: &Value) -> Option<usize> {
    let label = casefold(&entry_label(entry));
    let want = target(entry);
    let Some(Value::Seq(list)) = cfg.get("connections") else {
        return None;
    };
    list.iter().position(|c| {
        is_dict(c) && (casefold(&entry_label(c)) == label || (want.is_some() && target(c) == want))
    })
}

/// The stored entry with the tab's connection layered on: each key of `tab` (secrets dropped)
/// replaces the stored one, except `label` and `group`, which stay the entry's own; keys only
/// the stored entry has (its password, its other settings) are kept.
pub fn merged_with_tab(stored: &Value, tab: &Value) -> Value {
    let mut out = stored.clone();
    if let Value::Map(pairs) = without_secrets(tab) {
        for (k, v) in pairs {
            if let Value::Str(key) = &k
                && key != "label"
                && key != "group"
            {
                set_key(&mut out, key, v);
            }
        }
    }
    out
}
