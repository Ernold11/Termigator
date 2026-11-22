//! YAML module against the PyYAML/ruamel golden fixtures (compat/golden/yaml_*.json).
mod common;

use std::str::FromStr;

use serde_json::Value as J;
use termigator_core::config::doc::{
    Value, canonical_timestamp, dump, load, update_preserving_comments,
};

/// Inputs PyYAML's scanner rejects but libyaml accepts (tab as a separator).
const LENIENT: [&str; 3] = [
    "err_tab_after_colon",
    "err_tab_in_plain",
    "err_tab_trailing",
];

fn from_golden(v: &J) -> Value {
    if let Some((tag, p)) = common::tagged(v) {
        let s = || p.as_str().unwrap_or_default().to_owned();
        return match tag {
            "$int" => Value::Int(s().parse().expect("i64")),
            "$float" => Value::Float(s().parse().expect("f64")),
            "$timestamp" => Value::Timestamp(s()),
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

/// Exact: same types, same key order, timestamps by their isoformat, NaN equals NaN.
fn same(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Float(x), Value::Float(y)) => {
            x == y && x.is_sign_negative() == y.is_sign_negative() || x.is_nan() && y.is_nan()
        }
        (Value::Timestamp(x), Value::Timestamp(y)) => {
            canonical_timestamp(x).as_deref() == Some(y.as_str()) || x == y
        }
        (Value::Seq(x), Value::Seq(y)) => {
            x.len() == y.len() && x.iter().zip(y).all(|(p, q)| same(p, q))
        }
        (Value::Map(x), Value::Map(y)) => {
            x.len() == y.len()
                && x.iter()
                    .zip(y)
                    .all(|((k1, v1), (k2, v2))| same(k1, k2) && same(v1, v2))
        }
        _ => a == b,
    }
}

fn comments(text: &str) -> Vec<String> {
    yaml_edit::YamlFile::from_str(text)
        .map(|f| f.comments().map(|c| c.text().trim().to_owned()).collect())
        .unwrap_or_default()
}

#[test]
fn golden_load() {
    let cases = common::golden("yaml_load");
    let mut failures = Vec::new();
    for case in &cases {
        let name = common::name(case);
        let got = load(&common::input_text(case));
        match (case.get("expect"), got) {
            (Some(expect), Ok(v)) if same(&v, &from_golden(expect)) => {}
            (None, Err(_)) => {}
            (None, Ok(_)) if LENIENT.contains(&name) => {}
            (expect, got) => failures.push(format!("{name}: expected {expect:?}, got {got:?}")),
        }
    }
    assert!(
        failures.is_empty(),
        "{} of {} load cases differ:\n{}",
        failures.len(),
        cases.len(),
        failures.join("\n")
    );
}

#[test]
fn golden_dump_is_byte_identical() {
    let cases = common::golden("yaml_dump");
    let mut failures = Vec::new();
    for case in &cases {
        let got = dump(&from_golden(&case["value"]));
        let expect = case["expect"].as_str().unwrap_or_default();
        if got != expect {
            failures.push(format!(
                "{}:\n--- expected\n{expect}--- got\n{got}",
                common::name(case)
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "{} of {} dumps differ:\n{}",
        failures.len(),
        cases.len(),
        failures.join("\n")
    );
}

#[test]
fn canonical_session_is_a_noop_save() {
    let text = std::fs::read_to_string(
        common::repo_root()
            .join("crates/termigator-core/tests/fixtures/yaml/session_canonical.yaml"),
    )
    .expect("fixture");
    let header = "# Termigator SSH connections — edited by the app or by hand.\n";
    assert_eq!(header.to_owned() + &dump(&load(&text).expect("load")), text);
}

#[test]
fn yaml11_type_table() {
    let s = |x: &str| Value::Str(x.into());
    let table = [
        ("yes", Value::Bool(true)),
        ("No", Value::Bool(false)),
        ("on", Value::Bool(true)),
        ("y", s("y")),
        ("0777", Value::Int(511)),
        ("0x1F", Value::Int(31)),
        ("1:30", Value::Int(90)),
        ("1_000", Value::Int(1000)),
        (".inf", Value::Float(f64::INFINITY)),
        ("1e3", s("1e3")),
        ("2026-01-02", Value::Timestamp("2026-01-02".into())),
        ("~", Value::Null),
        ("", Value::Null),
        ("'yes'", s("yes")),
        ("\"0777\"", s("0777")),
    ];
    for (text, expect) in table {
        assert_eq!(
            load(&format!("v: {text}\n")).expect(text).get("v"),
            Some(&expect),
            "{text:?}"
        );
    }
}

#[test]
fn aliases_and_merge_keys_resolve() {
    let v = load(
        "base: &b {host: h, port: 22}\nlist: &l [1, 2]\nuse: *l\nm:\n  <<: *b\n  port: 2222\n",
    )
    .expect("load");
    assert_eq!(v.get("use"), v.get("list"));
    let m = v.get("m").expect("m");
    assert_eq!(m.get("host"), Some(&Value::Str("h".into())));
    assert_eq!(m.get("port"), Some(&Value::Int(2222)));
    assert!(load("a: *missing\n").is_err());
}

/// Three nested edits, one delete, one add on a commented config.yaml.
#[test]
fn g0_criterion_10_comments_survive_edits() {
    let original = std::fs::read_to_string(
        common::repo_root()
            .join("crates/termigator-core/tests/fixtures/yaml/config_commented.yaml"),
    )
    .expect("fixture");
    let mut target = load(&original).expect("load");
    let Value::Map(root) = &mut target else {
        panic!("not a mapping")
    };
    let window = section(root, "window");
    set(window, "width", Value::Int(1440));
    window.retain(|(k, _)| *k != Value::Str("list_width".into()));
    set(
        section(window, "dialog_sizes"),
        "ConnectionDialog",
        Value::Seq(vec![Value::Int(800), Value::Int(600)]),
    );
    let launch = section(root, "launch");
    set(launch, "shell", Value::Str("fish".into()));
    set(launch, "new_flag", Value::Str("yes".into()));

    let out = update_preserving_comments(&original, &target);
    eprintln!("{out}");
    let before = comments(&original);
    let after = comments(&out);
    let lost: Vec<_> = before.iter().filter(|c| !after.contains(c)).collect();
    assert!(lost.is_empty(), "comments lost: {lost:?}\n{out}");
    let reloaded = load(&out).expect("reload");
    assert!(same(&reloaded, &target), "{out}");
    let get = |sec: &str, k: &str| reloaded.get(sec).and_then(|s| s.get(k)).cloned();
    assert_eq!(get("window", "always_on_top"), Some(Value::Bool(true))); // `yes` kept as written
    assert_eq!(get("launch", "ssh_keepalive_seconds"), Some(Value::Int(90))); // `1:30`
    assert_eq!(get("launch", "umask"), Some(Value::Int(511))); // `0777`
    assert_eq!(get("launch", "new_flag"), Some(Value::Str("yes".into())));
    assert!(out.contains("always_on_top: yes # YAML 1.1 bool"), "{out}");
    assert!(out.contains("new_flag: 'yes'"), "{out}");
}

fn section<'a>(m: &'a mut [(Value, Value)], name: &str) -> &'a mut Vec<(Value, Value)> {
    match m.iter_mut().find(|(k, _)| *k == Value::Str(name.into())) {
        Some((_, Value::Map(inner))) => inner,
        _ => panic!("no mapping {name}"),
    }
}

fn set(m: &mut Vec<(Value, Value)>, key: &str, v: Value) {
    match m.iter_mut().find(|(k, _)| *k == Value::Str(key.into())) {
        Some(slot) => slot.1 = v,
        None => m.push((Value::Str(key.into()), v)),
    }
}

#[test]
fn golden_sync() {
    let mut failures = Vec::new();
    for case in common::golden("yaml_sync") {
        let name = common::name(&case).to_owned();
        let original = common::input_text(&case);
        let target = from_golden(&case["target"]);
        let out = update_preserving_comments(&original, &target);
        match load(&out) {
            Ok(v) if v.loose_eq(&target) => {}
            other => failures.push(format!("{name}: reloads as {other:?}\n{out}")),
        }
        if let Some(dropped) = case["dropped_comments"].as_array() {
            let dropped: Vec<&str> = dropped.iter().filter_map(J::as_str).collect();
            let after = comments(&out);
            for c in comments(&original) {
                if !dropped.contains(&c.as_str()) && !after.contains(&c) {
                    failures.push(format!("{name}: lost comment {c:?}\n{out}"));
                }
            }
        }
        if name == "noop" && out != original {
            failures.push(format!("noop changed the file:\n{out}"));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn sync_falls_back_to_a_plain_dump_when_it_cannot_edit_faithfully() {
    // Changing an anchored value would also change what the alias reads.
    let original = "# c\nbase: &b 1\nother: *b\n";
    let target = Value::Map(vec![
        (Value::Str("base".into()), Value::Int(2)),
        (Value::Str("other".into()), Value::Int(1)),
    ]);
    let out = update_preserving_comments(original, &target);
    assert!(load(&out).expect("reload").loose_eq(&target), "{out}");
    // Not a mapping at all: plain dump.
    assert_eq!(update_preserving_comments("- a\n", &target), dump(&target));
}
