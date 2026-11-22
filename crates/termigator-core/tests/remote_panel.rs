//! `termigator_core::remote::panel` logic:
//! target generations, DnD payloads, sorting, bookmarks, edit-in-place debounce.

use std::cell::Cell;
use std::path::{Path, PathBuf};

use termigator_core::config::doc::{self, Value};
use termigator_core::remote::listing::RemoteEntry;
use termigator_core::remote::panel::{
    DROP_STALE_MSG, EDIT_DEBOUNCE_MS, EditSession, Finish, Follow, PanelState, Row, SortColumn,
    add_bookmark, bookmarks, file_fingerprint, last_folder, local_drop_paths, parse_drag,
    remember_last, remove_bookmark,
};

fn conn(host: &str) -> Value {
    doc::load(&format!("host: {host}\nuser: u\nport: 22\n")).expect("yaml")
}

fn entry(name: &str, dir: bool, size: i64) -> RemoteEntry {
    RemoteEntry {
        name: name.as_bytes().to_vec(),
        is_dir: dir,
        is_link: false,
        size,
        mode: String::new(),
        mtime: String::new(),
        link_target: Vec::new(),
        owner: String::new(),
    }
}

fn listed(host: &str, cwd: &str, entries: Vec<RemoteEntry>) -> PanelState {
    let mut p = PanelState::new();
    p.set_target(Some(&conn(host)), b"");
    let t = p.begin_command().expect("idle panel");
    assert!(p.apply_listing(&t, cwd.into(), entries));
    p.finish_command(&t);
    p
}

fn names(p: &PanelState) -> Vec<String> {
    p.rows()
        .iter()
        .map(|r| match r {
            Row::Up => "..".to_owned(),
            Row::Entry(i) => String::from_utf8_lossy(&p.entries()[*i].name).into_owned(),
        })
        .collect()
}

#[test]
fn switching_target_during_a_listing_drops_the_stale_result_so_delete_acts_on_the_current_host() {
    let mut p = PanelState::new();
    p.set_target(Some(&conn("a")), b"");
    let t = p.begin_command().unwrap(); // listing of host A in flight
    p.set_target(Some(&conn("b")), b"");
    assert!(p.begin_command().is_none(), "panel still busy");
    assert!(!p.apply_listing(&t, b"/home/a".to_vec(), vec![entry("secret", false, 1)]));
    assert_eq!(
        p.finish_command(&t),
        Finish::Stale { relist: true },
        "current host must be listed now"
    );
    assert!(p.entries().is_empty() && p.cwd().is_empty());
    assert!(!p.accept(&t));
    // A delete issued now carries a ticket of host B only.
    let t2 = p.begin_command().unwrap();
    assert!(p.apply_listing(&t2, b"/home/b".to_vec(), vec![entry("x", false, 1)]));
    assert!(p.accept_listing(&t2));
    p.finish_command(&t2);
    assert!(p.accept(&p.ticket()), "a command started now is current");
    assert_eq!(p.host_key(), Some("u@b:22"));
}

#[test]
fn same_host_keeps_directory_and_generation_clearing_target_bumps() {
    let mut p = listed("a", "/srv", vec![]);
    let g = p.generation();
    assert!(!p.set_target(Some(&conn("a")), b""));
    assert_eq!((p.generation(), p.cwd()), (g, &b"/srv"[..]));
    p.set_target(None, b"");
    assert_eq!(p.generation(), g + 1);
    assert!(p.binding().is_none());
}

#[test]
fn post_upload_refresh_is_dropped_after_folder_change() {
    let mut p = listed("a", "/srv", vec![]);
    let job = p.ticket();
    assert!(p.accept(&job));
    let t = p.begin_command().unwrap();
    p.apply_listing(&t, b"/srv/sub".to_vec(), vec![]);
    p.finish_command(&t);
    assert!(!p.accept(&job), "folder changed");
}

#[test]
fn sync_stays_bound_to_the_host_it_was_opened_for() {
    let mut p = listed("a", "/srv", vec![]);
    let binding = p.binding().unwrap();
    p.set_target(Some(&conn("b")), b"");
    assert_eq!(binding.host_key, "u@a:22");
    assert_eq!(binding.conn, conn("a"));
    assert_eq!(p.binding().unwrap().host_key, "u@b:22");
    // compare survives the switch: the binding is a snapshot, not a view
    p.set_target(None, b"");
    assert_eq!(binding.conn.get("host"), Some(&Value::Str("a".into())));
}

