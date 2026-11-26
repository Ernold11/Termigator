use super::*;
use termigator_core::config::doc;
use termigator_core::config::save::merge_defaults;

fn cfg(yaml: &str) -> Value {
    merge_defaults(&doc::load(yaml).expect("test yaml"))
}

fn map(yaml: &str) -> Value {
    doc::load(yaml).expect("test yaml")
}

fn conn(i: usize) -> EntryRef {
    EntryRef {
        kind: EntryKind::Connection,
        index: i,
    }
}

fn labels(c: &Value, key: &str) -> Vec<String> {
    seq(c, key).iter().map(|e| text(e, "label")).collect()
}

#[test]
fn every_distro_with_a_logo_gets_a_readable_file_url() {
    // The logos are embedded, so this works without a source tree (installed binary).
    for (id, _) in DISTRO_LOGOS {
        let url = distro_logo_url(id);
        let path = url.strip_prefix("file://").unwrap_or_default();
        // file:///C:/… on Windows: the path starts after the third slash.
        let path = if cfg!(windows) {
            path.trim_start_matches('/')
        } else {
            path
        };
        assert!(std::path::Path::new(path).is_file(), "{id}: {url}");
    }
    assert!(DISTRO_LOGOS.len() >= 17);
    assert_eq!(distro_logo_url("no-such-distro"), "");
    assert_eq!(distro_logo_url(""), "");
}

#[test]
fn menu_icons_take_the_longest_prefix() {
    assert_eq!(menu_icon("Move up"), Some("go-up"));
    assert_eq!(menu_icon("Move 3 entries to folder…"), Some("go-jump"));
    assert_eq!(menu_icon("Move to group…"), Some("go-jump"));
    assert_eq!(
        menu_icon("Delete folder and contents…"),
        Some("edit-delete")
    );
    assert_eq!(
        menu_icon("Add SSH connection here…"),
        Some("utilities-terminal")
    );
    assert_eq!(menu_icon("Stop recording"), Some("media-playback-stop"));
    assert_eq!(menu_icon("Connect 2 entries"), Some("media-playback-start"));
    assert_eq!(menu_icon("&Folder…"), Some("folder-new"));
    assert_eq!(menu_icon("Default (by protocol / distro)"), None);
    // Every entry of the host-list menus has one (test_context_menus_get_theme_icons).
    for t in [
        "Connect / Run",
        "Edit…",
        "Delete…",
        "Move up",
        "Rename / move folder…",
        "Close tab",
        "Set icon",
        "Split right",
    ] {
        assert!(menu_icon(t).is_some(), "{t}");
    }
}

#[test]
fn distro_badges_and_labels() {
    assert_eq!(distro_badge("ubuntu"), ("#E95420".into(), "U".into()));
    assert_eq!(distro_badge("nixos"), ("#6E7681".into(), "N".into()));
    assert_eq!(distro_badge(""), ("#6E7681".into(), "?".into()));
    assert_eq!(distro_label("pop"), "Pop!_OS");
    assert_eq!(distro_label("nixos"), "nixos");
    assert_eq!(distro_label(""), "Linux");
    assert!(distro_logo_url("../etc").is_empty());
}

#[test]
fn tooltips_follow_the_protocol() {
    let rdp = map("{label: r, protocol: rdp, host: h, user: u, port: 3390}");
    assert_eq!(connection_tooltip(&rdp, None), "u@h:3390\n\nRDP (FreeRDP)");
    let ssh = map(
        "{label: web, host: h, user: u, port: 22, distro: debian, use_mosh: true, tags: [a, b]}",
    );
    assert_eq!(
        connection_tooltip(&ssh, None),
        "u@h\n\nDebian\nmosh\nTags: a, b"
    );
    assert_eq!(
        connection_tooltip(&ssh, Some("bad")),
        "u@h\nmosh\nTags: a, b\n\nbad"
    );
    let bare = map("{label: lonely}");
    assert_eq!(connection_tooltip(&bare, None), "lonely");
    let tel = map("{protocol: telnet, host: sw, port: 23}");
    assert_eq!(
        connection_tooltip(&tel, None),
        "sw\n\nTelnet (not encrypted)"
    );
    let ser = map("{protocol: serial, device: /dev/ttyUSB0}");
    assert!(connection_tooltip(&ser, None).ends_with("\n\nSerial console"));
}

