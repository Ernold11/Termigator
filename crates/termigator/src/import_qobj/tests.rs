use super::*;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use termigator_core::config::doc;

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("termigator-u10-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("scratch dir");
    dir
}

fn cfg(yaml: &str) -> Value {
    doc::load(yaml).expect("yaml")
}

fn hosts(c: &Value) -> Vec<String> {
    seq(c, "connections")
        .iter()
        .map(|e| {
            e.get("host")
                .map(termigator_core::config::model::value_str)
                .unwrap_or_default()
        })
        .collect()
}

#[test]
fn ssh_config_merges_and_reports_duplicates() {
    let dir = scratch("sshcfg");
    let file = dir.join("config");
    fs::write(
        &file,
        "Host web\n  HostName 10.0.0.1\n  User bob\nHost db\n  HostName 10.0.0.2\n  User bob\n",
    )
    .expect("write");
    let mut c = cfg("connections:\n- {label: x, host: 10.0.0.1, user: bob}\n");
    let out = ssh_config(&mut c, &file, &dir, "me");
    assert!(out.changed && !out.warn, "{}", out.text);
    assert!(
        out.text.starts_with("Imported 1 host(s) from "),
        "{}",
        out.text
    );
    assert!(out.text.contains("(1 duplicate(s) skipped)."));
    assert_eq!(hosts(&c), ["10.0.0.1", "10.0.0.2"]);
    // A second run adds nothing and leaves the config alone.
    let again = ssh_config(&mut c, &file, &dir, "me");
    assert!(!again.changed);
    assert!(again.text.starts_with("Nothing to import — all 2 host(s)"));
    let missing = ssh_config(&mut c, &dir.join("nope"), &dir, "me");
    assert!(missing.warn && missing.text.starts_with("Could not read:"));
}

#[test]
fn ssh_config_drops_commands_only_from_foreign_files() {
    let dir = scratch("sshcmd");
    let text = "Host web\n  HostName 10.0.0.1\n  User bob\n  ProxyCommand nc %h %p\n  Ciphers aes256-gcm@openssh.com\n";
    let extra = |c: &Value| format!("{:?}", seq(c, "connections")[0].get("extra_ssh_args"));
    // Someone else's file: the command is not imported and the user is warned.
    let foreign = dir.join("shared_config");
    fs::write(&foreign, text).expect("write");
    let mut c = cfg("connections: []\n");
    let out = ssh_config(&mut c, &foreign, &dir, "me");
    assert!(out.changed && out.warn, "{}", out.text);
    assert!(out.text.contains("web: ProxyCommand"), "{}", out.text);
    assert!(!extra(&c).contains("ProxyCommand") && extra(&c).contains("Ciphers"));
    // The user's own ~/.ssh/config: ssh runs it anyway, so it stays.
    fs::create_dir_all(dir.join(".ssh")).expect("mkdir");
    let own = dir.join(".ssh").join("config");
    fs::write(&own, text).expect("write");
    let mut c = cfg("connections: []\n");
    let out = ssh_config(&mut c, &own, &dir, "me");
    assert!(out.changed && !out.warn, "{}", out.text);
    assert!(extra(&c).contains("ProxyCommand"));
}

#[test]
fn putty_sessions_dir_and_empty_source() {
    let dir = scratch("putty");
    fs::write(
        dir.join("Web%20Box"),
        "HostName=10.0.0.9\nProtocol=ssh\nPortNumber=22\n",
    )
    .expect("write");
    let mut c = cfg("window: {}\n");
    let out = putty(&mut c, &dir, "me");
    assert!(out.changed, "{}", out.text);
    assert!(out.text.contains("got “me”"));
    assert_eq!(hosts(&c), ["10.0.0.9"]);
    // The terminal default is filled in by the config merge.
    assert!(seq(&c, "connections")[0].get("terminal").is_some());
    let none = putty(&mut c, &dir.join("missing.reg"), "me");
    assert!(!none.changed && none.text.starts_with("No SSH sessions found."));
}

#[test]
fn remmina_profiles_and_skipped_protocols() {
    let dir = scratch("remmina");
    fs::write(
        dir.join("a.remmina"),
        "[remmina]\nname=box\nprotocol=SSH\nserver=10.0.0.3\nusername=u\n",
    )
    .expect("write");
    fs::write(
        dir.join("b.remmina"),
        "[remmina]\nname=x\nprotocol=SPICE\nserver=10.0.0.4\n",
    )
    .expect("write");
    let mut c = cfg("window: {}\n");
    let out = remmina(&mut c, std::slice::from_ref(&dir), &dir);
    assert!(out.changed, "{}", out.text);
    assert!(
        out.text.contains("Not supported: 1× SPICE."),
        "{}",
        out.text
    );
    assert_eq!(hosts(&c), ["10.0.0.3"]);
    let empty = scratch("remmina-empty");
    let none = remmina(&mut c, &[empty], &dir);
    assert!(!none.changed && none.text.starts_with("No SSH, SFTP, RDP or VNC profiles"));
}

