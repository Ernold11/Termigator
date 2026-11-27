//! Application shell: `AppController`, exposed to QML as `Termigator.AppController`.
//!
//! The merged config ([`Value`]) and the layout of the whole UI live in one shared [`AppState`]
//! (GUI thread only). The controller's typed invokables read and change it and `persist()` writes
//! it; area QObjects (`<area>_qobj.rs`) use [`with_app`] and leave saving
//! to `controller.persist()` in QML, which also emits `configChanged`. It also runs the control socket (`--toggle` / `--show`),
//! registers the KGlobalAccel "Show / hide Termigator" action and hands QML the action table as JSON.
//! The other areas add their own `<area>_qobj.rs` rather than growing this object.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::pin::Pin;

use cxx_qt::{CxxQtType, Threading};
use cxx_qt_lib::{QSize, QString, QStringList, QUrl};
use serde_json::json;
use termigator_core::actions::{self, Context};
use termigator_core::close_action::{self, CloseAction};
use termigator_core::config::doc::Value;
use termigator_core::config::environments;
use termigator_core::config::model::{section, truthy, value_int};
use termigator_core::config::save::{self, ConfigError, merge_defaults, merge_defaults_owned};
use termigator_core::hosttree::entry_label;
use termigator_core::paths::{Layout, Paths, ProcessEnv};
use termigator_core::proc::external::follow_window;
use termigator_core::proto::ssh::which;
use termigator_core::quick_history;
use termigator_core::single_instance::{ControlServer, control_socket_path};
use termigator_core::update_check;
use termigator_core::vault;

use crate::vault_qobj;

/// Smoke-test mode: when set, the window runs `SmokeCheck.qml`, saves a grab to this path and
/// quits with a status code. Used only by `tests/smoke.rs`.
pub const SMOKE_TEST_ENV: &str = "TERMIGATOR_SMOKE_TEST";

/// Tests only: the update check reads this URL (a `file://` JSON) instead of GitHub, also in
/// smoke-test mode and without the once-a-day limit.
pub const UPDATE_URL_ENV: &str = "TERMIGATOR_UPDATE_URL";

#[cxx_qt::bridge]
pub mod qobject {
    unsafe extern "C++" {
        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;
        include!("cxx-qt-lib/qstringlist.h");
        type QStringList = cxx_qt_lib::QStringList;
        include!("cxx-qt-lib/qsize.h");
        type QSize = cxx_qt_lib::QSize;
        include!("cxx-qt-lib/qurl.h");
        type QUrl = cxx_qt_lib::QUrl;
    }

    #[namespace = "termigator::shell"]
    unsafe extern "C++" {
        include!("termigator/src/shell_native.h");
        #[rust_name = "about_qt_box"]
        fn aboutQt();
        #[rust_name = "about_termigator_box"]
        fn aboutTermigator(title: &QString, html: &QString);
        #[rust_name = "qt_version"]
        fn qtVersion() -> QString;
        #[rust_name = "theme_icon"]
        fn themeIcon(names: &QString) -> QString;
        #[rust_name = "has_theme_icon"]
        fn hasThemeIcon(name: &QString) -> bool;
        #[rust_name = "set_app_icon"]
        fn setAppIcon(assets_dir: &QString);
        #[rust_name = "apply_fixed_look"]
        fn applyFixedLook(mode: &QString);
        #[rust_name = "startup_error"]
        fn startupError(text: &QString, folder: &QString);
    }

