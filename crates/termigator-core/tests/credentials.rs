//! Named credentials against the golden fixtures (compat/golden/credentials.json) and the
//! logic tests.
mod common;

use serde_json::Value as J;
use termigator_core::config::doc::Value;
use termigator_core::credentials::{
    apply_credential, find_credential, normalize_credentials, sync_references, usage_count,
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

fn seq(v: &Value) -> Vec<Value> {
    match v {
        Value::Seq(x) => x.clone(),
        _ => Vec::new(),
    }
}

fn s(t: &str) -> Value {
    Value::Str(t.to_owned())
}

fn map(pairs: &[(&str, &str)]) -> Value {
    Value::Map(pairs.iter().map(|(k, v)| (s(k), s(v))).collect())
}

fn renames(v: &Value) -> Vec<(String, String)> {
    match v {
        Value::Map(p) => p
            .iter()
            .map(|(k, v)| match (k, v) {
                (Value::Str(k), Value::Str(v)) => (k.clone(), v.clone()),
                other => panic!("not a rename: {other:?}"),
            })
            .collect(),
        other => panic!("not renames: {other:?}"),
    }
}

#[test]
fn golden() {
    let cases = common::golden("credentials");
    assert!(cases.len() >= 200);
    for c in &cases {
        let name = common::name(c);
        let a: Vec<Value> = c["input"]
            .as_array()
            .unwrap()
            .iter()
            .map(from_golden)
            .collect();
        let want = from_golden(&c["expect"]);
        let got = match c["fn"].as_str().unwrap() {
            "normalize" => Value::Seq(normalize_credentials(&a[0])),
            "find" => find_credential(&seq(&a[0]), &a[1])
                .cloned()
                .unwrap_or(Value::Null),
            "apply" => {
                let out = apply_credential(&a[0], &seq(&a[1]));
                assert_eq!(a[0], from_golden(&c["input"][0]), "{name}: input changed");
                out
            }
            "usage" => {
                let Value::Str(n) = &a[1] else {
                    panic!("{name}: name")
                };
                Value::Int(usage_count(&seq(&a[0]), n) as i64)
            }
            "sync" => {
                let mut conns = seq(&a[0]);
                let n = sync_references(&mut conns, &renames(&a[1]), &seq(&a[2]));
                let conns = if matches!(a[0], Value::Null) {
                    Value::Null
                } else {
                    Value::Seq(conns)
                };
                Value::Map(vec![
                    (s("changed"), Value::Int(n as i64)),
                    (s("conns"), conns),
                ])
            }
            other => panic!("{name}: unknown fn {other}"),
        };
        assert_eq!(got, want, "{name}");
    }
}

#[test]
fn normalize_unique_named() {
    let raw = Value::Seq(vec![
        map(&[("name", "ops"), ("user", "root"), ("password", "x")]),
        map(&[("name", "ops")]),
        map(&[("user", "nameless")]),
        s("junk"),
        map(&[("name", "win"), ("user", "adm"), ("domain", "CORP")]),
    ]);
    assert_eq!(
        normalize_credentials(&raw),
        vec![
            map(&[("name", "ops"), ("user", "root"), ("password", "x")]),
            map(&[("name", "win"), ("user", "adm"), ("domain", "CORP")]),
        ]
    );
}

#[test]
fn apply_lends_user_and_password_only_to_a_copy() {
    let creds = [map(&[
        ("name", "ops"),
        ("user", "root"),
        ("password", "s3cret"),
    ])];
    let conn = map(&[
        ("host", "h"),
        ("user", "old"),
        ("password", "own"),
        ("credential", "ops"),
    ]);
    let out = apply_credential(&conn, &creds);
    assert_eq!(out.get("user"), Some(&s("root")));
    assert_eq!(out.get("password"), Some(&s("s3cret")));
    assert_eq!(conn.get("password"), Some(&s("own")));
    let plain = map(&[("host", "h"), ("user", "u")]);
    assert_eq!(apply_credential(&plain, &creds), plain);
    let dangling = map(&[("host", "h"), ("user", "u"), ("credential", "gone")]);
    assert_eq!(apply_credential(&dangling, &creds), dangling);
}

#[test]
fn sync_rename_user_change_and_delete() {
    let mut conns = vec![
        map(&[("host", "a"), ("user", "old"), ("credential", "ops")]),
        map(&[
            ("host", "b"),
            ("user", "x"),
            ("credential", "win"),
            ("protocol", "rdp"),
            ("domain", "OLD"),
        ]),
        map(&[("host", "c"), ("user", "y"), ("credential", "dropped")]),
        map(&[("host", "d"), ("user", "z")]),
    ];
    let creds = [
        map(&[("name", "admins"), ("user", "root")]),
        map(&[("name", "win"), ("user", "adm"), ("domain", "CORP")]),
    ];
    let n = sync_references(&mut conns, &[("ops".into(), "admins".into())], &creds);
    assert_eq!(n, 3);
    assert_eq!(
        conns[0],
        map(&[("host", "a"), ("user", "root"), ("credential", "admins")])
    );
    assert_eq!(conns[1].get("user"), Some(&s("adm")));
    assert_eq!(conns[1].get("domain"), Some(&s("CORP")));
    assert_eq!(conns[2], map(&[("host", "c"), ("user", "y")]));
    assert_eq!(conns[3], map(&[("host", "d"), ("user", "z")]));
}
