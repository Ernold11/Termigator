//! Terminal tab core: `TerminalCore`, exposed to QML as `Termigator.TerminalCore`, one per
//! tab (created by `qml/terminal/TabSession.qml`).
//!
//! It owns the per-tab terminal state: the
//! process on its pty (`termigator_core::proc::pty`), the tapped output's side work
//! ([`SessionSink`]: distro sniff, OSC 7, session log, asciicast) and the session state machine
//! ([`termigator_core::session_state`]: startup commands, "Session stopped", auto-reconnect,
//! attention marks). The bridge's `ExternalSession` (QML) reads the pty master and draws it; its
//! events come back here through a router (one handler for all sessions, keyed by `sessionId`),
//! queued onto the GUI thread.
//!
//! The state machine is clock-free (ARCHITECTURE.md §5): every call passes the time since this
//! object was made; the one deadline it wants next goes to QML as `timerArm(ms)` (a single-shot
//! `Timer` that calls `tick()`), and what it wants done comes as `effects(json)`: a JSON array of
//! `{"k": kind, …}` objects ([`effect_json`]) that `TabSession.qml` carries out.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::time::{Duration, Instant};
#[cfg(not(unix))]
use termigator_core::proto::local_shell::windows_local_dir;

use cxx_qt::{CxxQtThread, CxxQtType, Threading};
use cxx_qt_lib::{QString, QStringList};
use serde_json::{Value as J, json};
use termigator_core::config::doc::Value;
use termigator_core::config::model::{
    DEFAULT_SCROLLBACK_LINES, launch_embedded_int, section, strip_space, truthy, value_int,
    value_str,
};
use termigator_core::konsole::{
    XdgDirs, choose_color_scheme, default_appearance, effective_font_size, pick_terminal_font,
};
use termigator_core::proc::pty::{
    Pty, SessionSink, SessionSpec as PtySpec, ShellCommand, SinkEvent, Size, save_output,
};
use termigator_core::session_state::{Effect, Event, Session, SessionSpec, StopKey};
use termigator_core::text::osc7::OSC7_SHELL_SNIPPET;
use termigator_core::text::session_log::{DEFAULT_TEMPLATE, LocalTime, render_log_path};
use termigator_core::vault::Secret;
use termigator_term_bridge::{PtyEvents, set_pty_events};

use crate::platform;
use crate::shell_qobj::with_app;

#[cxx_qt::bridge]
pub mod qobject {
    unsafe extern "C++" {
        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;
        include!("cxx-qt-lib/qstringlist.h");
        type QStringList = cxx_qt_lib::QStringList;
    }

    #[namespace = "termigator"]
    unsafe extern "C++" {
        include!("termigator/src/bell_native.h");
        /// The terminal bell's sound (Plasma's beep, Windows' default beep).
        #[rust_name = "play_bell"]
        fn playBell();
    }

    #[namespace = "termigator::terminal"]
    unsafe extern "C++" {
        include!("termigator/src/terminal_native.h");
        #[rust_name = "local_stamp"]
        fn localStamp() -> i64;
        #[rust_name = "installed_families"]
        fn installedFamilies() -> QStringList;
    }

