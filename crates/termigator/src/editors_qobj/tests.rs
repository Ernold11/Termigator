use termigator_core::config::doc;
use termigator_core::config::save::merge_defaults;

use super::*;

const SEALED: &str = "enc:v1:AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";

fn cfg(yaml: &str) -> Value {
    merge_defaults(&doc::load(yaml).expect("test yaml"))
}

/// What the dialog stores when the user opens `initial` (JSON) and changes the form with `edit`.
fn save(kind: Kind, initial: &J, edit: impl FnOnce(&mut J)) -> J {
    save_in(&cfg("{}"), kind, initial, edit)
}

fn save_in(cfg: &Value, kind: Kind, initial: &J, edit: impl FnOnce(&mut J)) -> J {
    let entry = parse_entry("test", &initial.to_string());
    let mut form = prepare(cfg, kind, entry.as_ref(), "", &[])["form"].clone();
    edit(&mut form);
    build(kind, entry.as_ref(), &form)
        .expect("an entry")
        .to_json()
}

fn keys(v: &J) -> Vec<&str> {
    v.as_object()
        .expect("object")
        .keys()
        .map(String::as_str)
        .collect()
}

fn assert_has_all(out: &J, expected: &J) {
    for (k, v) in expected.as_object().expect("object") {
        assert_eq!(out.get(k), Some(v), "{k}");
    }
}

#[test]
fn ssh_clearing_password_and_key_removes_them() {
    let initial = json!({"label": "x", "host": "h", "user": "u", "password": "pw", "identity_file": "~/.ssh/k"});
    let out = save(Kind::Ssh, &initial, |f| {
        f["password"] = json!("");
        f["identityFile"] = json!("");
    });
    assert!(out.get("password").is_none() && out.get("identity_file").is_none());
}

#[test]
fn ssh_jump_host_key_is_loaded_saved_and_cleared() {
    let initial = json!({"label": "x", "host": "h", "user": "u", "proxy_jump": "bastion",
                         "jump_identity_file": "~/.ssh/jump"});
    let entry = parse_entry("test", &initial.to_string());
    let form = prepare(&cfg("{}"), Kind::Ssh, entry.as_ref(), "", &[])["form"].clone();
    assert_eq!(form["jumpIdentityFile"], "~/.ssh/jump");
    assert_eq!(
        save(Kind::Ssh, &initial, |_| {})["jump_identity_file"],
        "~/.ssh/jump"
    );
    let out = save(Kind::Ssh, &initial, |f| {
        f["jumpIdentityFile"] = json!("  /k/other ")
    });
    assert_eq!(out["jump_identity_file"], "/k/other");
    let out = save(Kind::Ssh, &initial, |f| f["jumpIdentityFile"] = json!(""));
    assert!(out.get("jump_identity_file").is_none());
    // An entry saved before the field existed is stored without it.
    let old = json!({"label": "x", "host": "h", "user": "u", "proxy_jump": "bastion"});
    assert!(
        save(Kind::Ssh, &old, |_| {})
            .get("jump_identity_file")
            .is_none()
    );
}