    #[auto_cxx_name]
    extern "RustQt" {
        #[qobject]
        #[qml_element]
        #[qproperty(QString, version, READ, CONSTANT)]
        #[qproperty(QString, actions_json, READ, CONSTANT)]
        #[qproperty(QString, shortcut_rows_json, READ, CONSTANT)]
        #[qproperty(QString, first_run_message, READ, CONSTANT)]
        #[qproperty(QString, about_text, READ, CONSTANT)]
        #[qproperty(QUrl, tray_icon_url, READ, CONSTANT)]
        #[qproperty(QString, tray_icon_name, READ, CONSTANT)]
        #[qproperty(QString, smoke_test_shot, READ, CONSTANT)]
        type AppController = super::AppControllerRust;

        /// `--toggle` / `--show` from the control socket or the global shortcut ("toggle"); any
        /// activation token is already handed to the windowing system.
        #[qsignal]
        fn remote_command(self: Pin<&mut AppController>, command: QString);

        /// A message for a warning box titled "Termigator".
        #[qsignal]
        fn warning(self: Pin<&mut AppController>, text: QString);

        /// The config was saved without new plain-text passwords because the vault is locked:
        /// the UI asks for the master password and saves again.
        #[qsignal]
        fn seal_needed(self: Pin<&mut AppController>);

        /// Follow mode: the window of the external terminal moved or resized; the Termigator window
        /// goes to (`x`, `y`).
        #[qsignal]
        fn follow_move(self: Pin<&mut AppController>, x: i32, y: i32);

        /// The config was saved (the list and the quick-connect completer refresh).
        #[qsignal]
        fn config_changed(self: Pin<&mut AppController>);

        /// `window.follow_konsole` is on and `xdotool` is installed (read once at start).
        #[qinvokable]
        fn follow_enabled(self: &AppController) -> bool;

        /// `int(window.follow_poll_ms or 500)` (at least 1).
        #[qinvokable]
        fn follow_poll_ms(self: &AppController) -> i32;

        /// One poll of follow mode (the timer's tick): looks the terminal window up on a worker
        /// thread (xdotool may block) and emits `followMove`; `frame_width` is this window's
        /// width. A tick while the previous lookup still runs is skipped.
        #[qinvokable]
        fn follow_tick(self: &AppController, frame_width: i32);

        /// `bool(window.get(key, fallback))`.
        #[qinvokable]
        fn window_bool(self: &AppController, key: &QString, fallback: bool) -> bool;

        /// `int(window.get(key) or fallback)`.
        #[qinvokable]
        fn window_int(self: &AppController, key: &QString, fallback: i32) -> i32;

        /// The colour of environment `name` (an id or display name), `""` when unknown.
        #[qinvokable]
        fn environment_color(self: &AppController, name: &QString) -> QString;

        /// `window[key] = value` (not saved; call `persist`).
        #[qinvokable]
        fn set_window_bool(self: &AppController, key: &QString, value: bool);

        /// Saves the config; on failure emits `warning` and returns false.
        #[qinvokable]
        fn persist(self: Pin<&mut AppController>) -> bool;

        /// The URL to ask for the latest release, `""` when not to check now (`window.check_updates`
        /// off, checked less than a day ago, smoke-test mode). Handing one out counts as a check.
        #[qinvokable]
        fn update_check_url(self: &AppController) -> QString;

        /// The answer to that request: `{"version", "url"}` of a newer, not skipped release as
        /// JSON, else `""` (an error page or a 404 while the repository is private included).
        #[qinvokable]
        fn update_check_result(self: &AppController, body: &QString) -> QString;

        /// "Skip this version": the banner does not come back for `version`.
        #[qinvokable]
        fn skip_update(self: &AppController, version: &QString);

        /// On exit: window size and sidebar width (and `last_session` dropped unless
        /// `session_restore`); a failed write never blocks closing.
        #[qinvokable]
        fn save_window_state(self: &AppController, width: i32, height: i32, list_width: i32);

        /// The next quit starts Termigator again, with the open tabs reopened (a new interface
        /// style needs a new process: Qt picks it before the first QML is loaded).
        #[qinvokable]
        fn request_restart(self: &AppController);

        /// Before [`save_window_state`](Self::save_window_state): whether the window is maximized
        /// and, with `with_position` (Windows), where its normal geometry was.
        #[qinvokable]
        fn set_window_placement(
            self: &AppController,
            x: i32,
            y: i32,
            with_position: bool,
            maximized: bool,
        );

        /// The close-confirmation question, or "" when the window may close at once.
        #[qinvokable]
        fn close_question(
            self: &AppController,
            sessions: i32,
            transfers: i32,
            tunnels: i32,
        ) -> QString;

        /// What the close button does (`termigator_core::close_action`): "ask", "tray" or "exit".
        #[qinvokable]
        fn close_action(self: &AppController) -> QString;

        /// Stores the close button's choice (not saved; call `persist`); unknown text is ignored.
        #[qinvokable]
        fn set_close_action(self: &AppController, action: &QString);

        /// The details under the close dialog's question: what Exit would end, "" when nothing runs.
        #[qinvokable]
        fn close_choice_details(
            self: &AppController,
            sessions: i32,
            transfers: i32,
            tunnels: i32,
        ) -> QString;

        /// Quick-connect history, then entry labels, without duplicates.
        #[qinvokable]
        fn quick_completions(self: &AppController) -> QStringList;

        /// The quick-connect history, newest first (`termigator_core::quick_history`).
        #[qinvokable]
        fn quick_history(self: &AppController) -> QStringList;

        /// Removes one quick-connect history entry (not saved; QML calls `persist()`).
        #[qinvokable]
        fn forget_quick_target(self: &AppController, target: &QString);

        /// Empties the quick-connect history (not saved; QML calls `persist()`).
        #[qinvokable]
        fn clear_quick_history(self: &AppController);

        /// `window.dialog_sizes.<name>`; an empty size when none is stored (ADR-13).
        #[qinvokable]
        fn dialog_size(self: &AppController, name: &QString) -> QSize;

        /// Stores a dialog size; written with the next save of the config.
        #[qinvokable]
        fn set_dialog_size(self: &AppController, name: &QString, width: i32, height: i32);

        /// File → Open config folder: the folder URL, or "" after a `warning` when it is missing.
        #[qinvokable]
        fn config_folder_url(self: Pin<&mut AppController>) -> QUrl;

        /// Answers and delivers the control-socket commands received since the last call.
        #[qinvokable]
        fn poll_control(self: Pin<&mut AppController>);

        /// Help → About Qt.
        #[qinvokable]
        fn about_qt(self: &AppController);

        /// Help → About Termigator (`QMessageBox::about`-style, application icon).
        #[qinvokable]
        fn about_termigator(self: &AppController, title: &QString);
    }

