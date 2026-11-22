//! Master-password vault: scrypt + AES-256-GCM, `enc:v2:` tokens.
//!
//! A master password is stretched with scrypt (N, r, p from the `vault` config section,
//! 32-byte output) into the key that seals each saved password as
//! `enc:v2:` + base64(nonce(12) ‖ ciphertext ‖ tag(16)). The associated data is
//! `termigator-vault-v2` **plus the entry's target** (protocol, host, port, user, device), so a
//! token copied from one connection to another does not open. Editing the target of an
//! entry needs the password entered again (it is sealed on save, `seal_pending`).
//!
//! The older `enc:v1:` tokens (fixed associated data `AAD`, see `compat/golden/vault.json`)
//! still open; [`migrate_connections`] re-seals them to v2 when the
//! vault is unlocked. The `check` value stays v1: it belongs to the master, not to an entry.
//!
//! Keys, and nothing derived from the master, live in [`Secret`]: `Debug`/`Display` print
//! `****` and the bytes are zeroized on drop. The process-wide key is kept with
//! [`remember_key`] / [`session_key`].
//!
//! Connection lists are slices of config [`Value`]s; sealing and re-keying are
//! all-or-nothing (nothing is modified unless every entry succeeds).

use crate::platform;
use std::borrow::Cow;
use std::collections::BTreeMap;
use std::ffi::OsString;
use std::fmt;
use std::sync::{Mutex, PoisonError};

use aes_gcm::aead::{Aead, KeyInit, Payload};
use aes_gcm::{Aes256Gcm, Nonce};
use base64::Engine as _;
use base64::alphabet::STANDARD as STANDARD_ALPHABET;
use base64::engine::{DecodePaddingMode, GeneralPurpose, GeneralPurposeConfig};
use zeroize::{Zeroize, Zeroizing};

use crate::config::doc::{Value, canonical_timestamp};

pub const SEALED_PREFIX: &str = "enc:v1:";
/// Current token prefix (associated data binds the token to its entry).
pub const SEALED_PREFIX_V2: &str = "enc:v2:";

/// `n` of a vault section that names none (older files).
const KDF_N: i64 = 1 << 15;
/// `n` for new vaults (~0.15 s, 128 MiB): 2^15 was the minimum of the 2016 scrypt advice, too cheap
/// against offline guessing today. Stored in the section, so old vaults keep theirs.
const NEW_KDF_N: i64 = 1 << 17;
const KDF_R: i64 = 8;
const KDF_P: i64 = 1;
const KEY_LEN: usize = 32;
const NONCE_LEN: usize = 12;
const SALT_LEN: usize = 16;
// Fixed format identifiers of the vault: every existing vault depends on them, never change them.
const AAD: &[u8] = b"ernsole-vault-v1";
/// v2 associated data; the entry's identity is appended after a NUL.
const AAD_V2: &[u8] = b"termigator-vault-v2";
const CHECK_PLAINTEXT: &str = "ernsole-vault-check";

/// Padded standard base64 that ignores non-zero trailing bits.
const B64: GeneralPurpose = GeneralPurpose::new(
    &STANDARD_ALPHABET,
    GeneralPurposeConfig::new()
        .with_decode_allow_trailing_bits(true)
        .with_decode_padding_mode(DecodePaddingMode::RequireCanonical),
);

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum VaultError {
    /// Vault unusable: corrupt token or parameters, locked, empty master.
    #[error("{0}")]
    Vault(String),
    /// The master password does not open the vault.
    #[error("Wrong master password.")]
    WrongMasterPassword,
}

fn err(msg: impl Into<String>) -> VaultError {
    VaultError::Vault(msg.into())
}

/// Secret bytes (vault key): masked in `Debug`/`Display`, zeroized on drop.
#[derive(Clone)]
pub struct Secret(Vec<u8>);

impl Secret {
    pub fn new(bytes: Vec<u8>) -> Self {
        Self(bytes)
    }

