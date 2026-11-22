//! `termigator_core::proto::scp`: golden corpus
//! (compat/golden/scp_*.json) and inline scp/sftp cases.
// Some cases are Unix-only (#[cfg(unix)]): their helpers and imports are unused on Windows.
#![cfg_attr(windows, allow(unused_imports, dead_code))]

mod common;

use serde_json::Value as J;
use termigator_core::config::doc::Value;
use termigator_core::proto::scp::{
    ScpTransfer, SftpResume, build_scp_command_with, scp_argv_with, scp_glob_escape, sftp_quote,
    sftp_resume_argv_with,
};
use termigator_core::proto::ssh::Env;

fn fix_dir() -> String {
    common::canon(common::repo_root().join("crates/termigator-core/tests/fixtures/ssh"))
        .to_string_lossy()
        .into_owned()
}

fn from_golden(v: &J, fix: &str) -> Value {
    if let Some((tag, p)) = common::tagged(v) {
        let s = || p.as_str().unwrap_or_default().to_owned();
        return match tag {
            "$int" => Value::Int(s().parse().expect("i64")),
            "$float" => Value::Float(s().parse().expect("f64")),
            "$map" => Value::Map(
                p.as_array()
                    .into_iter()
                    .flatten()
                    .map(|kv| (from_golden(&kv[0], fix), from_golden(&kv[1], fix)))
                    .collect(),
            ),
            other => panic!("unknown tag {other}"),
        };
    }
    match v {
        J::Null => Value::Null,
        J::Bool(b) => Value::Bool(*b),
        J::String(s) => Value::Str(s.replace("@FIX@", fix)),
        J::Array(a) => Value::Seq(a.iter().map(|x| from_golden(x, fix)).collect()),
        other => panic!("not a golden value: {other}"),
    }
}

fn env_for(tools: &[&str], control: Option<&str>, fix: &str) -> Env {
    let tools: Vec<String> = tools.iter().map(|s| (*s).to_owned()).collect();
    Env {
        which: Box::new(move |n| {
            tools
                .iter()
                .any(|t| t == n)
                .then(|| format!("/usr/bin/{n}"))
        }),
        control_dir: control.map(str::to_owned),
        home: Some(fix.to_owned()),
    }
}

