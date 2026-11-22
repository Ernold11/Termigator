//! Remmina import against the golden fixtures (compat/golden/remmina.json) and
//! hostile-input checks.
// Some cases are Unix-only (#[cfg(unix)]): their helpers and imports are unused on Windows.
#![cfg_attr(windows, allow(unused_imports, dead_code))]
mod common;

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value as J;
use termigator_core::config::doc::Value;
use termigator_core::import::remmina::{
    MAX_PROFILES, RemminaImport, parse_remmina_profiles, remmina_dirs,
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

fn tmp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("termigator-remmina-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&d);
    fs::create_dir_all(&d).expect("tmp dir");
    d
}

/// The Rust result in the shape of the golden `expect` (scratch path -> `@TMP@`).
fn shape(res: RemminaImport, tmp: &Path) -> Value {
    let t = tmp.to_string_lossy().into_owned();
    let fix = |s: &str| Value::Str(s.replace(&t, "@TMP@"));
    fn walk(v: &Value, f: &dyn Fn(&str) -> Value) -> Value {
        match v {
            Value::Str(s) => f(s),
            Value::Seq(x) => Value::Seq(x.iter().map(|y| walk(y, f)).collect()),
            Value::Map(x) => Value::Map(x.iter().map(|(k, y)| (walk(k, f), walk(y, f))).collect()),
            other => other.clone(),
        }
    }
    let list = |xs: &[String]| Value::Seq(xs.iter().map(|s| fix(s)).collect());
    Value::Map(vec![
        (
            Value::Str("connections".into()),
            walk(&Value::Seq(res.connections), &fix),
        ),
        (
            Value::Str("skipped".into()),
            Value::Map(
                res.skipped
                    .into_iter()
                    .map(|(k, n)| (Value::Str(k), Value::Int(n as i64)))
                    .collect(),
            ),
        ),
        (Value::Str("notes".into()), list(&res.notes)),
        (Value::Str("unreadable".into()), list(&res.unreadable)),
    ])
}

fn run_case(case: &J) -> (Value, Value) {
    let name = common::name(case);
    let d = tmp(&format!(
        "g{}",
        name.replace(|c: char| !c.is_ascii_alphanumeric(), "_")
    ));
    let t = d.to_string_lossy().into_owned();
    fs::create_dir_all(d.join("p")).expect("p");
    fs::create_dir_all(d.join("home")).expect("home");
    for (file, body) in case["files"].as_object().expect("files") {
        let bytes = match body {
            J::String(s) => s.replace("@TMP@", &t).into_bytes(),
            other => common::bytes(other),
        };
        fs::write(d.join("p").join(file), bytes).expect("write");
    }
    for sub in case["dirs"].as_array().into_iter().flatten() {
        fs::create_dir(d.join("p").join(sub.as_str().expect("dir"))).expect("mkdir");
    }
    for rel in case["touch"].as_array().into_iter().flatten() {
        let f = d.join(rel.as_str().expect("touch"));
        fs::create_dir_all(f.parent().expect("parent")).expect("mkdir");
        fs::write(f, "x").expect("touch");
    }
    let paths: Vec<PathBuf> = match case["paths"].as_array() {
        Some(a) => a
            .iter()
            .map(|p| d.join(p.as_str().expect("path")))
            .collect(),
        None => vec![d.join("p")],
    };
    let existing = match from_golden(&case["existing"]) {
        Value::Seq(s) => s,
        _ => Vec::new(),
    };
    let got = shape(
        parse_remmina_profiles(&paths, &existing, &d.join("home")),
        &d,
    );
    let _ = fs::remove_dir_all(&d);
    (got, from_golden(&case["expect"]))
}

// Remmina does not exist on Windows; the golden has `/` paths.
#[cfg(unix)]
#[test]
fn profiles_match_golden() {
    let cases = common::golden("remmina");
    assert!(cases.len() > 150);
    for case in cases {
        let (got, want) = run_case(&case);
        assert_eq!(got, want, "{}", common::name(&case));
    }
}

fn write(dir: &Path, name: &str, body: &str) {
    fs::write(dir.join(name), body).expect("write");
}

fn entry(res: &RemminaImport, label: &str) -> Value {
    res.connections
        .iter()
        .find(|c| c.get("label") == Some(&Value::Str(label.into())))
        .cloned()
        .unwrap_or_else(|| panic!("no entry {label}"))
}

#[test]
fn passwords_are_never_imported() {
    let d = tmp("pw");
    write(
        &d,
        "a.remmina",
        "[remmina]\nname=a\nprotocol=RDP\nserver=h\npassword=c2VjcmV0\nssh_password=x\nssh_tunnel_password=y\n",
    );
    let res = parse_remmina_profiles(&[&d], &[], &d);
    let text = format!("{:?}", res.connections);
    assert!(
        !text.contains("password") && !text.contains("c2VjcmV0"),
        "{text}"
    );
    let _ = fs::remove_dir_all(&d);
}

#[test]
fn dirs_are_injectable() {
    let d = tmp("dirs");
    let home = d.join("home");
    fs::create_dir_all(home.join(".remmina")).expect("old");
    fs::create_dir_all(d.join("xdg/remmina")).expect("xdg");
    let xdg = d.join("xdg");
    let xdg = xdg.to_str().expect("utf-8");
    assert_eq!(
        remmina_dirs(Some(xdg), &home),
        vec![d.join("xdg/remmina"), home.join(".remmina")]
    );
    // Without XDG_DATA_HOME (or empty): ~/.local/share/remmina, which does not exist here.
    assert_eq!(remmina_dirs(None, &home), vec![home.join(".remmina")]);
    assert_eq!(remmina_dirs(Some(""), &home), vec![home.join(".remmina")]);
    fs::create_dir_all(home.join(".local/share/remmina")).expect("default");
    assert_eq!(
        remmina_dirs(None, &home),
        vec![home.join(".local/share/remmina"), home.join(".remmina")]
    );
    let _ = fs::remove_dir_all(&d);
}

#[test]
fn tunnel_becomes_the_ssh_gateway_when_that_host_exists() {
    let d = tmp("gw");
    write(
        &d,
        "a.remmina",
        "[remmina]\nname=Raw VNC\nprotocol=VNC\nserver=10.0.0.240:5999\nssh_tunnel_enabled=1\n\
         ssh_tunnel_server=bastion.example.com\nssh_tunnel_username=jump\n",
    );
    let none = parse_remmina_profiles(&[&d], &[], &d);
    assert!(entry(&none, "Raw VNC").get("ssh_gateway").is_none());
    assert!(
        none.notes
            .iter()
            .any(|n| n.contains("Raw VNC") && n.contains("bastion.example.com"))
    );
    let existing = [Value::Map(vec![
        (Value::Str("label".into()), Value::Str("bastion".into())),
        (
            Value::Str("host".into()),
            Value::Str("bastion.example.com".into()),
        ),
        (Value::Str("user".into()), Value::Str("jump".into())),
    ])];
    let res = parse_remmina_profiles(&[&d], &existing, &d);
    assert_eq!(
        entry(&res, "Raw VNC").get("ssh_gateway"),
        Some(&Value::Str("bastion".into()))
    );
    assert!(res.notes.is_empty());
    let _ = fs::remove_dir_all(&d);
}

#[test]
fn non_ascii_or_huge_ports_count_as_absent() {
    // Superscript, bignum and Unicode-digit ports count as no port.
    let d = tmp("ports");
    for (i, s) in ["h:\u{b2}", "h:99999999999999999999999", "h:\u{663}\u{662}"]
        .iter()
        .enumerate()
    {
        write(
            &d,
            &format!("{i}.remmina"),
            &format!("[remmina]\nname=n{i}\nprotocol=SSH\nserver={s}\n"),
        );
    }
    let res = parse_remmina_profiles(&[&d], &[], &d);
    assert_eq!(res.connections.len(), 3);
    assert!(res.connections.iter().all(|c| c.get("port").is_none()));
    let _ = fs::remove_dir_all(&d);
}

#[test]
fn oversized_and_excess_files() {
    let d = tmp("big");
    let big = format!(
        "[remmina]\nname=big\nprotocol=SSH\nserver=h\n{}",
        "#".repeat(17 * 1024 * 1024)
    );
    write(&d, "big.remmina", &big);
    write(
        &d,
        "ok.remmina",
        "[remmina]\nname=ok\nprotocol=SSH\nserver=h\n",
    );
    let res = parse_remmina_profiles(&[&d], &[], &d);
    assert_eq!(res.connections.len(), 1);
    assert_eq!(res.unreadable.len(), 1);

    let many = tmp("many");
    for i in 0..MAX_PROFILES + 5 {
        write(
            &many,
            &format!("{i:06}.remmina"),
            "[remmina]\nname=n\nprotocol=SSH\nserver=h\n",
        );
    }
    let res = parse_remmina_profiles(&[&many], &[], &many);
    assert_eq!(res.connections.len(), MAX_PROFILES);
    assert!(res.notes.iter().any(|n| n.contains("first")));
    let _ = fs::remove_dir_all(&d);
    let _ = fs::remove_dir_all(&many);
}

#[test]
fn hostile_input_does_not_panic() {
    let d = tmp("hostile");
    let junk: String = (0..4000u32)
        .map(|i| char::from_u32(0x20 + (i * 7919) % 0x2f00).unwrap_or('?'))
        .collect();
    let samples = [
        "[".to_owned(),
        "]".to_owned(),
        "[]".to_owned(),
        "[[remmina]]".to_owned(),
        "[remmina]\n=".to_owned(),
        "[remmina]\n:\n".to_owned(),
        "[remmina]\n\u{1c}=\u{1f}\n".to_owned(),
        format!("[remmina]\nname={junk}\nserver=[{junk}\n"),
        format!("[remmina]\nname=a\n{}", "  x\n".repeat(100_000)),
        format!("[remmina]\n{}", "k=v\n\n".repeat(100_000)),
        "[DEFAULT]\n".repeat(50_000),
        "[remmina]\nprotocol=VNC\nserver=h:0\nssh_tunnel_enabled=1\nssh_tunnel_server=[\n"
            .to_owned(),
        "\0\0\0".to_owned(),
    ];
    for (i, s) in samples.iter().enumerate() {
        write(&d, &format!("{i:02}.remmina"), s);
    }
    fs::write(d.join("zz.remmina"), [0xffu8, 0xfe, 0x00, 0xd8]).expect("write");
    let res = parse_remmina_profiles(&[&d, &d.join("missing")], &[], &d);
    assert!(!res.unreadable.is_empty());
    let _ = fs::remove_dir_all(&d);
}
