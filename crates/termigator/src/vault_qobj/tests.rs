use std::sync::{Mutex, MutexGuard, PoisonError};

use termigator_core::config::doc;
use termigator_core::proto::ssh_keys::KEY_TYPES;

use super::*;

/// The session key is process-wide: the tests that touch it run one at a time.
static KEY_LOCK: Mutex<()> = Mutex::new(());

fn key_lock() -> MutexGuard<'static, ()> {
    KEY_LOCK.lock().unwrap_or_else(PoisonError::into_inner)
}

fn cfg(yaml: &str) -> Value {
    doc::load(yaml).expect("yaml")
}

fn password(c: &Value, i: usize) -> String {
    seq(c, "connections")[i]
        .get("password")
        .map(value_str)
        .unwrap_or_default()
}

/// `test_dialog_set_change_remove`: Set seals, Change re-keys, Remove returns the plain text.
#[test]
fn set_change_remove_like_golden() {
    let _guard = key_lock();
    let mut c = cfg(
        "connections:\n- {host: a, user: u, password: pa}\ncredentials:\n- {name: ops, user: r, password: pc}\n",
    );
    let (mut list, n) = combined(&c);
    let done = master_set(&mut list, "first").expect("set");
    assert_eq!(
        done.message,
        "Master password set — 2 saved password(s) encrypted."
    );
    apply_master(&mut c, list, n, done);
    assert!(vault::is_sealed(
        c.get("connections")
            .and_then(|s| match s {
                Value::Seq(v) => v[0].get("password"),
                _ => None,
            })
            .expect("password")
    ));
    let key = vault::session_key().expect("key remembered");
    let first = c
        .get("connections")
        .and_then(|s| match s {
            Value::Seq(v) => v.first().cloned(),
            _ => None,
        })
        .expect("connection");
    assert_eq!(
        vault::unseal_entry(&key, &password(&c, 0), &first).expect("unseal"),
        "pa"
    );

    let (mut list, n) = combined(&c);
    let done = master_change(&mut list, &key, "second").expect("change");
    assert_eq!(
        done.message,
        "Master password changed — 2 password(s) re-encrypted."
    );
    apply_master(&mut c, list, n, done);
    let params = c.get("vault").expect("vault").clone();
    assert!(vault::unlock(&params, "first").is_err());
    let key2 = vault::unlock(&params, "second").expect("new master");
    let first = c
        .get("connections")
        .and_then(|s| match s {
            Value::Seq(v) => v.first().cloned(),
            _ => None,
        })
        .expect("connection");
    assert_eq!(
        vault::unseal_entry(&key2, &password(&c, 0), &first).expect("unseal"),
        "pa"
    );

    let (mut list, n) = combined(&c);
    let done = master_remove(&mut list, &key2).expect("remove");
    apply_master(&mut c, list, n, done);
    assert!(c.get("vault").is_none());
    assert_eq!(password(&c, 0), "pa");
    assert!(vault::session_key().is_none());
}

/// A password that cannot be decrypted: nothing changes.
#[test]
fn change_and_remove_are_all_or_nothing() {
    let (_, key) = vault::create_vault("m").expect("vault");
    let (_, other) = vault::create_vault("x").expect("vault");
    let mut list = vec![
        cfg(&format!(
            "{{host: a, password: '{}'}}",
            vault::seal(&key, "pa").expect("seal")
        )),
        cfg(&format!(
            "{{host: b, password: '{}'}}",
            vault::seal(&other, "pb").expect("seal")
        )),
    ];
    let before = list.clone();
    let e = master_remove(&mut list, &key).err().expect("fails");
    assert!(
        e.starts_with("Nothing changed — a saved password could not be decrypted:\n"),
        "{e}"
    );
    assert_eq!(list, before);
    assert!(master_change(&mut list, &key, "n").is_err());
    assert_eq!(list, before);
}

/// `test_seal_pending_seals_new_password` and the locked case that asks.
#[test]
fn seal_pending_needs_the_key() {
    let _guard = key_lock();
    let (params, key) = vault::create_vault("m").expect("vault");
    let sealed = vault::seal(&key, "pa").expect("seal");
    let mut c = cfg(&format!(
        "connections:\n- {{host: a, password: '{sealed}'}}\n- {{host: c, password: new}}\n"
    ));
    set_top(&mut c, "vault", params);
    vault::remember_key(None);
    assert_eq!(seal_pending(&mut c), Seal::NeedsUnlock);
    assert_eq!(password(&c, 1), "new");
    vault::remember_key(Some(key.clone()));
    assert_eq!(seal_pending(&mut c), Seal::Sealed);
    let second = c
        .get("connections")
        .and_then(|s| match s {
            Value::Seq(v) => v.get(1).cloned(),
            _ => None,
        })
        .expect("connection");
    assert_eq!(
        vault::unseal_entry(&key, &password(&c, 1), &second).expect("unseal"),
        "new"
    );
    assert_eq!(seal_pending(&mut c), Seal::Nothing);
    vault::remember_key(None);
    let mut plain = cfg("connections:\n- {host: a, password: p}\n");
    assert_eq!(seal_pending(&mut plain), Seal::Nothing);
}

