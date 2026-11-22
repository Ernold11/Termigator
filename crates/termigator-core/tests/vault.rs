//! Vault tests plus known-answer vectors (compat/golden/vault.json).
mod common;

use serde_json::Value as J;
use termigator_core::config::doc::Value;
use termigator_core::vault::{
    Secret, VaultError, create_vault, create_vault_with_salt, derive_key, is_configured, is_sealed,
    is_sealed_v2, migrate_connections, plaintext_password_count, rekey_connections, remember_key,
    reveal_connection, seal, seal_connections, seal_entry, seal_with_nonce, sealed_password_count,
    session_key, strip_plaintext_passwords, unlock, unseal, unseal_connections, unseal_entry,
};

// ---- golden decoding -----------------------------------------------------------

fn value(v: &J) -> Value {
    if let Some((tag, p)) = common::tagged(v) {
        return match tag {
            "$int" => Value::Int(p.as_str().and_then(|s| s.parse().ok()).expect("int")),
            "$map" => Value::Map(
                p.as_array()
                    .into_iter()
                    .flatten()
                    .map(|kv| (value(&kv[0]), value(&kv[1])))
                    .collect(),
            ),
            other => panic!("tag {other}"),
        };
    }
    Value::Str(v.as_str().expect("string").to_owned())
}

fn key(v: &J) -> Secret {
    Secret::new(common::bytes(v))
}

fn arr<const N: usize>(v: &J) -> [u8; N] {
    common::bytes(v).try_into().expect("length")
}

fn message(e: &VaultError) -> String {
    e.to_string()
}

#[test]
fn known_answers_from_golden() {
    let cases = common::golden("vault");
    assert!(cases.len() > 20);
    for c in &cases {
        let name = common::name(c);
        let master = c["master"].as_str().unwrap_or_default();
        match c["kind"].as_str().expect("kind") {
            "kdf" | "unlock" => {
                let params = value(&c["params"]);
                let got = if c["kind"] == "kdf" {
                    derive_key(master, &params)
                } else {
                    unlock(&params, master)
                };
                match (&c["key"], &c["error"]) {
                    (k, J::Null) => assert_eq!(got.as_ref().ok(), Some(&key(k)), "{name}"),
                    (_, e) => {
                        let e = e.as_str().expect("error");
                        let got = got.expect_err(name);
                        if e == "WrongMasterPassword" {
                            assert_eq!(got, VaultError::WrongMasterPassword, "{name}");
                        }
                        if name == "kdf_err_md5" || e == "WrongMasterPassword" {
                            let want = c.get("message").unwrap_or(&c["error"]);
                            assert_eq!(Some(message(&got).as_str()), want.as_str(), "{name}");
                        }
                    }
                }
            }
            "token" => {
                let k = key(&c["key"]);
                let pt = c["plaintext"].as_str().expect("plaintext");
                let token = seal_with_nonce(&k, pt, &arr(&c["nonce"])).expect(name);
                assert_eq!(
                    token,
                    c["token"].as_str().expect("token"),
                    "{name}: rust token"
                );
                assert_eq!(unseal(&k, &token).as_deref(), Ok(pt), "{name}: unseal");
            }
            "vault" => {
                let want = value(&c["params"]);
                let int = |k: &str| match want.get(k) {
                    Some(Value::Int(i)) => *i,
                    other => panic!("{name}: {k} = {other:?}"),
                };
                let (params, k) = create_vault_with_salt(
                    master,
                    &arr(&c["salt"]),
                    &arr(&c["nonce"]),
                    (int("n"), int("r"), int("p")),
                )
                .expect(name);
                assert_eq!(params, want, "{name}: section");
                assert_eq!(k, key(&c["key"]), "{name}: key");
                assert_eq!(unlock(&params, master), Ok(k), "{name}: unlock");
                assert!(is_configured(&params));
            }
            "bad" => {
                let e =
                    unseal(&key(&c["key"]), c["token"].as_str().expect("token")).expect_err(name);
                assert_eq!(message(&e), c["error"].as_str().expect("error"), "{name}");
            }
            other => panic!("{name}: kind {other}"),
        }
    }
}

// ---- vault tests -------------------------------------------------

fn conns() -> Vec<Value> {
    let s = |t: &str| Value::Str(t.into());
    let c = |host: &str, pw: Option<&str>| {
        let mut m = vec![(s("host"), s(host))];
        if let Some(p) = pw {
            m.push((s("password"), s(p)));
        }
        Value::Map(m)
    };
    vec![
        c("a", Some("pa")),
        c("b", None),
        c("c", Some("")),
        s("not-a-dict"),
        c("d", Some("pd")),
    ]
}

fn pw(c: &Value) -> Option<&str> {
    match c.get("password") {
        Some(Value::Str(s)) => Some(s),
        _ => None,
    }
}

/// Small N: the default is slow in debug builds; the format is the same.
fn small_key(master: &str) -> Secret {
    let params = Value::Map(vec![
        (Value::Str("n".into()), Value::Int(16)),
        (
            Value::Str("salt".into()),
            Value::Str("BwcHBwcHBwcHBwcHBwcHBw==".into()),
        ),
    ]);
    derive_key(master, &params).expect("key")
}