    pub fn expose(&self) -> &[u8] {
        &self.0
    }
}

impl Drop for Secret {
    fn drop(&mut self) {
        self.0.zeroize();
    }
}

impl PartialEq for Secret {
    /// Constant-time for equal lengths.
    fn eq(&self, other: &Self) -> bool {
        self.0.len() == other.0.len()
            && self
                .0
                .iter()
                .zip(&other.0)
                .fold(0u8, |a, (x, y)| a | (x ^ y))
                == 0
    }
}

impl Eq for Secret {}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("****")
    }
}

impl fmt::Display for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("****")
    }
}

pub fn is_sealed(value: &Value) -> bool {
    matches!(value, Value::Str(s) if s.starts_with(SEALED_PREFIX) || s.starts_with(SEALED_PREFIX_V2))
}

/// True for a token of the current format (v1 is legacy, migrated on unlock).
pub fn is_sealed_v2(value: &Value) -> bool {
    matches!(value, Value::Str(s) if s.starts_with(SEALED_PREFIX_V2))
}

/// Associated data of an entry's token: v2 + the connection's target. A token sealed for `a` is
/// useless for `b`, which is what makes copying ciphertext between entries fail.
fn entry_aad(conn: &Value) -> Vec<u8> {
    let text = |name: &str| match conn.get(name) {
        Some(Value::Str(s)) => s.clone(),
        Some(Value::Int(i)) => i.to_string(),
        _ => String::new(),
    };
    let protocol = {
        let p = text("protocol");
        if p.is_empty() {
            "ssh".to_owned()
        } else {
            p.to_lowercase()
        }
    };
    let identity = format!(
        "{protocol}\u{0}{}\u{0}{}\u{0}{}\u{0}{}",
        text("host"),
        text("port"),
        text("user"),
        text("device")
    );
    let mut aad = AAD_V2.to_vec();
    aad.push(0);
    aad.extend_from_slice(identity.as_bytes());
    aad
}

/// Truthiness of a YAML value (empty, zero, false and null are falsy).
fn truthy(v: &Value) -> bool {
    match v {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Int(i) => *i != 0,
        Value::Float(f) => *f != 0.0,
        Value::Str(s) => !s.is_empty(),
        Value::Timestamp(_) => true,
        Value::Seq(s) => !s.is_empty(),
        Value::Map(m) => !m.is_empty(),
    }
}

/// `str(value)` for the scalars a password can plausibly be in YAML (`password: 1234`).
fn value_str(v: &Value) -> Result<String, VaultError> {
    Ok(match v {
        Value::Str(s) => s.clone(),
        Value::Int(i) => i.to_string(),
        Value::Bool(true) => "True".into(),
        Value::Bool(false) => "False".into(),
        Value::Float(f) if f.is_nan() => "nan".into(),
        Value::Float(f) => format!("{f:?}"),
        Value::Timestamp(t) => canonical_timestamp(t).unwrap_or_else(|| t.clone()),
        Value::Null | Value::Seq(_) | Value::Map(_) => {
            return Err(err("Unsupported password value."));
        }
    })
}

/// `params` is the `vault` config section: needs a truthy `salt` and `check`.
pub fn is_configured(params: &Value) -> bool {
    matches!(params, Value::Map(_))
        && params.get("salt").is_some_and(truthy)
        && params.get("check").is_some_and(truthy)
}

fn value_int(v: &Value) -> Result<i64, String> {
    match v {
        Value::Int(i) => Ok(*i),
        Value::Bool(b) => Ok(i64::from(*b)),
        Value::Float(f) if f.is_finite() => Ok(f.trunc() as i64),
        Value::Str(s) => s
            .trim()
            .parse()
            .map_err(|_| format!("invalid literal for int(): {s:?}")),
        _ => Err("int() argument must be a string or a number".into()),
    }
}

fn corrupt(e: impl fmt::Display) -> VaultError {
    err(format!("Corrupt vault parameters in config.yaml: {e}"))
}

