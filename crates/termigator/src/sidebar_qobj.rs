//! Host list: `HostListModel`, exposed to QML as `Termigator.HostListModel`.
//!
//! The flattened tree (core `hosttree`) as a list model for `qml/sidebar/HostsPanel.qml`,
//! plus the parts of the sidebar that need the command builders: the
//! entry error shown in the row, add / edit validation, Copy command, Test connection (the probe
//! runs on a worker thread) and the network tools. The expanded-folder set is kept here, in
//! memory only; the config is the shared one (`shell_qobj::with_app`) and QML saves it
//! with `controller.persist()`, whose `configChanged` brings the list back here (`refresh`).
//!
//! Mutations return an outcome as JSON: `changed` (QML persists), `select` (`[kind, index]` to
//! reveal afterwards), `info` / `warning` (a box), `notice` (the status bar), `copy` (clipboard).

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::pin::Pin;
use std::time::Duration;

use cxx_qt::{CxxQtType, Threading};
use cxx_qt_lib::{
    QByteArray, QHash, QHashPair_i32_QByteArray, QList, QModelIndex, QString, QStringList, QUrl,
    QVariant,
};
use serde_json::json;
use termigator_core::bulk::{ICON_CHOICES, apply_bulk};
use termigator_core::config::doc::Value;
use termigator_core::config::environments;
use termigator_core::config::groups::{group_path_ancestors, normalize_group_path};
use termigator_core::config::model::{
    PROTOCOL_RDP, PROTOCOL_SERIAL, PROTOCOL_SSH, PROTOCOL_TELNET, PROTOCOL_VNC,
    connection_protocol, section, strip_space, terminal_mode_for_entry, truthy, value_int,
    value_str,
};
use termigator_core::connectivity::{DEFAULT_TIMEOUT, ProbeReport, probe_connection};
use termigator_core::credentials::apply_credential;
use termigator_core::hosttree::{
    self, EntryKind, EntryRef, Hooks, Icon, Row, RowKind, TreeState, Zone, entry_label, group_of,
};
use termigator_core::proc::tunnel::free_local_port;
use termigator_core::proto::ssh::{Env, quote_repr};
use termigator_core::proto::{gateway, nettools, rdp, serial, ssh, telnet, vnc};

