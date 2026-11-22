//! The vault, named credentials and SSH keys in the real binary, offscreen
//! (`qml/vault/VaultSmoke.qml`), twice. The first run drives the master password dialog, the unlock
//! hook, a connection with a sealed password, the Credentials dialog with a locked vault, and the
//! SSH keys window; this side checks that `session.yaml` holds sealed passwords and no plain text.
//! The second run restores the tabs with the vault locked (Cancel, then the master password) and
//! removes the master password; this side checks that the passwords are plain again.

mod common;

use std::fs;
use std::path::PathBuf;
use std::process::Command;

// cxx-qt's generated C++ is linked into this test too; it needs the library's Rust half.
use termigator as _;

const CONFIG: &str = "\
window:
  session_restore: true
  confirm_close: false
  confirm_close_tab: false
connections:
- {label: web, host: 127.0.0.1, port: 1, user: u, password: pw1}
- {label: db, host: 127.0.0.1, port: 1, user: root, credential: ops}
credentials:
- {name: ops, user: root, password: cpw}
";
const CHECK: &str = "--smoke-check=vault/VaultSmoke.qml";

fn no_qml_errors(log: &str) {
    // The tab bar of the style warns once about a null height while the first tab is created
    // (not from this area).
    let ours: Vec<&str> = log
        .lines()
        .filter(|l| !l.contains("org/kde/desktop/TabBar.qml"))
        .collect();
    for bad in ["TypeError", "ReferenceError", "qml/vault/"] {
        assert!(
            !ours.iter().any(|l| l.contains(bad)),
            "{bad} in stderr:\n{log}"
        );
    }
}

#[test]
fn vault_credentials_and_ssh_keys_offscreen() {
    let ssh_dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("termigator-u7-ssh");
    let _ = fs::remove_dir_all(&ssh_dir);
    fs::create_dir_all(&ssh_dir).expect("ssh dir");
    let key = ssh_dir.join("id_ed25519");
    let made = Command::new("ssh-keygen")
        .args(["-q", "-t", "ed25519", "-N", "", "-C", "u7@test", "-f"])
        .arg(&key)
        .status()
        .expect("ssh-keygen is needed for this test");
    assert!(made.success(), "ssh-keygen failed");

    let dir_arg = format!("--u7-ssh-dir={}", ssh_dir.display());
    let mut run = common::launch("u7-vault", CONFIG, &[CHECK, &dir_arg]);
    let status = run.wait(120, |_| {});
    let log = run.log();
    assert!(status.success(), "exit {status}, stderr:\n{log}");
    assert!(
        log.contains("termigator vault smoke: first run done"),
        "stderr:\n{log}"
    );
    no_qml_errors(&log);
    assert!(
        fs::metadata(&run.shot).map(|m| m.len()).unwrap_or(0) > 0,
        "no grab of the SSH keys window"
    );

    // The connections are in session.yaml, the credentials and the vault in config.yaml.
    let session = fs::read_to_string(run.config.join("session.yaml")).expect("session.yaml");
    let config = fs::read_to_string(run.config.join("config.yaml")).expect("config.yaml");
    assert!(config.contains("vault:"), "no vault section:\n{config}");
    // web, prod and late are sealed; late2 (Cancel) never was. Passwords are v2 tokens
    // (bound to their entry), only the vault's own check value stays v1 (the old count of four
    // `enc:v1:` strings was these three plus that check).
    let tokens = session.matches("enc:v2:").count() + config.matches("enc:v2:").count();
    assert_eq!(tokens, 3, "{session}\n{config}");
    let legacy = session.matches("enc:v1:").count() + config.matches("enc:v1:").count();
    assert_eq!(legacy, 1, "only the check token is v1:\n{config}");
    for plain in ["pw1", "cpw", "pw3", "pw4"] {
        assert!(
            !session.contains(plain) && !config.contains(plain),
            "{plain} written in the clear:\n{session}\n{config}"
        );
    }
    assert!(
        session.contains("credential: prod"),
        "rename not followed:\n{session}"
    );
    assert!(config.contains("name: late"), "{config}");
    assert_eq!(
        config.matches("title: web").count(),
        2,
        "snapshot:\n{config}"
    );

    // The next start: the vault is locked, the restore asks.
    let rows: Vec<&str> = session.lines().filter(|l| !l.starts_with('#')).collect();
    let again = format!("{config}\nconnections:\n{}\n", rows.join("\n"));
    let mut second = common::launch("u7-vault-restore", &again, &[CHECK, "--u7-restore"]);
    let status = second.wait(120, |_| {});
    let log = second.log();
    assert!(status.success(), "exit {status}, stderr:\n{log}");
    assert!(
        log.contains("termigator vault smoke: restore run done"),
        "stderr:\n{log}"
    );
    no_qml_errors(&log);

    // Remove put the passwords back in plain text.
    let session = fs::read_to_string(second.config.join("session.yaml")).expect("session.yaml");
    let config = fs::read_to_string(second.config.join("config.yaml")).expect("config.yaml");
    assert!(
        !session.contains("enc:v1:") && !config.contains("enc:v1:") && !config.contains("vault:"),
        "{session}\n{config}"
    );
    assert!(session.contains("password: pw1"), "{session}");
    assert!(
        config.contains("password: cpw") && config.contains("password: pw3"),
        "{config}"
    );
}
