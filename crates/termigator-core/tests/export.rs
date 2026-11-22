//! Export against the golden fixtures (compat/golden/export_alias.json, export_config.json)
//! and: round-trip through the importer, `ssh -G`, backup zip.
// Unix-only for now: uses modes, symlinks, sockets or sh stubs.
#![cfg(unix)]
mod common;
mod live;

use std::collections::HashSet;
use std::fs;
use std::io::Read;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value as J;
use termigator_core::config::doc::{self, Value};
use termigator_core::export::{Stamp, build_backup_zip, export_ssh_config, ssh_alias};
use termigator_core::import::ssh_config::parse_ssh_config;

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

fn stamp(v: &J) -> Stamp {
    let n = |i: usize| v[i].as_i64().expect("int");
    Stamp {
        year: n(0) as i32,
        month: n(1) as u32,
        day: n(2) as u32,
        hour: n(3) as u32,
        minute: n(4) as u32,
    }
}

#[test]
fn golden_alias() {
    let cases = common::golden("export_alias");
    assert!(!cases.is_empty());
    for c in &cases {
        let mut taken = HashSet::new();
        let labels: Vec<String> = c["labels"]
            .as_array()
            .expect("labels")
            .iter()
            .map(|l| l.as_str().expect("str").to_owned())
            .collect();
        let aliases: Vec<String> = labels.iter().map(|l| ssh_alias(l, &mut taken)).collect();
        let want: Vec<String> = c["expect"]["aliases"]
            .as_array()
            .expect("aliases")
            .iter()
            .map(|a| a.as_str().expect("str").to_owned())
            .collect();
        assert_eq!(aliases, want, "{}", common::name(c));
        let mut t: Vec<String> = taken.into_iter().collect();
        t.sort();
        let want_t: Vec<String> = c["expect"]["taken"]
            .as_array()
            .expect("taken")
            .iter()
            .map(|a| a.as_str().expect("str").to_owned())
            .collect();
        assert_eq!(t, want_t, "{} taken", common::name(c));
    }
}

#[test]
fn golden_config() {
    let cases = common::golden("export_config");
    assert!(!cases.is_empty());
    for c in &cases {
        let Value::Seq(conns) = from_golden(&c["conns"]) else {
            panic!("conns");
        };
        let got = export_ssh_config(&conns, stamp(&c["now"]));
        let e = &c["expect"];
        let name = common::name(c);
        assert_eq!(got.text, e["text"].as_str().expect("text"), "{name}");
        assert_eq!(
            got.exported as u64,
            e["exported"].as_u64().expect("n"),
            "{name}"
        );
        assert_eq!(
            got.skipped as u64,
            e["skipped"].as_u64().expect("n"),
            "{name}"
        );
    }
}

fn tmp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("termigator-export-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&d);
    fs::create_dir_all(&d).expect("tmp dir");
    d
}

fn sample(key: &str) -> Vec<Value> {
    let yaml = format!(
        r#"
- {{label: Prod web, host: 10.0.0.5, user: deploy, port: 2222, identity_file: {key},
   proxy_jump: bastion, local_forwards: ["8080:localhost:80", "127.0.0.1:5433:db:5432"],
   dynamic_forward: 1080, forward_agent: true, password: SECRET, group: prod,
   extra_ssh_args: ["-o", "Ciphers=aes256-gcm@openssh.com", "-o", "ControlMaster=no", "-v"]}}
- {{label: Prod web, host: 10.0.0.6, user: root, remote_command: tmux attach,
   x11_forwarding: true, x11_trusted: true, remote_locale: pl_PL.UTF-8}}
- {{label: desk, protocol: rdp, host: win}}
"#
    );
    match doc::load(&yaml).expect("yaml") {
        Value::Seq(s) => s,
        other => panic!("{other:?}"),
    }
}

const NOW: Stamp = Stamp {
    year: 2026,
    month: 11,
    day: 8,
    hour: 9,
    minute: 5,
};

fn exported(dir: &Path) -> (PathBuf, PathBuf) {
    let key = dir.join("id_test");
    fs::write(&key, "KEY\n").expect("key");
    let r = export_ssh_config(&sample(&key.to_string_lossy()), NOW);
    assert_eq!((r.exported, r.skipped), (2, 1));
    assert!(!r.text.contains("SECRET") && !r.text.contains("Password"));
    assert!(!r.text.contains("ControlMaster"));
    assert!(
        r.text
            .contains("# not expressible here, from extra ssh arguments: -v")
    );
    assert!(
        r.text
            .replace('"', "")
            .contains("SetEnv LANG=pl_PL.UTF-8 LC_ALL=pl_PL.UTF-8")
    );
    let f = dir.join("termigator.conf");
    fs::write(&f, &r.text).expect("write");
    (f, key)
}

