//! `termigator_core::proto::gateway` against the golden corpus (compat/golden/gateway_*.json)
//! and the gateway logic (live socat / pty tests are separate).
// Some cases are Unix-only (#[cfg(unix)]): their helpers and imports are unused on Windows.
#![cfg_attr(windows, allow(unused_imports, dead_code))]
#![allow(clippy::unwrap_used, clippy::expect_used)] // test helpers decode trusted golden JSON

mod common;

use serde_json::Value as J;
use termigator_core::config::doc::Value;
use termigator_core::proto::gateway::{build_via_gateway, find_gateway, gateway_tunnel_argv};
use termigator_core::proto::rdp::build_rdp_command;
use termigator_core::proto::ssh::Env;
use termigator_core::proto::vnc::build_vnc_command;

fn fix_dir() -> String {
    common::canon(common::repo_root().join("crates/termigator-core/tests/fixtures/ssh"))
        .to_string_lossy()
        .into_owned()
}

fn decode(v: &J, fix: &str) -> Value {
    if let Some((tag, p)) = common::tagged(v) {
        let s = || p.as_str().unwrap_or_default().to_owned();
        return match tag {
            "$int" => Value::Int(s().parse().expect("i64")),
            "$float" => Value::Float(s().parse().expect("f64")),
            "$map" => Value::Map(
                p.as_array()
                    .into_iter()
                    .flatten()
                    .map(|kv| (decode(&kv[0], fix), decode(&kv[1], fix)))
                    .collect(),
            ),
            other => panic!("unknown tag {other}"),
        };
    }
    match v {
        J::Null => Value::Null,
        J::Bool(b) => Value::Bool(*b),
        J::String(s) => Value::Str(s.replace("@FIX@", fix)),
        J::Array(a) => Value::Seq(a.iter().map(|x| decode(x, fix)).collect()),
        other => panic!("not a golden value: {other}"),
    }
}

fn env(tools: &[String], fix: &str) -> Env {
    let tools = tools.to_vec();
    Env {
        which: Box::new(move |n| {
            tools
                .iter()
                .any(|t| t == n)
                .then(|| format!("/usr/bin/{n}"))
        }),
        control_dir: Some("/run/user/1000/termigator".to_owned()),
        home: Some(fix.to_owned()),
    }
}

fn strings(v: &J) -> Vec<String> {
    v.as_array()
        .into_iter()
        .flatten()
        .filter_map(|s| s.as_str().map(str::to_owned))
        .collect()
}

fn check<T: std::fmt::Debug>(
    n: &str,
    got: &Result<T, String>,
    case: &J,
    fix: &str,
    to_json: impl Fn(&T) -> J,
) {
    match (got, case.get("expect"), case.get("message")) {
        (Ok(g), Some(e), _) => assert_eq!(&to_json(g), &jfix(e, fix), "{n}"),
        (Err(g), None, Some(m)) => assert_eq!(
            Some(g.as_str()),
            m.as_str().map(|s| s.replace("@FIX@", fix)).as_deref(),
            "{n}"
        ),
        _ => panic!("{n}: got {got:?}, expected {case}"),
    }
}

fn jfix(v: &J, fix: &str) -> J {
    match v {
        J::String(s) => J::String(s.replace("@FIX@", fix)),
        J::Array(a) => J::Array(a.iter().map(|x| jfix(x, fix)).collect()),
        other => other.clone(),
    }
}

#[test]
fn find_matches_golden() {
    let fix = fix_dir();
    let cases = common::golden("gateway_find");
    assert!(cases.len() > 60);
    for case in &cases {
        let n = common::name(case);
        let Value::Seq(conns) = decode(&case["conns"], &fix) else {
            panic!("{n}: conns");
        };
        let label = decode(&case["label"], &fix);
        let got = find_gateway(&conns, Some(&label))
            .map(|g| conns.iter().position(|c| std::ptr::eq(c, g)).unwrap());
        let want = common::tagged(&case["expect"])
            .map(|(_, p)| p.as_str().unwrap().parse::<usize>().unwrap());
        assert_eq!(got, want, "{n}");
    }
}

#[cfg(unix)] // golden argv uses sshpass; Windows uses askpass
#[test]
fn tunnel_matches_golden() {
    let fix = fix_dir();
    let cases = common::golden("gateway_tunnel");
    assert!(cases.len() > 25);
    for case in &cases {
        let n = common::name(case);
        let (_, port) = common::tagged(&case["port"]).unwrap();
        let (_, lport) = common::tagged(&case["local_port"]).unwrap();
        let got = gateway_tunnel_argv(
            case["host"].as_str().unwrap(),
            port.as_str().unwrap().parse().unwrap(),
            &decode(&case["gw"], &fix),
            lport.as_str().unwrap().parse().unwrap(),
            &decode(&case["launch"], &fix),
            &env(&strings(&case["which"]), &fix),
        );
        check(n, &got, case, &fix, |v| J::from(v.clone()));
    }
}

#[cfg(unix)] // golden argv uses sshpass; Windows uses askpass
#[test]
fn via_matches_golden() {
    let fix = fix_dir();
    let cases = common::golden("gateway_via");
    assert!(cases.len() > 40);
    for case in &cases {
        let n = common::name(case);
        let client_launch = decode(&case["client_launch"], &fix);
        let mut tools = strings(&case["which"]);
        tools.extend(strings(&case["client_which"]));
        let e = env(&tools, &fix);
        let client = case["client"].as_str().unwrap();
        let build = |c: &Value| match client {
            "vnc" => build_vnc_command(c, &client_launch, &|name| tools.iter().any(|t| t == name)),
            "rdp" => build_rdp_command(c, &client_launch, &e),
            _ => Ok(format!(
                "stub {} {}",
                termigator_core::config::model::value_str(c.get("host").unwrap_or(&Value::Null)),
                termigator_core::config::model::value_str(c.get("port").unwrap_or(&Value::Null)),
            )),
        };
        let got = build_via_gateway(
            &decode(&case["conn"], &fix),
            &decode(&case["gw"], &fix),
            &build,
            &decode(&case["launch"], &fix),
            u16::try_from(case["local_port"].as_u64().unwrap()).unwrap(),
            &e,
        );
        check(n, &got, case, &fix, |v| J::from(v.clone()));
    }
}

fn s(t: &str) -> Value {
    Value::Str(t.to_owned())
}

#[test]
fn script_shape() {
    let e = env(&[], "/h");
    let gw = Value::Map(vec![
        (s("label"), s("bastion")),
        (s("host"), s("bastion.example")),
        (s("user"), s("jump")),
    ]);
    let conn = Value::Map(vec![
        (s("protocol"), s("vnc")),
        (s("host"), s("desk.lan")),
        (s("port"), Value::Int(5901)),
    ]);
    let no_ka = Value::Map(vec![(s("ssh_keepalive"), Value::Bool(false))]);
    let script = build_via_gateway(
        &conn,
        &gw,
        &|c| build_vnc_command(c, &Value::Null, &|_| false),
        &no_ka,
        40001,
        &e,
    )
    .unwrap();
    assert!(script.contains("-L 127.0.0.1:40001:desk.lan:5901"));
    assert!(script.contains("BatchMode=yes") && script.contains("ExitOnForwardFailure=yes"));
    assert!(script.contains("trap 'kill $TERMIGATOR_GW 2>/dev/null' EXIT HUP TERM INT"));
    assert!(script.contains("/dev/tcp/127.0.0.1/40001"));
    assert!(script.trim_end().ends_with("127.0.0.1::40001"));
}
