//! Serial clients (picocom, tio, screen), settings validation, device detection.
//!
//! An entry `{device, baud, data_bits, parity, stop_bits, flow}` runs picocom, tio or screen
//! (`launch.serial_client`, else the first installed one). Defaults: 115200 8N1, no flow control.
//! Guards (ARCHITECTURE.md §7, guard 7): the device starts with `/` and has no whitespace or `,`.
//! `screen` cannot do 5/6 data bits, hardware flow control, parity or 2 stop bits (screen 5 rejects
//! `parenb`/`parodd`/`cstopb` in any form), so [`serial_argv`] refuses them instead of connecting
//! differently; 8N1 gets `115200,cs8,-ixon,-ixoff` (no `-parenb,-cstopb`).
//!
//! [`detect_serial_devices_in`] takes the filesystem root so tests can use a temporary tree;
//! [`detect_serial_devices`] uses `/`. Golden:
//! `compat/golden/protocols_serial.json`, tests in `tests/proto_serial.rs`.

use std::fs;
use std::path::Path;

use crate::config::doc::Value;
use crate::config::model::{truthy, value_int};
use crate::shell;

use super::vnc::{Which, is_space, quote_repr, text, text_stripped};

pub const SERIAL_CLIENTS: [&str; 3] = ["picocom", "tio", "screen"];
pub const BAUD_RATES: [i64; 14] = [
    300, 1200, 2400, 4800, 9600, 19200, 38400, 57600, 115200, 230400, 460800, 921600, 1500000,
    3000000,
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Parity {
    None,
    Even,
    Odd,
}

impl Parity {
    pub fn as_str(self) -> &'static str {
        match self {
            Parity::None => "none",
            Parity::Even => "even",
            Parity::Odd => "odd",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Flow {
    None,
    Hardware,
    Software,
}

impl Flow {
    pub fn as_str(self) -> &'static str {
        match self {
            Flow::None => "none",
            Flow::Hardware => "hardware",
            Flow::Software => "software",
        }
    }
}

/// Validated line settings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SerialSettings {
    pub device: String,
    pub baud: i64,
    pub data_bits: i64,
    pub parity: Parity,
    pub stop_bits: i64,
    pub flow: Flow,
}

/// Serial ports under `root` (`/` in production): stable `dev/serial/by-id` names first, then
/// USB adapters and on-board ports; each group sorted.
pub fn detect_serial_devices_in(root: &Path) -> Vec<String> {
    let dev = root.join("dev");
    type Keep = fn(&str) -> bool;
    let groups: [(&str, Keep); 5] = [
        ("serial/by-id", |n| !n.starts_with('.')),
        ("", |n| n.starts_with("ttyUSB")),
        ("", |n| n.starts_with("ttyACM")),
        ("", |n| n.starts_with("ttyAMA")),
        ("", |n| matches!(n, "ttyS0" | "ttyS1" | "ttyS2" | "ttyS3")),
    ];
    let mut out = Vec::new();
    for (sub, keep) in groups {
        let dir = dev.join(sub);
        let mut names: Vec<String> = fs::read_dir(&dir)
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|e| e.file_name().into_string().ok())
            .filter(|n| keep(n))
            .collect();
        names.sort();
        out.extend(
            names
                .into_iter()
                .map(|n| dir.join(n).to_string_lossy().into_owned()),
        );
    }
    out
}

/// Serial ports present now.
pub fn detect_serial_devices() -> Vec<String> {
    detect_serial_devices_in(Path::new("/"))
}

/// `launch.serial_client`, else the first of picocom, tio, screen that `which` finds.
pub fn resolve_serial_client(launch: &Value, which: Which) -> Option<String> {
    let over = text_stripped(launch.get("serial_client"));
    if !over.is_empty() {
        return Some(over);
    }
    SERIAL_CLIENTS
        .iter()
        .find(|c| which(c))
        .map(|c| (*c).to_owned())
}

/// `int(conn.get(key) or default)`; `None` when not convertible.
fn int_or(conn: &Value, key: &str, default: i64) -> Option<i64> {
    match conn.get(key) {
        Some(v) if truthy(v) => value_int(v),
        _ => Some(default),
    }
}

/// Validated line settings from an entry (defaults 115200 8N1, no flow control).
pub fn serial_settings(conn: &Value) -> Result<SerialSettings, String> {
    serial_settings_for(conn, false)
}

/// [`serial_settings`] for Windows: the device is a `COMn` port, not a path.
pub fn serial_settings_windows(conn: &Value) -> Result<SerialSettings, String> {
    serial_settings_for(conn, true)
}

