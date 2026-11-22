//! Vault, named credentials and SSH keys: `VaultBackend`, exposed to QML as
//! `Termigator.VaultBackend`; the dialogs are in `qml/vault/`.
//!
//! Everything the vault, credentials and SSH keys UI decides lives here:
//! the unlock state ([`password_env`], [`status`]), the Set / Change / Remove master password
//! flows on the connection and credential lists ([`master_set`] / [`master_change`] /
//! [`master_remove`]), sealing before a save ([`seal_pending`], called by `AppController.persist`),
//! the credential list edits and the SSH key list and commands. The slow steps (scrypt, the
//! `ssh-keygen -l` per key, `ssh-add -l`) run on a worker thread and answer with a signal; the
//! results are applied to the shared config on the GUI thread. Master passwords and keys are never
//! logged or put into a result; the only secrets in a result are sealed tokens.

use std::path::{Path, PathBuf};

use cxx_qt::Threading;
use cxx_qt_lib::QString;
use serde_json::{Value as J, json};
use termigator_core::config::doc::Value;
use termigator_core::config::model::{PROTOCOL_SSH, connection_protocol, truthy, value_str};
use termigator_core::credentials::{
    apply_credential, find_credential, normalize_credentials, sync_references, usage_count,
};
use termigator_core::proto::ssh::ssh_password_env;
use termigator_core::proto::ssh_keys::{
    self, add_to_agent_argv, agent_fingerprints, copy_id_argv, keygen_argv,
};
use termigator_core::proto::vnc::which_in_path;
use termigator_core::shell;
use termigator_core::vault::{self, Secret};

use crate::shell_qobj::with_app;

#[cxx_qt::bridge]
pub mod qobject {
    unsafe extern "C++" {
        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;
    }