use crate::shell_qobj::{self, with_app};

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
        include!("cxx-qt-lib/qstringlist.h");
        type QStringList = cxx_qt_lib::QStringList;
        include!("cxx-qt-lib/qlist.h");
        type QList_i32 = cxx_qt_lib::QList<i32>;
    }

    extern "RustQt" {
        #[qobject]
        #[base = QAbstractListModel]
        #[qml_element]
        #[qproperty(QString, status)]
        #[qproperty(QString, hint)]
        type HostListModel = super::HostListModelRust;
    }

    unsafe extern "RustQt" {
        #[inherit]
        #[cxx_name = "beginResetModel"]
        fn begin_reset_model(self: Pin<&mut HostListModel>);

        #[inherit]
        #[cxx_name = "endResetModel"]
        fn end_reset_model(self: Pin<&mut HostListModel>);
    }

    unsafe extern "RustQt" {
        #[qinvokable]
        #[cxx_override]
        fn data(self: &HostListModel, index: &QModelIndex, role: i32) -> QVariant;

        #[qinvokable]
        #[cxx_override]
        #[cxx_name = "roleNames"]
        fn role_names(self: &HostListModel) -> QHash_i32_QByteArray;

        #[qinvokable]
        #[cxx_override]
        #[cxx_name = "rowCount"]
        fn row_count(self: &HostListModel, parent: &QModelIndex) -> i32;
    }

    #[auto_cxx_name]
    extern "RustQt" {
        /// The probe started by `test_connection` with this token has finished.
        #[qsignal]
        fn probe_finished(self: Pin<&mut HostListModel>, token: i32, report_json: QString);

        /// Rebuilds the rows from the config.
        #[qinvokable]
        fn refresh(self: Pin<&mut HostListModel>);

        /// The search text changed.
        #[qinvokable]
        fn set_filter(self: Pin<&mut HostListModel>, text: &QString);

        /// Expands / collapses a folder or Recent; returns the rows of `keep` after the rebuild.
        #[qinvokable]
        fn set_expanded(
            self: Pin<&mut HostListModel>,
            row: i32,
            expanded: bool,
            keep: &QList_i32,
        ) -> QList_i32;

        /// `{kind, index, path, inRecent, expanded, protocol, group, label}` of a row, or "".
        #[qinvokable]
        fn row_json(self: &HostListModel, row: i32) -> QString;

        /// The entry (`connection` / `favorite`, index) as JSON (`Value::to_json`), or "".
        #[qinvokable]
        fn entry_json(self: &HostListModel, kind: &QString, index: i32) -> QString;

        /// Config indices of the selected connection rows, each once, in config order.
        #[qinvokable]
        fn selected_connections(self: &HostListModel, rows: &QList_i32) -> QList_i32;

        /// -1 no drop here, 0 above, 1 on, 2 below (`y_fraction` 0 = top of the row).
        #[qinvokable]
        fn drop_zone(self: &HostListModel, row: i32, y_fraction: f64) -> i32;

        /// Drops the dragged rows (`current` plus `selected`) at `target` (-1 = empty space).
        #[qinvokable]
        fn drop_rows(
            self: Pin<&mut HostListModel>,
            current: i32,
            selected: &QList_i32,
            target: i32,
            y_fraction: f64,
        ) -> QString;

        /// Expands what hides the entry, rebuilds, and returns its row (-1 when not shown).
        #[qinvokable]
        fn reveal(self: Pin<&mut HostListModel>, kind: &QString, index: i32) -> i32;

        /// Every folder path in play, sorted.
        #[qinvokable]
        fn group_names(self: &HostListModel) -> QStringList;

        #[qinvokable]
        fn delete_group_question(self: &HostListModel, path: &QString) -> QString;

        #[qinvokable]
        fn dissolve_question(self: &HostListModel, path: &QString) -> QString;

        /// Adds (`entry_kind` "") or replaces an entry with the editor's result (`kind`: the
        /// editor, ssh|rdp|vnc|telnet|serial|favorite).
        #[qinvokable]
        fn store_entry(
            self: &HostListModel,
            kind: &QString,
            entry_kind: &QString,
            index: i32,
            data_json: &QString,
        ) -> QString;

        #[qinvokable]
        fn duplicate(self: &HostListModel, kind: &QString, index: i32) -> QString;

        #[qinvokable]
        fn move_by(self: &HostListModel, kind: &QString, index: i32, delta: i32) -> QString;

        #[qinvokable]
        fn move_to_group(
            self: Pin<&mut HostListModel>,
            kind: &QString,
            index: i32,
            name: &QString,
        ) -> QString;

        #[qinvokable]
        fn remove_entry(self: &HostListModel, kind: &QString, index: i32) -> QString;

        /// `indices`: config indices of connections.
        #[qinvokable]
        fn bulk_delete(self: &HostListModel, indices: &QList_i32) -> QString;

        /// `changes_json`: the `apply_bulk` changes (`{"icon": ...}`, `{"group": ...}`).
        #[qinvokable]
        fn bulk_apply(
            self: Pin<&mut HostListModel>,
            indices: &QList_i32,
            changes_json: &QString,
        ) -> QString;

        #[qinvokable]
        fn add_group(self: Pin<&mut HostListModel>, parent: &QString, name: &QString) -> QString;

        #[qinvokable]
        fn rename_group(self: Pin<&mut HostListModel>, old: &QString, path: &QString) -> QString;

        #[qinvokable]
        fn ungroup(self: Pin<&mut HostListModel>, path: &QString) -> QString;

        /// `{own, inherited, from}` — the folder's own environment, the one it shows
        /// (own or a parent's) and the folder that one comes from.
        #[qinvokable]
        fn group_environment(self: &HostListModel, path: &QString) -> QString;

        /// `[{id, name, color}, …]` of the configured environment categories.
        #[qinvokable]
        fn environments_json(self: &HostListModel) -> QString;

        /// `{own, inherited, from}` for an entry (its own environment or its folder's).
        #[qinvokable]
        fn entry_environment(self: &HostListModel, kind: &QString, index: i32) -> QString;

        /// Sets the entry's own environment ("" clears it and lets the folder's apply).
        #[qinvokable]
        fn set_entry_environment(
            self: Pin<&mut HostListModel>,
            kind: &QString,
            index: i32,
            env: &QString,
        ) -> QString;

        /// Marks folder `path` with an environment ("" clears it); everything inside
        /// without its own inherits it.
        #[qinvokable]
        fn set_group_environment(
            self: Pin<&mut HostListModel>,
            path: &QString,
            env: &QString,
        ) -> QString;

        /// The entry as `entry_json`, plus the environment it inherits from its folder — what a
        /// session opened from the list carries (its tab and terminal show it).
        #[qinvokable]
        fn open_json(self: &HostListModel, kind: &QString, index: i32) -> QString;

        #[qinvokable]
        fn delete_group(self: Pin<&mut HostListModel>, path: &QString) -> QString;

        /// Copy SSH / RDP / … command of connection `index`: `copy` + `notice`, or `warning`.
        #[qinvokable]
        fn copy_command(self: &HostListModel, index: i32) -> QString;

        /// `user@host[:port]` of a connection (the credential's user included), copied.
        #[qinvokable]
        fn copy_address(self: &HostListModel, index: i32) -> QString;

        /// `{cmd, title}` of a network tool for connection `index`, or `{warning}`.
        #[qinvokable]
        fn nettool(self: &HostListModel, index: i32, tool: &QString) -> QString;

        /// `[[key, label], …]` of the network tools.
        #[qinvokable]
        fn tools_json(self: &HostListModel) -> QString;

        /// `[[label, icon name, resolved theme icon], …]` of the Set icon submenu.
        #[qinvokable]
        fn icon_choices_json(self: &HostListModel) -> QString;

        /// Starts the reachability probe of a connection (JSON) off the GUI thread; the result
        /// arrives as `probe_finished(token, …)`. Returns the token.
        #[qinvokable]
        fn test_connection(self: Pin<&mut HostListModel>, entry_json: &QString) -> i32;

        #[qinvokable]
        fn probe_title(self: &HostListModel, entry_json: &QString) -> QString;

        /// `{"<kind>": "<why it cannot be opened here>"}` for the New connection menus; kinds
        /// that can be opened are left out (Linux: none; Windows: serial, VNC / Telnet without a client).
        #[qinvokable]
        fn kind_support(self: &HostListModel) -> QString;

        /// The shell button's menu, `[{id, label, available}]` (Linux: /etc/shells; Windows:
        /// cmd, PowerShell, WSL, the missing ones greyed out).
        #[qinvokable]
        fn shells(self: &HostListModel) -> QString;

        /// The theme icon of a context-menu entry, by its text.
        #[qinvokable]
        fn menu_icon(self: &HostListModel, text: &QString) -> QString;
    }

    impl cxx_qt::Threading for HostListModel {}
}