/// `COM3`, `com12` or the long form `\\.\COM12` that Windows needs above COM9.
fn com_port(dev: &str) -> Option<String> {
    let name = dev.strip_prefix(r"\\.\").unwrap_or(dev);
    let digits = name.get(..3).filter(|p| p.eq_ignore_ascii_case("COM"))?;
    let n = &name[digits.len()..];
    (!n.is_empty() && n.len() <= 3 && n.chars().all(|c| c.is_ascii_digit() && c != '\0'))
        .then(|| format!("COM{n}"))
}

fn serial_settings_for(conn: &Value, windows: bool) -> Result<SerialSettings, String> {
    let dev = text_stripped(conn.get("device"));
    let dev = if windows {
        if dev.is_empty() {
            return Err("Serial port is missing (e.g. COM3).".to_owned());
        }
        com_port(&dev)
            .ok_or_else(|| format!("device {}: use a port such as COM3.", quote_repr(&dev)))?
    } else {
        if dev.is_empty() {
            return Err("Serial device is missing (e.g. /dev/ttyUSB0).".to_owned());
        }
        if !dev.starts_with('/') {
            return Err(format!(
                "device {}: use a full path such as /dev/ttyUSB0.",
                quote_repr(&dev)
            ));
        }
        if dev.chars().any(|c| is_space(c) || c == ',') {
            return Err(format!(
                "device {}: must not contain spaces or ','.",
                quote_repr(&dev)
            ));
        }
        dev
    };
    let nums = (
        int_or(conn, "baud", 115200),
        int_or(conn, "data_bits", 8),
        int_or(conn, "stop_bits", 1),
    );
    let (Some(baud), Some(data_bits), Some(stop_bits)) = nums else {
        return Err("baud / data_bits / stop_bits must be numbers.".to_owned());
    };
    if !(50..=4_000_000).contains(&baud) {
        return Err("baud must be between 50 and 4000000.".to_owned());
    }
    if !(5..=8).contains(&data_bits) {
        return Err("data_bits must be 5, 6, 7 or 8.".to_owned());
    }
    if !matches!(stop_bits, 1 | 2) {
        return Err("stop_bits must be 1 or 2.".to_owned());
    }
    let word = |key: &str| {
        let t = text(conn.get(key)).to_lowercase();
        if t.is_empty() { "none".to_owned() } else { t }
    };
    let parity = match word("parity").as_str() {
        "none" => Parity::None,
        "even" => Parity::Even,
        "odd" => Parity::Odd,
        _ => return Err("parity must be one of none, even, odd.".to_owned()),
    };
    let flow = match word("flow").as_str() {
        "none" => Flow::None,
        "hardware" => Flow::Hardware,
        "software" => Flow::Software,
        _ => return Err("flow must be one of none, hardware, software.".to_owned()),
    };
    Ok(SerialSettings {
        device: dev,
        baud,
        data_bits,
        parity,
        stop_bits,
        flow,
    })
}

/// Argument vector for `client` (matched by basename); `Err` for what `screen` cannot do or an
/// unknown client.
pub fn serial_argv(client: &str, s: &SerialSettings) -> Result<Vec<String>, String> {
    let name = client.rsplit('/').next().unwrap_or(client);
    let c = client.to_owned();
    let (baud, bits, stop) = (
        s.baud.to_string(),
        s.data_bits.to_string(),
        s.stop_bits.to_string(),
    );
    match name {
        "picocom" => {
            let flow = match s.flow {
                Flow::None => "n",
                Flow::Hardware => "h",
                Flow::Software => "x",
            };
            let parity = &s.parity.as_str()[..1];
            Ok([
                c,
                "--baud".into(),
                baud,
                "--databits".into(),
                bits,
                "--parity".into(),
                parity.into(),
                "--stopbits".into(),
                stop,
                "--flow".into(),
                flow.into(),
                s.device.clone(),
            ]
            .into())
        }
        "tio" => {
            let flow = match s.flow {
                Flow::None => "none",
                Flow::Hardware => "hard",
                Flow::Software => "soft",
            };
            Ok([
                c,
                "--baudrate".into(),
                baud,
                "--databits".into(),
                bits,
                "--parity".into(),
                s.parity.as_str().into(),
                "--stopbits".into(),
                stop,
                "--flow".into(),
                flow.into(),
                s.device.clone(),
            ]
            .into())
        }
        "screen" => {
            // screen's tty window takes only these stty flags (man screen, WINDOW TYPES):
            // no cs5/cs6 and no crtscts — refuse rather than silently connect differently.
            if !matches!(s.data_bits, 7 | 8) {
                return Err(
                    "screen supports only 7 or 8 data bits — install picocom or tio.".to_owned(),
                );
            }
            if s.flow == Flow::Hardware {
                return Err(
                    "screen cannot set hardware (RTS/CTS) flow control — install picocom or tio."
                        .to_owned(),
                );
            }
            // screen >= 5 rejects every parenb/parodd/cstopb form ("stty failed - invalid option?",
            // verified with 5.0.2), 4.x accepts them. So the default 8N1 line omits them (screen
            // leaves the line at no parity, one stop bit) and non-default settings are refused.
            if s.parity != Parity::None {
                return Err(
                    "screen cannot set parity (screen 5 rejects it) — install picocom or tio."
                        .to_owned(),
                );
            }
            if s.stop_bits != 1 {
                return Err(
                    "screen cannot set 2 stop bits (screen 5 rejects it) — install picocom or tio."
                        .to_owned(),
                );
            }
            let mut stty = vec![baud, format!("cs{}", s.data_bits)];
            stty.extend(
                if s.flow == Flow::Software {
                    ["ixon", "ixoff"]
                } else {
                    ["-ixon", "-ixoff"]
                }
                .map(str::to_owned),
            );
            Ok(vec![c, s.device.clone(), stty.join(",")])
        }
        _ => Err(format!(
            "unsupported serial client {} (use picocom, tio or screen)",
            quote_repr(client)
        )),
    }
}

/// `plink -serial COMn -sercfg <baud>,<bits>,<parity>,<stop>,<flow>` (PuTTY's command-line
/// client; Windows has no picocom). Flow: `N` none, `X` XON/XOFF, `R` RTS/CTS.
pub fn serial_argv_windows(s: &SerialSettings) -> Vec<String> {
    let parity = match s.parity {
        Parity::None => "n",
        Parity::Even => "e",
        Parity::Odd => "o",
    };
    let flow = match s.flow {
        Flow::None => "N",
        Flow::Software => "X",
        Flow::Hardware => "R",
    };
    vec![
        "plink".to_owned(),
        "-serial".to_owned(),
        s.device.clone(),
        "-sercfg".to_owned(),
        format!("{},{},{parity},{},{flow}", s.baud, s.data_bits, s.stop_bits),
    ]
}

/// The shell command line of a serial entry.
pub fn build_serial_command(conn: &Value, launch: &Value, which: Which) -> Result<String, String> {
    let s = serial_settings(conn)?;
    let client = resolve_serial_client(launch, which).ok_or_else(|| {
        "No serial terminal found — install picocom (sudo apt install picocom), tio or screen."
            .to_owned()
    })?;
    Ok(shell::join(&serial_argv(&client, &s)?))
}

/// Why opening `device` will fail, in words — the usual trap is the dialout group.
pub fn serial_access_problem(device: &str) -> Option<String> {
    let path = Path::new(device);
    if !path.exists() {
        return Some(format!(
            "{device} does not exist — is the adapter plugged in?"
        ));
    }
    if !crate::platform::can_read_write(path) {
        return Some(format!(
            "No permission to open {device}. Add yourself to its group (usually \
             dialout: sudo usermod -aG dialout $USER) and log in again."
        ));
    }
    None
}

/// `/dev/ttyUSB0 @115200 8N1` for the host list.
pub fn serial_summary(conn: &Value) -> String {
    match serial_settings(conn) {
        Ok(s) => format!(
            "{} @{} {}{}{}",
            s.device,
            s.baud,
            s.data_bits,
            s.parity.as_str()[..1].to_uppercase(),
            s.stop_bits
        ),
        Err(_) => text(conn.get("device")),
    }
}

#[cfg(test)]
mod windows_tests {
    use super::*;
    use crate::config::doc::load;

    fn conn(yaml: &str) -> Value {
        load(yaml).expect("yaml")
    }

    #[test]
    fn com_ports_are_normalised_and_paths_refused() {
        for (given, want) in [
            ("COM3", "COM3"),
            ("com12", "COM12"),
            (r"\\.\COM12", "COM12"),
        ] {
            let c = conn(&format!("{{device: '{given}'}}"));
            assert_eq!(serial_settings_windows(&c).expect(given).device, want);
        }
        for bad in [
            "/dev/ttyUSB0",
            "COM",
            "COM1234",
            "COM3 -load x",
            "LPT1",
            "COM3,9",
        ] {
            let c = conn(&format!("{{device: '{bad}'}}"));
            assert!(serial_settings_windows(&c).is_err(), "{bad}");
        }
        assert!(serial_settings_windows(&conn("{}")).is_err());
        // The Unix check is unchanged.
        assert!(serial_settings(&conn("{device: COM3}")).is_err());
    }

    #[test]
    fn plink_argv_carries_the_line_settings() {
        let c = conn(
            "{device: COM4, baud: 9600, data_bits: 7, parity: even, stop_bits: 2, flow: hardware}",
        );
        let argv = serial_argv_windows(&serial_settings_windows(&c).expect("settings"));
        assert_eq!(
            argv,
            ["plink", "-serial", "COM4", "-sercfg", "9600,7,e,2,R"]
        );
        let d = serial_argv_windows(&serial_settings_windows(&conn("{device: COM1}")).expect("s"));
        assert_eq!(d[4], "115200,8,n,1,N");
    }
}
