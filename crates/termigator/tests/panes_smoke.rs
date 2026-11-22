//! `qml/panes/PanesSmoke.qml` offscreen, twice. The first run drives tabs, splits, the active
//! pane, rename, the tab keys and menus, detach / attach and closing, then quits through the
//! close path; this side checks the `last_session` it saved (no password). The second run
//! starts on that config and checks that the layout came back.

mod common;

use std::fs;

use termigator_core::config::doc::{self, Value};
use termigator_core::config::model::value_str;

// cxx-qt's generated C++ is linked into this test too; it needs the library's Rust half.
use termigator as _;

const CONFIG: &str = "window:\n  session_restore: true\n  confirm_close: false\n  confirm_close_tab: false\n  width: 1000\n  \
                      height: 700\n";
const STRESS_CONFIG: &str =
    "window:\n  confirm_close: false\n  confirm_close_tab: false\n  width: 1000\n  height: 700\n";
const CHECK: &str = "--smoke-check=panes/PanesSmoke.qml";

/// The titles of a snapshot's tab list.
fn titles(tabs: Option<&Value>) -> Vec<String> {
    match tabs {
        Some(Value::Seq(tabs)) => tabs
            .iter()
            .map(|t| t.get("title").map(value_str).unwrap_or_default())
            .collect(),
        _ => Vec::new(),
    }
}

#[test]
fn panes_tabs_detach_and_restore_offscreen() {
    let mut run = common::launch("u3-panes", CONFIG, &[CHECK]);
    // The scenario moves ~20 tabs between two windows; on a loaded machine it can pass 60 s
    // (the check itself has a 10 s per-step watchdog), so give the process room.
    let status = run.wait(120, |_| {});
    let log = run.log();
    assert!(status.success(), "exit {status}, stderr:\n{log}");
    assert!(
        log.contains("termigator u3: closed detached"),
        "quitting did not end the detached tab, stderr:\n{log}"
    );
    let shot = fs::metadata(&run.shot).map(|m| m.len()).unwrap_or(0);
    assert!(shot > 0, "no grab at {}", run.shot.display());

    let saved = fs::read_to_string(run.config.join("config.yaml")).expect("config.yaml");
    assert!(
        !saved.contains("secret"),
        "a password in the snapshot:\n{saved}"
    );
    let cfg = doc::load(&saved).expect("saved yaml");
    let snap = cfg.get("last_session").expect("last_session saved");
    assert_eq!(
        snap.get("split").map(value_str).as_deref(),
        Some("h"),
        "{saved}"
    );
    let Some(Value::Seq(kids)) = snap.get("children") else {
        panic!("no split children:\n{saved}");
    };
    assert_eq!(kids.len(), 2, "{saved}");
    assert_eq!(titles(kids[0].get("tabs")), ["Local shell", "gamma", "web"]);
    assert_eq!(kids[0].get("main"), Some(&Value::Bool(true)));
    let Some(Value::Seq(left)) = kids[0].get("tabs") else {
        panic!("no tabs");
    };
    let conn = left[2]
        .get("conn")
        .expect("the connection tab keeps its entry");
    assert!(conn.get("host").is_some() && conn.get("password").is_none());
    assert_eq!(
        titles(kids[1].get("tabs")),
        ["beta and a title longer than thirty-two characters"]
    );
    assert_eq!(titles(snap.get("detached")), ["gamma"]);
    let Some(Value::Seq(sizes)) = snap.get("sizes") else {
        panic!("no sizes");
    };
    assert_eq!(sizes.len(), 2);

    let mut again = common::launch("u3-restore", &saved, &[CHECK, "--u3-restore"]);
    let status = again.wait(120, |_| {});
    assert!(status.success(), "exit {status}, stderr:\n{}", again.log());
}

/// Docking tabs onto every side of random panes, 150 times: the app must not crash.
#[test]
fn many_docks_do_not_crash() {
    let mut run = common::launch("u3-stress", STRESS_CONFIG, &[CHECK, "--u3-stress"]);
    let status = run.wait(120, |_| {});
    let log = run.log();
    assert!(status.success(), "exit {status}, stderr:\n{log}");
}
