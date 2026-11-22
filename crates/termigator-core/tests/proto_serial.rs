//! Golden tests for `termigator_core::proto::serial` (compat/golden/protocols_serial.json).
// Unix-only for now: uses modes, symlinks, sockets or sh stubs.
#![cfg(unix)]
#![allow(clippy::unwrap_used)] // test helpers decode trusted golden JSON

mod common;

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;

use serde_json::Value as J;
use termigator_core::config::doc::Value;
use termigator_core::proto::serial::{
    Flow, Parity, SerialSettings, build_serial_command, detect_serial_devices_in,
    serial_access_problem, serial_argv, serial_settings, serial_summary,
};
use termigator_core::shell;

fn decode(v: &J) -> Value {
    match v {
        J::Null => Value::Null,
        J::Bool(b) => Value::Bool(*b),
        J::String(s) => Value::Str(s.clone()),
        J::Array(a) => Value::Seq(a.iter().map(decode).collect()),
        J::Object(_) => match common::tagged(v) {
            Some(("$int", p)) => Value::Int(p.as_str().unwrap().parse().unwrap()),
            Some(("$float", p)) => Value::Float(p.as_str().unwrap().parse().unwrap()),
            Some(("$map", p)) => Value::Map(
                p.as_array()
                    .unwrap()
                    .iter()
                    .map(|kv| (decode(&kv[0]), decode(&kv[1])))
                    .collect(),
            ),
            other => panic!("bad value {other:?}"),
        },
        J::Number(n) => panic!("bare number {n}"),
    }
}

/// `{"ok": T}` / `{"err": msg}` compared with a Rust `Result<String, String>`.
fn check(n: &str, got: Result<String, String>, expect: &J) {
    match (&got, expect.get("ok"), expect.get("err")) {
        (Ok(g), Some(e), _) => assert_eq!(Some(g.as_str()), e.as_str(), "{n}"),
        (Err(g), _, Some(e)) => assert_eq!(Some(g.as_str()), e.as_str(), "{n}"),
        _ => panic!("{n}: got {got:?}, expected {expect}"),
    }
}

/// A settings object (`{"$map": ...}` of the golden) as the Rust struct, through a conn mapping.
fn settings_of(v: &J) -> SerialSettings {
    serial_settings(&decode(v)).expect("golden settings are valid")
}

/// What the Rust builder returns where the golden argv is `want` for `client`. Identical for
/// picocom and tio. For `screen` this is a deliberate deviation: screen 5 rejects
/// `parenb`/`parodd`/`cstopb` in any form, so non-default parity / 2 stop bits are refused and the
/// 8N1 line loses `-parenb` and `-cstopb`.
fn screen_adapted(client: &str, mut want: Vec<String>) -> Result<Vec<String>, String> {
    if client.rsplit('/').next() != Some("screen") || want.len() != 3 {
        return Ok(want);
    }
    let toks: Vec<&str> = want[2].split(',').collect();
    if toks.contains(&"parenb") {
        return Err(
            "screen cannot set parity (screen 5 rejects it) — install picocom or tio.".into(),
        );
    }
    if toks.contains(&"cstopb") {
        return Err(
            "screen cannot set 2 stop bits (screen 5 rejects it) — install picocom or tio.".into(),
        );
    }
    want[2] = toks
        .into_iter()
        .filter(|t| !matches!(*t, "-parenb" | "-cstopb"))
        .collect::<Vec<_>>()
        .join(",");
    Ok(want)
}

/// The exact screen lines of the supported cases.
#[test]
fn screen_line_has_no_parity_or_stop_bit_options() {
    let settings = |parity, stop_bits, data_bits, flow| SerialSettings {
        device: "/dev/ttyUSB0".into(),
        baud: 115_200,
        data_bits,
        parity,
        stop_bits,
        flow,
    };
    let line = |s: &SerialSettings| serial_argv("screen", s);
    assert_eq!(
        line(&settings(Parity::None, 1, 8, Flow::None)).unwrap(),
        ["screen", "/dev/ttyUSB0", "115200,cs8,-ixon,-ixoff"]
    );
    assert_eq!(
        line(&settings(Parity::None, 1, 7, Flow::Software)).unwrap()[2],
        "115200,cs7,ixon,ixoff"
    );
    let e = line(&settings(Parity::Even, 1, 8, Flow::None)).unwrap_err();
    assert!(e.starts_with("screen cannot set parity"), "{e}");
    let e = line(&settings(Parity::None, 2, 8, Flow::None)).unwrap_err();
    assert!(e.starts_with("screen cannot set 2 stop bits"), "{e}");
}

fn settings_json(s: &SerialSettings) -> String {
    format!(
        "{}|{}|{}|{}|{}|{}",
        s.device,
        s.baud,
        s.data_bits,
        s.parity.as_str(),
        s.stop_bits,
        s.flow.as_str()
    )
}

