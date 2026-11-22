//! Files panel: `FilesCore`, `FileSyncCore` and `FilesHelper`, exposed to QML as
//! `Termigator.FilesCore` etc.
//!
//! `FilesCore` is the remote listing as a list model plus every command of the panel: the panel state
//! ([`PanelState`]: target generation, folder, sort, ".." row, follow, drag payloads) decides, the
//! argv builders ([`RemoteOps`]) build, and each command runs on a worker thread ([`runner`])
//! whose result is queued back to the GUI thread and applied only if the ticket it was started
//! with still matches the panel. Transfers go to the panel's `TransfersCore`
//! (`transfers_qobj.rs`) by queue name. `FileSyncCore` is the Synchronize dialog's logic bound to
//! the host it was opened for; `FilesHelper` holds the stateless parts (the one-shot scp
//! of the host menu, local path helpers).
//!
//! Selections cross from QML as JSON arrays of row numbers. Remote names are bytes; QML only
//! ever sees their lossy display form, so everything that acts on a name goes back
//! through the row.

use std::collections::{BTreeMap, HashMap};
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::pin::Pin;

use std::time::{Duration, Instant, SystemTime};
use termigator_core::remote::panel::EditUpload;

use cxx_qt::{CxxQtType, Threading};
use cxx_qt_lib::{QByteArray, QHash, QHashPair_i32_QByteArray, QModelIndex, QString, QVariant};
use serde_json::{Value as J, json};
use termigator_core::config::doc::Value;
use termigator_core::config::model::{section, strip_space};
use termigator_core::credentials::apply_credential;
use termigator_core::proc::runner::{self, CancelHandle, Command, DEFAULT_MAX_OUTPUT};
use termigator_core::proto::ssh::{Env, ssh_password_env};
use termigator_core::remote::listing::{
    Argv, RemoteEntry, RemoteOps, display_name, edit_cache_path, host_key, human_size,
    is_auth_failure, mode_bits, parse_ls_output, remote_join, remote_parent, with_typed_password,
};
use termigator_core::remote::panel::{
    self, EditSession, Finish, Follow, PanelState, Row, SortColumn, Ticket, file_fingerprint,
    local_drop_paths, parse_drag,
};
use termigator_core::shell::split;

use crate::shell_qobj::with_app;
use crate::transfers_qobj::{QueueError, ScpRequest, queue_scp, reveal_conn};

#[cxx_qt::bridge]
pub mod qobject {
    unsafe extern "C++" {
        include!(<QtCore/QAbstractListModel>);
        type QAbstractListModel;
        include!("cxx-qt-lib/qhash.h");
        type QHash_i32_QByteArray = cxx_qt_lib::QHash<cxx_qt_lib::QHashPair_i32_QByteArray>;
        include!("cxx-qt-lib/qmodelindex.h");
        type QModelIndex = cxx_qt_lib::QModelIndex;
        include!("cxx-qt-lib/qvariant.h");
        type QVariant = cxx_qt_lib::QVariant;
        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;
    }

    #[auto_cxx_name]
    extern "RustQt" {
        #[qobject]
        #[base = QAbstractListModel]
        #[qml_element]
        #[qproperty(QString, cwd)]
        #[qproperty(QString, status)]
        #[qproperty(bool, busy)]
        #[qproperty(bool, has_target)]
        #[qproperty(QString, label)]
        #[qproperty(bool, show_hidden)]
        #[qproperty(i32, sort_column)]
        #[qproperty(bool, ascending)]
        #[qproperty(bool, editing)]
        /// The server asked for a password Files does not have (typed in the terminal, not saved):
        /// the panel offers to ask for it, and the QML retries while the terminal logs in.
        #[qproperty(bool, auth_required)]
        #[qproperty(QString, transfers_id)]
        type FilesCore = super::FilesCoreRust;
    }

    unsafe extern "RustQt" {
        #[inherit]
        #[cxx_name = "beginResetModel"]
        fn begin_reset_model(self: Pin<&mut FilesCore>);

        #[inherit]
        #[cxx_name = "endResetModel"]
        fn end_reset_model(self: Pin<&mut FilesCore>);
    }

    unsafe extern "RustQt" {
        #[qinvokable]
        #[cxx_override]
        fn data(self: &FilesCore, index: &QModelIndex, role: i32) -> QVariant;

        #[qinvokable]
        #[cxx_override]
        #[cxx_name = "roleNames"]
        fn role_names(self: &FilesCore) -> QHash_i32_QByteArray;

        #[qinvokable]
        #[cxx_override]
        #[cxx_name = "rowCount"]
        fn row_count(self: &FilesCore, parent: &QModelIndex) -> i32;
    }

