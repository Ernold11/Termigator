//! PuTTY / .reg / SuperPuTTY import against the golden fixtures (compat/golden/putty_*.json), and hostile-input checks.
mod common;

use std::fs;
use std::path::PathBuf;

use serde_json::Value as J;
use termigator_core::config::doc::Value;
use termigator_core::import::moba::merge_moba_connections;
use termigator_core::import::putty::{
    MAX_FILE_BYTES, parse_putty_export, parse_putty_reg, parse_putty_session_file,
    parse_putty_sessions_dir, parse_superputty_xml,
};

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

fn rows(v: &J) -> Vec<Value> {
    match from_golden(v) {
        Value::Seq(s) => s,
        other => panic!("not a list: {other:?}"),
    }
}

fn user(case: &J) -> &str {
    case["user"].as_str().expect("user")
}

fn tmp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("termigator-putty-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&d);
    fs::create_dir_all(&d).expect("tmp dir");
    d
}

fn field<'a>(row: &'a Value, key: &str) -> &'a Value {
    row.get(key).unwrap_or(&Value::Null)
}

fn text(s: &str) -> Value {
    Value::Str(s.to_owned())
}

#[test]
fn session_files_match_golden() {
    let cases = common::golden("putty_session");
    assert!(cases.len() > 70);
    for case in cases {
        let got = parse_putty_session_file(
            case["file"].as_str().expect("file"),
            case["input"].as_str().expect("input"),
            user(&case),
        );
        assert_eq!(
            common::without_win_key_paths(&got.unwrap_or(Value::Null)),
            common::without_win_key_paths(&from_golden(&case["expect"])),
            "{}",
            common::name(&case)
        );
    }
}

#[test]
fn reg_matches_golden() {
    let cases = common::golden("putty_reg");
    assert!(cases.len() > 50);
    for case in cases {
        let got = parse_putty_reg(case["input"].as_str().expect("input"), user(&case));
        assert_eq!(
            common::without_win_key_paths(&Value::Seq(got)),
            common::without_win_key_paths(&Value::Seq(rows(&case["expect"]))),
            "{}",
            common::name(&case)
        );
    }
}

#[test]
fn xml_matches_golden() {
    let cases = common::golden("putty_xml");
    assert!(cases.len() > 50);
    for case in cases {
        let got = parse_superputty_xml(case["input"].as_str().expect("input"));
        assert_eq!(
            common::without_win_key_paths(&Value::Seq(got)),
            common::without_win_key_paths(&Value::Seq(rows(&case["expect"]))),
            "{}",
            common::name(&case)
        );
    }
}

#[test]
fn export_dispatch_matches_golden() {
    let d = tmp("export");
    for case in common::golden("putty_export") {
        let name = common::name(&case);
        let path = d.join(format!(
            "export{}",
            case["suffix"].as_str().expect("suffix")
        ));
        fs::write(&path, common::bytes(&case["input"])).expect("write");
        let got = parse_putty_export(&path, user(&case));
        assert_eq!(got, rows(&case["expect"]), "{name}");
    }
    let _ = fs::remove_dir_all(&d);
}

// The case data has sessions that differ only in letter case: one file on a case-insensitive Windows disk.
#[cfg(unix)]
#[test]
fn sessions_dir_matches_golden() {
    for case in common::golden("putty_dir") {
        let name = common::name(&case);
        let d = tmp(&format!("dir-{name}"));
        for (file, body) in case["files"].as_object().expect("files") {
            fs::write(d.join(file), body.as_str().expect("body")).expect("write");
        }
        for sub in case["subdirs"].as_array().expect("subdirs") {
            fs::create_dir(d.join(sub.as_str().expect("sub"))).expect("mkdir");
        }
        let got = parse_putty_sessions_dir(&d, user(&case));
        assert_eq!(got, rows(&case["expect"]), "{name}");
        let _ = fs::remove_dir_all(&d);
    }
}

