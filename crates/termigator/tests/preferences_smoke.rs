//! The settings dialog in the real binary, offscreen (`qml/preferences/PreferencesSmoke.qml`):
//! pages, change tracking, Apply / Restore Defaults / Cancel / OK, and what ends up in
//! `config.yaml` (including the comments of the user's file).

mod common;

use std::fs;

// cxx-qt's generated C++ is linked into this test too; it needs the library's Rust half.
use termigator as _;

#[test]
fn settings_dialog_edits_and_saves_the_config() {
    let yaml =
        "# my notes\nwindow:\n  width: 800 # keep\nlaunch:\n  term: screen\nconnections: []\n";
    let mut run = common::launch(
        "u6-preferences",
        yaml,
        &["--smoke-check=preferences/PreferencesSmoke.qml"],
    );
    let status = run.wait(90, |_| {});
    let log = run.log();
    assert!(status.success(), "exit {status}, stderr:\n{log}");
    assert!(
        log.contains("termigator prefs smoke: done"),
        "stderr:\n{log}"
    );
    assert!(
        fs::metadata(&run.shot).map(|m| m.len()).unwrap_or(0) > 0,
        "no grab of the dialog"
    );

    let saved = fs::read_to_string(run.config.join("config.yaml")).expect("config.yaml");
    assert!(saved.contains("# my notes"), "comment lost:\n{saved}");
    assert!(saved.contains("# keep"), "inline comment lost:\n{saved}");
    assert!(saved.contains("term: screen"), "other key lost:\n{saved}");
    assert!(saved.contains("font_size: 2"), "font size:\n{saved}");
    assert!(
        saved.contains("color_scheme: Solarized"),
        "scheme:\n{saved}"
    );
    // Fusion ships with Qt, so the smoke can always pick it.
    assert!(saved.contains("quick_style: Fusion"), "style:\n{saved}");
    assert!(saved.contains("shell: /bin/zsh"), "shell:\n{saved}");
    assert!(
        saved.contains("scrollback_unlimited: true"),
        "scrollback:\n{saved}"
    );
}