    #[auto_cxx_name]
    extern "RustQt" {
        /// A warning box for the user.
        #[qsignal]
        fn warning(self: Pin<&mut FilesCore>, text: QString);

        /// The server copy of an edited file changed since it was opened; answer with
        /// `edit_overwrite(local)` or leave it (the next save asks again).
        #[qsignal]
        fn edit_conflict(self: Pin<&mut FilesCore>, question: QString, local: QString);

        /// An upload into another folder found these names there already; answer with
        /// `upload_checked`.
        #[qsignal]
        fn upload_clashes_found(self: Pin<&mut FilesCore>, question: QString);

        /// The config changed in a way that should be saved now (bookmarks, two panes).
        #[qsignal]
        fn persist_requested(self: Pin<&mut FilesCore>);

        /// The user went to another folder (path bar, up, a folder opened, a bookmark): the
        /// panel pauses "Follow terminal folder" until it is clicked or the terminal cd's.
        #[qsignal]
        fn navigated_by_user(self: Pin<&mut FilesCore>);

        /// Open a local file with the desktop's default application (Edit without `editor`).
        #[qsignal]
        fn open_url(self: Pin<&mut FilesCore>, path: QString);

        /// Binds the panel to an SSH entry (JSON, as stored), `""` = no SSH tab.
        #[qinvokable]
        fn set_target(self: Pin<&mut FilesCore>, entry_json: &QString, label: &QString);

        /// The panel is shown (lists a stale target) or hidden (does not).
        #[qinvokable]
        fn set_active(self: Pin<&mut FilesCore>, on: bool);

        #[qinvokable]
        fn refresh(self: Pin<&mut FilesCore>);

        /// A password typed for the current host (kept in memory for this session); lists again.
        #[qinvokable]
        fn submit_password(self: Pin<&mut FilesCore>, password: &QString);

        /// List `path` (text of the path bar; blank = home).
        #[qinvokable]
        fn navigate(self: Pin<&mut FilesCore>, path: &QString);

        #[qinvokable]
        fn go_up(self: Pin<&mut FilesCore>);

        /// Double click: "" when handled (entered a folder / went up), "download" for a file.
        #[qinvokable]
        fn activate(self: Pin<&mut FilesCore>, row: i32) -> QString;

        /// The active session reported a new directory (OSC 7).
        #[qinvokable]
        fn follow_directory(self: Pin<&mut FilesCore>, path: &QString, enabled: bool);

        #[qinvokable]
        fn set_hidden(self: Pin<&mut FilesCore>, on: bool);

        /// Header click: 0 name, 1 size, 2 modified, 3 permissions, 4 owner.
        #[qinvokable]
        fn sort_by(self: Pin<&mut FilesCore>, column: i32);

        /// `{kind: up|entry|none, name, path, isDir, isLink}` of a row.
        #[qinvokable]
        fn row_info(self: &FilesCore, row: i32) -> QString;

        /// Single file → `local` (scp; a link may point at a folder, so it goes with `-r`).
        #[qinvokable]
        fn download_entry(self: Pin<&mut FilesCore>, row: i32, local: &QString);

        /// The question about local names that exist in `target`, "" when none.
        #[qinvokable]
        fn download_clashes(self: &FilesCore, rows_json: &QString, target: &QString) -> QString;

        /// Rows → folder `target`; `skip_clashes`: leave out what exists there.
        #[qinvokable]
        fn download_into(
            self: Pin<&mut FilesCore>,
            rows_json: &QString,
            target: &QString,
            skip_clashes: bool,
        );

        /// The question about remote names that exist in the target folder, "" when none.
        #[qinvokable]
        fn upload_clashes(self: &FilesCore, paths_json: &QString, target_row: i32) -> QString;

        /// Local paths → the folder of `target_row` (-1: the current one).
        #[qinvokable]
        fn upload_paths(
            self: Pin<&mut FilesCore>,
            paths_json: &QString,
            target_row: i32,
            skip_clashes: bool,
        );

        /// After `upload_clashes_found`: start the checked upload, skipping the clashes or not.
        #[qinvokable]
        fn upload_checked(self: Pin<&mut FilesCore>, skip_clashes: bool);

        /// After `edit_conflict`: upload the save anyway.
        #[qinvokable]
        fn edit_overwrite(self: Pin<&mut FilesCore>, local: &QString);

        #[qinvokable]
        fn mkdir(self: Pin<&mut FilesCore>, name: &QString);

        #[qinvokable]
        fn rename(self: Pin<&mut FilesCore>, row: i32, new_name: &QString);

        #[qinvokable]
        fn delete_question(self: &FilesCore, rows_json: &QString) -> QString;

        #[qinvokable]
        fn delete_rows(self: Pin<&mut FilesCore>, rows_json: &QString);

        /// `{what, mode, anyDir}` for the permissions dialog.
        #[qinvokable]
        fn permissions_info(self: &FilesCore, rows_json: &QString) -> QString;

        #[qinvokable]
        fn chmod(self: Pin<&mut FilesCore>, rows_json: &QString, mode: i32, recursive: bool);

        /// Edit (auto-upload on save).
        #[qinvokable]
        fn edit_entry(self: Pin<&mut FilesCore>, row: i32);

        /// Copy remote path.
        #[qinvokable]
        fn copy_path(self: &FilesCore, row: i32) -> QString;

        /// `{marks: [...], canAdd, canRemove}` of the bookmark menu.
        #[qinvokable]
        fn bookmark_state(self: &FilesCore) -> QString;

        #[qinvokable]
        fn add_bookmark(self: Pin<&mut FilesCore>);

        #[qinvokable]
        fn remove_bookmark(self: Pin<&mut FilesCore>);

        /// The payload of rows being dragged out (for the local pane).
        #[qinvokable]
        fn drag_payload(self: &FilesCore, rows_json: &QString) -> QString;

        /// `{error}` or `{rows}` for a payload dropped on the local pane.
        #[qinvokable]
        fn drop_rows(self: &FilesCore, payload: &QString) -> QString;

        /// Local paths of dropped file-manager URLs (JSON array in and out; remote URLs: `[]`).
        #[qinvokable]
        fn local_drop_paths(self: &FilesCore, urls_json: &QString) -> QString;

        /// A transfer of this panel's queue ended (`TransfersCore.jobFinished`).
        #[qinvokable]
        fn transfer_finished(self: Pin<&mut FilesCore>, job_json: &QString);

        /// Looks for saves of the working copies (a timer calls it while `editing`).
        #[qinvokable]
        fn poll_edits(self: Pin<&mut FilesCore>);

        /// Optional columns shown: `["permissions", "owner"]`.
        #[qinvokable]
        fn shown_columns(self: &FilesCore) -> QString;

        #[qinvokable]
        fn set_column(self: Pin<&mut FilesCore>, key: &QString, on: bool);

        /// Remembered column widths: `{"size": 74, ...}` (`launch.files_column_widths`).
        #[qinvokable]
        fn column_widths(self: &FilesCore) -> QString;

        #[qinvokable]
        fn set_column_width(self: Pin<&mut FilesCore>, key: &QString, width: i32);

        /// The longest texts of a column (0 Name, 1 Size, 2 Modified, 3 Permissions, 4 Owner) among
        /// the listed entries, JSON `["…", …]`, longest first (at most 6): the candidates for the
        /// widest cell, which the view measures with its font (double click on a column's grip).
        #[qinvokable]
        fn column_texts(self: &FilesCore, column: i32) -> QString;

        /// "Reset column widths": forgets every remembered width.
        #[qinvokable]
        fn reset_column_widths(self: Pin<&mut FilesCore>);

        /// `launch.<key>` (files_two_pane_side / files_two_pane_tabs).
        #[qinvokable]
        fn two_pane(self: &FilesCore, key: &QString) -> bool;

        #[qinvokable]
        fn set_two_pane(self: Pin<&mut FilesCore>, key: &QString, on: bool);

        #[qinvokable]
        fn local_dir(self: &FilesCore) -> QString;

        #[qinvokable]
        fn set_local_dir(self: &FilesCore, path: &QString);

        /// `{conn, label, cwd}` for a Synchronize dialog, "" without a target.
        #[qinvokable]
        fn binding_json(self: &FilesCore) -> QString;

        /// Window or tab closing: ends the command in flight.
        #[qinvokable]
        fn shutdown(self: Pin<&mut FilesCore>);
    }

