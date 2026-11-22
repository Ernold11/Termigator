//! Golden tests for `termigator_core::proto::vnc` (compat/golden/protocols_vnc.json).
#![allow(clippy::unwrap_used)] // test helpers decode trusted golden JSON

mod common;

use serde_json::Value as J;
use termigator_core::config::doc::Value;
use termigator_core::proto::vnc::{build_vnc_command, resolve_vnc_client};

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

fn avail(case: &J) -> Vec<String> {
    let names = case["which"].as_array().unwrap();
    names
        .iter()
        .map(|s| s.as_str().unwrap().to_owned())
        .collect()
}

#[test]
fn build_matches_golden() {
    let cases = common::golden("protocols_vnc");
    assert!(cases.len() > 400);
    for case in &cases {
        let n = common::name(case);
        let have = avail(case);
        let which = |name: &str| have.iter().any(|h| h == name);
        let got = build_vnc_command(&decode(&case["conn"]), &decode(&case["launch"]), &which);
        match (&got, case["expect"].get("ok"), case["expect"].get("err")) {
            (Ok(g), Some(e), _) => assert_eq!(Some(g.as_str()), e.as_str(), "{n}"),
            (Err(g), _, Some(e)) => assert_eq!(Some(g.as_str()), e.as_str(), "{n}"),
            _ => panic!("{n}: got {got:?}, expected {}", case["expect"]),
        }
    }
}

#[test]
fn client_resolution_order() {
    let none = Value::Null;
    let have = |names: &'static [&'static str]| move |n: &str| names.contains(&n);
    assert_eq!(resolve_vnc_client(&none, &have(&[])), "vncviewer");
    assert_eq!(
        resolve_vnc_client(&none, &have(&["gvncviewer"])),
        "gvncviewer"
    );
    assert_eq!(
        resolve_vnc_client(&none, &have(&["gvncviewer", "xtigervncviewer"])),
        "xtigervncviewer"
    );
}