#[test]
fn moba_unreadable_and_no_bookmarks() {
    let dir = scratch("moba");
    let mut c = cfg("window: {}\n");
    let gone = moba(&mut c, &dir.join("nope.ini"), "me");
    assert!(gone.warn && gone.text.starts_with("Could not read file:"));
    let file = dir.join("s.ini");
    fs::write(&file, "[Bookmarks]\nSubRep=\nImgNum=42\n").expect("write");
    let empty = moba(&mut c, &file, "me");
    assert!(!empty.changed && empty.text.starts_with("No SSH, RDP or VNC bookmarks"));
}

#[test]
fn moba_imports_an_ssh_bookmark() {
    let dir = scratch("moba-ssh");
    let file = dir.join("s.mxtsessions");
    fs::write(
            &file,
            "[Bookmarks]\nSubRep=\nImgNum=42\nweb=#109#0%10.0.0.5%22%admin%%-1%-1%%%%%0%0%0%%%-1%0%0%0%%1080%%0%0%1#MobaFont%10%0%0%-1%15%236,236,236%30,30,30%180,180,192%0%-1%0%%xterm%-1%-1%_Std_Colors_0_%80%24%0%1%-1%<none>%%0%0%-1#0# #-1\n",
        )
        .expect("write");
    let mut c = cfg("window: {}\n");
    let out = moba(&mut c, &file, "me");
    assert!(out.changed, "{}", out.text);
    assert_eq!(hosts(&c), ["10.0.0.5"]);
    assert!(
        out.text
            .starts_with("Imported 1 new connection(s) from MobaXterm.")
    );
}

#[test]
#[cfg(unix)] // Unix modes (0600)
fn ssh_export_writes_private_file_with_include_hint() {
    let dir = scratch("export");
    let c = cfg(
        "connections:\n- {label: web, host: h1, user: u}\n- {label: rd, protocol: rdp, host: h2}\n",
    );
    let target = dir.join("hosts.conf");
    fs::write(&target, "old").expect("write");
    fs::set_permissions(&target, fs::Permissions::from_mode(0o644)).expect("chmod");
    let now = Stamp {
        year: 2024,
        month: 5,
        day: 6,
        hour: 7,
        minute: 8,
    };
    let out = ssh_export(&c, &target, &dir, now);
    assert!(!out.warn, "{}", out.text);
    assert!(
        out.text
            .contains("(1 RDP / VNC / Telnet / serial entry skipped).")
    );
    assert!(out.text.contains("    Include ~/hosts.conf\n"));
    let text = fs::read_to_string(&target).expect("read");
    assert!(text.contains("Host web") && text.contains("2024-05-06 07:08"));
    assert_eq!(
        fs::metadata(&target).expect("meta").permissions().mode() & 0o777,
        0o600
    );
    let none = ssh_export(&cfg("connections: []\n"), &target, &dir, now);
    assert_eq!(none.text, "There are no SSH entries to export.");
    assert_eq!(file_stamp(now), "20240506-0708");
}

#[test]
#[cfg(unix)] // Unix modes (0600)
fn backup_has_both_files_readme_and_mode_0600() {
    let dir = scratch("backup");
    let paths = Paths {
        repo_root: dir.clone(),
        config_dir: dir.clone(),
        config_path: dir.join("config.yaml"),
        session_path: dir.join("session.yaml"),
        config_example_path: dir.join("c.example"),
        session_example_path: dir.join("s.example"),
    };
    fs::write(&paths.config_path, "window: {}\n").expect("write");
    let target = dir.join("termigator-settings-x.zip");
    let out = backup_zip(&paths, &target);
    // session.yaml is absent: skipped, the README is still there.
    assert_eq!(
        out.text,
        format!(
            "Backup saved: config.yaml, README.txt → {}",
            target.display()
        )
    );
    assert_eq!(
        fs::metadata(&target).expect("meta").permissions().mode() & 0o777,
        0o600
    );
    // Deflated entries keep their names in clear in the zip's directory.
    let raw = fs::read(&target).expect("read zip");
    assert!(raw.starts_with(b"PK"));
    assert!(raw.windows(10).any(|w| w == b"README.txt"));
    let bad = backup_zip(&paths, &dir.join("no/such/dir/x.zip"));
    assert!(bad.warn && bad.text.starts_with("Could not write the backup:"));
}

#[test]
fn plaintext_passwords_count_connections_and_credentials() {
    let c = cfg(
        "connections:\n- {label: a, host: h, password: pw}\n- {label: b, host: h}\n\
             credentials:\n- {name: n, password: x}\n",
    );
    assert_eq!(plain_passwords(&c), 2);
}