    #[auto_cxx_name]
    extern "RustQt" {
        #[qobject]
        #[qml_element]
        #[qproperty(QString, ssh_dir)]
        type VaultBackend = super::VaultBackendRust;

        /// `start_unlock` finished: `{ok, error, fatal}` (`fatal`: the vault is unusable, not a
        /// wrong password).
        #[qsignal]
        fn unlock_finished(self: Pin<&mut VaultBackend>, token: i32, result_json: QString);

        /// `start_set` / `start_change` / `start_remove` finished: `{ok, message, warning}`. The
        /// config is already changed when `ok`; saving it is the caller's (`controller.persist()`).
        #[qsignal]
        fn master_finished(self: Pin<&mut VaultBackend>, token: i32, result_json: QString);

        /// `start_list_keys` finished: `{agent, keys: [{name, path, pub, type, fingerprint,
        /// comment, inAgent}]}`.
        #[qsignal]
        fn keys_listed(self: Pin<&mut VaultBackend>, token: i32, result_json: QString);

        /// True when sealed passwords exist but the master password was not entered this run.
        #[qinvokable]
        fn needs_unlock(self: &VaultBackend) -> bool;

        /// Forgets the key in memory (Change / Remove re-confirm the current master).
        #[qinvokable]
        fn forget_key(self: &VaultBackend);

        /// Checks `master` against the vault off the GUI thread; answers with `unlock_finished`.
        #[qinvokable]
        fn start_unlock(self: &VaultBackend, token: i32, master: &QString);

        /// The Master password dialog's state: `{text, canSet, canChange, canRemove}`.
        #[qinvokable]
        fn status_json(self: &VaultBackend) -> QString;

        /// Creates the vault and seals every password; answers with `master_finished`.
        #[qinvokable]
        fn start_set(self: &VaultBackend, token: i32, master: &QString);

        /// Re-keys under a new master (the current one must be unlocked); `master_finished`.
        #[qinvokable]
        fn start_change(self: &VaultBackend, token: i32, master: &QString);

        /// Decrypts every password and drops the vault (unlocked first); `master_finished`.
        #[qinvokable]
        fn start_remove(self: &VaultBackend, token: i32);

        /// The environment for a command that needs the entry's saved password:
        /// `{state: "none"|"env"|"locked"|"error", env, warning}`.
        #[qinvokable]
        fn password_env(self: &VaultBackend, entry_json: &QString) -> QString;

        /// The named credentials (normalized) as a JSON array.
        #[qinvokable]
        fn credentials_json(self: &VaultBackend) -> QString;

        /// Whether `password` is an encrypted token (`enc:v2:`, or legacy `enc:v1:`) — never shown in a field.
        #[qinvokable]
        fn is_sealed(self: &VaultBackend, password: &QString) -> bool;

        /// How many entries use the credential `name`.
        #[qinvokable]
        fn credential_usage(self: &VaultBackend, name: &QString) -> i32;

        /// OK in the Credentials dialog: normalizes `creds_json`, follows `renames_json`
        /// (`{old: new}`) into the entries and stores the list. Not saved; call `persist`.
        #[qinvokable]
        fn apply_credentials(self: &VaultBackend, creds_json: &QString, renames_json: &QString);

        /// Lists the key pairs and the agent off the GUI thread; answers with `keys_listed`.
        #[qinvokable]
        fn start_list_keys(self: &VaultBackend, token: i32);

        /// The first unused `id_<type>` path in the SSH directory.
        #[qinvokable]
        fn free_key_path(self: &VaultBackend, key_type: &QString) -> QString;

        /// `user@host` for the new key's comment.
        #[qinvokable]
        fn default_comment(self: &VaultBackend) -> QString;

        /// `ssh-keygen` for the Generate dialog: `{inner, title}` or `{error}`.
        #[qinvokable]
        fn keygen_command(
            self: &VaultBackend,
            key_type: &QString,
            path: &QString,
            comment: &QString,
        ) -> QString;

        /// `ssh-add KEY` as `{inner, title}`.
        #[qinvokable]
        fn add_to_agent_command(self: &VaultBackend, key: &QString) -> QString;

        /// The SSH entries with a host: `[{label, entry}]`.
        #[qinvokable]
        fn ssh_entries_json(self: &VaultBackend) -> QString;

        /// `ssh-copy-id` for an entry: `{inner, title}` or `{error}`.
        #[qinvokable]
        fn copy_id_command(
            self: &VaultBackend,
            pub_path: &QString,
            entry_json: &QString,
            with_password: bool,
        ) -> QString;

        /// The text of a public key file: `{text}` or `{error}`.
        #[qinvokable]
        fn read_public_key(self: &VaultBackend, pub_path: &QString) -> QString;
    }

    impl cxx_qt::Threading for VaultBackend {}
}

pub struct VaultBackendRust {
    ssh_dir: QString,
}

impl Default for VaultBackendRust {
    fn default() -> Self {
        Self {
            ssh_dir: QString::from(default_ssh_dir().to_string_lossy().as_ref()),
        }
    }
}

fn qs(text: &str) -> QString {
    QString::from(text)
}

fn parse(invokable: &str, text: &QString) -> J {
    crate::qml_json::value(invokable, &text.to_string()).unwrap_or(J::Null)
}

impl qobject::VaultBackend {
    fn ssh_dir_path(&self) -> PathBuf {
        PathBuf::from(self.ssh_dir().to_string())
    }

    fn needs_unlock(&self) -> bool {
        with_app(|app| needs_unlock(&app.cfg))
    }

    fn forget_key(&self) {
        vault::remember_key(None);
    }

    fn start_unlock(&self, token: i32, master: &QString) {
        let params = with_app(|app| app.cfg.get("vault").cloned().unwrap_or(Value::Null));
        let master = master.to_string();
        let thread = self.qt_thread();
        run_worker("termigator-unlock", move || {
            let result = match vault::unlock(&params, &master) {
                Ok(key) => Ok(key),
                Err(vault::VaultError::WrongMasterPassword) => {
                    Err(("Wrong master password — try again.".to_owned(), false))
                }
                Err(e) => Err((e.to_string(), true)),
            };
            let _ = thread.queue(move |mut m| {
                let json = match result {
                    Ok(key) => {
                        vault::remember_key(Some(key));
                        json!({"ok": true, "error": "", "fatal": false})
                    }
                    Err((error, fatal)) => json!({"ok": false, "error": error, "fatal": fatal}),
                };
                m.as_mut().unlock_finished(token, qs(&json.to_string()));
            });
        });
    }