#[test]
fn key_file_hints_for_the_jump_host_and_tunnel_keys() {
    let dir = std::env::temp_dir().join(format!("termigator-key-hint-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("mkdir");
    let public = dir.join("id.pub");
    std::fs::write(&public, "ssh-ed25519 AAAAC3Nza me@host\n").expect("write");
    let locked = dir.join("locked");
    std::fs::write(
        &locked,
        "-----BEGIN ENCRYPTED PRIVATE KEY-----\nMIIE\n-----END ENCRYPTED PRIVATE KEY-----\n",
    )
    .expect("write");
    let path = |p: &std::path::Path| p.to_string_lossy().into_owned();
    assert_eq!(key_file_problem("  "), "");
    assert!(key_file_problem(&path(&public)).contains("public key"));
    let missing = path(&dir.join("missing"));
    assert!(key_file_problem(&missing).contains("not found"));
    assert!(key_locked(&path(&locked)));
    assert!(!key_locked(&path(&public)));
    assert!(!key_locked(""));
    assert_eq!(key_path(" "), "");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn ssh_untouched_fields_keep_values() {
    let initial = json!({"label": "x", "host": "h", "user": "u", "password": "pw", "identity_file": "~/.ssh/k"});
    let out = save(Kind::Ssh, &initial, |_| {});
    assert_eq!(out["password"], "pw");
    assert_eq!(out["identity_file"], "~/.ssh/k");
}

#[test]
fn rdp_clearing_password_removes_it() {
    let initial =
        json!({"label": "x", "host": "h", "user": "u", "password": "pw", "protocol": "rdp"});
    let out = save(Kind::Rdp, &initial, |f| f["password"] = json!(""));
    assert!(out.get("password").is_none());
}

#[test]
fn vnc_stray_password_dropped_on_save() {
    let initial = json!({"label": "x", "host": "h", "protocol": "vnc", "password": "pw"});
    assert!(save(Kind::Vnc, &initial, |_| {}).get("password").is_none());
}

#[test]
fn sealed_password_kept_unless_forget_ticked() {
    for kind in [Kind::Ssh, Kind::Rdp] {
        let initial = json!({"label": "x", "host": "h", "user": "u", "password": SEALED});
        let entry = parse_entry("test", &initial.to_string());
        let shown = prepare(&cfg("{}"), kind, entry.as_ref(), "", &[]);
        assert_eq!(shown["form"]["password"], ""); // the token is never put into the field
        assert_eq!(shown["form"]["passwordSealed"], true);
        assert_eq!(save(kind, &initial, |_| {})["password"], SEALED);
        let forgotten = save(kind, &initial, |f| f["forgetPassword"] = json!(true));
        assert!(forgotten.get("password").is_none());
    }
    let plain = json!({"label": "x", "host": "h", "user": "u", "password": "pw"});
    let entry = parse_entry("test", &plain.to_string());
    let shown = prepare(&cfg("{}"), Kind::Ssh, entry.as_ref(), "", &[]);
    assert_eq!(shown["form"]["passwordSealed"], false);
}

#[test]
fn group_path_is_normalized_on_save() {
    for kind in [Kind::Ssh, Kind::Rdp, Kind::Vnc] {
        let out = save(
            kind,
            &json!({"label": "x", "host": "h", "user": "u"}),
            |f| {
                f["group"] = json!("  Lab /  Internal// ");
            },
        );
        assert_eq!(out["group"], "Lab/Internal");
    }
}

#[test]
fn ssh_session_tab_round_trip() {
    let initial = json!({
        "label": "x", "host": "h", "user": "u", "x11_forwarding": true, "x11_trusted": true,
        "compression": true, "request_tty": true, "ssh_keepalive": false, "remote_locale": "en_US.UTF-8",
        "font_family": "Hack", "font_size": 13, "color_scheme": "DarkPastels",
        "startup_commands": "sudo -i\ncd /srv",
    });
    assert_has_all(&save(Kind::Ssh, &initial, |_| {}), &initial);
    let plain = save(
        Kind::Ssh,
        &json!({"label": "x", "host": "h", "user": "u"}),
        |_| {},
    );
    for k in [
        "x11_forwarding",
        "x11_trusted",
        "compression",
        "request_tty",
        "ssh_keepalive",
        "remote_locale",
        "font_family",
        "font_size",
        "color_scheme",
        "startup_commands",
    ] {
        assert!(plain.get(k).is_none(), "{k}");
    }
}

#[test]
fn rdp_redirection_round_trip() {
    let initial = json!({
        "label": "w", "protocol": "rdp", "host": "h", "clipboard": false, "sound": true, "microphone": true,
        "home_drive": true, "multimon": true, "dynamic_resolution": true, "shared_folders": ["/srv", "~/x"],
        "gateway": "gw:443", "gateway_user": "g", "gateway_domain": "D",
    });
    assert_has_all(&save(Kind::Rdp, &initial, |_| {}), &initial);
    let plain = save(
        Kind::Rdp,
        &json!({"label": "w", "protocol": "rdp", "host": "h"}),
        |_| {},
    );
    for k in ["clipboard", "sound", "shared_folders", "gateway"] {
        assert!(plain.get(k).is_none(), "{k}");
    }
}

#[test]
fn ssh_dialog_files_only_round_trip() {
    let initial = json!({"label": "nas", "host": "nas.lan", "files_only": true, "port": 2222, "user": "backup"});
    assert_eq!(save(Kind::Ssh, &initial, |_| {})["files_only"], true);
    let off = save(Kind::Ssh, &initial, |f| f["filesOnly"] = json!(false));
    assert!(off.get("files_only").is_none());
}

#[test]
fn dialogs_store_the_gateway() {
    for (kind, proto) in [(Kind::Rdp, "rdp"), (Kind::Vnc, "vnc")] {
        let mut initial =
            json!({"label": "x", "protocol": proto, "host": "h", "ssh_gateway": "bastion"});
        if proto == "rdp" {
            initial["gateway"] = json!("rdgw.example");
        }
        let out = save(kind, &initial, |_| {});
        assert_eq!(out["ssh_gateway"], "bastion");
        if proto == "rdp" {
            assert_eq!(out["gateway"], "rdgw.example"); // the RD Gateway field is untouched
        }
        let direct = save(kind, &initial, |f| f["sshGateway"] = json!(""));
        assert!(direct.get("ssh_gateway").is_none());
    }
}

#[test]
fn telnet_and_serial_round_trip() {
    let t = json!({"label": "sw", "protocol": "telnet", "host": "10.0.0.2", "port": 2323,
                   "user": "admin", "group": "net"});
    assert_has_all(&save(Kind::Telnet, &t, |_| {}), &t);
    let s = json!({"label": "con", "protocol": "serial", "device": "/dev/ttyUSB1", "baud": 9600,
                   "data_bits": 7, "parity": "even", "stop_bits": 2, "flow": "software"});
    assert_has_all(&save(Kind::Serial, &s, |_| {}), &s);
    let plain = save(
        Kind::Serial,
        &json!({"label": "c", "protocol": "serial", "device": "/dev/ttyUSB1"}),
        |_| {},
    );
    for k in ["data_bits", "parity", "stop_bits", "flow"] {
        assert!(plain.get(k).is_none(), "{k}"); // 8N1 defaults store nothing
    }
}

#[test]
fn unknown_keys_survive_and_edited_ones_move_last() {
    let initial = json!({"zz": {"a": 1}, "label": "x", "keep": [1, 2], "host": "h", "user": "u",
                         "tags": ["a"], "mine": true});
    let out = save(Kind::Ssh, &initial, |f| f["tags"] = json!("a, b"));
    assert_eq!(out["zz"], json!({"a": 1}));
    assert_eq!(out["keep"], json!([1, 2]));
    assert_eq!(out["mine"], true);
    assert_eq!(out["tags"], json!(["a", "b"]));
    // The managed keys are removed and set again in dialog order.
    assert_eq!(
        keys(&out),
        [
            "zz", "keep", "mine", "label", "host", "user", "port", "terminal", "tags"
        ]
    );
    // Telnet entries keep their tags untouched.
    let t = save(
        Kind::Telnet,
        &json!({"host": "h", "tags": ["x"], "extra": 1}),
        |_| {},
    );
    assert_eq!(t["tags"], json!(["x"]));
    assert_eq!(t["extra"], 1);
}

#[test]
fn credential_choice_locks_login_fields() {
    let c = cfg(
        "credentials:\n- {name: ops, user: root, password: x}\n- {name: win, user: adm, domain: CORP}\n",
    );
    let initial = json!({"label": "l", "host": "h", "user": "me", "password": "own"});
    let shown = prepare(
        &c,
        Kind::Ssh,
        parse_entry("test", &initial.to_string()).as_ref(),
        "",
        &[],
    );
    assert_eq!(shown["credentials"][0]["label"], "ops  (root)");
    // The dialog puts the credential's user in the field and clears the password.
    let out = save_in(&c, Kind::Ssh, &initial, |f| {
        f["credential"] = json!("ops");
        f["user"] = json!("root");
        f["password"] = json!("");
    });
    assert_eq!(out["credential"], "ops");
    assert_eq!(out["user"], "root");
    assert!(out.get("password").is_none());
    let back = save_in(&c, Kind::Ssh, &initial, |_| {});
    assert!(back.get("credential").is_none());
    assert_eq!(
        (back["user"].as_str(), back["password"].as_str()),
        (Some("me"), Some("own"))
    );
    let rdp = save_in(
        &c,
        Kind::Rdp,
        &json!({"label": "l", "host": "h", "credential": "win"}),
        |f| {
            f["user"] = json!("adm");
            f["domain"] = json!("CORP");
        },
    );
    assert_eq!(rdp["credential"], "win");
    assert_eq!(
        (rdp["user"].as_str(), rdp["domain"].as_str()),
        (Some("adm"), Some("CORP"))
    );
}

#[test]
fn choices_come_from_the_config() {
    let c = cfg(
        "connections:\n- {label: bastion, host: b, user: u, group: prod/web}\n- {label: bastion, host: b2}\n\
         - {label: win, protocol: rdp, host: w}\ngroups: [lab]\nfavorites:\n- {label: f, command: ls, group: ops}\n\
         credentials:\n- {name: n}\n- not a dict\n- {user: no-name}\n",
    );
    let shown = prepare(&c, Kind::Rdp, None, "ops", &[]);
    assert_eq!(shown["gateways"], json!(["bastion"]));
    assert_eq!(shown["groups"], json!(["lab", "ops", "prod", "prod/web"]));
    assert_eq!(shown["credentials"][0]["label"], "n  (no user)");
    assert_eq!(shown["credentials"].as_array().map(Vec::len), Some(1));
    assert_eq!(shown["title"], "Add RDP connection");
    assert_eq!(shown["form"]["group"], "ops"); // the folder the editor was opened for
    assert_eq!(shown["form"]["port"], 3389);
    let edit = prepare(
        &c,
        Kind::Serial,
        parse_entry("test", r#"{"label":"c"}"#).as_ref(),
        "",
        &["/dev/ttyUSB0".into()],
    );
    assert_eq!(edit["title"], "Edit serial connection");
    assert_eq!(edit["form"]["device"], "/dev/ttyUSB0"); // the first detected one
    assert_eq!(edit["form"]["baud"], "115200");
}

// Windows has no external terminal (terminal_mode_for_entry), which this test relies on.
#[cfg(not(windows))]
#[test]
fn terminal_follows_the_entry_then_the_config() {
    let c = cfg("window: {default_terminal: external}\n");
    let term = |e: Option<&str>| {
        let entry = e.and_then(|e| parse_entry("test", e));
        prepare(&c, Kind::Vnc, entry.as_ref(), "", &[])["form"]["terminal"].clone()
    };
    assert_eq!(term(None), "external");
    assert_eq!(
        term(Some(r#"{"host":"h","terminal":"embedded"}"#)),
        "embedded"
    );
    assert_eq!(term(Some(r#"{"host":"h","terminal":" "}"#)), "external");
}

#[test]
fn favorite_needs_label_and_command_and_forgets_the_rest() {
    let fav = json!({"label": "sh", "command": "bash", "group": "a", "extra": 1});
    let out = save(Kind::Favorite, &fav, |f| f["group"] = json!(" a / b "));
    assert_eq!(
        out,
        json!({"label": "sh", "command": "bash", "terminal": "embedded", "group": "a/b"})
    );
    let entry = parse_entry("test", &fav.to_string());
    let mut form = prepare(&cfg("{}"), Kind::Favorite, entry.as_ref(), "", &[])["form"].clone();
    form["command"] = json!("  ");
    assert!(build(Kind::Favorite, entry.as_ref(), &form).is_none());
}

#[test]
fn odd_stored_values_show_like_golden() {
    let c = cfg("{}");
    let shown =
        |kind, e: &str| prepare(&c, kind, parse_entry("test", e).as_ref(), "", &[])["form"].clone();
    let f = shown(
        Kind::Ssh,
        r#"{"host":"h","port":"abc","extra_ssh_args":["-o","A=b c"],"tags":"x","dynamic_forward":"off","font_size":"x","strict_host_key_checking":"nope"}"#,
    );
    assert_eq!(f["port"], 22);
    assert_eq!(f["extra"], "-o 'A=b c'");
    assert_eq!(f["tags"], "x");
    assert_eq!(f["dynamic"], "off"); // only none/false/no are hidden
    assert_eq!(f["fontSize"], 0);
    assert_eq!(f["strictHost"], "");
    assert_eq!(
        shown(Kind::Vnc, r#"{"host":"h","port":99999}"#)["port"],
        65535
    );
    assert_eq!(shown(Kind::Telnet, r#"{"host":"h","port":0}"#)["port"], 23);
    // SOCKS port digits become a number, unbalanced quotes stay text.
    let out = save(Kind::Ssh, &json!({"host": "h"}), |f| {
        f["dynamic"] = json!("1080");
        f["extra"] = json!("-o 'x");
    });
    assert_eq!(out["dynamic_forward"], 1080);
    assert_eq!(out["extra_ssh_args"], "-o 'x");
    assert!(out["port"].is_null());
}

#[test]
fn serial_hint_only_for_device_paths() {
    assert_eq!(serial_problem(""), "");
    assert_eq!(serial_problem("COM3"), "");
    assert!(!serial_problem("/dev/definitely-not-here").is_empty());
}

#[test]
fn probe_timeout_reads_launch_options() {
    assert_eq!(probe_timeout(&cfg("{}")), DEFAULT_TIMEOUT);
    assert_eq!(
        probe_timeout(&cfg("launch: {probe_timeout: 9}\n")),
        Duration::from_secs(9)
    );
    assert_eq!(
        probe_timeout(&cfg("launch: {probe_timeout: 0}\n")),
        DEFAULT_TIMEOUT
    );
}

#[test]
fn the_key_field_warns_about_public_and_putty_keys_but_not_about_names() {
    let dir = std::env::temp_dir().join(format!("termigator-keyproblem-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("mkdir");
    let put = |name: &str, body: &str| {
        let p = dir.join(name);
        std::fs::write(&p, body).expect("write");
        p.to_string_lossy().into_owned()
    };
    // A private key under a name ending in .pub is fine; only the content counts.
    let private = put(
        "work.pub",
        "-----BEGIN OPENSSH PRIVATE KEY-----\nAAAA\n-----END OPENSSH PRIVATE KEY-----\n",
    );
    // Readable by others it gets a private copy for ssh, and the field says so.
    #[cfg(unix)]
    assert!(key_problem(&private).contains("private copy"));
    termigator_core::platform::set_private_file(std::path::Path::new(&private)).expect("0600");
    assert_eq!(key_problem(&private), "");
    assert!(key_problem(&put("id_ed25519", "ssh-ed25519 AAAA me\n")).contains("public key"));
    assert!(
        key_problem(&put("k.ppk", "PuTTY-User-Key-File-3: ssh-ed25519\n")).contains("puttygen")
    );
    // Empty, unset and missing paths say nothing (the connection reports a missing file itself).
    assert_eq!(key_problem(""), "");
    assert_eq!(key_problem(&dir.join("nope").to_string_lossy()), "");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn forms_say_whether_a_master_password_is_set() {
    let (vault, _) =
        termigator_core::vault::create_vault_with_salt("m", &[1; 16], &[2; 12], (2, 1, 1))
            .expect("vault");
    let mut with_vault = cfg("{}");
    if let Value::Map(m) = &mut with_vault {
        m.push((Value::Str("vault".into()), vault));
    }
    for kind in [Kind::Ssh, Kind::Rdp] {
        assert_eq!(
            prepare(&cfg("{}"), kind, None, "", &[])["form"]["vaultOn"],
            false
        );
        assert_eq!(
            prepare(&with_vault, kind, None, "", &[])["form"]["vaultOn"],
            true
        );
    }
}
