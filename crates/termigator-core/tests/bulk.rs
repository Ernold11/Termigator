//! Bulk edit (compat/golden/bulk*.json) and the logic tests.
mod common;

use serde_json::Value as J;
use termigator_core::bulk::{ICON_CHOICES, apply_bulk, is_icon_name};
use termigator_core::config::doc::Value;

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

fn s(t: &str) -> Value {
    Value::Str(t.to_owned())
}

fn map(pairs: Vec<(&str, Value)>) -> Value {
    Value::Map(pairs.into_iter().map(|(k, v)| (s(k), v)).collect())
}

fn strs(items: &[&str]) -> Value {
    Value::Seq(items.iter().map(|t| s(t)).collect())
}

fn seq(v: &Value) -> Vec<Value> {
    match v {
        Value::Seq(x) => x.clone(),
        _ => Vec::new(),
    }
}

#[test]
fn golden_bulk() {
    let cases = common::golden("bulk");
    assert!(cases.len() >= 250);
    for c in &cases {
        let name = common::name(c);
        let a: Vec<Value> = c["input"]
            .as_array()
            .unwrap()
            .iter()
            .map(from_golden)
            .collect();
        let idx: Vec<i64> = seq(&a[1])
            .iter()
            .map(|v| match v {
                Value::Int(i) => *i,
                other => panic!("{name}: index {other:?}"),
            })
            .collect();
        let mut conns = seq(&a[0]);
        let (changed, na) = apply_bulk(&mut conns, &idx, &a[2], &seq(&a[3]));
        let got = map(vec![
            ("changed", Value::Int(changed as i64)),
            ("na", Value::Int(na as i64)),
            ("conns", Value::Seq(conns)),
        ]);
        assert_eq!(got, from_golden(&c["expect"]), "{name}");
    }
}

#[test]
fn golden_icons() {
    let cases = common::golden("bulk_icons");
    let want = from_golden(&cases[0]["expect"]);
    let got = Value::Seq(ICON_CHOICES.iter().map(|(l, n)| strs(&[l, n])).collect());
    assert_eq!(got, want);
}

fn entries() -> Vec<Value> {
    vec![
        map(vec![
            ("label", s("a")),
            ("host", s("a")),
            ("user", s("u")),
            ("tags", strs(&["prod"])),
            ("proxy_jump", s("old")),
            ("password", s("own")),
        ]),
        map(vec![
            ("label", s("b")),
            ("protocol", s("rdp")),
            ("host", s("b")),
            ("user", s("x")),
        ]),
        map(vec![
            ("label", s("c")),
            ("protocol", s("vnc")),
            ("host", s("c")),
            ("group", s("lab")),
        ]),
    ]
}

#[test]
fn only_ticked_fields_change() {
    let mut conns = entries();
    let ch = map(vec![
        ("group", s("prod/web ")),
        ("tags_add", strs(&["db", "prod"])),
    ]);
    assert_eq!(apply_bulk(&mut conns, &[0, 1, 2], &ch, &[]), (3, 0));
    assert!(conns.iter().all(|c| c.get("group") == Some(&s("prod/web"))));
    assert_eq!(conns[0].get("tags"), Some(&strs(&["prod", "db"])));
    assert_eq!(conns[1].get("tags"), Some(&strs(&["db", "prod"])));
    assert_eq!(conns[0].get("proxy_jump"), Some(&s("old")));
}

#[test]
fn tags_remove_and_set() {
    let mut conns = entries();
    apply_bulk(
        &mut conns,
        &[0],
        &map(vec![("tags_remove", strs(&["prod"]))]),
        &[],
    );
    assert!(conns[0].get("tags").is_none());
    apply_bulk(
        &mut conns,
        &[1, 2],
        &map(vec![("tags_set", strs(&["x"]))]),
        &[],
    );
    assert_eq!(conns[1].get("tags"), Some(&strs(&["x"])));
    assert_eq!(conns[2].get("tags"), Some(&strs(&["x"])));
}

#[test]
fn protocol_specific_fields_are_not_forced() {
    let mut conns = entries();
    let creds = [map(vec![
        ("name", s("ops")),
        ("user", s("root")),
        ("domain", s("CORP")),
    ])];
    let ch = map(vec![("proxy_jump", s("bastion")), ("credential", s("ops"))]);
    let (_, na) = apply_bulk(&mut conns, &[0, 1, 2], &ch, &creds);
    assert_eq!(conns[0].get("proxy_jump"), Some(&s("bastion")));
    assert_eq!(conns[0].get("credential"), Some(&s("ops")));
    assert_eq!(conns[0].get("user"), Some(&s("root")));
    assert!(conns[0].get("password").is_none());
    assert_eq!(conns[1].get("credential"), Some(&s("ops")));
    assert_eq!(conns[1].get("domain"), Some(&s("CORP")));
    assert!(conns[1].get("proxy_jump").is_none());
    assert!(conns[2].get("credential").is_none() && conns[2].get("proxy_jump").is_none());
    assert_eq!(na, 2);
    let clear = map(vec![("proxy_jump", s("")), ("credential", s(""))]);
    apply_bulk(&mut conns, &[0], &clear, &[]);
    assert!(conns[0].get("proxy_jump").is_none() && conns[0].get("credential").is_none());
}

#[test]
fn icons() {
    let mut conns = entries();
    apply_bulk(
        &mut conns,
        &[0, 2],
        &map(vec![("icon", s("server-database"))]),
        &[],
    );
    assert_eq!(conns[0].get("icon"), Some(&s("server-database")));
    assert_eq!(conns[2].get("icon"), Some(&s("server-database")));
    assert!(is_icon_name("server-database") && !is_icon_name("nope"));
    apply_bulk(&mut conns, &[0], &map(vec![("icon", s(""))]), &[]);
    assert!(conns[0].get("icon").is_none());
}

#[test]
fn environment_is_set_and_cleared() {
    let mut conns = entries();
    let set = map(vec![("environment", s("production"))]);
    apply_bulk(&mut conns, &[0, 1], &set, &[]);
    assert_eq!(conns[0].get("environment"), Some(&s("production")));
    assert_eq!(conns[1].get("environment"), Some(&s("production")));
    assert_eq!(
        termigator_core::hosttree::environment_of(&conns[0]),
        "production"
    );
    apply_bulk(&mut conns, &[0], &map(vec![("environment", s(""))]), &[]);
    assert_eq!(conns[0].get("environment"), None);
}