    fn status_json(&self) -> QString {
        let s = with_app(|app| status(&app.cfg, vault::session_key().is_some()));
        qs(
            &json!({"text": s.text, "canSet": s.can_set, "canChange": s.can_change,
            "canRemove": s.can_remove})
            .to_string(),
        )
    }

    fn start_set(&self, token: i32, master: &QString) {
        let master = master.to_string();
        self.start_master(token, move |list| master_set(list, &master));
    }

    fn start_change(&self, token: i32, master: &QString) {
        let master = master.to_string();
        self.start_master(token, move |list| match vault::session_key() {
            Some(old) => master_change(list, &old, &master),
            None => Err("The vault is locked.".to_owned()),
        });
    }

    fn start_remove(&self, token: i32) {
        self.start_master(token, |list| match vault::session_key() {
            Some(key) => master_remove(list, &key),
            None => Err("The vault is locked.".to_owned()),
        });
    }

    /// Runs `work` on the connection and credential lists (a copy) off the GUI thread, then
    /// stores the outcome in the config on the GUI thread.
    fn start_master(
        &self,
        token: i32,
        work: impl FnOnce(&mut Vec<Value>) -> Result<MasterDone, String> + Send + 'static,
    ) {
        let (mut list, n_conns) = with_app(|app| combined(&app.cfg));
        let before = list.clone();
        let thread = self.qt_thread();
        run_worker("termigator-vault", move || {
            let done = work(&mut list).map(|d| (d, list));
            let _ = thread.queue(move |mut m| {
                let json = match done {
                    Ok((done, list)) => {
                        let message = done.message.clone();
                        // The lists must be what the worker started from, or its result would
                        // overwrite a change made meanwhile.
                        let stored = with_app(|app| {
                            let same = combined(&app.cfg) == (before, n_conns);
                            if same {
                                apply_master(&mut app.cfg, list, n_conns, done);
                            }
                            same
                        });
                        if stored {
                            json!({"ok": true, "message": message, "warning": ""})
                        } else {
                            json!({"ok": false, "message": "", "warning": "The connection list changed meanwhile — nothing was changed, try again."})
                        }
                    }
                    Err(warning) => json!({"ok": false, "message": "", "warning": warning}),
                };
                m.as_mut().master_finished(token, qs(&json.to_string()));
            });
        });
    }

    fn password_env(&self, entry_json: &QString) -> QString {
        let entry = Value::from_json(&parse("password_env", entry_json));
        let key = vault::session_key();
        let out = with_app(|app| password_env(&app.cfg, &entry, key.as_ref()));
        qs(&match out {
            PasswordEnv::None => json!({"state": "none", "env": {}, "warning": ""}),
            PasswordEnv::Locked => json!({"state": "locked", "env": {}, "warning": ""}),
            PasswordEnv::Warning(w) => json!({"state": "error", "env": {}, "warning": w}),
            PasswordEnv::Env(pairs) => {
                let env: serde_json::Map<String, J> =
                    // shortcut: the password leaves zeroizing memory for QML here, upgrade with
                    // panes env_json.
                    pairs
                        .into_iter()
                        .map(|(k, v)| (k, J::String(String::from_utf8_lossy(v.expose()).into_owned())))
                        .collect();
                json!({"state": "env", "env": env, "warning": ""})
            }
        }
        .to_string())
    }

    fn credentials_json(&self) -> QString {
        let list = with_app(|app| {
            normalize_credentials(app.cfg.get("credentials").unwrap_or(&Value::Null))
        });
        qs(&J::Array(list.iter().map(Value::to_json).collect()).to_string())
    }

    fn is_sealed(&self, password: &QString) -> bool {
        vault::is_sealed(&Value::Str(password.to_string()))
    }