/// The key for `master` under the `vault` section's KDF parameters (no check against `check`).
pub fn derive_key(master: &str, params: &Value) -> Result<Secret, VaultError> {
    let salt_v = params.get("salt").ok_or_else(|| corrupt("'salt'"))?;
    let Value::Str(salt_text) = salt_v else {
        return Err(corrupt("salt is not text"));
    };
    let salt = B64.decode(salt_text).map_err(corrupt)?;
    let get = |key: &str, default: i64| match params.get(key) {
        Some(v) => value_int(v).map_err(corrupt),
        None => Ok(default),
    };
    let (n, r, p) = (get("n", KDF_N)?, get("r", KDF_R)?, get("p", KDF_P)?);
    if let Some(kdf) = params.get("kdf") {
        let name = value_str(kdf).unwrap_or_default();
        if name != "scrypt" {
            return Err(err(format!("Unsupported vault KDF: '{name}'")));
        }
    }
    // 2^22 = 4 GiB of scrypt memory at r=8; more is a damaged file, not a setting.
    if !(2..=1 << 22).contains(&n) || n & (n - 1) != 0 {
        return Err(corrupt("n must be a power of 2 from 2 to 4194304"));
    }
    let log_n = n.trailing_zeros() as u8;
    let to_u32 = |x: i64, what: &str| {
        u32::try_from(x)
            .ok()
            .filter(|v| *v > 0)
            .ok_or_else(|| corrupt(format!("invalid {what}")))
    };
    let sp = scrypt::Params::new(log_n, to_u32(r, "r")?, to_u32(p, "p")?).map_err(corrupt)?;
    let mut out = vec![0u8; KEY_LEN];
    scrypt::scrypt(master.as_bytes(), &salt, &sp, &mut out).map_err(corrupt)?;
    Ok(Secret(out))
}

fn random<const N: usize>() -> Result<[u8; N], VaultError> {
    let mut buf = [0u8; N];
    getrandom::fill(&mut buf).map_err(|e| err(format!("No system randomness: {e}")))?;
    Ok(buf)
}

fn cipher(key: &Secret) -> Result<Aes256Gcm, VaultError> {
    Aes256Gcm::new_from_slice(key.expose()).map_err(|_| err("Invalid vault key."))
}

/// Encrypt with a random nonce.
pub fn seal(key: &Secret, plaintext: &str) -> Result<String, VaultError> {
    seal_with_nonce(key, plaintext, &random::<NONCE_LEN>()?)
}

/// [`seal`] with a caller-chosen nonce (known-answer tests; never reuse a nonce with one key).
pub fn seal_with_nonce(
    key: &Secret,
    plaintext: &str,
    nonce: &[u8; NONCE_LEN],
) -> Result<String, VaultError> {
    let ct = cipher(key)?
        .encrypt(
            &Nonce::from(*nonce),
            Payload {
                msg: plaintext.as_bytes(),
                aad: AAD,
            },
        )
        .map_err(|_| err("Encryption failed."))?;
    let mut raw = nonce.to_vec();
    raw.extend_from_slice(&ct);
    Ok(format!("{SEALED_PREFIX}{}", B64.encode(raw)))
}

/// Seals `plaintext` for connection `conn` (v2: the token opens only for that target).
pub fn seal_entry(key: &Secret, plaintext: &str, conn: &Value) -> Result<String, VaultError> {
    let aad = entry_aad(conn);
    let nonce = random::<NONCE_LEN>()?;
    let ct = cipher(key)?
        .encrypt(
            &Nonce::from(nonce),
            Payload {
                msg: plaintext.as_bytes(),
                aad: &aad,
            },
        )
        .map_err(|_| err("Encryption failed."))?;
    let mut raw = nonce.to_vec();
    raw.extend_from_slice(&ct);
    Ok(format!("{SEALED_PREFIX_V2}{}", B64.encode(raw)))
}

