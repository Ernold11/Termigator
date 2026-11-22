//! Launches the real binary offscreen in smoke-test mode (`TERMIGATOR_SMOKE_TEST`, see
//! `src/shell_qobj.rs` and `qml/shell/SmokeCheck.qml`) with a private config dir and control
//! socket. The QML side checks menus, toolbar, keys, the shortcut list, the status bar, dialog
//! sizes and the close question; this side sends `toggle` over the socket (control-socket protocol) and checks
//! that the window quit through the close path and saved its geometry.

mod common;

use std::fs;

use termigator_core::single_instance;

// cxx-qt's generated C++ is linked into this test too; it needs the library's Rust half.
use termigator as _;

#[test]
fn shell_window_runs_offscreen() {
    // A config file exists, so no first-run notice.
    let mut run = common::launch("u1-smoke", "window:\n  width: 800\n", &[]);
    let socket = run.socket.clone();
    let mut sent = false;
    let status = run.wait(60, |log| {
        if !sent && log.contains("termigator smoke: ready") {
            let ok = single_instance::send(&socket, "toggle", "").expect("valid command");
            assert!(ok, "the running window did not answer ok");
            sent = true;
        }
    });
    let (config, shot) = (&run.config, &run.shot);
    let log = run.log();

    assert!(status.success(), "exit {status}, stderr:\n{log}");
    assert!(sent, "never ready, stderr:\n{log}");
    assert!(
        log.contains("termigator smoke: remote=toggle"),
        "stderr:\n{log}"
    );
    let size = fs::metadata(shot).map(|m| m.len()).unwrap_or(0);
    assert!(size > 0, "no window grab at {}", shot.display());
    let saved = fs::read_to_string(config.join("config.yaml")).expect("config.yaml");
    assert!(saved.contains("height:"), "geometry not saved:\n{saved}");
    assert!(
        saved.contains("SmokeDialog"),
        "dialog size not saved:\n{saved}"
    );
    assert!(!socket.exists(), "control socket left behind");
}