#[test]
fn status_texts_like_golden() {
    let off = cfg("connections:\n- {host: a, password: p}\n");
    let s = status(&off, false);
    assert_eq!(
        s.text,
        "⚠ 1 saved password(s) stored in plain text in session.yaml."
    );
    assert_eq!(
        (s.can_set, s.can_change, s.can_remove),
        (true, false, false)
    );
    assert_eq!(
        status(&cfg("connections: []\n"), false).text,
        "No master password — saved passwords are stored in plain text in session.yaml."
    );
    let (params, key) = vault::create_vault("m").expect("vault");
    let mut on = cfg(&format!(
        "connections:\n- {{host: a, password: '{}'}}\n- {{host: b, password: q}}\n",
        vault::seal(&key, "pa").expect("seal")
    ));
    set_top(&mut on, "vault", params);
    let s = status(&on, false);
    assert_eq!(
        s.text,
        "✔ Master password is set — 1 saved password(s) encrypted.\n⚠ 1 new password(s) not \
             encrypted yet (sealed on the next save).\nLocked — the master password is asked when a \
             saved password is needed."
    );
    assert_eq!((s.can_set, s.can_change, s.can_remove), (false, true, true));
    assert!(!status(&on, true).text.contains("Locked"));
}

/// What ssh gets for `pw`: `SSHPASS` for sshpass, on Windows the askpass channel (one token per
/// password, so asking again gives the same environment).
fn pw_env(pw: &str) -> PasswordEnv {
    if cfg!(windows) {
        let conn = cfg(&format!("{{password: '{pw}'}}"));
        PasswordEnv::Env(ssh_password_env(&conn).expect("askpass env"))
    } else {
        PasswordEnv::Env(vec![(
            "SSHPASS".to_owned(),
            Secret::new(pw.as_bytes().to_vec()),
        )])
    }
}

/// `test_plain_entry_never_prompts`, and the locked / unlocked answers of the install flow.
#[test]
fn password_env_states() {
    let _guard = key_lock();
    let plain = cfg("connections: []\n");
    assert_eq!(
        password_env(&plain, &cfg("{host: a}"), None),
        PasswordEnv::None
    );
    assert_eq!(
        password_env(&plain, &cfg("{host: a, password: pw}"), None),
        pw_env("pw")
    );
    let (params, key) = vault::create_vault("m").expect("vault");
    let token = vault::seal(&key, "pa").expect("seal");
    let mut c = cfg("connections: []\ncredentials:\n- {name: ops, user: r, password: x}\n");
    set_top(&mut c, "vault", params);
    let entry = cfg(&format!("{{host: a, password: '{token}'}}"));
    assert_eq!(password_env(&c, &entry, None), PasswordEnv::Locked);
    assert_eq!(password_env(&c, &entry, Some(&key)), pw_env("pa"));
    let (_, wrong) = vault::create_vault("w").expect("vault");
    assert!(
        matches!(password_env(&c, &entry, Some(&wrong)), PasswordEnv::Warning(w)
            if w.starts_with("Cannot decrypt the saved password:\n"))
    );
    // A named credential lends its password.
    let by_cred = cfg("{host: a, credential: ops}");
    assert_eq!(password_env(&c, &by_cred, None), pw_env("x"));
}

#[test]
fn credentials_follow_renames_and_deletes() {
    let mut c = cfg(
        "connections:\n- {host: a, credential: ops, user: old}\n- {host: b, credential: gone}\ncredentials:\n- {name: ops, user: r}\n- {name: gone, user: g}\n",
    );
    let creds = normalize_credentials(&cfg("[{name: prod, user: root, password: p}]"));
    store_credentials(&mut c, &[("ops".to_owned(), "prod".to_owned())], creds);
    let conns = seq(&c, "connections");
    assert_eq!(
        conns[0].get("credential"),
        Some(&Value::Str("prod".to_owned()))
    );
    assert_eq!(conns[0].get("user"), Some(&Value::Str("root".to_owned())));
    assert!(conns[1].get("credential").is_none());
    store_credentials(&mut c, &[], Vec::new());
    assert!(c.get("credentials").is_none());
}

#[test]
fn key_paths_and_commands() {
    let dir = std::env::temp_dir().join(format!("termigator-u7-keys-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("dir");
    assert_eq!(
        free_key_path(&dir, "ed25519"),
        dir.join("id_ed25519").to_string_lossy()
    );
    std::fs::write(dir.join("id_ed25519.pub"), "x").expect("pub");
    std::fs::write(dir.join("id_ed25519_2"), "x").expect("key");
    assert_eq!(
        free_key_path(&dir, "ed25519"),
        dir.join("id_ed25519_3").to_string_lossy()
    );
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(file_name("/home/u/.ssh/id_rsa"), "id_rsa");
    assert_eq!(
        command_json(&["ssh-add".to_owned(), "/k y".to_owned()], "ssh-add: k y")["inner"],
        "ssh-add '/k y'"
    );
    assert_eq!(KEY_TYPES[0].0, "ed25519");
}

#[test]
fn ssh_entry_list_has_hosts_only() {
    let c = cfg(
        "connections:\n- {label: web, host: h}\n- {host: ''}\n- {protocol: rdp, host: w}\n- {host: bare}\n",
    );
    let rows = ssh_entries(&c);
    let labels: Vec<_> = rows
        .iter()
        .map(|r| r["label"].as_str().unwrap_or(""))
        .collect();
    assert_eq!(labels, ["web", "bare"]);
}
