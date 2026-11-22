use super::*;

#[test]
fn tray_icon_prefers_theme_then_file_then_none() {
    let dir = std::env::temp_dir().join(format!("termigator-icon-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    let file = dir.join("termigator-icon.png");
    std::fs::write(&file, b"png").expect("temp file");
    let missing = dir.join("missing.png");
    assert_eq!(choose_tray_icon(true, &file), TrayIcon::Theme("termigator"));
    assert_eq!(choose_tray_icon(false, &file), TrayIcon::File(file.clone()));
    assert_eq!(
        choose_tray_icon(true, &missing),
        TrayIcon::Theme("termigator")
    );
    assert_eq!(choose_tray_icon(false, &missing), TrayIcon::None);
    let _ = std::fs::remove_dir_all(&dir);
}
use termigator_core::config::doc;
use termigator_core::config::model::value_str;

fn cfg(yaml: &str) -> Value {
    merge_defaults(&doc::load(yaml).expect("test yaml"))
}

#[test]
fn follow_mode_reads_config_like_golden() {
    let on = cfg("window: {follow_konsole: true, follow_poll_ms: 250}\n");
    assert!(follow_wanted(&on, true));
    assert!(!follow_wanted(&on, false)); // no xdotool
    assert!(!follow_wanted(&cfg(""), true)); // off by default
    assert_eq!(follow_interval_ms(&on), 250);
    assert_eq!(follow_interval_ms(&cfg("")), 500);
    assert_eq!(
        follow_interval_ms(&cfg("window: {follow_poll_ms: 0}\n")),
        500
    );
    assert_eq!(
        follow_interval_ms(&cfg("window: {follow_poll_ms: -5}\n")),
        1
    );
}

#[test]
fn follow_target_sits_under_the_terminal() {
    // Terminal 700x400 at (5, 6); our frame 300 wide: centred, 2 px below.
    assert_eq!(follow_target((5, 6, 700, 400), 300), Some((205, 408)));
    // Wider than the terminal: never left of its edge.
    assert_eq!(follow_target((5, 6, 200, 100), 300), Some((5, 108)));
    assert_eq!(follow_target((-30, 0, 201, 50), 100), Some((20, 52)));
    assert_eq!(follow_target((0, 0, i64::MAX, 1), 1), None);
}

#[test]
fn window_values_read_like_golden() {
    let c = cfg("window: {width: 0, toolbar: false, height: '700'}\n");
    assert_eq!(window_int(&c, "width", 900), 900); // `or` default for a falsy 0
    assert_eq!(window_int(&c, "height", 560), 700);
    assert!(!window_bool(&c, "toolbar", true));
    assert!(window_bool(&c, "tray_icon", false)); // from the defaults
    assert!(window_bool(&c, "missing", true));
}

#[test]
fn toolbar_setting_is_saved_as_a_bool() {
    let mut c = cfg("{}");
    set_in(&mut c, &["window", "toolbar"], Value::Bool(false));
    assert_eq!(
        section(&c, "window").get("toolbar"),
        Some(&Value::Bool(false))
    );
}

#[test]
fn close_question_lists_what_runs() {
    let c = cfg("{}");
    assert_eq!(close_question(&c, 0, 0, 0), None);
    assert_eq!(
        close_question(&c, 2, 1, 3).as_deref(),
        Some(
            "2 embedded session(s) are still running\n1 file transfer(s) are still in \
                 progress\n3 tunnel(s) will be stopped.\nClose the window and end them?"
        )
    );
    assert_eq!(
        close_question(&c, 0, 0, 1).as_deref(),
        Some("1 tunnel(s) will be stopped.\nClose the window and end them?")
    );
    let quiet = cfg("window: {confirm_close: false}\n");
    assert_eq!(close_question(&quiet, 5, 0, 0), None);
}

#[test]
fn window_state_is_saved_and_last_session_follows_session_restore() {
    let mut c = cfg("last_session: {tabs: []}\n");
    save_window_state(&mut c, 1000, 700, 0);
    assert_eq!(window_int(&c, "width", 0), 1000);
    assert_eq!(window_int(&c, "height", 0), 700);
    assert_eq!(window_int(&c, "list_width", 0), 0); // a hidden sidebar keeps the old width
    assert!(c.get("last_session").is_none());
    let mut keep = cfg("window: {session_restore: true}\nlast_session: {tabs: []}\n");
    save_window_state(&mut keep, 900, 600, 250);
    assert!(keep.get("last_session").is_some());
    assert_eq!(window_int(&keep, "list_width", 0), 250);
}

#[test]
fn window_placement_position_only_when_asked() {
    let mut c = cfg("window: {}\n");
    set_window_placement(&mut c, None, true);
    assert!(window_bool(&c, "maximized", false));
    assert_eq!(window_int(&c, "x", -1), -1);
    set_window_placement(&mut c, Some((-1200, 40)), false);
    assert!(!window_bool(&c, "maximized", true));
    assert_eq!(
        (window_int(&c, "x", 0), window_int(&c, "y", 0)),
        (-1200, 40)
    );
}

#[test]
fn quick_completer_has_history_then_labels_once() {
    let mut c = cfg(
        "quick_history: [root@a, 7, db]\nconnections:\n- {label: db}\n- {user: u, host: h}\n- x\n",
    );
    assert_eq!(quick_completions(&c), ["root@a", "db", "u@h"]);
    for i in 0..12 {
        remember_quick_target(&mut c, &format!("t{i}"));
    }
    remember_quick_target(&mut c, "t5");
    let hist = quick_history(&c);
    assert_eq!(hist.len(), 10);
    assert_eq!(hist[..3], ["t5", "t11", "t10"]);
    assert_eq!(hist.iter().filter(|h| *h == "t5").count(), 1);
}

#[test]
fn dialog_sizes_round_trip_outside_window_dialogs() {
    let mut c = cfg("window: {dialogs: {SnippetsDialog: abc}}\n");
    assert_eq!(dialog_size(&c, "SnippetsDialog"), None);
    set_dialog_size(&mut c, "SnippetsDialog", 700, 455);
    assert_eq!(dialog_size(&c, "SnippetsDialog"), Some((700, 455)));
    // A base64 geometry saved earlier stays untouched (ARCHITECTURE §8).
    let dialogs = section(&c, "window").get("dialogs").expect("kept");
    assert_eq!(
        value_str(dialogs.get("SnippetsDialog").expect("kept")),
        "abc"
    );
}

#[test]
fn first_run_message_names_the_files() {
    let dir = std::env::temp_dir().join(format!("termigator-u1-{}", std::process::id()));
    let paths = Paths::resolve(
        Layout::ACTIVE,
        &|k: &str| (k == "TERMIGATOR_CONFIG_DIR").then(|| dir.clone().into_os_string()),
        Path::new("/nonexistent-root"),
    );
    let msg = first_run_message(&paths).expect("no config yet");
    assert!(msg.starts_with("Welcome to Termigator."));
    assert!(msg.contains(&format!("will be saved in {}.", dir.display())));
    assert!(msg.contains("File → Import"));
    assert!(!msg.contains("repository"));
}

#[test]
fn a_stray_config_in_the_source_tree_is_never_read_or_migrated() {
    let base = std::env::temp_dir().join(format!("termigator-legacy-{}", std::process::id()));
    let (root, cfg) = (base.join("root"), base.join("cfg"));
    std::fs::create_dir_all(&root).expect("root dir");
    std::fs::write(
        root.join("config.yaml"),
        "connections:\n- {label: private, host: 10.9.8.7}\n",
    )
    .expect("legacy file");
    let env = |k: &str| (k == "TERMIGATOR_CONFIG_DIR").then(|| cfg.clone().into_os_string());
    let paths = Paths::resolve(Layout::ACTIVE, &env, &root);
    let doc = save::load(&paths).expect("load");
    assert!(!format!("{doc:?}").contains("10.9.8.7"), "{doc:?}");
    assert!(!cfg.join("config.yaml").exists() && !cfg.join("session.yaml").exists());
    assert!(first_run_message(&paths).is_some());
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn about_names_version_and_toolkit() {
    let html = about_html("0.2.0", "6.12.0");
    assert!(html.starts_with("<h3>Termigator 0.2.0</h3>"));
    assert!(html.contains("Qt 6.12.0 · Ghostty terminal engine"));
    assert!(html.contains("<a href=\"https://github.com/Ernold11/Termigator\">"));
}

#[test]
fn action_json_carries_the_whole_table() {
    let text = actions_json(&|names| names.split('|').next().unwrap_or("").to_owned());
    let v: serde_json::Value = serde_json::from_str(&text).expect("json");
    assert_eq!(v["menus"][0], "&File");
    let list = v["actions"].as_array().expect("actions");
    assert_eq!(list.len(), actions::ACTIONS.len());
    let search = list
        .iter()
        .find(|a| a["id"] == "edit.search_hosts")
        .expect("row");
    assert_eq!(search["icon"], "search");
    assert_eq!(search["keys"][0], "Ctrl+Shift+H");
    let rows: serde_json::Value = serde_json::from_str(&shortcut_rows_json()).expect("json");
    assert!(
        rows.as_array()
            .expect("rows")
            .iter()
            .any(|r| r[1] == "Quit")
    );
}
