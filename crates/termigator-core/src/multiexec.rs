//! MultiExec targets, broadcast line, per-target snippet expansion (no Qt, no I/O).
//!
//! Covers the broadcast line, the snippet run, the MultiExec targets and the
//! banner text. The caller describes every embedded/detached tab as a [`TabInfo`]; this module only
//! decides *which* tab ids receive input and *what* is sent. The C++ hub replays keys to the ids
//! returned by [`multiexec_targets`] through `termigator_term_bridge::set_multiexec_targets`.
//!
//! Rules: live embedded tabs that have a connection (so local shells never qualify), detached
//! windows included; MultiExec additionally honours "Exclude from MultiExec", the line broadcast
//! does not.

use std::collections::HashMap;

use crate::panes::TabId;
use crate::text::snippets::{Snippet, SnippetConn, expand, is_space, text_to_send};

pub use crate::text::snippets::questions;

/// Everything the target rules need to know about one terminal tab.
#[derive(Debug, Clone, Default)]
pub struct TabInfo {
    pub id: TabId,
    /// Display title (`termigator_title`); `?` in the broadcast listing when empty.
    pub title: String,
    /// An embedded terminal (not a Files/VNC/... tab).
    pub embedded: bool,
    /// The session has not finished.
    pub alive: bool,
    pub has_connection: bool,
    pub local_shell: bool,
    /// "Exclude from MultiExec" is ticked.
    pub excluded: bool,
    /// Lives in a detached window (still a valid target).
    pub detached: bool,
    pub conn: Option<SnippetConn>,
}

fn live_remote(t: &TabInfo) -> bool {
    t.embedded && t.alive && t.has_connection && !t.local_shell
}

/// Live embedded remote sessions not excluded by the user, in the given order.
pub fn multiexec_targets(tabs: &[TabInfo]) -> Vec<TabId> {
    tabs.iter()
        .filter(|t| live_remote(t) && !t.excluded)
        .map(|t| t.id)
        .collect()
}

/// Live remote sessions for the line broadcast (exclusion is ignored).
pub fn broadcast_targets(tabs: &[TabInfo]) -> Vec<&TabInfo> {
    tabs.iter().filter(|t| live_remote(t)).collect()
}

pub const BROADCAST_TITLE: &str = "Broadcast to remote sessions";
pub const BROADCAST_PROMPT: &str =
    "Text to send to every live remote session (newline is appended):";
pub const NO_LIVE_SESSIONS: &str = "No live remote sessions are open.";
pub const NO_SNIPPET_TARGET: &str = "Open an embedded terminal to send a snippet to.";

/// True for blank text (empty or only Unicode whitespace): the broadcast is silently dropped.
pub fn broadcast_is_blank(text: &str) -> bool {
    text.chars().all(is_space)
}

/// Confirmation question: count, the text, and up to 12 session names ("…" after that).
pub fn broadcast_confirmation(text: &str, targets: &[&TabInfo]) -> String {
    let mut listing = targets
        .iter()
        .take(12)
        .map(|t| {
            let n = if t.title.is_empty() { "?" } else { &t.title };
            format!("  • {n}")
        })
        .collect::<Vec<_>>()
        .join("\n");
    if targets.len() > 12 {
        listing.push_str("\n  …");
    }
    format!(
        "Send this line to {} session(s)?\n\n{text}\n\n{listing}",
        targets.len()
    )
}

/// The bytes sent to each target: the text with trailing newlines collapsed to a single one.
pub fn broadcast_payload(text: &str) -> String {
    format!("{}\n", text.trim_end_matches('\n'))
}

/// Banner shown while MultiExec is on.
pub fn multiexec_banner(count: usize) -> String {
    format!(
        "MultiExec — what you type goes to {count} remote session(s) \
         (Ctrl+Alt+M or Session → MultiExec to stop; tab menu: Exclude from MultiExec)"
    )
}

/// Snippet recipients: every MultiExec target when MultiExec is on, else the current tab if it
/// is an embedded terminal. Empty means "show [`NO_SNIPPET_TARGET`]".
pub fn snippet_targets(
    tabs: &[TabInfo],
    multiexec_on: bool,
    current: Option<TabId>,
) -> Vec<&TabInfo> {
    if multiexec_on {
        let ids = multiexec_targets(tabs);
        return tabs.iter().filter(|t| ids.contains(&t.id)).collect();
    }
    tabs.iter()
        .filter(|t| Some(t.id) == current && t.embedded)
        .collect()
}

/// What to type into each target: placeholders expanded per target (label falls back to the
/// tab title), questions answered once for all.
pub fn snippet_plan(
    snippet: &Snippet,
    answers: &HashMap<String, String>,
    targets: &[&TabInfo],
) -> Vec<(TabId, Vec<u8>)> {
    targets
        .iter()
        .map(|t| {
            let mut conn = t.conn.clone().unwrap_or_default();
            if conn.label.is_empty() {
                conn.label.clone_from(&t.title);
            }
            let text = text_to_send(&expand(&snippet.command, &conn, answers), snippet.enter);
            (t.id, text.into_bytes())
        })
        .collect()
}
