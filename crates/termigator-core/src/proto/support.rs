//! Which kinds of new entries this machine can open, for the "New connection" menus: a kind it
//! cannot open is shown greyed out, with the reason, instead of letting the user build an entry
//! that only fails when connecting.

use crate::config::doc::Value;
use crate::proto::vnc::{VNC_CLIENT_CANDIDATES, Which, text_stripped};

/// Why `kind` (`ssh`, `rdp`, `vnc`, `telnet`, `serial`, `favorite`) cannot be opened here, or
/// `None`. `windows`: the platform; `launch`: the `launch` section (a configured VNC client counts
/// as found); `which`: is a program on PATH. Linux opens every kind (a missing client is reported
/// when connecting, as before).
pub fn kind_unavailable(kind: &str, windows: bool, launch: &Value, which: Which) -> Option<String> {
    if !windows {
        return None;
    }
    match kind {
        "serial" => Some("Serial connections are not supported on Windows yet.".to_owned()),
        "vnc"
            if text_stripped(launch.get("vnc_client")).is_empty()
                && !VNC_CLIENT_CANDIDATES.iter().any(|n| which(n)) =>
        {
            Some("No VNC viewer found: install TigerVNC (vncviewer.exe on PATH).".to_owned())
        }
        "telnet" if crate::proto::telnet::telnet_client_argv(which).is_none() => Some(
            "telnet.exe is not installed: turn on the Windows feature \"Telnet Client\"."
                .to_owned(),
        ),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn linux_opens_everything() {
        for kind in ["ssh", "rdp", "vnc", "telnet", "serial", "favorite"] {
            assert_eq!(
                kind_unavailable(kind, false, &Value::Null, &|_| false),
                None
            );
        }
    }

    #[test]
    fn windows_greys_serial_and_missing_clients() {
        let none = |_: &str| false;
        assert!(kind_unavailable("serial", true, &Value::Null, &none).is_some());
        assert!(kind_unavailable("vnc", true, &Value::Null, &none).is_some());
        assert!(kind_unavailable("telnet", true, &Value::Null, &none).is_some());
        for kind in ["ssh", "rdp", "favorite"] {
            assert_eq!(
                kind_unavailable(kind, true, &Value::Null, &none),
                None,
                "{kind}"
            );
        }
        let all = |_: &str| true;
        assert_eq!(kind_unavailable("vnc", true, &Value::Null, &all), None);
        assert_eq!(kind_unavailable("telnet", true, &Value::Null, &all), None);
        let configured =
            crate::config::doc::load("vnc_client: C:/TigerVNC/vncviewer.exe\n").expect("yaml");
        assert_eq!(kind_unavailable("vnc", true, &configured, &none), None);
    }
}