    fn credential_usage(&self, name: &QString) -> i32 {
        let name = name.to_string();
        let n = with_app(|app| usage_count(seq(&app.cfg, "connections"), &name));
        i32::try_from(n).unwrap_or(i32::MAX)
    }

    fn apply_credentials(&self, creds_json: &QString, renames_json: &QString) {
        let creds =
            normalize_credentials(&Value::from_json(&parse("apply_credentials", creds_json)));
        let renames: Vec<(String, String)> = match parse("apply_credentials", renames_json) {
            J::Object(map) => map
                .into_iter()
                .filter_map(|(old, new)| Some((old, new.as_str()?.to_owned())))
                .collect(),
            _ => Vec::new(),
        };
        with_app(|app| store_credentials(&mut app.cfg, &renames, creds));
    }

    fn start_list_keys(&self, token: i32) {
        let dir = self.ssh_dir_path();
        let thread = self.qt_thread();
        run_worker("termigator-keys", move || {
            let out = keys_json(&dir);
            let _ = thread.queue(move |mut m| m.as_mut().keys_listed(token, qs(&out.to_string())));
        });
    }

    fn free_key_path(&self, key_type: &QString) -> QString {
        qs(&free_key_path(&self.ssh_dir_path(), &key_type.to_string()))
    }

    fn default_comment(&self) -> QString {
        qs(&default_comment())
    }

    fn keygen_command(&self, key_type: &QString, path: &QString, comment: &QString) -> QString {
        let home = std::env::var("HOME").ok();
        let out = keygen_argv(
            &key_type.to_string(),
            &path.to_string(),
            &comment.to_string(),
            home.as_deref(),
        )
        .map(|argv| {
            let name = argv
                .iter()
                .position(|a| a == "-f")
                .and_then(|i| argv.get(i + 1))
                .map(|p| file_name(p))
                .unwrap_or_default();
            command_json(&argv, &format!("ssh-keygen: {name}"))
        })
        .unwrap_or_else(|e| json!({"error": e}));
        qs(&out.to_string())
    }

    fn add_to_agent_command(&self, key: &QString) -> QString {
        let key = key.to_string();
        let argv = add_to_agent_argv(Path::new(&key));
        qs(&command_json(&argv, &format!("ssh-add: {}", file_name(&key))).to_string())
    }

    fn ssh_entries_json(&self) -> QString {
        let rows = with_app(|app| ssh_entries(&app.cfg));
        qs(&J::Array(rows).to_string())
    }

    fn copy_id_command(
        &self,
        pub_path: &QString,
        entry_json: &QString,
        with_password: bool,
    ) -> QString {
        let entry = Value::from_json(&parse("copy_id_command", entry_json));
        let out = copy_id_argv(
            Path::new(&pub_path.to_string()),
            &entry,
            with_password,
            &which_in_path,
        )
        .map(|argv| command_json(&argv, &format!("ssh-copy-id: {}", entry_name(&entry))))
        .unwrap_or_else(|e| json!({"error": e}));
        qs(&out.to_string())
    }

    fn read_public_key(&self, pub_path: &QString) -> QString {
        let path = pub_path.to_string();
        let out = match std::fs::read_to_string(&path) {
            Ok(text) => json!({"text": text.trim()}),
            Err(e) => json!({"error": format!("Could not read {path}: {e}")}),
        };
        qs(&out.to_string())
    }
}

/// A plain thread; once the backend is gone the queued result is dropped (as the probes do).
fn run_worker(name: &str, f: impl FnOnce() + Send + 'static) {
    // A spawn that fails leaves the dialog waiting; nothing better to do than not crash.
    let _ = std::thread::Builder::new().name(name.to_owned()).spawn(f);
}

// ---- Pure logic (unit-tested below) ------------------------------------------------------

fn default_ssh_dir() -> PathBuf {
    std::env::var_os("HOME")
        .map_or_else(|| PathBuf::from("."), PathBuf::from)
        .join(".ssh")
}

fn seq<'a>(cfg: &'a Value, key: &str) -> &'a [Value] {
    match cfg.get(key) {
        Some(Value::Seq(items)) => items,
        _ => &[],
    }
}