    impl cxx_qt::Threading for AppController {}
    impl cxx_qt::Initialize for AppController {}
}

/// The layout and the merged config: prepared by `run()` before the QApplication exists, then
/// shared by every QObject through [`with_app`].
pub struct AppState {
    pub paths: Paths,
    pub cfg: Value,
    /// Pid of the last external terminal started for follow mode.
    pub last_konsole_pid: Option<u32>,
}

impl AppState {
    /// `merge_defaults(load_config())` for the active layout (ADR-13).
    pub fn load() -> Result<AppState, ConfigError> {
        let paths = app_paths();
        save::tighten_modes(&paths);
        termigator_core::remote::listing::clear_edit_cache();
        let cfg = merge_defaults_owned(save::load(&paths)?);
        Ok(AppState {
            paths,
            cfg,
            last_konsole_pid: None,
        })
    }
}

/// The config layout of the app (ADR-13).
pub(crate) fn app_paths() -> Paths {
    Paths::resolve(
        Layout::ACTIVE,
        &ProcessEnv,
        &source_root().unwrap_or_default(),
    )
}

thread_local! {
    static APP: RefCell<Option<AppState>> = const { RefCell::new(None) };
}

/// Hands the loaded config to the QObjects that `ShellWindow.qml` creates.
pub fn hand_over(state: AppState) {
    APP.with(|s| *s.borrow_mut() = Some(state));
}

/// Runs `f` on the shared config and layout (loaded here when `run()` did not, e.g. in tests).
/// `f` must not call into QML or `with_app` again: the state is borrowed while it runs.
pub(crate) fn with_app<R>(f: impl FnOnce(&mut AppState) -> R) -> R {
    APP.with(|s| {
        let mut s = s.borrow_mut();
        f(s.get_or_insert_with(|| {
            AppState::load().unwrap_or_else(|_| AppState {
                paths: app_paths(),
                cfg: merge_defaults(&Value::Map(Vec::new())),
                last_konsole_pid: None,
            })
        }))
    })
}

const THEME_ICON_NAME: &str = "termigator";

