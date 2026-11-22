//! Config load / merge / strip / save against the golden fixtures (compat/golden/config_*.json)
//! and the SaveConfig, MergeDefaults and StripDefaults cases.
// Unix-only for now: uses modes, symlinks, sockets or sh stubs.
#![cfg(unix)]
mod common;

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value as J;
use termigator_core::config::doc::{Value, dump, load as yaml_load};
use termigator_core::config::save::{
    SESSION_HEADER, load, merge_defaults, save, strip_defaults_for_save, tighten_modes,
    write_atomic,
};
use termigator_core::paths::Paths;

const FILES: [&str; 3] = [
    "cfg/config.yaml",
    "cfg/session.yaml",
    "cfg/config.example.yaml",
];

fn from_golden(v: &J) -> Value {
    if let Some((tag, p)) = common::tagged(v) {
        let s = || p.as_str().unwrap_or_default().to_owned();
        return match tag {
            "$int" => Value::Int(s().parse().expect("i64")),
            "$float" => Value::Float(s().parse().expect("f64")),
            "$timestamp" => Value::Timestamp(s()),
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

/// A fresh case dir: `cfg/` is the config dir, `root/` the repo root.
fn case_dir(name: &str) -> (PathBuf, Paths) {
    let base = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join("config_save")
        .join(name);
    let _ = fs::remove_dir_all(&base);
    fs::create_dir_all(base.join("root")).expect("mkdir");
    let paths = paths_in(&base);
    (base, paths)
}

fn paths_in(base: &Path) -> Paths {
    let cfg = base.join("cfg");
    let root = base.join("root");
    Paths {
        repo_root: root.clone(),
        config_path: cfg.join("config.yaml"),
        session_path: cfg.join("session.yaml"),
        config_example_path: cfg.join("config.example.yaml"),
        session_example_path: cfg.join("session.example.yaml"),
        config_dir: cfg,
    }
}

fn write_files(base: &Path, files: &J) {
    for (rel, text) in files.as_object().expect("files") {
        let p = base.join(rel);
        fs::create_dir_all(p.parent().expect("parent")).expect("mkdir");
        fs::write(p, text.as_str().expect("text")).expect("write");
    }
}

fn read(p: &Path) -> String {
    fs::read_to_string(p).unwrap_or_else(|e| panic!("{}: {e}", p.display()))
}

fn yaml(text: &str) -> Value {
    yaml_load(text).expect("yaml")
}

/// Hidden files (temp leftovers) in a directory.
fn dot_files(dir: &Path) -> Vec<String> {
    fs::read_dir(dir)
        .expect("readdir")
        .map(|e| e.expect("entry").file_name().to_string_lossy().into_owned())
        .filter(|n| n.starts_with('.'))
        .collect()
}

#[test]
fn golden_merge_and_strip() {
    for case in common::golden("config_merge") {
        let got = merge_defaults(&from_golden(&case["input"]));
        assert_eq!(got, from_golden(&case["expect"]), "{}", common::name(&case));
    }
    for case in common::golden("config_strip") {
        let got = strip_defaults_for_save(&from_golden(&case["input"]));
        assert_eq!(got, from_golden(&case["expect"]), "{}", common::name(&case));
    }
}

#[test]
fn golden_load() {
    for case in common::golden("config_load") {
        let name = common::name(&case);
        let (base, paths) = case_dir(&format!("load_{name}"));
        write_files(&base, &case["files"]);
        let got = load(&paths).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(got, from_golden(&case["expect"]), "{name}");
        for rel in FILES {
            let after = fs::read_to_string(base.join(rel)).ok();
            assert_eq!(
                after.as_deref(),
                case["after"][rel].as_str(),
                "{name}: {rel}"
            );
        }
    }
}

/// Runs every golden save case; returns the case dirs for the cross-read.
fn run_golden_save(prefix: &str) -> Vec<(String, PathBuf, Paths)> {
    let mut dirs = Vec::new();
    for case in common::golden("config_save") {
        let name = common::name(&case).to_owned();
        let (base, paths) = case_dir(&format!("{prefix}_{name}"));
        write_files(&base, &case["files"]);
        let vault = case["vault_configured"].as_bool().expect("flag");
        save(&paths, &from_golden(&case["cfg"]), vault).unwrap_or_else(|e| panic!("{name}: {e}"));
        for rel in FILES {
            let after = fs::read_to_string(base.join(rel)).ok();
            if rel == "cfg/config.yaml" && case["config_edited"] == J::Bool(true) {
                // The fixtures were recorded through ruamel, Rust edits through yaml-edit: same content.
                let after = after.unwrap_or_else(|| panic!("{name}: config.yaml missing"));
                assert!(
                    yaml(&after).loose_eq(&from_golden(&case["config_loaded"])),
                    "{name}: {after}"
                );
            } else {
                assert_eq!(
                    after.as_deref(),
                    case["after"][rel].as_str(),
                    "{name}: {rel}"
                );
            }
        }
        assert_eq!(dot_files(&paths.config_dir), Vec::<String>::new(), "{name}");
        dirs.push((name, base, paths));
    }
    dirs
}

#[test]
fn golden_save() {
    run_golden_save("save");
}

fn cfg_with_conns(extra: &str) -> Value {
    merge_defaults(&yaml(&format!(
        "{extra}connections:\n- host: h\n  user: u\n"
    )))
}

#[test]
fn saved_config_is_minimal() {
    let (_, p) = case_dir("minimal");
    save(&p, &cfg_with_conns("window: {width: 1234}\n"), false).expect("save");
    let text = read(&p.config_path);
    assert!(text.contains("width: 1234"));
    assert!(!text.contains("template:") && !text.contains("ssh_keepalive"));
    assert!(read(&p.session_path).contains("host: h"));
}

#[test]
fn noop_save_preserves_comments() {
    let (_, p) = case_dir("noop");
    let cfg = cfg_with_conns("window: {width: 1234}\n");
    save(&p, &cfg, false).expect("save");
    let c = "# my precious comment\n".to_owned() + &read(&p.config_path);
    let s = "# session note\n".to_owned() + &read(&p.session_path);
    fs::write(&p.config_path, &c).expect("write");
    fs::write(&p.session_path, &s).expect("write");
    save(&p, &cfg, false).expect("save");
    assert_eq!(read(&p.config_path), c);
    assert_eq!(read(&p.session_path), s);
}

#[test]
fn session_untouched_when_only_config_changes() {
    let (_, p) = case_dir("only_config");
    save(&p, &cfg_with_conns(""), false).expect("save");
    let s = "# keep me\n".to_owned() + &read(&p.session_path);
    fs::write(&p.session_path, &s).expect("write");
    save(&p, &cfg_with_conns("window: {width: 777}\n"), false).expect("save");
    assert_eq!(read(&p.session_path), s);
    assert!(read(&p.config_path).contains("width: 777"));
}

#[test]
fn shared_lists_written_without_yaml_anchors() {
    let (_, p) = case_dir("anchors");
    // These entries share one list object (shallow duplicate).
    let cfg = merge_defaults(&yaml(
        "connections:\n- {host: a, user: u, tags: &t [prod, db]}\n- {host: b, user: u, tags: *t}\n",
    ));
    save(&p, &cfg, false).expect("save");
    let text = read(&p.session_path);
    assert!(text.starts_with(SESSION_HEADER));
    assert!(!text.contains("&id") && !text.contains("*id"));
    assert_eq!(text.matches("- prod").count(), 2);
}

#[test]
fn comments_kept_on_value_change() {
    let (_, p) = case_dir("comments");
    fs::create_dir_all(&p.config_dir).expect("mkdir");
    fs::write(
        &p.config_path,
        "# top comment\nwindow:\n  width: 1234  # my width\n  list_width: 300\n",
    )
    .expect("write");
    save(
        &p,
        &merge_defaults(&yaml("window: {width: 1234, list_width: 400}\n")),
        false,
    )
    .expect("save");
    let text = read(&p.config_path);
    assert!(
        text.contains("# top comment") && text.contains("# my width"),
        "{text}"
    );
    assert!(text.contains("list_width: 400"), "{text}");
}

#[test]
fn save_through_symlink_keeps_link() {
    let (base, p) = case_dir("symlink");
    fs::create_dir_all(&p.config_dir).expect("mkdir");
    let real = base.join("real-session.yaml");
    fs::write(&real, "[]\n").expect("write");
    fs::set_permissions(&real, fs::Permissions::from_mode(0o640)).expect("chmod");
    std::os::unix::fs::symlink(&real, &p.session_path).expect("symlink");
    save(&p, &cfg_with_conns(""), false).expect("save");
    assert!(
        fs::symlink_metadata(&p.session_path)
            .expect("lstat")
            .file_type()
            .is_symlink()
    );
    assert!(read(&real).contains("host: h"));
    assert_eq!(
        fs::metadata(&real).expect("stat").permissions().mode() & 0o777,
        0o640
    );
    assert!(dot_files(&base).is_empty());
}

#[test]
fn new_files_are_private_and_modes_kept() {
    let (base, _) = case_dir("modes");
    let f = base.join("new.yaml");
    write_atomic(&f, "a: 1\n").expect("write");
    assert_eq!(
        fs::metadata(&f).expect("stat").permissions().mode() & 0o777,
        0o600
    );
    fs::set_permissions(&f, fs::Permissions::from_mode(0o644)).expect("chmod");
    write_atomic(&f, "a: 2\n").expect("write");
    assert_eq!(read(&f), "a: 2\n");
    assert_eq!(
        fs::metadata(&f).expect("stat").permissions().mode() & 0o777,
        0o644
    );
}

#[test]
fn vault_keeps_plaintext_passwords_off_disk() {
    let (_, p) = case_dir("vault");
    let cfg = merge_defaults(&yaml(
        "vault: {salt: c2FsdA==, check: 'enc:v1:Y2hr'}\nconnections:\n- {host: a, user: u, password: plain-secret}\n- {host: b, user: u, password: 'enc:v1:c2VhbGVk'}\n",
    ));
    save(&p, &cfg, true).expect("save");
    let ses = read(&p.session_path);
    assert!(
        !ses.contains("plain-secret") && ses.contains("enc:v1:c2VhbGVk"),
        "{ses}"
    );
    let loaded = load(&p).expect("load");
    assert_eq!(
        loaded.get("vault").and_then(|v| v.get("salt")),
        Some(&Value::Str("c2FsdA==".into()))
    );
    // Without a vault, plain text stays as before.
    save(&p, &cfg, false).expect("save");
    assert!(read(&p.session_path).contains("password: plain-secret"));
}

/// A write that dies half-way (here: RLIMIT_FSIZE, as a full disk would) leaves the
/// previous files and no temp file. The limit is process-wide, so it runs in a child.
#[test]
fn failed_write_keeps_previous_files() {
    const CHILD: &str = "TERMIGATOR_TEST_FSIZE_CHILD";
    if let Some(dir) = std::env::var_os(CHILD) {
        let p = paths_in(Path::new(&dir));
        rustix::process::setrlimit(
            rustix::process::Resource::Fsize,
            rustix::process::Rlimit {
                current: Some(8),
                maximum: Some(8),
            },
        )
        .expect("setrlimit");
        // SAFETY: SIG_IGN installs no handler code; this child process does nothing else.
        unsafe {
            nix::sys::signal::signal(
                nix::sys::signal::Signal::SIGXFSZ,
                nix::sys::signal::SigHandler::SigIgn,
            )
        }
        .expect("signal");
        let new = merge_defaults(&yaml(
            "window: {width: 2222}\nconnections: [{host: new, user: u}]\n",
        ));
        let err = save(&p, &new, false).expect_err("write must fail");
        println!("child error: {err}");
        return;
    }
    let (base, p) = case_dir("enospc");
    let cfg = cfg_with_conns("window: {width: 1111}\n");
    save(&p, &cfg, false).expect("save");
    let before_ses = read(&p.session_path);
    let before_cfg = read(&p.config_path);
    let out = Command::new(std::env::current_exe().expect("exe"))
        .args([
            "--exact",
            "failed_write_keeps_previous_files",
            "--nocapture",
        ])
        .env(CHILD, &base)
        .output()
        .expect("child");
    let log = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success() && log.contains("child error:"),
        "{log}"
    );
    eprintln!("{log}");
    assert_eq!(read(&p.session_path), before_ses);
    assert_eq!(read(&p.config_path), before_cfg);
    assert!(dot_files(&p.config_dir).is_empty(), "{}", base.display());
    // Unchanged config: the dumps above really are what a fresh save writes.
    assert_eq!(before_cfg, dump(&yaml("window: {width: 1111}\n")));
}

/// The config directory becomes 0700 and its YAML files 0600. A copied setuid bit never survives a save.
#[test]
fn tighten_modes_closes_the_config_dir() {
    let (_, p) = case_dir("tighten");
    fs::create_dir_all(&p.config_dir).expect("mkdir");
    for f in [&p.config_path, &p.session_path] {
        fs::write(f, "{}\n").expect("write");
        fs::set_permissions(f, fs::Permissions::from_mode(0o644)).expect("chmod");
    }
    fs::set_permissions(&p.config_dir, fs::Permissions::from_mode(0o755)).expect("chmod");
    let mode = |f: &Path| fs::metadata(f).expect("stat").permissions().mode() & 0o7777;
    tighten_modes(&p);
    assert_eq!(
        (
            mode(&p.config_dir),
            mode(&p.config_path),
            mode(&p.session_path)
        ),
        (0o700, 0o600, 0o600)
    );
    fs::set_permissions(&p.config_path, fs::Permissions::from_mode(0o4644)).expect("chmod");
    write_atomic(&p.config_path, "a: 1\n").expect("write");
    assert_eq!(mode(&p.config_path), 0o644);
}
