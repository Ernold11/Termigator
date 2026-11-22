//! `termigator_core::proto::rdp`: golden corpus (compat/golden/rdp_*.json) and the
//! logic cases.
#![allow(clippy::unwrap_used, clippy::expect_used)] // test helpers decode trusted golden JSON

mod common;

use serde_json::Value as J;
use termigator_core::config::doc::Value;
use termigator_core::proto::rdp::{
    RDP_CLIENT_CANDIDATES, build_rdp_command, clipboard_rdp_command, rdp_password_env,
    resolution_setting_is_valid, resolve_rdp_client, with_default_geometry,
};
use termigator_core::proto::ssh::Env;

pub fn decode(v: &J) -> Value {
    if let Some((tag, p)) = common::tagged(v) {
        let s = || p.as_str().unwrap_or_default().to_owned();
        return match tag {
            "$int" => Value::Int(s().parse().expect("i64")),
            "$float" => Value::Float(s().parse().expect("f64")),
            "$map" => Value::Map(
                p.as_array()
                    .into_iter()
                    .flatten()
                    .map(|kv| (decode(&kv[0]), decode(&kv[1])))
                    .collect(),
            ),
            other => panic!("unknown tag {other}"),
        };
    }
    match v {
        J::Null => Value::Null,
        J::Bool(b) => Value::Bool(*b),
        J::String(s) => Value::Str(s.clone()),
        J::Array(a) => Value::Seq(a.iter().map(decode).collect()),
        other => panic!("not a golden value: {other}"),
    }
}

