//! Terminal panes and tabs: `PanesController`, exposed to QML as `Termigator.PanesController`.
//!
//! The pane tree, where each tab sits, the active pane and the detached windows are the pane
//! structure ([`Panes`]); `qml/panes/PanesArea.qml` rebuilds its views from [`Panes::to_json`]
//! after every change. A tab is only an id here: its title, command and session live in QML
//! (`PaneTab.qml`) and come back as records when the `last_session` snapshot is taken.
//!
//! The dialogs (warnings, the vault unlock) are QML's: the plans below say which one to show.

use std::collections::{BTreeMap, HashMap};
use std::ffi::OsString;
use std::pin::Pin;

use cxx_qt::CxxQtType;
use cxx_qt_lib::QString;
use serde_json::{Value as J, json};
use termigator_core::config::doc::Value;
use termigator_core::config::model::{
    PROTOCOL_RDP, PROTOCOL_SERIAL, PROTOCOL_SSH, PROTOCOL_VNC, TERMINAL_EXTERNAL,
    connection_protocol, default_terminal_from_cfg, normalize_terminal_mode, section, strip_space,
    terminal_mode_for_entry, truthy, value_str,
};
use termigator_core::credentials::apply_credential;
use termigator_core::hosttree::{entry_label, touch_recent};
use termigator_core::panes::{
    Dock, Location, Orientation, Panes, RestoreSource, RestoreTab, TabId, TabRecord, hit_zone,
    restore,
};
use termigator_core::proc::external::spawn_external_terminal;
use termigator_core::proto::gateway;
use termigator_core::proto::konsole::external_terminal_argv;
use termigator_core::proto::local_shell::{self, LocalShell};
use termigator_core::proto::rdp::rdp_password_env;
use termigator_core::proto::serial::serial_access_problem;
use termigator_core::proto::ssh::{
    build_ssh_command, parse_quick_ssh_target, ssh_password_env, ssh_session_env, which,
    x11_display_warning,
};
use termigator_core::shell::quote;
use termigator_core::vault::{self, Revealed, Secret, VaultError};

use crate::shell_qobj::{remember_quick_target, window_bool, with_app};
use crate::sidebar_qobj::build_conn_command;

#[cxx_qt::bridge]
pub mod qobject {
    unsafe extern "C++" {
        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;
        include!("cxx-qt-lib/qstringlist.h");
        type QStringList = cxx_qt_lib::QStringList;
    }

