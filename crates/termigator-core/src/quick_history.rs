//! Quick-connect history: the targets that opened a tab from the toolbar field, newest first.
//!
//! Stored as the top-level `quick_history` list of config.yaml; `window.quick_history_limit`
//! (default [`DEFAULT_LIMIT`], 0 = keep none) caps it. A reused target moves to the front instead
//! of appearing twice. Credentials never go in: [`scrub`] drops anything shaped like
//! `user:password@host` or holding control characters, on the way in and when an old list is read.
//!
//! Tests: `tests/quick_history.rs`.

use crate::config::doc::Value;
use crate::config::model::{section, strip_space, value_int};

/// The config key of the list.
pub const KEY: &str = "quick_history";
/// The `window` key of the limit.
pub const LIMIT_KEY: &str = "quick_history_limit";
pub const DEFAULT_LIMIT: usize = 20;
/// The largest limit honoured (and offered by the preferences spin box).
pub const MAX_LIMIT: usize = 200;

/// `window.quick_history_limit`, clamped to `0..=MAX_LIMIT`; [`DEFAULT_LIMIT`] when missing or
/// not a number.
pub fn limit(cfg: &Value) -> usize {
    section(cfg, "window")
        .get(LIMIT_KEY)
        .and_then(value_int)
        .map_or(DEFAULT_LIMIT, |n| {
            usize::try_from(n.clamp(0, MAX_LIMIT as i64)).unwrap_or(DEFAULT_LIMIT)
        })
}

/// What of `target` may be stored: trimmed, or `None` when it is empty, holds control characters
/// or carries a password (`:` before the first `@`, as in `user:secret@host`).
pub fn scrub(target: &str) -> Option<String> {
    let t = strip_space(target);
    if t.is_empty() || t.chars().any(char::is_control) {
        return None;
    }
    if t.split_once('@')
        .is_some_and(|(user, _)| user.contains(':'))
    {
        return None;
    }
    Some(t.to_owned())
}

/// The stored history, newest first: scrubbed, without duplicates, at most [`limit`] entries.
pub fn entries(cfg: &Value) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    if let Some(Value::Seq(items)) = cfg.get(KEY) {
        for item in items {
            if let Value::Str(s) = item
                && let Some(s) = scrub(s)
                && !out.contains(&s)
            {
                out.push(s);
            }
        }
    }
    out.truncate(limit(cfg));
    out
}

/// Puts `target` first (moving it when already stored) and trims the list to [`limit`].
/// A target [`scrub`] refuses is not stored, but the rest of the list is still cleaned.
pub fn remember(cfg: &mut Value, target: &str) {
    let mut hist: Vec<String> = scrub(target).into_iter().collect();
    for h in entries(cfg) {
        if !hist.contains(&h) {
            hist.push(h);
        }
    }
    hist.truncate(limit(cfg));
    store(cfg, hist);
}

/// Removes `target` from the history.
pub fn forget(cfg: &mut Value, target: &str) {
    let want = strip_space(target);
    let hist = entries(cfg).into_iter().filter(|h| h != want).collect();
    store(cfg, hist);
}

/// Empties the history.
pub fn clear(cfg: &mut Value) {
    store(cfg, Vec::new());
}

/// `cfg[KEY] = hist`; `cfg` becomes a mapping if it is not one, the key is appended when new.
fn store(cfg: &mut Value, hist: Vec<String>) {
    let list = Value::Seq(hist.into_iter().map(Value::Str).collect());
    if !matches!(cfg, Value::Map(_)) {
        *cfg = Value::Map(Vec::new());
    }
    let Value::Map(pairs) = cfg else { return };
    let key = Value::Str(KEY.to_owned());
    match pairs.iter_mut().find(|(k, _)| *k == key) {
        Some((_, slot)) => *slot = list,
        None => pairs.push((key, list)),
    }
}