#[test]
fn round_trip() {
    let (params, k) = create_vault("correct horse").expect("vault");
    // New vaults use scrypt N=2^17; the value is stored, so old vaults keep theirs.
    assert_eq!(params.get("n"), Some(&Value::Int(1 << 17)));
    let t = seal(&k, "s3cret ąę").expect("seal");
    assert!(is_sealed(&Value::Str(t.clone())));
    assert!(!t.contains("s3cret"));
    assert_eq!(unseal(&k, &t).as_deref(), Ok("s3cret ąę"));
    assert_ne!(seal(&k, "pw"), seal(&k, "pw"));
    assert_eq!(unlock(&params, "correct horse"), Ok(k.clone()));
    assert_eq!(
        unlock(&params, "wrong"),
        Err(VaultError::WrongMasterPassword)
    );
    assert!(!format!("{params:?}").contains("correct horse"));
    assert!(is_configured(&params));
}

#[test]
fn tampered_and_foreign_tokens_rejected() {
    let k = small_key("m");
    let other = small_key("another");
    let t = seal(&k, "pw").expect("seal");
    let body = &t["enc:v1:".len()..];
    let flip = if body.as_bytes()[5] == b'A' { "B" } else { "A" };
    let bad = format!("enc:v1:{}{flip}{}", &body[..5], &body[6..]);
    assert!(unseal(&k, &bad).is_err());
    assert!(unseal(&other, &t).is_err());
    for bad in ["enc:v1:", "enc:v1:!!!", "enc:v1:AAAA", "plain"] {
        assert!(unseal(&k, bad).is_err(), "{bad}");
    }
}

#[test]
fn empty_master_and_corrupt_params() {
    assert!(create_vault("").is_err());
    let s = |t: &str| Value::Str(t.into());
    let bad = Value::Map(vec![
        (s("salt"), s("x")),
        (s("check"), s("y")),
        (s("kdf"), s("md5")),
    ]);
    assert!(unlock(&bad, "m").is_err());
    assert!(unlock(&Value::Map(vec![]), "m").is_err());
    assert!(!is_configured(&Value::Null));
}

#[test]
fn seal_all_and_idempotent() {
    let k = small_key("m");
    let mut cs = conns();
    assert_eq!(seal_connections(&mut cs, &k), Ok(2));
    assert_eq!(plaintext_password_count(&cs), 0);
    assert_eq!(sealed_password_count(&cs), 2);
    let tokens = (pw(&cs[0]).map(str::to_owned), pw(&cs[4]).map(str::to_owned));
    assert_eq!(seal_connections(&mut cs, &k), Ok(0));
    assert_eq!(
        (pw(&cs[0]).map(str::to_owned), pw(&cs[4]).map(str::to_owned)),
        tokens
    );
    assert!(cs[1].get("password").is_none());
    assert_eq!(pw(&cs[2]), Some("")); // empty password is left alone
}

#[test]
fn unseal_restores_and_is_all_or_nothing() {
    let k = small_key("m");
    let mut cs = conns();
    seal_connections(&mut cs, &k).expect("seal");
    let sealed = cs.clone();
    assert_eq!(unseal_connections(&mut cs, &k), Ok(2));
    assert_eq!((pw(&cs[0]), pw(&cs[4])), (Some("pa"), Some("pd")));

    let mut cs = sealed;
    cs[4] = Value::Map(vec![(
        Value::Str("password".into()),
        Value::Str(format!("enc:v1:{}", "A".repeat(32))),
    )]);
    let before = cs.clone();
    assert!(unseal_connections(&mut cs, &k).is_err());
    assert_eq!(cs, before);
}

#[test]
fn rekey() {
    let k = small_key("m");
    let new_key = small_key("new");
    let mut cs = conns();
    seal_connections(&mut cs, &k).expect("seal");
    assert_eq!(rekey_connections(&mut cs, &k, &new_key), Ok(2));
    let t = pw(&cs[0]).expect("sealed").to_owned();
    assert_eq!(unseal_entry(&new_key, &t, &cs[0]).as_deref(), Ok("pa"));
    assert!(unseal_entry(&k, &t, &cs[0]).is_err());

    let before = cs.clone();
    assert!(rekey_connections(&mut cs, &k, &new_key).is_err());
    assert_eq!(cs, before);
}

#[test]
fn reveal() {
    let k = small_key("m");
    let mut cs = conns();
    assert!(matches!(
        reveal_connection(&cs[0], None),
        Ok(termigator_core::vault::Revealed(
            std::borrow::Cow::Borrowed(_)
        ))
    ));
    seal_connections(&mut cs, &k).expect("seal");
    let token = pw(&cs[0]).map(str::to_owned);
    let r = reveal_connection(&cs[0], Some(&k)).expect("reveal");
    assert_eq!(pw(&r), Some("pa"));
    assert_eq!(pw(&cs[0]).map(str::to_owned), token);
    assert_eq!(
        reveal_connection(&cs[0], None)
            .expect_err("locked")
            .to_string(),
        "The vault is locked."
    );
}