// Windows has no external terminal (terminal_mode_for_entry), which this test relies on.
#[cfg(not(windows))]
#[test]
fn rows_carry_icons_tooltips_and_errors() {
    let c = cfg(
        "connections:\n- {label: web, host: h, user: u, distro: ubuntu}\n\
             - {label: desk, protocol: vnc, host: 10.0.0.8, ssh_gateway: gone}\n\
             favorites:\n- {label: top, command: htop, terminal: external}\n",
    );
    let (views, status, hint) = build_views(&c, &TreeState::default(), "", &BTreeSet::new());
    assert_eq!(status, "3 entries");
    assert_eq!(hint, "");
    let web = &views[0];
    assert_eq!(web.icon, "");
    assert_eq!(web.distro.as_ref().map(|d| d.0.as_str()), Some("#E95420"));
    assert!(web.tooltip.ends_with("\n\nUbuntu"));
    let desk = &views[1];
    assert!(desk.row.error);
    assert_eq!(desk.icon, "dialog-warning");
    assert!(
        desk.tooltip
            .ends_with("SSH gateway 'gone' is not a saved SSH entry."),
        "{}",
        desk.tooltip
    );
    let fav = &views[2];
    assert_eq!(fav.row.badge, "ext");
    assert_eq!(fav.tooltip, "Command:\nhtop\n\nTerminal: external");
}

#[test]
fn search_collapse_is_transient() {
    let c = cfg(
        "connections:\n- {label: a, host: h, user: u, group: lab}\n- {label: b, host: h2, user: u}\n",
    );
    let collapsed: BTreeSet<String> = ["lab".to_owned()].into();
    let (views, ..) = build_views(&c, &TreeState::default(), "h", &collapsed);
    let kinds: Vec<_> = views.iter().map(|v| (v.row.kind, v.row.expanded)).collect();
    assert_eq!(
        kinds,
        [(RowKind::Group, false), (RowKind::Connection, false)]
    );
}

#[test]
fn adding_validates_like_golden() {
    let mut c = cfg("{}");
    let need = store_entry(&mut c, "ssh", None, map("{host: h}"));
    assert_eq!(need, Err(Msg::Info("Enter host and user.".into())));
    assert_eq!(
        store_entry(&mut c, "rdp", None, map("{user: u}")),
        Err(Msg::Info("Enter a host.".into()))
    );
    assert_eq!(
        store_entry(&mut c, "serial", None, map("{label: x}")),
        Err(Msg::Info("Enter a device.".into()))
    );
    assert_eq!(
        store_entry(&mut c, "favorite", None, Value::Null),
        Err(Msg::Info("Enter label and command.".into()))
    );
    assert_eq!(
        store_entry(&mut c, "ssh", None, map("{host: h, user: u, group: lab}")),
        Ok(true)
    );
    assert_eq!(labels(&c, "connections"), ["u@h"]);
    assert_eq!(
        store_entry(&mut c, "favorite", None, map("{label: top, command: htop}")),
        Ok(true)
    );
    assert_eq!(labels(&c, "favorites"), ["top"]);
    // A builder error is a warning and nothing is stored.
    let bad = store_entry(
        &mut c,
        "ssh",
        None,
        map("{host: '-oProxyCommand=x', user: u}"),
    );
    assert!(matches!(bad, Err(Msg::Warning(_))), "{bad:?}");
    assert_eq!(seq(&c, "connections").len(), 1);
}

#[test]
fn editing_validates_by_the_stored_protocol() {
    let mut c = cfg(
        "connections:\n- {label: a, host: h, user: u}\n- {label: s, protocol: serial, device: /dev/ttyS0}\n\
             favorites:\n- {label: f, command: ls}\n",
    );
    assert_eq!(
        store_entry(&mut c, "serial", Some(conn(1)), map("{protocol: serial}")),
        Err(Msg::Info("Enter a device.".into()))
    );
    assert_eq!(
        store_entry(&mut c, "ssh", Some(conn(0)), map("{host: h2, user: v}")),
        Ok(true)
    );
    assert_eq!(labels(&c, "connections"), ["v@h2", "s"]);
    let fav = EntryRef {
        kind: EntryKind::Favorite,
        index: 0,
    };
    assert_eq!(
        store_entry(
            &mut c,
            "favorite",
            Some(fav),
            map("{label: g, command: ''}")
        ),
        Err(Msg::Info("Enter label and command.".into()))
    );
    assert_eq!(
        store_entry(
            &mut c,
            "favorite",
            Some(fav),
            map("{label: g, command: df}")
        ),
        Ok(true)
    );
    assert_eq!(labels(&c, "favorites"), ["g"]);
    assert_eq!(
        store_entry(&mut c, "ssh", Some(conn(9)), map("{host: h, user: u}")),
        Ok(false)
    );
}

