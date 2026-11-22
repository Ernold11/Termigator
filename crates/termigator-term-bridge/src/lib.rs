//! Qt bridge: the terminal view, its session object and the application's QML types.
//!
//! Terminal emulation is Rust (`ghostty-vt`); this crate is the thin
//! C++/Qt layer around it, plus KDE integration. The QML names stay the application's own
//! (`QMLTermWidget`, `ExternalSession`), registered by [`register_ghostty_types`].
//!
//! External ptys (ARCHITECTURE.md §4.1, ADR-3): Rust spawns the process
//! (`termigator_core::proc::pty`), QML creates an `ExternalSession { sessionId: N }` and points one or
//! more `QMLTermWidget.session` at it, and [`attach_ghostty_pty`] hands the master fd to that
//! session. The session reads the fd with a `QSocketNotifier` on the GUI thread, feeds the engine
//! and calls the [`PtyEvents`] handler synchronously with every chunk, every size change and EOF.
//!
//! Views are disposable, sessions are not: to move a terminal (split, detach, attach), create the
//! new `QMLTermWidget` with `session` set at creation, then destroy the old view; never reassign
//! `session` on a live view (it stays connected to the old session). Destroy every view of a
//! session before the session. MultiExec: [`set_multiexec_targets`].
//!
//! The view's QML API: `findNext()` + `searchResult`, `selectedText()`,
//! `zoomIn/zoomOut/resetZoom()`, `hotspotUrlAt()` + `linkActivated` (Ctrl+click, scheme allowlist),
//! `confirmMultilinePaste` + `multilinePasteRequested` + `pasteText()`, `copyOnSelect`,
//! `pasteOnRightClick`, `pasteOnMiddleClick` (the mouse selection, or the app's own copy of it where
//! the platform has no primary selection; Shift overrides a program's mouse reporting),
//! `bracketedPasteDisabled`; a program's cursor style (DECSCUSR) is drawn from
//! the snapshot. Sessions: `historySize` (negative: unlimited), `history` (the whole scrollback as
//! text, for "Save terminal output"), `clear()`, `bell`, `activity` + `monitorActivity`, and the
//! built-in colour schemes (Konsole's directories are scanned by `termigator-core`). KDE:
//! [`register_toggle_shortcut`], [`set_activation_token`].

pub mod engine;
pub mod session;

use std::cell::RefCell;
#[cfg(unix)]
use std::os::fd::{AsRawFd, BorrowedFd};

use session::SessionEngine;

#[cxx::bridge(namespace = "termigator::term")]
mod ffi {
    unsafe extern "C++" {
        include!("termigator-term-bridge/src/register.h");
        include!("termigator-term-bridge/src/multiexec.h");
        include!("termigator-term-bridge/src/probe.h");
        include!("termigator-term-bridge/src/kde.h");
        include!("termigator-term-bridge/src/terminal_item.h");
        include!("termigator-term-bridge/src/ghostty_session.h");

        /// Registers the `GhosttySpike` and `GhosttySession` QML types for the spikes.
        #[rust_name = "register_spike_types"]
        fn registerSpikeTypes();

        /// Registers `QMLTermWidget`/`ExternalSession`/`MultiExec` on the Ghostty engine.
        #[rust_name = "register_ghostty_types"]
        fn registerGhosttyTypes();

        /// Registers the `Probe` singleton (synthetic platform key events, RSS) in the QML module
        /// `Termigator.Terminal.Probe` 1.0, for examples and headless checks; the app never does.
        #[rust_name = "register_probe_types"]
        fn registerProbeTypes();

        /// Attaches a pty master to a `GhosttySession` (the application's only session type).
        #[rust_name = "attach_ghostty_pty_fd"]
        fn attachPtyGhostty(id: u64, fd: i32) -> Result<()>;

        /// Puts a `GhosttySession` in feed mode: output arrives by [`feed_ghostty_pty`], input leaves
        /// through `PtyEvents::write`, resizes through `PtyEvents::resized`. GUI thread only.
        #[rust_name = "attach_ghostty_pty_feed_ffi"]
        fn attachPtyFeed(id: u64) -> Result<()>;

        /// Output of a feed-mode session's pty; any thread, delivered in order on the GUI thread.
        #[rust_name = "post_feed_pty"]
        fn postFeedPtyGhostty(id: u64, data: &[u8]);

        /// End of a feed-mode session's pty; any thread, delivered after the output posted before.
        #[rust_name = "post_eof_pty"]
        fn postEofPtyGhostty(id: u64);

        /// Sets the sessions MultiExec types into, by `sessionId`; an empty set turns it off. A
        /// key typed in a view of one of them is replayed into the others, each encoding it with
        /// its own Emulation modes (application cursor keys and the like); keys from sessions
        /// outside the set stay local. A replay never reaches its source again and never loops.
        /// GUI thread only.
        #[rust_name = "set_multiexec_targets"]
        fn setMultiExecTargets(ids: &[u64]);

        #[rust_name = "register_toggle_shortcut_ffi"]
        fn registerToggleShortcut() -> bool;

        /// Hands the Wayland activation token that came with `termigator --toggle`/`--show` to the
        /// windowing system: the next `requestActivate()` raises the window with it (on X11 it
        /// becomes the startup id). An empty token does nothing. GUI thread only.
        #[rust_name = "set_activation_token"]
        fn setActivationToken(token: &str);
    }

