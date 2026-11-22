//! The FreeRDP command line for real — a stub `xfreerdp3` checks what the shell actually
//! hands the client (flags as argv, the password on stdin only), and the installed FreeRDP 3
//! must accept the redirection flags (a parse error exits with 23
//! and prints "Failed at index"). The connection then fails (port 1 is closed); with `$DISPLAY` or a private `Xvfb`
//! it gets as far as the TCP connect, without a display FreeRDP 3 still parses the whole command line before giving up in pre-connect.
// Unix-only for now: uses modes, symlinks, sockets or sh stubs.
#![cfg(unix)]

mod live;

use std::{
    fs::{self, File},
    io::{BufRead, BufReader},
    process::{Command, Stdio},
    time::Duration,
};

use live::{Daemon, have, map, s, wait_until};
use termigator_core::{
    config::doc::Value,
    proto::{
        rdp::{build_rdp_command, rdp_password_env},
        ssh::{self, Env},
    },
    shell,
};

/// The connection used for the flag checks.
fn rdp_with_flags(extra: &[(&str, Value)], without: &[&str]) -> Value {
    let mut pairs: Vec<(&str, Value)> = vec![
        ("protocol", s("rdp")),
        ("host", s("win.example")),
        ("user", s("adm")),
        ("clipboard", Value::Bool(false)),
        ("home_drive", Value::Bool(true)),
        ("shared_folders", Value::Seq(vec![s("/tmp")])),
        ("sound", Value::Bool(true)),
        ("microphone", Value::Bool(true)),
        ("multimon", Value::Bool(true)),
        ("dynamic_resolution", Value::Bool(true)),
        ("gateway", s("gw.example:443")),
        ("gateway_user", s("gwuser")),
        ("gateway_domain", s("CORP")),
    ];
    pairs.retain(|(k, _)| !without.contains(k) && !extra.iter().any(|(e, _)| e == k));
    pairs.extend(extra.iter().cloned());
    map(&pairs)
}

fn env_on(path: &std::ffi::OsStr, home: &std::path::Path) -> Env {
    let path = path.to_string_lossy().into_owned();
    Env {
        which: Box::new(move |n| ssh::which(n, Some(&path))),
        control_dir: None,
        home: Some(home.to_string_lossy().into_owned()),
    }
}

const STUB: &str = "#!/bin/sh\n\
    for a; do printf '%s\\n' \"$a\"; done > \"$TERMIGATOR_STUB_ARGV\"\n\
    IFS= read -r pw; printf '%s' \"$pw\" > \"$TERMIGATOR_STUB_STDIN\"\n";

#[test]
fn stub_xfreerdp3_gets_flags_as_argv_and_the_password_on_stdin() {
    let _g = live::serial();
    if !have(
        "stub_xfreerdp3_gets_flags_as_argv_and_the_password_on_stdin",
        &["bash"],
    ) {
        return;
    }
    let d = live::scratch("rdp-stub");
    let bin = d.join("bin");
    fs::create_dir(&bin).expect("bin");
    live::script(&bin.join("xfreerdp3"), STUB);
    let path = live::path_with(&bin);
    let conn = rdp_with_flags(&[("password", s("s3cret"))], &[]);
    let launch = map(&[("rdp_client", s("xfreerdp3"))]);
    let cmd = build_rdp_command(&conn, &launch, &env_on(&path, &d)).expect("rdp command");
    assert!(!cmd.contains("s3cret"), "{cmd}");
    let env = rdp_password_env(&conn).expect("password env");
    let (argv_file, stdin_file) = (d.join("argv"), d.join("stdin"));
    let out = Command::new("bash")
        .arg("-c")
        .arg(&cmd)
        .env("PATH", &path)
        .envs(
            env.iter()
                .map(|(k, v)| (k.as_str(), std::str::from_utf8(v.expose()).expect("utf-8"))),
        )
        .env("TERMIGATOR_STUB_ARGV", &argv_file)
        .env("TERMIGATOR_STUB_STDIN", &stdin_file)
        .output()
        .expect("bash");
    assert!(out.status.success(), "{out:?}");
    let words = shell::split(&cmd).expect("split");
    let pipe = words.iter().position(|w| w == "|").expect("password pipe");
    assert_eq!(words[pipe + 1], "xfreerdp3");
    let want: Vec<&str> = words[pipe + 2..].iter().map(String::as_str).collect();
    let got = fs::read_to_string(&argv_file).expect("argv");
    assert_eq!(got.lines().collect::<Vec<_>>(), want);
    assert!(
        want.contains(&"/from-stdin") && want.contains(&"/u:adm"),
        "{want:?}"
    );
    assert_eq!(fs::read_to_string(&stdin_file).expect("stdin"), "s3cret");
    let _ = fs::remove_dir_all(&d);
}