#[test]
fn gateway_must_be_a_saved_ssh_entry() {
    let c = cfg("connections:\n- {label: desk, protocol: rdp, host: h, ssh_gateway: gone}\n");
    let err = build_conn_command(&c, &seq(&c, "connections")[0]);
    assert_eq!(
        err,
        Err("SSH gateway 'gone' is not a saved SSH entry.".into())
    );
}

#[test]
fn copy_masks_the_password() {
    let c = cfg("credentials:\n- {name: ops, user: root, password: pw}\n\
             connections:\n- {label: web, host: h, credential: ops}\n- {label: plain, host: h, user: u}\n");
    let conns = seq(&c, "connections");
    if vnc::which_in_path("sshpass") {
        let (cmd, notice) = copy_command(&c, &conns[0]).expect("ssh line");
        assert!(cmd.starts_with("SSHPASS='****' sshpass -e "), "{cmd}");
        assert!(cmd.contains("root@h"), "{cmd}");
        assert_eq!(notice, "Copied the SSH command of “web” to the clipboard.");
    }
    let (cmd, _) = copy_command(&c, &conns[1]).expect("ssh line");
    assert!(!cmd.contains("SSHPASS"));
    let broken = cfg("connections:\n- {label: x, host: h}\n");
    assert_eq!(
        copy_command(&broken, &seq(&broken, "connections")[0]),
        Err("Cannot build SSH command:\nHost or user is missing.".into())
    );
}

#[test]
fn nettools_aim_at_the_entry() {
    let web = map("{label: web, host: 10.0.0.5, user: u}");
    let (_, title) = nettool_line(&web, "port").unwrap_or_else(|e| (String::new(), e));
    assert!(
        title == "Check port: 10.0.0.5:22" || title.contains("not installed"),
        "{title}"
    );
    let bad = map("{label: x, host: '-x'}");
    assert!(nettool_line(&bad, "ping").is_err());
}

#[test]
fn reveal_expands_the_folders_or_recent() {
    let c = cfg(
        "connections:\n- {label: a, host: ha, user: u, group: prod/web}\n- {label: b, host: hb, user: u}\n\
             recent: ['hb|u|']\n",
    );
    let mut st = TreeState {
        recent_expanded: false,
        ..TreeState::default()
    };
    reveal_entry(&c, &mut st, "", conn(0));
    assert_eq!(
        st.expanded_groups,
        ["prod".into(), "prod/web".into()].into()
    );
    reveal_entry(&c, &mut st, "", conn(1));
    assert!(st.recent_expanded, "the Recent copy comes first");
    let mut quiet = TreeState::default();
    reveal_entry(&c, &mut quiet, "ha", conn(0));
    assert!(quiet.expanded_groups.is_empty());
}

#[test]
fn drops_on_the_dragged_rows_are_refused() {
    let c = cfg(
        "connections:\n- {label: a, host: ha, user: u, group: lab}\n- {label: b, host: hb, user: u}\n\
             - {label: c, host: hc, user: u}\n",
    );
    let st = TreeState {
        expanded_groups: ["lab".to_owned()].into(),
        recent_expanded: true,
    };
    let rows = hosttree::build(&c, &st, "", &()).rows;
    // rows: lab, a, b, c
    assert!(drop_blocked(&rows, 0, &[0], 1), "into the dragged folder");
    assert!(drop_blocked(&rows, 2, &[2, 3], 3), "onto a dragged entry");
    assert!(!drop_blocked(&rows, 2, &[2], 0));
    assert!(!drop_blocked(&rows, 1, &[1], 3));
}

#[test]
fn rows_are_found_again_after_a_rebuild() {
    let c = cfg(
        "connections:\n- {label: a, host: ha, user: u, group: lab}\n- {label: b, host: hb, user: u}\n",
    );
    let closed = hosttree::build(&c, &TreeState::default(), "", &()).rows;
    let open_state = TreeState {
        expanded_groups: ["lab".to_owned()].into(),
        recent_expanded: true,
    };
    let open = hosttree::build(&c, &open_state, "", &()).rows;
    assert_eq!(
        remap_rows(&closed, &open, &[0, 1, 7]),
        [Some(0), Some(2), None]
    );
    assert_eq!(remap_rows(&open, &closed, &[1]), [None]);
}

