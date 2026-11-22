//! Config model accessors, defaults and folder paths against the golden
//! fixtures (compat/golden/model_*.json), plus group-path and accessor tests.
mod common;

use serde_json::Value as J;
use termigator_core::config::doc::{Value, load as yaml_load};
use termigator_core::config::groups::{
    group_path_ancestors, map_group_on_dissolve, map_group_on_rename, normalize_group_path,
    normalize_group_value,
};
use termigator_core::config::model::{
    connection_protocol, default_config, default_terminal_from_cfg, launch_embedded_int,
    normalize_terminal_mode, proxy_jump, proxy_jump_value, terminal_mode_for_entry, value_int,
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

fn args(case: &J) -> Vec<Value> {
    case["input"]
        .as_array()
        .into_iter()
        .flatten()
        .map(from_golden)
        .collect()
}

fn text(v: &Value) -> &str {
    match v {
        Value::Str(t) => t,
        other => panic!("not text: {other:?}"),
    }
}

fn s(t: &str) -> Value {
    Value::Str(t.to_owned())
}

fn cfg(pairs: Vec<(&str, Value)>) -> Value {
    Value::Map(pairs.into_iter().map(|(k, v)| (s(k), v)).collect())
}

#[test]
fn golden_defaults() {
    let cases = common::golden("model_defaults");
    assert_eq!(cases.len(), 1);
    assert_eq!(default_config(), from_golden(&cases[0]["expect"]));
}

#[test]
fn golden_protocol() {
    let cases = common::golden("model_protocol");
    assert!(cases.len() >= 40);
    for c in &cases {
        let got = connection_protocol(&args(c)[0]);
        assert_eq!(s(got), from_golden(&c["expect"]), "{}", common::name(c));
    }
}

#[test]
fn golden_terminal() {
    let cases = common::golden("model_terminal");
    assert!(cases.len() >= 60);
    for c in &cases {
        let a = args(c);
        let got = match c["fn"].as_str() {
            Some("normalize") => normalize_terminal_mode(Some(&a[0])),
            Some("normalize_absent") => normalize_terminal_mode(None),
            Some("default") => default_terminal_from_cfg(&a[0]),
            Some("entry") => terminal_mode_for_entry(Some(&a[0]), &a[1]),
            other => panic!("{}: fn {other:?}", common::name(c)),
        };
        // Windows has no external terminal: the effective mode is always embedded.
        let want = if cfg!(windows) && matches!(c["fn"].as_str(), Some("default" | "entry")) {
            s("embedded")
        } else {
            from_golden(&c["expect"])
        };
        assert_eq!(s(got), want, "{}", common::name(c));
    }
}

#[test]
fn golden_launch_embedded_int() {
    let cases = common::golden("model_launch_int");
    assert!(cases.len() >= 60);
    for c in &cases {
        let a = args(c);
        let keys: Vec<&str> = match &a[1] {
            Value::Seq(k) => k.iter().map(text).collect(),
            other => panic!("keys: {other:?}"),
        };
        let Value::Int(default) = a[2] else {
            panic!("default");
        };
        let got = launch_embedded_int(&a[0], &keys, default);
        assert_eq!(
            Value::Int(got),
            from_golden(&c["expect"]),
            "{}",
            common::name(c)
        );
    }
}

#[test]
fn golden_proxy_aliases() {
    let cases = common::golden("model_proxy");
    assert!(cases.len() >= 19);
    for c in &cases {
        let conn = &args(c)[0];
        let expect = from_golden(&c["expect"]);
        let want_value = expect.get("value").expect("value");
        let got_value = proxy_jump_value(conn).unwrap_or(&Value::Null);
        assert_eq!(got_value, want_value, "{}", common::name(c));
        assert_eq!(
            proxy_jump(conn),
            text(expect.get("text").expect("text")),
            "{}",
            common::name(c)
        );
    }
}

#[test]
fn golden_group_paths() {
    let cases = common::golden("model_groups");
    assert!(cases.len() >= 800);
    for c in &cases {
        let a = args(c);
        let t = |i: usize| match &a[i] {
            Value::Null => String::new(),
            v => text(v).to_owned(),
        };
        let want = from_golden(&c["expect"]);
        let got = match c["fn"].as_str() {
            Some("normalize") => s(&normalize_group_value(&a[0])),
            Some("ancestors") => Value::Seq(
                group_path_ancestors(&normalize_group_value(&a[0]))
                    .iter()
                    .map(|p| s(p))
                    .collect(),
            ),
            Some("rename") => s(&map_group_on_rename(&t(0), &t(1), &t(2))),
            Some("dissolve") => s(&map_group_on_dissolve(&t(0), &t(1))),
            other => panic!("{}: fn {other:?}", common::name(c)),
        };
        assert_eq!(got, want, "{}", common::name(c));
    }
}

// --- group paths ---

#[test]
fn normalize() {
    assert_eq!(normalize_group_path("prod"), "prod");
    assert_eq!(normalize_group_path("prod/web"), "prod/web");
    assert_eq!(normalize_group_path(r"prod\web\eu"), "prod/web/eu");
    assert_eq!(normalize_group_path("  prod //  web / "), "prod/web");
    for empty in ["", "   ", "///"] {
        assert_eq!(normalize_group_path(empty), "");
    }
    assert_eq!(normalize_group_value(&Value::Null), "");
    assert_eq!(normalize_group_value(&Value::Int(42)), "42");
}

#[test]
fn ancestors() {
    assert_eq!(group_path_ancestors("a/b/c"), ["a", "a/b", "a/b/c"]);
    assert_eq!(group_path_ancestors("a"), ["a"]);
    assert!(group_path_ancestors("").is_empty());
}

#[test]
fn rename() {
    assert_eq!(map_group_on_rename("old", "old", "new"), "new");
    assert_eq!(
        map_group_on_rename("old/web/eu", "old", "new"),
        "new/web/eu"
    );
    assert_eq!(
        map_group_on_rename("web/eu", "web", "prod/web"),
        "prod/web/eu"
    );
    assert_eq!(map_group_on_rename("older", "old", "new"), "older");
    assert_eq!(map_group_on_rename("x/old", "old", "new"), "x/old");
    assert_eq!(map_group_on_rename("", "old", "new"), "");
}

#[test]
fn dissolve() {
    assert_eq!(map_group_on_dissolve("a/b", "a/b"), "a");
    assert_eq!(map_group_on_dissolve("a", "a"), "");
    assert_eq!(map_group_on_dissolve("a/b/c", "a/b"), "a/c");
    assert_eq!(map_group_on_dissolve("b/c/d", "b"), "c/d");
    assert_eq!(map_group_on_dissolve("ab", "a"), "ab");
    assert_eq!(map_group_on_dissolve("x/a", "a"), "x/a");
}

// --- TerminalModes, LaunchEmbeddedInt, default_config shape ---

#[test]
fn terminal_modes() {
    for v in ["embedded", "embed", "pty", "internal", "in", "", "weird"] {
        assert_eq!(normalize_terminal_mode(Some(&s(v))), "embedded");
    }
    assert_eq!(normalize_terminal_mode(None), "embedded");
    for v in ["external", "konsole", "window", "out", "EXTERNAL"] {
        assert_eq!(normalize_terminal_mode(Some(&s(v))), "external");
    }
    let ext = cfg(vec![(
        "window",
        cfg(vec![("default_terminal", s("external"))]),
    )]);
    // No external terminal on Windows.
    let external = if cfg!(windows) {
        "embedded"
    } else {
        "external"
    };
    assert_eq!(default_terminal_from_cfg(&ext), external);
    assert_eq!(default_terminal_from_cfg(&cfg(vec![])), "embedded");
    let own = cfg(vec![("terminal", s("embedded"))]);
    assert_eq!(terminal_mode_for_entry(Some(&own), &ext), "embedded");
    assert_eq!(terminal_mode_for_entry(Some(&cfg(vec![])), &ext), external);
    assert_eq!(terminal_mode_for_entry(None, &ext), external);
}

#[test]
fn launch_embedded_int_aliases() {
    let keys = ["embedded_pyte_history", "pyte_history"];
    let lo = |k: &str, v: Value| cfg(vec![(k, v)]);
    assert_eq!(
        launch_embedded_int(&lo(keys[0], Value::Int(5)), &keys, 1),
        5
    );
    assert_eq!(
        launch_embedded_int(&lo(keys[1], Value::Int(9)), &keys, 1),
        9
    );
    assert_eq!(launch_embedded_int(&cfg(vec![]), &keys, 42), 42);
    assert_eq!(
        launch_embedded_int(&lo(keys[0], s("bad")), &keys[..1], 7),
        7
    );
    assert_eq!(value_int(&s(" 1_0 ")), Some(10));
    assert_eq!(value_int(&Value::Null), None);
}

#[test]
fn default_config_shape() {
    let d = default_config();
    assert_eq!(d.get("connections"), Some(&Value::Seq(vec![])));
    assert_eq!(d.get("favorites"), Some(&Value::Seq(vec![])));
    let window = d.get("window").expect("window");
    assert_eq!(window.get("default_terminal"), Some(&s("embedded")));
    assert_eq!(window.get("width"), Some(&Value::Int(900)));
    let launch = d.get("launch").expect("launch");
    assert_eq!(
        launch.get("embedded_pyte_history"),
        Some(&Value::Int(10_000))
    );
    assert_eq!(launch.get("shell_executable"), Some(&Value::Null));
}

#[test]
fn proxy_jump_prefers_proxy_jump_over_legacy_aliases() {
    let conn = yaml_load("jumphost: b\nproxy_jump: ' a '\njump_host: c\n").expect("yaml");
    assert_eq!(proxy_jump(&conn), "a");
    assert_eq!(proxy_jump(&yaml_load("jump_host: c\n").expect("yaml")), "c");
    assert_eq!(proxy_jump(&yaml_load("{}\n").expect("yaml")), "");
}