#[test]
fn drag_payload_from_an_old_generation_is_rejected() {
    let mut p = listed("a", "/srv", vec![entry("f", false, 1), entry("d", true, 0)]);
    let (f, d) = (p.entries()[0].clone(), p.entries()[1].clone());
    let dot = entry("..", true, 0);
    let bytes = p.encode_drag(&[&f, &d, &dot]);
    let payload = parse_drag(&bytes).unwrap();
    assert_eq!(payload.entries.len(), 2, "'..' is never carried");
    assert_eq!(p.resolve_drop(&payload).unwrap().len(), 2);

    // same folder name on another host: generation differs
    p.set_target(Some(&conn("b")), b"");
    let t = p.begin_command().unwrap();
    p.apply_listing(&t, b"/srv".to_vec(), vec![entry("f", false, 1)]);
    assert_eq!(p.resolve_drop(&payload), Err(DROP_STALE_MSG));

    let mut q = listed("a", "/srv", vec![entry("f", false, 1)]);
    let old = parse_drag(&q.encode_drag(&[&entry("f", false, 1)])).unwrap();
    let t = q.begin_command().unwrap();
    q.apply_listing(&t, b"/other".to_vec(), vec![entry("f", false, 1)]);
    assert_eq!(q.resolve_drop(&old), Err(DROP_STALE_MSG), "folder changed");
    assert!(parse_drag(b"not json").is_none());
    assert!(parse_drag(br#"{"gen":1,"cwd":"zz","entries":[]}"#).is_none());
}

#[test]
fn drag_payload_keeps_non_utf8_names() {
    let bad = RemoteEntry {
        name: vec![b'c', 0xe9],
        ..entry("x", false, 1)
    };
    let p = listed("a", "/srv", vec![bad.clone()]);
    let payload = parse_drag(&p.encode_drag(&[&bad])).unwrap();
    assert_eq!(p.resolve_drop(&payload).unwrap(), vec![bad]);
}

#[test]
fn drop_of_non_local_urls_is_refused() {
    let yes = |_: &Path| true;
    assert_eq!(
        local_drop_paths(&["file:///tmp/a%20b/"], yes),
        vec![PathBuf::from("/tmp/a b")]
    );
    assert!(local_drop_paths(&["file:///tmp/a", "https://x/y"], yes).is_empty());
    assert!(local_drop_paths(&["file://other/tmp/a"], yes).is_empty());
    assert!(local_drop_paths(&["file:///tmp/gone"], |_| false).is_empty());
}

#[test]
fn sorting_table() {
    let es = vec![
        entry("b.txt", false, 5),
        entry("A.txt", false, 50),
        entry("zdir", true, 0),
        entry(".hid", false, 1),
        entry("adir", true, 0),
    ];
    let mut p = listed("a", "/srv", es);
    assert_eq!(names(&p), ["..", "adir", "zdir", ".hid", "A.txt", "b.txt"]);
    p.sort_by(SortColumn::Name); // flip
    assert_eq!(names(&p), ["..", "zdir", "adir", "b.txt", "A.txt", ".hid"]);
    p.sort_by(SortColumn::Size);
    assert_eq!(names(&p), ["..", "adir", "zdir", ".hid", "b.txt", "A.txt"]);
    p.sort_by(SortColumn::Size); // descending: dirs still first, ".." still pinned
    assert_eq!(names(&p), ["..", "zdir", "adir", "A.txt", "b.txt", ".hid"]);
    p.show_hidden = false;
    assert_eq!(names(&p), ["..", "zdir", "adir", "A.txt", "b.txt"]);
    let root = listed("a", "/", vec![entry("x", false, 1)]);
    assert_eq!(names(&root), ["x"], "no '..' at /");
}

#[test]
fn bookmarks_and_last_folder_round_trip() {
    let mut store = Value::Null;
    assert!(add_bookmark(&mut store, "u@a:22", b"/srv"));
    assert!(!add_bookmark(&mut store, "u@a:22", b"/srv"));
    assert!(add_bookmark(&mut store, "u@a:22", b"/etc"));
    remember_last(&mut store, "u@a:22", b"/etc");
    remember_last(&mut store, "u@b:22", b"/var");
    let back = doc::load(&doc::dump(&store)).unwrap();
    assert_eq!(
        bookmarks(&back, "u@a:22"),
        [b"/srv".to_vec(), b"/etc".to_vec()]
    );
    assert_eq!(last_folder(&back, "u@a:22"), b"/etc");
    assert_eq!(last_folder(&back, "u@b:22"), b"/var");
    assert!(bookmarks(&back, "u@b:22").is_empty());
    assert!(remove_bookmark(&mut store, "u@a:22", b"/srv"));
    assert!(!remove_bookmark(&mut store, "u@a:22", b"/srv"));
    assert_eq!(bookmarks(&store, "u@a:22"), [b"/etc".to_vec()]);
    assert_eq!(last_folder(&store, "u@a:22"), b"/etc", "last survives");
    // garbage in the store is tolerated
    let mut junk = doc::load("u@a:22: 5\n").unwrap();
    assert!(bookmarks(&junk, "u@a:22").is_empty());
    assert!(add_bookmark(&mut junk, "u@a:22", b"/x"));
}

#[test]
fn reopening_a_host_starts_in_its_last_folder_and_a_vanished_one_falls_back_to_home() {
    let mut store = Value::Null;
    remember_last(&mut store, "u@a:22", b"/gone");
    let mut p = PanelState::new();
    p.set_target(Some(&conn("a")), &last_folder(&store, "u@a:22"));
    assert_eq!(p.refresh_path(), b"/gone");
    let t = p.begin_command().unwrap();
    assert_eq!(p.listing_failed(&t, b"/gone"), Some(b".".to_vec()));
    p.finish_command(&t);
    assert_eq!(p.refresh_path(), b".");
}

#[test]
fn follow_directory_respects_toggle_and_busy() {
    let mut p = listed("a", "/srv", vec![]);
    assert_eq!(p.follow_directory(b"/x", false), Follow::Ignore);
    assert_eq!(p.follow_directory(b"/srv", true), Follow::Ignore);
    assert_eq!(
        p.follow_directory(b"/x", true),
        Follow::Navigate(b"/x".to_vec())
    );
    let t = p.begin_command().unwrap();
    assert_eq!(p.follow_directory(b"/x", true), Follow::Deferred);
    let Finish::Accepted { pending_follow } = p.finish_command(&t) else {
        panic!("accepted")
    };
    assert_eq!(
        p.follow_after(pending_follow.unwrap()),
        Some(b"/x".to_vec())
    );
}

fn session() -> EditSession {
    EditSession::new(
        PathBuf::from("/c/f"),
        conn("a"),
        b"/srv/f".to_vec(),
        Some((1, "a".into())),
    )
}

#[test]
fn edit_changes_within_debounce_make_one_upload() {
    let mut s = session();
    let fp = |_: &Path| Some((2u64, "b".to_owned()));
    for t in [0, 200, 400] {
        s.changed(t);
        assert!(s.poll(t + 100, fp).is_none(), "debounce restarts");
    }
    assert_eq!(s.next_due(), Some(400 + EDIT_DEBOUNCE_MS));
    assert!(s.poll(1099, fp).is_none());
    let up = s.poll(1100, fp).unwrap();
    assert_eq!((up.remote, up.conn), (b"/srv/f".to_vec(), conn("a")));
    assert!(s.poll(5000, fp).is_none(), "timer disarmed");
}

#[test]
fn edit_unchanged_fingerprint_or_vanished_file_is_not_uploaded() {
    let mut s = session();
    let calls = Cell::new(0);
    let same = |_: &Path| {
        calls.set(calls.get() + 1);
        Some((1u64, "a".to_owned()))
    };
    s.changed(0);
    assert!(s.poll(700, same).is_none());
    assert_eq!(calls.get(), 1);
    s.changed(1000);
    assert!(s.poll(1700, |_| None).is_none());
    // a later real change still uploads, and a repeat of it does not
    s.changed(2000);
    assert!(s.poll(2700, |_| Some((3, "c".into()))).is_some());
    s.changed(3000);
    assert!(s.poll(3700, |_| Some((3, "c".into()))).is_none());
}

#[test]
fn file_fingerprint_reads_size_and_sha1() {
    let dir = std::env::temp_dir().join(format!("termigator-panel-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let f = dir.join("f");
    std::fs::write(&f, b"abc").unwrap();
    assert_eq!(
        file_fingerprint(&f).unwrap(),
        (3, "a9993e364706816aba3e25717850c26c9cd0d89d".to_owned())
    );
    std::fs::remove_dir_all(&dir).unwrap();
    assert!(file_fingerprint(&f).is_none());
}

#[test]
fn failed_edit_upload_is_retried_with_the_same_content() {
    let mut s = session();
    let fp = |_: &Path| Some((3u64, "c".to_owned()));
    s.changed(0);
    assert!(s.poll(700, fp).is_some());
    assert!(
        s.poll(10_000, fp).is_none(),
        "nothing armed after a success"
    );
    s.upload_failed(1000);
    assert!(s.poll(1000 + 4999, fp).is_none(), "pause before the retry");
    assert!(
        s.poll(1000 + 5000, fp).is_some(),
        "same content is sent again"
    );
}
