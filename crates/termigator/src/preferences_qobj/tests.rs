use super::*;
use termigator_core::config::doc;
use termigator_core::config::model::TERMINAL_EXTERNAL;
use termigator_core::config::save::merge_defaults;

fn cfg(yaml: &str) -> Value {
    merge_defaults(&doc::load(yaml).expect("test yaml"))
}

#[test]
fn ui_theme_defaults_to_auto_and_accepts_light_and_dark() {
    assert_eq!(configured_ui_theme(&cfg("window: {}")), "auto");
    assert_eq!(
        configured_ui_theme(&cfg("window: {ui_theme: ' Dark '}")),
        "dark"
    );
    assert_eq!(
        configured_ui_theme(&cfg("window: {ui_theme: light}")),
        "light"
    );
    assert_eq!(
        configured_ui_theme(&cfg("window: {ui_theme: purple}")),
        "auto"
    );
}

#[test]
fn defaults_fill_the_pages() {
    let v = page_values(&default_config());
    assert_eq!(v["font_size"], 0);
    assert_eq!(v["embedded_pyte_history"], 10_000);
    assert_eq!(v["template"], "konsole -e bash -lc {cmd_quoted}");
    assert_eq!(v["session_log_template"], LOG_TEMPLATE);
    assert_eq!(v["default_terminal"], "embedded");
    assert_eq!(v["rdp_client"], "");
    assert_eq!(v["tray_icon"], true);
    assert_eq!(v["notify_on_disconnect"], false);
    assert_eq!(v["quick_style"], DEFAULT_QUICK_STYLE);
}

#[test]
fn style_choice_is_stored_and_read() {
    assert_eq!(configured_style(&cfg("{}")), "org.kde.desktop");
    assert_eq!(
        configured_style(&cfg("window: {quick_style: ' Material '}")),
        "Material"
    );
    // The legacy key no longer does anything.
    assert_eq!(
        configured_style(&cfg("window: {modern_style: false}")),
        "org.kde.desktop"
    );
    // an unchanged page writes nothing
    let mut c = cfg("{}");
    let shown = page_values(&c);
    apply_values(&mut c, &shown);
    assert_eq!(section(&c, "window").get("quick_style"), None);
    // a changed one is stored
    let mut draft = page_values(&c);
    assert_eq!(draft["quick_style"], DEFAULT_QUICK_STYLE);
    draft["quick_style"] = json!("Basic");
    apply_values(&mut c, &draft);
    assert_eq!(
        section(&c, "window").get("quick_style"),
        Some(&Value::Str("Basic".into()))
    );
}

#[test]
fn startup_falls_back_when_the_style_is_not_installed() {
    let avail = vec!["org.kde.desktop".to_owned(), "Fusion".to_owned()];
    let c = cfg("window: {quick_style: Material}");
    assert_eq!(startup_style(&c, &avail), "org.kde.desktop");
    let c = cfg("window: {quick_style: fusion}");
    assert_eq!(startup_style(&c, &avail), "Fusion");
    assert_eq!(startup_style(&cfg("{}"), &[]), "org.kde.desktop");
}

#[test]
fn values_are_read_like_golden() {
    let c = cfg(
        "launch: {font_size: '14', font_family: '  Hack ', ssh_keepalive_seconds: 9999, \
             embedded_pyte_history: 5, probe_timeout: abc, copy_on_select: 1, \
             paste_on_middle_click: false}\n\
             window: {default_terminal: konsole, always_on_top: null}\n",
    );
    let v = page_values(&c);
    assert_eq!(v["font_size"], 14);
    assert_eq!(v["font_family"], "Hack");
    assert_eq!(v["ssh_keepalive_seconds"], 300); // clamped by the spin box
    assert_eq!(v["embedded_pyte_history"], 1_000);
    assert_eq!(v["probe_timeout"], 5); // `int("abc")` fails: default
    assert_eq!(v["copy_on_select"], true);
    assert_eq!(v["paste_on_middle_click"], false);
    assert_eq!(v["default_terminal"], "external");
    assert_eq!(v["always_on_top"], false);
}

#[test]
fn apply_after_load_moves_nothing_that_was_set() {
    // test_settings_round_trip_keeps_the_config
    let mut c = cfg(
        "launch: {shell: /bin/zsh, color_scheme: Breeze, scrollback_unlimited: true}\n\
             window: {tray_icon: false, session_restore: true}\n",
    );
    let before = c.clone();
    let shown = page_values(&c);
    apply_values(&mut c, &shown);
    for sec in ["window", "launch"] {
        let (Some(Value::Map(old)), Some(Value::Map(new))) = (before.get(sec), c.get(sec)) else {
            panic!("section {sec} missing");
        };
        for (k, nv) in new {
            let ov = old.iter().find(|(ok, _)| ok == k).map(|(_, v)| v);
            if ov.is_some_and(truthy) {
                assert_eq!(ov, Some(nv), "{sec}.{k:?} moved");
            }
        }
    }
}

