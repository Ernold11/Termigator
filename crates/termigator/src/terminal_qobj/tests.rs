use super::*;

#[test]
fn bell_sounds_at_most_every_gap() {
    let t0 = Instant::now() + Duration::from_secs(3600); // clear of other tests' bells
    assert!(bell_due(t0));
    assert!(!bell_due(t0 + Duration::from_millis(100)));
    assert!(bell_due(t0 + BELL_GAP));
}
use termigator_core::config::doc;
use termigator_core::session_state::TabIcon;

fn yaml(text: &str) -> Value {
    doc::load(text).expect("test yaml")
}

fn xdg_none() -> XdgDirs {
    XdgDirs {
        home: PathBuf::from("/nonexistent-termigator-home"),
        config_home: None,
        data_home: None,
        data_dirs: Some("/nonexistent-termigator-share".to_owned()),
    }
}

#[test]
fn effects_have_one_json_shape_each() {
    let e = effect_json(&Effect::SetIcon(TabIcon::Disconnected));
    assert_eq!(e, json!({"k": "icon", "v": "network-disconnect"}));
    assert_eq!(
        effect_json(&Effect::SetIcon(TabIcon::None)),
        json!({"k": "icon", "v": ""})
    );
    let e = effect_json(&Effect::Reconnect {
        attempt: 2,
        log: Some("/l".into()),
    });
    assert_eq!(e, json!({"k": "reconnect", "attempt": 2, "log": "/l"}));
    assert_eq!(effect_json(&Effect::CloseTab), json!({"k": "close"}));
}

#[test]
fn look_follows_settings_and_the_entrys_own_font() {
    let lo = yaml(
        "font_family: Hack\nfont_size: 13\ncolor_scheme: Linux\nembedded_pyte_history: 5000\nblinking_cursor: true\n",
    );
    let installed: HashSet<String> = ["Hack".to_owned(), "Fira Code".to_owned()].into();
    let avail = vec!["Linux".to_owned(), "Solarized".to_owned()];
    let j = look_json(&lo, None, &avail, true, &installed, &xdg_none());
    assert_eq!(j["history"], 5000);
    assert_eq!(j["family"], "Hack");
    assert_eq!(j["size"], 13);
    assert_eq!(j["scheme"], "Linux");
    assert_eq!(j["blinking_cursor"], true);
    assert_eq!(j["bracketed_paste"], true);
    assert_eq!(j["confirm_multiline"], true);
    assert_eq!(j["copy_on_select"], false);
    assert_eq!(j["paste_on_middle_click"], true);

    let conn = yaml("font_family: Fira Code\nfont_size: 9\ncolor_scheme: Solarized\n");
    let j = look_json(&lo, Some(&conn), &avail, true, &installed, &xdg_none());
    assert_eq!(
        (j["family"].as_str(), j["size"].as_i64()),
        (Some("Fira Code"), Some(9))
    );
    assert_eq!(j["scheme"], "Solarized");
}

#[test]
fn unlimited_scrollback_and_defaults() {
    let lo = yaml("scrollback_unlimited: true\n");
    let j = look_json(&lo, None, &[], false, &HashSet::new(), &xdg_none());
    assert_eq!(j["history"], -1);
    assert_eq!(j["size"], 11, "no size anywhere: the default");
    assert_eq!(j["scheme"], "", "no schemes to choose from");
    let j = look_json(
        &yaml("{}\n"),
        None,
        &[],
        false,
        &HashSet::new(),
        &xdg_none(),
    );
    assert_eq!(j["history"], 10_000);
    assert_eq!(j["attention"], true);
    assert_eq!(
        (j["opacity"].as_f64(), j["blur"].as_bool()),
        (Some(1.0), Some(false))
    );
}

#[test]
fn look_carries_the_background_setting_for_every_tab() {
    let lo = yaml("background_opacity: 70\nbackground_blur: true\n");
    let conn = yaml("color_scheme: Solarized\n");
    let j = look_json(&lo, Some(&conn), &[], false, &HashSet::new(), &xdg_none());
    assert_eq!(
        (j["opacity"].as_f64(), j["blur"].as_bool()),
        (Some(0.7), Some(true))
    );
}

#[test]
fn distro_goes_into_the_equal_saved_entry_only() {
    let mut cfg =
        yaml("connections:\n- {name: a, host: h1}\n- {name: b, host: h2, distro: debian}\n");
    assert!(set_distro_in(
        &mut cfg,
        &yaml("{name: a, host: h1}\n"),
        "ubuntu"
    ));
    assert!(set_distro_in(
        &mut cfg,
        &yaml("{name: b, host: h2, distro: debian}\n"),
        "arch"
    ));
    assert!(!set_distro_in(&mut cfg, &yaml("{name: c}\n"), "arch"));
    let conns = cfg.get("connections").expect("connections");
    let Value::Seq(list) = conns else {
        panic!("a list")
    };
    assert_eq!(list[0].get("distro"), Some(&Value::Str("ubuntu".into())));
    assert_eq!(list[1].get("distro"), Some(&Value::Str("arch".into())));
}

#[test]
fn os_errors_read_like_golden() {
    // "Permission denied" is errno 13 on Unix, ERROR_ACCESS_DENIED (5) on Windows.
    let (n, text) = if cfg!(windows) {
        (5, "Access is denied.")
    } else {
        (13, "Permission denied")
    };
    let e = std::io::Error::from_raw_os_error(n);
    assert_eq!(
        os_error_text(&e, Path::new("/x/y.log")),
        format!("[Errno {n}] {text}: '/x/y.log'")
    );
}
