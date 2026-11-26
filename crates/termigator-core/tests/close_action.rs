//! The close button's choice (`termigator_core::close_action`): the default, the legacy
//! `close_to_tray` flag, storing a choice and that it survives a restart.

use std::fs;
use std::path::PathBuf;

use termigator_core::close_action::{CloseAction, KEY, LEGACY_KEY, configured, set};
use termigator_core::config::doc::{Value, dump, load as yaml_load};
use termigator_core::config::model::section;
use termigator_core::config::save::{load, save};
use termigator_core::paths::Paths;

fn cfg(text: &str) -> Value {
    yaml_load(text).expect("yaml")
}

#[test]
fn asks_by_default() {
    assert_eq!(configured(&cfg("{}\n")), CloseAction::Ask);
    assert_eq!(
        configured(&cfg("window: {tray_icon: true}\n")),
        CloseAction::Ask
    );
    assert_eq!(configured(&cfg("window: null\n")), CloseAction::Ask);
}

#[test]
fn reads_the_stored_choice() {
    for (text, want) in [
        ("ask", CloseAction::Ask),
        ("tray", CloseAction::Tray),
        ("exit", CloseAction::Exit),
        (" Tray ", CloseAction::Tray),
        ("EXIT", CloseAction::Exit),
    ] {
        let c = cfg(&format!("window: {{close_action: '{text}'}}\n"));
        assert_eq!(configured(&c), want, "{text:?}");
    }
}

#[test]
fn an_unknown_choice_asks() {
    assert_eq!(
        configured(&cfg("window: {close_action: hide}\n")),
        CloseAction::Ask
    );
    assert_eq!(
        configured(&cfg("window: {close_action: 3}\n")),
        CloseAction::Ask
    );
    assert_eq!(
        configured(&cfg("window: {close_action: ''}\n")),
        CloseAction::Ask
    );
}

#[test]
fn an_old_config_with_close_to_tray_keeps_hiding_in_the_tray() {
    assert_eq!(
        configured(&cfg("window: {close_to_tray: true}\n")),
        CloseAction::Tray
    );
    assert_eq!(
        configured(&cfg("window: {close_to_tray: false}\n")),
        CloseAction::Ask
    );
    // The new key wins over the old flag.
    assert_eq!(
        configured(&cfg("window: {close_to_tray: true, close_action: ask}\n")),
        CloseAction::Ask
    );
    assert_eq!(
        configured(&cfg("window: {close_to_tray: true, close_action: bogus}\n")),
        CloseAction::Tray
    );
}

#[test]
fn set_stores_the_choice_and_drops_the_legacy_flag() {
    let mut c = cfg("window:\n  close_to_tray: true\n  width: 900\n");
    set(&mut c, CloseAction::Exit);
    let w = section(&c, "window");
    assert_eq!(w.get(KEY), Some(&Value::Str("exit".to_owned())));
    assert_eq!(w.get(LEGACY_KEY), None);
    assert_eq!(w.get("width"), Some(&Value::Int(900)));
    assert_eq!(configured(&c), CloseAction::Exit);

    // Back to "Always ask": the remembered choice is reset.
    set(&mut c, CloseAction::Ask);
    assert_eq!(configured(&c), CloseAction::Ask);
    assert_eq!(
        section(&c, "window").get(KEY),
        Some(&Value::Str("ask".to_owned()))
    );
}

#[test]
fn set_without_a_change_leaves_the_config_as_written() {
    let mut c = cfg("window: {width: 900}\n");
    let before = dump(&c);
    set(&mut c, CloseAction::Ask);
    assert_eq!(dump(&c), before);

    let mut old = cfg("window: {close_to_tray: true}\n");
    let before = dump(&old);
    set(&mut old, CloseAction::Tray);
    assert_eq!(dump(&old), before);
}

#[test]
fn set_makes_the_window_section_when_missing() {
    let mut c = cfg("{}\n");
    set(&mut c, CloseAction::Tray);
    assert_eq!(configured(&c), CloseAction::Tray);
    let mut odd = cfg("window: [1, 2]\n");
    set(&mut odd, CloseAction::Exit);
    assert_eq!(configured(&odd), CloseAction::Exit);
}

#[test]
fn the_choice_survives_saving_and_loading_the_config() {
    let base = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("close_action_restart");
    let _ = fs::remove_dir_all(&base);
    let dir = base.join("cfg");
    fs::create_dir_all(&dir).expect("mkdir");
    let paths = Paths {
        repo_root: base.clone(),
        config_path: dir.join("config.yaml"),
        session_path: dir.join("session.yaml"),
        config_example_path: dir.join("config.example.yaml"),
        session_example_path: dir.join("session.example.yaml"),
        config_dir: dir,
    };
    for action in [CloseAction::Tray, CloseAction::Exit, CloseAction::Ask] {
        let mut c = load(&paths).expect("load");
        set(&mut c, action);
        save(&paths, &c, false).expect("save");
        assert_eq!(configured(&load(&paths).expect("reload")), action);
    }
}
