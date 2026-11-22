//! Import, export and backup: `ImportBackend`, exposed to QML as `Termigator.ImportBackend`.
//!
//! The parsers and the writers are in `termigator-core` (`import::*`, `export`); this file
//! merges their rows into the shared config and words the result.
//! File pickers and message boxes are `qml/import/ImportHost.qml`.
//!
//! Every action returns JSON `{text, changed, warn}`: `text` is the message box, `changed` says the
//! config now differs (QML then calls `controller.persist()`, whose `configChanged` refreshes the
//! host list), `warn` picks a warning box over an information box.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use cxx_qt_lib::{QDateTime, QString};
use serde_json::json;
use termigator_core::config::doc::Value;
use termigator_core::config::model::default_terminal_from_cfg;
use termigator_core::export::{Stamp, build_backup_zip, export_ssh_config};
use termigator_core::import::moba::{
    decode_moba_bytes, local_user, merge_moba_connections, parse_moba_export,
};
use termigator_core::import::putty::{
    default_sessions_dir, parse_putty_export, parse_putty_reg, parse_putty_sessions_dir,
};
use termigator_core::import::remmina::{parse_remmina_profiles, remmina_dirs};
use termigator_core::import::ssh_config::{
    default_ssh_config_path, parse_ssh_config, strip_command_options,
};
use termigator_core::import::windows::{PUTTY_REG, local_moba, local_putty_reg, local_superputty};
use termigator_core::paths::Paths;
use termigator_core::vault::plaintext_password_count;

use crate::shell_qobj::{set_in, with_app};

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
        type ImportBackend = super::ImportBackendRust;

        /// File → Import → MobaXterm for the picked `.ini` / `.mxtsessions`; "" = this machine's
        /// MobaXterm (Windows: [`moba_local`](Self::moba_local)).
        #[qinvokable]
        fn import_moba(self: &ImportBackend, path: &QString) -> QString;

        /// This machine's MobaXterm sessions (Windows: registry or `MobaXterm.ini`):
        /// `{"where", "count"}`, or `{}` when there are none (ask for a file).
        #[qinvokable]
        fn moba_local(self: &ImportBackend) -> QString;

        /// SuperPuTTY's `Sessions.xml` on this machine (Windows), "" when there is none.
        #[qinvokable]
        fn superputty_path(self: &ImportBackend) -> QString;

        /// Sessions found in `~/.putty/sessions` (Windows: PuTTY's registry key; 0 = ask for a
        /// file); `putty_sessions_dir` names it.
        #[qinvokable]
        fn putty_local_count(self: &ImportBackend) -> i32;

        #[qinvokable]
        fn putty_sessions_dir(self: &ImportBackend) -> QString;

        /// File → Import → PuTTY: `~/.putty/sessions` (Windows: the registry) when `path` is
        /// empty, else a `.reg` / `.xml`.
        #[qinvokable]
        fn import_putty(self: &ImportBackend, path: &QString) -> QString;

        /// `~/.ssh/config` when it is a file, else "" (ask for one).
        #[qinvokable]
        fn ssh_config_path(self: &ImportBackend) -> QString;

        #[qinvokable]
        fn import_ssh_config(self: &ImportBackend, path: &QString) -> QString;

        /// Remmina's own folders exist (otherwise ask for `*.remmina` files).
        #[qinvokable]
        fn remmina_found(self: &ImportBackend) -> bool;

        /// `paths_json`: JSON array of files; `[]` reads Remmina's folders.
        #[qinvokable]
        fn import_remmina(self: &ImportBackend, paths_json: &QString) -> QString;

        /// The SSH entries that File → Export would write (0: nothing to export).
        #[qinvokable]
        fn ssh_export_count(self: &ImportBackend) -> i32;

        #[qinvokable]
        fn default_export_path(self: &ImportBackend) -> QString;

        #[qinvokable]
        fn export_ssh_config(self: &ImportBackend, path: &QString) -> QString;

        /// Saved passwords that are not encrypted (they would be in the zip as they are).
        #[qinvokable]
        fn plaintext_passwords(self: &ImportBackend) -> i32;

        /// `termigator-settings-<stamp>.zip` in the home folder.
        #[qinvokable]
        fn default_backup_path(self: &ImportBackend) -> QString;

        /// Writes the zip; save the config first (the zip holds what is on screen).
        #[qinvokable]
        fn backup(self: &ImportBackend, path: &QString) -> QString;
    }
}