#[test]
fn folder_questions_count_and_name_the_destination() {
    assert_eq!(
        delete_group_question("lab", 1),
        "Delete the “lab” folder, all its subfolders and its 1 entry?\nThis cannot be undone."
    );
    assert!(delete_group_question("lab", 0).contains("and no entries?"));
    assert!(delete_group_question("lab", 3).contains("its 3 entries?"));
    assert_eq!(
        dissolve_question("prod/web"),
        "Dissolve the “prod/web” folder and move its contents to “prod”?\n(The entries \
             themselves are kept.)"
    );
    assert!(dissolve_question("lab").contains("to the top level?"));
}

#[test]
fn bulk_changes_report_like_golden() {
    let mut c = cfg(
        "connections:\n- {label: a, host: ha, user: u}\n- {label: r, protocol: rdp, host: hr}\n\
             - {label: b, host: hb, user: u}\n",
    );
    let mut st = TreeState::default();
    let out = bulk_apply(
        &mut c,
        &mut st,
        &[0, 2],
        &map("{icon: server-database, group: core/db}"),
    );
    assert!(out.changed);
    assert_eq!(out.notice.as_deref(), Some("Changed 2 entries."));
    assert_eq!(st.expanded_groups, ["core".into(), "core/db".into()].into());
    let conns = seq(&c, "connections");
    assert_eq!(text(&conns[0], "icon"), "server-database");
    assert_eq!(text(&conns[1], "icon"), "");
    let out = bulk_apply(&mut c, &mut st, &[1], &map("{proxy_jump: bastion}"));
    assert_eq!(
        out.info.as_deref(),
        Some(
            "1 entry was left partly unchanged — a jump host applies only to SSH, a \
                 credential only to SSH and RDP."
        )
    );
    assert!(!out.changed);
}

#[test]
fn probe_timeout_reads_like_golden() {
    assert_eq!(
        probe_timeout(&cfg("launch: {probe_timeout: 2}\n")),
        Duration::from_secs(2)
    );
    assert_eq!(
        probe_timeout(&cfg("launch: {probe_timeout: '1.5'}\n")),
        Duration::from_millis(1500)
    );
    assert_eq!(
        probe_timeout(&cfg("launch: {probe_timeout: 0}\n")),
        DEFAULT_TIMEOUT
    );
    assert_eq!(
        probe_timeout(&cfg("launch: {probe_timeout: x}\n")),
        DEFAULT_TIMEOUT
    );
    assert_eq!(
        probe_timeout(&cfg("launch: {probe_timeout: -3}\n")),
        DEFAULT_TIMEOUT
    );
    assert_eq!(
        probe_timeout(&cfg("launch: {probe_timeout: true}\n")),
        Duration::from_secs(1)
    );
}

#[test]
fn probe_report_marks_the_steps() {
    let c = map("{protocol: serial, device: /dev/nonexistent-termigator}");
    let j: serde_json::Value =
        serde_json::from_str(&report_json(&probe_connection(&c, DEFAULT_TIMEOUT, false)))
            .expect("json");
    assert!(j["headline"].as_str().unwrap_or("").starts_with("✘  "));
    assert!(
        j["steps"][0]["check"]
            .as_str()
            .unwrap_or("")
            .starts_with("✘  Device")
    );
}

#[test]
fn tab_save_plan_drops_secrets_and_finds_the_duplicate() {
    let c = cfg(
        "connections:\n  - label: prod-db\n    host: db.example.test\n    user: admin\n    password: hunter2\n",
    );
    let fresh = tab_save_plan(
        &c,
        &map("label: web\nhost: web.example.test\nuser: root\npassword: pw\n"),
    );
    assert_eq!(fresh["kind"], "ssh");
    assert_eq!(fresh["existing"], -1);
    assert!(fresh["merged"].is_null());
    assert!(fresh["entry"].get("password").is_none());
    assert_eq!(fresh["entry"]["host"], "web.example.test");

    let dup = tab_save_plan(
        &c,
        &map("label: admin@db.example.test\nhost: db.example.test\nuser: admin\nport: 22\n"),
    );
    assert_eq!(dup["existing"], 0);
    assert_eq!(dup["existingLabel"], "prod-db");
    assert_eq!(dup["merged"]["label"], "prod-db");
    // The stored entry's own password stays in the update.
    assert_eq!(dup["merged"]["password"], "hunter2");
    assert!(dup["entry"].get("password").is_none());

    assert_eq!(dup["mergedKind"], "ssh");

    let telnet = tab_save_plan(&c, &map("label: sw\nprotocol: telnet\nhost: 10.0.0.9\n"));
    assert_eq!(telnet["kind"], "telnet");
}