    #[auto_cxx_name]
    extern "RustQt" {
        #[qobject]
        #[qml_element]
        type TerminalCore = super::TerminalCoreRust;

        /// What the tab has to do ([`super::effect_json`]): a JSON array, in order.
        #[qsignal]
        fn effects(self: Pin<&mut TerminalCore>, json: QString);

        /// The process exited (before the effects that follow from it).
        #[qsignal]
        fn exited(self: Pin<&mut TerminalCore>);

        /// The remote distro was recognised in the output (once): a canonical id.
        #[qsignal]
        fn distro_detected(self: Pin<&mut TerminalCore>, id: QString);

        /// The shell reported its directory with OSC 7.
        #[qsignal]
        fn directory_changed(self: Pin<&mut TerminalCore>, path: QString);

        /// Arm the single-shot timer that calls `tick()` in `ms`; a negative value stops it.
        #[qsignal]
        fn timer_arm(self: Pin<&mut TerminalCore>, ms: i32);

        /// Starts the process on a pty and hands it to the `ExternalSession` whose `sessionId`
        /// is `request.session_id`. `request` (JSON): `session_id`, `inner`, `env` (object of
        /// strings), `conn` (the entry or null), `title`, `attempt` (auto-reconnect attempts so
        /// far), `carry_log` (a running log to keep appending to, or ""), `cols`, `rows`.
        /// Returns "" or the error to show (a critical message box).
        #[qinvokable]
        fn start(self: Pin<&mut TerminalCore>, request: &QString) -> QString;

        /// Reaps the child if it exited (QML calls it every second, and the bridge's EOF does).
        #[qinvokable]
        fn poll(self: Pin<&mut TerminalCore>);

        /// The timer armed by `timerArm` fired.
        #[qinvokable]
        fn tick(self: Pin<&mut TerminalCore>);

        /// A key on the "Session stopped" overlay: `key` is `r`, `enter` or `s`; `modifier` is
        /// Ctrl, Alt or Meta held. True when the overlay took it.
        #[qinvokable]
        fn overlay_key(self: Pin<&mut TerminalCore>, key: &QString, modifier: bool) -> bool;

        /// Restart outside the overlay.
        #[qinvokable]
        fn restart_requested(self: Pin<&mut TerminalCore>);

        /// The tab became the current tab of an active window.
        #[qinvokable]
        fn tab_seen(self: Pin<&mut TerminalCore>);

        /// The terminal rang the bell / reported activity; the tab is current / its window active.
        #[qinvokable]
        fn bell(self: Pin<&mut TerminalCore>, current_tab: bool, window_active: bool);
        #[qinvokable]
        fn activity(self: Pin<&mut TerminalCore>, current_tab: bool, window_active: bool);

        /// Starts (appends to) a session log; "" or the error text.
        #[qinvokable]
        fn start_log(self: Pin<&mut TerminalCore>, path: &QString) -> QString;

        /// Stops the session log (nothing when none runs).
        #[qinvokable]
        fn stop_log(self: Pin<&mut TerminalCore>);

        /// The running log's path, or "".
        #[qinvokable]
        fn log_path(self: &TerminalCore) -> QString;

        /// Starts an asciicast recording; "" or the error text.
        #[qinvokable]
        fn start_recording(
            self: Pin<&mut TerminalCore>,
            path: &QString,
            title: &QString,
        ) -> QString;

        /// Finishes the recording.
        #[qinvokable]
        fn stop_recording(self: Pin<&mut TerminalCore>);

        /// The recording's path, or "".
        #[qinvokable]
        fn recording_path(self: &TerminalCore) -> QString;

        /// "Save terminal output": writes `text`; "" or the error text.
        #[qinvokable]
        fn save_output_text(self: &TerminalCore, path: &QString, text: &QString) -> QString;

        /// Kills a live process, closes the log and the recording and drops the session.
        #[qinvokable]
        fn close(self: Pin<&mut TerminalCore>);

        /// True while the process has not exited.
        #[qinvokable]
        fn is_running(self: &TerminalCore) -> bool;

        /// The working folder of a local shell; "" when it cannot be read. Linux reads
        /// `/proc/<pid>/cwd`; Windows maps `reported`, the folder the shell sent with OSC 7
        /// (cmd, PowerShell: `/C:\\dir`; WSL: a Linux path, under `\\\\wsl.localhost\\<distro>`).
        #[qinvokable]
        fn local_cwd(self: &TerminalCore, reported: &QString) -> QString;

        /// The OSC 7 prompt hook typed by "Enable folder tracking in this shell".
        #[qinvokable]
        fn osc7_snippet(self: &TerminalCore) -> QString;

        /// `_terminal_look` + the `launch` options of `_apply_tab_options` as JSON: `history`,
        /// `family`, `size`, `scheme` and the booleans `blinking_cursor`, `bracketed_paste`,
        /// `confirm_multiline`, `copy_on_select`, `paste_on_right_click`, `paste_on_middle_click`,
        /// `clipboard_write` (OSC 52), `attention`,
        /// `notify_on_disconnect`. `available` are the terminal's colour schemes, `dark` the UI
        /// palette.
        #[qinvokable]
        fn look(
            self: &TerminalCore,
            conn_json: &QString,
            available: &QStringList,
            dark: bool,
        ) -> QString;

        /// `_on_distro_detected`: stores `distro` in the saved entry equal to `conn_json`
        /// (not saved; call `persist`). True when something changed.
        #[qinvokable]
        fn remember_distro(self: &TerminalCore, conn_json: &QString, distro: &QString) -> bool;

        /// `$HOME/<name>`: the suggested file of a save dialog.
        #[qinvokable]
        fn home_file(self: &TerminalCore, name: &QString) -> QString;

        /// The suggested file of a new recording: `~/termigator-recordings/<title>_<date>_<time>.cast`.
        #[qinvokable]
        fn recording_default_path(self: &TerminalCore, title: &QString) -> QString;
    }