    impl cxx_qt::Threading for FilesCore {}
}

mod commands;
mod helpers;

pub use helpers::*;

// ---- FilesCore -----------------------------------------------------------------------------

/// What a command in flight is for.
enum Pending {
    /// A listing of this path.
    List(Vec<u8>),
    /// mkdir / rename / delete / chmod: refresh on success, else `"<prefix>: <stderr>"`.
    Op(&'static str),
    /// The listing of `dir` before `paths` are uploaded into it.
    UploadCheck { dir: Vec<u8>, paths: Vec<String> },
}

/// What a remote `stat` of an edited file is for.
enum SideStat {
    /// The server copy as the working copy starts from (after the download or a save).
    Base(PathBuf),
    /// A save about to go back: upload it only if the server copy is still the base.
    Check(EditUpload),
}

pub struct FilesCoreRust {
    cwd: QString,
    status: QString,
    busy: bool,
    has_target: bool,
    label: QString,
    show_hidden: bool,
    sort_column: i32,
    ascending: bool,
    editing: bool,
    auth_required: bool,
    /// Passwords typed into Files this session, by `user@host:port` (memory only, never saved).
    typed_passwords: HashMap<String, termigator_core::vault::Secret>,
    transfers_id: QString,
    panel: PanelState,
    rows: Vec<Row>,
    active: bool,
    /// An upload ended while a command ran: list again when it is over.
    refresh_after: bool,
    next_token: u64,
    pending: HashMap<u64, (Pending, Ticket, CancelHandle)>,
    next_tag: u64,
    /// The tickets of the uploads in flight (the refresh after one only if the panel still shows
    /// that host and folder).
    upload_tickets: HashMap<u64, Ticket>,
    /// An upload waiting for the user: (folder, local paths, names already there).
    upload_check: Option<(Vec<u8>, Vec<String>, Vec<String>)>,
    /// Remote `stat`s in flight, the server copy each working copy started from, the
    /// saves held back because that copy changed, and where each save goes (to stat it after).
    side_pending: HashMap<u64, SideStat>,
    edit_base: HashMap<PathBuf, String>,
    edit_conflicts: HashMap<PathBuf, EditUpload>,
    save_targets: HashMap<PathBuf, (Value, Vec<u8>)>,
    /// `purpose` of an Edit download → (host, remote path).
    edit_pending: HashMap<String, (Value, Vec<u8>)>,
    edits: HashMap<PathBuf, EditSession>,
    edit_seen: HashMap<PathBuf, (SystemTime, u64)>,
    clock: Instant,
    next_edit: u64,
}

impl Default for FilesCoreRust {
    fn default() -> Self {
        Self {
            cwd: QString::default(),
            status: qs(NO_TARGET_TEXT),
            busy: false,
            has_target: false,
            label: QString::default(),
            show_hidden: true,
            sort_column: 0,
            ascending: true,
            editing: false,
            auth_required: false,
            typed_passwords: HashMap::new(),
            transfers_id: QString::default(),
            panel: PanelState::new(),
            rows: Vec::new(),
            active: false,
            refresh_after: false,
            next_token: 0,
            pending: HashMap::new(),
            next_tag: 0,
            upload_tickets: HashMap::new(),
            upload_check: None,
            side_pending: HashMap::new(),
            edit_base: HashMap::new(),
            edit_conflicts: HashMap::new(),
            save_targets: HashMap::new(),
            edit_pending: HashMap::new(),
            edits: HashMap::new(),
            edit_seen: HashMap::new(),
            clock: Instant::now(),
            next_edit: 0,
        }
    }
}

impl qobject::FilesCore {
    fn data(&self, index: &QModelIndex, role: i32) -> QVariant {
        let Some(row) = usize::try_from(index.row())
            .ok()
            .and_then(|r| self.rows.get(r))
        else {
            return QVariant::default();
        };
        let text = |s: &str| QVariant::from(&qs(s));
        let flag = |b: bool| QVariant::from(&b);
        let Row::Entry(i) = *row else {
            return match role - USER_ROLE {
                0 => text("up"),
                1 => text(".."),
                2 => flag(true),
                3 => flag(false),
                4 => text("go-up"),
                9 => text("Parent directory"),
                5..=8 => text(""),
                _ => QVariant::default(),
            };
        };
        let Some(e) = self.panel.entries().get(i) else {
            return QVariant::default();
        };
        match role - USER_ROLE {
            0 => text("entry"),
            1 => text(&display_name(&e.name)),
            2 => flag(e.is_dir),
            3 => flag(e.is_link),
            4 => text(if e.is_link {
                "emblem-symbolic-link"
            } else if e.is_dir {
                "folder"
            } else {
                "text-x-generic"
            }),
            5 => text(&if e.is_dir {
                String::new()
            } else {
                human_size(e.size)
            }),
            6 => text(&e.mtime),
            7 => text(&e.mode),
            8 => text(&e.owner),
            9 => {
                let mut tip = format!(
                    "{}  {}",
                    e.mode,
                    display_name(&remote_join(self.panel.cwd(), &e.name))
                );
                if !e.link_target.is_empty() {
                    tip.push_str(&format!(" -> {}", display_name(&e.link_target)));
                }
                text(&tip)
            }
            _ => QVariant::default(),
        }
    }

