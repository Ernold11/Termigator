//! MultiExec / broadcast / snippet fan-out rules.

use std::collections::HashMap;

use termigator_core::multiexec::*;
use termigator_core::text::snippets::{Snippet, SnippetConn};

fn tab(id: u64, title: &str) -> TabInfo {
    TabInfo {
        id,
        title: title.into(),
        embedded: true,
        alive: true,
        has_connection: true,
        conn: Some(SnippetConn {
            host: format!("h{id}"),
            user: "root".into(),
            ..Default::default()
        }),
        ..Default::default()
    }
}

fn mixed() -> Vec<TabInfo> {
    let mut dead = tab(2, "dead");
    dead.alive = false;
    let mut local = tab(3, "local");
    local.has_connection = false;
    local.local_shell = true;
    local.conn = None;
    let mut excl = tab(4, "excl");
    excl.excluded = true;
    let mut det = tab(5, "det");
    det.detached = true;
    let mut files = tab(6, "files");
    files.embedded = false;
    vec![tab(1, "a"), dead, local, excl, det, files]
}

#[test]
fn multiexec_rules() {
    assert_eq!(multiexec_targets(&mixed()), vec![1, 5]);
}

#[test]
fn test_broadcast_only_to_live_remote_sessions() {
    let tabs = mixed();
    let ids: Vec<_> = broadcast_targets(&tabs).iter().map(|t| t.id).collect();
    assert_eq!(ids, vec![1, 4, 5]); // exclusion does not apply
    assert!(broadcast_targets(&[]).is_empty());
}

#[test]
fn broadcast_texts() {
    let tabs: Vec<_> = (1..=13).map(|i| tab(i, &format!("s{i}"))).collect();
    let t = broadcast_targets(&tabs);
    let q = broadcast_confirmation("ls", &t);
    assert!(q.starts_with("Send this line to 13 session(s)?\n\nls\n\n  • s1\n"));
    assert!(q.ends_with("  • s12\n  …"));
    let one = tab(1, "");
    assert_eq!(
        broadcast_confirmation("x", &[&one]),
        "Send this line to 1 session(s)?\n\nx\n\n  • ?"
    );
    assert_eq!(broadcast_payload("reboot\n\n"), "reboot\n");
    assert!(broadcast_is_blank(" \t\n"));
    assert!(!broadcast_is_blank(" a "));
    assert!(
        multiexec_banner(2).starts_with("MultiExec — what you type goes to 2 remote session(s) (")
    );
}

fn sn(cmd: &str, enter: bool) -> Snippet {
    Snippet {
        name: "n".into(),
        command: cmd.into(),
        enter,
    }
}

fn ans(k: &str, v: &str) -> HashMap<String, String> {
    HashMap::from([(k.to_owned(), v.to_owned())])
}

#[test]
fn test_snippet_with_multiexec_fills_each_host() {
    let tabs = vec![tab(1, "a"), tab(2, "b")];
    let t = snippet_targets(&tabs, true, Some(1));
    let plan = snippet_plan(
        &sn("ssh {host} {label} {?who}", true),
        &ans("who", "me"),
        &t,
    );
    assert_eq!(
        plan,
        vec![
            (1, b"ssh h1 a me\n".to_vec()),
            (2, b"ssh h2 b me\n".to_vec())
        ]
    );
    assert_eq!(questions("{?who} {?who} {?x}"), vec!["who", "x"]);
}

#[test]
fn test_snippet_with_question_goes_to_current_tab() {
    let mut tabs = vec![tab(1, "a"), tab(2, "b")];
    tabs[1].embedded = false;
    assert_eq!(snippet_targets(&tabs, false, Some(1))[0].id, 1);
    assert!(snippet_targets(&tabs, false, Some(2)).is_empty());
    assert!(snippet_targets(&tabs, false, None).is_empty());
    let t = snippet_targets(&tabs, false, Some(1));
    let plan = snippet_plan(&sn("echo {?q}", false), &ans("q", "hi"), &t);
    assert_eq!(plan, vec![(1, b"echo hi".to_vec())]);
}

#[test]
fn test_cancelled_question_sends_nothing() {
    // Cancelling aborts before planning: no targets -> empty plan.
    assert!(snippet_plan(&sn("x", true), &HashMap::new(), &[]).is_empty());
}