/// Where the tray icon comes from: the installed theme icon first, then the build tree's file
/// (development run), else nothing (QML falls back to `utilities-terminal`).
#[derive(Debug, PartialEq, Eq)]
enum TrayIcon {
    Theme(&'static str),
    File(PathBuf),
    /// The PNG compiled into the binary (build.rs): an installed build without the hicolor
    /// theme icon, e.g. on Windows.
    Bundled,
}

const BUNDLED_TRAY_ICON: &str = "qrc:/qt/qml/Termigator/qml/assets/icons/termigator-64.png";

fn choose_tray_icon(theme_has_icon: bool, dev_file: &Path) -> TrayIcon {
    if theme_has_icon {
        TrayIcon::Theme(THEME_ICON_NAME)
    } else if dev_file.is_file() {
        TrayIcon::File(dev_file.to_path_buf())
    } else {
        TrayIcon::Bundled
    }
}

/// The source tree of a development build, the base of `assets/` there. A release build has
/// none: the building machine's path must not end up in the binary.
#[cfg(debug_assertions)]
fn source_root() -> Option<PathBuf> {
    Some(Path::new(env!("CARGO_MANIFEST_DIR")).join("../.."))
}

#[cfg(not(debug_assertions))]
fn source_root() -> Option<PathBuf> {
    None
}

pub struct AppControllerRust {
    control: Option<ControlServer>,
    version: QString,
    actions_json: QString,
    shortcut_rows_json: QString,
    first_run_message: QString,
    about_text: QString,
    tray_icon_url: QUrl,
    tray_icon_name: QString,
    smoke_test_shot: QString,
}

impl Default for AppControllerRust {
    fn default() -> Self {
        let first_run = with_app(|app| first_run_message(&app.paths)).unwrap_or_default();
        let version = env!("CARGO_PKG_VERSION");
        let tray = choose_tray_icon(
            qobject::has_theme_icon(&QString::from(THEME_ICON_NAME)),
            &source_root()
                .map(|r| r.join("assets/termigator-icon.png"))
                .unwrap_or_default(),
        );
        let shot = std::env::var_os(SMOKE_TEST_ENV).unwrap_or_default();
        let resolve_icon = |names: &str| qobject::theme_icon(&QString::from(names)).to_string();
        Self {
            first_run_message: QString::from(first_run.as_str()),
            control: None,
            version: QString::from(version),
            actions_json: QString::from(actions_json(&resolve_icon).as_str()),
            shortcut_rows_json: QString::from(shortcut_rows_json().as_str()),
            about_text: QString::from(
                about_html(version, &qobject::qt_version().to_string()).as_str(),
            ),
            tray_icon_url: match &tray {
                TrayIcon::File(icon) => {
                    QUrl::from_local_file(&QString::from(icon.to_string_lossy().as_ref()))
                }
                TrayIcon::Bundled => QUrl::from(BUNDLED_TRAY_ICON),
                TrayIcon::Theme(_) => QUrl::default(),
            },
            tray_icon_name: match tray {
                TrayIcon::Theme(name) => QString::from(name),
                _ => QString::default(),
            },
            smoke_test_shot: QString::from(shot.to_string_lossy().as_ref()),
        }
    }
}

impl cxx_qt::Initialize for qobject::AppController {
    fn initialize(mut self: Pin<&mut Self>) {
        // The source tree's assets in a development build, else the copy compiled into the app
        // (an installed build without the hicolor theme icon, e.g. on Windows).
        let assets = match source_root().map(|r| r.join("assets")) {
            Some(dir) if dir.is_dir() => dir.to_string_lossy().into_owned(),
            _ => ":/qt/qml/Termigator/qml/assets".to_owned(),
        };
        qobject::set_app_icon(&QString::from(assets.as_str()));
        // No private runtime directory, no socket; a second Termigator leaves it to the
        // first; a failure to listen only costs `--toggle` (ignored the same way).
        let control = control_socket_path(&ProcessEnv)
            .and_then(|path| ControlServer::start(&path).ok().flatten());
        self.as_mut().rust_mut().control = control;
        // The smoke test must not register a component in the user's KGlobalAccel.
        if self.smoke_test_shot.is_empty() {
            let thread = self.qt_thread();
            termigator_term_bridge::register_toggle_shortcut(Box::new(move || {
                let _ = thread.queue(|c| c.remote_command(QString::from("toggle")));
            }));
        }
    }
}

impl qobject::AppController {
    fn follow_enabled(&self) -> bool {
        let found = which("xdotool", std::env::var("PATH").ok().as_deref()).is_some();
        with_app(|app| follow_wanted(&app.cfg, found))
    }

    fn follow_poll_ms(&self) -> i32 {
        with_app(|app| follow_interval_ms(&app.cfg))
    }