    fn role_names(&self) -> QHash<QHashPair_i32_QByteArray> {
        let mut roles = QHash::<QHashPair_i32_QByteArray>::default();
        for (i, name) in (USER_ROLE..).zip(ROLES) {
            roles.insert(i, QByteArray::from(name));
        }
        roles
    }

    fn row_count(&self, _parent: &QModelIndex) -> i32 {
        i32::try_from(self.rows.len()).unwrap_or(i32::MAX)
    }

    // ---- target

    fn set_target(mut self: Pin<&mut Self>, entry_json: &QString, label: &QString) {
        let text = entry_json.to_string();
        let parsed = crate::qml_json::value("set_target", &text)
            .map(|j| Value::from_json(&j))
            .filter(|v| matches!(v, Value::Map(_)));
        let Some(entry) = parsed else {
            self.as_mut().rust_mut().panel.set_target(None, b"");
            self.as_mut().rust_mut().refresh_after = false;
            self.as_mut().set_has_target(false);
            self.as_mut().set_label(QString::default());
            self.as_mut().repopulate();
            self.as_mut().set_cwd(QString::default());
            self.say(NO_TARGET_TEXT);
            return;
        };
        // sshpass needs the password's presence.
        let conn = with_app(|app| apply_credential(&entry, credentials(&app.cfg)));
        let key = host_key(&conn);
        // A password typed for this host earlier in the session (the entry has none saved).
        let conn = match self.typed_passwords.get(&key) {
            Some(pw)
                if conn
                    .get("password")
                    .is_none_or(|v| !termigator_core::config::model::truthy(v)) =>
            {
                with_typed_password(&conn, &String::from_utf8_lossy(pw.expose()))
            }
            _ => conn,
        };
        let last = with_store(|s| panel::last_folder(s, &key));
        let reset = self
            .as_mut()
            .rust_mut()
            .panel
            .set_target(Some(&conn), &last);
        self.as_mut().set_has_target(true);
        self.as_mut().set_label(label.clone());
        if !reset {
            return; // same host — keep the directory the user navigated to
        }
        self.as_mut().repopulate();
        self.as_mut().set_cwd(QString::default());
        self.as_mut().say(&format!("{label}: ready."));
        if self.active {
            self.refresh();
        }
    }

    fn set_active(mut self: Pin<&mut Self>, on: bool) {
        self.as_mut().rust_mut().active = on;
        if on && self.panel.is_stale() && self.has_target && !self.panel.is_busy() {
            self.refresh();
        }
    }

    // ---- commands

    fn refresh(self: Pin<&mut Self>) {
        let path = self.panel.refresh_path();
        self.navigate_bytes(path);
    }

    fn submit_password(mut self: Pin<&mut Self>, password: &QString) {
        let password = password.to_string();
        let Some(conn) = self.panel.conn().cloned() else {
            return;
        };
        if password.is_empty() {
            return;
        }
        let key = host_key(&conn);
        let typed = with_typed_password(&conn, &password);
        let last = with_store(|s| panel::last_folder(s, &key));
        self.as_mut().rust_mut().typed_passwords.insert(
            key,
            termigator_core::vault::Secret::new(password.into_bytes()),
        );
        self.as_mut()
            .rust_mut()
            .panel
            .set_target(Some(&typed), &last);
        self.as_mut().set_auth_required(false);
        self.refresh();
    }

    fn navigate(mut self: Pin<&mut Self>, path: &QString) {
        let p = stripped(path);
        self.as_mut().navigated_by_user();
        self.navigate_bytes(if p.is_empty() {
            b".".to_vec()
        } else {
            p.into_bytes()
        });
    }

    fn go_up(mut self: Pin<&mut Self>) {
        let cwd = self.panel.cwd().to_vec();
        if !cwd.is_empty() && cwd != b"/" {
            self.as_mut().navigated_by_user();
            self.navigate_bytes(remote_parent(&cwd));
        }
    }

