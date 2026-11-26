//! `termigator_core::proto::ssh` golden corpus (compat/golden/ssh_*.json) and the
//! non-scp command-line tests.
// Unix-only for now: uses modes, symlinks, sockets or sh stubs.
#![cfg(unix)]

mod common;

use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;

use serde_json::Value as J;
use termigator_core::config::doc::Value;
use termigator_core::proto::ssh::{
    Env, KeyKind, available_shells, available_shells_from, build_ssh_command_with,
    classify_key_file, clipboard_ssh_command, default_interactive_shell, format_proxy_jump_for_ui,
    forward_specs_from_yaml, identity_flags, identity_warning, keepalive_ssh_opts,
    parse_openssh_version, parse_quick_ssh_target, ssh_control_opts, ssh_password_env,
    ssh_session_env, x11_display_warning,
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

fn expected(case: &J, fix: &str) -> String {
    case["expect"]
        .as_str()
        .expect("expect")
        .replace("@FIX@", fix)
}

fn message(case: &J, fix: &str) -> String {
    case["message"]
        .as_str()
        .expect("message")
        .replace("@FIX@", fix)
}

fn strings(v: &J) -> Vec<String> {
    v.as_array()
        .expect("list")
        .iter()
        .map(|s| s.as_str().expect("str").to_owned())
        .collect()
}

fn pairs(v: &J) -> Vec<(String, String)> {
    v.as_object()
        .expect("map")
        .iter()
        .map(|(k, v)| (k.clone(), v.as_str().expect("str").to_owned()))
        .collect()
}

#[test]
fn build_matches_golden() {
    let fix = fix_dir();
    let cases = common::golden("ssh_build");
    assert!(cases.len() > 2000);
    for case in &cases {
        let n = common::name(case);
        let conn = from_golden(&case["conn"], &fix);
        let launch = from_golden(&case["launch"], &fix);
        let got = build_ssh_command_with(&conn, &launch, &case_env(case, &fix));
        if case.get("error").is_some() {
            assert_eq!(got, Err(message(case, &fix)), "{n}");
        } else {
            assert_eq!(got, Ok(expected(case, &fix)), "{n}");
        }
    }
}

#[test]
fn quick_target_matches_golden() {
    let cases = common::golden("ssh_quick");
    assert!(cases.len() > 95);
    for case in &cases {
        let n = common::name(case);
        let got = parse_quick_ssh_target(case["input"].as_str().expect("input"));
        if case.get("error").is_some() {
            assert_eq!(got, Err(message(case, "")), "{n}");
        } else {
            let e = &case["expect"];
            let port = e[2]["$int"]
                .as_str()
                .map(|p| p.parse::<u16>().expect("port"));
            let t = got.unwrap_or_else(|err| panic!("{n}: {err}"));
            assert_eq!(t.user, e[0].as_str().expect("user"), "{n}");
            assert_eq!(t.host, e[1].as_str().expect("host"), "{n}");
            assert_eq!(t.port, port, "{n}");
        }
    }
}

#[test]
fn misc_matches_golden() {
    let fix = fix_dir();
    let cases = common::golden("ssh_misc");
    assert!(cases.len() > 300);
    for case in &cases {
        let n = common::name(case);
        let conn = || from_golden(&case["conn"], &fix);
        match case["fn"].as_str().expect("fn") {
            "keepalive" => {
                let secs = case["input"]["$int"]
                    .as_str()
                    .expect("int")
                    .parse()
                    .expect("i64");
                assert_eq!(keepalive_ssh_opts(secs), strings(&case["expect"]), "{n}");
            }
            "control_opts" => assert_eq!(
                ssh_control_opts(case["input"].as_str()),
                strings(&case["expect"]),
                "{n}"
            ),
            "password_env" => {
                let got = ssh_password_env(&conn());
                if case.get("error").is_some() {
                    assert_eq!(
                        got.expect_err(n).to_string(),
                        case["message"].as_str().expect("message"),
                        "{n}"
                    );
                } else {
                    assert_eq!(exposed(&got.expect(n)), pairs_list(&case["expect"]), "{n}");
                }
            }
            "clipboard" => assert_eq!(
                clipboard_ssh_command(case["cmd"].as_str().expect("cmd"), &conn()),
                case["expect"].as_str().expect("expect"),
                "{n}"
            ),
            "session_env" => {
                assert_eq!(ssh_session_env(&conn()), pairs_list(&case["expect"]), "{n}")
            }
            "x11" => {
                let display = case["env"]["DISPLAY"].as_str();
                assert_eq!(
                    x11_display_warning(&conn(), display),
                    case["expect"].as_str().map(str::to_owned),
                    "{n}"
                );
            }
            "forward_specs" => {
                let v = from_golden(&case["input"], &fix);
                let opt = (!matches!(v, Value::Null)).then_some(&v);
                assert_eq!(
                    forward_specs_from_yaml(opt),
                    strings(&case["expect"]),
                    "{n}"
                );
            }
            "proxy_ui" => assert_eq!(
                format_proxy_jump_for_ui(&conn()),
                case["expect"].as_str().expect("expect"),
                "{n}"
            ),
            "shell" => {
                let cfg = from_golden(&case["cfg"], &fix);
                let tools = strings(&case["which"]);
                let which = |name: &str| {
                    tools
                        .iter()
                        .any(|t| t == name)
                        .then(|| format!("/usr/bin/{name}"))
                };
                assert_eq!(
                    default_interactive_shell(
                        &cfg,
                        case["shell_env"].as_str().expect("shell_env"),
                        &which
                    ),
                    case["expect"].as_str().expect("expect"),
                    "{n}"
                );
            }
            other => panic!("{n}: unknown fn {other}"),
        }
    }
}

/// Golden env dicts keep insertion order; JSON objects here are sorted, so compare as sets.
fn pairs_list(v: &J) -> Vec<(String, String)> {
    let mut p = pairs(v);
    p.sort();
    p
}

// ---- command-line tests (non-scp) ----

fn conn(pairs: &[(&str, Value)]) -> Value {
    Value::Map(
        pairs
            .iter()
            .map(|(k, v)| (Value::Str((*k).to_owned()), v.clone()))
            .collect(),
    )
}

fn s(t: &str) -> Value {
    Value::Str(t.to_owned())
}

fn nk() -> Value {
    conn(&[("ssh_keepalive", Value::Bool(false))])
}

fn build(c: &Value, l: &Value, tools: &[&str]) -> Result<String, String> {
    build_ssh_command_with(
        c,
        l,
        &env_for(tools, Some("/run/user/1000/termigator"), &fix_dir()),
    )
}

#[test]
fn keepalive_opts() {
    assert_eq!(
        keepalive_ssh_opts(15),
        ["-o", "ServerAliveInterval=5", "-o", "ServerAliveCountMax=3"]
    );
    assert!(keepalive_ssh_opts(20).contains(&"ServerAliveInterval=7".to_owned()));
    assert!(keepalive_ssh_opts(1).contains(&"ServerAliveInterval=1".to_owned()));
}

#[test]
fn password_env_refuses_sealed() {
    let err = ssh_password_env(&conn(&[("password", s("enc:v1:AAAA"))])).expect_err("sealed");
    assert_eq!(
        err.to_string(),
        "Password is still encrypted — unlock the vault first."
    );
    assert_eq!(
        ssh_password_env(&conn(&[("password", s("pw"))])).map(|e| exposed(&e)),
        Ok(vec![("SSHPASS".to_owned(), "pw".to_owned())])
    );
    assert_eq!(ssh_password_env(&conn(&[])), Ok(vec![]));
}

#[test]
fn injection_table() {
    let bad = [
        ("-oProxyCommand=touch /tmp/pwned", "h", "user"),
        ("u", "-oProxyCommand=id", "host"),
        (" -oX=y", "h", "user"),
        ("u", "h x", "spaces"),
        ("u\nv", "h", "control"),
        ("u", "evil@h", "'@'"),
    ];
    for (u, h, why) in bad {
        let c = conn(&[("host", s(h)), ("user", s(u))]);
        let err = build(&c, &nk(), &[]).expect_err(u);
        assert!(err.contains(why), "{err}");
        let m = conn(&[
            ("host", s(h)),
            ("user", s(u)),
            ("use_mosh", Value::Bool(true)),
        ]);
        assert!(build(&m, &nk(), &["mosh"]).is_err());
    }
}

/// Through a jump host the saved password does not go to sshpass (it would
/// answer the jump host's prompt) but to the askpass helper, which answers only the target.
#[test]
fn a_password_through_a_jump_host_reaches_only_the_target() {
    use termigator_core::proto::askpass::{CHANNEL_ENV, TOKEN_ENV, fetch};
    let jumped = conn(&[
        ("host", s("db")),
        ("user", s("pg")),
        ("password", s("hunter2")),
        ("proxy_jump", s("j@bastion")),
    ]);
    let line = build(&jumped, &nk(), &["sshpass"]).expect("built");
    assert_eq!(line, "ssh -J j@bastion pg@db");
    let direct = conn(&[("host", s("db")), ("user", s("pg")), ("password", s("x"))]);
    assert!(
        build(&direct, &nk(), &["sshpass"])
            .expect("built")
            .starts_with("sshpass -e ")
    );

    let Ok(env) = ssh_password_env(&jumped).map(|e| exposed(&e)) else {
        return; // no private runtime directory for the helper on this machine
    };
    assert!(env.iter().all(|(k, _)| k != "SSHPASS"), "{env:?}");
    let get = |k: &str| env.iter().find(|(n, _)| n == k).map(|(_, v)| v.clone());
    let channel = PathBuf::from(get(CHANNEL_ENV).expect("channel"));
    let token = get(TOKEN_ENV).expect("token");
    let ask = |prompt: &str| fetch(&channel, &token, prompt).expect("fetch");
    assert_eq!(
        ask("pg@db's password: ").as_deref(),
        Some(&b"hunter2\n"[..])
    );
    assert_eq!(ask("j@bastion's password: "), None);
    assert_eq!(ask("Password: "), None);
}

#[test]
fn jump_hops_with_spaces_are_refused() {
    for hop in ["x -oProxyCommand=id", "a,b c", "j\th"] {
        for mosh in [false, true] {
            let c = conn(&[
                ("host", s("h")),
                ("user", s("u")),
                ("proxy_jump", s(hop)),
                ("use_mosh", Value::Bool(mosh)),
            ]);
            let err = build(&c, &nk(), &["mosh"]).expect_err(hop);
            assert!(err.contains("spaces"), "{err}");
        }
    }
    let ok = conn(&[
        ("host", s("h")),
        ("user", s("u")),
        ("proxy_jump", s("a@j:2,k")),
    ]);
    assert_eq!(build(&ok, &nk(), &[]).expect("ok"), "ssh -J a@j:2,k u@h");
}

#[test]
fn remote_command_and_mosh() {
    let rc = "bash -c 'tmux attach || tmux new'";
    let c = conn(&[
        ("host", s("h")),
        ("user", s("u")),
        ("remote_command", s(rc)),
    ]);
    let line = build(&c, &nk(), &[]).expect("ok");
    assert_eq!(
        line,
        format!("ssh -t u@h {}", termigator_core::shell::quote(rc))
    );
    let m = conn(&[
        ("host", s("h")),
        ("user", s("u")),
        ("use_mosh", Value::Bool(true)),
        ("port", Value::Int(2222)),
        ("remote_command", s("htop")),
    ]);
    assert_eq!(
        build(&m, &nk(), &["mosh"]).expect("ok"),
        "mosh '--ssh=ssh -p 2222' u@h -- sh -c htop"
    );
    let bad = conn(&[
        ("host", s("h")),
        ("user", s("u")),
        ("remote_command", s("echo 'oops")),
    ]);
    assert!(
        build(&bad, &Value::Null, &[])
            .expect_err("quoting")
            .contains("quoting")
    );
}

#[test]
fn basics_and_password() {
    let c = conn(&[("host", s("h")), ("user", s("u"))]);
    assert_eq!(build(&c, &nk(), &[]), Ok("ssh u@h".to_owned()));
    assert!(
        build(&conn(&[("user", s("u"))]), &Value::Null, &[])
            .expect_err("missing")
            .to_lowercase()
            .contains("missing")
    );
    let p = conn(&[
        ("host", s("h")),
        ("user", s("u")),
        ("password", s("s3cret")),
    ]);
    assert_eq!(
        build(&p, &nk(), &["sshpass"]),
        Ok("sshpass -e ssh u@h".to_owned())
    );
    assert!(
        build(&p, &Value::Null, &[])
            .expect_err("no sshpass")
            .contains("sshpass")
    );
    assert_eq!(
        clipboard_ssh_command("sshpass -e ssh u@h", &p),
        "SSHPASS='****' sshpass -e ssh u@h"
    );
    assert_eq!(clipboard_ssh_command("ssh u@h", &c), "ssh u@h");
}

#[test]
fn f44_session_options() {
    let t = Value::Bool(true);
    let c = conn(&[
        ("host", s("h")),
        ("user", s("u")),
        ("x11_forwarding", t.clone()),
        ("x11_trusted", t.clone()),
        ("compression", t.clone()),
        ("request_tty", t.clone()),
        ("remote_locale", s("pl_PL.UTF-8")),
    ]);
    let cmd = build(&c, &nk(), &[]).expect("ok");
    assert!(cmd.contains(" -Y ") && !cmd.contains(" -X "));
    assert!(cmd.contains("-o Compression=yes") && cmd.contains(" -t "));
    assert!(cmd.contains("-o 'SendEnv=LANG LC_ALL'"));
    let bad = conn(&[
        ("host", s("h")),
        ("user", s("u")),
        ("remote_locale", s("x; rm -rf /")),
    ]);
    assert!(build(&bad, &Value::Null, &[]).is_err());
    assert_eq!(
        ssh_session_env(&conn(&[("remote_locale", s("pl_PL.UTF-8"))])).len(),
        2
    );
    let x = conn(&[("x11_forwarding", t)]);
    assert!(x11_display_warning(&x, None).is_some());
    assert!(x11_display_warning(&x, Some(":0")).is_none());
    assert!(x11_display_warning(&conn(&[]), None).is_none());
}

#[test]
fn control_master_dir() {
    assert!(ssh_control_opts(None).is_empty());
    assert_eq!(ssh_control_opts(Some("/d"))[3], "ControlPath=/d/cm-%C");
}

#[test]
fn unreadable_identity_is_refused() {
    let dir = std::env::temp_dir().join(format!("termigator-ssh-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("mkdir");
    let key = dir.join("key");
    std::fs::write(&key, "k").expect("write");
    std::fs::set_permissions(&key, std::fs::Permissions::from_mode(0o000)).expect("chmod");
    let readable = std::fs::File::open(&key).is_ok(); // root reads anything
    let c = conn(&[
        ("host", s("h")),
        ("user", s("u")),
        ("identity_file", s(&key.to_string_lossy())),
    ]);
    let got = build(&c, &Value::Null, &[]);
    let _ = std::fs::remove_dir_all(&dir);
    if !readable {
        assert!(
            got.expect_err("unreadable")
                .starts_with("SSH key file is not readable: ")
        );
    }
}

#[test]
fn key_kind_follows_the_content_not_the_name() {
    let dir = std::env::temp_dir().join(format!("termigator-keykind-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("mkdir");
    let put = |name: &str, body: &str| {
        let p = dir.join(name);
        std::fs::write(&p, body).expect("write");
        p
    };
    // A private key kept under a name ending in .pub is still a private key.
    let private = put(
        "work.pub",
        "-----BEGIN OPENSSH PRIVATE KEY-----\nAAAA\n-----END OPENSSH PRIVATE KEY-----\n",
    );
    // 0600 as ssh wants it: a readable one warns for another reason (see the next test).
    std::fs::set_permissions(&private, std::fs::Permissions::from_mode(0o600)).expect("chmod");
    let public = put("id_ed25519", "ssh-ed25519 AAAAC3Nza me@host\n");
    let ppk = put("k.ppk", "PuTTY-User-Key-File-3: ssh-ed25519\n");
    let junk = put("notes.pub", "hello\n");
    assert_eq!(classify_key_file(&private), KeyKind::Private);
    assert_eq!(classify_key_file(&public), KeyKind::Public);
    assert_eq!(classify_key_file(&ppk), KeyKind::Putty);
    assert_eq!(classify_key_file(&junk), KeyKind::Unknown);
    assert_eq!(classify_key_file(&dir.join("missing")), KeyKind::Unknown);

    let with = |p: &std::path::Path| {
        conn(&[
            ("host", s("h")),
            ("user", s("u")),
            ("identity_file", s(&p.to_string_lossy())),
        ])
    };
    // No warning for a private key named .pub, and the command still carries -i.
    assert!(identity_warning(&with(&private), None).is_none());
    let cmd = build(&with(&private), &Value::Null, &[]).expect("private .pub goes ahead");
    assert!(cmd.contains("-i "), "{cmd}");
    // A public key (or .ppk) warns but is not refused.
    assert!(identity_warning(&with(&public), None).is_some_and(|w| w.contains("public key")));
    assert!(identity_warning(&with(&ppk), None).is_some_and(|w| w.contains("puttygen")));
    assert!(build(&with(&public), &Value::Null, &[]).is_ok());
    // A name ending in .pub with unrecognised content is not judged at all.
    assert!(identity_warning(&with(&junk), None).is_none());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn default_shell_rules() {
    let none = |_: &str| None;
    let sh = conn(&[(
        "launch",
        conn(&[("shell", s("bash")), ("shell_executable", s("/bin/sh"))]),
    )]);
    assert_eq!(default_interactive_shell(&sh, "", &none), "/bin/sh -l");
    let which = |n: &str| (n == "sh").then(|| "/usr/bin/sh".to_owned());
    let empty = conn(&[("launch", conn(&[("shell", s(""))]))]);
    assert_eq!(
        default_interactive_shell(&empty, "sh", &which),
        "/usr/bin/sh -l"
    );
    assert_eq!(
        default_interactive_shell(&conn(&[]), "", &none),
        "/bin/sh -l"
    );
}

#[test]
fn available_shells_dedupes_and_skips_non_shells() {
    let tmp = std::env::temp_dir().join(format!("termigator-shells-{}", std::process::id()));
    let (bin, usr): (PathBuf, PathBuf) = (tmp.join("bin"), tmp.join("usr/bin"));
    std::fs::create_dir_all(&bin).expect("mkdir");
    std::fs::create_dir_all(&usr).expect("mkdir");
    for d in [&bin, &usr] {
        for name in ["bash", "zsh", "rbash", "screen"] {
            let f = d.join(name);
            std::fs::write(&f, "#!/bin/sh\n").expect("write");
            std::fs::set_permissions(&f, std::fs::Permissions::from_mode(0o755)).expect("chmod");
        }
    }
    std::fs::write(usr.join("fish"), "").expect("write");
    let mut lines = vec!["# /etc/shells: valid login shells".to_owned()];
    for name in ["bash", "rbash", "zsh", "screen"] {
        lines.push(bin.join(name).display().to_string());
        lines.push(usr.join(name).display().to_string());
    }
    lines.push(usr.join("fish").display().to_string());
    lines.extend(["/nonexistent/ksh", "", "  # comment"].map(str::to_owned));
    let shells = tmp.join("shells");
    std::fs::write(&shells, lines.join("\n") + "\n").expect("write");
    let want = vec![
        bin.join("bash").display().to_string(),
        bin.join("zsh").display().to_string(),
    ];
    assert_eq!(available_shells(&shells.to_string_lossy()), want);
    assert_eq!(available_shells_from(&lines.join("\r\n")), want);
    assert!(available_shells(&tmp.join("missing").to_string_lossy()).is_empty());
    let _ = std::fs::remove_dir_all(&tmp);
}

/// The env pairs with their secret values shown, for exact comparison.
fn exposed(env: &[(String, termigator_core::vault::Secret)]) -> Vec<(String, String)> {
    env.iter()
        .map(|(k, v)| {
            (
                k.clone(),
                String::from_utf8(v.expose().to_vec()).expect("utf-8"),
            )
        })
        .collect()
}

#[test]
fn folder_tracking_replaces_only_a_missing_remote_command() {
    let host = [
        ("host", s("h")),
        ("user", s("u")),
        ("ssh_keepalive", Value::Bool(false)),
    ];
    let on = conn(&[("track_remote_folder", Value::Bool(true))]);
    let off = conn(&[("track_remote_folder", Value::Bool(false))]);
    let plain = build(&conn(&host), &Value::Map(Vec::new()), &[]).unwrap();
    assert!(!plain.contains("base64"), "no flag, no bootstrap: {plain}");
    assert_eq!(build(&conn(&host), &off, &[]).unwrap(), plain);
    let tracked = build(&conn(&host), &on, &[]).unwrap();
    assert!(
        tracked.contains(" -t ") && tracked.contains("base64 -d"),
        "{tracked}"
    );
    // The entry's own remote command wins.
    let mut own = host.to_vec();
    own.push(("remote_command", s("tmux a")));
    let kept = build(&conn(&own), &on, &[]).unwrap();
    assert!(
        kept.contains("tmux a") && !kept.contains("base64"),
        "{kept}"
    );
}

#[test]
fn openssh_versions_from_ssh_dash_v() {
    assert_eq!(
        parse_openssh_version("OpenSSH_9.6p1 Ubuntu-3ubuntu13, OpenSSL 3.0.13"),
        Some((9, 6))
    );
    assert_eq!(
        parse_openssh_version("OpenSSH_for_Windows_9.5p1, LibreSSL 3.8.2"),
        Some((9, 5))
    );
    assert_eq!(parse_openssh_version("OpenSSH_8.3p1"), Some((8, 3)));
    assert_eq!(parse_openssh_version("OpenSSH_10.0"), Some((10, 0)));
    assert_eq!(parse_openssh_version("Dropbear v2024.85"), None);
}

#[test]
fn a_key_readable_by_others_goes_to_ssh_as_a_private_copy() {
    // OpenSSH refuses it ("UNPROTECTED PRIVATE KEY FILE"); MobaXterm's own ssh does not.
    let dir = std::env::temp_dir().join(format!("termigator-keymode-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("mkdir");
    let key = dir.join("id_shared");
    let body = "-----BEGIN OPENSSH PRIVATE KEY-----\nAAAA\n-----END OPENSSH PRIVATE KEY-----\n";
    std::fs::write(&key, body).expect("write");
    std::fs::write(
        dir.join("id_shared-cert.pub"),
        "ssh-ed25519-cert-v01@openssh.com AAAA\n",
    )
    .expect("cert");
    std::fs::set_permissions(&key, std::fs::Permissions::from_mode(0o644)).expect("chmod");
    let c = conn(&[
        ("host", s("h")),
        ("identity_file", s(&key.to_string_lossy())),
    ]);
    let flags = identity_flags(&c, None).expect("flags");
    let copy = PathBuf::from(&flags[1]);
    assert_ne!(copy, key, "{flags:?}");
    assert_eq!(std::fs::read_to_string(&copy).expect("copy"), body);
    let mode = std::fs::metadata(&copy).expect("meta").permissions().mode();
    assert_eq!(mode & 0o777, 0o600);
    let cert = PathBuf::from(format!("{}-cert.pub", copy.display()));
    assert!(cert.is_file(), "the certificate goes along");
    // The user's file is left alone, and the editor / terminal say why.
    let mode = std::fs::metadata(&key).expect("meta").permissions().mode();
    assert_eq!(mode & 0o777, 0o644);
    assert!(identity_warning(&c, None).is_some_and(|w| w.contains("private copy")));
    // A changed key replaces the copy; a fixed one is used as it is.
    std::fs::write(&key, "-----BEGIN OPENSSH PRIVATE KEY-----\nBBBB\n").expect("write");
    let again = identity_flags(&c, None).expect("flags");
    assert_eq!(again[1], flags[1]);
    assert!(
        std::fs::read_to_string(&copy)
            .expect("copy")
            .contains("BBBB")
    );
    std::fs::set_permissions(&key, std::fs::Permissions::from_mode(0o600)).expect("chmod");
    assert_eq!(
        identity_flags(&c, None).expect("flags")[1],
        key.to_string_lossy()
    );
    assert!(identity_warning(&c, None).is_none());
    let _ = std::fs::remove_dir_all(&dir);
}