    fn follow_tick(&self, frame_width: i32) {
        use std::sync::atomic::{AtomicBool, Ordering};
        static BUSY: AtomicBool = AtomicBool::new(false);
        if BUSY.swap(true, Ordering::AcqRel) {
            return;
        }
        let pid = with_app(|app| app.last_konsole_pid).unwrap_or(0);
        let thread = self.qt_thread();
        std::thread::spawn(move || {
            let target = follow_window(pid).and_then(|geo| follow_target(geo, frame_width));
            if let Some((x, y)) = target {
                let _ = thread.queue(move |c| c.follow_move(x, y));
            }
            BUSY.store(false, Ordering::Release);
        });
    }

    fn window_bool(&self, key: &QString, fallback: bool) -> bool {
        with_app(|app| window_bool(&app.cfg, &key.to_string(), fallback))
    }

    fn window_int(&self, key: &QString, fallback: i32) -> i32 {
        let v = with_app(|app| window_int(&app.cfg, &key.to_string(), i64::from(fallback)));
        i32::try_from(v).unwrap_or(fallback)
    }

    fn environment_color(&self, name: &QString) -> QString {
        QString::from(with_app(|app| environments::color(&app.cfg, &name.to_string())).as_str())
    }

    fn set_window_bool(&self, key: &QString, value: bool) {
        let key = key.to_string();
        with_app(|app| set_in(&mut app.cfg, &["window", &key], Value::Bool(value)));
    }

    fn update_check_url(&self) -> QString {
        // Tests point this at a local file; it also lifts the smoke-test and once-a-day limits.
        let forced = std::env::var(UPDATE_URL_ENV).ok().filter(|u| !u.is_empty());
        if forced.is_none() && std::env::var_os(SMOKE_TEST_ENV).is_some() {
            return QString::default();
        }
        let url = with_app(|app| {
            if !window_bool(&app.cfg, "check_updates", true) {
                return None;
            }
            let dir = &app.paths.config_dir;
            let mut state = update_check::State::load(dir);
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_secs());
            if forced.is_none() && !state.due(now) {
                return None;
            }
            state.checked_at = now;
            if let Err(e) = state.save(dir) {
                eprintln!(
                    "termigator: update check: cannot save {}: {e}",
                    update_check::STATE_FILE
                );
            }
            forced.clone().or_else(update_check::latest_release_url)
        });
        QString::from(url.unwrap_or_default().as_str())
    }

    fn update_check_result(&self, body: &QString) -> QString {
        let skipped = with_app(|app| update_check::State::load(&app.paths.config_dir).skipped);
        let found =
            update_check::newer_release(&body.to_string(), env!("CARGO_PKG_VERSION"), &skipped);
        QString::from(
            found
                .map(|r| json!({"version": r.version, "url": r.url}).to_string())
                .unwrap_or_default()
                .as_str(),
        )
    }

    fn skip_update(&self, version: &QString) {
        with_app(|app| {
            let dir = &app.paths.config_dir;
            let mut state = update_check::State::load(dir);
            state.skipped = version.to_string();
            if let Err(e) = state.save(dir) {
                eprintln!(
                    "termigator: update check: cannot save {}: {e}",
                    update_check::STATE_FILE
                );
            }
        });
    }

    fn persist(mut self: Pin<&mut Self>) -> bool {
        // New plain-text passwords are encrypted first. Without
        // the key in memory they stay out of the file and `seal_needed` makes the UI ask.
        let seal = with_app(|app| vault_qobj::seal_pending(&mut app.cfg));
        match with_app(|app| save_config(&app.paths, &app.cfg)) {
            Ok(()) => {
                self.as_mut().config_changed();
                if seal == vault_qobj::Seal::NeedsUnlock {
                    self.as_mut().seal_needed();
                }
                true
            }
            Err(e) => {
                let text = format!("Could not save config.yaml: {e}");
                self.as_mut().warning(QString::from(text.as_str()));
                false
            }
        }
    }

    fn save_window_state(&self, width: i32, height: i32, list_width: i32) {
        with_app(|app| {
            save_window_state(&mut app.cfg, width, height, list_width);
            let _ = save_config(&app.paths, &app.cfg); // closing anyway — never block exit
        });
    }

    fn request_restart(&self) {
        RESTART.store(true, std::sync::atomic::Ordering::SeqCst);
    }

    fn set_window_placement(&self, x: i32, y: i32, with_position: bool, maximized: bool) {
        with_app(|app| {
            set_window_placement(&mut app.cfg, with_position.then_some((x, y)), maximized);
        });
    }

