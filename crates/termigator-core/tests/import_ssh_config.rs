//! ~/.ssh/config import (compat/golden/ssh_config.json), unit cases (including the
//! `ssh -G` cross-check when OpenSSH is installed), and hostile-input checks.
// Unix-only for now: uses modes, symlinks, sockets or sh stubs.
#![cfg(unix)]
mod common;
mod live;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use serde_json::Value as J;
use termigator_core::config::doc::Value;
use termigator_core::import::ssh_config::{MAX_INCLUDED_FILES, SshConfigImport, parse_ssh_config};

const USER: &str = "tester";

fn from_golden(v: &J) -> Value {
    if let Some((tag, p)) = common::tagged(v) {
        let s = || p.as_str().unwrap_or_default().to_owned();
        return match tag {
            "$int" => Value::Int(s().parse().expect("i64")),
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
        other => panic!("unexpected {other}"),
    }
}

fn walk(v: &Value, f: &dyn Fn(&str) -> String) -> Value {
    match v {
        Value::Str(s) => Value::Str(f(s)),
        Value::Seq(x) => Value::Seq(x.iter().map(|y| walk(y, f)).collect()),
        Value::Map(x) => Value::Map(x.iter().map(|(k, y)| (walk(k, f), walk(y, f))).collect()),
        other => other.clone(),
    }
}

fn tmp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("termigator-sshcfg-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&d);
    fs::create_dir_all(&d).expect("tmp dir");
    d
}

fn shape(res: SshConfigImport, tmp: &Path) -> Value {
    let t = tmp.to_string_lossy().into_owned();
    let fix = |s: &str| s.replace(&t, "@TMP@");
    Value::Map(vec![
        (
            Value::Str("connections".into()),
            walk(&Value::Seq(res.connections), &fix),
        ),
        (
            Value::Str("skipped_match_blocks".into()),
            Value::Int(res.skipped_match_blocks as i64),
        ),
        (
            Value::Str("unreadable".into()),
            Value::Seq(res.unreadable.iter().map(|s| Value::Str(fix(s))).collect()),
        ),
    ])
}

fn run_case(case: &J) -> (Value, Value) {
    let name = common::name(case);
    let d = tmp(&format!(
        "g{}",
        name.replace(|c: char| !c.is_ascii_alphanumeric(), "_")
    ));
    let t = d.to_string_lossy().into_owned();
    let root = d.join("ssh");
    fs::create_dir_all(&root).expect("ssh");
    fs::create_dir_all(d.join("home")).expect("home");
    for (file, body) in case["files"].as_object().expect("files") {
        let bytes = match body {
            J::String(s) => s.replace("@TMP@", &t).into_bytes(),
            other => common::bytes(other),
        };
        let f = root.join(file);
        fs::create_dir_all(f.parent().expect("parent")).expect("mkdir");
        fs::write(f, bytes).expect("write");
    }
    for sub in case["dirs"].as_array().into_iter().flatten() {
        fs::create_dir_all(root.join(sub.as_str().expect("dir"))).expect("mkdir");
    }
    for rel in case["touch"].as_array().into_iter().flatten() {
        let f = d.join(rel.as_str().expect("touch"));
        fs::create_dir_all(f.parent().expect("parent")).expect("mkdir");
        fs::write(f, "x").expect("touch");
    }
    let main = root.join(case["main"].as_str().unwrap_or("config"));
    let user = case["user"].as_str().unwrap_or(USER);
    let got = shape(parse_ssh_config(&main, &d.join("home"), user), &d);
    let _ = fs::remove_dir_all(&d);
    (got, from_golden(&case["expect"]))
}

#[test]
fn configs_match_golden() {
    let cases = common::golden("ssh_config");
    assert!(cases.len() > 250);
    for case in cases {
        let (got, want) = run_case(&case);
        assert_eq!(got, want, "{}", common::name(&case));
    }
}

fn s(v: &str) -> Value {
    Value::Str(v.to_owned())
}

fn parse(text: &str, d: &Path) -> SshConfigImport {
    let p = d.join("config");
    fs::write(&p, text).expect("write");
    parse_ssh_config(&p, d, USER)
}

fn by_label(res: &SshConfigImport, label: &str) -> Value {
    res.connections
        .iter()
        .find(|c| c.get("label") == Some(&s(label)))
        .cloned()
        .unwrap_or_else(|| panic!("no entry {label}"))
}

#[test]
fn no_user_means_the_local_user() {
    let d = tmp("nouser");
    let res = parse("Host box\n  HostName box.example\n", &d);
    assert_eq!(
        res.connections,
        vec![Value::Map(vec![
            (s("label"), s("box")),
            (s("host"), s("box.example")),
            (s("user"), s(USER)),
            (s("group"), s("ssh_config")),
        ])]
    );
    let _ = fs::remove_dir_all(&d);
}

#[test]
fn missing_file_is_reported() {
    let d = tmp("missing");
    let res = parse_ssh_config(&d.join("nope"), &d, USER);
    assert!(res.connections.is_empty());
    assert_eq!(
        res.unreadable,
        vec![d.join("nope").to_string_lossy().into_owned()]
    );
    let _ = fs::remove_dir_all(&d);
}

#[test]
fn proxycommand_is_copied_like_golden() {
    // Kept as is; the caller is expected to warn.
    let d = tmp("k4");
    let res = parse(
        "Host a\n  ProxyCommand nc %h %p\n  LocalCommand echo hi\n  PermitLocalCommand yes\n",
        &d,
    );
    let args = by_label(&res, "a");
    let args = args.get("extra_ssh_args").expect("extra args").clone();
    assert_eq!(
        args,
        Value::Seq(
            [
                "-o",
                "ProxyCommand=nc %h %p",
                "-o",
                "LocalCommand=echo hi",
                "-o",
                "PermitLocalCommand=yes"
            ]
            .map(s)
            .to_vec()
        )
    );
    let _ = fs::remove_dir_all(&d);
}

#[test]
fn include_bombs_are_bounded() {
    // Every level includes the next ten times: 10^16 reads without a cap.
    let d = tmp("bomb");
    fs::write(d.join("config"), "Include a\nHost al\n  HostName h\n").expect("write");
    let body = "Include b\n".repeat(10);
    fs::write(d.join("a"), format!("{body}Host a\n")).expect("write");
    fs::write(d.join("b"), "Include a\nInclude a\nHost b\n").expect("write");
    let started = Instant::now();
    let res = parse_ssh_config(&d.join("config"), &d, USER);
    assert!(started.elapsed() < Duration::from_secs(20));
    assert!(res.truncated);
    assert!(res.connections.len() <= 3);

    let wide = tmp("wide");
    for i in 0..MAX_INCLUDED_FILES + 50 {
        fs::write(wide.join(format!("{i:05}.conf")), format!("Host h{i}\n")).expect("write");
    }
    fs::write(wide.join("config"), "Include *.conf\n").expect("write");
    let res = parse_ssh_config(&wide.join("config"), &wide, USER);
    assert!(res.truncated);
    assert!(res.connections.len() < MAX_INCLUDED_FILES);
    let _ = fs::remove_dir_all(&d);
    let _ = fs::remove_dir_all(&wide);
}

#[test]
fn many_blocks_and_aliases_are_capped() {
    let d = tmp("blocks");
    let text: String = (0..30_000)
        .map(|i| format!("Host h{i}\n  User u{i}\n"))
        .collect();
    let started = Instant::now();
    let res = parse(&text, &d);
    assert!(started.elapsed() < Duration::from_secs(60));
    assert!(res.truncated);
    assert!(res.connections.len() <= 2_000);
    let _ = fs::remove_dir_all(&d);
}

#[test]
fn oversized_files_are_unreadable() {
    let d = tmp("big");
    let big = format!("Host a\n{}", "#".repeat(17 * 1024 * 1024));
    let res = parse(&big, &d);
    assert!(res.connections.is_empty());
    assert_eq!(res.unreadable.len(), 1);
    let _ = fs::remove_dir_all(&d);
}

#[test]
fn hostile_input_does_not_panic_or_hang() {
    let d = tmp("hostile");
    let junk: String = (0..4000u32)
        .map(|i| char::from_u32(0x20 + (i * 7919) % 0x2f00).unwrap_or('?'))
        .collect();
    let pathological = format!(
        "Host {}b\n  User u\nHost {}\n",
        "a*".repeat(2000),
        "a".repeat(3000)
    );
    let samples = [
        "Host [\n".to_owned(),
        "Host [!\n".to_owned(),
        "Host [a-\n".to_owned(),
        "Host [z-a-\n  User u\nHost z\n".to_owned(),
        "Host [c-b-a]\nHost b c a -\n".to_owned(),
        "Host [b-a-c]\nHost b c a -\n".to_owned(),
        "Host a b [a-\n".to_owned(),
        "Match\nMatch all all\nHost\nHost ,\nHost \"\n".to_owned(),
        "Include\nInclude \"\nInclude [\nInclude *\nInclude **/**/**\nInclude /proc/*/nope\n"
            .to_owned(),
        format!("Host {junk}\n  HostName {junk}\n  Port {junk}\n  LocalForward {junk}\n"),
        pathological,
        "Host al\n  HostName %\n  IdentityFile %%%%%\n  Port 99999999999999999999999\n".to_owned(),
        "%".repeat(100_000),
        "Host al\n".to_owned() + &"  Ciphers x\n".repeat(100_000),
        "\0Host a\n\0".to_owned(),
        "=".to_owned(),
        "= = =\n".repeat(1000),
    ];
    for (i, text) in samples.iter().enumerate() {
        let p = d.join(format!("c{i}"));
        fs::write(&p, text).expect("write");
        let started = Instant::now();
        let _ = parse_ssh_config(&p, &d, USER);
        assert!(
            started.elapsed() < Duration::from_secs(30),
            "sample {i} is slow"
        );
    }
    assert!(
        !parse_ssh_config(&d.join("missing"), &d, USER)
            .unreadable
            .is_empty()
    );
    let _ = fs::remove_dir_all(&d);
}

/// Cross-check host / user / port with OpenSSH's own resolution (`ssh -G`) for aliases not
/// touched by the unevaluable `Match` block. The config has no `Include`: ssh would resolve
/// it against the real `~/.ssh`, which these tests must not read.
#[test]
fn matches_ssh_dash_g() {
    let have_ssh = Command::new("ssh")
        .arg("-V")
        .output()
        .is_ok_and(|o| o.status.success());
    if !have_ssh {
        live::skip("matches_ssh_dash_g", "no OpenSSH client");
        return;
    }
    let d = tmp("sshg");
    let key = d.join("id_work");
    fs::write(&key, "PRIVATE KEY\n").expect("key");
    let text = format!(
        "# global\nCompression yes\n\nHost web\n    HostName 10.0.0.5\n    Port 2222\n    User = deploy\n\
         \x20   IdentityFile {d}/missing_key\n    IdentityFile {key}\n    LocalForward 8080 localhost:80\n\
         \x20   ForwardAgent yes\n    ProxyCommand ssh -W %h:%p bastion\n    Ciphers aes256-gcm@openssh.com\n\n\
         Host db-*  !db-test\n    User dba\n\nHost db-prod\n    HostName \"db.prod.example\"\n\
         \x20   ProxyJump bastion,jump2\n    StrictHostKeyChecking accept-new\n\n\
         Host db-test\n    HostName db.test.example\n\nHost *\n    User fallback\n    ServerAliveInterval 30\n",
        d = d.display(),
        key = key.display()
    );
    let res = parse(&text, &d);
    for alias in ["web", "db-prod", "db-test"] {
        let out = Command::new("ssh")
            .arg("-G")
            .arg("-F")
            .arg(d.join("config"))
            .arg(alias)
            .output()
            .expect("ssh -G");
        assert!(out.status.success(), "ssh -G {alias}");
        let out = String::from_utf8_lossy(&out.stdout).into_owned();
        let get = |k: &str| {
            out.lines()
                .find_map(|l| l.strip_prefix(&format!("{k} ")))
                .unwrap_or_else(|| panic!("no {k} for {alias}"))
                .to_owned()
        };
        let row = by_label(&res, alias);
        assert_eq!(row.get("host"), Some(&s(&get("hostname"))), "{alias}");
        assert_eq!(row.get("user"), Some(&s(&get("user"))), "{alias}");
        let port = match row.get("port") {
            Some(Value::Int(p)) => *p,
            _ => 22,
        };
        assert_eq!(port.to_string(), get("port"), "{alias}");
    }
    let _ = fs::remove_dir_all(&d);
}
