//! Keyboard management of the host tree: rename of an entry (F2) and the tree clipboard
//! (Ctrl+C / Ctrl+X / Ctrl+V of entries and folders), and that nothing is lost on the way.

use termigator_core::config::doc::{Value, load as yaml_load};
use termigator_core::hosttree::{
    Clip, ClipItem, EntryKind, EntryRef, TreeState, all_group_names, explicit_groups,
    group_environments, group_of, paste, rename_entry,
};

fn cfg(text: &str) -> Value {
    yaml_load(text).expect("yaml")
}

fn sample() -> Value {
    cfg(r#"
connections:
  - {label: web1, host: w1.lan, user: u, port: 2222, group: prod/web, password: enc-x}
  - {label: desk, protocol: rdp, host: d1.lan, group: prod, ssh_gateway: web1}
  - {label: solo, host: s1.lan}
favorites:
  - {label: fav, command: htop, group: prod}
groups: [prod/empty, lab]
group_environments: {prod/web: staging}
"#)
}

const fn conn(index: usize) -> EntryRef {
    EntryRef {
        kind: EntryKind::Connection,
        index,
    }
}

fn items(c: &Value, key: &str) -> Vec<Value> {
    match c.get(key) {
        Some(Value::Seq(v)) => v.clone(),
        _ => Vec::new(),
    }
}

fn s(v: &Value, key: &str) -> String {
    match v.get(key) {
        Some(Value::Str(s)) => s.clone(),
        _ => String::new(),
    }
}

/// `label@group` of every connection, in list order.
fn conns(c: &Value) -> Vec<String> {
    items(c, "connections")
        .iter()
        .map(|e| format!("{}@{}", s(e, "label"), group_of(e)))
        .collect()
}

#[test]
fn rename_changes_only_the_label_and_keeps_the_place() {
    let mut c = sample();
    let before = items(&c, "connections");
    assert_eq!(rename_entry(&mut c, conn(2), "  alone "), Ok(conn(2)));
    let after = items(&c, "connections");
    assert_eq!(s(&after[2], "label"), "alone");
    assert_eq!(s(&after[2], "host"), "s1.lan");
    assert_eq!(after.len(), before.len());
    assert_eq!(after[..2], before[..2]);
}

#[test]
fn rename_refuses_blank_names_and_names_taken_in_the_folder() {
    let mut c = sample();
    let before = c.clone();
    assert!(rename_entry(&mut c, conn(1), "   ").is_err());
    // The favorite "fav" is in the same folder as desk.
    let err = rename_entry(&mut c, conn(1), "fav").expect_err("taken");
    assert!(err.contains("“fav” already exists in “prod”"), "{err}");
    assert_eq!(c, before);
    // The same name in another folder is fine.
    assert!(rename_entry(&mut c, conn(2), "fav").is_ok());
}

#[test]
fn rename_of_a_gateway_keeps_the_entries_that_use_it() {
    let mut c = sample();
    rename_entry(&mut c, conn(0), "bastion").expect("renamed");
    let after = items(&c, "connections");
    assert_eq!(s(&after[0], "label"), "bastion");
    assert_eq!(s(&after[1], "ssh_gateway"), "bastion");
    // Another SSH entry called like the new name would take the gateway over: refused.
    let mut c = sample();
    let err = rename_entry(&mut c, conn(0), "solo").expect_err("ambiguous gateway");
    assert!(err.contains("SSH gateway"), "{err}");
    assert_eq!(s(&items(&c, "connections")[1], "ssh_gateway"), "web1");
}

#[test]
fn copy_of_an_entry_gets_a_free_name_in_the_destination() {
    let mut c = sample();
    let mut st = TreeState::default();
    let clip = Clip::new(&c, &[conn(2)], &[], false);
    let got = paste(&mut c, &mut st, &clip, "").expect("pasted");
    assert_eq!(got.entry, Some(conn(3)));
    let got = paste(&mut c, &mut st, &clip, "").expect("pasted again");
    assert_eq!(got.entry, Some(conn(4)));
    paste(&mut c, &mut st, &clip, "lab").expect("into lab");
    assert_eq!(
        conns(&c),
        [
            "web1@prod/web",
            "desk@prod",
            "solo@",
            "solo (copy)@",
            "solo (copy 2)@",
            "solo@lab"
        ]
    );
    // The copy is the whole entry, the original untouched; the clipboard can be pasted again.
    let all = items(&c, "connections");
    assert_eq!(s(&all[3], "host"), "s1.lan");
    assert!(!clip.is_empty());
}

#[test]
fn a_copy_pastes_the_entry_as_it_was_copied() {
    let mut c = sample();
    let mut st = TreeState::default();
    let clip = Clip::new(&c, &[conn(2)], &[], false);
    rename_entry(&mut c, conn(2), "renamed").expect("renamed");
    let got = paste(&mut c, &mut st, &clip, "lab").expect("pasted");
    assert_eq!(got.entry, Some(conn(3)));
    assert_eq!(conns(&c)[2..], ["renamed@", "solo@lab"]);
}

#[test]
fn copy_of_a_folder_takes_its_subtree() {
    let mut c = sample();
    let mut st = TreeState::default();
    let clip = Clip::new(&c, &[], &["prod".into()], false);
    let got = paste(&mut c, &mut st, &clip, "").expect("pasted");
    assert_eq!(got.folder.as_deref(), Some("prod (copy)"));
    assert_eq!(
        conns(&c),
        [
            "web1@prod/web",
            "desk@prod",
            "solo@",
            "web1@prod (copy)/web",
            "desk@prod (copy)"
        ]
    );
    let favs: Vec<String> = items(&c, "favorites").iter().map(group_of).collect();
    assert_eq!(favs, ["prod", "prod (copy)"]);
    assert!(explicit_groups(&c).contains(&"prod (copy)/empty".to_owned()));
    assert_eq!(
        group_environments(&c)
            .get("prod (copy)/web")
            .map(String::as_str),
        Some("staging")
    );
    // The copied RDP entry still uses web1 as its gateway, like the original.
    assert_eq!(s(&items(&c, "connections")[4], "ssh_gateway"), "web1");
    // Into another folder: the plain name.
    let got = paste(&mut c, &mut st, &clip, "lab").expect("into lab");
    assert_eq!(got.folder.as_deref(), Some("lab/prod"));
    assert!(all_group_names(&c).contains(&"lab/prod/web".to_owned()));
}

#[test]
fn a_folder_never_goes_into_itself() {
    for cut in [false, true] {
        let mut c = sample();
        let before = c.clone();
        let mut st = TreeState::default();
        let clip = Clip::new(&c, &[], &["prod".into()], cut);
        for dest in ["prod", "prod/web"] {
            let err = paste(&mut c, &mut st, &clip, dest).expect_err("cycle");
            assert!(err.contains("into itself"), "{err}");
        }
        assert_eq!(c, before);
    }
}

#[test]
fn cut_moves_keeping_the_place_and_every_setting() {
    let mut c = sample();
    let mut st = TreeState::default();
    let before = items(&c, "connections");
    let clip = Clip::new(&c, &[conn(0)], &[], true);
    let got = paste(&mut c, &mut st, &clip, "lab").expect("moved");
    assert_eq!(got.entry, Some(conn(0)));
    let after = items(&c, "connections");
    assert_eq!(after.len(), before.len());
    assert_eq!(group_of(&after[0]), "lab");
    for key in ["label", "host", "user", "password"] {
        assert_eq!(after[0].get(key), before[0].get(key), "{key}");
    }
    // The gateway reference still finds it.
    assert_eq!(s(&after[1], "ssh_gateway"), "web1");
}

#[test]
fn cut_folder_moves_and_refuses_a_name_taken_in_the_destination() {
    let mut c = sample();
    let mut st = TreeState::default();
    let clip = Clip::new(&c, &[], &["prod/web".into()], true);
    let got = paste(&mut c, &mut st, &clip, "lab").expect("moved");
    assert_eq!(got.folder.as_deref(), Some("lab/web"));
    assert_eq!(conns(&c)[0], "web1@lab/web");
    assert_eq!(
        group_environments(&c).get("lab/web").map(String::as_str),
        Some("staging")
    );
    // Another "web" in the way: refused, nothing changes.
    let mut c = cfg("groups: [a/web, b/web]\n");
    let before = c.clone();
    let clip = Clip::new(&c, &[], &["a/web".into()], true);
    let err = paste(&mut c, &mut st, &clip, "b").expect_err("taken");
    assert!(err.contains("“web” already exists in “b”"), "{err}");
    assert_eq!(c, before);
}

#[test]
fn a_cut_entry_that_changed_meanwhile_is_not_moved() {
    let mut c = sample();
    let mut st = TreeState::default();
    let clip = Clip::new(&c, &[conn(2)], &[], true);
    rename_entry(&mut c, conn(2), "other").expect("renamed");
    let before = c.clone();
    let err = paste(&mut c, &mut st, &clip, "lab").expect_err("gone");
    assert!(err.contains("“solo” no longer exists"), "{err}");
    assert_eq!(c, before);
    // An entry that only moved in its list is found again.
    let mut c = sample();
    let clip = Clip::new(&c, &[conn(2)], &[], true);
    let Some(Value::Seq(list)) = c.get("connections").cloned() else {
        panic!("connections")
    };
    let mut list = list;
    list.insert(0, cfg("{label: new, host: n.lan}"));
    if let Value::Map(pairs) = &mut c {
        for (k, v) in pairs.iter_mut() {
            if *k == Value::Str("connections".into()) {
                *v = Value::Seq(list.clone());
            }
        }
    }
    let got = paste(&mut c, &mut st, &clip, "lab").expect("found");
    assert_eq!(got.entry, Some(conn(3)));
    assert_eq!(conns(&c)[3], "solo@lab");
}

#[test]
fn the_clipboard_leaves_out_what_a_picked_folder_holds() {
    let c = sample();
    let clip = Clip::new(
        &c,
        &[conn(0), conn(2), conn(2)],
        &["prod".into(), "prod/web".into()],
        false,
    );
    let kinds: Vec<String> = clip
        .items
        .iter()
        .map(|i| match i {
            ClipItem::Folder(f) => format!("folder {f}"),
            ClipItem::Entry { at, .. } => format!("entry {}", at.index),
        })
        .collect();
    assert_eq!(kinds, ["folder prod", "entry 2"]);
    assert!(clip.holds_folder("prod") && !clip.holds_folder("prod/web"));
    assert!(clip.holds_entry(&c, conn(2)) && !clip.holds_entry(&c, conn(0)));
}