    impl cxx_qt::Threading for TerminalCore {}
}

type Handle = CxxQtThread<qobject::TerminalCore>;

thread_local! {
    /// `sessionId` -> the core to deliver the bridge's events to (GUI thread).
    static ROUTES: RefCell<HashMap<u64, Handle>> = RefCell::new(HashMap::new());
    static ROUTER_SET: RefCell<bool> = const { RefCell::new(false) };
}

/// Delivers the bridge's synchronous events to the owning core, queued on the GUI thread.
struct Router;

fn route(id: u64, f: impl FnOnce(Pin<&mut qobject::TerminalCore>) + Send + 'static) {
    ROUTES.with(|r| {
        if let Some(handle) = r.borrow().get(&id) {
            let _ = handle.queue(f); // Err: the QObject is gone — the tab closed
        }
    });
}

impl PtyEvents for Router {
    fn output(&mut self, id: u64, data: &[u8]) {
        let bytes = data.to_vec();
        route(id, move |c| c.on_output(&bytes));
    }

    fn resized(&mut self, id: u64, cols: u16, rows: u16) {
        route(id, move |c| c.on_resized(cols, rows));
    }

    fn eof(&mut self, id: u64) {
        route(id, |c| c.poll());
    }

    fn write(&mut self, id: u64, data: &[u8]) {
        // Feed mode only: the bridge has no descriptor to write to itself. Queued for the
        // session's writer thread, so the GUI thread never blocks on the pty.
        platform::write(id, data);
    }
}

/// Makes the bridge's events reach the cores; `run()` calls it once the application exists.
pub fn install_router() {
    if ROUTER_SET.with(|s| std::mem::replace(&mut *s.borrow_mut(), true)) {
        return;
    }
    set_pty_events(Box::new(Router));
}

pub struct TerminalCoreRust {
    id: u64,
    pty: Option<Pty>,
    sink: SessionSink,
    state: Option<Session>,
    base: Instant,
    size: Size,
    armed: Option<Duration>,
    has_exited: bool,
    /// The command line of the session, for [`windows_local_dir`] (which WSL distribution).
    inner: String,
}

impl Default for TerminalCoreRust {
    fn default() -> Self {
        Self {
            id: 0,
            pty: None,
            sink: SessionSink::new(),
            state: None,
            base: Instant::now(),
            size: Size { cols: 80, rows: 24 },
            armed: None,
            has_exited: false,
            inner: String::new(),
        }
    }
}

/// The JSON of one [`Effect`] for `TabSession.qml`.
pub fn effect_json(e: &Effect) -> J {
    match e {
        Effect::SendStartupCommands(t) => json!({"k": "startup", "v": t}),
        Effect::ScheduleReconnect { after, attempt } => {
            json!({"k": "schedule", "ms": after.as_millis() as u64, "attempt": attempt})
        }
        Effect::UpdateTitle(t) => json!({"k": "title", "v": t}),
        Effect::SetTooltip(t) => json!({"k": "tip", "v": t}),
        Effect::SetIcon(i) => json!({"k": "icon", "v": i.theme_name().unwrap_or("")}),
        Effect::SetLogMarker(on) => json!({"k": "logmarker", "v": on}),
        Effect::ShowStoppedOverlay { text } => json!({"k": "overlay", "v": text}),
        Effect::HideStoppedOverlay => json!({"k": "overlay_hide"}),
        Effect::Restart { log } => json!({"k": "restart", "log": log}),
        Effect::Reconnect { attempt, log } => {
            json!({"k": "reconnect", "attempt": attempt, "log": log})
        }
        Effect::CloseTab => json!({"k": "close"}),
        Effect::SaveOutput => json!({"k": "save"}),
        Effect::AlertWindow => json!({"k": "alert"}),
        Effect::RearmActivity => json!({"k": "rearm"}),
        Effect::NotifyDisconnect { label, lived } => {
            json!({"k": "notify", "label": label, "lived": lived.as_secs_f64()})
        }
    }
}

fn local_time() -> LocalTime {
    let n = qobject::local_stamp();
    let part = |div: i64, modulo: i64| u32::try_from(n / div % modulo).unwrap_or(0);
    LocalTime {
        year: part(10_000_000_000, 100_000),
        month: part(100_000_000, 100),
        day: part(1_000_000, 100),
        hour: part(10_000, 100),
        minute: part(100, 100),
        second: part(1, 100),
    }
}

