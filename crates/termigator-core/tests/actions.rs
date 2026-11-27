//! The action table against the golden table (compat/golden/actions.json).

mod common;

use serde_json::{Value as J, json};
use termigator_core::actions::{
    ACTIONS, Action, Context, MENUS, by_id, conflicts, conflicts_in, menu_actions, shortcut_rows,
    strip_mnemonics, terminal_claims,
};

fn case(name: &str) -> J {
    common::golden("actions")
        .into_iter()
        .find(|c| common::name(c) == name)
        .unwrap_or_else(|| panic!("no golden case {name}"))["expect"]
        .clone()
}

fn item(a: &Action, items: J) -> J {
    let mut v = json!({
        "text": a.text,
        "keys": a.keys,
        "checkable": a.checkable,
        "tip": a.tip,
        "display_only": !a.keys.is_empty() && a.context == Context::DisplayOnly,
        "kind": if a.submenu { "submenu" } else { "action" },
    });
    if a.submenu {
        v["items"] = items;
    }
    v
}

fn items(path: &str) -> J {
    let mut out = Vec::new();
    for a in menu_actions(path) {
        if a.sep_before {
            out.push(json!({"kind": "separator"}));
        }
        let sub = if a.submenu {
            items(&format!("{path}/{}", strip_mnemonics(a.text)))
        } else {
            J::Null
        };
        out.push(item(a, sub));
    }
    J::Array(out)
}

#[test]
fn menu_layout_equals_golden() {
    let rust: Vec<J> = MENUS
        .iter()
        .map(|m| json!({"text": m, "items": items(&strip_mnemonics(m))}))
        .collect();
    assert_eq!(J::Array(rust), case("menus"));
}

#[test]
fn window_actions_outside_menus_equal_golden() {
    let rust: Vec<J> = ACTIONS
        .iter()
        .filter(|a| a.menu.is_empty() && a.context == Context::Window)
        .map(|a| item(a, J::Null))
        .collect();
    assert_eq!(J::Array(rust), case("window_actions_outside_menus"));
}

#[test]
fn shortcut_rows_equal_golden() {
    let rust: Vec<J> = shortcut_rows()
        .into_iter()
        .map(|(m, a, k)| json!([m, a, k]))
        .collect();
    assert_eq!(J::Array(rust), case("shortcut_rows"));
}

#[test]
fn ids_unique_and_lookup_works() {
    for (i, a) in ACTIONS.iter().enumerate() {
        assert!(
            ACTIONS[..i].iter().all(|b| b.id != a.id),
            "duplicate id {}",
            a.id
        );
        assert_eq!(by_id(a.id).map(|b| b.id), Some(a.id));
    }
    assert!(by_id("no.such").is_none());
    assert_eq!(
        by_id("file.quit").map(|a| a.keys),
        Some(&["Ctrl+Shift+Q"][..])
    );
}

const DUP_MNEMONICS: [&str; 4] = [
    "file.quit",
    "edit.copy_conn_command",
    "view.split_right",
    "session.close_tab",
];

#[test]
fn every_menu_path_exists_and_mnemonics_are_unique() {
    let mut paths: Vec<String> = MENUS.iter().map(|m| strip_mnemonics(m)).collect();
    for a in ACTIONS.iter().filter(|a| a.submenu) {
        paths.push(format!("{}/{}", a.menu, strip_mnemonics(a.text)));
    }
    for a in ACTIONS.iter().filter(|a| !a.menu.is_empty()) {
        assert!(
            paths.iter().any(|p| p == a.menu),
            "{}: no menu {}",
            a.id,
            a.menu
        );
    }
    // Qt cycles through entries sharing a mnemonic; these repeats are kept as in the golden table.
    let mut dups = Vec::new();
    for p in &paths {
        let mut seen = Vec::new();
        for a in menu_actions(p) {
            let t = a.text.replace("&&", "");
            if let Some(i) = t.find('&') {
                let c = t[i + 1..].chars().next().map(|c| c.to_ascii_lowercase());
                if seen.contains(&c) {
                    dups.push(a.id);
                }
                seen.push(c);
            }
        }
    }
    assert_eq!(dups, DUP_MNEMONICS);
    let tops: Vec<_> = MENUS
        .iter()
        .map(|m| m.replace('&', "").to_lowercase())
        .collect();
    assert_eq!(tops.len(), 7);
}

#[test]
fn no_shortcut_conflicts() {
    assert_eq!(conflicts(), vec![]);
}

