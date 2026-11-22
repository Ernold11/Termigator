// One case is Unix-only (no external terminal on Windows): its imports are unused there.
#![cfg_attr(windows, allow(unused_imports))]
use super::*;
use crate::files_helper_qobj::scp_plan_json;
use crate::files_sync_qobj::plan_json;
use termigator_core::config::doc;
use termigator_core::remote::sync::{SyncAction, SyncOp};

fn entry(name: &str, is_dir: bool) -> RemoteEntry {
    RemoteEntry {
        name: name.as_bytes().to_vec(),
        is_dir,
        is_link: false,
        size: 1,
        mode: "-rw-r--r--".to_owned(),
        mtime: "2026-01-01 00:00".to_owned(),
        link_target: Vec::new(),
        owner: "u:g".to_owned(),
    }
}

#[test]
fn err_text_is_the_last_line_or_the_exit_code() {
    assert_eq!(
        err_text(b"warning\nPermission denied\n", 1),
        "Permission denied"
    );
    assert_eq!(err_text(b"  \n", 3), "failed (exit 3)");
    assert_eq!(err_text(b"", 124), "failed (exit 124)");
}

#[test]
fn clash_question_lists_ten_names_and_an_ellipsis() {
    let names: Vec<String> = (1..=12).map(|i| format!("f{i}")).collect();
    let q = clash_question(&names, "/srv");
    assert!(
        q.starts_with("12 item(s) already exist in /srv:\n  • f1\n"),
        "{q}"
    );
    assert!(
        q.contains("  • f10\n  …\n\nYes = overwrite them, No = skip them."),
        "{q}"
    );
    assert!(!q.contains("f11"), "{q}");
    let one = clash_question(&names[..1], "/srv");
    assert_eq!(
        one,
        "1 item(s) already exist in /srv:\n  • f1\n\nYes = overwrite them, No = skip them."
    );
}

#[test]
fn delete_questions_name_what_goes() {
    let (dir, file) = (entry("d", true), entry("f.txt", false));
    assert_eq!(
        delete_question_text(&[&dir]),
        "Delete remote directory (recursively!) “d”?\nThis cannot be undone."
    );
    assert_eq!(
        delete_question_text(&[&file]),
        "Delete remote file “f.txt”?\nThis cannot be undone."
    );
    assert_eq!(
        delete_question_text(&[&dir, &file]),
        "Delete 2 remote item(s) (1 folder(s), recursively!)?\n  • d/\n  • f.txt\nThis cannot be undone."
    );
    assert_eq!(
        delete_question_text(&[&file, &file]),
        "Delete 2 remote item(s)?\n  • f.txt\n  • f.txt\nThis cannot be undone."
    );
}

#[test]
fn listing_status_counts_shown_total_and_folders() {
    let entries = [entry("a", true), entry("b", false), entry(".c", false)];
    assert_eq!(
        listing_status("box", b"/srv", 2, &entries),
        "box · /srv — 2 shown (3 total, 1 dir(s))"
    );
}

#[test]
fn slot_makes_the_mappings_it_needs() {
    let mut cfg = Value::Map(Vec::new());
    set_in(
        slot(&mut cfg, "launch"),
        "files_local_dir",
        Value::Str("/x".into()),
    );
    set_in(
        slot(&mut cfg, "launch"),
        "files_local_dir",
        Value::Str("/y".into()),
    );
    let dir = cfg.get("launch").and_then(|l| l.get("files_local_dir"));
    assert_eq!(dir, Some(&Value::Str("/y".into())));
    // A value of another type is replaced by a mapping.
    let mut odd = Value::Map(vec![(Value::Str("launch".into()), Value::Int(3))]);
    set_in(slot(&mut odd, "launch"), "k", Value::Bool(true));
    assert_eq!(
        odd.get("launch").and_then(|l| l.get("k")),
        Some(&Value::Bool(true))
    );
}