/// OS error text in the `[Errno 13] Permission denied: '/path'` style.
fn os_error_text(e: &std::io::Error, path: &Path) -> String {
    match e.raw_os_error() {
        Some(n) => {
            let text = std::io::Error::from_raw_os_error(n).to_string();
            let text = text.split(" (os error").next().unwrap_or(&text);
            format!("[Errno {n}] {text}: '{}'", path.display())
        }
        None => e.to_string(),
    }
}

/// The `launch` mapping of the shared config.
fn launch_options() -> Value {
    with_app(|app| section(&app.cfg, "launch").clone())
}

fn opt_bool(lo: &Value, key: &str, default: bool) -> bool {
    lo.get(key).map_or(default, truthy)
}

/// `str(v or "").strip() or None`.
fn text_opt(v: Option<&Value>) -> Option<String> {
    let t = strip_space(&v.filter(|v| truthy(v)).map(value_str).unwrap_or_default()).to_owned();
    (!t.is_empty()).then_some(t)
}

/// The terminal look (appearance and terminal options) as JSON.
/// `installed` are the installed font families, `available` the terminal's colour schemes.
pub fn look_json(
    lo: &Value,
    conn: Option<&Value>,
    available: &[String],
    dark: bool,
    installed: &HashSet<String>,
    xdg: &XdgDirs,
) -> J {
    // An entry's own font / scheme on top of the settings.
    let own = |k: &str| {
        conn.and_then(|c| c.get(k))
            .filter(|v| truthy(v))
            .or_else(|| lo.get(k))
    };
    let mut history = launch_embedded_int(
        lo,
        &["embedded_pyte_history", "pyte_history"],
        DEFAULT_SCROLLBACK_LINES,
    );
    if matches!(lo.get("scrollback_unlimited"), Some(Value::Bool(true))) {
        history = -1; // a negative size = unlimited (file-backed) history
    }
    let configured_size = own("font_size")
        .filter(|v| !matches!(v, Value::Null))
        .and_then(value_int);
    let konsole = default_appearance(xdg);
    let family = text_opt(own("font_family")).or(konsole.font_family.clone());
    let scheme = text_opt(own("color_scheme")).or(konsole.color_scheme.clone());
    // One setting for every tab (not the entry's own scheme): termigator_core::terminal_background.
    let background = termigator_core::terminal_background::configured(lo, xdg);
    json!({
        "history": history,
        "family": pick_terminal_font(family.as_deref(), installed),
        "size": effective_font_size(configured_size, konsole.font_size),
        "scheme": choose_color_scheme(available, scheme.as_deref(), dark).unwrap_or_default(),
        "blinking_cursor": opt_bool(lo, "blinking_cursor", false),
        "bracketed_paste": opt_bool(lo, "bracketed_paste", true),
        "confirm_multiline": opt_bool(lo, "confirm_multiline_paste", true),
        "copy_on_select": opt_bool(lo, "copy_on_select", false),
        "paste_on_right_click": opt_bool(lo, "paste_on_right_click", false),
        "paste_on_middle_click": opt_bool(lo, "paste_on_middle_click", true),
        "clipboard_write": opt_bool(lo, "allow_clipboard_write", true),
        "attention": opt_bool(lo, "tab_attention_marks", true),
        "opacity": background.opacity as f64 / 100.0,
        "blur": background.blur,
    })
}

thread_local! {
    static LAST_BELL: std::cell::Cell<Option<Instant>> = const { std::cell::Cell::new(None) };
}

/// One bell sound per [`BELL_GAP`] at most: a shell that rings on every failed completion, or a
/// `cat` of a binary file, would otherwise play a burst. GUI thread only (thread-local).
const BELL_GAP: Duration = Duration::from_millis(250);

fn bell_due(now: Instant) -> bool {
    LAST_BELL.with(|last| {
        let due = last.get().is_none_or(|t| now.duration_since(t) >= BELL_GAP);
        if due {
            last.set(Some(now));
        }
        due
    })
}

