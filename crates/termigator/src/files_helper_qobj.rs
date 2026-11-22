//! Stateless parts of the Files area.

use std::path::Path;

use cxx_qt_lib::{QString, QUrl};
use serde_json::{Value as J, json};
use termigator_core::config::doc::Value;
use termigator_core::config::model::{
    PROTOCOL_SSH, TERMINAL_EMBEDDED, connection_protocol, strip_space, terminal_mode_for_entry,
};
use termigator_core::credentials::apply_credential;
use termigator_core::hosttree::{entry_label, touch_recent};
use termigator_core::proto::scp::build_scp_command;
use termigator_core::proto::ssh::{expand_user, ssh_password_env};
use termigator_core::remote::listing::human_size;
use termigator_core::remote::local_ops;

use crate::files_qobj::{credentials, qs, text_json};
use crate::shell_qobj::with_app;
use crate::transfers_qobj::reveal_conn;

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
        type FilesHelper = super::FilesHelperRust;

        /// The one-shot scp of the host menu:
        /// `{cmd, title, mode, env}` for `openInner`, `{error}`, or `{locked, text}`.
        #[qinvokable]
        fn scp_plan(
            self: &FilesHelper,
            entry_json: &QString,
            local: &QString,
            remote: &QString,
            upload: bool,
        ) -> QString;

        /// The row label of an entry (JSON), as in the host list.
        #[qinvokable]
        fn entry_label(self: &FilesHelper, entry_json: &QString) -> QString;

        /// Puts the entry first in "Recent"; not saved.
        #[qinvokable]
        fn touch_recent(self: &FilesHelper, entry_json: &QString);

        /// An entry (JSON) that is an SSH connection.
        #[qinvokable]
        fn is_ssh(self: &FilesHelper, entry_json: &QString) -> bool;

        /// "Save file from" suggestion: the remote file's name, else `download`.
        #[qinvokable]
        fn suggested_name(self: &FilesHelper, remote: &QString) -> QString;

        #[qinvokable]
        fn home_dir(self: &FilesHelper) -> QString;

        /// `os.path.expanduser`.
        #[qinvokable]
        fn expand_user(self: &FilesHelper, path: &QString) -> QString;

        #[qinvokable]
        fn is_dir(self: &FilesHelper, path: &QString) -> bool;

        /// `file:` URL of a local path (Windows: `C:\\dir` -> `file:///C:/dir`), percent-encoded.
        #[qinvokable]
        fn file_url(self: &FilesHelper, path: &QString) -> QString;

        /// The local path of a `file:` URL in this system's spelling; "" for other URLs.
        #[qinvokable]
        fn local_path(self: &FilesHelper, url: &QString) -> QString;

        /// The parent of a local folder (`/` stays).
        #[qinvokable]
        fn parent_dir(self: &FilesHelper, path: &QString) -> QString;

        /// "N.N K" sizes as in the remote list.
        #[qinvokable]
        fn human_size(self: &FilesHelper, bytes: f64) -> QString;

        /// New folder `name` in the local folder `parent`; "" on success, else the reason.
        #[qinvokable]
        fn local_mkdir(self: &FilesHelper, parent: &QString, name: &QString) -> QString;

        /// Renames a local entry inside its folder; "" on success, else the reason.
        #[qinvokable]
        fn local_rename(self: &FilesHelper, path: &QString, new_name: &QString) -> QString;

        /// Deletes local entries (a JSON list of paths); "" on success, else the reasons.
        #[qinvokable]
        fn local_delete(self: &FilesHelper, paths_json: &QString) -> QString;
    }
}

#[derive(Default)]
pub struct FilesHelperRust;

/// Qt hands out `C:/dir` on Windows; the rest of the app (and the user) spells it `C:\\dir`.
fn native_separators(p: &str) -> String {
    if cfg!(windows) {
        p.replace('/', "\\")
    } else {
        p.to_owned()
    }
}

/// Appended to the scp line in an external terminal, which would close before the output
/// can be read.
const PAUSE: &str = "; printf '\\n[scp finished — press Enter to close]'; read -r _";

fn parse(invokable: &str, json: &QString) -> Value {
    crate::qml_json::value(invokable, &json.to_string())
        .map(|j| Value::from_json(&j))
        .unwrap_or(Value::Null)
}