/// [`seal_entry`] with a caller-chosen nonce (tests).
pub fn seal_entry_with_nonce(
    key: &Secret,
    plaintext: &str,
    conn: &Value,
    nonce: &[u8; NONCE_LEN],
) -> Result<String, VaultError> {
    let aad = entry_aad(conn);
    let ct = cipher(key)?
        .encrypt(
            &Nonce::from(*nonce),
            Payload {
                msg: plaintext.as_bytes(),
                aad: &aad,
            },
        )
        .map_err(|_| err("Encryption failed."))?;
    let mut raw = nonce.to_vec();
    raw.extend_from_slice(&ct);
    Ok(format!("{SEALED_PREFIX_V2}{}", B64.encode(raw)))
}

/// Opens a token for `conn`: v2 checks the entry's target, v1 (legacy) does not (migrated on
/// unlock, see [`migrate_connections`]).
pub fn unseal_entry(key: &Secret, token: &str, conn: &Value) -> Result<String, VaultError> {
    if let Some(body) = token.strip_prefix(SEALED_PREFIX_V2) {
        let raw = B64
            .decode(body)
            .map_err(|_| err("Corrupt encrypted value."))?;
        if raw.len() <= NONCE_LEN {
            return Err(err("Corrupt encrypted value."));
        }
        let (nonce, ct) = raw.split_at(NONCE_LEN);
        let nonce = Nonce::try_from(nonce).map_err(|_| err("Corrupt encrypted value."))?;
        let aad = entry_aad(conn);
        let pt = Zeroizing::new(
            cipher(key)?
                .decrypt(&nonce, Payload { msg: ct, aad: &aad })
                .map_err(|_| err("Encrypted value does not match this entry (or was altered)."))?,
        );
        return String::from_utf8(pt.to_vec()).map_err(|_| err("Corrupt encrypted value."));
    }
    unseal(key, token)
}

pub fn unseal(key: &Secret, token: &str) -> Result<String, VaultError> {
    let Some(body) = token.strip_prefix(SEALED_PREFIX) else {
        return Err(err("Not an encrypted value."));
    };
    let raw = B64
        .decode(body)
        .map_err(|_| err("Corrupt encrypted value."))?;
    if raw.len() <= NONCE_LEN {
        return Err(err("Corrupt encrypted value."));
    }
    let (nonce, ct) = raw.split_at(NONCE_LEN);
    let nonce = Nonce::try_from(nonce).map_err(|_| err("Corrupt encrypted value."))?;
    let pt = Zeroizing::new(
        cipher(key)?
            .decrypt(&nonce, Payload { msg: ct, aad: AAD })
            .map_err(|_| {
                err("Encrypted value does not match the master password (or was altered).")
            })?,
    );
    String::from_utf8(pt.to_vec()).map_err(|_| err("Corrupt encrypted value."))
}

/// New vault parameters (for `cfg['vault']`) and the derived key.
pub fn create_vault(master: &str) -> Result<(Value, Secret), VaultError> {
    create_vault_with_salt(
        master,
        &random::<SALT_LEN>()?,
        &random::<NONCE_LEN>()?,
        (NEW_KDF_N, KDF_R, KDF_P),
    )
}

/// [`create_vault`] with a caller-chosen salt, `check` nonce and scrypt `(n, r, p)`
/// (known-answer tests, small `n`).
pub fn create_vault_with_salt(
    master: &str,
    salt: &[u8; SALT_LEN],
    check_nonce: &[u8; NONCE_LEN],
    (n, r, p): (i64, i64, i64),
) -> Result<(Value, Secret), VaultError> {
    if master.is_empty() {
        return Err(err("The master password must not be empty."));
    }
    let s = |t: &str| Value::Str(t.to_owned());
    let mut pairs = vec![
        (s("kdf"), s("scrypt")),
        (s("n"), Value::Int(n)),
        (s("r"), Value::Int(r)),
        (s("p"), Value::Int(p)),
        (s("salt"), Value::Str(B64.encode(salt))),
    ];
    let key = derive_key(master, &Value::Map(pairs.clone()))?;
    pairs.push((
        s("check"),
        Value::Str(seal_with_nonce(&key, CHECK_PLAINTEXT, check_nonce)?),
    ));
    Ok((Value::Map(pairs), key))
}

