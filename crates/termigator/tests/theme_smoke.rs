//! Light and dark look (Preferences → Appearance → Interface theme): the window comes up in both,
//! and the icon theme it asks for exists. On Windows KDE Craft ships only the light Breeze, and
//! asking for "breeze-dark" left every themed icon blank (tool buttons fell back to cut-off text).

mod common;

fn look(theme: &str) {
    let log = common::run_check(
        &format!("theme-{theme}"),
        &format!("window: {{ui_theme: {theme}}}\n"),
        "shell/LookCheck.qml",
    );
    assert!(
        !log.lines()
            .any(|l| l.contains("Icon theme") && l.contains("not found")),
        "{theme}: missing icon theme\n{log}"
    );
}

#[test]
fn light_theme_finds_its_icons() {
    look("light");
}

#[test]
fn dark_theme_finds_its_icons() {
    look("dark");
}

/// Preferences → Colours switches the theme of a running window without a restart; `from` is the
/// theme it starts with, `to` the one chosen (the grab in `shot.bar.png` shows the result).
fn switch(from: &str, to: &str) {
    let mut run = common::launch(
        &format!("theme-switch-{from}-{to}"),
        &format!("window: {{ui_theme: {from}}}\n"),
        &[
            "--smoke-check=shell/ThemeSwitchCheck.qml",
            &format!("--switch-theme={to}"),
        ],
    );
    let status = run.wait(60, |_| {});
    let log = run.log();
    assert!(status.success(), "exit {status}, stderr:\n{log}");
    assert!(
        !log.lines()
            .any(|l| l.contains("Icon theme") && l.contains("not found")),
        "missing icon theme\n{log}"
    );
    let saved = std::fs::read_to_string(run.config.join("config.yaml")).expect("config");
    assert!(saved.contains(&format!("ui_theme: {to}")), "{saved}");
}

#[test]
fn light_switches_to_dark_without_a_restart() {
    switch("light", "dark");
}

#[test]
fn dark_switches_to_light_without_a_restart() {
    switch("dark", "light");
}
