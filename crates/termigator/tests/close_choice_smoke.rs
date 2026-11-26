//! The close button's choice (`window.close_action`, `qml/shell/CloseChoiceCheck.qml`) offscreen,
//! with the real system dialog: ask, Cancel, Hide in Tray (with and without "Remember my
//! choice"), Settings back to "Always ask", a remembered Exit; the remembered choice in the next
//! process; and a logout never waiting on the dialog.

mod common;

use termigator as _;

const SHELL: &str = "launch: {shell_executable: /bin/sh}\n";

fn run(name: &str, config: &str, args: &[&str]) -> (String, String) {
    let mut run = common::launch(name, config, args);
    let status = run.wait(60, |_| {});
    let log = run.log();
    assert!(status.success(), "exit {status}, stderr:\n{log}");
    assert!(!log.contains("termigator close: FAILED"), "stderr:\n{log}");
    let saved = std::fs::read_to_string(run.config.join("config.yaml")).expect("config.yaml");
    (log, saved)
}

#[test]
fn the_close_dialog_asks_remembers_and_settings_reset_it() {
    let (log, saved) = run(
        "close-ask",
        SHELL,
        &["--smoke-check=shell/CloseChoiceCheck.qml"],
    );
    // Exit with "Remember my choice": shut down once, the choice stored on disk.
    assert_eq!(
        log.matches("termigator close: shutting down").count(),
        1,
        "stderr:\n{log}"
    );
    assert!(
        log.contains("termigator close: shutting down action=exit asked=false"),
        "stderr:\n{log}"
    );
    assert!(
        saved.contains("close_action: exit"),
        "config.yaml:\n{saved}"
    );

    // 7. The next start: the remembered Exit quits at once, no dialog.
    let (log, _) = run(
        "close-remembered-exit",
        &saved,
        &[
            "--smoke-check=shell/CloseChoiceCheck.qml",
            "--close-remembered",
        ],
    );
    assert!(
        log.contains("termigator close: shutting down action=exit asked=false"),
        "stderr:\n{log}"
    );
}

#[test]
fn a_remembered_tray_choice_hides_without_asking_after_a_restart() {
    let (log, _) = run(
        "close-remembered-tray",
        "window: {close_action: tray}\n",
        &[
            "--smoke-check=shell/CloseChoiceCheck.qml",
            "--close-remembered",
        ],
    );
    assert!(log.contains("termigator close: ok"), "stderr:\n{log}");
    assert!(!log.contains("shutting down"), "stderr:\n{log}");
}

#[test]
fn an_old_close_to_tray_config_still_hides_without_asking() {
    let (log, saved) = run(
        "close-legacy-tray",
        "window: {close_to_tray: true}\n",
        &[
            "--smoke-check=shell/CloseChoiceCheck.qml",
            "--close-remembered",
        ],
    );
    assert!(log.contains("termigator close: ok"), "stderr:\n{log}");
    assert!(
        saved.contains("close_to_tray: true"),
        "config.yaml:\n{saved}"
    );
}

#[test]
fn a_session_end_with_the_window_in_the_tray_still_shuts_down() {
    let (log, _) = run(
        "close-session-end",
        "",
        &[
            "--smoke-check=shell/CloseChoiceCheck.qml",
            "--close-session-end",
        ],
    );
    assert!(
        log.contains("termigator close: shutting down action=ask asked=false"),
        "stderr:\n{log}"
    );
}

#[test]
fn a_session_end_closes_the_window_without_asking() {
    let (log, _) = run(
        "close-session-end-visible",
        "",
        &[
            "--smoke-check=shell/CloseChoiceCheck.qml",
            "--close-session-end-visible",
        ],
    );
    assert!(
        log.contains("termigator close: shutting down action=ask asked=false"),
        "stderr:\n{log}"
    );
}