/// Derive the key for `master`; [`VaultError::WrongMasterPassword`] if it does not open the vault.
pub fn unlock(params: &Value, master: &str) -> Result<Secret, VaultError> {
    if !is_configured(params) {
        return Err(err("No master password is set."));
    }
    let key = derive_key(master, params)?;
    let check = params
        .get("check")
        .map(value_str)
        .transpose()?
        .unwrap_or_default();
    if unseal(&key, &check).is_ok_and(|p| p == CHECK_PLAINTEXT) {
        Ok(key)
    } else {
        Err(VaultError::WrongMasterPassword)
    }
}

// ---- connection lists --------------------------------------------------------

/// The `password` value of a connection map when it is truthy.
fn password_of(conn: &Value) -> Option<&Value> {
    conn.get("password").filter(|p| truthy(p))
}

fn password_slot(conn: &mut Value) -> Option<&mut Value> {
    let Value::Map(pairs) = conn else { return None };
    pairs
        .iter_mut()
        .find(|(k, _)| matches!(k, Value::Str(s) if s == "password"))
        .map(|(_, v)| v)
        .filter(|v| truthy(v))
}

pub fn plaintext_password_count(conns: &[Value]) -> usize {
    conns
        .iter()
        .filter_map(password_of)
        .filter(|p| !is_sealed(p))
        .count()
}

pub fn sealed_password_count(conns: &[Value]) -> usize {
    conns
        .iter()
        .filter_map(password_of)
        .filter(|p| is_sealed(p))
        .count()
}

/// Run `f` on every sealed (`want_sealed`) or plain password, with its entry; all results are
/// computed before any entry changes, so one failure leaves the list untouched.
fn rewrite(
    conns: &mut [Value],
    want_sealed: bool,
    mut f: impl FnMut(&Value, &Value) -> Result<String, VaultError>,
) -> Result<usize, VaultError> {
    let mut updates: Vec<(usize, String)> = Vec::new();
    for (index, conn) in conns.iter().enumerate() {
        let Some(password) = password_of(conn).filter(|v| is_sealed(v) == want_sealed) else {
            continue;
        };
        updates.push((index, f(conn, password)?));
    }
    let n = updates.len();
    for (index, token) in updates {
        if let Some(slot) = password_slot(&mut conns[index]) {
            *slot = Value::Str(token);
        }
    }
    Ok(n)
}

/// Encrypt every plain-text `password` in place (v2: bound to the entry's target).
pub fn seal_connections(conns: &mut [Value], key: &Secret) -> Result<usize, VaultError> {
    rewrite(conns, false, |conn, v| {
        seal_entry(key, &value_str(v)?, conn)
    })
}

/// Decrypt every sealed `password` in place (removing the vault / re-keying).
pub fn unseal_connections(conns: &mut [Value], key: &Secret) -> Result<usize, VaultError> {
    rewrite(conns, true, |conn, v| {
        unseal_entry(key, &value_str(v)?, conn)
    })
}

/// Re-encrypt sealed passwords under `new_key` (master password change); v1 tokens come out as v2.
pub fn rekey_connections(
    conns: &mut [Value],
    old_key: &Secret,
    new_key: &Secret,
) -> Result<usize, VaultError> {
    rewrite(conns, true, |conn, v| {
        let plain = Zeroizing::new(unseal_entry(old_key, &value_str(v)?, conn)?);
        seal_entry(new_key, &plain, conn)
    })
}