#[test]
fn round_trip_through_importer() {
    let dir = tmp("rt");
    let (f, key) = exported(&dir);
    let res = parse_ssh_config(&f, &dir, "tester");
    let row = |label: &str| {
        res.connections
            .iter()
            .find(|r| r.get("label") == Some(&Value::Str(label.into())))
            .unwrap_or_else(|| panic!("no {label}"))
            .clone()
    };
    let s = |v: &Value, k: &str| v.get(k).cloned().unwrap_or(Value::Null);
    let web = row("Prod-web");
    assert_eq!(s(&web, "host"), Value::Str("10.0.0.5".into()));
    assert_eq!(s(&web, "user"), Value::Str("deploy".into()));
    assert_eq!(s(&web, "port"), Value::Int(2222));
    assert_eq!(
        s(&web, "identity_file"),
        Value::Str(key.to_string_lossy().into())
    );
    assert_eq!(s(&web, "proxy_jump"), Value::Str("bastion".into()));
    assert_eq!(s(&web, "forward_agent"), Value::Bool(true));
    assert_eq!(
        s(&web, "local_forwards"),
        Value::Seq(vec![
            Value::Str("8080:localhost:80".into()),
            Value::Str("127.0.0.1:5433:db:5432".into())
        ])
    );
    assert_eq!(s(&web, "dynamic_forward"), Value::Str("1080".into()));
    let Value::Seq(extra) = s(&web, "extra_ssh_args") else {
        panic!("extra_ssh_args");
    };
    assert_eq!(
        extra[..2],
        [
            Value::Str("-o".into()),
            Value::Str("Ciphers=aes256-gcm@openssh.com".into())
        ]
    );
    let second = row("Prod-web-2");
    assert_eq!(
        s(&second, "remote_command"),
        Value::Str("tmux attach".into())
    );
    assert_eq!(s(&second, "x11_forwarding"), Value::Bool(true));
}

fn ssh_g(f: &Path, alias: &str) -> Option<std::collections::HashMap<String, String>> {
    let out = Command::new("ssh")
        .args(["-G", "-F"])
        .arg(f)
        .arg(alias)
        .output()
        .ok()?;
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    Some(
        String::from_utf8_lossy(&out.stdout)
            .lines()
            .filter_map(|l| l.split_once(' '))
            .map(|(k, v)| (k.to_owned(), v.to_owned()))
            .collect(),
    )
}

#[test]
fn openssh_accepts_the_file() {
    let dir = tmp("ssh");
    let (f, _) = exported(&dir);
    for (alias, host, user, port) in [
        ("Prod-web", "10.0.0.5", "deploy", "2222"),
        ("Prod-web-2", "10.0.0.6", "root", "22"),
    ] {
        let Some(g) = ssh_g(&f, alias) else {
            live::skip("openssh_accepts_the_file", "ssh not installed");
            return;
        };
        assert_eq!(
            (
                g["hostname"].as_str(),
                g["user"].as_str(),
                g["port"].as_str()
            ),
            (host, user, port)
        );
    }
    let Some(g) = ssh_g(&f, "Prod-web") else {
        return;
    };
    assert_eq!(g["proxyjump"], "bastion");
    assert_eq!(g["forwardagent"], "yes");
    assert_eq!(g["ciphers"], "aes256-gcm@openssh.com");
}

#[test]
fn backup_zip_contents_and_mode() {
    let dir = tmp("zip");
    let a = dir.join("config.yaml");
    fs::write(&a, "window: {}\n").expect("write");
    let z = dir.join("out.zip");
    // An existing wider-mode target ends up 0600 too.
    fs::write(&z, "old").expect("write");
    fs::set_permissions(&z, fs::Permissions::from_mode(0o644)).expect("chmod");
    let names = build_backup_zip(
        &z,
        &[
            ("config.yaml", a.as_path()),
            ("session.yaml", &dir.join("missing.yaml")),
        ],
        "hi",
    )
    .expect("zip");
    assert_eq!(names, ["config.yaml", "README.txt"]);
    assert_eq!(
        fs::metadata(&z).expect("meta").permissions().mode() & 0o777,
        0o600
    );
    let mut zf = zip::ZipArchive::new(fs::File::open(&z).expect("open")).expect("archive");
    assert_eq!(zf.len(), 2);
    let mut read = |n: &str| {
        let mut s = String::new();
        let mut e = zf.by_name(n).expect("entry");
        assert_eq!(e.compression(), zip::CompressionMethod::Deflated);
        e.read_to_string(&mut s).expect("read");
        s
    };
    assert_eq!(read("config.yaml"), "window: {}\n");
    assert_eq!(read("README.txt"), "hi");
    // No note: no README.
    let names = build_backup_zip(&z, &[("config.yaml", a.as_path())], "").expect("zip");
    assert_eq!(names, ["config.yaml"]);
}

/// A line break in a value must not start a new directive.
#[test]
fn line_breaks_in_values_do_not_inject_directives() {
    let yaml = "- {label: a, host: h, user: u, group: \"g\\nProxyCommand evil\", remote_command: \"ls\\nProxyCommand evil\"}\n";
    let Value::Seq(conns) = doc::load(yaml).expect("yaml") else {
        panic!("seq")
    };
    let now = Stamp {
        year: 2024,
        month: 1,
        day: 2,
        hour: 3,
        minute: 4,
    };
    let out = export_ssh_config(&conns, now);
    assert_eq!(out.exported, 1);
    for line in out.text.lines() {
        assert!(
            !line.trim_start().starts_with("ProxyCommand"),
            "injected directive:\n{}",
            out.text
        );
    }
    assert!(out.text.contains("RemoteCommand \"ls ProxyCommand evil\""));
}