/// `shutil.which` knows only `tools`; HOME is the golden one.
fn env(tools: &[&str]) -> Env {
    let tools: Vec<String> = tools.iter().map(|s| (*s).to_owned()).collect();
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

fn case_env(case: &J) -> Env {
    let tools: Vec<&str> = case["which"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(J::as_str)
        .collect();
    env(&tools)
}

#[test]
fn build_matches_golden() {
    let cases = common::golden("rdp_build");
    assert!(cases.len() > 450);
    for case in &cases {
        let n = common::name(case);
        let got = build_rdp_command(
            &decode(&case["conn"]),
            &decode(&case["launch"]),
            &case_env(case),
        );
        match (&got, case.get("expect"), case.get("message")) {
            (Ok(g), Some(e), _) => assert_eq!(Some(g.as_str()), e.as_str(), "{n}"),
            (Err(g), None, Some(m)) => assert_eq!(Some(g.as_str()), m.as_str(), "{n}"),
            _ => panic!("{n}: got {got:?}, expected {case}"),
        }
    }
}

#[test]
fn misc_matches_golden() {
    let cases = common::golden("rdp_misc");
    assert!(cases.len() > 60);
    for case in &cases {
        let n = common::name(case);
        match case["fn"].as_str().unwrap() {
            "resolve" => {
                let got = resolve_rdp_client(&decode(&case["launch"]), &case_env(case));
                assert_eq!(Some(got.as_str()), case["expect"].as_str(), "{n}");
            }
            "env" => {
                let got = rdp_password_env(&decode(&case["conn"]));
                match (&got, case.get("expect"), case.get("message")) {
                    (Ok(g), Some(e), _) => {
                        let want: Vec<(String, String)> = e
                            .as_array()
                            .unwrap()
                            .iter()
                            .map(|kv| {
                                (
                                    kv[0].as_str().unwrap().to_owned(),
                                    kv[1].as_str().unwrap().to_owned(),
                                )
                            })
                            .collect();
                        assert_eq!(exposed(g), want, "{n}");
                    }
                    (Err(g), None, Some(m)) => {
                        assert_eq!(Some(g.to_string().as_str()), m.as_str(), "{n}")
                    }
                    _ => panic!("{n}: got {got:?}, expected {case}"),
                }
            }
            "clipboard" => {
                let got =
                    clipboard_rdp_command(case["cmd"].as_str().unwrap(), &decode(&case["conn"]));
                assert_eq!(Some(got.as_str()), case["expect"].as_str(), "{n}");
            }
            other => panic!("{n}: unknown fn {other}"),
        }
    }
}

fn map(pairs: &[(&str, Value)]) -> Value {
    Value::Map(
        pairs
            .iter()
            .map(|(k, v)| (Value::Str((*k).to_owned()), v.clone()))
            .collect(),
    )
}

fn s(t: &str) -> Value {
    Value::Str(t.to_owned())
}

#[test]
fn password_goes_through_stdin_not_argv() {
    let launch = map(&[("rdp_client", s("xfreerdp"))]);
    let conn = map(&[
        ("host", s("h")),
        ("user", s("u")),
        ("password", s("s3cret")),
    ]);
    let line = build_rdp_command(&conn, &launch, &env(&[])).unwrap();
    assert!(!line.contains("s3cret") && !line.contains("/p:"));
    assert!(line.starts_with("printf '%s\\n' \"$TERMIGATOR_RDP_PW\" | xfreerdp "));
    assert!(line.ends_with("/from-stdin"));
    assert_eq!(
        exposed(&rdp_password_env(&conn).unwrap()),
        [("TERMIGATOR_RDP_PW".to_owned(), "s3cret".to_owned())]
    );
}

#[test]
fn sealed_token_is_refused() {
    let conn = map(&[("password", s("enc:v1:AAAA"))]);
    assert_eq!(
        rdp_password_env(&conn).unwrap_err().to_string(),
        "Password is still encrypted — unlock the vault first."
    );
}

#[test]
fn client_resolution_order() {
    let none = Value::Null;
    assert_eq!(
        resolve_rdp_client(&none, &env(&["xfreerdp", "xfreerdp3"])),
        "xfreerdp3"
    );
    assert_eq!(
        resolve_rdp_client(&none, &env(&["sdl-freerdp3"])),
        "sdl-freerdp3"
    );
    assert_eq!(resolve_rdp_client(&none, &env(&["xfreerdp"])), "xfreerdp");
    assert_eq!(
        resolve_rdp_client(&none, &env(&[])),
        RDP_CLIENT_CANDIDATES[0]
    );
    assert_eq!(
        resolve_rdp_client(&map(&[("rdp_client", s("my-rdp"))]), &env(&["xfreerdp3"])),
        "my-rdp"
    );
}

#[test]
fn injection_guards() {
    let launch = map(&[("rdp_client", s("xfreerdp"))]);
    let e = env(&[]);
    for (key, bad) in [
        ("host", "/bad"),
        ("user", "-x"),
        ("domain", "+d"),
        ("gateway", "/g"),
    ] {
        let conn = map(&[("host", s("h")), (key, s(bad))]);
        let conn = if key == "host" {
            map(&[("host", s(bad))])
        } else {
            conn
        };
        assert!(build_rdp_command(&conn, &launch, &e).is_err(), "{key}");
    }
    let sf = |p: &str| map(&[("host", s("h")), ("shared_folders", Value::Seq(vec![s(p)]))]);
    assert!(build_rdp_command(&sf("/a,b"), &launch, &e).is_err());
    assert!(build_rdp_command(&sf("rel/dir"), &launch, &e).is_err());
    assert_eq!(
        build_rdp_command(&sf("/tmp"), &launch, &e).unwrap(),
        "xfreerdp /v:h /drive:tmp,/tmp"
    );
}

#[test]
fn the_start_size_is_the_screen_unless_the_host_or_settings_say_otherwise() {
    let host = map(&[("host", s("h"))]);
    let get = |c: &Value, k: &str| c.get(k).cloned();
    // Default: the screen the window is on.
    let got = with_default_geometry(&host, &Value::Null, Some("2560x1440"));
    assert_eq!(get(&got, "resolution"), Some(s("2560x1440")));
    // Without a known screen nothing is set: the client chooses.
    assert_eq!(with_default_geometry(&host, &Value::Null, None), host);
    // The program-wide setting: full screen, a fixed size, or "" for the client's default.
    let full = map(&[("rdp_resolution", s("fullscreen"))]);
    assert_eq!(
        get(
            &with_default_geometry(&host, &full, Some("2560x1440")),
            "fullscreen"
        ),
        Some(Value::Bool(true))
    );
    let fixed = map(&[("rdp_resolution", s(" 1600X900 "))]);
    assert_eq!(
        get(
            &with_default_geometry(&host, &fixed, Some("2560x1440")),
            "resolution"
        ),
        Some(s("1600x900"))
    );
    let none = map(&[("rdp_resolution", s(""))]);
    assert_eq!(with_default_geometry(&host, &none, Some("2560x1440")), host);
    // A host's own choice always wins.
    let own = map(&[("host", s("h")), ("resolution", s("1280x720"))]);
    assert_eq!(
        with_default_geometry(&own, &Value::Null, Some("2560x1440")),
        own
    );
    let fs = map(&[("host", s("h")), ("fullscreen", Value::Bool(true))]);
    assert_eq!(with_default_geometry(&fs, &fixed, Some("2560x1440")), fs);
    // A bad setting does not break the connection.
    let bad = map(&[("rdp_resolution", s("huge"))]);
    assert_eq!(with_default_geometry(&host, &bad, Some("2560x1440")), host);
    assert!(resolution_setting_is_valid("1920x1080") && resolution_setting_is_valid("screen"));
    assert!(!resolution_setting_is_valid("huge") && !resolution_setting_is_valid("12x12"));
    // And the command line carries the result.
    let launch = map(&[("rdp_client", s("xfreerdp"))]);
    let got = with_default_geometry(&host, &Value::Null, Some("1920x1080"));
    let line = build_rdp_command(&got, &launch, &env(&["xfreerdp"])).expect("command");
    assert!(line.contains("/size:1920x1080"), "{line}");
}

/// The env pairs with their secret values shown, for exact comparison.
fn exposed(env: &[(String, termigator_core::vault::Secret)]) -> Vec<(String, String)> {
    env.iter()
        .map(|(k, v)| {
            (
                k.clone(),
                String::from_utf8(v.expose().to_vec()).expect("utf-8"),
            )
        })
        .collect()
}
