//! Real shells — the OSC 7 snippet in bash and zsh, folder
//! tracking end to end in an interactive shell and an asciicast
//! recording of a real terminal, through the production pty and [`SessionSink`].
// Unix-only for now: uses modes, symlinks, sockets or sh stubs.
#![cfg(unix)]

mod live;

use std::{ffi::OsString, process::Command, time::Duration};

use live::{Term, have, wait_until};
use serde_json::Value as J;
use termigator_core::{
    proc::pty::{SessionSink, ShellCommand, SinkEvent},
    shell,
    text::osc7::{OSC7_SHELL_SNIPPET, Osc7Scanner},
};

const SECS: Duration = Duration::from_secs(10);

#[test]
fn osc7_snippet_reports_pwd_in_real_shells() {
    for sh in ["bash", "zsh"] {
        if !have(
            &format!("osc7_snippet_reports_pwd_in_real_shells[{sh}]"),
            &[sh],
        ) {
            continue;
        }
        let d = live::scratch("osc7").join("my dir");
        std::fs::create_dir(&d).expect("dir");
        let dq = shell::quote(d.to_str().expect("utf-8"));
        let script = if sh == "bash" {
            format!("{OSC7_SHELL_SNIPPET} >/dev/null; cd {dq}; eval \"$PROMPT_COMMAND\"")
        } else {
            format!(
                "{OSC7_SHELL_SNIPPET} >/dev/null; cd {dq}; for f in $precmd_functions; do eval \"$f\"; done"
            )
        };
        let out = Command::new(sh)
            .arg("-c")
            .arg(&script)
            .env_clear()
            .env("PATH", std::env::var_os("PATH").unwrap_or_default())
            .output()
            .expect("shell");
        let got = Osc7Scanner::new().feed(&String::from_utf8_lossy(&out.stdout));
        assert_eq!(
            got.last().map(String::as_str),
            d.to_str(),
            "{sh}: {:?}",
            out
        );
    }
}

/// The environment of an interactive test shell: no user rc files (a private `HOME` with
/// empty ones, so zsh does not start its new-user wizard) and a UTF-8 locale.
fn shell_env(home: &std::path::Path) -> Vec<(OsString, OsString)> {
    for rc in [".zshrc", ".bashrc", ".bash_profile"] {
        std::fs::write(home.join(rc), "").expect("rc file");
    }
    vec![
        ("PATH".into(), std::env::var_os("PATH").unwrap_or_default()),
        ("HOME".into(), home.into()),
        ("LANG".into(), "C.UTF-8".into()),
    ]
}

/// "Enable folder tracking" types the OSC 7 hook into the shell (with a leading space, like
/// the terminal tab); every `cd` is then reported by the session's sink.
#[test]
fn folder_tracking_end_to_end_in_a_real_shell() {
    let _g = live::serial();
    for sh in ["bash", "zsh"] {
        let test = format!("folder_tracking_end_to_end_in_a_real_shell[{sh}]");
        if !have(&test, &[sh]) {
            continue;
        }
        let exe = live::which(sh).expect("shell");
        let home = live::scratch("track");
        let target = home.join("my dir");
        std::fs::create_dir(&target).expect("dir");
        let mut tab = Term::start(
            ShellCommand::LoginShell(exe.to_string_lossy().into_owned()),
            shell_env(&home),
            &[],
            Some("dumb"),
        );
        let mut sink = SessionSink::new();
        let mut seen = Vec::new();
        let mut pump = |tab: &mut Term, seen: &mut Vec<String>| {
            for c in tab.new_chunks() {
                for ev in sink.feed(&c) {
                    if let SinkEvent::Cwd(d) = ev {
                        seen.push(d);
                    }
                }
            }
        };
        std::thread::sleep(Duration::from_millis(500));
        tab.send(&format!(" {OSC7_SHELL_SNIPPET}\n"));
        std::thread::sleep(Duration::from_millis(300));
        tab.send(&format!("cd '{}'\n", target.display()));
        let want = target.to_string_lossy().into_owned();
        let found = wait_until(SECS, || {
            pump(&mut tab, &mut seen);
            seen.contains(&want)
        });
        assert!(found, "{test}: reported {seen:?}; output:\n{}", tab.text());
        tab.close();
        let _ = std::fs::remove_dir_all(&home);
    }
}

#[test]
fn recording_a_real_terminal() {
    let _g = live::serial();
    if !have("recording_a_real_terminal", &["bash"]) {
        return;
    }
    let home = live::scratch("rec");
    let path = home.join("s.cast");
    let env = shell_env(&home);
    let mut tab = Term::start(
        ShellCommand::BashLc("exec bash --norc -i".to_owned()),
        env.clone(),
        &[],
        None,
    );
    let mut sink = SessionSink::new();
    std::thread::sleep(Duration::from_millis(500));
    for c in tab.new_chunks() {
        sink.feed(&c);
    }
    let rec_env: Vec<(String, String)> = env
        .iter()
        .map(|(k, v)| {
            (
                k.to_string_lossy().into_owned(),
                v.to_string_lossy().into_owned(),
            )
        })
        .collect();
    sink.start_recording(&path, live::SIZE, "rec", &rec_env)
        .expect("start recording");
    assert_eq!(sink.recording_path(), Some(path.as_path()));
    tab.send("printf 'zażółć %s\\n' END-OF-DEMO\n");
    let done = wait_until(SECS, || {
        for c in tab.new_chunks() {
            sink.feed(&c);
        }
        std::fs::read_to_string(&path)
            .is_ok_and(|t| t.replace("\\r\\n", "\n").contains("zażółć END-OF-DEMO"))
    });
    assert!(
        done,
        "recording: {:?}\noutput:\n{}",
        std::fs::read_to_string(&path),
        tab.text()
    );
    sink.stop_recording().expect("stop recording");
    assert_eq!(sink.recording_path(), None);
    let text = std::fs::read_to_string(&path).expect("cast");
    let mut lines = text.lines();
    let header: J = serde_json::from_str(lines.next().expect("header")).expect("json header");
    assert_eq!(header["version"], 2);
    for l in lines {
        let ev: J = serde_json::from_str(l).expect("json event");
        assert!(matches!(ev[1].as_str(), Some("o" | "r")), "{l}");
    }
    tab.close();
    let _ = std::fs::remove_dir_all(&home);
}
