//! `termigator_core::proto::nettools` against the golden corpus
//! (compat/golden/tools_nettools.json) and the logic tests.

mod common;

use serde_json::Value as J;
use termigator_core::config::doc::Value;
use termigator_core::proto::nettools::{TOOLS, tool_argv, tool_for_entry};

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

fn tools_of(case: &J) -> Vec<String> {
    case["which"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|s| s.as_str().map(str::to_owned))
        .collect()
}

fn strings(v: &J) -> Vec<String> {
    v.as_array()
        .expect("list")
        .iter()
        .map(|s| s.as_str().expect("str").to_owned())
        .collect()
}

fn int(v: &J) -> i64 {
    v["$int"].as_str().expect("$int").parse().expect("i64")
}

#[test]
fn golden_corpus() {
    let cases = common::golden("tools_nettools");
    assert!(cases.len() > 1500);
    for c in &cases {
        let n = common::name(c);
        let tools = tools_of(c);
        let which = |name: &str| tools.iter().any(|t| t == name);
        match c["fn"].as_str().expect("fn") {
            "tool_argv" => {
                let got = tool_argv(
                    c["tool"].as_str().expect("tool"),
                    c["host"].as_str().expect("host"),
                    int(&c["port"]),
                    &which,
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
            "tool_for_entry" => {
                let got = tool_for_entry(
                    c["tool"].as_str().expect("tool"),
                    &from_golden(&c["conn"]),
                    &which,
                );
                if c.get("error").is_some() {
                    assert_eq!(
                        got,
                        Err(c["message"].as_str().expect("message").to_owned()),
                        "{n}"
                    );
                } else {
                    let want = strings(&c["expect"]);
                    assert_eq!(got, Ok((want[0].clone(), want[1].clone())), "{n}");
                }
            }
            "tools" => {
                let want: Vec<(String, String)> = c["expect"]
                    .as_array()
                    .expect("list")
                    .iter()
                    .map(|p| {
                        (
                            p[0].as_str().unwrap_or_default().to_owned(),
                            p[1].as_str().unwrap_or_default().to_owned(),
                        )
                    })
                    .collect();
                let got: Vec<(String, String)> = TOOLS
                    .iter()
                    .map(|(k, l)| ((*k).to_owned(), (*l).to_owned()))
                    .collect();
                assert_eq!(got, want, "{n}");
            }
            other => panic!("{n}: unknown fn {other}"),
        }
    }
}

fn only<'a>(names: &'a [&'a str]) -> impl Fn(&str) -> bool + 'a {
    move |n| names.contains(&n)
}

fn argv(tool: &str, host: &str, port: i64, names: &[&str]) -> Result<Vec<String>, String> {
    tool_argv(tool, host, port, &only(names))
}

/// test_best_installed_program_is_picked
#[test]
fn best_installed_program_is_picked() {
    assert_eq!(
        argv("trace", "h", 22, &["mtr", "traceroute"]).expect("mtr")[0],
        "mtr"
    );
    assert_eq!(
        argv("trace", "h", 22, &["tracepath"]),
        Ok(vec!["tracepath".into(), "h".into()])
    );
    assert!(argv("trace", "h", 22, &[]).is_err());
    assert_eq!(
        argv("dns", "h", 22, &["host"]),
        Ok(vec!["host".into(), "h".into()])
    );
    assert_eq!(
        argv("dns", "h", 22, &[]),
        Ok(vec!["getent".into(), "ahosts".into(), "h".into()])
    );
    assert_eq!(
        argv("port", "h", 2222, &["nc"]),
        Ok(["nc", "-zv", "-w", "5", "h", "2222"]
            .map(String::from)
            .to_vec())
    );
    assert_eq!(
        &argv("port", "h", 2222, &[]).expect("bash")[..2],
        ["bash", "-c"]
    );
    assert_eq!(
        argv("scan", "h", 22, &[]),
        Err("Install nmap to scan ports.".to_owned())
    );
    let keys: Vec<&str> = TOOLS.iter().map(|(k, _)| *k).collect();
    assert_eq!(keys, ["ping", "trace", "dns", "port", "scan"]);
}

/// test_hosts_that_are_options_or_code_are_refused
#[test]
fn hosts_that_are_options_or_code_are_refused() {
    for bad in ["-oProxyCommand=x", "a b", "h;rm -rf ~", "$(id)", ""] {
        assert!(argv("ping", bad, 22, &["ping"]).is_err(), "{bad:?}");
    }
}

/// test_entry_endpoint_and_bastion_note
#[test]
fn entry_endpoint_and_bastion_note() {
    let s = |t: &str| Value::Str(t.to_owned());
    let m = |p: &[(&str, Value)]| Value::Map(p.iter().map(|(k, v)| (s(k), v.clone())).collect());
    let (cmd, title) = tool_for_entry(
        "port",
        &m(&[
            ("label", s("r")),
            ("protocol", s("rdp")),
            ("host", s("win")),
        ]),
        &only(&["nc"]),
    )
    .expect("port");
    assert!(cmd.ends_with("win 3389") && title == "Check port: win:3389");
    let (cmd, _) = tool_for_entry(
        "ping",
        &m(&[("host", s("db")), ("proxy_jump", s("jump@bastion:2200"))]),
        &only(&["ping"]),
    )
    .expect("ping");
    assert!(
        cmd.starts_with("echo ")
            && cmd.ends_with("ping bastion")
            && cmd.contains("behind the bastion")
    );
    assert_eq!(
        tool_for_entry("ping", &m(&[("host", s(""))]), &only(&["ping"])),
        Err("Host is missing.".to_owned())
    );
}