#[test]
fn conflicts_are_detected_only_in_overlapping_contexts() {
    const fn a(id: &'static str, keys: &'static [&'static str], context: Context) -> Action {
        Action {
            id,
            menu: "",
            text: "",
            icon: "",
            keys,
            context,
            sep_before: false,
            checkable: false,
            submenu: false,
            terminal_consumes: false,
            tip: "",
        }
    }
    let t = [
        a("w", &["Ctrl+Shift+W"], Context::Window),
        a("d", &["Ctrl+Shift+W"], Context::DetachedWindow),
        a("h", &["Shift+Ctrl+W", "F2"], Context::HostList),
        a("x", &["F2"], Context::DisplayOnly),
    ];
    let got = conflicts_in(&t);
    assert_eq!(got.len(), 1, "{got:?}");
    assert_eq!((got[0].first, got[0].second), ("w", "h"));
}

#[test]
fn no_action_takes_a_bare_ctrl_letter_from_the_shell() {
    for a in ACTIONS {
        for k in a.keys {
            if a.context == Context::HostList {
                continue; // live only while the host list has focus, never in a terminal
            }
            assert!(!terminal_claims(k), "{}: {k} belongs to the terminal", a.id);
        }
    }
    // The host-list exceptions: Enter, the search, Esc and the editing keys of a file manager.
    let claimed: Vec<_> = ACTIONS
        .iter()
        .filter(|a| a.context == Context::HostList)
        .flat_map(|a| a.keys.iter().copied())
        .filter(|k| terminal_claims(k))
        .collect();
    assert_eq!(
        claimed,
        [
            "Return", "Enter", "Ctrl+F", "Ctrl+L", "Esc", "Ctrl+A", "Ctrl+C", "Ctrl+X", "Ctrl+V",
            "Ctrl+N"
        ]
    );
}

#[test]
fn terminal_routing_table() {
    for k in [
        "Ctrl+A",
        "Ctrl+L",
        "Ctrl+C",
        "Ctrl+Z",
        "Ctrl+Space",
        "Ctrl+@",
        "Ctrl+[",
        "Ctrl+\\",
        "Ctrl+]",
        "Ctrl+^",
        "Ctrl+_",
        "Ctrl+Shift+_",
        "Ctrl+Shift+@",
        "Ctrl+Shift+^",
        "Tab",
        "Return",
        "Enter",
        "Backspace",
        "Esc",
    ] {
        assert!(terminal_claims(k), "{k} must go to the terminal");
    }
    for k in [
        "Ctrl+Shift+A",
        "Ctrl+Shift+L",
        "Ctrl+Shift+Space",
        "Ctrl+Alt+S",
        "Ctrl+Alt+A",
        "Alt+1",
        "Alt+Up",
        "Ctrl+Tab",
        "Ctrl+Shift+Tab",
        "Ctrl+PgUp",
        "Ctrl+F4",
        "F11",
        "F2",
        "Delete",
        "Ctrl+0",
        "Ctrl+-",
        "Ctrl++",
        "Ctrl+=",
        "Ctrl+(",
        "Ctrl+)",
        "Ctrl+,",
        "Ctrl+Shift+,",
        "Shift+Ins",
        "Shift+F10",
        "Meta+A",
        "Shift+A",
        "Ctrl+Shift+F11",
    ] {
        assert!(!terminal_claims(k), "{k} must reach the application");
    }
}

#[test]
fn spec_alternatives_are_present() {
    let keys = |id: &str| by_id(id).map(|a| a.keys.to_vec());
    assert_eq!(
        keys("view.split_right"),
        Some(vec!["Ctrl+(", "Ctrl+Shift+E"])
    );
    assert_eq!(
        keys("view.split_down"),
        Some(vec!["Ctrl+)", "Ctrl+Shift+D"])
    );
    assert_eq!(keys("view.next_tab"), Some(vec!["Ctrl+Tab", "Ctrl+PgDown"]));
    assert_eq!(
        keys("session.close_tab"),
        Some(vec!["Ctrl+Shift+W", "Ctrl+F4"])
    );
    assert_eq!(
        keys("settings.configure"),
        Some(vec!["Ctrl+Shift+,", "Ctrl+,"])
    );
    assert_eq!(keys("tabs.goto_last"), Some(vec!["Alt+9"]));
}

/// Every action that appears in a menu or on the toolbar has a theme icon.
#[test]
fn every_menu_action_has_an_icon() {
    let bare: Vec<_> = ACTIONS
        .iter()
        .filter(|a| !a.menu.is_empty() && !a.text.is_empty() && a.icon.is_empty())
        .map(|a| a.id)
        .collect();
    assert!(bare.is_empty(), "menu actions without an icon: {bare:?}");
}

/// KDE's Show Menubar key; MultiExec moved off it to Ctrl+Alt+M (also in detached windows).
#[test]
fn menubar_takes_ctrl_shift_m_multiexec_moves_to_ctrl_alt_m() {
    let m = by_id("view.menubar").expect("menubar");
    assert_eq!(
        (m.menu, m.keys, m.checkable),
        ("View", &["Ctrl+Shift+M"][..], true)
    );
    assert_eq!(
        by_id("session.multiexec").map(|a| a.keys),
        Some(&["Ctrl+Alt+M"][..])
    );
    assert_eq!(
        by_id("detached.multiexec").map(|a| a.keys),
        Some(&["Ctrl+Alt+M"][..])
    );
    assert!(!terminal_claims("Ctrl+Alt+M") && !terminal_claims("Ctrl+Shift+M"));
}

/// The ☰ shows the top-level menus as submenus with an icon each, as Konsole's does.
#[test]
fn every_top_level_menu_has_an_icon() {
    use termigator_core::actions::menu_icon;
    for title in MENUS {
        assert!(!menu_icon(title).is_empty(), "no icon for {title}");
    }
    assert_eq!(menu_icon("&Help"), "help-contents");
    assert_eq!(menu_icon("&Nope"), "");
}
