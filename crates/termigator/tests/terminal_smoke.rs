//! `qml/terminal/TerminalSmoke.qml` offscreen. Real sessions (a `cat` or a short script behind
//! each tab): output, keys, zoom, find, paste, the context menu, OSC 7, the distro, startup
//! commands, the session log and the recording, the "Session stopped" overlay with R and Enter,
//! auto-reconnect, bell / activity marks, MultiExec, settings applied to open tabs. This side reads
//! what the check wrote to disk and what the config became.

mod common;

use std::fs;
use std::path::PathBuf;

use termigator_core::config::doc::{self, Value};
use termigator_core::config::model::value_str;

// cxx-qt's generated C++ is linked into this test too; it needs the library's Rust half.
use termigator as _;

const CONFIG: &str = "window:\n  confirm_close: false\n  confirm_close_tab: false\n  width: 1000\n  height: 700\n\
                      connections:\n- {label: dist, host: dh}\n";
const CHECK: &str = "--smoke-check=terminal/TerminalSmoke.qml";

/// Runs the check once; `env` selects the pty wiring of the app (`TERMIGATOR_PTY_FEED`).
fn check(name: &str, env: &[(&str, &str)]) {
    // An empty HOME: the developer's ~/.local/share/konsole schemes would set the background opacity.
    let home = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("termigator-{name}-home"));
    fs::create_dir_all(&home).expect("home dir");
    let home = home.to_str().expect("utf-8 path");
    let env: Vec<_> = [("HOME", home)]
        .into_iter()
        .chain(env.iter().copied())
        .collect();
    let mut run = common::launch_env(name, CONFIG, &[CHECK], &env);
    let status = run.wait(90, |_| {});
    let log = run.log();
    assert!(status.success(), "exit {status}, stderr:\n{log}");
    assert!(
        log.contains("U4_DONE"),
        "the check did not finish, stderr:\n{log}"
    );

    let dir: PathBuf = run.shot.parent().expect("test dir").to_owned();
    let text = fs::read_to_string(dir.join("u4.log")).expect("the session log");
    assert!(text.contains("### Termigator session log start "), "{text}");
    assert!(text.contains("LOGGED_TEXT"), "{text}");

    let cast = fs::read_to_string(dir.join("u4.cast")).expect("the recording");
    let first = cast.lines().next().expect("a header line");
    assert!(
        first.contains("\"version\": 2") && first.contains("\"width\""),
        "{first}"
    );
    assert!(cast.contains("REC_TEXT"), "{cast}");

    // The distro went into the saved entry (the connections live in session.yaml).
    let session = fs::read_to_string(run.config.join("session.yaml")).expect("session.yaml");
    let Ok(Value::Seq(conns)) = doc::load(&session) else {
        panic!("no connections:\n{session}");
    };
    assert_eq!(
        conns[0].get("distro").map(value_str).as_deref(),
        Some("ubuntu"),
        "{session}"
    );
    // Settings → Apply was saved.
    let saved = fs::read_to_string(run.config.join("config.yaml")).expect("config.yaml");
    let cfg = doc::load(&saved).expect("saved yaml");
    assert_eq!(
        cfg.get("launch")
            .and_then(|l| l.get("font_size"))
            .map(value_str)
            .as_deref(),
        Some("15"),
        "{saved}"
    );
}

#[test]
fn terminal_tabs_offscreen() {
    check("u4-terminal", &[]);
}

/// The I/O mode Windows has to use (ConPTY has no descriptor to watch), here on Linux through
/// `TERMIGATOR_PTY_FEED`: a reader thread feeds the session, input and resizes go back through Rust.
#[test]
fn terminal_tabs_offscreen_feed_mode() {
    check("u4-terminal-feed", &[("TERMIGATOR_PTY_FEED", "1")]);
}
