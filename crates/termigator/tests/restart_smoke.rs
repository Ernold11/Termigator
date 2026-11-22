//! Preferences → Restart now (a new interface style): the window quits, a new process starts and
//! reopens the open tabs although `window.session_restore` is off.

mod common;

use std::time::{Duration, Instant};

#[test]
fn restart_reopens_the_tabs() {
    let mut run = common::launch(
        "restart",
        "window: {confirm_close: false}\nlaunch: {shell_executable: /bin/sh}\n",
        &["--smoke-check=shell/RestartCheck.qml"],
    );
    let status = run.wait(60, |_| {});
    assert!(status.success(), "exit {status}, stderr:\n{}", run.log());
    // The new process shares the log (inherited stderr) and quits by itself.
    let deadline = Instant::now() + Duration::from_secs(60);
    while !run.log().contains("restart: restored") && !run.log().contains("restart: timed out") {
        assert!(
            Instant::now() < deadline,
            "no restarted process:\n{}",
            run.log()
        );
        std::thread::sleep(Duration::from_millis(200));
    }
    let log = run.log();
    assert!(log.contains("restart: restored 1"), "{log}");
    let saved = std::fs::read_to_string(run.config.join("config.yaml")).expect("config");
    assert!(
        saved.contains("last_session"),
        "snapshot kept for the restart:\n{saved}"
    );
}