/// Re-seal legacy v1 tokens to v2; called when the vault is unlocked, so a token copied
/// between entries stops opening. Returns how many were migrated (v2 tokens are left alone).
pub fn migrate_connections(conns: &mut [Value], key: &Secret) -> Result<usize, VaultError> {
    let mut updates: Vec<(usize, String)> = Vec::new();
    for (index, conn) in conns.iter().enumerate() {
        let Some(token) = password_of(conn).filter(|v| is_sealed(v) && !is_sealed_v2(v)) else {
            continue;
        };
        let plain = Zeroizing::new(unseal_entry(key, &value_str(token)?, conn)?);
        updates.push((index, seal_entry(key, &plain, conn)?));
    }
    let n = updates.len();
    for (index, token) in updates {
        if let Some(slot) = password_slot(&mut conns[index]) {
            *slot = Value::Str(token);
        }
    }
    Ok(n)
}

/// A connection from [`reveal_connection`]. An owned one carries the decrypted password, which
/// is wiped when it is dropped; a borrowed one is the caller's own (nothing was sealed).
#[derive(Debug)]
pub struct Revealed<'a>(pub Cow<'a, Value>);

impl Revealed<'_> {
    /// An owned copy that no longer borrows the input (wiped on drop as well).
    pub fn into_static(mut self) -> Revealed<'static> {
        Revealed(Cow::Owned(
            std::mem::replace(&mut self.0, Cow::Owned(Value::Null)).into_owned(),
        ))
    }
}

impl std::ops::Deref for Revealed<'_> {
    type Target = Value;
    fn deref(&self) -> &Value {
        &self.0
    }
}

impl Drop for Revealed<'_> {
    fn drop(&mut self) {
        if let Cow::Owned(Value::Map(pairs)) = &mut self.0 {
            for (k, v) in pairs {
                if let (Value::Str(k), Value::Str(p)) = (k, v)
                    && k == "password"
                {
                    p.zeroize();
                }
            }
        }
    }
}

/// Overwrites every value of an environment map built for a child, once it has been spawned.
pub fn wipe_env(env: &mut BTreeMap<OsString, OsString>) {
    for v in env.values_mut() {
        platform::os_string_into_vec(std::mem::take(v)).zeroize();
    }
}

/// The connection with its password decrypted — what `ssh_password_env` /
/// `rdp_password_env` must receive. Anything not sealed is returned as is (borrowed).
pub fn reveal_connection<'a>(
    conn: &'a Value,
    key: Option<&Secret>,
) -> Result<Revealed<'a>, VaultError> {
    let Some(pw) = conn.get("password").filter(|p| is_sealed(p)) else {
        return Ok(Revealed(Cow::Borrowed(conn)));
    };
    let key = key.ok_or_else(|| err("The vault is locked."))?;
    let plain = Zeroizing::new(unseal_entry(key, &value_str(pw)?, conn)?);
    let Value::Map(pairs) = conn else {
        return Ok(Revealed(Cow::Borrowed(conn)));
    };
    let mut out = pairs.clone();
    for (k, v) in &mut out {
        if matches!(k, Value::Str(s) if s == "password") {
            *v = Value::Str(plain.to_string());
        }
    }
    Ok(Revealed(Cow::Owned(Value::Map(out))))
}

/// Disk-safe copy of a connection list: plain-text passwords dropped, sealed kept.
pub fn strip_plaintext_passwords(conns: &[Value]) -> Vec<Value> {
    conns
        .iter()
        .map(|c| match (c, password_of(c)) {
            (Value::Map(pairs), Some(p)) if !is_sealed(p) => Value::Map(
                pairs
                    .iter()
                    .filter(|(k, _)| !matches!(k, Value::Str(s) if s == "password"))
                    .cloned()
                    .collect(),
            ),
            _ => c.clone(),
        })
        .collect()
}

// ---- per-process key ----------------------------------------------------------

static SESSION_KEY: Mutex<Option<Secret>> = Mutex::new(None);

pub fn remember_key(key: Option<Secret>) {
    *SESSION_KEY.lock().unwrap_or_else(PoisonError::into_inner) = key;
}

pub fn session_key() -> Option<Secret> {
    SESSION_KEY
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone()
}
