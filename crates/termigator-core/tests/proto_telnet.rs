//! Golden tests for `termigator_core::proto::telnet` (compat/golden/protocols_telnet.json).
#![allow(clippy::unwrap_used)] // test helpers decode trusted golden JSON

mod common;

use serde_json::Value as J;
use termigator_core::config::doc::Value;
use termigator_core::proto::telnet::{build_telnet_command, telnet_client_argv};

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

#[test]
fn build_matches_golden() {
    let cases = common::golden("protocols_telnet");
    assert!(cases.len() > 100);
    for case in &cases {
        let n = common::name(case);
        let have: Vec<&str> = case["which"]
            .as_array()
            .unwrap()
            .iter()
            .map(|s| s.as_str().unwrap())
            .collect();
        let which = |name: &str| have.contains(&name);
        let got = build_telnet_command(&decode(&case["conn"]), &which);
        match (&got, case["expect"].get("ok"), case["expect"].get("err")) {
            (Ok(g), Some(e), _) => assert_eq!(Some(g.as_str()), e.as_str(), "{n}"),
            (Err(g), _, Some(e)) => assert_eq!(Some(g.as_str()), e.as_str(), "{n}"),
            _ => panic!("{n}: got {got:?}, expected {}", case["expect"]),
        }
    }
}

#[test]
fn client_argv_order() {
    let have = |names: &'static [&'static str]| move |n: &str| names.contains(&n);
    assert_eq!(telnet_client_argv(&have(&[])), None);
    assert_eq!(
        telnet_client_argv(&have(&["busybox"])),
        Some(vec!["busybox".to_owned(), "telnet".to_owned()])
    );
    assert_eq!(
        telnet_client_argv(&have(&["busybox", "telnet"])),
        Some(vec!["telnet".to_owned()])
    );
}