    fn activate(mut self: Pin<&mut Self>, row: i32) -> QString {
        let Ok(r) = usize::try_from(row) else {
            return QString::default();
        };
        match self.rows.get(r).copied() {
            Some(Row::Up) => self.go_up(),
            Some(Row::Entry(_)) => {
                let Some(e) = self.entry_at(r) else {
                    return QString::default();
                };
                if !(e.is_dir || e.is_link) {
                    return qs("download");
                }
                // Links: try to enter; if it is a file the cd fails and the status says so.
                let target = remote_join(self.panel.cwd(), &e.name);
                self.as_mut().navigated_by_user();
                self.navigate_bytes(target);
            }
            None => {}
        }
        QString::default()
    }

    fn follow_directory(mut self: Pin<&mut Self>, path: &QString, enabled: bool) {
        let p = path.to_string().into_bytes();
        let verdict = self.as_mut().rust_mut().panel.follow_directory(&p, enabled);
        if let Follow::Navigate(p) = verdict {
            self.navigate_bytes(p);
        }
    }

    fn set_hidden(mut self: Pin<&mut Self>, on: bool) {
        self.as_mut().rust_mut().panel.show_hidden = on;
        self.as_mut().set_show_hidden(on);
        self.repopulate();
    }

    fn sort_by(mut self: Pin<&mut Self>, column: i32) {
        self.as_mut().rust_mut().panel.sort_by(column_of(column));
        let (col, asc) = (self.panel.sort_column, self.panel.ascending);
        let index = [
            SortColumn::Name,
            SortColumn::Size,
            SortColumn::Modified,
            SortColumn::Mode,
            SortColumn::Owner,
        ]
        .iter()
        .position(|c| *c == col)
        .unwrap_or(0);
        self.as_mut()
            .set_sort_column(i32::try_from(index).unwrap_or(0));
        self.as_mut().set_ascending(asc);
        self.repopulate();
    }

    fn row_info(&self, row: i32) -> QString {
        let Some(r) = usize::try_from(row)
            .ok()
            .and_then(|r| self.rows.get(r).map(|x| (r, x)))
        else {
            return text_json(&json!({"kind": "none"}));
        };
        if matches!(r.1, Row::Up) {
            return text_json(&json!({"kind": "up"}));
        }
        match self.entry_at(r.0) {
            Some(e) => text_json(&json!({
                "kind": "entry",
                "name": display_name(&e.name),
                "path": display_name(&remote_join(self.panel.cwd(), &e.name)),
                "isDir": e.is_dir,
                "isLink": e.is_link,
            })),
            None => text_json(&json!({"kind": "none"})),
        }
    }

    // ---- transfers

    fn download_entry(self: Pin<&mut Self>, row: i32, local: &QString) {
        let Some(e) = usize::try_from(row)
            .ok()
            .and_then(|r| self.entry_at(r))
            .cloned()
        else {
            return;
        };
        if self.panel.conn().is_none() || local.is_empty() {
            return;
        }
        let remote = remote_join(self.panel.cwd(), &e.name);
        // A symlink may point at a directory — plain scp then fails with "not a regular
        // file"; -r is harmless for a file.
        let _ = self.enqueue(
            None,
            Transfer {
                upload: false,
                local: PathBuf::from(local.to_string()),
                remote,
                label: display_name(&e.name).into_bytes(),
                recursive: e.is_link,
                purpose: String::new(),
            },
        );
    }

    fn download_clashes(&self, rows_json: &QString, target: &QString) -> QString {
        let target = target.to_string();
        let names: Vec<String> = self
            .entries_of(&int_list("download_clashes", rows_json))
            .iter()
            .map(|e| display_name(&e.name))
            .filter(|n| std::fs::symlink_metadata(Path::new(&target).join(n)).is_ok())
            .collect();
        qs(&if names.is_empty() {
            String::new()
        } else {
            clash_question(&names, &target)
        })
    }

    fn download_into(
        mut self: Pin<&mut Self>,
        rows_json: &QString,
        target: &QString,
        skip_clashes: bool,
    ) {
        if self.panel.conn().is_none() {
            return;
        }
        let target_dir = PathBuf::from(target.to_string());
        let entries = self.entries_of(&int_list("download_into", rows_json));
        for e in entries {
            let name = display_name(&e.name);
            if skip_clashes && std::fs::symlink_metadata(target_dir.join(&name)).is_ok() {
                continue;
            }
            let remote = remote_join(self.panel.cwd(), &e.name);
            let label = format!("{name}{}", if e.is_dir { "/" } else { "" });
            let _ = self.as_mut().enqueue(
                None,
                Transfer {
                    upload: false,
                    local: target_dir.clone(),
                    remote,
                    label: label.into_bytes(),
                    recursive: e.is_dir || e.is_link,
                    purpose: String::new(),
                },
            );
        }
    }

    fn upload_clashes(&self, paths_json: &QString, target_row: i32) -> QString {
        let target = self.upload_dir(target_row);
        if target != self.panel.cwd() {
            return QString::default(); // only the listed folder is known (no extra round-trip)
        }
        let names: Vec<String> = string_list("upload_clashes", paths_json)
            .iter()
            .map(|p| base_name(Path::new(p)))
            .filter(|b| {
                self.panel
                    .entries()
                    .iter()
                    .any(|e| display_name(&e.name) == *b)
            })
            .collect();
        qs(&if names.is_empty() {
            String::new()
        } else {
            clash_question(&names, &display_name(&target))
        })
    }

