use super::*;
use termigator_core::config::doc;
use termigator_core::config::save::merge_defaults;

fn cfg(yaml: &str) -> Value {
    merge_defaults(&doc::load(yaml).expect("test yaml"))
}

fn rec(title: &str, inner: &str) -> TabRecord {
    TabRecord {
        title: title.to_owned(),
        conn: None,
        inner: inner.to_owned(),
    }
}

#[test]
fn local_shell_like_golden() {
    let c = cfg("window: {default_terminal: konsole}\nlaunch: {shell: zsh}\n");
    let which = |n: &str| (n == "zsh").then(|| "/usr/bin/zsh".to_owned());
    // Windows picks its default shell from its own list (core: `local_shell::windows_*`).
    if cfg!(unix) {
        assert_eq!(
            local_shell(&c, "", "/bin/bash", &which, &[]),
            (
                "/usr/bin/zsh -l".to_owned(),
                "Local shell".to_owned(),
                "external"
            )
        );
    }
    let plain = cfg("{}");
    assert_eq!(
        local_shell(&plain, "/opt/my shells/fish", "", &|_| None, &[]),
        (
            "'/opt/my shells/fish' -l".to_owned(),
            "fish".to_owned(),
            "embedded"
        )
    );
    // A menu id (Windows) runs its own command line under its label.
    let ps = LocalShell {
        id: "powershell".into(),
        label: "Windows PowerShell".into(),
        inner: Some("'C:\\ps.exe'".into()),
    };
    assert_eq!(
        local_shell(&plain, "powershell", "", &|_| None, &[ps]),
        (
            "'C:\\ps.exe'".to_owned(),
            "Windows PowerShell".to_owned(),
            "embedded"
        )
    );
    assert!(is_external("Konsole") && is_external(" window "));
    assert!(!is_external("") && !is_external("embedded") && !is_external("tab"));
}

#[test]
fn external_terminal_errors_are_golden() {
    let c = cfg("launch: {template: 'nosuchterm -e {cmd}'}\n");
    let err = external_terminal_argv(section(&c, "launch"), "htop", &|_| None);
    assert_eq!(
        err,
        Err(
            "`nosuchterm` not found in PATH. Install Konsole (KDE) or set launch.template \
             to another terminal emulator."
                .to_owned()
        )
    );
    let env = env_map("test", r#"{"SSHPASS": "s", "N": 1}"#);
    assert_eq!(env.len(), 1);
    assert!(env_map("test", "").is_empty() && env_map("test", "[1]").is_empty());
}

#[test]
fn records_from_qml() {
    let r = parse_records(
        "test",
        r#"{"3": {"title": "web", "conn": {"host": "h", "password": "p"}, "inner": ""},
            "4": {"title": "sh", "conn": null, "inner": "bash -l"}, "x": {}, "5": 7}"#,
    );
    assert_eq!(r.len(), 3);
    assert_eq!(r[&4], rec("sh", "bash -l"));
    assert!(r[&3].conn.as_ref().is_some_and(|c| c.get("host").is_some()));
    assert_eq!(r[&5], rec("", ""));
    assert!(parse_records("test", "nonsense").is_empty());
}

