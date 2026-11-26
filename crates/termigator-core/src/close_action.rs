//! What the main window's close button (and Alt+F4, and any other window-manager close request)
//! does: ask, hide the window in the system tray, or exit.
//!
//! Stored as `window.close_action` in config.yaml: `ask` (default), `tray` or `exit`. Older
//! configs only had the `window.close_to_tray` flag; while `close_action` is missing, a true flag
//! still means [`CloseAction::Tray`]. [`set`] writes the new key and drops the old one, so the two
//! never disagree. File → Quit and the tray's Exit always exit; this choice is only the close
//! button's.
//!
//! Tests: `tests/close_action.rs`.

use crate::config::doc::Value;
use crate::config::model::{section, strip_space, truthy, value_str};

/// The `window` key of the choice.
pub const KEY: &str = "close_action";
/// The `window` flag older configs used (true = [`CloseAction::Tray`]).
pub const LEGACY_KEY: &str = "close_to_tray";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CloseAction {
    /// The close dialog: Hide in Tray, Exit or Cancel.
    #[default]
    Ask,
    /// Hide the window; sessions, transfers and tunnels keep running.
    Tray,
    /// Quit, as File → Quit does.
    Exit,
}

impl CloseAction {
    /// The stored spelling.
    pub fn as_str(self) -> &'static str {
        match self {
            CloseAction::Ask => "ask",
            CloseAction::Tray => "tray",
            CloseAction::Exit => "exit",
        }
    }

    /// `ask` / `tray` / `exit` (any case, surrounding space ignored); `None` for anything else.
    pub fn parse(text: &str) -> Option<Self> {
        match strip_space(text).to_lowercase().as_str() {
            "ask" => Some(CloseAction::Ask),
            "tray" => Some(CloseAction::Tray),
            "exit" => Some(CloseAction::Exit),
            _ => None,
        }
    }
}

/// `window.close_action`; when it is missing or unknown, [`CloseAction::Tray`] for a true legacy
/// `window.close_to_tray`, else [`CloseAction::Ask`].
pub fn configured(cfg: &Value) -> CloseAction {
    let w = section(cfg, "window");
    if let Some(action) = w
        .get(KEY)
        .filter(|v| truthy(v))
        .and_then(|v| CloseAction::parse(&value_str(v)))
    {
        return action;
    }
    if w.get(LEGACY_KEY).is_some_and(truthy) {
        CloseAction::Tray
    } else {
        CloseAction::Ask
    }
}

/// Stores `action` as `window.close_action` (the key appended when new) and removes the legacy
/// `window.close_to_tray`. Nothing changes when `action` is already in effect, so a config that
/// never chose keeps its text as written.
pub fn set(cfg: &mut Value, action: CloseAction) {
    if configured(cfg) == action {
        return;
    }
    let key = |t: &str| Value::Str(t.to_owned());
    if !matches!(cfg, Value::Map(_)) {
        *cfg = Value::Map(Vec::new());
    }
    let Value::Map(top) = cfg else { return };
    let at = top
        .iter()
        .position(|(k, _)| *k == key("window"))
        .unwrap_or_else(|| {
            top.push((key("window"), Value::Null));
            top.len() - 1
        });
    if !matches!(top[at].1, Value::Map(_)) {
        top[at].1 = Value::Map(Vec::new());
    }
    let Value::Map(pairs) = &mut top[at].1 else {
        return;
    };
    pairs.retain(|(k, _)| *k != key(LEGACY_KEY));
    let value = Value::Str(action.as_str().to_owned());
    match pairs.iter_mut().find(|(k, _)| *k == key(KEY)) {
        Some((_, slot)) => *slot = value,
        None => pairs.push((key(KEY), value)),
    }
}