#[test]
fn strip_plaintext() {
    let k = small_key("m");
    let mut cs = conns();
    let sealed = seal(&k, "pd").expect("seal");
    cs[4] = Value::Map(vec![(Value::Str("password".into()), Value::Str(sealed))]);
    let out = strip_plaintext_passwords(&cs);
    assert!(out[0].get("password").is_none());
    assert_eq!(out[0].get("host"), Some(&Value::Str("a".into())));
    assert!(is_sealed(out[4].get("password").expect("kept")));
    assert_eq!(pw(&cs[0]), Some("pa")); // original untouched
    assert_eq!(out[3], cs[3]);
}

#[test]
fn integer_password_is_sealed_as_text() {
    let k = small_key("m");
    let mut cs = vec![Value::Map(vec![(
        Value::Str("password".into()),
        Value::Int(1234),
    )])];
    assert_eq!(plaintext_password_count(&cs), 1);
    assert_eq!(seal_connections(&mut cs, &k), Ok(1));
    assert_eq!(unseal_connections(&mut cs, &k), Ok(1));
    assert_eq!(pw(&cs[0]), Some("1234"));
}

#[test]
fn secret_is_masked() {
    let k = Secret::new(vec![1, 2, 3]);
    assert_eq!(format!("{k:?} {k}"), "**** ****");
    assert_eq!(k.expose(), [1, 2, 3]);
}

#[test]
fn session_key_remember_and_forget() {
    assert!(session_key().is_none());
    remember_key(Some(Secret::new(vec![b'k'; 32])));
    assert_eq!(
        session_key().map(|k| k.expose().to_vec()),
        Some(vec![b'k'; 32])
    );
    remember_key(None);
    assert!(session_key().is_none());
}

#[test]
fn wipe_env_clears_every_value() {
    let mut env: std::collections::BTreeMap<std::ffi::OsString, std::ffi::OsString> = [
        ("SSHPASS".into(), "s3cret".into()),
        ("PATH".into(), "/bin".into()),
    ]
    .into();
    termigator_core::vault::wipe_env(&mut env);
    assert!(env.values().all(|v| v.is_empty()) && env.len() == 2);
}

// ---- tokens are bound to their entry -------------------------------------------

#[test]
fn a_token_does_not_open_for_another_entry() {
    let k = small_key("m");
    let mut cs = conns();
    seal_connections(&mut cs, &k).expect("seal");
    let token_a = pw(&cs[0]).expect("sealed").to_owned();
    assert!(is_sealed_v2(cs[0].get("password").expect("password")));
    // Same key, same vault: copying the ciphertext into another entry must not open.
    let b = cs[1].clone();
    let e = unseal_entry(&k, &token_a, &b).expect_err("copied token");
    assert!(e.to_string().contains("does not match this entry"), "{e}");
    // The right entry still opens.
    assert_eq!(unseal_entry(&k, &token_a, &cs[0]).as_deref(), Ok("pa"));
}

#[test]
fn editing_the_target_invalidates_the_token() {
    let k = small_key("m");
    let conn = Value::Map(vec![
        (Value::Str("host".into()), Value::Str("a".into())),
        (Value::Str("user".into()), Value::Str("u".into())),
    ]);
    let token = seal_entry(&k, "pa", &conn).expect("seal");
    assert_eq!(unseal_entry(&k, &token, &conn).as_deref(), Ok("pa"));
    let mut changed = conn.clone();
    if let Value::Map(pairs) = &mut changed {
        pairs[0].1 = Value::Str("b".into());
    }
    assert!(unseal_entry(&k, &token, &changed).is_err());
}

#[test]
fn legacy_v1_tokens_still_open_and_migrate() {
    let k = small_key("m");
    // What earlier versions (unbound v1 tokens) wrote.
    let legacy = seal_with_nonce(&k, "pa", &[7; 12]).expect("seal");
    let mut conn = Value::Map(vec![
        (Value::Str("host".into()), Value::Str("a".into())),
        (Value::Str("password".into()), Value::Str(legacy.clone())),
    ]);
    assert!(!is_sealed_v2(conn.get("password").expect("password")));
    assert_eq!(unseal_entry(&k, &legacy, &conn).as_deref(), Ok("pa"));

    assert_eq!(
        migrate_connections(std::slice::from_mut(&mut conn), &k),
        Ok(1)
    );
    assert!(is_sealed_v2(conn.get("password").expect("password")));
    let migrated = pw(&conn).expect("migrated").to_owned();
    assert!(migrated.starts_with("enc:v2:"), "{migrated}");
    assert_eq!(unseal_entry(&k, &migrated, &conn).as_deref(), Ok("pa"));
    // Idempotent: a v2 token is left as it is (nothing migrated).
    assert_eq!(
        migrate_connections(std::slice::from_mut(&mut conn), &k),
        Ok(0)
    );
    assert_eq!(pw(&conn), Some(migrated.as_str()));
}