    fn close_question(&self, sessions: i32, transfers: i32, tunnels: i32) -> QString {
        let q = with_app(|app| close_question(&app.cfg, sessions, transfers, tunnels))
            .unwrap_or_default();
        QString::from(q.as_str())
    }

    fn close_action(&self) -> QString {
        QString::from(with_app(|app| close_action::configured(&app.cfg)).as_str())
    }

    fn set_close_action(&self, action: &QString) {
        if let Some(action) = CloseAction::parse(&action.to_string()) {
            with_app(|app| close_action::set(&mut app.cfg, action));
        }
    }

    fn close_choice_details(&self, sessions: i32, transfers: i32, tunnels: i32) -> QString {
        QString::from(close_choice_details(sessions, transfers, tunnels).as_str())
    }

    fn quick_completions(&self) -> QStringList {
        string_list(&with_app(|app| quick_completions(&app.cfg)))
    }

    fn quick_history(&self) -> QStringList {
        string_list(&with_app(|app| quick_history::entries(&app.cfg)))
    }

    fn forget_quick_target(&self, target: &QString) {
        with_app(|app| quick_history::forget(&mut app.cfg, &target.to_string()));
    }

    fn clear_quick_history(&self) {
        with_app(|app| quick_history::clear(&mut app.cfg));
    }

    fn dialog_size(&self, name: &QString) -> QSize {
        let (w, h) = with_app(|app| dialog_size(&app.cfg, &name.to_string())).unwrap_or((-1, -1));
        QSize::new(w, h)
    }

    fn set_dialog_size(&self, name: &QString, width: i32, height: i32) {
        let name = name.to_string();
        with_app(|app| set_dialog_size(&mut app.cfg, &name, width, height));
    }

    fn config_folder_url(mut self: Pin<&mut Self>) -> QUrl {
        let dir = with_app(|app| {
            std::path::absolute(&app.paths.config_dir)
                .unwrap_or_else(|_| app.paths.config_dir.clone())
        });
        if !dir.is_dir() {
            let text = format!("Config folder does not exist:\n{}", dir.display());
            self.as_mut().warning(QString::from(text.as_str()));
            return QUrl::default();
        }
        QUrl::from_local_file(&QString::from(dir.to_string_lossy().as_ref()))
    }

    fn poll_control(mut self: Pin<&mut Self>) {
        let requests: Vec<_> = match &self.control {
            Some(server) => server.requests().try_iter().collect(),
            None => return,
        };
        for req in requests {
            let command = QString::from(req.command.as_str());
            // Before the window's requestActivate() (`XDG_ACTIVATION_TOKEN`).
            termigator_term_bridge::set_activation_token(&req.token);
            req.respond(true);
            self.as_mut().remote_command(command);
        }
    }

    fn about_qt(&self) {
        qobject::about_qt_box();
    }

    fn about_termigator(&self, title: &QString) {
        qobject::about_termigator_box(title, self.about_text());
    }
}

// ---- Pure logic (unit-tested below) ------------------------------------------------------

fn save_config(paths: &Paths, cfg: &Value) -> Result<(), ConfigError> {
    save::save(paths, cfg, vault::is_configured(section(cfg, "vault")))
}

fn key(text: &str) -> Value {
    Value::Str(text.to_owned())
}

/// `cfg[path[0]][path[1]]… = value`, creating (or replacing non-mapping) sections on the way.
pub(crate) fn set_in(cfg: &mut Value, path: &[&str], value: Value) {
    let Some((last, parents)) = path.split_last() else {
        return;
    };
    let mut node = cfg;
    for name in parents {
        node = map_entry(node, name);
        if !matches!(node, Value::Map(_)) {
            *node = Value::Map(Vec::new());
        }
    }
    *map_entry(node, last) = value;
}

/// The slot for `name` in the mapping `node` (inserted as `None` when missing); `node` becomes a
/// mapping if it is not one.
fn map_entry<'a>(node: &'a mut Value, name: &str) -> &'a mut Value {
    if !matches!(node, Value::Map(_)) {
        *node = Value::Map(Vec::new());
    }
    let Value::Map(pairs) = node else {
        unreachable!("made a mapping just above");
    };
    let pos = match pairs.iter().position(|(k, _)| *k == key(name)) {
        Some(pos) => pos,
        None => {
            pairs.push((key(name), Value::Null));
            pairs.len() - 1
        }
    };
    &mut pairs[pos].1
}