#[derive(Default)]
pub struct ImportBackendRust;

/// What QML shows after an action.
#[derive(Debug, PartialEq)]
pub struct Outcome {
    pub text: String,
    pub changed: bool,
    pub warn: bool,
}

impl Outcome {
    fn info(text: String) -> Self {
        Outcome {
            text,
            changed: false,
            warn: false,
        }
    }

    fn warning(text: String) -> Self {
        Outcome {
            text,
            changed: false,
            warn: true,
        }
    }

    fn json(&self) -> QString {
        let j = json!({"text": self.text, "changed": self.changed, "warn": self.warn});
        QString::from(j.to_string().as_str())
    }
}

fn home() -> PathBuf {
    std::env::home_dir().unwrap_or_else(|| PathBuf::from("/"))
}

fn path_of(q: &QString) -> PathBuf {
    PathBuf::from(q.to_string())
}

fn stamp_now() -> Stamp {
    let now = QDateTime::current_date_time();
    let (d, t) = (now.date(), now.time());
    Stamp {
        year: d.year(),
        month: u32::try_from(d.month()).unwrap_or(1),
        day: u32::try_from(d.day()).unwrap_or(1),
        hour: u32::try_from(t.hour()).unwrap_or(0),
        minute: u32::try_from(t.minute()).unwrap_or(0),
    }
}

fn file_stamp(s: Stamp) -> String {
    format!(
        "{:04}{:02}{:02}-{:02}{:02}",
        s.year, s.month, s.day, s.hour, s.minute
    )
}

impl qobject::ImportBackend {
    fn import_moba(&self, path: &QString) -> QString {
        if path.is_empty()
            && let Some((_, text)) = local_moba()
        {
            return with_app(|app| moba_text(&mut app.cfg, &text, &local_user())).json();
        }
        with_app(|app| moba(&mut app.cfg, &path_of(path), &local_user())).json()
    }

    fn moba_local(&self) -> QString {
        let found = local_moba().map(|(place, text)| {
            let count = parse_moba_export(&text, &local_user()).0.len();
            json!({"where": place, "count": count})
        });
        QString::from(found.unwrap_or_else(|| json!({})).to_string().as_str())
    }

    fn superputty_path(&self) -> QString {
        QString::from(
            local_superputty()
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_default()
                .as_str(),
        )
    }

    fn putty_local_count(&self) -> i32 {
        let n = if cfg!(windows) {
            parse_putty_reg(&local_putty_reg(), &local_user()).len()
        } else {
            default_sessions_dir().map_or(0, |d| parse_putty_sessions_dir(&d, &local_user()).len())
        };
        i32::try_from(n).unwrap_or(i32::MAX)
    }

    fn putty_sessions_dir(&self) -> QString {
        if cfg!(windows) {
            return QString::from(format!("HKEY_CURRENT_USER\\{PUTTY_REG}").as_str());
        }
        QString::from(
            default_sessions_dir()
                .map(|d| d.to_string_lossy().into_owned())
                .unwrap_or_default()
                .as_str(),
        )
    }

    fn import_putty(&self, path: &QString) -> QString {
        let picked = path.to_string();
        if picked.is_empty() && cfg!(windows) {
            let rows = parse_putty_reg(&local_putty_reg(), &local_user());
            return with_app(|app| putty_rows(&mut app.cfg, rows, &local_user())).json();
        }
        let source = if picked.is_empty() {
            default_sessions_dir().unwrap_or_default()
        } else {
            PathBuf::from(picked)
        };
        with_app(|app| putty(&mut app.cfg, &source, &local_user())).json()
    }

    fn ssh_config_path(&self) -> QString {
        let p = default_ssh_config_path(&home());
        QString::from(if p.is_file() {
            p.to_string_lossy().into_owned()
        } else {
            String::new()
        })
    }

    fn import_ssh_config(&self, path: &QString) -> QString {
        with_app(|app| ssh_config(&mut app.cfg, &path_of(path), &home(), &local_user())).json()
    }

    fn remmina_found(&self) -> bool {
        !remmina_sources(&[], &home()).is_empty()
    }