/// `$DISPLAY`, or a private Xvfb (kept alive by the returned guard). Without either FreeRDP 3
/// still parses the command line (it stops later, in pre-connect), so the test runs anyway.
fn display(test: &str) -> Option<(String, Option<Daemon>)> {
    if let Some(d) = std::env::var("DISPLAY").ok().filter(|d| !d.is_empty()) {
        return Some((d, None));
    }
    if live::which("Xvfb").is_none() {
        eprintln!("{test}: no $DISPLAY and no Xvfb — only the command line parse is checked");
        return None;
    }
    let mut x = Daemon::spawn(
        Command::new("Xvfb")
            .args(["-displayfd", "1", "-nolisten", "tcp"])
            .stdout(Stdio::piped())
            .stderr(Stdio::null()),
    );
    let mut line = String::new();
    BufReader::new(x.0.stdout.take().expect("Xvfb stdout"))
        .read_line(&mut line)
        .expect("Xvfb display number");
    let n = line.trim();
    assert!(!n.is_empty(), "Xvfb printed no display number");
    Some((format!(":{n}"), Some(x)))
}

#[test]
fn redirection_flags_parse_in_real_freerdp() {
    let _g = live::serial();
    let test = "redirection_flags_parse_in_real_freerdp";
    if !have(test, &["xfreerdp3"]) {
        return;
    }
    let (disp, _x) = display(test).unwrap_or_default();
    let d = live::scratch("rdp-real");
    let conn = rdp_with_flags(
        &[
            ("host", s("127.0.0.1")),
            ("port", Value::Int(1)),
            ("extra_rdp_args", Value::Seq(vec![s("/cert:ignore")])),
        ],
        &["gateway"],
    );
    let launch = map(&[("rdp_client", s("xfreerdp3"))]);
    let path = std::env::var_os("PATH").unwrap_or_default();
    let cmd = build_rdp_command(&conn, &launch, &env_on(&path, &d)).expect("rdp command");
    let argv = shell::split(&cmd).expect("split");
    let (out_file, err_file) = (d.join("out"), d.join("err"));
    let mut freerdp = Command::new(&argv[0]);
    if disp.is_empty() {
        freerdp.env_remove("DISPLAY");
    } else {
        freerdp.env("DISPLAY", &disp);
    }
    let mut child = freerdp
        .args(&argv[1..])
        .env("HOME", &d) // FreeRDP writes its config under ~/.config
        .stdin(Stdio::null())
        .stdout(File::create(&out_file).expect("out"))
        .stderr(File::create(&err_file).expect("err"))
        .spawn()
        .expect("xfreerdp3");
    let mut status = None;
    let finished = wait_until(Duration::from_secs(30), || {
        status = child.try_wait().expect("wait");
        status.is_some()
    });
    if !finished {
        let _ = child.kill();
        let _ = child.wait();
    }
    let text = format!(
        "{}{}",
        fs::read_to_string(&out_file).unwrap_or_default(),
        fs::read_to_string(&err_file).unwrap_or_default()
    );
    assert!(finished, "xfreerdp3 hung: {text}");
    let code = status.and_then(|s| s.code());
    eprintln!(
        "{test}: xfreerdp3 on {disp} exited with {code:?} (port 1 is closed): {}",
        text.lines().last().unwrap_or_default()
    );
    assert!(
        !text.contains("Failed at index") && code != Some(23),
        "FreeRDP refused the flags ({code:?}): {cmd}\n{text}"
    );
    let _ = fs::remove_dir_all(&d);
}