fn case_env(case: &J, fix: &str) -> Env {
    let tools: Vec<&str> = case["which"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(J::as_str)
        .collect();
    env_for(&tools, case["control_dir"].as_str(), fix)
}

fn s(v: &J, fix: &str) -> String {
    v.as_str().expect("string").replace("@FIX@", fix)
}

fn strings(v: &J, fix: &str) -> Vec<String> {
    v.as_array()
        .expect("list")
        .iter()
        .map(|x| s(x, fix))
        .collect()
}

// POSIX quoting of a `C:\...` path; Windows builds argv instead (ShellCommand::Argv).
#[cfg(unix)]
#[test]
fn build_matches_golden() {
    let fix = fix_dir();
    let cases = common::golden("scp_build");
    assert!(cases.len() > 150);
    for case in &cases {
        let n = common::name(case);
        let conn = from_golden(&case["conn"], &fix);
        let env = env_for(
            &case["which"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(J::as_str)
                .collect::<Vec<_>>(),
            None,
            &fix,
        );
        let got = build_scp_command_with(
            &conn,
            &s(&case["local"], &fix),
            &s(&case["remote"], &fix),
            case["upload"].as_bool().expect("upload"),
            &env,
        );
        if case.get("error").is_some() {
            assert_eq!(got, Err(s(&case["message"], &fix)), "{n}");
        } else {
            assert_eq!(got, Ok(s(&case["expect"], &fix)), "{n}");
        }
    }
}

#[cfg(unix)] // golden argv uses sshpass; Windows uses askpass
#[test]
fn argv_matches_golden() {
    let fix = fix_dir();
    let cases = common::golden("scp_argv");
    assert!(cases.len() > 250);
    for case in &cases {
        let n = common::name(case);
        let conn = from_golden(&case["conn"], &fix);
        let launch = from_golden(&case["launch"], &fix);
        let env = case_env(case, &fix);
        let a = &case["args"];
        let (local, remote) = (s(&a["local_path"], &fix), s(&a["remote_path"], &fix));
        let flag = |k: &str| a[k].as_bool().expect(k);
        let (got, batch) = match case["fn"].as_str().expect("fn") {
            "scp" => (
                scp_argv_with(
                    &conn,
                    &launch,
                    &ScpTransfer {
                        local_path: &local,
                        remote_path: &remote,
                        upload: flag("upload"),
                        recursive: flag("recursive"),
                        preserve: flag("preserve"),
                    },
                    &env,
                ),
                None,
            ),
            "sftp" => {
                let batch_path = s(&a["batch_path"], &fix);
                let r = sftp_resume_argv_with(
                    &conn,
                    &launch,
                    &SftpResume {
                        local_path: &local,
                        remote_path: &remote,
                        batch_path: &batch_path,
                        recursive: flag("recursive"),
                    },
                    &env,
                );
                match r {
                    Ok((argv, batch)) => (Ok(argv), Some(batch)),
                    Err(e) => (Err(e), None),
                }
            }
            other => panic!("{n}: unknown fn {other}"),
        };
        if case.get("error").is_some() {
            assert_eq!(got, Err(s(&case["message"], &fix)), "{n}");
        } else if let Some(batch) = batch {
            assert_eq!(got, Ok(strings(&case["expect"][0], &fix)), "{n}");
            assert_eq!(batch, s(&case["expect"][1], &fix), "{n}");
        } else {
            assert_eq!(got, Ok(strings(&case["expect"], &fix)), "{n}");
        }
    }
}

#[test]
fn misc_matches_golden() {
    let cases = common::golden("scp_misc");
    assert!(cases.len() > 20);
    for case in &cases {
        let n = common::name(case);
        let input = case["input"].as_str().expect("input");
        let got = match case["fn"].as_str().expect("fn") {
            "glob_escape" => scp_glob_escape(input),
            "sftp_quote" => sftp_quote(input),
            other => panic!("{n}: unknown fn {other}"),
        };
        assert_eq!(got, case["expect"].as_str().expect("expect"), "{n}");
    }
}

// ---- inline scp/sftp cases ----

fn conn(pairs: &[(&str, Value)]) -> Value {
    Value::Map(
        pairs
            .iter()
            .map(|(k, v)| (Value::Str((*k).to_owned()), v.clone()))
            .collect(),
    )
}

fn st(t: &str) -> Value {
    Value::Str(t.to_owned())
}

#[test]
fn glob_escape_and_quote_examples() {
    assert_eq!(scp_glob_escape("[ab].log"), "\\[ab\\].log");
    assert_eq!(scp_glob_escape("q?.txt"), "q\\?.txt");
    assert_eq!(scp_glob_escape("a\\b"), "a\\\\b");
    assert_eq!(scp_glob_escape("{a,b}.log"), "{a,b}.log");
    assert_eq!(sftp_quote("a \"b\" \\c [1]"), "\"a \\\"b\\\" \\\\c [1]\"");
}

#[test]
fn tilde_is_quoted_and_ipv6_bracketed() {
    let env = env_for(&[], None, "/nonexistent-home");
    let c = conn(&[("host", st("::1")), ("user", st("u"))]);
    let line = build_scp_command_with(&c, "/tmp/x", "~/a", false, &env);
    assert_eq!(line, Ok("scp 'u@[::1]:~/a' /tmp/x".to_owned()));
}

#[test]
fn dash_paths_and_targets_are_rejected() {
    let env = env_for(&[], None, "/nonexistent-home");
    let c = conn(&[("host", st("h")), ("user", st("u"))]);
    assert_eq!(
        build_scp_command_with(&c, "-x", "/r", false, &env),
        Err("Local path must not start with '-'.".to_owned())
    );
    assert_eq!(
        build_scp_command_with(&c, "/tmp/x", "-x", false, &env),
        Err("Remote path must not start with '-'.".to_owned())
    );
    let bad = conn(&[("host", st("h")), ("user", st("-oProxyCommand=id"))]);
    assert!(build_scp_command_with(&bad, "/tmp/x", "/r", false, &env).is_err());
}

#[test]
fn sftp_resume_shape() {
    let env = env_for(&[], None, "/nonexistent-home");
    let c = conn(&[("host", st("::1")), ("user", st("u"))]);
    let (argv, batch) = sftp_resume_argv_with(
        &c,
        &Value::Null,
        &SftpResume {
            local_path: "/tmp/o",
            remote_path: "/r/[a]",
            batch_path: "/b",
            recursive: true,
        },
        &env,
    )
    .expect("argv");
    assert_eq!(argv[0], "sftp");
    assert_eq!(&argv[argv.len() - 3..], ["-b", "/b", "u@[::1]"]);
    assert_eq!(batch, "progress\nreget -R -p \"/r/[a]\" \"/tmp/o\"\n");
}

#[test]
fn sftp_resume_refuses_line_breaks_and_nul_in_paths_k15() {
    // A server-sent name "x\n!touch /tmp/pwned" must not become a second batch line.
    let env = env_for(&[], None, "/nonexistent-home");
    let c = conn(&[("host", st("h")), ("user", st("u"))]);
    for bad in ["/r/x\n!touch /tmp/pwned", "/r/x\r!id", "/r/x\0y"] {
        for (local, remote) in [("/tmp/o", bad), (bad, "/r/ok")] {
            let r = sftp_resume_argv_with(
                &c,
                &Value::Null,
                &SftpResume {
                    local_path: local,
                    remote_path: remote,
                    batch_path: "/b",
                    recursive: false,
                },
                &env,
            );
            assert_eq!(
                r,
                Err(termigator_core::proto::scp::LINE_BREAK_MSG.to_owned()),
                "{bad:?}"
            );
        }
    }
}