    fn import_remmina(&self, paths_json: &QString) -> QString {
        let paths_json = paths_json.to_string();
        let parsed = serde_json::from_str::<Vec<String>>(&paths_json);
        let picked: Vec<PathBuf> = crate::qml_json::logged("import_remmina", &paths_json, parsed)
            .unwrap_or_default()
            .into_iter()
            .map(PathBuf::from)
            .collect();
        let sources = remmina_sources(&picked, &home());
        with_app(|app| remmina(&mut app.cfg, &sources, &home())).json()
    }

    fn ssh_export_count(&self) -> i32 {
        with_app(|app| {
            i32::try_from(export_ssh_config(seq(&app.cfg, "connections"), stamp_now()).exported)
                .unwrap_or(i32::MAX)
        })
    }

    fn default_export_path(&self) -> QString {
        QString::from(
            home()
                .join(".ssh")
                .join("termigator_hosts.conf")
                .to_string_lossy()
                .as_ref(),
        )
    }

    fn export_ssh_config(&self, path: &QString) -> QString {
        with_app(|app| ssh_export(&app.cfg, &path_of(path), &home(), stamp_now())).json()
    }

    fn plaintext_passwords(&self) -> i32 {
        with_app(|app| i32::try_from(plain_passwords(&app.cfg)).unwrap_or(i32::MAX))
    }

    fn default_backup_path(&self) -> QString {
        let name = format!("termigator-settings-{}.zip", file_stamp(stamp_now()));
        QString::from(home().join(name).to_string_lossy().as_ref())
    }

    fn backup(&self, path: &QString) -> QString {
        with_app(|app| backup_zip(&app.paths, &path_of(path))).json()
    }
}

// ---- Pure logic (unit-tested below) ------------------------------------------------------

fn seq<'a>(cfg: &'a Value, key: &str) -> &'a [Value] {
    match cfg.get(key) {
        Some(Value::Seq(items)) => items,
        _ => &[],
    }
}

fn plain_passwords(cfg: &Value) -> usize {
    plaintext_password_count(seq(cfg, "connections"))
        + plaintext_password_count(seq(cfg, "credentials"))
}

fn remmina_sources(picked: &[PathBuf], home: &Path) -> Vec<PathBuf> {
    if picked.is_empty() {
        let xdg = std::env::var("XDG_DATA_HOME").ok();
        remmina_dirs(xdg.as_deref(), home)
    } else {
        picked.to_vec()
    }
}

/// `merge_moba_connections` into `cfg.connections`; returns the number added.
fn merge_in(cfg: &mut Value, parsed: &[Value]) -> usize {
    let existing = seq(cfg, "connections").to_vec();
    let (merged, added) = merge_moba_connections(&existing, parsed, default_terminal_from_cfg(cfg));
    if added > 0 {
        set_in(cfg, &["connections"], Value::Seq(merged));
    }
    added
}

fn nothing_new(count: usize, what: &str) -> Outcome {
    Outcome::info(format!(
        "Nothing to import — all {count} {what} already exist (matched by host + user + port)."
    ))
}

fn dup_note(parsed: usize, added: usize) -> String {
    match parsed - added {
        0 => ".".to_owned(),
        n => format!(" ({n} duplicate(s) skipped)."),
    }
}

/// `_import_mobaxterm`.
pub fn moba(cfg: &mut Value, path: &Path, user: &str) -> Outcome {
    let text = match fs::read(path) {
        Ok(raw) => decode_moba_bytes(&raw).map_err(|_| "not valid UTF-16 / UTF-8 text".to_owned()),
        Err(e) => Err(e.to_string()),
    };
    match text {
        Ok(t) => moba_text(cfg, &t, user),
        Err(e) => Outcome::warning(format!("Could not read file:\n{e}")),
    }
}

/// [`moba`] for MobaXterm text already read (a file, or the registry on Windows).
pub fn moba_text(cfg: &mut Value, text: &str, user: &str) -> Outcome {
    let (parsed, mut other) = parse_moba_export(text, user);
    other.sort();
    let not_imported = other
        .iter()
        .map(|(kind, n)| format!("{n} {kind}"))
        .collect::<Vec<_>>()
        .join(", ");
    if parsed.is_empty() {
        let mut t = "No SSH, RDP or VNC bookmarks were found in this file".to_owned();
        if not_imported.is_empty() {
            t.push('.');
        } else {
            t.push_str(&format!(" (not supported yet: {not_imported})."));
        }
        t.push_str("\nExport your MobaXterm sessions (Settings → Export) and pick that .ini file.");
        return Outcome::info(t);
    }
    let added = merge_in(cfg, &parsed);
    if added == 0 {
        return nothing_new(parsed.len(), "parsed session(s)");
    }
    let mut t = format!(
        "Imported {added} new connection(s) from MobaXterm{}",
        dup_note(parsed.len(), added)
    );
    if !not_imported.is_empty() {
        t.push_str(&format!(
            "\nNot imported (session type not supported yet): {not_imported}."
        ));
    }
    t.push_str("\n\nReview imported entries — adjust SSH key paths if your keys live elsewhere.");
    Outcome {
        text: t,
        changed: true,
        warn: false,
    }
}

