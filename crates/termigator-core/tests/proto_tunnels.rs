//! `termigator_core::proto::tunnels` against the golden corpus (compat/golden/tools_tunnels.json)
//! and the tunnel logic (the live socat manager is not covered here).
// Some cases are Unix-only (#[cfg(unix)]): their helpers and imports are unused on Windows.
#![cfg_attr(windows, allow(unused_imports, dead_code))]

mod common;

use serde_json::Value as J;
use termigator_core::config::doc::Value;
use termigator_core::proto::ssh::Env;
use termigator_core::proto::tunnels::{
    forwards_summary, normalize_tunnels, parse_forwards, tunnel_argv,
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

fn env_for(case: &J) -> Env {
    let tools: Vec<String> = case["which"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|s| s.as_str().map(str::to_owned))
        .collect();
    Env {
        which: Box::new(move |n| {
            tools
                .iter()
                .any(|t| t == n)
                .then(|| format!("/usr/bin/{n}"))
        }),
        control_dir: None,
        home: Some("/home/golden".to_owned()),
    }
}

fn strings(v: &J) -> Vec<String> {
    v.as_array()
        .expect("list")
        .iter()
        .map(|s| s.as_str().expect("str").to_owned())
        .collect()
}

#[cfg(unix)] // golden argv uses sshpass; Windows uses askpass
#[test]
fn golden_corpus() {
    let cases = common::golden("tools_tunnels");
    assert!(cases.len() > 100);
    for c in &cases {
        let n = common::name(c);
        match c["fn"].as_str().expect("fn") {
            "parse_forwards" => {
                let got = parse_forwards(Some(&from_golden(&c["input"])));
                if c.get("error").is_some() {
                    assert_eq!(
                        got,
                        Err(c["message"].as_str().expect("message").to_owned()),
                        "{n}"
                    );
                } else {
                    let want: Vec<(char, String)> = c["expect"]
                        .as_array()
                        .expect("list")
                        .iter()
                        .map(|p| {
                            (
                                p[0].as_str().and_then(|s| s.chars().next()).expect("kind"),
                                p[1].as_str().expect("spec").to_owned(),
                            )
                        })
                        .collect();
                    assert_eq!(got, Ok(want), "{n}");
                }
            }
            "normalize" => {
                let got: Vec<J> = normalize_tunnels(&from_golden(&c["input"]))
                    .into_iter()
                    .map(|t| {
                        serde_json::json!({
                            "name": t.name, "via": t.via, "forwards": t.forwards, "autostart": t.autostart
                        })
                    })
                    .collect();
                assert_eq!(J::Array(got), c["expect"], "{n}");
            }
            "tunnel_argv" => {
                let via = from_golden(&c["via"]);
                let got = tunnel_argv(
                    &from_golden(&c["tunnel"]),
                    (via != Value::Null).then_some(&via),
                    &from_golden(&c["launch"]),
                    &env_for(c),
                );
                if c.get("error").is_some() {
                    assert_eq!(
                        got,
                        Err(c["message"].as_str().expect("message").to_owned()),
                        "{n}"
                    );
                } else {
                    assert_eq!(got, Ok(strings(&c["expect"])), "{n}");
                }
            }
            "summary" => assert_eq!(
                forwards_summary(&from_golden(&c["tunnel"])),
                c["expect"].as_str().expect("expect"),
                "{n}"
            ),
            other => panic!("{n}: unknown fn {other}"),
        }
    }
}

fn s(t: &str) -> Value {
    Value::Str(t.to_owned())
}

fn map(pairs: &[(&str, Value)]) -> Value {
    Value::Map(pairs.iter().map(|(k, v)| (s(k), v.clone())).collect())
}

fn seq(items: &[&str]) -> Value {
    Value::Seq(items.iter().map(|i| s(i)).collect())
}

/// test_parse_forwards
#[test]
fn forwards_parse_and_reject() {
    let got = parse_forwards(Some(&s(
        "L 15432:db:5432\n# comment\nr 9000:localhost:22\nD 1080\n",
    )));
    assert_eq!(
        got,
        Ok(vec![
            ('L', "15432:db:5432".to_owned()),
            ('R', "9000:localhost:22".to_owned()),
            ('D', "127.0.0.1:1080".to_owned()),
        ])
    );
    assert!(parse_forwards(Some(&seq(&["X 1"]))).is_err());
    assert!(parse_forwards(Some(&Value::Seq(vec![]))).is_err());
    assert!(parse_forwards(Some(&seq(&["L -oProxyCommand=x"]))).is_err());
    assert!(parse_forwards(None).is_err());
}

/// test_tunnel_argv
#[test]
fn argv_for_a_bastion() {
    let via = map(&[
        ("label", s("bastion")),
        ("host", s("bastion.example")),
        ("user", s("jump")),
        ("remote_command", s("tmux")),
        ("local_forwards", seq(&["1:x:1"])),
    ]);
    let t = map(&[
        ("name", s("db")),
        ("via", s("bastion")),
        ("forwards", seq(&["L 15432:db:5432", "D 1080"])),
    ]);
    let launch = map(&[("ssh_keepalive", Value::Bool(false))]);
    let env = Env {
        which: Box::new(|_| None),
        control_dir: None,
        home: None,
    };
    let argv = tunnel_argv(&t, Some(&via), &launch, &env).expect("argv");
    assert_eq!(argv.first().map(String::as_str), Some("ssh"));
    assert_eq!(
        argv.last().map(String::as_str),
        Some("jump@bastion.example")
    );
    for part in ["-N", "ExitOnForwardFailure=yes", "BatchMode=yes"] {
        assert!(argv.iter().any(|a| a == part), "{part}");
    }
    let at = |flag: &str| {
        argv.iter()
            .position(|a| a == flag)
            .map(|i| argv[i + 1].as_str())
    };
    assert_eq!(at("-L"), Some("15432:db:5432"));
    assert_eq!(argv.iter().filter(|a| *a == "-L").count(), 1); // the entry's own -L dropped
    assert_eq!(at("-D"), Some("127.0.0.1:1080"));
    assert!(!argv.iter().any(|a| a == "tmux"));
    assert!(tunnel_argv(&t, None, &launch, &env).is_err());
}

/// test_normalize
#[test]
fn normalize_drops_unnamed_and_repeated() {
    let raw = Value::Seq(vec![
        map(&[
            ("name", s("a")),
            ("via", s("b")),
            ("forwards", s("L 1:h:2\nD 3")),
            ("autostart", Value::Bool(true)),
        ]),
        map(&[("name", s("a"))]),
        map(&[("via", s("x"))]),
    ]);
    let got = normalize_tunnels(&raw);
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].forwards, ["L 1:h:2", "D 3"]);
    assert!(got[0].autostart && got[0].via == "b");
}