    #[auto_cxx_name]
    extern "RustQt" {
        #[qobject]
        #[qml_element]
        type PanesController = super::PanesControllerRust;

        /// The tree as JSON ([`termigator_core::panes::Panes::to_json`]).
        #[qinvokable]
        fn tree(self: &PanesController) -> QString;

        /// A new tab in `pane` at `at` (< 0: the end), current, its pane active; 0 if no such pane.
        #[qinvokable]
        fn add_tab(self: Pin<&mut PanesController>, pane: u64, at: i32) -> u64;

        /// A new tab in the detached `window` at `at` (< 0: the end); 0 if no such window.
        #[qinvokable]
        fn add_window_tab(self: Pin<&mut PanesController>, window: u64, at: i32) -> u64;

        #[qinvokable]
        fn set_current(self: Pin<&mut PanesController>, pane: u64, index: i32);

        #[qinvokable]
        fn set_window_current(self: Pin<&mut PanesController>, window: u64, index: i32);

        #[qinvokable]
        fn set_active(self: Pin<&mut PanesController>, pane: u64);

        /// Splits the active pane (right, or down when `vertical`); `extent` is its width (or
        /// height). Returns the new, now active pane.
        #[qinvokable]
        fn split(self: Pin<&mut PanesController>, vertical: bool, extent: i32) -> u64;

        /// The sizes of the splitter at `path_json` (child indices from the root, `[]` = root).
        #[qinvokable]
        fn set_sizes(self: Pin<&mut PanesController>, path_json: &QString, sizes_json: &QString);

        /// Takes a tab out; an emptied non-main pane or detached window goes too unless `keep_pane`.
        #[qinvokable]
        fn remove_tab(self: Pin<&mut PanesController>, tab: u64, keep_pane: bool);

        /// Takes out every tab of `pane` and the pane (the main pane is only emptied).
        #[qinvokable]
        fn close_pane(self: Pin<&mut PanesController>, pane: u64);

        /// Moves a tab into a new detached window; returns the window (0 if no such tab).
        #[qinvokable]
        fn detach_tab(self: Pin<&mut PanesController>, tab: u64) -> u64;

        /// Moves a detached tab back into the active pane as its current tab.
        #[qinvokable]
        fn attach_tab(self: Pin<&mut PanesController>, tab: u64);

        /// Drops `tab` on a pane (`to_window` false) or a detached window (`target` is its id).
        /// `dock` is "center" or a side; `at` < 0 means the end; `extent` as in `split`. Returns
        /// whether it moved.
        #[qinvokable]
        fn move_tab(
            self: Pin<&mut PanesController>,
            tab: u64,
            to_window: bool,
            target: u64,
            dock: &QString,
            at: i32,
            extent: i32,
        ) -> bool;

        /// Docks `tab` at an outer edge of the pane area (`dock` a side); returns the new
        /// pane, 0 on failure.
        #[qinvokable]
        fn dock_at_edge(
            self: Pin<&mut PanesController>,
            tab: u64,
            dock: &QString,
            extent: i32,
        ) -> u64;

        /// The dock zone ("center", "left", ... or "") under (`x`, `y`) in a `w` × `h` pane.
        #[qinvokable]
        fn hit_zone(self: &PanesController, w: f64, h: f64, x: f64, y: f64, guide: f64) -> QString;

        /// This process's id, in a dragged tab's MIME data (a drop from another instance is refused).
        #[qinvokable]
        fn pid(self: &PanesController) -> u32;

        /// On exit: `last_session` from the tree and `records_json` (`{"<tab id>": {title,
        /// conn, inner}}` for the embedded terminal tabs) when `window.session_restore`, else
        /// dropped. Not saved here: the window's `saveWindowState` writes the config next.
        #[qinvokable]
        fn save_snapshot(self: &PanesController, records_json: &QString);

        /// At start: rebuilds the tree from `last_session` (when `window.session_restore`)
        /// and returns the tabs to open as JSON: `[{tab, pane, title, detached, inner}` or
        /// `{…, entry, matched}]`; `[]` leaves the tree as it was.
        #[qinvokable]
        fn restore(self: Pin<&mut PanesController>) -> QString;

        /// `_new_local_shell`: `{inner, title, mode}` for `shell_path` ("" = the default shell).
        #[qinvokable]
        fn local_shell(self: &PanesController, shell_path: &QString) -> QString;

        /// The "+" menu's shells, `[{id, label, available}]` (Linux: /etc/shells; Windows: cmd,
        /// PowerShell, WSL, the missing ones greyed out).
        #[qinvokable]
        fn local_shells(self: &PanesController) -> QString;

        /// Whether a terminal mode (`normalize_terminal_mode`) means the external terminal.
        #[qinvokable]
        fn is_external(self: &PanesController, mode: &QString) -> bool;

        /// `spawn_konsole`: starts `inner` in the external terminal (`launch.template`) with
        /// `extra_env_json` ("" or a JSON object of strings); "" or the error to show.
        #[qinvokable]
        fn spawn_external(
            self: &PanesController,
            inner: &QString,
            extra_env_json: &QString,
        ) -> QString;

        /// `openConnection` for a saved connection or favorite ([`super::connection_plan`]) as
        /// JSON: `{files}`, `{stop, locked}` or `{inner, title, mode, error, env, notice, conn,
        /// distro, recent}`; `recent`: it is now the newest Recent entry (not saved yet). `screen`: `WxH`
        /// of the display the window is on (the RDP start size), "" when unknown.
        #[qinvokable]
        fn connection_plan(
            self: &PanesController,
            entry_json: &QString,
            screen: &QString,
        ) -> QString;

        /// `_reopen_tab_record` for a restored connection tab: `{inner, env}`, `{}` = drop it.
        #[qinvokable]
        fn reopen_plan(self: &PanesController, entry_json: &QString) -> QString;

        /// `_quick_connect` ([`super::quick_plan`]): `{entry}`, `{error}` or `{inner, title,
        /// mode, conn}` (then the target is first in `quick_history`, not saved yet).
        #[qinvokable]
        fn quick_plan(self: &PanesController, text: &QString) -> QString;
    }
}

