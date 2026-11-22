//! MobaXterm import (compat/golden/moba_*.json), the importer unit tests, and the
//! `termigator-import-moba` CLI on a temp dir.
mod common;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde_json::Value as J;
use termigator_core::config::doc::{Value, load as yaml_load};
use termigator_core::import::moba::{
    connection_key, decode_moba_bytes, merge_moba_connections, moba_identity_to_linux,
    parse_moba_bookmarks, parse_moba_export,
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
        other => panic!("unexpected {other}"),
    }
}

fn seq(v: &J) -> Vec<Value> {
    match from_golden(v) {
        Value::Seq(s) => s,
        other => panic!("not a list: {other:?}"),
    }
}

/// Ordered comparison (`loose_eq` ignores key order, but session.yaml order is visible).
fn same(a: &[Value], b: &[Value], case: &str) {
    let neutral = |v: &[Value]| common::without_win_key_paths(&Value::Seq(v.to_vec()));
    assert_eq!(neutral(a), neutral(b), "{case}");
}

#[test]
fn decode_matches_golden() {
    for case in common::golden("moba_decode") {
        let name = common::name(&case);
        let got = decode_moba_bytes(&common::bytes(&case["input"]));
        if case.get("error").is_some() {
            assert!(got.is_err(), "{name}");
        } else {
            assert_eq!(got.ok().as_deref(), case["expect"].as_str(), "{name}");
        }
    }
}

#[test]
fn parse_matches_golden() {
    let cases = common::golden("moba_parse");
    assert!(cases.len() > 50);
    for case in cases {
        let name = common::name(&case);
        let text = case["input"].as_str().expect("input");
        let user = case["user"].as_str().expect("user");
        match case["kind"].as_str() {
            Some("bookmarks") => {
                same(
                    &parse_moba_bookmarks(text, user),
                    &seq(&case["expect"]),
                    name,
                );
            }
            Some("export") => {
                let (rows, skipped) = parse_moba_export(text, user);
                let expect = from_golden(&case["expect"]);
                let Some(Value::Seq(want_rows)) = expect.get("rows") else {
                    panic!("{name}: no rows")
                };
                same(&rows, want_rows, name);
                let want = expect.get("skipped").cloned().expect("skipped");
                let got = Value::Map(
                    skipped
                        .iter()
                        .map(|(k, n)| (Value::Str(k.clone()), Value::Int(i64::from(*n))))
                        .collect(),
                );
                assert_eq!(got, want, "{name} skipped");
            }
            other => panic!("{name}: kind {other:?}"),
        }
    }
}

#[test]
fn identity_matches_golden() {
    for case in common::golden("moba_identity") {
        let name = common::name(&case);
        let got =
            moba_identity_to_linux(case["path"].as_str(), case["user"].as_str().expect("user"));
        assert_eq!(got.as_deref(), case["expect"].as_str(), "{name}");
    }
}

#[test]
fn moba_folder_tokens_become_linux_paths() {
    let to = |p: &str| moba_identity_to_linux(Some(p), "bob");
    assert_eq!(
        to(r"_DesktopDir_\mykey.ppk").as_deref(),
        Some("~/Desktop/mykey.ppk")
    );
    assert_eq!(
        to(r"_ProfileDir_\.ssh\id_rsa").as_deref(),
        Some("~/.ssh/id_rsa")
    );
    assert_eq!(to(r"_CurrentDrive_:\keys\a").as_deref(), Some("~/keys/a"));
    // An unknown token is left as written, not guessed.
    assert_eq!(to(r"_WhatDir_\k").as_deref(), Some("_WhatDir_/k"));
}

#[test]
fn a_bookmark_with_a_folder_token_keeps_its_key() {
    let text = "[Bookmarks]\r\nSubRep=\r\nImgNum=42\r\n\
                srv=#109#0%h.example%22%bob%%-1%-1%%%%%0%0%0%%%-1%0%0%0%%1080%%0%0%1%%0%%%%0%-1%_DesktopDir_\\mykey.ppk%0%0%0%0%\r\n";
    let got = parse_moba_bookmarks(text, "bob");
    assert_eq!(got.len(), 1);
    let key = got[0]
        .get("identity_file")
        .map(termigator_core::config::model::value_str);
    if cfg!(windows) {
        // Windows: the real Desktop folder (shell folder; OneDrive may move it).
        let key = key.unwrap_or_default();
        assert!(
            key.ends_with("\\mykey.ppk") && !key.contains("_DesktopDir_"),
            "{key}"
        );
    } else {
        assert_eq!(key.as_deref(), Some("~/Desktop/mykey.ppk"));
    }
}

#[test]
fn merge_and_key_match_golden() {
    for case in common::golden("moba_merge") {
        let name = common::name(&case);
        match case["kind"].as_str() {
            Some("key") => {
                let (h, u, p) = connection_key(&from_golden(&case["conn"]));
                let want = seq(&case["expect"]);
                let got = vec![Value::Str(h), Value::Str(u), Value::Int(p)];
                assert_eq!(got, want, "{name}");
            }
            Some("merge") => {
                let (merged, added) = merge_moba_connections(
                    &seq(&case["existing"]),
                    &seq(&case["parsed"]),
                    case["default_terminal"].as_str().expect("terminal"),
                );
                same(&merged, &seq(&case["merged"]), name);
                assert_eq!(
                    added as u64,
                    case["added"].as_u64().expect("added"),
                    "{name}"
                );
            }
            other => panic!("{name}: kind {other:?}"),
        }
    }
}