/// Sets `distro` on the first saved connection equal to `conn`.
pub fn set_distro_in(cfg: &mut Value, conn: &Value, distro: &str) -> bool {
    let Value::Map(root) = cfg else { return false };
    let Some((_, Value::Seq(list))) = root
        .iter_mut()
        .find(|(k, _)| matches!(k, Value::Str(s) if s == "connections"))
    else {
        return false;
    };
    let Some(Value::Map(entry)) = list.iter_mut().find(|e| e.loose_eq(conn)) else {
        return false;
    };
    let new = Value::Str(distro.to_owned());
    match entry
        .iter_mut()
        .find(|(k, _)| matches!(k, Value::Str(s) if s == "distro"))
    {
        Some((_, v)) => *v = new,
        None => entry.push((Value::Str("distro".to_owned()), new)),
    }
    true
}

fn home_dir() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_dir())
        .unwrap_or_else(|| PathBuf::from("/"))
}

fn parse_json(invokable: &str, text: &QString) -> J {
    crate::qml_json::value(invokable, &text.to_string()).unwrap_or(J::Null)
}

fn str_of<'a>(j: &'a J, key: &str) -> &'a str {
    j.get(key).and_then(J::as_str).unwrap_or("")
}

fn dim(j: &J, key: &str, default: u16) -> u16 {
    j.get(key)
        .and_then(J::as_u64)
        .and_then(|n| u16::try_from(n).ok())
        .filter(|&n| n > 0)
        .unwrap_or(default)
}

impl TerminalCoreRust {
    fn now(&self) -> Duration {
        self.base.elapsed()
    }
}

impl qobject::TerminalCore {
    /// Feeds the state machine and emits what it asks for.
    fn dispatch(mut self: Pin<&mut Self>, event: Event<'_>, force_arm: bool) {
        let (effects, deadline, now) = {
            let r = self.as_mut().rust_mut().get_mut();
            let now = r.now();
            let Some(state) = r.state.as_mut() else {
                return;
            };
            (state.handle(now, event), state.next_deadline(), now)
        };
        if !effects.is_empty() {
            let list: Vec<J> = effects.iter().map(effect_json).collect();
            let text = J::Array(list).to_string();
            self.as_mut().effects(QString::from(text.as_str()));
        }
        if force_arm {
            self.as_mut().rust_mut().armed = None;
        }
        if self.armed != deadline {
            self.as_mut().rust_mut().armed = deadline;
            // +1: the timer rounds down, and an early tick would find the deadline still ahead.
            let ms = deadline.map_or(-1, |d| {
                i32::try_from(d.saturating_sub(now).as_millis() + 1).unwrap_or(i32::MAX)
            });
            self.as_mut().timer_arm(ms);
        }
    }

    fn on_output(mut self: Pin<&mut Self>, data: &[u8]) {
        let events = self.as_mut().rust_mut().sink.feed(data);
        self.as_mut().dispatch(Event::Output(data), false);
        for e in events {
            match e {
                SinkEvent::Distro(id) => self.as_mut().distro_detected(QString::from(id)),
                SinkEvent::Cwd(dir) => self.as_mut().directory_changed(QString::from(dir.as_str())),
            }
        }
    }

    fn on_resized(mut self: Pin<&mut Self>, cols: u16, rows: u16) {
        let r = self.as_mut().rust_mut().get_mut();
        r.size = Size { cols, rows };
        r.sink.resized(r.size);
        // Descriptor mode: the bridge sets TIOCSWINSZ itself; feed mode leaves the pty to us.
        if platform::feeds()
            && let Some(pty) = r.pty.as_ref()
        {
            let _ = pty.resize(r.size); // the child is gone, or the pty closed under us
        }
    }

    fn warn(mut self: Pin<&mut Self>, text: &str) {
        let list = J::Array(vec![json!({"k": "warn", "v": text})]).to_string();
        self.as_mut().effects(QString::from(list.as_str()));
    }