#[derive(Default)]
pub struct PanesControllerRust {
    panes: Panes,
}

fn index(i: i32) -> Option<usize> {
    usize::try_from(i).ok()
}

impl qobject::PanesController {
    fn tree(&self) -> QString {
        QString::from(self.panes.to_json().to_string().as_str())
    }

    fn add_tab(self: Pin<&mut Self>, pane: u64, at: i32) -> u64 {
        let p = self.panes_mut();
        let tab = p.new_tab_id();
        p.add_tab(pane, tab, index(at)).map_or(0, |()| tab)
    }

    fn add_window_tab(self: Pin<&mut Self>, window: u64, at: i32) -> u64 {
        let p = self.panes_mut();
        let tab = p.new_tab_id();
        p.add_window_tab(window, tab, index(at)).map_or(0, |()| tab)
    }

    fn set_current(self: Pin<&mut Self>, pane: u64, i: i32) {
        if let Some(i) = index(i) {
            let _ = self.panes_mut().set_current(pane, i);
        }
    }

    fn set_window_current(self: Pin<&mut Self>, window: u64, i: i32) {
        if let Some(i) = index(i) {
            let _ = self.panes_mut().set_window_current(window, i);
        }
    }

    fn set_active(self: Pin<&mut Self>, pane: u64) {
        let _ = self.panes_mut().set_active(pane);
    }

    fn split(self: Pin<&mut Self>, vertical: bool, extent: i32) -> u64 {
        let p = self.panes_mut();
        let o = if vertical {
            Orientation::Vertical
        } else {
            Orientation::Horizontal
        };
        p.split(p.active(), o, true, extent.into()).unwrap_or(0)
    }

    fn set_sizes(self: Pin<&mut Self>, path_json: &QString, sizes_json: &QString) {
        let (path_json, sizes_json) = (path_json.to_string(), sizes_json.to_string());
        let path: Option<Vec<usize>> =
            crate::qml_json::logged("set_sizes", &path_json, serde_json::from_str(&path_json));
        let sizes: Option<Vec<i64>> =
            crate::qml_json::logged("set_sizes", &sizes_json, serde_json::from_str(&sizes_json));
        if let (Some(path), Some(sizes)) = (path, sizes) {
            let _ = self.panes_mut().set_sizes(&path, sizes);
        }
    }

    fn remove_tab(self: Pin<&mut Self>, tab: u64, keep_pane: bool) {
        let _ = self.panes_mut().close_tab(tab, keep_pane);
    }

    fn close_pane(self: Pin<&mut Self>, pane: u64) {
        let _ = self.panes_mut().close_pane(pane);
    }

    fn detach_tab(self: Pin<&mut Self>, tab: u64) -> u64 {
        self.panes_mut().detach_tab(tab).unwrap_or(0)
    }

    fn attach_tab(self: Pin<&mut Self>, tab: u64) {
        let _ = self.panes_mut().attach_tab(tab);
    }

    fn move_tab(
        self: Pin<&mut Self>,
        tab: u64,
        to_window: bool,
        target: u64,
        dock: &QString,
        at: i32,
        extent: i32,
    ) -> bool {
        let Some(dock) = Dock::parse(&dock.to_string()) else {
            return false;
        };
        let to = if to_window {
            Location::Window(target)
        } else {
            Location::Pane(target)
        };
        self.panes_mut()
            .move_tab(tab, to, dock, index(at), extent.into())
            .is_ok()
    }

    fn dock_at_edge(self: Pin<&mut Self>, tab: u64, dock: &QString, extent: i32) -> u64 {
        let Some(dock) = Dock::parse(&dock.to_string()) else {
            return 0;
        };
        self.panes_mut()
            .dock_at_edge(tab, dock, extent.into())
            .unwrap_or(0)
    }

    fn hit_zone(&self, w: f64, h: f64, x: f64, y: f64, guide: f64) -> QString {
        QString::from(hit_zone(w, h, x, y, guide).map_or("", Dock::name))
    }

    fn pid(&self) -> u32 {
        std::process::id()
    }

    fn save_snapshot(&self, records_json: &QString) {
        let records = parse_records("save_snapshot", &records_json.to_string());
        with_app(|app| save_snapshot(&mut app.cfg, &self.panes, &records));
    }

