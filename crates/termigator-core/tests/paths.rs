//! `paths` against the golden fixtures and the private-directory tests.
// Unix-only for now: uses modes, symlinks, sockets or sh stubs.
#![cfg(unix)]
mod common;

use std::ffi::OsString;
use std::fs;
use std::os::unix::fs::{MetadataExt, PermissionsExt, symlink};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

use termigator_core::paths::{APP_DIR, CUTOVER_APP_DIR, Layout, Paths, private_runtime_dir_in};

struct Tmp(PathBuf);

impl Tmp {
    fn new() -> Tmp {
        static N: AtomicU32 = AtomicU32::new(0);
        let p = std::env::temp_dir().join(format!(
            "termigator-paths-test-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&p).expect("create temp dir");
        Tmp(p.canonicalize().expect("canonicalize temp dir"))
    }
}

impl Drop for Tmp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn env_of<'a>(pairs: &'a [(&'a str, OsString)]) -> impl Fn(&str) -> Option<OsString> + 'a {
    move |k| pairs.iter().find(|(n, _)| *n == k).map(|(_, v)| v.clone())
}

fn uid() -> u32 {
    rustix::process::getuid().as_raw()
}

fn mode(p: &Path) -> u32 {
    fs::symlink_metadata(p).expect("lstat").mode() & 0o7777
}

fn rt(
    layout: Layout,
    env: &[(&str, OsString)],
    tmp: &Path,
    uid: u32,
    sub: &[&str],
) -> Option<PathBuf> {
    private_runtime_dir_in(layout, &env_of(env), tmp, uid, sub)
}

#[test]
fn golden_matches_golden() {
    let uid = uid();
    for case in common::golden("paths") {
        let name = common::name(&case);
        let tmp = Tmp::new();
        let t = tmp.0.to_str().expect("utf-8 tmp");
        let env: Vec<(String, OsString)> = case["env"]
            .as_object()
            .expect("env")
            .iter()
            .map(|(k, v)| {
                let v = v.as_str().expect("env value");
                (
                    k.clone(),
                    OsString::from(
                        v.replace("$ROOT", t)
                            .replace("$HOME", "/home/golden")
                            .replace("$TMP", t),
                    ),
                )
            })
            .collect();
        let lookup = |k: &str| env.iter().find(|(n, _)| n == k).map(|(_, v)| v.clone());
        let back = |p: &Path| p.to_str().expect("utf-8").replace(t, "$ROOT");
        match case["kind"].as_str().expect("kind") {
            "config" => {
                match case["root"].as_str().expect("root") {
                    "repo" => {
                        fs::create_dir_all(tmp.0.join("src/termigator")).expect("mkdir");
                        fs::write(tmp.0.join("requirements.txt"), "").expect("write");
                    }
                    "no_requirements" => {
                        fs::create_dir_all(tmp.0.join("src/termigator")).expect("mkdir")
                    }
                    _ => {}
                }
                let p = Paths::resolve(Layout::ACTIVE, &lookup, &tmp.0);
                let e = &case["expect"];
                // Repo-checkout mode is gone: its cases have no Rust counterpart.
                if e["is_repo"].as_bool().expect("is_repo") {
                    continue;
                }
                for (key, got) in [
                    ("repo_root", &p.repo_root),
                    ("config_dir", &p.config_dir),
                    ("config_path", &p.config_path),
                    ("session_path", &p.session_path),
                    ("config_example_path", &p.config_example_path),
                    ("session_example_path", &p.session_example_path),
                ] {
                    assert_eq!(back(got), e[key].as_str().expect(key), "{name}: {key}");
                }
            }
            "runtime" => {
                let xdg = tmp.0.join("xdg");
                let tmpdir = tmp.0.join("tmp");
                fs::create_dir_all(&tmpdir).expect("mkdir");
                if case["xdg_exists"].as_bool().expect("xdg_exists") {
                    fs::create_dir_all(&xdg).expect("mkdir");
                }
                let xdg_set = env
                    .iter()
                    .any(|(k, v)| k == "XDG_RUNTIME_DIR" && Path::new(v).is_dir());
                let base = if xdg_set {
                    xdg.join("termigator")
                } else {
                    tmpdir.join(format!("termigator-{uid}"))
                };
                match case["pre"].as_str().expect("pre") {
                    "loose" => {
                        fs::create_dir(&base).expect("mkdir");
                        fs::set_permissions(&base, fs::Permissions::from_mode(0o775))
                            .expect("chmod");
                    }
                    "symlink" => {
                        fs::create_dir(tmp.0.join("elsewhere")).expect("mkdir");
                        symlink(tmp.0.join("elsewhere"), &base).expect("symlink");
                    }
                    "file" => fs::write(&base, "").expect("write"),
                    _ => {}
                }
                let sub: Vec<&str> = case["sub"]
                    .as_array()
                    .expect("sub")
                    .iter()
                    .map(|s| s.as_str().expect("str"))
                    .collect();
                let got =
                    private_runtime_dir_in(Layout::ACTIVE, &lookup, &tmpdir, uid, &sub).map(|p| {
                        p.to_str()
                            .expect("utf-8")
                            .replace(t, "$TMP")
                            .replace(&format!("termigator-{uid}"), "termigator-$UID")
                    });
                assert_eq!(got.as_deref(), case["expect"].as_str(), "{name}: result");
                let dir_mode =
                    (base.is_dir() && !base.is_symlink()).then(|| format!("{:o}", mode(&base)));
                let want = case["base_mode"].as_str().map(str::to_owned);
                assert_eq!(dir_mode, want, "{name}: base mode");
            }
            k => panic!("{name}: unknown kind {k}"),
        }
    }
}

#[test]
fn golden_is_not_empty() {
    let cases = common::golden("paths");
    let n = |k: &str| {
        cases
            .iter()
            .filter(|c| c["kind"].as_str() == Some(k))
            .count()
    };
    assert!(n("config") > 10 && n("runtime") > 10);
}

#[test]
fn cutover_defaults_use_termigator() {
    assert_eq!(APP_DIR, CUTOVER_APP_DIR);
    assert_eq!(Layout::ACTIVE.app_dir, "termigator");

    let tmp = Tmp::new();
    fs::create_dir_all(tmp.0.join("src/termigator")).expect("mkdir");
    fs::write(tmp.0.join("requirements.txt"), "").expect("write");
    let env = [("HOME", OsString::from("/home/u"))];
    let p = Paths::resolve(Layout::ACTIVE, &env_of(&env), &tmp.0);
    assert_eq!(p.config_dir, Path::new("/home/u/.config/termigator"));
    // The explicit override still wins (and is what the repo's live config would be pointed at).
    let env = [
        ("HOME", OsString::from("/home/u")),
        ("TERMIGATOR_CONFIG_DIR", OsString::from("/o")),
    ];
    assert_eq!(
        Paths::resolve(Layout::ACTIVE, &env_of(&env), &tmp.0).session_path,
        Path::new("/o/session.yaml")
    );

    let t = Tmp::new();
    let d = rt(Layout::ACTIVE, &[], &t.0, uid(), &[]).expect("dir");
    assert_eq!(d, t.0.join(format!("termigator-{}", uid())));
}

// --- private directories -------------------------------------------------

#[test]
fn xdg_runtime_dir_is_used_and_tightened() {
    let tmp = Tmp::new();
    let base = tmp.0.join("termigator");
    fs::create_dir(&base).expect("mkdir");
    fs::set_permissions(&base, fs::Permissions::from_mode(0o775)).expect("chmod");
    let env = [("XDG_RUNTIME_DIR", tmp.0.clone().into_os_string())];
    let d = rt(Layout::ACTIVE, &env, &tmp.0, uid(), &["qss"]).expect("dir");
    assert_eq!(d, base.join("qss"));
    assert!(d.is_dir());
    assert_eq!(mode(&base), 0o700);
    assert_eq!(mode(&d), 0o700);
}

#[test]
fn shared_tmp_fallback_refuses_planted_dirs() {
    let tmp = Tmp::new();
    let me = uid();
    let target = tmp.0.join(format!("termigator-{me}"));
    fs::create_dir(tmp.0.join("elsewhere")).expect("mkdir");
    symlink(tmp.0.join("elsewhere"), &target).expect("symlink"); // someone planted a symlink
    assert_eq!(rt(Layout::ACTIVE, &[], &tmp.0, me, &[]), None);
    fs::remove_file(&target).expect("unlink");
    fs::create_dir(&target).expect("mkdir");
    // "Owned by someone else": the directory is ours, but we claim to be another uid.
    fs::create_dir(tmp.0.join(format!("termigator-{}", me + 1))).expect("mkdir");
    assert_eq!(rt(Layout::ACTIVE, &[], &tmp.0, me + 1, &[]), None);
    assert_eq!(rt(Layout::ACTIVE, &[], &tmp.0, me, &[]), Some(target));
}

#[test]
fn xdg_symlinked_base_and_unwritable_parent_give_none() {
    let tmp = Tmp::new();
    let xdg = tmp.0.join("xdg");
    fs::create_dir_all(tmp.0.join("elsewhere")).expect("mkdir");
    fs::create_dir(&xdg).expect("mkdir");
    symlink(tmp.0.join("elsewhere"), xdg.join("termigator")).expect("symlink");
    let env = [("XDG_RUNTIME_DIR", xdg.into_os_string())];
    assert_eq!(rt(Layout::ACTIVE, &env, &tmp.0, uid(), &["x"]), None);
    // A regular file where the base should be, and a tmp dir that is itself a file: any I/O error -> None.
    let f = tmp.0.join("file");
    fs::write(&f, "").expect("write");
    assert_eq!(rt(Layout::ACTIVE, &[], &f, uid(), &[]), None);
}