    extern "Rust" {
        fn on_output(id: u64, data: &[u8]);
        fn on_resize(id: u64, cols: u16, rows: u16);
        fn on_write(id: u64, data: &[u8]);
        fn on_eof(id: u64);
        fn on_toggle_shortcut();

        // One engine per terminal session.
        type SessionEngine;
        fn new_session_engine(cols: u16, rows: u16, scrollback: i64) -> Box<SessionEngine>;
        fn feed(self: &mut SessionEngine, data: &[u8]);
        fn resize(self: &mut SessionEngine, cols: u16, rows: u16);
        fn scroll(self: &mut SessionEngine, delta: i32);
        fn history_text(self: &SessionEngine) -> String;
        fn snapshot_json(self: &mut SessionEngine) -> String;
        fn snapshot_binary(self: &mut SessionEngine) -> Vec<u8>;
        fn events_json(self: &mut SessionEngine) -> String;
        fn scrollbar_json(self: &SessionEngine) -> String;
        fn set_history(self: &mut SessionEngine, lines: i64);
        fn clear(self: &mut SessionEngine);
        fn apply_konsole_default_scheme(self: &mut SessionEngine) -> String;
        fn set_scheme(self: &mut SessionEngine, name: &str) -> bool;
        fn set_scheme_text(self: &mut SessionEngine, text: &str) -> bool;
        fn select(
            self: &mut SessionEngine,
            start_col: u16,
            start_row: u16,
            end_col: u16,
            end_row: u16,
            rectangle: bool,
        );
        fn select_word(self: &mut SessionEngine, col: u16, row: u16);
        fn select_line(self: &mut SessionEngine, col: u16, row: u16);
        fn clear_selection(self: &mut SessionEngine);
        fn selected_text(self: &SessionEngine) -> String;
        fn paste(self: &SessionEngine, text: &str) -> Vec<u8>;
        fn paste_mode(self: &SessionEngine, text: &str, bracketed: bool) -> Vec<u8>;
        fn bracketed_paste(self: &SessionEngine) -> bool;
        fn paste_is_safe(text: &str) -> bool;
        fn find_next(
            self: &mut SessionEngine,
            needle: &str,
            case_sensitive: bool,
            regex: bool,
            forward: bool,
            current: bool,
        ) -> bool;
        fn url_at(self: &mut SessionEngine, col: u16, row: u16) -> String;
        fn available_schemes() -> Vec<String>;
        fn scrollbar_total(self: &SessionEngine) -> u32;
        fn scrollbar_offset(self: &SessionEngine) -> u32;
        fn scrollbar_len(self: &SessionEngine) -> u32;
        fn scroll_to_row(self: &mut SessionEngine, row: i32);
        fn encode_mouse(
            self: &mut SessionEngine,
            x: u16,
            y: u16,
            button: i32,
            action: i32,
            modifiers: u32,
            cell_size: u32,
        ) -> Vec<u8>;
        fn encode_qt_key(self: &mut SessionEngine, key: i32, modifiers: u32, text: &str)
        -> Vec<u8>;
    }
}

pub use ffi::{
    register_ghostty_types, register_probe_types, register_spike_types, set_activation_token,
    set_multiexec_targets,
};

fn paste_is_safe(text: &str) -> bool {
    SessionEngine::paste_is_safe(text)
}

fn available_schemes() -> Vec<String> {
    SessionEngine::available_schemes()
}

fn new_session_engine(cols: u16, rows: u16, scrollback: i64) -> Box<SessionEngine> {
    let scrollback = usize::try_from(scrollback).ok();
    let build = |cols, rows, scrollback| SessionEngine::new(cols, rows, scrollback);
    // A 1x1 terminal only fails on allocation failure; there is nothing sensible to draw then.
    let engine = build(cols.max(1), rows.max(1), scrollback)
        .or_else(|_| build(1, 1, None))
        .expect("session engine: terminal allocation failed");
    Box::new(engine)
}

