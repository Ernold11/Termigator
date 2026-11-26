//! The toolbar's quick-connect history offscreen (`qml/shell/QuickConnectSmoke.qml`): it is read
//! from config.yaml at start, works with the keyboard and the mouse, and what the check removed
//! is gone from the saved file. A password-shaped entry in an old list is never shown nor saved
//! again.

mod common;

use std::fs;

use termigator as _;

#[test]
fn quick_connect_history_by_keyboard_and_mouse() {
    let log = common::run_check(
        "quick-connect",
        "window:\n  width: 900\nquick_history:\n- u@new\n- 'root:pw@leak'\n- u@old\n- web prod\n\
         - u@new\nconnections:\n- {label: web prod, host: w.example, user: u}\n",
        "shell/QuickConnectSmoke.qml",
    );
    assert!(log.contains("termigator quick: ok"), "stderr:\n{log}");
    let dir = std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join("termigator-quick-connect")
        .join("config");
    let saved = fs::read_to_string(dir.join("config.yaml")).expect("config.yaml");
    assert!(saved.contains("quick_history: []"), "not cleared:\n{saved}");
    assert!(!saved.contains("root:pw"), "password kept:\n{saved}");
}