fn set_top(cfg: &mut Value, key: &str, value: Value) {
    if let Value::Map(pairs) = cfg {
        match pairs
            .iter_mut()
            .find(|(k, _)| matches!(k, Value::Str(s) if s == key))
        {
            Some((_, v)) => *v = value,
            None => pairs.push((Value::Str(key.to_owned()), value)),
        }
    }
}

fn remove_top(cfg: &mut Value, key: &str) {
    if let Value::Map(pairs) = cfg {
        pairs.retain(|(k, _)| !matches!(k, Value::Str(s) if s == key));
    }
}

/// The entries, then the named credentials — everything holding a
/// `password` — and how many of them are entries.
fn combined(cfg: &Value) -> (Vec<Value>, usize) {
    let conns = seq(cfg, "connections");
    let list = conns
        .iter()
        .chain(seq(cfg, "credentials"))
        .cloned()
        .collect();
    (list, conns.len())
}

/// Puts the (changed) combined list back where it came from.
fn store_combined(cfg: &mut Value, mut list: Vec<Value>, n_conns: usize) {
    let creds = list.split_off(n_conns.min(list.len()));
    if matches!(cfg.get("connections"), Some(Value::Seq(_))) {
        set_top(cfg, "connections", Value::Seq(list));
    }
    if matches!(cfg.get("credentials"), Some(Value::Seq(_))) {
        set_top(cfg, "credentials", Value::Seq(creds));
    }
}

/// The vault section is configured, so the saved passwords are sealed, and the key is not in
/// memory yet.
fn needs_unlock(cfg: &Value) -> bool {
    cfg.get("vault").is_some_and(vault::is_configured) && vault::session_key().is_none()
}

/// What `AppController.persist` found before saving.
#[derive(Debug, PartialEq, Eq)]
pub enum Seal {
    /// No vault or no plain-text password: nothing to do.
    Nothing,
    /// New plain-text passwords were encrypted in the config.
    Sealed,
    /// They need the master password, which was not entered this run: the file is written
    /// without them and the UI asks.
    NeedsUnlock,
}

/// Encrypts new plain-text passwords, without any prompt.
pub fn seal_pending(cfg: &mut Value) -> Seal {
    if !cfg.get("vault").is_some_and(vault::is_configured) {
        return Seal::Nothing;
    }
    // With the key in memory, upgrade legacy v1 tokens to v2 (bound to their entry). A
    // token that cannot be opened stays v1 (it still opens the old way).
    if let Some(key) = vault::session_key() {
        let (mut list, n_conns) = combined(cfg);
        if vault::migrate_connections(&mut list, &key).is_ok_and(|n| n > 0) {
            store_combined(cfg, list, n_conns);
        }
    }
    let (mut list, n_conns) = combined(cfg);
    if vault::plaintext_password_count(&list) == 0 {
        return Seal::Nothing;
    }
    let Some(key) = vault::session_key() else {
        return Seal::NeedsUnlock;
    };
    // A password that cannot be sealed stays plain in memory and is not written (save strips it).
    if vault::seal_connections(&mut list, &key).is_err() {
        return Seal::Nothing;
    }
    store_combined(cfg, list, n_conns);
    Seal::Sealed
}

/// The Master password dialog's text and buttons (the "cryptography package
/// is missing" state does not exist here: the crypto is compiled in).
pub struct Status {
    pub text: String,
    pub can_set: bool,
    pub can_change: bool,
    pub can_remove: bool,
}

pub fn status(cfg: &Value, unlocked: bool) -> Status {
    let (list, _) = combined(cfg);
    let plain = vault::plaintext_password_count(&list);
    let sealed = vault::sealed_password_count(&list);
    let on = cfg.get("vault").is_some_and(vault::is_configured);
    let text = if on {
        let mut text = format!("✔ Master password is set — {sealed} saved password(s) encrypted.");
        if plain > 0 {
            text += &format!(
                "\n⚠ {plain} new password(s) not encrypted yet (sealed on the next save)."
            );
        }
        if !unlocked {
            text += "\nLocked — the master password is asked when a saved password is needed.";
        }
        text
    } else if plain > 0 {
        format!("⚠ {plain} saved password(s) stored in plain text in session.yaml.")
    } else {
        "No master password — saved passwords are stored in plain text in session.yaml.".to_owned()
    };
    Status {
        text,
        can_set: !on,
        can_change: on,
        can_remove: on,
    }
}