// ---- Rows for QML ------------------------------------------------------------------------

const ROLES: [&str; 17] = [
    "depth",
    "kind",
    "label",
    "subtitle",
    "badge",
    "iconName",
    "distroColor",
    "distroGlyph",
    "distroLogo",
    "error",
    "expanded",
    "hasChildren",
    "tooltip",
    "draggable",
    "environment",
    "environmentName",
    "environmentColor",
];
/// `Qt::UserRole`.
const USER_ROLE: i32 = 0x0100;

/// A built row with what the delegate shows.
#[derive(Debug, Clone, PartialEq)]
pub struct RowView {
    pub row: Row,
    pub icon: String,
    /// Badge colour, glyph and logo URL of a distro icon.
    pub distro: Option<(String, String, String)>,
    pub tooltip: String,
    /// The entry's environment id, its display name and its colour.
    pub environment: String,
    pub environment_name: String,
    pub environment_color: String,
}

#[derive(Default)]
pub struct HostListModelRust {
    views: Vec<RowView>,
    state: TreeState,
    filter: String,
    /// Folders collapsed by hand while a search is active (not remembered).
    filter_collapsed: BTreeSet<String>,
    probe_seq: i32,
    status: QString,
    hint: QString,
}

fn qs(s: &str) -> QString {
    QString::from(s)
}

fn kind_name(kind: RowKind) -> &'static str {
    match kind {
        RowKind::Recent => "recent",
        RowKind::Group => "group",
        RowKind::Connection => "connection",
        RowKind::Favorite => "favorite",
    }
}

fn entry_ref(kind: &QString, index: i32) -> Option<EntryRef> {
    Some(EntryRef {
        kind: EntryKind::parse(&kind.to_string())?,
        index: usize::try_from(index).ok()?,
    })
}

/// The mutable entry at `e`, for the environment menu.
fn entry_at_mut(cfg: &mut Value, e: EntryRef) -> Option<&mut Value> {
    let key = match e.kind {
        EntryKind::Connection => "connections",
        EntryKind::Favorite => "favorites",
    };
    match cfg.get_mut(key)? {
        Value::Seq(items) => items
            .get_mut(e.index)
            .filter(|v| matches!(v, Value::Map(_))),
        _ => None,
    }
}