    fn upload_paths(
        self: Pin<&mut Self>,
        paths_json: &QString,
        target_row: i32,
        skip_clashes: bool,
    ) {
        if self.panel.conn().is_none() || self.panel.cwd().is_empty() {
            return;
        }
        let target = self.upload_dir(target_row);
        let paths = string_list("upload_paths", paths_json);
        if target != self.panel.cwd() {
            // That folder is not listed here; look at it first, then ask about clashes.
            self.check_upload(target, paths);
            return;
        }
        let skip: Vec<String> = if skip_clashes {
            paths
                .iter()
                .map(|p| base_name(Path::new(p)))
                .filter(|b| {
                    self.panel
                        .entries()
                        .iter()
                        .any(|e| display_name(&e.name) == *b)
                })
                .collect()
        } else {
            Vec::new()
        };
        self.enqueue_uploads(&target, &paths, &skip);
    }

    fn upload_checked(mut self: Pin<&mut Self>, skip_clashes: bool) {
        let Some((dir, paths, clashes)) = self.as_mut().rust_mut().upload_check.take() else {
            return;
        };
        let skip = if skip_clashes { clashes } else { Vec::new() };
        self.enqueue_uploads(&dir, &paths, &skip);
    }

    // ---- remote file operations

    fn mkdir(self: Pin<&mut Self>, name: &QString) {
        let name = stripped(name);
        if self.panel.cwd().is_empty() || name.is_empty() || name.contains('/') {
            return;
        }
        let path = remote_join(self.panel.cwd(), name.as_bytes());
        self.operation("mkdir failed", move |o| o.mkdir_argv(&path));
    }

    fn rename(self: Pin<&mut Self>, row: i32, new_name: &QString) {
        let Some(e) = usize::try_from(row)
            .ok()
            .and_then(|r| self.entry_at(r))
            .cloned()
        else {
            return;
        };
        let new = stripped(new_name);
        if new.is_empty()
            || new.as_bytes() == e.name
            || new == display_name(&e.name)
            || new.contains('/')
        {
            return;
        }
        let old_path = remote_join(self.panel.cwd(), &e.name);
        let new_path = remote_join(self.panel.cwd(), new.as_bytes());
        self.operation("Rename failed", move |o| {
            o.rename_argv(&old_path, &new_path)
        });
    }

    fn delete_question(&self, rows_json: &QString) -> QString {
        let entries = self.entries_of(&int_list("delete_question", rows_json));
        let refs: Vec<&RemoteEntry> = entries.iter().collect();
        qs(&if refs.is_empty() {
            String::new()
        } else {
            delete_question_text(&refs)
        })
    }

    fn delete_rows(self: Pin<&mut Self>, rows_json: &QString) {
        let paths: Vec<Vec<u8>> = self
            .entries_of(&int_list("delete_rows", rows_json))
            .iter()
            .map(|e| remote_join(self.panel.cwd(), &e.name))
            .collect();
        if paths.is_empty() {
            return;
        }
        self.operation("Delete failed", move |o| o.delete_many_argv(&paths));
    }

    fn permissions_info(&self, rows_json: &QString) -> QString {
        let entries = self.entries_of(&int_list("permissions_info", rows_json));
        let Some(first) = entries.first() else {
            return qs("{}");
        };
        let what = if let [e] = entries.as_slice() {
            display_name(&e.name)
        } else {
            format!("{} items", entries.len())
        };
        text_json(&json!({
            "what": what,
            "mode": mode_bits(&first.mode),
            "anyDir": entries.iter().any(|e| e.is_dir),
        }))
    }

    fn chmod(self: Pin<&mut Self>, rows_json: &QString, mode: i32, recursive: bool) {
        let paths: Vec<Vec<u8>> = self
            .entries_of(&int_list("chmod", rows_json))
            .iter()
            .map(|e| remote_join(self.panel.cwd(), &e.name))
            .collect();
        if paths.is_empty() {
            return;
        }
        self.operation("chmod failed", move |o| {
            o.chmod_argv(&paths, i64::from(mode), recursive)
        });
    }

    fn copy_path(&self, row: i32) -> QString {
        usize::try_from(row)
            .ok()
            .and_then(|r| self.entry_at(r))
            .map_or_else(QString::default, |e| {
                qs(&display_name(&remote_join(self.panel.cwd(), &e.name)))
            })
    }

    // ---- bookmarks

    fn bookmark_state(&self) -> QString {
        let Some(key) = self.panel.host_key() else {
            return text_json(&json!({"marks": [], "canAdd": false, "canRemove": false}));
        };
        let marks = with_store(|s| panel::bookmarks(s, key));
        let cwd = self.panel.cwd();
        text_json(&json!({
            "marks": marks.iter().map(|m| display_name(m)).collect::<Vec<_>>(),
            "canAdd": !cwd.is_empty() && !marks.iter().any(|m| m == cwd),
            "canRemove": marks.iter().any(|m| m == cwd),
        }))
    }

    fn add_bookmark(mut self: Pin<&mut Self>) {
        let Some(key) = self.panel.host_key().map(str::to_owned) else {
            return;
        };
        let cwd = self.panel.cwd().to_vec();
        if with_store(|s| panel::add_bookmark(s, &key, &cwd)) {
            self.as_mut().persist_requested();
        }
    }

    fn remove_bookmark(mut self: Pin<&mut Self>) {
        let Some(key) = self.panel.host_key().map(str::to_owned) else {
            return;
        };
        let cwd = self.panel.cwd().to_vec();
        if with_store(|s| panel::remove_bookmark(s, &key, &cwd)) {
            self.as_mut().persist_requested();
        }
    }

    // ---- drag and drop

    fn drag_payload(&self, rows_json: &QString) -> QString {
        let entries = self.entries_of(&int_list("drag_payload", rows_json));
        let refs: Vec<&RemoteEntry> = entries.iter().collect();
        qs(&String::from_utf8_lossy(&self.panel.encode_drag(&refs)))
    }