#[test]
fn k2_rdp_and_ssh_for_one_host_collide() {
    let (rows, _) = parse_moba_export("[Bookmarks]\nA=#1#4%h%3389%u\nC=#109#0%h%22%u%\n", "me");
    let (_, added) = merge_moba_connections(&[], &rows, "embedded");
    assert_eq!(added, 1);
}

// ---- CLI

const SAMPLE: &str = "[Misc]\nLastUsername=alice\n\n[Bookmarks]\nSubRep=Production\nProd Web=#109#0%web.example%22%root%\n\n[Bookmarks_2]\nSubRep=Lab\\Internal\nLab Box=#109#0%10.0.0.5%2222%ubuntu%\n";

fn tmp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("termigator-moba-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&d);
    fs::create_dir_all(&d).expect("tmp dir");
    d
}

fn cli(dir: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_termigator-import-moba"))
        .args(args)
        .env("TERMIGATOR_CONFIG_DIR", dir.join("cfgdir"))
        .env("HOME", dir)
        .env("USER", "tester")
        .output()
        .expect("run cli")
}

fn session_hosts(path: &Path) -> Vec<String> {
    let Value::Seq(rows) = yaml_load(&fs::read_to_string(path).expect("session")).expect("yaml")
    else {
        panic!("session is not a list")
    };
    rows.iter()
        .map(|r| match r.get("host") {
            Some(Value::Str(h)) => h.clone(),
            _ => String::new(),
        })
        .collect()
}

#[test]
fn cli_writes_merges_and_honours_config_dir() {
    let dir = tmp("write");
    let ini = dir.join("moba.ini");
    // UTF-16 with BOM: the CLI decodes like the library.
    let mut bytes = vec![0xff, 0xfe];
    bytes.extend(SAMPLE.encode_utf16().flat_map(u16::to_le_bytes));
    fs::write(&ini, bytes).expect("ini");
    let session = dir.join("cfgdir/session.yaml");
    fs::create_dir_all(dir.join("cfgdir")).expect("cfgdir");
    fs::write(&session, "- host: web.example\n  user: root\n").expect("session");

    let out = cli(&dir, &[ini.to_str().expect("utf8")]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("added 1 connections (2 from Moba, duplicates skipped)."),
        "{stdout}"
    );
    assert_eq!(session_hosts(&session), ["web.example", "10.0.0.5"]);
    let text = fs::read_to_string(&session).expect("session");
    assert!(
        text.contains("terminal: embedded") && text.contains("group: Lab/Internal"),
        "{text}"
    );

    // A second run adds nothing.
    let out = cli(&dir, &[ini.to_str().expect("utf8")]);
    assert!(String::from_utf8_lossy(&out.stdout).contains("added 0 connections"));
    assert_eq!(session_hosts(&session).len(), 2);
}

#[test]
fn cli_dry_run_and_explicit_paths() {
    let dir = tmp("dry");
    let ini = dir.join("moba.ini");
    fs::write(&ini, SAMPLE).expect("ini");
    let session = dir.join("out/session.yaml");
    let config = dir.join("out/config.yaml");
    fs::create_dir_all(dir.join("out")).expect("out");
    fs::write(&config, "window:\n  default_terminal: external\n").expect("config");
    let args = [
        ini.to_str().expect("utf8"),
        "--session",
        session.to_str().expect("utf8"),
        "--config",
        config.to_str().expect("utf8"),
    ];

    let mut dry = args.to_vec();
    dry.push("--dry-run");
    let out = cli(&dir, &dry);
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.starts_with("# Termigator SSH connections — merged from MobaXterm import.\n"));
    assert!(stdout.contains("terminal: external"), "{stdout}");
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("# Would add 2 new connections (2 parsed")
    );
    assert!(!session.exists());
    assert!(!dir.join("cfgdir").exists());

    let out = cli(&dir, &args);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(session_hosts(&session), ["web.example", "10.0.0.5"]);
    assert!(
        fs::read_to_string(&session)
            .expect("session")
            .contains("terminal: external")
    );
}

#[test]
fn cli_errors() {
    let dir = tmp("err");
    let ini = dir.join("empty.ini");
    fs::write(&ini, "[Misc]\nx=1\n").expect("ini");
    let out = cli(&dir, &[ini.to_str().expect("utf8")]);
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(
        String::from_utf8_lossy(&out.stderr),
        "No #109 SSH bookmarks found.\n"
    );
    assert_eq!(cli(&dir, &[]).status.code(), Some(2));
    assert_eq!(cli(&dir, &["--bogus", "x"]).status.code(), Some(2));
    assert_eq!(
        cli(&dir, &[dir.join("missing.ini").to_str().expect("utf8")])
            .status
            .code(),
        Some(1)
    );
    let _ = fs::remove_dir_all(&dir);
}