fn remove_key(cfg: &mut Value, name: &str) {
    if let Value::Map(pairs) = cfg {
        pairs.retain(|(k, _)| *k != key(name));
    }
}

/// `follow_konsole` is set and `xdotool` is available.
pub fn follow_wanted(cfg: &Value, xdotool_found: bool) -> bool {
    window_bool(cfg, "follow_konsole", false) && xdotool_found
}

/// `follow_poll_ms` (default 500); a QTimer interval below 1 ms would spin.
pub fn follow_interval_ms(cfg: &Value) -> i32 {
    i32::try_from(window_int(cfg, "follow_poll_ms", 500).max(1)).unwrap_or(500)
}

/// Below the terminal window (2 px gap), horizontally centred on
/// it but never left of its edge. `None` when the numbers do not fit an `i32`.
pub fn follow_target(geo: (i64, i64, i64, i64), frame_width: i32) -> Option<(i32, i32)> {
    let (x, y, w, h) = geo;
    let dx = (w - i64::from(frame_width)).div_euclid(2).max(0);
    Some((i32::try_from(x + dx).ok()?, i32::try_from(y + h + 2).ok()?))
}

pub fn window_bool(cfg: &Value, name: &str, default: bool) -> bool {
    section(cfg, "window").get(name).map_or(default, truthy)
}

pub fn window_int(cfg: &Value, name: &str, default: i64) -> i64 {
    section(cfg, "window")
        .get(name)
        .filter(|v| truthy(v))
        .and_then(value_int)
        .unwrap_or(default)
}

/// Set by `requestRestart`: `main` starts a new process after the event loop ends.
pub static RESTART: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// The command-line flag of that new process: reopen `last_session` once, whatever
/// `window.session_restore` says.
pub const RESTORE_ARG: &str = "--restore-session";

/// Whether `last_session` is written on exit: `window.session_restore`, or a restart.
pub fn keep_session(cfg: &Value) -> bool {
    RESTART.load(std::sync::atomic::Ordering::SeqCst) || window_bool(cfg, "session_restore", false)
}

/// Saves the window geometry plus the `last_session` part of the close handling.
pub fn save_window_state(cfg: &mut Value, width: i32, height: i32, list_width: i32) {
    if !keep_session(cfg) {
        remove_key(cfg, "last_session");
    }
    set_in(cfg, &["window", "width"], Value::Int(width.into()));
    set_in(cfg, &["window", "height"], Value::Int(height.into()));
    if list_width > 0 {
        set_in(
            cfg,
            &["window", "list_width"],
            Value::Int(list_width.into()),
        );
    }
}

/// `window.maximized`, and `window.x` / `window.y` of the normal geometry when `pos` is given
/// (Windows: the window manager does not place a window there; Wayland ignores positions).
pub fn set_window_placement(cfg: &mut Value, pos: Option<(i32, i32)>, maximized: bool) {
    set_in(cfg, &["window", "maximized"], Value::Bool(maximized));
    if let Some((x, y)) = pos {
        set_in(cfg, &["window", "x"], Value::Int(x.into()));
        set_in(cfg, &["window", "y"], Value::Int(y.into()));
    }
}

/// On close: what is still running, when `window.confirm_close` asks about it.
pub fn close_question(cfg: &Value, sessions: i32, transfers: i32, tunnels: i32) -> Option<String> {
    if (sessions <= 0 && transfers <= 0 && tunnels <= 0) || !window_bool(cfg, "confirm_close", true)
    {
        return None;
    }
    Some(running_lines(sessions, transfers, tunnels) + ".\nClose the window and end them?")
}

/// The close dialog's details: what Exit ends and Hide in Tray keeps; "" when nothing runs.
pub fn close_choice_details(sessions: i32, transfers: i32, tunnels: i32) -> String {
    if sessions <= 0 && transfers <= 0 && tunnels <= 0 {
        return String::new();
    }
    format!(
        "{}.\nExit ends them; in the tray they keep running.",
        running_lines(sessions, transfers, tunnels)
    )
}