    fn drop_rows(&self, payload: &QString) -> QString {
        let Some(p) = parse_drag(payload.to_string().as_bytes()) else {
            return text_json(&json!({"error": panel::DROP_STALE_MSG}));
        };
        match self.panel.resolve_drop(&p) {
            Err(msg) => text_json(&json!({"error": msg})),
            Ok(entries) => {
                let rows: Vec<usize> = self
                    .rows
                    .iter()
                    .enumerate()
                    .filter_map(|(r, row)| match row {
                        Row::Entry(i) => self
                            .panel
                            .entries()
                            .get(*i)
                            .filter(|e| entries.contains(e))
                            .map(|_| r),
                        Row::Up => None,
                    })
                    .collect();
                text_json(&json!({"rows": rows}))
            }
        }
    }

    fn local_drop_paths(&self, urls_json: &QString) -> QString {
        let urls = string_list("local_drop_paths", urls_json);
        let refs: Vec<&str> = urls.iter().map(String::as_str).collect();
        let paths: Vec<String> = local_drop_paths(&refs, |p| p.exists())
            .iter()
            .map(|p| p.to_string_lossy().into_owned())
            .collect();
        text_json(&json!(paths))
    }

    // ---- edit in place

    fn edit_entry(mut self: Pin<&mut Self>, row: i32) {
        let Some(e) = usize::try_from(row)
            .ok()
            .and_then(|r| self.entry_at(r))
            .cloned()
        else {
            return;
        };
        let Some(conn) = self.panel.conn().cloned() else {
            return;
        };
        if e.is_dir {
            return;
        }
        let remote = remote_join(self.panel.cwd(), &e.name);
        let local = edit_cache_path(&conn, &remote, None);
        // Only when missing: on Windows each call runs icacls on the GUI thread.
        if let Some(dir) = local.parent()
            && !dir.is_dir()
            && let Err(err) = termigator_core::platform::private_dir_create(dir, true)
        // copies of server files: private to the user
        {
            self.say(&format!("Could not create {}: {err}", dir.display()));
            return;
        }
        // Re-download; don't upload the download.
        self.as_mut().rust_mut().edits.remove(&local);
        let n = self.next_edit + 1;
        self.as_mut().rust_mut().next_edit = n;
        let purpose = format!("edit:{n}");
        self.as_mut()
            .rust_mut()
            .edit_pending
            .insert(purpose.clone(), (conn.clone(), remote.clone()));
        let label = format!("✎ {}", display_name(&e.name));
        let queued = self.as_mut().enqueue(
            Some(conn),
            Transfer {
                upload: false,
                local,
                remote,
                label: label.into_bytes(),
                recursive: false,
                purpose: purpose.clone(),
            },
        );
        if queued.is_err() {
            // No job will ever report this purpose.
            self.as_mut().rust_mut().edit_pending.remove(&purpose);
        }
    }

    /// A save of a working copy was not uploaded: let `poll_edits` send it again.
    fn edit_upload_failed(mut self: Pin<&mut Self>, local: &Path) {
        let now = u64::try_from(self.clock.elapsed().as_millis()).unwrap_or(u64::MAX);
        if let Some(s) = self.as_mut().rust_mut().edits.get_mut(local) {
            s.upload_failed(now);
        }
    }

    fn transfer_finished(mut self: Pin<&mut Self>, job_json: &QString) {
        let Some(j) = crate::qml_json::value("transfer_finished", &job_json.to_string()) else {
            return;
        };
        let text = |k: &str| j.get(k).and_then(J::as_str).unwrap_or_default().to_owned();
        let upload = j.get("upload").and_then(J::as_bool).unwrap_or(false);
        let (state, label, purpose) = (text("state"), text("plainLabel"), text("purpose"));
        if let Some((conn, remote)) = self.as_mut().rust_mut().edit_pending.remove(&purpose)
            && state == "done"
        {
            self.as_mut()
                .open_for_edit(PathBuf::from(text("local")), conn, remote);
        }
        let ticket = purpose
            .strip_prefix("upload:")
            .and_then(|t| t.parse::<u64>().ok())
            .and_then(|t| self.as_mut().rust_mut().upload_tickets.remove(&t));
        if state == "failed"
            && let Some(local) = purpose.strip_prefix("save:")
        {
            self.as_mut().edit_upload_failed(Path::new(local));
        }
        // Our own save is the server copy's new base.
        if state == "done"
            && let Some(local) = purpose.strip_prefix("save:").map(PathBuf::from)
            && let Some((conn, remote)) = self.as_mut().rust_mut().save_targets.remove(&local)
        {
            self.as_mut()
                .side_stat(conn, &remote, SideStat::Base(local));
        }
        if state == "failed" {
            let what = if upload { "Upload" } else { "Download" };
            self.as_mut()
                .say(&format!("{what} of {label} failed: {}", text("error")));
        } else if state == "done" && !upload {
            self.as_mut()
                .say(&format!("Downloaded {label} → {}", text("local")));
        }
        // Refresh after an upload only if the panel still shows that host + directory.
        if upload && state == "done" && ticket.is_some_and(|t| self.panel.accept(&t)) {
            if self.panel.is_busy() {
                self.as_mut().rust_mut().refresh_after = true;
            } else {
                self.refresh();
            }
        }
    }