#[test]
fn sync_plan_rows_and_messages_follow_golden() {
    let act = |op, rel: &str, size| SyncAction {
        op,
        rel: rel.as_bytes().to_vec(),
        reason: "why",
        size,
    };
    let (rows, level, text) = plan_json(&[], &[]);
    assert_eq!(
        (level, text.as_str()),
        ("positive", "Both folders are already in sync.")
    );
    assert_eq!(rows["rows"].as_array().map(Vec::len), Some(0));

    let plan = [
        act(SyncOp::Upload, "a.txt", 2048),
        act(SyncOp::MkdirRemote, "sub", 0),
        act(SyncOp::Conflict, "c.txt", 5),
    ];
    let (rows, level, text) = plan_json(&plan, &[]);
    assert_eq!(level, "warning");
    assert_eq!(
        text,
        "1 file(s) to copy, 1 conflict(s) left alone — untick what should not be copied."
    );
    let rows = rows["rows"].as_array().cloned().unwrap_or_default();
    assert_eq!(rows[0]["opLabel"], "↑ Upload");
    assert_eq!(rows[0]["size"], "2.0 K");
    assert_eq!(rows[1]["size"], "");
    assert_eq!(rows[2]["checkable"], false);

    // A copy over a newer file starts unticked and is counted in the message.
    let older = [SyncAction {
        reason: termigator_core::remote::sync::OLDER_HERE,
        ..act(SyncOp::Upload, "old.txt", 1)
    }];
    let (rows, level, text) = plan_json(&older, &[]);
    assert_eq!(level, "warning");
    assert_eq!(
        text,
        "1 file(s) to copy, 1 would replace a newer file (unticked) — untick what should not be copied."
    );
    let row = &rows["rows"][0];
    assert_eq!(
        (&row["checked"], &row["risky"], &row["replaces"]),
        (&J::Bool(false), &J::Bool(true), &J::Bool(true))
    );

    // Paths that leave the folder are reported, not silently dropped.
    let (_, level, text) = plan_json(&plan[..1], &[b"../x".to_vec()]);
    assert_eq!(level, "error");
    assert_eq!(
        text,
        "The server listed 1 path(s) that point outside the folder (e.g. '../x') — left out."
    );
}

// Windows has no external terminal (terminal_mode_for_entry), which this test relies on.
#[cfg(not(windows))]
#[test]
fn scp_plan_runs_in_the_terminal_and_pauses_an_external_one() {
    let cfg = Value::Map(Vec::new());
    let ssh = |extra: &str| {
        let text = format!("{{label: box, host: h, user: u{extra}}}");
        doc::load(&text).expect("yaml")
    };
    // An upload needs its file to exist.
    let file = std::env::temp_dir().join(format!("termigator-u9-scp {}", std::process::id()));
    std::fs::write(&file, "x").expect("write");
    let name = file.to_string_lossy().into_owned();
    let p = scp_plan_json(&cfg, &ssh(""), &name, " /srv/x ", true);
    let cmd = p["cmd"].as_str().unwrap_or_default();
    assert!(cmd.starts_with("scp "), "{p}");
    assert!(
        cmd.contains(&format!("'{name}'")) && cmd.ends_with("u@h:/srv/x"),
        "{cmd}"
    );
    assert_eq!(p["title"], "scp → box");
    assert_eq!(p["mode"], "embedded");

    let external = scp_plan_json(&cfg, &ssh(", terminal: external"), &name, "/srv/x", false);
    let cmd = external["cmd"].as_str().unwrap_or_default();
    assert!(
        cmd.ends_with("; printf '\\n[scp finished — press Enter to close]'; read -r _"),
        "{cmd}"
    );
    assert_eq!(external["title"], "scp ← box");

    // The builder's refusals are shown as they are.
    let bad = scp_plan_json(&cfg, &ssh(""), &name, "-x", true);
    assert_eq!(bad["error"], "Remote path must not start with '-'.");
    // Only SSH entries.
    let rdp = doc::load("{label: d, protocol: rdp, host: h}").expect("yaml");
    assert_eq!(scp_plan_json(&cfg, &rdp, &name, "/x", true), json!({}));
}

#[test]
fn upload_clash_names_are_the_dropped_names_the_folder_has() {
    let there = [entry("a.txt", false), entry("conf", true)];
    let paths = ["/tmp/x/a.txt", "/tmp/x/b.txt", "/home/u/conf"].map(String::from);
    assert_eq!(upload_clash_names(&paths, &there), ["a.txt", "conf"]);
    assert!(upload_clash_names(&paths, &[]).is_empty());
}
