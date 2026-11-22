//! `termigator_core::remote::listing` against the golden corpus
//! (compat/golden/listing_*.json), the non-OSC7, non-scp logic, and
//! the listing/mkdir/rename/delete/chmod commands run for real by a local `sh` on non-UTF-8 names.
// Unix-only for now: uses modes, symlinks, sockets or sh stubs.
#![cfg(unix)]

mod common;

use std::ffi::OsString;
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value as J;
use termigator_core::config::doc::Value;
use termigator_core::proto::ssh::Env;
use termigator_core::remote::listing::{
    LS_MARKER, RemoteEntry, RemoteOps, browse_conn, display_name, edit_cache_path, host_key,
    human_size, is_auth_failure, mode_bits, parse_ls_output, quote_bytes, remote_join,
    remote_parent, with_typed_password,
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

fn case_env(case: &J, fix: &str) -> Env {
    let tools: Vec<String> = case["which"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(J::as_str)
        .map(str::to_owned)
        .collect();
    Env {
        which: Box::new(move |n| {
            tools
                .iter()
                .any(|t| t == n)
                .then(|| format!("/usr/bin/{n}"))
        }),
        control_dir: case["control_dir"].as_str().map(str::to_owned),
        home: Some(fix.to_owned()),
    }
}

fn byte_list(v: &J) -> Vec<Vec<u8>> {
    v.as_array()
        .expect("list")
        .iter()
        .map(common::bytes)
        .collect()
}

/// The golden files write the fixture directory as `@FIX@`.
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

#[test]
fn argv_matches_golden() {
    let fix = fix_dir();
    let cases = common::golden("listing_argv");
    assert!(cases.len() > 400);
    for case in &cases {
        let n = common::name(case);
        let conn = from_golden(&case["conn"], &fix);
        let launch = from_golden(&case["launch"], &fix);
        let env = case_env(case, &fix);
        let ops = RemoteOps {
            conn: &conn,
            launch: &launch,
            env: &env,
        };
        let args = byte_list(&case["args"]);
        let got = match case["fn"].as_str().expect("fn") {
            "exec" => ops.exec_argv(&args[0]),
            "list" => ops.list_dir_argv(&args[0]),
            "mkdir" => ops.mkdir_argv(&args[0]),
            "rename" => ops.rename_argv(&args[0], &args[1]),
            "delete" => ops.delete_argv(&args[0]),
            "delete_many" => ops.delete_many_argv(&args),
            "chmod" => ops.chmod_argv(
                &args,
                case["mode"].as_i64().expect("mode"),
                case["recursive"].as_bool().expect("recursive"),
            ),
            other => panic!("{n}: unknown fn {other}"),
        };
        if case.get("error").is_some() {
            let msg = case["message"].as_str().expect("message");
            assert_eq!(got, Err(msg.to_owned()), "{n}");
        } else {
            let want: Vec<Vec<u8>> = byte_list(&case["expect"])
                .into_iter()
                .map(|a| replace_fix(&a, &fix))
                .collect();
            assert_eq!(got, Ok(want), "{n}");
        }
    }
}

fn entry_from_golden(e: &J) -> RemoteEntry {
    let text = |k: &str| String::from_utf8(common::bytes(&e[k])).expect("utf-8 field");
    RemoteEntry {
        name: common::bytes(&e["name"]),
        is_dir: e["is_dir"].as_bool().expect("is_dir"),
        is_link: e["is_link"].as_bool().expect("is_link"),
        size: e["size"].as_i64().expect("size"),
        mode: text("mode"),
        mtime: text("mtime"),
        link_target: common::bytes(&e["link_target"]),
        owner: text("owner"),
    }
}

#[test]
fn parse_matches_golden() {
    let cases = common::golden("listing_parse");
    assert!(cases.len() > 200);
    for case in &cases {
        let n = common::name(case);
        let got = parse_ls_output(&common::bytes(&case["input"]));
        if case.get("error").is_some() {
            let msg = case["message"].as_str().expect("message");
            assert_eq!(got, Err(msg.to_owned()), "{n}");
        } else {
            let want = (
                common::bytes(&case["expect"]["cwd"]),
                case["expect"]["entries"]
                    .as_array()
                    .expect("entries")
                    .iter()
                    .map(entry_from_golden)
                    .collect::<Vec<_>>(),
            );
            assert_eq!(got, Ok(want), "{n}");
        }
    }
}

#[test]
fn misc_matches_golden() {
    let fix = fix_dir();
    let cases = common::golden("listing_misc");
    assert!(cases.len() > 350);
    for case in &cases {
        let n = common::name(case);
        let a = &case["args"];
        let want = &case["expect"];
        match case["fn"].as_str().expect("fn") {
            "mode_bits" => {
                let got = mode_bits(a[0].as_str().expect("mode"));
                assert_eq!(i64::from(got), want.as_i64().expect("int"), "{n}");
            }
            "human_size" => {
                let (_, v) = common::tagged(&a[0]).expect("$int");
                let got = human_size(v.as_str().expect("digits").parse().expect("i64"));
                assert_eq!(got, want.as_str().expect("str"), "{n}");
            }
            "display_name" => {
                assert_eq!(
                    display_name(&common::bytes(&a[0])),
                    want.as_str().expect("str"),
                    "{n}"
                );
            }
            "host_key" => {
                let conn = from_golden(&a[0], &fix);
                assert_eq!(host_key(&conn), want.as_str().expect("str"), "{n}");
            }
            "edit_cache_path" => {
                let conn = from_golden(&a[0], &fix);
                let root = PathBuf::from(a[2].as_str().expect("root"));
                let got = edit_cache_path(&conn, &common::bytes(&a[1]), Some(&root));
                assert_eq!(got.as_os_str().as_bytes(), common::bytes(want), "{n}");
            }
            "remote_join" => {
                let got = remote_join(&common::bytes(&a[0]), &common::bytes(&a[1]));
                assert_eq!(got, common::bytes(want), "{n}");
            }
            "remote_parent" => {
                assert_eq!(
                    remote_parent(&common::bytes(&a[0])),
                    common::bytes(want),
                    "{n}"
                );
            }
            "browse_conn" => {
                let got = browse_conn(&from_golden(&a[0], &fix));
                assert_eq!(got, from_golden(want, &fix), "{n}");
            }
            other => panic!("{n}: unknown fn {other}"),
        }
    }
}

// ---------------------------------------------------------------------------- real sh

fn scratch(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("remote_listing_{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch dir");
    dir
}

fn os(b: &[u8]) -> OsString {
    OsString::from_vec(b.to_vec())
}

fn path_bytes(dir: &Path, name: &[u8]) -> Vec<u8> {
    [dir.as_os_str().as_bytes(), b"/", name].concat()
}

fn ops_env() -> Env {
    Env {
        which: Box::new(|_| None),
        control_dir: None,
        home: None,
    }
}

/// Run the remote command of `argv` with a local `sh -c` (what the remote login shell would do).
fn run_remote(argv: &[Vec<u8>], prefix: &[u8], env: &[(&str, &str)]) -> (Vec<u8>, Vec<u8>, bool) {
    let cmd = [prefix, argv.last().expect("remote command")].concat();
    let mut c = Command::new("sh");
    c.arg("-c").arg(os(&cmd));
    for (k, v) in env {
        c.env(k, v);
    }
    let out = c.output().expect("sh");
    (out.stdout, out.stderr, out.status.success())
}

fn conn() -> Value {
    Value::Map(vec![
        (Value::Str("host".into()), Value::Str("srv".into())),
        (Value::Str("user".into()), Value::Str("root".into())),
    ])
}

#[test]
fn real_listing_keeps_non_utf8_newline_and_arrow_names() {
    let dir = scratch("list");
    let names: [&[u8]; 6] = [
        b"caf\xe9",
        b"new\nline",
        b"a -> b",
        b"my file.txt",
        b"it's \"q\"",
        b"plain",
    ];
    for n in names {
        std::fs::write(os(&path_bytes(&dir, n)), b"x").expect("file");
    }
    std::os::unix::fs::symlink("plain", os(&path_bytes(&dir, b"link -> \xff"))).expect("link");
    std::fs::create_dir(os(&path_bytes(&dir, b"d\xe9"))).expect("dir");

    let (conn, launch, env) = (conn(), Value::Null, ops_env());
    let ops = RemoteOps {
        conn: &conn,
        launch: &launch,
        env: &env,
    };
    let argv = ops.list_dir_argv(dir.as_os_str().as_bytes()).expect("argv");
    // `.bashrc` noise before the marker and a TIME_STYLE that used to eat the first word.
    let (out, _, ok) = run_remote(&argv, b"echo noise; ", &[("TIME_STYLE", "long-iso")]);
    assert!(ok);
    assert!(out.starts_with(b"noise\n"));
    let (cwd, entries) = parse_ls_output(&out).expect("parse");
    assert_eq!(
        cwd,
        std::fs::canonicalize(&dir)
            .expect("canon")
            .as_os_str()
            .as_bytes()
    );
    let mut got: Vec<&[u8]> = entries.iter().map(|e| e.name.as_slice()).collect();
    got.sort();
    let mut want: Vec<&[u8]> = names.to_vec();
    want.extend([b"link -> \xff".as_slice(), b"d\xe9"]);
    want.sort();
    assert_eq!(got, want);
    let link = entries
        .iter()
        .find(|e| e.name == b"link -> \xff")
        .expect("link");
    assert!(link.is_link);
    assert_eq!(link.link_target, b"plain");
    assert!(
        entries
            .iter()
            .find(|e| e.name == b"d\xe9")
            .expect("dir")
            .is_dir
    );
    assert_eq!(display_name(b"caf\xe9"), "caf\u{fffd}");
    assert!(
        out.windows(LS_MARKER.len())
            .any(|w| w == LS_MARKER.as_bytes())
    );
}

#[test]
fn real_stat_gives_size_and_mtime() {
    let dir = scratch("stat");
    let file = dir.join("it's.txt");
    std::fs::write(&file, b"12345").expect("file");
    let (conn, launch, env) = (conn(), Value::Null, ops_env());
    let ops = RemoteOps {
        conn: &conn,
        launch: &launch,
        env: &env,
    };
    let argv = ops.stat_argv(file.as_os_str().as_bytes()).expect("argv");
    let (out, _, ok) = run_remote(&argv, b"", &[]);
    assert!(ok);
    let text = String::from_utf8_lossy(&out);
    let mut it = text.split_whitespace();
    assert_eq!(it.next(), Some("5"));
    assert!(
        it.next().is_some_and(|m| m.parse::<u64>().is_ok()),
        "{text}"
    );
}

#[test]
fn real_tilde_expands_and_missing_dir_fails() {
    let home = scratch("home");
    std::fs::create_dir_all(home.join("my dir")).expect("dirs");
    let home = std::fs::canonicalize(&home).expect("canon");
    let (conn, launch, env) = (conn(), Value::Null, ops_env());
    let ops = RemoteOps {
        conn: &conn,
        launch: &launch,
        env: &env,
    };
    let home_s = home.to_str().expect("utf-8");
    for (typed, want) in [
        (&b"~"[..], home.clone()),
        (b"~/", home.clone()),
        (b"~/my dir", home.join("my dir")),
    ] {
        let argv = ops.list_dir_argv(typed).expect("argv");
        let (out, _, ok) = run_remote(&argv, b"", &[("HOME", home_s)]);
        assert!(ok);
        let (cwd, _) = parse_ls_output(&out).expect("parse");
        assert_eq!(cwd, want.as_os_str().as_bytes(), "{typed:?}");
    }
    let argv = ops.list_dir_argv(b"~/nope").expect("argv");
    assert!(!run_remote(&argv, b"", &[("HOME", home_s)]).2);
}

#[test]
fn real_rename_never_clobbers_and_moves_non_utf8() {
    let d = scratch("rename");
    let p = |n: &[u8]| path_bytes(&d, n);
    std::fs::create_dir(os(&p(b"dir"))).expect("dir");
    std::fs::write(os(&p(b"a.txt")), "A").expect("a");
    std::fs::write(os(&p(b"b.txt")), "precious").expect("b");
    std::fs::write(os(&p(b"it's c\xe9")), "C").expect("c");
    let (conn, launch, env) = (conn(), Value::Null, ops_env());
    let ops = RemoteOps {
        conn: &conn,
        launch: &launch,
        env: &env,
    };
    let run =
        |a: &[u8], b: &[u8]| run_remote(&ops.rename_argv(&p(a), &p(b)).expect("argv"), b"", &[]);

    let (_, err, ok) = run(b"a.txt", b"b.txt");
    assert!(!ok && String::from_utf8_lossy(&err).contains("already exists"));
    assert_eq!(std::fs::read(os(&p(b"b.txt"))).expect("b"), b"precious");
    assert!(!run(b"it's c\xe9", b"dir").2);
    assert_eq!(std::fs::read_dir(os(&p(b"dir"))).expect("dir").count(), 0);
    assert!(run(b"it's c\xe9", b"new \xe9 name").2);
    assert_eq!(
        std::fs::read(os(&p(b"new \xe9 name"))).expect("moved"),
        b"C"
    );
}

#[test]
fn real_mkdir_delete_chmod_hit_the_right_non_utf8_files() {
    let d = scratch("ops");
    let p = |n: &[u8]| path_bytes(&d, n);
    let (conn, launch, env) = (conn(), Value::Null, ops_env());
    let ops = RemoteOps {
        conn: &conn,
        launch: &launch,
        env: &env,
    };

    assert!(
        run_remote(
            &ops.mkdir_argv(&p(b"n\xe9w dir/sub")).expect("argv"),
            b"",
            &[]
        )
        .2
    );
    assert!(Path::new(&os(&p(b"n\xe9w dir/sub"))).is_dir());

    std::fs::write(os(&p(b"keep")), "k").expect("keep");
    std::fs::write(os(&p(b"caf\xe9")), "x").expect("file");
    std::fs::write(os(&p(b"caf\xc3\xa9")), "y").expect("utf8 twin");
    let argv = ops
        .chmod_argv(&[p(b"caf\xe9")], 0o600, false)
        .expect("argv");
    assert!(run_remote(&argv, b"", &[]).2);
    let mode = |n: &[u8]| {
        use std::os::unix::fs::PermissionsExt;
        std::fs::metadata(os(&p(n)))
            .expect("meta")
            .permissions()
            .mode()
            & 0o7777
    };
    assert_eq!(mode(b"caf\xe9"), 0o600);
    assert_ne!(
        mode(b"caf\xc3\xa9"),
        0o600,
        "the UTF-8 twin must not be touched"
    );

    let argv = ops
        .delete_many_argv(&[p(b"caf\xe9"), p(b"n\xe9w dir")])
        .expect("argv");
    assert!(run_remote(&argv, b"", &[]).2);
    assert!(
        !Path::new(&os(&p(b"caf\xe9"))).exists() && !Path::new(&os(&p(b"n\xe9w dir"))).exists()
    );
    assert!(Path::new(&os(&p(b"caf\xc3\xa9"))).exists() && Path::new(&os(&p(b"keep"))).exists());
}

#[test]
fn quote_bytes_round_trips_through_sh_and_matches_shell_quote() {
    let samples: [&[u8]; 14] = [
        b"",
        b"plain",
        b"a b",
        b"it's",
        b"\"q\"",
        b"$HOME `id` ;&|<>*?[]{}()#!~\\",
        b"caf\xe9",
        b"\xff\xfe",
        b"new\nline",
        b"tab\t",
        b"-rf",
        "zażółć".as_bytes(),
        b"a\x01b",
        b"/safe/p-1_2.3@x%y+z=w:v,u",
    ];
    for s in samples {
        let q = quote_bytes(s);
        let mut c = Command::new("sh");
        c.arg("-c")
            .arg(os(&[b"printf %s ".as_slice(), &q].concat()));
        assert_eq!(c.output().expect("sh").stdout, s, "{s:?}");
        if let Ok(t) = std::str::from_utf8(s) {
            assert_eq!(q, termigator_core::shell::quote(t).into_bytes(), "{t:?}");
        }
    }
}

#[test]
fn delete_guard_refuses_placeholders() {
    let (conn, launch, env) = (conn(), Value::Null, ops_env());
    let ops = RemoteOps {
        conn: &conn,
        launch: &launch,
        env: &env,
    };
    for bad in [&b""[..], b"/", b"//", b".", b"./", b"..", b"../", b"..//"] {
        assert!(ops.delete_argv(bad).is_err(), "{bad:?}");
        assert!(
            ops.delete_many_argv(&[&b"/ok"[..], bad]).is_err(),
            "{bad:?}"
        );
    }
    assert!(ops.delete_many_argv::<&[u8]>(&[]).is_err());
    assert!(ops.delete_argv(b"/t/a b").is_ok());
}

#[test]
fn argv_shape() {
    let (conn, launch, env) = (conn(), Value::Null, ops_env());
    let ops = RemoteOps {
        conn: &conn,
        launch: &launch,
        env: &env,
    };
    let argv = ops.exec_argv(b"pwd").expect("argv");
    assert_eq!(argv[0], b"ssh");
    assert_eq!(argv[argv.len() - 2], b"root@srv");
    assert_eq!(argv[argv.len() - 1], b"pwd");
    let line: Vec<String> = argv
        .iter()
        .map(|a| String::from_utf8_lossy(a).into_owned())
        .collect();
    assert!(
        line.contains(&"BatchMode=yes".to_owned())
            && line.contains(&"ConnectTimeout=10".to_owned())
    );
}

/// A server-chosen `..` or `.` never leaves the digest folder.
#[test]
fn edit_cache_path_stays_inside_its_digest_folder() {
    let conn = Value::Map(Vec::new());
    let root = std::path::Path::new("/cache");
    for remote in [&b"/srv/.."[..], b"/srv/.", b"..", b"/"] {
        let p = edit_cache_path(&conn, remote, Some(root));
        assert_eq!(
            p.file_name().and_then(|n| n.to_str()),
            Some("file"),
            "{p:?}"
        );
        assert_eq!(p.parent().and_then(|d| d.parent()), Some(root), "{p:?}");
    }
}

#[test]
fn a_refused_login_is_told_from_other_failures() {
    let denied = b"root@srv: Permission denied (publickey,password).\n";
    assert!(is_auth_failure(255, denied));
    assert!(is_auth_failure(255, b"Authentication failed.\n"));
    // Other ssh failures, and success, are not an authentication problem.
    assert!(!is_auth_failure(
        255,
        b"ssh: connect to host srv port 22: Connection refused\n"
    ));
    assert!(!is_auth_failure(1, denied));
    assert!(!is_auth_failure(0, b""));
}

#[test]
fn a_typed_password_lifts_batch_mode() {
    let launch = Value::Null;
    let env = Env {
        which: Box::new(|n| (n == "sshpass").then(|| "/usr/bin/sshpass".to_owned())),
        control_dir: None,
        home: None,
    };
    let text = |argv: Vec<Vec<u8>>| {
        argv.iter()
            .map(|a| String::from_utf8_lossy(a).into_owned())
            .collect::<Vec<_>>()
            .join(" ")
    };
    let plain = conn();
    let ops = RemoteOps {
        conn: &plain,
        launch: &launch,
        env: &env,
    };
    let before = text(ops.exec_argv(b"true").expect("argv"));
    assert!(
        before.contains("BatchMode=yes") && !before.contains("sshpass"),
        "{before}"
    );

    let typed = with_typed_password(&plain, "s3cret");
    assert_eq!(host_key(&typed), host_key(&plain));
    let ops = RemoteOps {
        conn: &typed,
        launch: &launch,
        env: &env,
    };
    let after = text(ops.exec_argv(b"true").expect("argv"));
    // The password goes through sshpass's environment, never the command line.
    assert!(
        after.starts_with("sshpass -e")
            && !after.contains("BatchMode")
            && !after.contains("s3cret"),
        "{after}"
    );
}