/// Closing saves a snapshot only when enabled, the snapshot reopens detached tabs, and the
/// window restores the last session on start, all through the config.
#[test]
fn snapshot_saved_only_when_enabled_and_restored() {
    let mut p = Panes::new();
    let (a, b, c) = (p.new_tab_id(), p.new_tab_id(), p.new_tab_id());
    p.add_tab(0, a, None).expect("main");
    p.add_tab(0, b, None).expect("main");
    let right = p
        .split(0, Orientation::Horizontal, true, 800)
        .expect("split");
    p.add_tab(right, c, None).expect("right");
    p.detach_tab(c).expect("detach");
    let mut records = HashMap::new();
    records.insert(a, rec("alpha", "sleep 300"));
    records.insert(
        b,
        TabRecord {
            title: "web".to_owned(),
            conn: Some(doc::load("{host: h, user: u, password: secret}").expect("yaml")),
            inner: String::new(),
        },
    );
    records.insert(c, rec("", "htop"));

    let mut off = cfg("last_session: {tabs: []}\nzz: 1\n");
    save_snapshot(&mut off, &p, &records);
    assert!(off.get("last_session").is_none());
    assert!(restore_session(&cfg("last_session: {tabs: []}\n"), false).is_none());
    // The restart after a style change reopens the tabs without session_restore.
    assert!(restore_session(&cfg("last_session: {tabs: []}\n"), true).is_some());

    let mut on = cfg("window: {session_restore: true}\nlast_session: old\nzz: 1\n");
    save_snapshot(&mut on, &p, &records);
    let text = doc::dump(&on);
    assert!(!text.contains("secret"), "{text}");
    let Value::Map(pairs) = &on else {
        panic!("a mapping")
    };
    let keys: Vec<String> = pairs.iter().map(|(k, _)| doc::dump(k)).collect();
    let pos = |name| keys.iter().position(|k| k.starts_with(name)).expect(name);
    assert!(pos("last_session") < pos("zz"), "kept in place: {keys:?}");

    let restored = restore_session(&on, false).expect("restore on");
    let tabs = restore_json(&restored.tabs);
    assert_eq!(tabs[0]["title"], "alpha");
    assert_eq!(tabs[0]["inner"], "sleep 300");
    assert_eq!(tabs[1]["entry"]["host"], "h");
    assert_eq!(tabs[1]["matched"], J::Null);
    assert!(tabs[1].get("inner").is_none());
    assert_eq!(tabs[2]["title"], "Session"); // an empty title is stored as "Session"
    assert_eq!(tabs[2]["detached"], true);
    assert_eq!(restored.panes.list_panes().len(), 2);
    assert_eq!(restored.panes.windows().len(), 1);
}

fn entry(yaml: &str) -> Value {
    doc::load(yaml).expect("test entry")
}

fn open(plan: Plan) -> (String, String, Vec<(String, Secret)>, Option<String>, bool) {
    match plan {
        Plan::Open {
            inner,
            title,
            env,
            notice,
            distro,
            error,
            ..
        } => (inner, title + &error, env, notice, distro),
        other => panic!("not opened: {other:?}"),
    }
}

/// Opening a favorite or a connection: favorites, files-only, X11 notice, SSH env, the vault,
/// the RDP gateway.
#[test]
fn connection_plans_like_golden() {
    let c = cfg("window: {default_terminal: konsole}\n");
    let fav = connection_plan(
        &c,
        &entry("{command: '  htop  ', terminal: embedded}"),
        None,
        None,
        None,
    );
    let Plan::Open {
        inner,
        title,
        mode,
        conn,
        ..
    } = &fav
    else {
        panic!("{fav:?}")
    };
    assert_eq!(
        (inner.as_str(), title.as_str(), *mode, *conn),
        ("htop", "Favorite", "embedded", false)
    );
    let files = entry("{host: h, files_only: true}");
    assert_eq!(connection_plan(&c, &files, None, None, None), Plan::Files);

    let ssh =
        entry("{label: web, host: h, user: u, x11_forwarding: true, remote_locale: de_DE.UTF-8}");
    let (inner, title, env, notice, distro) = open(connection_plan(&c, &ssh, None, None, None));
    assert!(
        inner.contains("u@h") && title == "web" && distro,
        "{inner} {title}"
    );
    assert!(notice.is_some_and(|n| n.starts_with("X11 forwarding is on, but DISPLAY")));
    assert_eq!(env.len(), 2);
    let (.., notice, _) = open(connection_plan(&c, &ssh, None, Some(":0"), None));
    assert_eq!(notice, None);

    let with_pw = entry("{host: h, user: u, password: s3}");
    let (_, title, env, ..) = open(connection_plan(&c, &with_pw, None, None, None));
    assert!(title.starts_with("u@h"));
    let want = if cfg!(windows) {
        // Windows: the askpass channel, one token per password.
        termigator_core::proto::ssh::ssh_password_env(&doc::load("password: s3").expect("yaml"))
            .expect("askpass env")
    } else {
        vec![("SSHPASS".to_owned(), Secret::new(b"s3".to_vec()))]
    };
    assert_eq!(env, want);

    let sealed = entry("{host: h, user: u, password: 'enc:v1:AAAA'}");
    let text = "Cannot decrypt the saved password:\nThe vault is locked.".to_owned();
    assert_eq!(
        connection_plan(&c, &sealed, None, None, None),
        Plan::Stop(Stop {
            text: text.clone(),
            locked: false
        })
    );
    let vaulted = cfg("vault: {salt: abc, check: def}\n");
    assert_eq!(
        connection_plan(&vaulted, &sealed, None, None, None),
        Plan::Stop(Stop { text, locked: true })
    );

    // The RDP start size: the window's screen unless the host or Settings say otherwise (FreeRDP
    // argv; mstsc gets it in the .rdp file, `proto::rdp` tests).
    if !cfg!(windows) {
        let plain = entry("{protocol: rdp, host: w, user: u}");
        let (inner, ..) = open(connection_plan(&c, &plain, None, None, Some("2560x1440")));
        assert!(inner.contains("/size:2560x1440"), "{inner}");
        let (inner, ..) = open(connection_plan(&c, &plain, None, None, None));
        assert!(
            !inner.contains("/size:") && !inner.contains("/f"),
            "{inner}"
        );
        let own = entry("{protocol: rdp, host: w, user: u, resolution: 1280x720}");
        let (inner, ..) = open(connection_plan(&c, &own, None, None, Some("2560x1440")));
        assert!(
            inner.contains("/size:1280x720") && !inner.contains("2560"),
            "{inner}"
        );
        let fixed = cfg("launch: {rdp_resolution: fullscreen}\n");
        let (inner, ..) = open(connection_plan(
            &fixed,
            &plain,
            None,
            None,
            Some("2560x1440"),
        ));
        assert!(inner.contains(" /f"), "{inner}");
    }
    let rdp = entry("{protocol: rdp, host: w, ssh_gateway: jump}");
    let (inner, title, ..) = open(connection_plan(&c, &rdp, None, None, None));
    // Windows (mstsc) refuses an SSH gateway for RDP before looking the gateway up.
    let why = if cfg!(windows) {
        "An SSH gateway for RDP is not supported on Windows yet."
    } else {
        "SSH gateway 'jump' is not a saved SSH entry."
    };
    assert_eq!(
        (inner.as_str(), title.as_str()),
        ("", format!("w{why}").as_str())
    );
}