pub fn scp_plan_json(cfg: &Value, entry: &Value, local: &str, remote: &str, upload: bool) -> J {
    if !matches!(entry, Value::Map(_)) || connection_protocol(entry) != PROTOCOL_SSH {
        return json!({});
    }
    let conn = apply_credential(entry, credentials(cfg));
    let mut cmd = match build_scp_command(&conn, local, remote.trim(), upload) {
        Ok(c) => c,
        Err(e) => return json!({"error": e}),
    };
    let mode = terminal_mode_for_entry(Some(entry), cfg);
    if mode != TERMINAL_EMBEDDED {
        cmd.push_str(PAUSE);
    }
    let Some(revealed) = reveal_conn(&conn) else {
        return json!({
            "locked": true,
            "text": "Cannot decrypt the saved password:\nThe vault is locked.",
        });
    };
    let env: serde_json::Map<String, J> = ssh_password_env(&revealed)
        .unwrap_or_default()
        .into_iter()
        // shortcut: the password leaves zeroizing memory for QML here, upgrade with panes env_json.
        .map(|(k, v)| {
            (
                k,
                J::String(String::from_utf8_lossy(v.expose()).into_owned()),
            )
        })
        .collect();
    json!({
        "cmd": cmd,
        "title": format!("scp {} {}", if upload { "→" } else { "←" }, entry_label(entry)),
        "mode": mode,
        "env": env,
    })
}

impl qobject::FilesHelper {
    fn scp_plan(
        &self,
        entry_json: &QString,
        local: &QString,
        remote: &QString,
        upload: bool,
    ) -> QString {
        let entry = parse("scp_plan", entry_json);
        let plan = with_app(|app| {
            scp_plan_json(
                &app.cfg,
                &entry,
                &local.to_string(),
                &remote.to_string(),
                upload,
            )
        });
        text_json(&plan)
    }

    fn entry_label(&self, entry_json: &QString) -> QString {
        qs(&entry_label(&parse("entry_label", entry_json)))
    }

    fn touch_recent(&self, entry_json: &QString) {
        let entry = parse("touch_recent", entry_json);
        with_app(|app| touch_recent(&mut app.cfg, &entry));
    }

    fn is_ssh(&self, entry_json: &QString) -> bool {
        let entry = parse("is_ssh", entry_json);
        matches!(entry, Value::Map(_)) && connection_protocol(&entry) == PROTOCOL_SSH
    }

    fn suggested_name(&self, remote: &QString) -> QString {
        let text = remote.to_string();
        let trimmed = strip_space(&text).trim_end_matches('/');
        let base = trimmed.rsplit('/').next().unwrap_or_default();
        qs(if base.is_empty() { "download" } else { base })
    }

    fn home_dir(&self) -> QString {
        qs(&std::env::home_dir()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_else(|| "/".to_owned()))
    }

    fn expand_user(&self, path: &QString) -> QString {
        qs(&expand_user(
            &path.to_string(),
            std::env::var("HOME").ok().as_deref(),
        ))
    }

    fn is_dir(&self, path: &QString) -> bool {
        Path::new(&path.to_string()).is_dir()
    }

    fn file_url(&self, path: &QString) -> QString {
        let url = QUrl::from_local_file(path);
        qs(&String::from_utf8_lossy(url.to_encoded().as_slice()))
    }

    fn local_path(&self, url: &QString) -> QString {
        let p = QUrl::from(url).to_local_file().map(|p| p.to_string());
        qs(&native_separators(&p.unwrap_or_default()))
    }

    fn parent_dir(&self, path: &QString) -> QString {
        let text = path.to_string();
        let trimmed = text.trim_end_matches('/');
        // A root has no parent: `/` and, on Windows, `C:\` stay where they are.
        let root = if trimmed.is_empty() { "/" } else { trimmed };
        qs(Path::new(trimmed)
            .parent()
            .and_then(Path::to_str)
            .filter(|p| !p.is_empty())
            .unwrap_or(root))
    }

    fn human_size(&self, bytes: f64) -> QString {
        qs(&human_size(bytes as i64))
    }

    fn local_mkdir(&self, parent: &QString, name: &QString) -> QString {
        let result = local_ops::mkdir(Path::new(&parent.to_string()), &name.to_string());
        qs(&result.err().unwrap_or_default())
    }

    fn local_rename(&self, path: &QString, new_name: &QString) -> QString {
        let result = local_ops::rename(Path::new(&path.to_string()), &new_name.to_string());
        qs(&result.err().unwrap_or_default())
    }

    fn local_delete(&self, paths_json: &QString) -> QString {
        let Some(J::Array(items)) = crate::qml_json::value("local_delete", &paths_json.to_string())
        else {
            return qs("");
        };
        let paths: Vec<std::path::PathBuf> = items
            .iter()
            .filter_map(J::as_str)
            .map(std::path::PathBuf::from)
            .collect();
        qs(&local_ops::delete(&paths).err().unwrap_or_default())
    }
}