/// What a master password action changes besides the list.
pub enum VaultChange {
    /// `vault` section and session key replaced (Set, Change).
    Replace(Value, Secret),
    /// `vault` section dropped, session key forgotten (Remove).
    Drop,
}

pub struct MasterDone {
    pub change: VaultChange,
    pub message: String,
}

fn err_text(e: &vault::VaultError) -> String {
    e.to_string()
}

/// Set: create the vault and seal every plain password.
pub fn master_set(list: &mut [Value], master: &str) -> Result<MasterDone, String> {
    let (params, key) = vault::create_vault(master).map_err(|e| err_text(&e))?;
    let n = vault::seal_connections(list, &key).map_err(|e| err_text(&e))?;
    Ok(MasterDone {
        change: VaultChange::Replace(params, key),
        message: format!("Master password set — {n} saved password(s) encrypted."),
    })
}

/// Change: re-encrypt under a new master, then seal the passwords that were still plain.
pub fn master_change(list: &mut [Value], old: &Secret, master: &str) -> Result<MasterDone, String> {
    let (params, new_key) = vault::create_vault(master).map_err(|e| err_text(&e))?;
    let n = vault::rekey_connections(list, old, &new_key)
        .map_err(|e| format!("Nothing changed — a saved password could not be decrypted:\n{e}"))?;
    vault::seal_connections(list, &new_key).map_err(|e| err_text(&e))?;
    Ok(MasterDone {
        change: VaultChange::Replace(params, new_key),
        message: format!("Master password changed — {n} password(s) re-encrypted."),
    })
}

/// Remove: decrypt every password.
pub fn master_remove(list: &mut [Value], key: &Secret) -> Result<MasterDone, String> {
    vault::unseal_connections(list, key)
        .map_err(|e| format!("Nothing changed — a saved password could not be decrypted:\n{e}"))?;
    Ok(MasterDone {
        change: VaultChange::Drop,
        message: "Master password removed — saved passwords are plain text again.".to_owned(),
    })
}

fn apply_master(cfg: &mut Value, list: Vec<Value>, n_conns: usize, done: MasterDone) {
    store_combined(cfg, list, n_conns);
    match done.change {
        VaultChange::Replace(params, key) => {
            set_top(cfg, "vault", params);
            vault::remember_key(Some(key));
        }
        VaultChange::Drop => {
            remove_top(cfg, "vault");
            vault::remember_key(None);
        }
    }
}

/// The result of asking for the saved password of an entry.
#[derive(Debug, PartialEq, Eq)]
pub enum PasswordEnv {
    /// The entry has no saved password: it is typed in the terminal.
    None,
    Env(Vec<(String, Secret)>),
    /// Sealed and the master password was not entered yet.
    Locked,
    Warning(String),
}

/// `ssh_password_env(vault_ui.reveal(entry))` of the key install flow.
pub fn password_env(cfg: &Value, entry: &Value, key: Option<&Secret>) -> PasswordEnv {
    let creds = seq(cfg, "credentials");
    // A token was sealed on the entry that carries it: the credential's own entry when the
    // connection only references one (a v2 token is bound to that entry's fields).
    let owner = entry
        .get("credential")
        .and_then(|name| find_credential(creds, name))
        .unwrap_or(entry)
        .clone();
    let mut c = apply_credential(entry, creds);
    let Some(password) = owner.get("password").filter(|p| truthy(p)) else {
        return PasswordEnv::None;
    };
    if vault::is_sealed(password)
        && key.is_none()
        && cfg.get("vault").is_some_and(vault::is_configured)
    {
        return PasswordEnv::Locked;
    }
    // Not sealed: borrowed as is, never fails.
    let revealed = match vault::reveal_connection(&owner, key) {
        Ok(v) => v,
        Err(e) => return PasswordEnv::Warning(format!("Cannot decrypt the saved password:\n{e}")),
    };
    let plain = revealed.get("password").map(value_str).unwrap_or_default();
    if let Value::Map(pairs) = &mut c {
        for (k, v) in pairs {
            if matches!(k, Value::Str(k) if k == "password") {
                *v = Value::Str(plain.clone());
            }
        }
    }
    match ssh_password_env(&c) {
        Ok(env) if env.is_empty() => PasswordEnv::None,
        Ok(env) => PasswordEnv::Env(env),
        Err(e) => PasswordEnv::Warning(e.to_string()),
    }
}