/// Receives what `ExternalSession`s see, on the GUI thread, synchronously.
pub trait PtyEvents {
    /// Raw bytes as read from the master, before the emulation decodes them.
    fn output(&mut self, id: u64, data: &[u8]);
    /// The emulation changed the window size (TIOCSWINSZ already done).
    fn resized(&mut self, _id: u64, _cols: u16, _rows: u16) {}
    /// Read returned EOF or EIO: the child side is closed. Reap the child with `try_wait`.
    fn eof(&mut self, id: u64);
    /// Feed mode: the session wants these bytes written to the pty (queue them, never block).
    fn write(&mut self, _id: u64, _data: &[u8]) {}
}

thread_local! {
    static HANDLER: RefCell<Option<Box<dyn PtyEvents>>> = const { RefCell::new(None) };
    static TOGGLE: RefCell<Option<Box<dyn FnMut()>>> = const { RefCell::new(None) };
}

/// Installs the handler for all `ExternalSession`s; call on the GUI thread.
///
/// Events raised while the handler itself runs (it re-entered the event loop) are dropped.
pub fn set_pty_events(handler: Box<dyn PtyEvents>) {
    HANDLER.with(|h| *h.borrow_mut() = Some(handler));
}

fn with_handler(f: impl FnOnce(&mut dyn PtyEvents)) {
    HANDLER.with(|h| {
        if let Ok(mut guard) = h.try_borrow_mut()
            && let Some(handler) = guard.as_mut()
        {
            f(handler.as_mut());
        }
    });
}

fn on_output(id: u64, data: &[u8]) {
    with_handler(|h| h.output(id, data));
}

fn on_resize(id: u64, cols: u16, rows: u16) {
    with_handler(|h| h.resized(id, cols, rows));
}

fn on_eof(id: u64) {
    with_handler(|h| h.eof(id));
}

fn on_write(id: u64, data: &[u8]) {
    with_handler(|h| h.write(id, data));
}

/// Attaches a pty master to the live `GhosttySession` whose `sessionId` is `id`, replacing the pty
/// it had (a reconnect keeps the scrollback). The session duplicates `fd`, so the caller keeps
/// ownership of its own. Fails when no such session exists. GUI thread only.
#[cfg(unix)]
pub fn attach_ghostty_pty(id: u64, fd: BorrowedFd<'_>) -> Result<(), cxx::Exception> {
    ffi::attach_ghostty_pty_fd(id, fd.as_raw_fd())
}

// Without file descriptors (Windows) sessions only use feed mode; the bridge still declares the
// descriptor entry point, so name it here instead of an unused-function warning.
#[cfg(not(unix))]
const _: fn(u64, i32) -> Result<(), cxx::Exception> = ffi::attach_ghostty_pty_fd;

/// Puts the live `GhosttySession` whose `sessionId` is `id` in feed mode (no descriptor: a thread
/// reads the pty and calls [`feed_ghostty_pty`]). GUI thread only.
pub fn attach_ghostty_pty_feed(id: u64) -> Result<(), cxx::Exception> {
    ffi::attach_ghostty_pty_feed_ffi(id)
}

/// Delivers pty output to a feed-mode session; callable from any thread, in order.
pub fn feed_ghostty_pty(id: u64, data: &[u8]) {
    ffi::post_feed_pty(id, data);
}

/// Tells a feed-mode session its pty is closed; callable from any thread, after the output.
pub fn eof_ghostty_pty(id: u64) {
    ffi::post_eof_pty(id);
}

/// Registers the global "Show / hide Termigator" action with KGlobalAccel, with no default key (the
/// user assigns one in System Settings → Shortcuts), and calls `on_toggle` on every press. Call
/// once on the GUI thread after the `QGuiApplication` exists; a later call only replaces the
/// handler. Returns false when KGlobalAccel is unavailable (no kglobalaccel on the session bus).
pub fn register_toggle_shortcut(on_toggle: Box<dyn FnMut()>) -> bool {
    TOGGLE.with(|t| *t.borrow_mut() = Some(on_toggle));
    ffi::register_toggle_shortcut_ffi()
}

fn on_toggle_shortcut() {
    TOGGLE.with(|t| {
        if let Ok(mut guard) = t.try_borrow_mut()
            && let Some(handler) = guard.as_mut()
        {
            handler();
        }
    });
}
