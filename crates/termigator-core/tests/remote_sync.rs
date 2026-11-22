//! `termigator_core::remote::sync`: golden corpus (compat/golden/sync_*.json),
//! the path traversal table, and real directories for `scan_local`.
// Unix-only for now: uses modes, symlinks, sockets or sh stubs.
#![cfg(unix)]

mod common;

use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::fs;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};

use serde_json::Value as J;
use termigator_core::config::doc::Value;
use termigator_core::proto::ssh::Env;
use termigator_core::remote::listing::RemoteOps;
use termigator_core::remote::sync::{
    CopyJob, Direction, Node, OLDER_HERE, OLDER_THERE, OUTSIDE_MSG, RunPlan, SyncAction, SyncOp,
    Tree, local_target, parse_remote_tree, plan_sync, remote_tree_argv, safe_rel, scan_local,
};

fn fix_dir() -> String {
    common::repo_root()
        .join("crates/termigator-core/tests/fixtures/ssh")
        .canonicalize()
        .expect("fixtures dir")
        .to_string_lossy()
        .into_owned()
}

fn from_golden(v: &J, fix: &str) -> Value {
    if let Some((tag, p)) = common::tagged(v) {
        let s = || p.as_str().unwrap_or_default().to_owned();
        return match tag {
            "$int" => Value::Int(s().parse().expect("i64")),
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

fn replace_fix(b: &[u8], fix: &str) -> Vec<u8> {
    let (needle, mut out, mut i) = (b"@FIX@".as_slice(), Vec::new(), 0);
    while i < b.len() {
        if b[i..].starts_with(needle) {
            out.extend_from_slice(fix.as_bytes());
            i += needle.len();
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    out
}

fn tree_from(entries: &J) -> Tree {
    entries
        .as_array()
        .expect("entries")
        .iter()
        .map(|e| {
            (
                common::bytes(&e[0]),
                Node {
                    is_dir: e[1].as_str() == Some("d") || e[1].as_bool() == Some(true),
                    size: e[2].as_i64().expect("size"),
                    mtime: e[3].as_i64().expect("mtime"),
                },
            )
        })
        .collect()
}

#[test]
fn plan_matches_golden() {
    let cases = common::golden("sync_plan");
    assert!(cases.len() > 150);
    for case in &cases {
        let n = common::name(case);
        let dir = Direction::parse(case["direction"].as_str().expect("dir")).expect("direction");
        let got: Vec<(String, Vec<u8>, String, i64)> =
            plan_sync(&tree_from(&case["local"]), &tree_from(&case["remote"]), dir)
                .into_iter()
                .map(|a| {
                    // This departs from the golden fixtures here on purpose: its "differs" now says which side is older.
                    let reason = if a.replaces_newer() {
                        "differs"
                    } else {
                        a.reason
                    };
                    (a.op.as_str().to_owned(), a.rel, reason.to_owned(), a.size)
                })
                .collect();
        let want: Vec<(String, Vec<u8>, String, i64)> = case["expect"]
            .as_array()
            .expect("expect")
            .iter()
            .map(|a| {
                (
                    a[0].as_str().expect("op").to_owned(),
                    common::bytes(&a[1]),
                    a[2].as_str().expect("reason").to_owned(),
                    a[3].as_i64().expect("size"),
                )
            })
            .collect();
        assert_eq!(got, want, "{n}");
    }
}

#[test]
fn one_way_copy_over_a_newer_file_says_so() {
    let t = 1_700_000_000;
    let f = |s, m| Node {
        is_dir: false,
        size: s,
        mtime: m,
    };
    let local: Tree = BTreeMap::from([
        (b"old".to_vec(), f(1, t)),
        (b"new".to_vec(), f(1, t + 100)),
        (b"only".to_vec(), f(1, t)),
    ]);
    let remote: Tree =
        BTreeMap::from([(b"old".to_vec(), f(2, t + 100)), (b"new".to_vec(), f(2, t))]);
    let up = plan_sync(&local, &remote, Direction::Upload);
    let got: Vec<(&[u8], &str, bool, bool)> = up
        .iter()
        .map(|a| {
            (
                a.rel.as_slice(),
                a.reason,
                a.replaces_newer(),
                a.replaces_existing(),
            )
        })
        .collect();
    assert_eq!(
        got,
        [
            (b"new".as_slice(), "newer here", false, true),
            (b"old".as_slice(), OLDER_HERE, true, true),
            (b"only".as_slice(), "missing on the server", false, false),
        ]
    );
    let down = plan_sync(&local, &remote, Direction::Download);
    let new = down.iter().find(|a| a.rel == b"new").expect("new");
    assert_eq!((new.reason, new.replaces_newer()), (OLDER_THERE, true));
}

#[test]
fn plan_never_deletes_and_puts_folders_first() {
    let t = 1_700_000_000;
    let d = |m| Node {
        is_dir: true,
        size: 0,
        mtime: m,
    };
    let f = |s, m| Node {
        is_dir: false,
        size: s,
        mtime: m,
    };
    let local: Tree = BTreeMap::from([(b"a".to_vec(), f(1, t)), (b"d".to_vec(), d(t))]);
    let remote: Tree = BTreeMap::from([(b"z".to_vec(), f(1, t)), (b"rd".to_vec(), d(t))]);
    let plan = plan_sync(&local, &remote, Direction::Both);
    let kinds: Vec<bool> = plan
        .iter()
        .map(|a| matches!(a.op, SyncOp::MkdirLocal | SyncOp::MkdirRemote))
        .collect();
    assert_eq!(kinds, [true, true, false, false]);
}

#[test]
fn parse_matches_golden() {
    let cases = common::golden("sync_parse");
    assert!(cases.len() >= 20);
    for case in &cases {
        let n = common::name(case);
        let (tree, rejected) = parse_remote_tree(&common::bytes(&case["input"]));
        let want = tree_from(&case["expect"]["tree"]);
        assert_eq!(tree, want, "{n}");
        let want_rej: Vec<Vec<u8>> = case["expect"]["rejected"]
            .as_array()
            .expect("rejected")
            .iter()
            .map(common::bytes)
            .collect();
        assert_eq!(rejected, want_rej, "{n}");
    }
}

#[test]
fn paths_match_golden() {
    let cases = common::golden("sync_paths");
    assert!(cases.len() > 250);
    for case in &cases {
        let n = common::name(case);
        let a = &case["args"];
        match case["fn"].as_str().expect("fn") {
            "safe_rel" => {
                let want = case["expect"].as_bool().expect("bool");
                assert_eq!(safe_rel(&common::bytes(&a[0])), want, "{n}");
            }
            "local_target" => {
                let root = a[0].as_str().expect("root");
                let got = local_target(Path::new(root), &common::bytes(&a[1]));
                let want = case["expect"].as_str().map(PathBuf::from);
                assert_eq!(got, want, "{n}");
            }
            other => panic!("{n}: unknown fn {other}"),
        }
    }
}

#[test]
fn local_target_guard_table() {
    let root = Path::new("/home/u/sync");
    for bad in [
        "../x",
        "a/../../x",
        "/etc/passwd",
        "a//b",
        "./a",
        "",
        "..",
        "a/.",
        "sub/../../x",
    ] {
        assert_eq!(local_target(root, bad.as_bytes()), None, "{bad:?}");
        assert!(!safe_rel(bad.as_bytes()), "{bad:?}");
    }
    assert_eq!(local_target(root, b"a\0b"), None);
    assert!(safe_rel(b"..hidden") && safe_rel(b"a..b"));
    assert_eq!(
        local_target(root, b"sub/f"),
        Some(PathBuf::from("/home/u/sync/sub/f"))
    );
    // a relative root is anchored at the cwd
    let cwd = std::env::current_dir().expect("cwd");
    assert_eq!(local_target(Path::new("r"), b"x"), Some(cwd.join("r/x")));
}

#[test]
fn argv_matches_golden() {
    let fix = fix_dir();
    let cases = common::golden("sync_argv");
    assert!(cases.len() > 60);
    for case in &cases {
        let n = common::name(case);
        let conn = from_golden(&case["conn"], &fix);
        let launch = from_golden(&case["launch"], &fix);
        let tools: Vec<String> = case["which"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(J::as_str)
            .map(str::to_owned)
            .collect();
        let env = Env {
            which: Box::new(move |t| {
                tools
                    .iter()
                    .any(|x| x == t)
                    .then(|| format!("/usr/bin/{t}"))
            }),
            control_dir: case["control_dir"].as_str().map(str::to_owned),
            home: Some(fix.clone()),
        };
        let ops = RemoteOps {
            conn: &conn,
            launch: &launch,
            env: &env,
        };
        let got = remote_tree_argv(&ops, &common::bytes(&case["root"]));
        if case.get("error").is_some() {
            assert_eq!(
                got,
                Err(case["message"].as_str().expect("msg").to_owned()),
                "{n}"
            );
        } else {
            let want: Vec<Vec<u8>> = case["expect"]
                .as_array()
                .expect("expect")
                .iter()
                .map(|a| replace_fix(&common::bytes(a), &fix))
                .collect();
            assert_eq!(got, Ok(want), "{n}");
        }
    }
}

fn tmp(name: &str) -> PathBuf {
    let p = std::env::temp_dir().join(format!("termigator-sync-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&p);
    fs::create_dir_all(&p).expect("mkdir");
    p
}

#[test]
fn scan_local_lists_files_and_folders_but_not_symlinks() {
    let root = tmp("scan");
    fs::create_dir_all(root.join("sub/deep")).expect("mkdir");
    fs::write(root.join("a.txt"), b"hello").expect("write");
    fs::write(root.join("sub/deep/b c.txt"), b"xy").expect("write");
    let outside = tmp("scan-outside");
    fs::write(outside.join("secret"), b"s").expect("write");
    std::os::unix::fs::symlink(&outside, root.join("linkdir")).expect("symlink");
    std::os::unix::fs::symlink(outside.join("secret"), root.join("linkfile")).expect("symlink");
    let name = OsStr::from_bytes(b"caf\xe9");
    fs::write(root.join(name), b"1").expect("write non-utf8");

    let tree = scan_local(&root);
    let keys: Vec<&[u8]> = tree.keys().map(Vec::as_slice).collect();
    assert_eq!(
        keys,
        [
            b"a.txt".as_slice(),
            b"caf\xe9",
            b"sub",
            b"sub/deep",
            b"sub/deep/b c.txt"
        ]
    );
    assert_eq!(tree[b"a.txt".as_slice()].size, 5);
    assert!(tree[b"sub".as_slice()].is_dir && tree[b"sub".as_slice()].size == 0);
    assert!(scan_local(&root.join("a.txt")).is_empty());
    assert!(scan_local(&root.join("missing")).is_empty());
    let _ = fs::remove_dir_all(&root);
    let _ = fs::remove_dir_all(&outside);
}

fn action(op: SyncOp, rel: &str) -> SyncAction {
    SyncAction {
        op,
        rel: rel.as_bytes().to_vec(),
        reason: "",
        size: 0,
    }
}

#[test]
fn run_plan_collects_dirs_and_copies() {
    let todo = [
        action(SyncOp::MkdirLocal, "ld"),
        action(SyncOp::MkdirRemote, "rd"),
        action(SyncOp::Upload, "sub/new here.txt"),
        action(SyncOp::Upload, "top.txt"),
        action(SyncOp::Download, "only there.txt"),
        action(SyncOp::Conflict, "clash"),
    ];
    let plan = RunPlan::prepare(Path::new("/l"), b"/srv/data", &todo).expect("plan");
    assert_eq!(plan.local_dirs, [PathBuf::from("/l/ld")]);
    assert_eq!(
        plan.remote_dirs,
        [b"/srv/data/rd".to_vec(), b"/srv/data/sub".to_vec()]
    );
    assert_eq!(
        plan.copies,
        [
            CopyJob {
                upload: true,
                local: PathBuf::from("/l/sub/new here.txt"),
                remote: b"/srv/data/sub/new here.txt".to_vec(),
                label: b"sub/new here.txt".to_vec()
            },
            CopyJob {
                upload: true,
                local: PathBuf::from("/l/top.txt"),
                remote: b"/srv/data/top.txt".to_vec(),
                label: b"top.txt".to_vec()
            },
            CopyJob {
                upload: false,
                local: PathBuf::from("/l/only there.txt"),
                remote: b"/srv/data/only there.txt".to_vec(),
                label: b"only there.txt".to_vec()
            },
        ]
    );
}

#[test]
fn run_plan_refuses_everything_when_one_path_escapes() {
    let todo = [
        action(SyncOp::MkdirLocal, "ok"),
        action(SyncOp::Download, "fine.txt"),
        action(SyncOp::Download, "../../evil.txt"),
    ];
    assert_eq!(
        RunPlan::prepare(Path::new("/l"), b"/r", &todo),
        Err(OUTSIDE_MSG.to_owned())
    );
}

#[test]
fn run_plan_mkdir_argv_quotes_and_is_none_without_dirs() {
    let conn = Value::Map(vec![
        (Value::Str("host".into()), Value::Str("srv".into())),
        (Value::Str("user".into()), Value::Str("root".into())),
    ]);
    let launch = Value::Map(vec![]);
    let env = Env {
        which: Box::new(|_| None),
        control_dir: None,
        home: None,
    };
    let ops = RemoteOps {
        conn: &conn,
        launch: &launch,
        env: &env,
    };
    let todo = [action(SyncOp::Upload, "a b/c")];
    let plan = RunPlan::prepare(Path::new("/l"), b"/r", &todo).expect("plan");
    let argv = plan.mkdir_argv(&ops).expect("dirs").expect("argv");
    assert_eq!(
        argv.last().map(Vec::as_slice),
        Some(b"mkdir -p -- '/r/a b'".as_slice())
    );
    let none =
        RunPlan::prepare(Path::new("/l"), b"/r", &[action(SyncOp::Upload, "c")]).expect("plan");
    assert!(none.mkdir_argv(&ops).is_none());
}