fn expected_settings(v: &J) -> String {
    let m = |k: &str| -> String {
        let pairs = v["$map"].as_array().unwrap();
        let hit = pairs.iter().find(|kv| kv[0].as_str() == Some(k)).unwrap();
        match common::tagged(&hit[1]) {
            Some((_, p)) => p.as_str().unwrap().to_owned(),
            None => hit[1].as_str().unwrap().to_owned(),
        }
    };
    format!(
        "{}|{}|{}|{}|{}|{}",
        m("device"),
        m("baud"),
        m("data_bits"),
        m("parity"),
        m("stop_bits"),
        m("flow")
    )
}

fn tmp_dir(tag: &str) -> PathBuf {
    let d = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("proto_serial-{}-{tag}", std::process::id()));
    let _ = fs::remove_dir_all(&d);
    fs::create_dir_all(&d).unwrap();
    d
}

#[test]
fn golden_cases() {
    let cases = common::golden("protocols_serial");
    assert!(cases.len() > 900);
    let tmp = tmp_dir("golden");
    let tmp_s = tmp.to_str().unwrap().to_owned();
    let is_root = rustix::process::geteuid().is_root();
    for case in &cases {
        let n = common::name(case);
        let expect = &case["expect"];
        match case["fn"].as_str().unwrap() {
            "settings" => match serial_settings(&decode(&case["conn"])) {
                Ok(s) => assert_eq!(settings_json(&s), expected_settings(&expect["ok"]), "{n}"),
                Err(e) => assert_eq!(Some(e.as_str()), expect["err"].as_str(), "{n}"),
            },
            "summary" => {
                let got = serial_summary(&decode(&case["conn"]));
                assert_eq!(Some(got.as_str()), expect["ok"].as_str(), "{n}");
            }
            "argv" => {
                let s = settings_of(&case["settings"]);
                let client = case["client"].as_str().unwrap();
                let got = serial_argv(client, &s);
                match (got, expect.get("ok"), expect.get("err")) {
                    (g, Some(e), _) => {
                        let want: Vec<String> = e
                            .as_array()
                            .unwrap()
                            .iter()
                            .map(|x| x.as_str().unwrap().to_owned())
                            .collect();
                        assert_eq!(g, screen_adapted(client, want), "{n}");
                    }
                    (Err(g), _, Some(e)) => assert_eq!(Some(g.as_str()), e.as_str(), "{n}"),
                    (g, ..) => panic!("{n}: got {g:?}, expected {expect}"),
                }
            }
            "build" => {
                let have: Vec<&str> = case["which"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|s| s.as_str().unwrap())
                    .collect();
                let which = |name: &str| have.contains(&name);
                let got =
                    build_serial_command(&decode(&case["conn"]), &decode(&case["launch"]), &which);
                match expect.get("ok").and_then(J::as_str) {
                    Some(line) => {
                        let argv = shell::split(line).unwrap();
                        let want = if argv[0].rsplit('/').next() == Some("screen") {
                            screen_adapted("screen", argv).map(|v| shell::join(&v))
                        } else {
                            Ok(line.to_owned())
                        };
                        assert_eq!(got, want, "{n}");
                    }
                    None => check(n, got, expect),
                }
            }
            "access" => {
                let label = n.strip_prefix("access:").unwrap();
                let path = tmp.join(format!("tty_{label}"));
                if let Some(mode) = case["mode"].as_u64() {
                    fs::write(&path, "").unwrap();
                    fs::set_permissions(&path, fs::Permissions::from_mode(mode as u32)).unwrap();
                }
                let got = serial_access_problem(path.to_str().unwrap())
                    .map(|m| m.replace(&tmp_s, "{tmp}"));
                if is_root && matches!(label, "none" | "ro" | "wo") {
                    assert_eq!(got, None, "{n}: root bypasses permissions");
                } else {
                    assert_eq!(got.as_deref(), expect["ok"].as_str(), "{n}");
                }
            }
            "detect" => {
                let label = n.strip_prefix("detect:").unwrap();
                let root = tmp.join(format!("root_{label}"));
                fs::create_dir_all(&root).unwrap();
                for f in case["files"].as_array().unwrap() {
                    let p = root.join(f.as_str().unwrap());
                    fs::create_dir_all(p.parent().unwrap()).unwrap();
                    fs::write(&p, "").unwrap();
                }
                let prefix = format!("{}/", root.to_str().unwrap());
                let got: Vec<String> = detect_serial_devices_in(&root)
                    .iter()
                    .map(|p| p.strip_prefix(&prefix).unwrap().to_owned())
                    .collect();
                let want: Vec<&str> = expect["ok"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|x| x.as_str().unwrap())
                    .collect();
                assert_eq!(got, want, "{n}");
            }
            other => panic!("{n}: unknown fn {other}"),
        }
    }
    let _ = fs::remove_dir_all(&tmp);
}