    fn start(mut self: Pin<&mut Self>, request: &QString) -> QString {
        let req = parse_json("start", request);
        let lo = launch_options();
        let id = req.get("session_id").and_then(J::as_u64).unwrap_or(0);
        let conn: Option<Value> = req
            .get("conn")
            .filter(|c| c.is_object())
            .map(Value::from_json);
        let term = lo
            .get("term")
            .filter(|v| truthy(v))
            .map(|v| strip_space(&value_str(v)).to_owned())
            .filter(|t| !t.is_empty())
            .unwrap_or_else(|| "xterm-256color".to_owned());
        let default_shell = std::env::var("SHELL")
            .ok()
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "/bin/bash".to_owned());
        #[cfg_attr(not(windows), allow(unused_mut))]
        let mut extra_env: Vec<(String, Secret)> = req
            .get("env")
            .and_then(J::as_object)
            .map(|m| {
                m.iter()
                    .filter_map(|(k, v)| {
                        v.as_str()
                            .map(|v| (k.clone(), Secret::new(v.as_bytes().to_vec())))
                    })
                    .collect()
            })
            .unwrap_or_default();
        // cmd.exe has no cwd hook except PROMPT: put an OSC 7 report in front of the user's own prompt
        // (cmd passes PROMPT on to its children, so it is often set already) so Files can follow the
        // shell. Other shells ignore PROMPT.
        #[cfg(windows)]
        if conn.is_none() {
            let base = std::env::var("PROMPT").unwrap_or_else(|_| "$P$G".to_owned());
            if !base.contains("]7;") {
                let prompt = format!("$E]7;file://localhost/$P$E\\{base}");
                extra_env.push(("PROMPT".to_owned(), Secret::new(prompt.into_bytes())));
            }
        }
        let size = Size {
            cols: dim(&req, "cols", 80),
            rows: dim(&req, "rows", 24),
        };
        let spec = PtySpec {
            command: ShellCommand::from_command_line(str_of(&req, "inner"), &default_shell),
            extra_env,
            term: Some(term),
            size,
            cwd: home_dir(),
        };
        let pty = match Pty::spawn_session(&spec, std::env::vars_os()) {
            Ok(p) => p,
            Err(e) => return QString::from(e.to_string().as_str()),
        };
        self.as_mut().rust_mut().inner = str_of(&req, "inner").to_owned();
        let title = str_of(&req, "title").to_owned();
        let startup = conn
            .as_ref()
            .and_then(|c| c.get("startup_commands"))
            .filter(|v| truthy(v))
            .map(value_str);
        let auto_reconnect = conn
            .as_ref()
            .and_then(|c| c.get("auto_reconnect"))
            .is_some_and(|v| matches!(v, Value::Bool(true)));
        let carry = str_of(&req, "carry_log").to_owned();
        let state_spec = SessionSpec {
            title: title.clone(),
            startup_commands: startup,
            auto_reconnect,
            has_connection: conn.is_some(),
            reconnect_attempt: req
                .get("attempt")
                .and_then(J::as_u64)
                .and_then(|n| u32::try_from(n).ok())
                .unwrap_or(0),
            log_path: None,
        };
        // Routes first: the bridge may deliver the first bytes right after attach_pty().
        let handle = self.qt_thread();
        ROUTES.with(|r| r.borrow_mut().insert(id, handle));
        let mut pty = pty;
        if let Err(e) = platform::attach(id, &mut pty) {
            ROUTES.with(|r| r.borrow_mut().remove(&id));
            platform::detach(id);
            let _ = pty.close();
            return QString::from(e.to_string().as_str());
        }
        {
            let r = self.as_mut().rust_mut().get_mut();
            r.id = id;
            r.size = size;
            r.pty = Some(pty);
            r.state = Some(Session::new(state_spec));
            r.base = Instant::now();
            r.has_exited = false;
        }
        self.as_mut().dispatch(Event::Spawned, false);
        // A remote session logs itself to a templated path, unless a log runs on from the
        // tab this one replaces.
        if !carry.is_empty() {
            let _ = self.as_mut().start_log(&QString::from(carry.as_str()));
        } else if let Some(c) = conn.as_ref()
            && opt_bool(&lo, "session_log_auto", false)
        {
            let template = value_str(lo.get("session_log_template").unwrap_or(&Value::Null));
            let template = if template.is_empty() {
                DEFAULT_TEMPLATE.to_owned()
            } else {
                template
            };
            let text_of = |k: &str| c.get(k).map(value_str).unwrap_or_default();
            let path = render_log_path(
                &template,
                &title,
                &text_of("host"),
                &text_of("user"),
                local_time(),
            );
            let err = self.as_mut().start_log(&QString::from(path.as_str()));
            if !err.is_empty() {
                let text = format!("Automatic session log could not start:\n{path}\n{}", err);
                self.as_mut().warn(&text);
            }
        }
        QString::default()
    }

    fn poll(mut self: Pin<&mut Self>) {
        let exited = {
            let r = self.as_mut().rust_mut().get_mut();
            if r.has_exited {
                return;
            }
            let Some(pty) = r.pty.as_mut() else { return };
            // A child we cannot ask about is as dead as one that exited.
            !matches!(pty.try_wait(), Ok(None))
        };
        if !exited {
            return;
        }
        let id = self.as_mut().rust_mut().id;
        if platform::feeds() {
            // ConPTY keeps its pipe open after the child exits, so the reader thread never reaches
            // EOF: end the session here, the way descriptor mode's EOF would, and drop the writer
            // so both feed threads finish instead of staying blocked on a dead pty.
            platform::detach(id);
            termigator_term_bridge::eof_ghostty_pty(id);
        }
        self.as_mut().rust_mut().has_exited = true;
        let notify = with_app(|app| {
            section(&app.cfg, "window")
                .get("notify_on_disconnect")
                .is_some_and(truthy)
        });
        self.as_mut().exited();
        self.as_mut().dispatch(
            Event::Exited {
                notify_on_disconnect: notify,
            },
            false,
        );
    }

    fn tick(self: Pin<&mut Self>) {
        self.dispatch(Event::Tick, true);
    }

    fn overlay_key(self: Pin<&mut Self>, key: &QString, modifier: bool) -> bool {
        let key = match key.to_string().as_str() {
            "r" => StopKey::R,
            "enter" => StopKey::Enter,
            "s" => StopKey::S,
            _ => return false,
        };
        if modifier {
            return false;
        }
        // The machine only reacts on a stopped session with its overlay up; the key is the
        // overlay's either way, so the terminal never sees R / Enter / S of a dead session.
        self.dispatch(Event::Key { key, modifier }, false);
        true
    }

    fn restart_requested(self: Pin<&mut Self>) {
        self.dispatch(Event::RestartRequested, false);
    }

    fn tab_seen(self: Pin<&mut Self>) {
        self.dispatch(Event::TabSeen, false);
    }

    fn bell(self: Pin<&mut Self>, current_tab: bool, window_active: bool) {
        let sound = with_app(|app| {
            let empty = Value::Null;
            opt_bool(app.cfg.get("launch").unwrap_or(&empty), "bell_sound", true)
        });
        if sound && bell_due(Instant::now()) {
            qobject::play_bell();
        }
        self.dispatch(
            Event::Bell {
                current_tab,
                window_active,
            },
            false,
        );
    }

    fn activity(self: Pin<&mut Self>, current_tab: bool, window_active: bool) {
        self.dispatch(
            Event::Activity {
                current_tab,
                window_active,
            },
            false,
        );
    }

    fn start_log(mut self: Pin<&mut Self>, path: &QString) -> QString {
        let lo = launch_options();
        let path_text = path.to_string();
        let p = PathBuf::from(&path_text);
        let abs = std::path::absolute(&p).unwrap_or_else(|_| p.clone());
        let started = self.as_mut().rust_mut().sink.start_log(
            &abs,
            opt_bool(&lo, "session_log_strip_ansi", true),
            opt_bool(&lo, "session_log_timestamps", false),
            Box::new(local_time),
        );
        match started {
            Ok(()) => {
                self.dispatch(Event::SessionLog(Some(path_text.as_str())), false);
                QString::default()
            }
            Err(e) => QString::from(os_error_text(&e, &abs).as_str()),
        }
    }

    fn stop_log(mut self: Pin<&mut Self>) {
        if self.sink.log_path().is_none() {
            return;
        }
        self.as_mut().rust_mut().sink.stop_log();
        self.dispatch(Event::SessionLog(None), false);
    }

    fn log_path(&self) -> QString {
        self.sink
            .log_path()
            .map(|p| QString::from(p.to_string_lossy().as_ref()))
            .unwrap_or_default()
    }

    fn start_recording(mut self: Pin<&mut Self>, path: &QString, title: &QString) -> QString {
        let p = PathBuf::from(path.to_string());
        let env = |k: &str, default: &str| {
            (
                k.to_owned(),
                std::env::var(k)
                    .ok()
                    .filter(|v| !v.is_empty())
                    .unwrap_or_else(|| default.to_owned()),
            )
        };
        let env = [env("TERM", "xterm-256color"), env("SHELL", "")];
        let r = self.as_mut().rust_mut().get_mut();
        match r.sink.start_recording(&p, r.size, &title.to_string(), &env) {
            Ok(()) => QString::default(),
            Err(e) => QString::from(os_error_text(&e, &p).as_str()),
        }
    }

    fn stop_recording(mut self: Pin<&mut Self>) {
        let _ = self.as_mut().rust_mut().sink.stop_recording();
    }

    fn recording_path(&self) -> QString {
        self.sink
            .recording_path()
            .map(|p| QString::from(p.to_string_lossy().as_ref()))
            .unwrap_or_default()
    }

    fn save_output_text(&self, path: &QString, text: &QString) -> QString {
        let p = PathBuf::from(path.to_string());
        match save_output(&p, text.to_string().as_bytes()) {
            Ok(()) => QString::default(),
            Err(e) => {
                let t = e.to_string();
                // Qt's QFile::errorString() has no "(os error N)".
                QString::from(t.split(" (os error").next().unwrap_or(&t))
            }
        }
    }

    fn close(mut self: Pin<&mut Self>) {
        let id = self.id;
        ROUTES.with(|r| r.borrow_mut().remove(&id));
        platform::detach(id);
        let disarm = {
            let r = self.as_mut().rust_mut().get_mut();
            r.sink.close();
            if let Some(mut pty) = r.pty.take() {
                let _ = pty.close(); // kills only our own child, only while it is alive
            }
            let now = r.base.elapsed();
            if let Some(state) = r.state.as_mut() {
                state.handle(now, Event::UserClosed);
            }
            r.has_exited = true;
            r.armed.take().is_some()
        };
        if disarm {
            self.timer_arm(-1);
        }
    }

    fn is_running(&self) -> bool {
        self.pty.is_some() && !self.has_exited
    }

    fn local_cwd(&self, reported: &QString) -> QString {
        // Windows: the child's directory is not ours to read, only what the shell reports.
        #[cfg(unix)]
        let cwd = {
            let _ = reported;
            self.pty
                .as_ref()
                .filter(|_| !self.has_exited)
                .and_then(|p| std::fs::read_link(format!("/proc/{}/cwd", p.pid())).ok())
                .map(|c| c.to_string_lossy().into_owned())
                .unwrap_or_default()
        };
        #[cfg(not(unix))]
        let cwd = windows_local_dir(&self.inner, &reported.to_string());
        QString::from(cwd.as_str())
    }

    fn osc7_snippet(&self) -> QString {
        QString::from(OSC7_SHELL_SNIPPET)
    }

    fn look(&self, conn_json: &QString, available: &QStringList, dark: bool) -> QString {
        let conn: Option<Value> = crate::qml_json::value("look", &conn_json.to_string())
            .filter(J::is_object)
            .map(|j| Value::from_json(&j));
        let available: Vec<String> = available.iter().map(|s| s.to_string()).collect();
        let installed: HashSet<String> = qobject::installed_families()
            .iter()
            .map(|s| s.to_string())
            .collect();
        let lo = launch_options();
        let mut j = look_json(
            &lo,
            conn.as_ref(),
            &available,
            dark,
            &installed,
            &XdgDirs::from_env(),
        );
        let notify = with_app(|app| {
            section(&app.cfg, "window")
                .get("notify_on_disconnect")
                .is_some_and(truthy)
        });
        j["notify_on_disconnect"] = J::Bool(notify);
        QString::from(j.to_string().as_str())
    }

    fn remember_distro(&self, conn_json: &QString, distro: &QString) -> bool {
        let Some(j) = crate::qml_json::value("remember_distro", &conn_json.to_string()) else {
            return false;
        };
        let conn = Value::from_json(&j);
        if conn
            .get("distro")
            .is_some_and(|d| value_str(d) == distro.to_string())
        {
            return false;
        }
        with_app(|app| set_distro_in(&mut app.cfg, &conn, &distro.to_string()))
    }

    fn home_file(&self, name: &QString) -> QString {
        let path = home_dir().join(name.to_string());
        QString::from(path.to_string_lossy().as_ref())
    }

    fn recording_default_path(&self, title: &QString) -> QString {
        let safe: String = title
            .to_string()
            .chars()
            .map(|c| {
                if c.is_alphanumeric() || "-_.".contains(c) {
                    c
                } else {
                    '_'
                }
            })
            .take(60)
            .collect();
        let safe = if safe.is_empty() { "session" } else { &safe };
        let t = local_time();
        let name = format!(
            "{safe}_{:04}-{:02}-{:02}_{:02}{:02}{:02}.cast",
            t.year, t.month, t.day, t.hour, t.minute, t.second
        );
        let path = home_dir().join("termigator-recordings").join(name);
        QString::from(path.to_string_lossy().as_ref())
    }
}

#[cfg(test)]
mod tests;