    /// shortcut: the working copies are polled by size + mtime, not a
    /// file-system watcher; the 700 ms debounce and the fingerprint are core's.
    fn poll_edits(mut self: Pin<&mut Self>) {
        let now = u64::try_from(self.clock.elapsed().as_millis()).unwrap_or(u64::MAX);
        let paths: Vec<PathBuf> = self.edits.keys().cloned().collect();
        let mut uploads = Vec::new();
        for path in paths {
            let meta = std::fs::metadata(&path)
                .ok()
                .and_then(|m| Some((m.modified().ok()?, m.len())));
            let seen = self.edit_seen.get(&path).copied();
            if meta != seen
                && let Some(m) = meta
            {
                self.as_mut().rust_mut().edit_seen.insert(path.clone(), m);
                // The first look only records the state the download left.
                if seen.is_some()
                    && let Some(s) = self.as_mut().rust_mut().edits.get_mut(&path)
                {
                    s.changed(now);
                }
            }
            if let Some(s) = self.as_mut().rust_mut().edits.get_mut(&path)
                && let Some(up) = s.poll(now, file_fingerprint)
            {
                uploads.push(up);
            }
        }
        for up in uploads {
            let (conn, remote) = (up.conn.clone(), up.remote.clone());
            self.as_mut().side_stat(conn, &remote, SideStat::Check(up));
        }
    }

    fn edit_overwrite(mut self: Pin<&mut Self>, local: &QString) {
        let local = PathBuf::from(local.to_string());
        if let Some(up) = self.as_mut().rust_mut().edit_conflicts.remove(&local) {
            self.enqueue_save(up);
        }
    }

    // ---- remembered settings

    fn shown_columns(&self) -> QString {
        let shown: Vec<String> = match launch().get("files_columns") {
            Some(Value::Seq(items)) => items
                .iter()
                .map(termigator_core::config::model::value_str)
                .collect(),
            _ => Vec::new(),
        };
        text_json(&json!(shown))
    }

    fn set_column(self: Pin<&mut Self>, key: &QString, on: bool) {
        let key = key.to_string();
        with_app(|app| {
            let launch = slot(&mut app.cfg, "launch");
            let mut shown: Vec<Value> = match launch.get("files_columns") {
                Some(Value::Seq(items)) => items
                    .iter()
                    .filter(|v| !matches!(v, Value::Str(s) if *s == key))
                    .cloned()
                    .collect(),
                _ => Vec::new(),
            };
            if on {
                shown.push(Value::Str(key));
            }
            set_in(launch, "files_columns", Value::Seq(shown));
        });
    }

    fn column_widths(&self) -> QString {
        let widths: serde_json::Map<String, J> = match launch().get("files_column_widths") {
            Some(Value::Map(pairs)) => pairs
                .iter()
                .filter_map(|(k, v)| match (k, v) {
                    (Value::Str(k), Value::Int(w)) => Some((k.clone(), json!(w))),
                    _ => None,
                })
                .collect(),
            _ => Default::default(),
        };
        text_json(&J::Object(widths))
    }

    fn set_column_width(mut self: Pin<&mut Self>, key: &QString, width: i32) {
        let key = key.to_string();
        with_app(|app| {
            let widths = slot(slot(&mut app.cfg, "launch"), "files_column_widths");
            set_in(widths, &key, Value::Int(i64::from(width)));
        });
        self.as_mut().persist_requested();
    }

    fn column_texts(&self, column: i32) -> QString {
        let mut texts: Vec<String> = self
            .panel
            .entries()
            .iter()
            .map(|e| match column {
                0 => display_name(&e.name),
                1 if e.is_dir => String::new(),
                1 => human_size(e.size),
                2 => e.mtime.clone(),
                3 => e.mode.clone(),
                4 => e.owner.clone(),
                _ => String::new(),
            })
            .collect();
        texts.sort_by_key(|t| std::cmp::Reverse(t.chars().count()));
        texts.dedup();
        texts.truncate(6);
        text_json(&json!(texts))
    }

    fn reset_column_widths(mut self: Pin<&mut Self>) {
        with_app(|app| {
            set_in(
                slot(&mut app.cfg, "launch"),
                "files_column_widths",
                Value::Map(Vec::new()),
            );
        });
        self.as_mut().persist_requested();
    }

    fn two_pane(&self, key: &QString) -> bool {
        let key = key.to_string();
        launch()
            .get(&key)
            .is_some_and(termigator_core::config::model::truthy)
    }

    fn set_two_pane(mut self: Pin<&mut Self>, key: &QString, on: bool) {
        let key = key.to_string();
        let changed = with_app(|app| {
            let launch = slot(&mut app.cfg, "launch");
            let was = launch
                .get(&key)
                .is_some_and(termigator_core::config::model::truthy);
            set_in(launch, &key, Value::Bool(on));
            was != on
        });
        if changed {
            self.as_mut().persist_requested();
        }
    }

    fn local_dir(&self) -> QString {
        qs(&launch()
            .get("files_local_dir")
            .map(termigator_core::config::model::value_str)
            .unwrap_or_default())
    }

    fn set_local_dir(&self, path: &QString) {
        let path = path.to_string();
        with_app(|app| {
            set_in(
                slot(&mut app.cfg, "launch"),
                "files_local_dir",
                Value::Str(path),
            )
        });
    }

    fn binding_json(&self) -> QString {
        match self.panel.binding() {
            Some(b) => text_json(&json!({
                "conn": b.conn.to_json(),
                "label": self.host_label(),
                "cwd": display_name(self.panel.cwd()),
            })),
            None => QString::default(),
        }
    }

    fn shutdown(mut self: Pin<&mut Self>) {
        for (_, _, cancel) in self.pending.values() {
            cancel.cancel();
        }
        self.as_mut().rust_mut().pending.clear();
    }
}

/// What [`qobject::FilesCore::enqueue`] queues.
struct Transfer {
    upload: bool,
    local: PathBuf,
    remote: Vec<u8>,
    label: Vec<u8>,
    recursive: bool,
    purpose: String,
}

#[cfg(test)]
mod tests;