/// `_import_putty`: `source` is a folder of session files, a `.reg` or a SuperPuTTY `.xml`.
pub fn putty(cfg: &mut Value, source: &Path, user: &str) -> Outcome {
    let parsed = if source.is_dir() {
        parse_putty_sessions_dir(source, user)
    } else {
        parse_putty_export(source, user)
    };
    putty_rows(cfg, parsed, user)
}

/// [`putty`] for rows already parsed (Windows: PuTTY's registry key).
pub fn putty_rows(cfg: &mut Value, parsed: Vec<Value>, user: &str) -> Outcome {
    if parsed.is_empty() {
        return Outcome::info(
            "No SSH sessions found.\nSupported: ~/.putty/sessions files, a regedit export of\n\
             HKCU\\Software\\SimonTatham\\PuTTY\\Sessions (.reg), or SuperPuTTY sessions.xml."
                .to_owned(),
        );
    }
    let added = merge_in(cfg, &parsed);
    if added == 0 {
        return nothing_new(parsed.len(), "parsed session(s)");
    }
    Outcome {
        text: format!(
            "Imported {added} new SSH connection(s) from PuTTY/SuperPuTTY{}\n\nSessions without a \
             username got “{user}” — edit them if needed. PuTTY .ppk keys were skipped (convert \
             with puttygen).",
            dup_note(parsed.len(), added)
        ),
        changed: true,
        warn: false,
    }
}

/// `_import_ssh_config`.
pub fn ssh_config(cfg: &mut Value, path: &Path, home: &Path, user: &str) -> Outcome {
    let mut res = parse_ssh_config(path, home, user);
    // Commands from a file someone else wrote must not run on connect.
    let own = path == default_ssh_config_path(home);
    let dropped = if own {
        Vec::new()
    } else {
        strip_command_options(&mut res.connections)
    };
    if !res.unreadable.is_empty() && res.connections.is_empty() {
        return Outcome::warning(format!("Could not read:\n{}", res.unreadable.join("\n")));
    }
    if res.connections.is_empty() {
        return Outcome::info(format!(
            "No hosts found in {} (only wildcard Host / Match blocks).",
            path.display()
        ));
    }
    let added = merge_in(cfg, &res.connections);
    let mut notes = Vec::new();
    if res.skipped_match_blocks > 0 {
        notes.push(format!(
            "{} Match block(s) were not evaluated — ssh still applies them when connecting.",
            res.skipped_match_blocks
        ));
    }
    if !res.unreadable.is_empty() {
        notes.push(format!("Could not read: {}", res.unreadable.join(", ")));
    }
    if res.truncated {
        notes.push("The file is very large: only the first hosts were read.".to_owned());
    }
    if added == 0 {
        return nothing_new(res.connections.len(), "host(s)");
    }
    let mut t = format!(
        "Imported {added} host(s) from {} into the “ssh_config” group{}\n\nOptions set only for an \
         alias (Ciphers, ProxyJump, …) were kept as extra ssh arguments.",
        path.display(),
        dup_note(res.connections.len(), added)
    );
    if !notes.is_empty() {
        t.push_str(&format!("\n\n{}", notes.join("\n")));
    }
    if !dropped.is_empty() {
        t.push_str(&format!(
            "\n\nNot imported, because on connect they run programs on this computer or hand \
             the server your SSH agent or display (if you trust this file, add them back in the \
             entry: Extra ssh arguments, Forward agent, X11 forwarding):\n{}",
            dropped.join("\n")
        ));
    }
    Outcome {
        text: t,
        changed: true,
        warn: !dropped.is_empty(),
    }
}