#[test]
fn missing_dir_and_files_are_empty() {
    let d = tmp("missing");
    assert!(parse_putty_sessions_dir(&d.join("nope"), "u").is_empty());
    assert!(parse_putty_export(&d.join("nope.reg"), "u").is_empty());
    let _ = fs::remove_dir_all(&d);
}

#[test]
fn oversized_files_are_skipped() {
    let d = tmp("big");
    let big = format!("HostName=h\n{}", "#".repeat(MAX_FILE_BYTES as usize));
    fs::write(d.join("big"), &big).expect("write");
    fs::write(d.join("ok"), "HostName=h2\n").expect("write");
    let got = parse_putty_sessions_dir(&d, "u");
    assert_eq!(got.len(), 1);
    assert_eq!(field(&got[0], "host"), &text("h2"));
    fs::write(d.join("big.reg"), &big).expect("write");
    assert!(parse_putty_export(&d.join("big.reg"), "u").is_empty());
    let _ = fs::remove_dir_all(&d);
}

#[test]
fn hostile_input_does_not_panic() {
    let deep = format!("{}{}", "<a>".repeat(200_000), "</a>".repeat(200_000));
    assert!(parse_superputty_xml(&deep).is_empty());
    let unclosed = "<a>".repeat(200_000);
    assert!(parse_superputty_xml(&unclosed).is_empty());
    let many = format!(
        "<r>{}</r>",
        r#"<SessionData Host="h" Port="99999999999999999999999" />"#.repeat(5000)
    );
    assert_eq!(parse_superputty_xml(&many).len(), 5000);
    let junk: String = (0..4000u32)
        .map(|i| char::from_u32(0x20 + (i * 7919) % 0x2f00).unwrap_or('?'))
        .collect();
    for s in [
        "[".to_owned(),
        "[HKEY_".to_owned(),
        "[HKEY_\u{e9}\u{e9}\\PuTTY\\Sessions\\x]".to_owned(),
        "[HKEY_x\\PuTTY\\Sessions\\]".to_owned(),
        "\"".to_owned(),
        "\"a\"=\"".to_owned(),
        "\"a\"=dword:".to_owned(),
        "[HKEY_\\PuTTY\\Sessions\\\u{1f600}".to_owned(),
        junk.clone(),
        format!("[HKEY_x\\PuTTY\\Sessions\\{junk}]\n\"HostName\"=\"{junk}\""),
    ] {
        let _ = parse_putty_reg(&s, "u");
        let _ = parse_superputty_xml(&s);
        let _ = parse_putty_session_file(&s, &s, "u");
    }
    let long = "PortForwardings=".to_owned() + &",".repeat(100_000) + "L1=a:1\nHostName=h\n";
    assert!(parse_putty_session_file("x", &long, "u").is_some());
}

#[test]
fn billion_laughs_is_not_expanded() {
    let xml = r#"<?xml version="1.0"?>
<!DOCTYPE lolz [<!ENTITY lol "lol"><!ENTITY lol2 "&lol;&lol;&lol;&lol;&lol;&lol;&lol;&lol;&lol;&lol;">]>
<r><SessionData Host="&lol2;" /></r>"#;
    assert!(parse_superputty_xml(xml).is_empty());
}

#[test]
fn imported_rows_merge_into_existing() {
    let parsed = parse_putty_reg(
        "[HKEY_CURRENT_USER\\Software\\SimonTatham\\PuTTY\\Sessions\\a]\n\"HostName\"=\"h\"\n\"UserName\"=\"u\"\n",
        "tester",
    );
    let (merged, added) = merge_moba_connections(&[], &parsed, "embedded");
    assert_eq!(added, 1);
    assert_eq!(field(&merged[0], "terminal"), &text("embedded"));
    let (_, again) = merge_moba_connections(&merged, &parsed, "embedded");
    assert_eq!(again, 0);
}