/// OK in the Credentials dialog: follow renames / user changes / deletes into the entries, then
/// store the list (dropped when empty).
fn store_credentials(cfg: &mut Value, renames: &[(String, String)], creds: Vec<Value>) {
    if let Some(Value::Seq(conns)) = cfg.get("connections").cloned().as_mut() {
        sync_references(conns, renames, &creds);
        set_top(cfg, "connections", Value::Seq(std::mem::take(conns)));
    }
    if creds.is_empty() {
        remove_top(cfg, "credentials");
    } else {
        set_top(cfg, "credentials", Value::Seq(creds));
    }
}

fn file_name(path: &str) -> String {
    Path::new(path)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// `{inner, title}`: the command line for `openInner` (the argv shell-joined).
fn command_json(argv: &[String], title: &str) -> J {
    json!({"inner": shell::join(argv), "title": title})
}

/// The entry's label, else its host.
fn entry_name(entry: &Value) -> String {
    ["label", "host"]
        .iter()
        .filter_map(|k| entry.get(k).filter(|v| truthy(v)))
        .map(value_str)
        .next()
        .unwrap_or_default()
}

fn ssh_entries(cfg: &Value) -> Vec<J> {
    seq(cfg, "connections")
        .iter()
        .filter(|c| matches!(c, Value::Map(_)) && connection_protocol(c) == PROTOCOL_SSH)
        .filter(|c| c.get("host").is_some_and(truthy))
        .map(|c| json!({"label": entry_name(c), "entry": c.to_json()}))
        .collect()
}

fn free_key_path(ssh_dir: &Path, key_type: &str) -> String {
    let taken = |p: &Path| p.exists() || PathBuf::from(format!("{}.pub", p.display())).exists();
    let mut p = ssh_dir.join(format!("id_{key_type}"));
    let mut n = 2;
    while taken(&p) {
        p = ssh_dir.join(format!("id_{key_type}_{n}"));
        n += 1;
    }
    p.to_string_lossy().into_owned()
}

/// `user@hostname` of the current user and machine.
fn default_comment() -> String {
    let user = ["LOGNAME", "USER", "LNAME", "USERNAME"]
        .iter()
        .find_map(|k| std::env::var(k).ok().filter(|v| !v.is_empty()))
        .unwrap_or_else(|| "user".to_owned());
    let host = termigator_core::platform::hostname().unwrap_or_else(|| "localhost".to_owned());
    format!("{user}@{host}")
}

fn keys_json(dir: &Path) -> J {
    let keys = ssh_keys::list_keys(dir);
    let loaded = agent_fingerprints();
    let rows: Vec<J> = keys
        .iter()
        .map(|k| {
            let in_agent = match &loaded {
                None => "—",
                Some(set) if set.contains(&k.fingerprint) => "✔",
                Some(_) => "",
            };
            json!({
                "name": k.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
                "path": k.path.to_string_lossy(),
                "pub": k.pub_path.to_string_lossy(),
                "type": format!("{} {}", k.key_type, k.bits),
                "fingerprint": k.fingerprint,
                "comment": k.comment,
                "inAgent": in_agent,
            })
        })
        .collect();
    json!({"agent": loaded.is_some(), "keys": rows})
}

#[cfg(test)]
mod tests;