/// `_import_remmina`: never the passwords.
pub fn remmina(cfg: &mut Value, sources: &[PathBuf], home: &Path) -> Outcome {
    let existing: Vec<Value> = seq(cfg, "connections")
        .iter()
        .filter(|c| matches!(c, Value::Map(_)))
        .cloned()
        .collect();
    let res = parse_remmina_profiles(sources, &existing, home);
    let mut skipped = res.skipped.clone();
    skipped.sort();
    let skipped_types = skipped
        .iter()
        .map(|(p, n)| format!("{n}× {p}"))
        .collect::<Vec<_>>()
        .join(", ");
    if res.connections.is_empty() {
        let places: Vec<_> = sources.iter().map(|s| s.display().to_string()).collect();
        let mut t = format!(
            "No SSH, SFTP, RDP or VNC profiles found in {}.",
            places.join(", ")
        );
        if !skipped_types.is_empty() {
            t.push_str(&format!("\nNot supported: {skipped_types}."));
        }
        return Outcome::info(t);
    }
    let added = merge_in(cfg, &res.connections);
    if added == 0 {
        return nothing_new(res.connections.len(), "profile(s)");
    }
    let mut notes = res.notes.clone();
    if !skipped_types.is_empty() {
        notes.push(format!("Not supported: {skipped_types}."));
    }
    if !res.unreadable.is_empty() {
        notes.push(format!("Could not read: {}", res.unreadable.join(", ")));
    }
    let mut t = format!(
        "Imported {added} profile(s) from Remmina{}\n\nPasswords are not imported — Remmina keeps \
         them encrypted; enter them once when connecting (or set a master password and save them).",
        dup_note(res.connections.len(), added)
    );
    if !notes.is_empty() {
        t.push_str(&format!("\n\n{}", notes.join("\n")));
    }
    Outcome {
        text: t,
        changed: true,
        warn: false,
    }
}

/// `_export_ssh_config` after the file was chosen: written with mode 0600.
pub fn ssh_export(cfg: &Value, path: &Path, home: &Path, now: Stamp) -> Outcome {
    let out = export_ssh_config(seq(cfg, "connections"), now);
    if out.exported == 0 {
        return Outcome::info("There are no SSH entries to export.".to_owned());
    }
    let write = || -> std::io::Result<()> {
        let mut f = termigator_core::platform::private_file(
            OpenOptions::new().write(true).create(true).truncate(true),
        )
        .open(path)?;
        // An existing file keeps its old mode on open.
        termigator_core::platform::set_private_file(path)?;
        f.write_all(out.text.as_bytes())
    };
    if let Err(e) = write() {
        return Outcome::warning(format!("Could not write the file:\n{e}"));
    }
    let shown = path.display().to_string();
    let home_s = home.to_string_lossy();
    let include = shown.replacen(home_s.as_ref(), "~", 1);
    let skipped = match out.skipped {
        0 => ".".to_owned(),
        1 => " (1 RDP / VNC / Telnet / serial entry skipped).".to_owned(),
        n => format!(" ({n} RDP / VNC / Telnet / serial entries skipped)."),
    };
    Outcome::info(format!(
        "Exported {} SSH host(s) to {shown}{skipped}\n\nTo use them with plain ssh / scp / rsync, \
         put this line at the top of ~/.ssh/config (before any Host block):\n\n    Include \
         {include}\n\nPasswords are never exported.",
        out.exported
    ))
}

/// `_backup_settings` after the file was chosen and the config saved.
pub fn backup_zip(paths: &Paths, target: &Path) -> Outcome {
    let note = format!(
        "Termigator settings backup.\n\nRestore: quit Termigator, unzip config.yaml and session.yaml into \
         the config\ndirectory ({} on the machine that made this backup; the installer's\nlauncher \
         sets TERMIGATOR_CONFIG_DIR), start Termigator. Encrypted passwords need the same master \
         password.\n",
        paths.config_dir.display()
    );
    let files = [
        ("config.yaml", paths.config_path.as_path()),
        ("session.yaml", paths.session_path.as_path()),
    ];
    match build_backup_zip(target, &files, &note) {
        Ok(names) => Outcome::info(format!(
            "Backup saved: {} → {}",
            names.join(", "),
            target.display()
        )),
        Err(e) => Outcome::warning(format!("Could not write the backup:\n{e}")),
    }
}

#[cfg(test)]
mod tests;