    fn restore(self: Pin<&mut Self>) -> QString {
        let Some(restored) = with_app(|app| {
            let forced = std::env::args().any(|a| a == crate::shell_qobj::RESTORE_ARG);
            restore_session(&app.cfg, forced)
        }) else {
            return QString::from("[]");
        };
        *self.panes_mut() = restored.panes;
        QString::from(restore_json(&restored.tabs).to_string().as_str())
    }

    fn panes_mut(self: Pin<&mut Self>) -> &mut Panes {
        &mut self.rust_mut().get_mut().panes
    }

    fn local_shell(&self, shell_path: &QString) -> QString {
        let shell_env = std::env::var("SHELL").unwrap_or_default();
        let (inner, title, mode) = with_app(|app| {
            let shells = local_shell::detect(&path_which);
            local_shell(
                &app.cfg,
                &shell_path.to_string(),
                &shell_env,
                &path_which,
                &shells,
            )
        });
        let out = json!({"inner": inner, "title": title, "mode": mode});
        QString::from(out.to_string().as_str())
    }

    fn local_shells(&self) -> QString {
        QString::from(local_shells_json(&local_shell::detect(&path_which)).as_str())
    }

    fn is_external(&self, mode: &QString) -> bool {
        is_external(&mode.to_string())
    }

    fn spawn_external(&self, inner: &QString, extra_env_json: &QString) -> QString {
        let argv = with_app(|app| {
            external_terminal_argv(
                section(&app.cfg, "launch"),
                inner.to_string().trim(),
                &path_which,
            )
        });
        let mut env = env_map("spawn_external", &extra_env_json.to_string());
        let spawned = argv.and_then(|a| spawn_external_terminal(&a, &env));
        vault::wipe_env(&mut env);
        let err = match spawned {
            Ok(pid) => {
                // Follow mode targets this exact window.
                with_app(|app| app.last_konsole_pid = Some(pid));
                String::new()
            }
            Err(e) if e.is_empty() => "Could not start external terminal.".to_owned(),
            Err(e) => e,
        };
        QString::from(err.as_str())
    }

    fn connection_plan(&self, entry_json: &QString, screen: &QString) -> QString {
        let Some(entry) = parse_entry("connection_plan", entry_json) else {
            return QString::from("{}");
        };
        let display = std::env::var("DISPLAY").ok();
        let screen = screen.to_string();
        let key = vault::session_key();
        let out = with_app(|app| {
            let plan = connection_plan(
                &app.cfg,
                &entry,
                key.as_ref(),
                display.as_deref(),
                Some(screen.as_str()).filter(|s| !s.is_empty()),
            );
            let recent = matches!(&plan, Plan::Open { conn: true, error, inner, .. }
                if error.is_empty() && !inner.is_empty());
            if recent {
                touch_recent(&mut app.cfg, &entry);
            }
            plan_json(&plan, recent)
        });
        QString::from(out.to_string().as_str())
    }

    fn reopen_plan(&self, entry_json: &QString) -> QString {
        let key = vault::session_key();
        let entry = parse_entry("reopen_plan", entry_json);
        // `locked`: the password is sealed and the master was not entered yet — the restore asks
        // for it and plans again.
        let locked = entry
            .as_ref()
            .is_some_and(|e| with_app(|app| reopen_locked(&app.cfg, e, key.as_ref())));
        let out = entry
            .and_then(|e| with_app(|app| reopen_plan(&app.cfg, &e, key.as_ref())))
            .map_or_else(
                || json!({}),
                |(inner, env)| json!({"inner": inner, "env": env_json(&env), "locked": locked}),
            );
        QString::from(out.to_string().as_str())
    }

    fn quick_plan(&self, text: &QString) -> QString {
        let target = text.to_string();
        let out = with_app(|app| match quick_plan(&app.cfg, &target) {
            Quick::Entry(e) => json!({"entry": e.to_json()}),
            Quick::Error(e) => json!({"error": e}),
            Quick::Open {
                inner,
                title,
                mode,
                conn,
            } => {
                remember_quick_target(&mut app.cfg, strip_space(&target));
                json!({"inner": inner, "title": title, "mode": mode, "conn": conn.to_json()})
            }
        });
        QString::from(out.to_string().as_str())
    }
}

// ---- Pure logic (unit-tested in `tests`) ----------------------------------------------------

mod plans;

pub use plans::*;

#[cfg(test)]
mod tests;