#[test]
fn rdp_start_size_defaults_to_the_screen_and_is_only_stored_when_chosen() {
    let mut c = cfg("{}");
    let mut draft = page_values(&c);
    assert_eq!(draft["rdp_resolution"], "screen");
    // One line per host until asked otherwise; the choice is stored and read back.
    assert_eq!(draft["host_list_detailed"], false);
    draft["host_list_detailed"] = json!(true);
    apply_values(&mut c, &draft);
    assert_eq!(page_values(&c)["host_list_detailed"], true);
    draft["host_list_detailed"] = json!(false);
    apply_values(&mut c, &draft);
    // Untouched: nothing is written.
    apply_values(&mut c, &draft);
    assert!(
        c.get("launch")
            .is_none_or(|l| l.get("rdp_resolution").is_none())
    );
    // A fixed size, full screen, the client's own default (empty) are all kept.
    for (typed, stored) in [
        (" 1920X1080 ", "1920x1080"),
        ("fullscreen", "fullscreen"),
        ("", ""),
    ] {
        draft["rdp_resolution"] = json!(typed);
        apply_values(&mut c, &draft);
        let launch = c.get("launch").expect("launch");
        assert_eq!(
            launch.get("rdp_resolution"),
            Some(&Value::Str(stored.to_owned()))
        );
        assert_eq!(page_values(&c)["rdp_resolution"], stored);
    }
    // Rubbish falls back to the default; once chosen the key stays, so a change back is kept.
    draft["rdp_resolution"] = json!("huge");
    apply_values(&mut c, &draft);
    assert_eq!(
        c.get("launch").and_then(|l| l.get("rdp_resolution")),
        Some(&Value::Str("screen".to_owned()))
    );
}

#[test]
fn apply_writes_expected_shapes() {
    let mut c = cfg("{}");
    let mut draft = page_values(&c);
    draft["font_size"] = json!(16);
    draft["rdp_client"] = json!("  ");
    draft["vnc_client"] = json!(" tigervnc ");
    draft["template"] = json!("");
    draft["shell"] = json!(" /bin/fish ");
    draft["session_log_template"] = json!("");
    draft["default_terminal"] = json!("external");
    apply_values(&mut c, &draft);
    let l = section(&c, "launch");
    assert_eq!(l.get("font_size"), Some(&Value::Int(16)));
    assert_eq!(l.get("rdp_client"), Some(&Value::Null));
    assert_eq!(l.get("vnc_client"), Some(&Value::Str("tigervnc".into())));
    // an empty template keeps the stored one
    assert_eq!(
        l.get("template"),
        Some(&Value::Str("konsole -e bash -lc {cmd_quoted}".into()))
    );
    assert_eq!(l.get("shell"), Some(&Value::Str("/bin/fish".into())));
    assert_eq!(
        l.get("session_log_template"),
        Some(&Value::Str(LOG_TEMPLATE.into()))
    );
    assert_eq!(
        section(&c, "window").get("default_terminal"),
        Some(&Value::Str(TERMINAL_EXTERNAL.into()))
    );
    draft["shell"] = json!("");
    apply_values(&mut c, &draft);
    assert_eq!(
        section(&c, "launch").get("shell"),
        Some(&Value::Str(String::new()))
    );
}

#[test]
fn unrelated_keys_survive_an_apply() {
    let mut c = cfg("window: {width: 1000, mystery: 7}\nlaunch: {term: screen}\nextra: 1\n");
    let shown = page_values(&c);
    apply_values(&mut c, &shown);
    assert_eq!(section(&c, "window").get("width"), Some(&Value::Int(1000)));
    assert_eq!(section(&c, "window").get("mystery"), Some(&Value::Int(7)));
    assert_eq!(
        section(&c, "launch").get("term"),
        Some(&Value::Str("screen".into()))
    );
    assert_eq!(c.get("extra"), Some(&Value::Int(1)));
}

#[test]
fn scheme_list_merges_directories_once_sorted_ignoring_case() {
    let dir = std::env::temp_dir().join(format!("termigator-u6-schemes-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    for f in ["zeta.colorscheme", "Breeze.colorscheme", "notes.txt"] {
        std::fs::write(dir.join(f), "").expect("file");
    }
    let names = scheme_names(
        vec!["Solarized".into(), "Breeze".into()],
        std::slice::from_ref(&dir),
    );
    assert_eq!(names, ["Breeze", "Solarized", "zeta"]);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn quick_history_limit_is_shown_and_stored_only_when_changed() {
    let mut c = cfg("{}");
    let shown = page_values(&c);
    assert_eq!(shown["quick_history_limit"], 20);
    apply_values(&mut c, &shown);
    assert_eq!(section(&c, "window").get("quick_history_limit"), None);
    let mut draft = page_values(&c);
    draft["quick_history_limit"] = json!(5);
    apply_values(&mut c, &draft);
    assert_eq!(page_values(&c)["quick_history_limit"], 5);
    draft["quick_history_limit"] = json!(9999);
    apply_values(&mut c, &draft);
    assert_eq!(page_values(&c)["quick_history_limit"], 200);
}