/// Quick connect and reopening a tab record.
#[test]
fn quick_connect_and_reopen() {
    let c = cfg("connections: [{label: Straße, host: s}]\n");
    assert_eq!(
        quick_plan(&c, " STRASSE "),
        Quick::Entry(entry("{label: Straße, host: s}"))
    );
    assert_eq!(
        quick_plan(&c, ""),
        Quick::Error("Enter user@host.".to_owned())
    );
    assert!(matches!(quick_plan(&c, "nohost"), Quick::Error(e) if e.starts_with("Use user@host")));
    let Quick::Open {
        inner,
        title,
        mode,
        conn,
    } = quick_plan(&c, "root@10.0.0.5:2222")
    else {
        panic!("no session")
    };
    assert!(
        inner.contains("root@10.0.0.5") && inner.contains("2222"),
        "{inner}"
    );
    assert_eq!((title.as_str(), mode), ("SSH root@10.0.0.5", "embedded"));
    assert_eq!(
        conn.get("label"),
        Some(&Value::Str("root@10.0.0.5".to_owned()))
    );
    assert_eq!(conn.get("port"), Some(&Value::Int(2222)));

    let (cmd, env) = reopen_plan(
        &c,
        &entry("{host: h, user: u, remote_locale: C.UTF-8}"),
        None,
    )
    .expect("ssh reopens");
    assert!(cmd.contains("u@h") && env.len() == 2, "{cmd} {env:?}");
    assert_eq!(reopen_plan(&c, &entry("{protocol: telnet}"), None), None);
    let sealed = entry("{host: h, user: u, password: 'enc:v1:AAAA'}");
    assert!(reopen_plan(&c, &sealed, None).is_none_or(|(_, env)| env.is_empty()));
    // Only a configured vault asks; a plain entry and a telnet entry never do.
    let vaulted = cfg("vault: {salt: abc, check: def}\n");
    assert!(reopen_locked(&vaulted, &sealed, None));
    assert!(!reopen_locked(&c, &sealed, None));
    assert!(!reopen_locked(
        &vaulted,
        &entry("{host: h, password: p}"),
        None
    ));
    let telnet = entry("{protocol: telnet, host: h, password: 'enc:v1:AAAA'}");
    assert!(!reopen_locked(&vaulted, &telnet, None));
}