fn set_entry_key(entry: &mut Value, key: &str, value: Value) {
    let Value::Map(pairs) = entry else { return };
    match pairs
        .iter_mut()
        .find(|(k, _)| matches!(k, Value::Str(s) if s == key))
    {
        Some(slot) => slot.1 = value,
        None => pairs.push((Value::Str(key.to_owned()), value)),
    }
}

fn remove_entry_key(entry: &mut Value, key: &str) {
    if let Value::Map(pairs) = entry {
        pairs.retain(|(k, _)| !matches!(k, Value::Str(s) if s == key));
    }
}

fn indices(list: &QList<i32>) -> Vec<usize> {
    list.iter()
        .filter_map(|&i| usize::try_from(i).ok())
        .collect()
}

fn to_qlist(rows: impl IntoIterator<Item = usize>) -> QList<i32> {
    rows.into_iter()
        .filter_map(|r| i32::try_from(r).ok())
        .collect()
}

fn parse_json(invokable: &str, text: &QString) -> Value {
    crate::qml_json::value(invokable, &text.to_string())
        .map(|j| Value::from_json(&j))
        .unwrap_or(Value::Null)
}

impl qobject::HostListModel {
    fn data(&self, index: &QModelIndex, role: i32) -> QVariant {
        let Some(v) = usize::try_from(index.row())
            .ok()
            .and_then(|r| self.views.get(r))
        else {
            return QVariant::default();
        };
        let (color, glyph, logo) = v.distro.clone().unwrap_or_default();
        let text = |s: &str| QVariant::from(&qs(s));
        match role - USER_ROLE {
            0 => QVariant::from(&i32::try_from(v.row.depth).unwrap_or(0)),
            1 => text(kind_name(v.row.kind)),
            2 => text(&v.row.label),
            3 => text(&v.row.subtitle),
            4 => text(&v.row.badge),
            5 => text(&v.icon),
            6 => text(&color),
            7 => text(&glyph),
            8 => text(&logo),
            9 => QVariant::from(&v.row.error),
            10 => QVariant::from(&v.row.expanded),
            11 => QVariant::from(&v.row.has_children),
            12 => text(&v.tooltip),
            13 => QVariant::from(&(!v.row.in_recent && v.row.kind != RowKind::Recent)),
            14 => text(&v.environment),
            15 => text(&v.environment_name),
            16 => text(&v.environment_color),
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
        i32::try_from(self.views.len()).unwrap_or(i32::MAX)
    }

    fn view(&self, row: i32) -> Option<&RowView> {
        usize::try_from(row).ok().and_then(|r| self.views.get(r))
    }

    fn refresh(mut self: Pin<&mut Self>) {
        let (views, status, hint) = with_app(|app| {
            let me = self.rust();
            build_views(&app.cfg, &me.state, &me.filter, &me.filter_collapsed)
        });
        // The same rows again (a config change that touched nothing shown, a repeated search):
        // keep the delegates, the scroll position and the selection instead of rebuilding them.
        if self.views != views {
            self.as_mut().begin_reset_model();
            self.as_mut().rust_mut().views = views;
            self.as_mut().end_reset_model();
        }
        self.as_mut().set_status(qs(&status));
        self.as_mut().set_hint(qs(&hint));
    }

    fn set_filter(mut self: Pin<&mut Self>, text: &QString) {
        let text = text.to_string();
        if text == self.filter {
            return;
        }
        let mut me = self.as_mut().rust_mut();
        me.filter = text;
        me.filter_collapsed.clear();
        self.refresh();
    }

    fn set_expanded(
        mut self: Pin<&mut Self>,
        row: i32,
        expanded: bool,
        keep: &QList<i32>,
    ) -> QList<i32> {
        let Some(view) = self.view(row) else {
            return keep.clone();
        };
        let target = view.row.clone();
        let old: Vec<Row> = self.views.iter().map(|v| v.row.clone()).collect();
        {
            let mut me = self.as_mut().rust_mut();
            let filtering = !strip_space(&me.filter).is_empty();
            if filtering && target.kind == RowKind::Group {
                if expanded {
                    me.filter_collapsed.remove(&target.path);
                } else {
                    me.filter_collapsed.insert(target.path.clone());
                }
            }
            me.state.set_expanded(&target, expanded, filtering);
        }
        self.as_mut().refresh();
        let new: Vec<Row> = self.views.iter().map(|v| v.row.clone()).collect();
        remap_rows(&old, &new, &indices(keep))
            .into_iter()
            .map(|r| r.and_then(|r| i32::try_from(r).ok()).unwrap_or(-1))
            .collect()
    }

    fn row_json(&self, row: i32) -> QString {
        let Some(v) = self.view(row) else {
            return QString::default();
        };
        let r = &v.row;
        let (protocol, group) =
            with_app(|app| match r.entry().and_then(|e| entry_at(&app.cfg, e)) {
                Some(e) => (connection_protocol(e), group_of(e)),
                None => ("", String::new()),
            });
        let protocol = if r.kind == RowKind::Connection {
            protocol
        } else {
            ""
        };
        let info = json!({
            "kind": kind_name(r.kind),
            "index": r.index,
            "path": r.path,
            "inRecent": r.in_recent,
            "expanded": r.expanded,
            "protocol": protocol,
            "group": group,
            "label": r.label,
        });
        qs(&info.to_string())
    }

    fn entry_json(&self, kind: &QString, index: i32) -> QString {
        let Some(e) = entry_ref(kind, index) else {
            return QString::default();
        };
        with_app(|app| entry_at(&app.cfg, e).map(|v| qs(&v.to_json().to_string())))
            .unwrap_or_default()
    }

    fn selected_connections(&self, rows: &QList<i32>) -> QList<i32> {
        let built: Vec<Row> = self.views.iter().map(|v| v.row.clone()).collect();
        to_qlist(hosttree::selected_connection_indices(
            &built,
            &indices(rows),
        ))
    }

    fn drop_zone(&self, row: i32, y_fraction: f64) -> i32 {
        match self
            .view(row)
            .and_then(|v| hosttree::drop_zone(&v.row, y_fraction))
        {
            Some(Zone::Above) => 0,
            Some(Zone::On) => 1,
            Some(Zone::Below) => 2,
            None => -1,
        }
    }

    fn drop_rows(
        mut self: Pin<&mut Self>,
        current: i32,
        selected: &QList<i32>,
        target: i32,
        y_fraction: f64,
    ) -> QString {
        let rows: Vec<Row> = self.views.iter().map(|v| v.row.clone()).collect();
        let selected = indices(selected);
        let Ok(current) = usize::try_from(current) else {
            return Outcome::default().json();
        };
        let target = usize::try_from(target).ok().filter(|&t| t < rows.len());
        if target.is_some_and(|t| drop_blocked(&rows, current, &selected, t)) {
            return Outcome::default().json();
        }
        let target = target.map(|t| {
            let zone = hosttree::drop_zone(&rows[t], y_fraction).unwrap_or(Zone::On);
            (t, zone)
        });
        let action = hosttree::resolve_drop(&rows, current, &selected, target);
        let mut state = self.state.clone();
        let out = with_app(|app| {
            let before = app.cfg.clone();
            match hosttree::apply_drop(&mut app.cfg, &mut state, &action) {
                Ok(select) => Outcome {
                    changed: app.cfg != before,
                    select,
                    ..Outcome::default()
                },
                // A moved folder reports in the status bar, not a box.
                Err(e) => Outcome::notice(e),
            }
        });
        self.as_mut().rust_mut().state = state;
        out.json()
    }

    fn reveal(mut self: Pin<&mut Self>, kind: &QString, index: i32) -> i32 {
        let Some(entry) = entry_ref(kind, index) else {
            return -1;
        };
        let mut state = self.state.clone();
        with_app(|app| reveal_entry(&app.cfg, &mut state, &self.filter, entry));
        self.as_mut().rust_mut().state = state;
        self.as_mut().refresh();
        self.views
            .iter()
            .position(|v| v.row.entry() == Some(entry))
            .and_then(|r| i32::try_from(r).ok())
            .unwrap_or(-1)
    }

    fn group_names(&self) -> QStringList {
        with_app(|app| hosttree::all_group_names(&app.cfg))
            .iter()
            .map(|s| qs(s))
            .collect()
    }

    fn delete_group_question(&self, path: &QString) -> QString {
        let path = normalize_group_path(&path.to_string());
        let doomed = with_app(|app| hosttree::group_entry_count(&app.cfg, &path));
        qs(&delete_group_question(&path, doomed))
    }

    fn dissolve_question(&self, path: &QString) -> QString {
        qs(&dissolve_question(&normalize_group_path(&path.to_string())))
    }

    fn store_entry(
        &self,
        kind: &QString,
        entry_kind: &QString,
        index: i32,
        data_json: &QString,
    ) -> QString {
        let target = entry_ref(entry_kind, index);
        let data = parse_json("store_entry", data_json);
        let out = with_app(|app| store_entry(&mut app.cfg, &kind.to_string(), target, data));
        match out {
            Ok(true) => Outcome::changed(None),
            Ok(false) => Outcome::default(),
            Err(Msg::Info(t)) => Outcome::info(t),
            Err(Msg::Warning(t)) => Outcome::warning(t),
        }
        .json()
    }

    fn duplicate(&self, kind: &QString, index: i32) -> QString {
        let Some(e) = entry_ref(kind, index) else {
            return Outcome::default().json();
        };
        with_app(|app| hosttree::duplicate_entry(&mut app.cfg, e))
            .map_or_else(Outcome::default, |s| Outcome::changed(Some(s)))
            .json()
    }

    fn move_by(&self, kind: &QString, index: i32, delta: i32) -> QString {
        let Some(e) = entry_ref(kind, index) else {
            return Outcome::default().json();
        };
        let delta = isize::try_from(delta).unwrap_or(0);
        with_app(|app| hosttree::move_entry(&mut app.cfg, e, delta))
            .map_or_else(Outcome::default, |s| Outcome::changed(Some(s)))
            .json()
    }

    fn move_to_group(
        mut self: Pin<&mut Self>,
        kind: &QString,
        index: i32,
        name: &QString,
    ) -> QString {
        let Some(e) = entry_ref(kind, index) else {
            return Outcome::default().json();
        };
        let mut state = self.state.clone();
        let out =
            with_app(|app| hosttree::move_to_group(&mut app.cfg, &mut state, e, &name.to_string()));
        self.as_mut().rust_mut().state = state;
        out.map_or_else(Outcome::default, |s| Outcome::changed(Some(s)))
            .json()
    }

    fn remove_entry(&self, kind: &QString, index: i32) -> QString {
        let Some(e) = entry_ref(kind, index) else {
            return Outcome::default().json();
        };
        with_app(|app| hosttree::remove_entry(&mut app.cfg, e));
        Outcome::changed(None).json()
    }

    fn bulk_delete(&self, list: &QList<i32>) -> QString {
        with_app(|app| hosttree::bulk_delete(&mut app.cfg, &indices(list)));
        Outcome::changed(None).json()
    }

    fn bulk_apply(mut self: Pin<&mut Self>, list: &QList<i32>, changes_json: &QString) -> QString {
        let changes = parse_json("bulk_apply", changes_json);
        let picked: Vec<i64> = list.iter().map(|&i| i64::from(i)).collect();
        let mut state = self.state.clone();
        let out = with_app(|app| bulk_apply(&mut app.cfg, &mut state, &picked, &changes));
        self.as_mut().rust_mut().state = state;
        out.json()
    }

    fn add_group(mut self: Pin<&mut Self>, parent: &QString, name: &QString) -> QString {
        let mut state = self.state.clone();
        let added = with_app(|app| {
            hosttree::add_group(
                &mut app.cfg,
                &mut state,
                &parent.to_string(),
                &name.to_string(),
            )
        });
        self.as_mut().rust_mut().state = state;
        if added {
            Outcome::changed(None)
        } else {
            Outcome::default()
        }
        .json()
    }

    fn rename_group(mut self: Pin<&mut Self>, old: &QString, path: &QString) -> QString {
        let mut state = self.state.clone();
        let out = with_app(|app| {
            let before = app.cfg.clone();
            match hosttree::apply_group_rename(
                &mut app.cfg,
                &mut state,
                &old.to_string(),
                &path.to_string(),
            ) {
                Ok(()) => Outcome {
                    changed: app.cfg != before,
                    ..Outcome::default()
                },
                Err(e) => Outcome::warning(e),
            }
        });
        self.as_mut().rust_mut().state = state;
        out.json()
    }

    fn group_environment(&self, path: &QString) -> QString {
        let path = normalize_group_path(&path.to_string());
        with_app(|app| {
            let own = hosttree::group_environments(&app.cfg)
                .get(&path)
                .cloned()
                .unwrap_or_default();
            let (shown, from) = hosttree::folder_environment(&app.cfg, &path);
            qs(&serde_json::json!({ "own": own, "inherited": shown, "from": from }).to_string())
        })
    }

    fn environments_json(&self) -> QString {
        with_app(|app| qs(&environments::to_json(&environments::list(&app.cfg)).to_string()))
    }

    fn entry_environment(&self, kind: &QString, index: i32) -> QString {
        let Some(e) = entry_ref(kind, index) else {
            return QString::default();
        };
        with_app(|app| {
            let Some(v) = entry_at(&app.cfg, e) else {
                return QString::default();
            };
            let own = hosttree::environment_of(v);
            let (inherited, from) = hosttree::folder_environment(&app.cfg, &group_of(v));
            qs(
                &serde_json::json!({ "own": own, "inherited": inherited, "from": from })
                    .to_string(),
            )
        })
    }

    fn set_entry_environment(
        self: Pin<&mut Self>,
        kind: &QString,
        index: i32,
        env: &QString,
    ) -> QString {
        let Some(e) = entry_ref(kind, index) else {
            return Outcome::default().json();
        };
        with_app(|app| {
            let id = environments::canonical_id(&app.cfg, &env.to_string());
            let Some(entry) = entry_at_mut(&mut app.cfg, e) else {
                return;
            };
            match id {
                Some(id) => set_entry_key(entry, "environment", Value::Str(id)),
                None => remove_entry_key(entry, "environment"),
            }
        });
        Outcome::changed(None).json()
    }

    fn set_group_environment(self: Pin<&mut Self>, path: &QString, env: &QString) -> QString {
        let path = normalize_group_path(&path.to_string());
        if path.is_empty() {
            return Outcome::default().json();
        }
        with_app(|app| hosttree::set_group_environment(&mut app.cfg, &path, &env.to_string()));
        Outcome::changed(None).json()
    }

    fn open_json(&self, kind: &QString, index: i32) -> QString {
        let Some(e) = entry_ref(kind, index) else {
            return QString::default();
        };
        with_app(|app| {
            entry_at(&app.cfg, e).map(|v| {
                let mut v = v.clone();
                let env = hosttree::effective_environment(&app.cfg, &v);
                if !env.is_empty()
                    && hosttree::environment_of(&v).is_empty()
                    && let Value::Map(pairs) = &mut v
                {
                    pairs.push((Value::Str("environment".into()), Value::Str(env)));
                }
                qs(&v.to_json().to_string())
            })
        })
        .unwrap_or_default()
    }

    fn ungroup(mut self: Pin<&mut Self>, path: &QString) -> QString {
        let path = normalize_group_path(&path.to_string());
        if path.is_empty() {
            return Outcome::default().json();
        }
        let mut state = self.state.clone();
        with_app(|app| hosttree::ungroup_group(&mut app.cfg, &mut state, &path));
        self.as_mut().rust_mut().state = state;
        Outcome::changed(None).json()
    }

    fn delete_group(mut self: Pin<&mut Self>, path: &QString) -> QString {
        let path = normalize_group_path(&path.to_string());
        if path.is_empty() {
            return Outcome::default().json();
        }
        let mut state = self.state.clone();
        with_app(|app| hosttree::delete_group(&mut app.cfg, &mut state, &path));
        self.as_mut().rust_mut().state = state;
        Outcome::changed(None).json()
    }

    fn copy_command(&self, index: i32) -> QString {
        let out = with_app(|app| {
            let c = connection_at(&app.cfg, index)?;
            Some(match copy_command(&app.cfg, c) {
                Ok((text, notice)) => Outcome {
                    copy: Some(text),
                    notice: Some(notice),
                    ..Outcome::default()
                },
                Err(w) => Outcome::warning(w),
            })
        });
        out.unwrap_or_default().json()
    }

    fn copy_address(&self, index: i32) -> QString {
        let out = with_app(|app| {
            let Some(c) = connection_at(&app.cfg, index) else {
                return Some(Outcome::default());
            };
            let c = with_credential(&app.cfg, c);
            let host = strip_space(&text(&c, "host")).to_owned();
            let host = if host.is_empty() {
                strip_space(&text(&c, "device")).to_owned() // serial entries have no host
            } else {
                host
            };
            if host.is_empty() {
                return Some(Outcome::warning("This entry has no host.".to_owned()));
            }
            let user = strip_space(&text(&c, "user")).to_owned();
            let port = c.get("port").and_then(value_int).unwrap_or(22);
            let mut address = if user.is_empty() {
                host
            } else {
                format!("{user}@{host}")
            };
            if port != 22 {
                address.push_str(&format!(":{port}"));
            }
            Some(Outcome {
                copy: Some(address.clone()),
                notice: Some(format!("Copied the address “{address}” to the clipboard.")),
                ..Outcome::default()
            })
        });
        out.unwrap_or_default().json()
    }

    fn nettool(&self, index: i32, tool: &QString) -> QString {
        let tool = tool.to_string();
        let out = with_app(|app| connection_at(&app.cfg, index).map(|c| nettool_line(c, &tool)));
        match out {
            Some(Ok((cmd, title))) => qs(&json!({ "cmd": cmd, "title": title }).to_string()),
            Some(Err(w)) => Outcome::warning(w).json(),
            None => Outcome::default().json(),
        }
    }

    fn tools_json(&self) -> QString {
        qs(&json!(nettools::TOOLS).to_string())
    }

    fn icon_choices_json(&self) -> QString {
        let rows: Vec<_> = ICON_CHOICES
            .iter()
            .map(|(label, name)| json!([label, name, theme_icon(name)]))
            .collect();
        qs(&json!(rows).to_string())
    }

    fn test_connection(mut self: Pin<&mut Self>, entry_json: &QString) -> i32 {
        let conn = parse_json("test_connection", entry_json);
        if !matches!(conn, Value::Map(_)) {
            return -1;
        }
        let timeout = with_app(|app| probe_timeout(&app.cfg));
        let token = self.probe_seq.wrapping_add(1);
        self.as_mut().rust_mut().probe_seq = token;
        let thread = self.qt_thread();
        // A plain thread; a probe cannot be interrupted, and once the model is gone the
        // queued result is dropped.
        let spawned = std::thread::Builder::new()
            .name("termigator-probe".to_owned())
            .spawn(move || {
                let text = report_json(&probe_connection(&conn, timeout, true));
                let _ = thread.queue(move |m| m.probe_finished(token, qs(&text)));
            });
        if spawned.is_ok() { token } else { -1 }
    }

    fn probe_title(&self, entry_json: &QString) -> QString {
        qs(&format!(
            "Test connection — {}",
            entry_label(&parse_json("probe_title", entry_json))
        ))
    }

    fn kind_support(&self) -> QString {
        let launch = with_app(|app| app.cfg.get("launch").cloned().unwrap_or(Value::Null));
        let which = termigator_core::proto::vnc::which_in_path;
        let out: serde_json::Map<String, serde_json::Value> =
            ["ssh", "rdp", "vnc", "telnet", "serial", "favorite"]
                .into_iter()
                .filter_map(|k| {
                    termigator_core::proto::support::kind_unavailable(
                        k,
                        cfg!(windows),
                        &launch,
                        &which,
                    )
                    .map(|why| (k.to_owned(), json!(why)))
                })
                .collect();
        qs(&serde_json::Value::Object(out).to_string())
    }

    fn shells(&self) -> QString {
        let which = |n: &str| ssh::which(n, std::env::var("PATH").ok().as_deref());
        qs(&crate::panes_qobj::local_shells_json(
            &termigator_core::proto::local_shell::detect(&which),
        ))
    }

    fn menu_icon(&self, text: &QString) -> QString {
        qs(&menu_icon(&text.to_string())
            .map(theme_icon)
            .unwrap_or_default())
    }
}

/// The first icon of `a|b` the theme has.
fn theme_icon(names: &str) -> String {
    shell_qobj::qobject::theme_icon(&qs(names)).to_string()
}

// ---- Pure logic (unit-tested below) ------------------------------------------------------

/// What a mutation tells QML.
#[derive(Debug, Default, PartialEq)]
struct Outcome {
    changed: bool,
    select: Option<EntryRef>,
    info: Option<String>,
    warning: Option<String>,
    notice: Option<String>,
    copy: Option<String>,
}

impl Outcome {
    fn changed(select: Option<EntryRef>) -> Self {
        Self {
            changed: true,
            select,
            ..Self::default()
        }
    }

    fn info(text: String) -> Self {
        Self {
            info: Some(text),
            ..Self::default()
        }
    }

    fn warning(text: String) -> Self {
        Self {
            warning: Some(text),
            ..Self::default()
        }
    }

    fn notice(text: String) -> Self {
        Self {
            notice: Some(text),
            ..Self::default()
        }
    }

    fn json(&self) -> QString {
        let select = self.select.map(|e| json!([e.kind.name(), e.index]));
        qs(&json!({
            "changed": self.changed,
            "select": select,
            "info": self.info,
            "warning": self.warning,
            "notice": self.notice,
            "copy": self.copy,
        })
        .to_string())
    }
}

/// A message box: `information` or `warning`, titled "Termigator".
#[derive(Debug, PartialEq)]
pub enum Msg {
    Info(String),
    Warning(String),
}

mod entries;
mod icons;
mod rows;

pub use entries::*;
pub use icons::*;
pub use rows::*;

#[cfg(test)]
mod tests;