/// One line per kind of work still running (none for a zero count).
fn running_lines(sessions: i32, transfers: i32, tunnels: i32) -> String {
    let mut what = Vec::new();
    if sessions > 0 {
        what.push(format!("{sessions} embedded session(s) are still running"));
    }
    if transfers > 0 {
        what.push(format!(
            "{transfers} file transfer(s) are still in progress"
        ));
    }
    if tunnels > 0 {
        what.push(format!("{tunnels} tunnel(s) will be stopped"));
    }
    what.join("\n")
}

fn string_list(items: &[String]) -> QStringList {
    items.iter().map(|s| QString::from(s.as_str())).collect()
}

/// Quick-connect completions: history then labels, without duplicates.
pub fn quick_completions(cfg: &Value) -> Vec<String> {
    let labels = match cfg.get("connections") {
        Some(Value::Seq(conns)) => conns
            .iter()
            .filter(|c| matches!(c, Value::Map(_)))
            .map(entry_label)
            .collect(),
        _ => Vec::new(),
    };
    let mut out: Vec<String> = Vec::new();
    for s in quick_history::entries(cfg).into_iter().chain(labels) {
        if !out.contains(&s) {
            out.push(s);
        }
    }
    out
}

pub fn dialog_size(cfg: &Value, name: &str) -> Option<(i32, i32)> {
    let entry = section(cfg, "window").get("dialog_sizes")?.get(name)?;
    let dim = |k| {
        entry
            .get(k)
            .and_then(value_int)
            .and_then(|v| i32::try_from(v).ok())
            .filter(|v| *v > 0)
    };
    Some((dim("width")?, dim("height")?))
}

pub fn set_dialog_size(cfg: &mut Value, name: &str, width: i32, height: i32) {
    let size = Value::Map(vec![
        (key("width"), Value::Int(width.into())),
        (key("height"), Value::Int(height.into())),
    ]);
    set_in(cfg, &["window", "dialog_sizes", name], size);
}

/// The welcome shown when neither config file exists yet.
pub fn first_run_message(paths: &Paths) -> Option<String> {
    if paths.config_path.is_file() {
        return None;
    }
    Some(format!(
        "Welcome to Termigator.\n\nYour connections and settings will be saved in {}.\n\n\
         Add a connection with the + button above the host list (or File → New connection), or \
         bring your existing sessions in with File → Import (MobaXterm, PuTTY, Remmina, \
         ~/.ssh/config).",
        paths.config_dir.display(),
    ))
}

/// Help → About Termigator (the toolkit line names what Rust uses).
pub fn about_html(version: &str, qt: &str) -> String {
    let repo = env!("CARGO_PKG_REPOSITORY");
    format!(
        "<h3>Termigator {version}</h3><p>SSH, RDP, VNC, Telnet and serial sessions with an embedded \
         terminal: a session manager for Linux and Windows.</p>\
         <p>Qt {qt} · Ghostty terminal engine · Rust</p><p>MIT License</p>\
         <p><a href=\"{repo}\">{repo}</a></p>"
    )
}

fn context_name(c: Context) -> &'static str {
    match c {
        Context::Window => "window",
        Context::HostList => "hostList",
        Context::Terminal => "terminal",
        Context::DetachedWindow => "detachedWindow",
        Context::DisplayOnly => "displayOnly",
    }
}

/// The action table for `ShellActions.qml`: `{menus: [...], menuIcons: {menu: icon}, actions: [...]}`;
/// `icon` resolves `a|b`.
pub fn actions_json(icon: &dyn Fn(&str) -> String) -> String {
    let list: Vec<_> = actions::ACTIONS
        .iter()
        .map(|a| {
            json!({
                "id": a.id,
                "menu": a.menu,
                "text": a.text,
                "icon": if a.icon.is_empty() { String::new() } else { icon(a.icon) },
                "keys": a.keys,
                "context": context_name(a.context),
                "sep": a.sep_before,
                "checkable": a.checkable,
                "submenu": a.submenu,
                "tip": a.tip,
            })
        })
        .collect();
    let menu_icons: serde_json::Map<String, serde_json::Value> = actions::MENUS
        .iter()
        .map(|m| ((*m).to_owned(), json!(icon(actions::menu_icon(m)))))
        .collect();
    json!({ "menus": actions::MENUS, "menuIcons": menu_icons, "actions": list }).to_string()
}

/// Help → Keyboard shortcuts: `[[menu, action, keys], …]`.
pub fn shortcut_rows_json() -> String {
    json!(actions::shortcut_rows()).to_string()
}

#[cfg(test)]
mod tests;
