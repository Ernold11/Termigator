//! Folder management of the host tree: rename, move, delete with the contents kept elsewhere,
//! folders that stay when they empty, and that no entry or setting is lost on the way.

use termigator_core::config::doc::{Value, dump, load as yaml_load};
use termigator_core::config::groups::map_group_on_relocate;
use termigator_core::hosttree::{
    EntryKind, EntryRef, FolderContents, TreeState, all_group_names, delete_folder_into,
    entry_moved, explicit_groups, folder_contents, folder_move_targets, folder_moved,
    group_environments, group_of, rename_folder, with_folders_kept,
};

fn cfg(text: &str) -> Value {
    yaml_load(text).expect("yaml")
}

fn sample() -> Value {
    cfg(r#"
connections:
  - {label: web1, host: w1.lan, user: u, port: 2222, group: prod/web, environment: production}
  - {label: db1, host: d1.lan, user: u, group: prod/db, ssh_gateway: web1}
  - {label: solo, host: s1.lan}
favorites:
  - {label: fav, command: htop, group: prod}
groups: [prod/empty, lab]
group_environments: {prod/web: staging}
"#)
}

fn groups(c: &Value) -> Vec<(String, String)> {
    ["connections", "favorites"]
        .iter()
        .flat_map(|k| match c.get(k) {
            Some(Value::Seq(items)) => items.clone(),
            _ => Vec::new(),
        })
        .map(|e| {
            let label = match e.get("label") {
                Some(Value::Str(s)) => s.clone(),
                _ => String::new(),
            };
            (label, group_of(&e))
        })
        .collect()
}

/// Every entry with its `group` removed: what must survive a folder operation untouched.
fn settings(c: &Value) -> Vec<String> {
    ["connections", "favorites"]
        .iter()
        .flat_map(|k| match c.get(k) {
            Some(Value::Seq(items)) => items.clone(),
            _ => Vec::new(),
        })
        .map(|e| match e {
            Value::Map(pairs) => Value::Map(
                pairs
                    .into_iter()
                    .filter(|(k, _)| *k != Value::Str("group".into()))
                    .collect(),
            )
            .to_json()
            .to_string(),
            other => other.to_json().to_string(),
        })
        .collect()
}

#[test]
fn relocate_maps_the_folder_and_its_subtree() {
    assert_eq!(map_group_on_relocate("prod", "prod", "lab"), "lab");
    assert_eq!(map_group_on_relocate("prod/web", "prod", "lab"), "lab/web");
    assert_eq!(map_group_on_relocate("prod/web", "prod", ""), "web");
    assert_eq!(
        map_group_on_relocate("production", "prod", "lab"),
        "production"
    );
    assert_eq!(map_group_on_relocate("x/prod", "prod", "lab"), "x/prod");
}

#[test]
fn rename_changes_only_the_last_segment() {
    let mut c = sample();
    let before = settings(&c);
    let mut st = TreeState::default();
    assert_eq!(
        rename_folder(&mut c, &mut st, "prod/web", "frontend"),
        Ok("prod/frontend".to_owned())
    );
    assert_eq!(
        groups(&c)[0],
        ("web1".to_owned(), "prod/frontend".to_owned())
    );
    assert_eq!(
        group_environments(&c)
            .get("prod/frontend")
            .map(String::as_str),
        Some("staging")
    );
    assert_eq!(settings(&c), before);
}

#[test]
fn rename_refuses_a_slash_a_blank_name_and_a_sibling() {
    let mut c = sample();
    let mut st = TreeState::default();
    let untouched = c.clone();
    assert!(rename_folder(&mut c, &mut st, "prod/web", "a/b").is_err());
    assert!(rename_folder(&mut c, &mut st, "prod/web", "a\\b").is_err());
    assert!(rename_folder(&mut c, &mut st, "prod/web", "  ").is_err());
    let e = rename_folder(&mut c, &mut st, "prod/web", "db").expect_err("sibling");
    assert!(e.contains("“db”") && e.contains("“prod”"), "{e}");
    assert_eq!(c, untouched);
    // The same name is no change.
    assert_eq!(
        rename_folder(&mut c, &mut st, "prod/web", " web "),
        Ok("prod/web".to_owned())
    );
    assert_eq!(c, untouched);
}

#[test]
fn move_targets_leave_out_the_folder_and_its_subtree() {
    let c = sample();
    assert_eq!(folder_move_targets(&c, "prod"), ["lab"]);
    assert_eq!(
        folder_move_targets(&c, "prod/web"),
        ["lab", "prod", "prod/db", "prod/empty"]
    );
}

#[test]
fn a_folder_cannot_move_below_itself() {
    let mut c = sample();
    let untouched = c.clone();
    let mut st = TreeState::default();
    assert!(folder_moved(&mut c, &mut st, "prod", "prod").is_err());
    assert!(folder_moved(&mut c, &mut st, "prod", "prod/web").is_err());
    assert_eq!(c, untouched);
}

#[test]
fn contents_count_entries_and_subfolders() {
    let c = sample();
    assert_eq!(
        folder_contents(&c, "prod"),
        FolderContents {
            entries: 3,
            folders: 3
        }
    );
    assert_eq!(folder_contents(&c, "lab"), FolderContents::default());
}

#[test]
fn delete_into_the_parent_keeps_every_entry() {
    let mut c = sample();
    let before = settings(&c);
    let mut st = TreeState::default();
    delete_folder_into(&mut c, &mut st, "prod", "").expect("ok");
    assert_eq!(
        groups(&c),
        [
            ("web1".to_owned(), "web".to_owned()),
            ("db1".to_owned(), "db".to_owned()),
            ("solo".to_owned(), String::new()),
            ("fav".to_owned(), String::new()),
        ]
    );
    assert_eq!(explicit_groups(&c), ["empty", "lab"]);
    assert_eq!(settings(&c), before);
}

#[test]
fn delete_into_another_folder_moves_the_subtree_there() {
    let mut c = sample();
    let before = settings(&c);
    let mut st = TreeState::default();
    delete_folder_into(&mut c, &mut st, "prod", "lab").expect("ok");
    assert_eq!(
        groups(&c),
        [
            ("web1".to_owned(), "lab/web".to_owned()),
            ("db1".to_owned(), "lab/db".to_owned()),
            ("solo".to_owned(), String::new()),
            ("fav".to_owned(), "lab".to_owned()),
        ]
    );
    assert_eq!(
        all_group_names(&c),
        ["lab", "lab/db", "lab/empty", "lab/web"]
    );
    assert_eq!(
        group_environments(&c).get("lab/web").map(String::as_str),
        Some("staging")
    );
    assert!(st.expanded_groups.contains("lab"));
    assert_eq!(settings(&c), before);
}

#[test]
fn delete_into_its_own_subtree_is_refused() {
    let mut c = sample();
    let untouched = c.clone();
    let mut st = TreeState::default();
    assert!(delete_folder_into(&mut c, &mut st, "prod", "prod").is_err());
    assert!(delete_folder_into(&mut c, &mut st, "prod", "prod/web").is_err());
    assert_eq!(c, untouched);
}

#[test]
fn a_folder_emptied_by_a_move_stays() {
    let mut c = cfg("connections:\n  - {label: a, host: a.lan, group: x/y}\n");
    let mut st = TreeState::default();
    let a = EntryRef {
        kind: EntryKind::Connection,
        index: 0,
    };
    with_folders_kept(&mut c, |_| false, |c| entry_moved(c, &mut st, a, ""));
    assert_eq!(groups(&c), [("a".to_owned(), String::new())]);
    assert_eq!(explicit_groups(&c), ["x/y"]); // x comes back through x/y
    assert_eq!(all_group_names(&c), ["x", "x/y"]);
}

#[test]
fn a_moved_folder_leaves_its_old_parent_but_not_its_old_path() {
    let mut c = cfg("connections:\n  - {label: a, host: a.lan, group: x/y}\n");
    let mut st = TreeState::default();
    let gone = |g: &str| g == "x/y" || g.starts_with("x/y/");
    with_folders_kept(&mut c, gone, |c| folder_moved(c, &mut st, "x/y", "")).expect("ok");
    assert_eq!(all_group_names(&c), ["x", "y"]);
    assert_eq!(explicit_groups(&c), ["x"]);
}

#[test]
fn folder_changes_survive_a_save_and_reload() {
    let mut c = sample();
    let mut st = TreeState::default();
    rename_folder(&mut c, &mut st, "lab", "test").expect("ok");
    folder_moved(&mut c, &mut st, "prod/empty", "test").expect("ok");
    delete_folder_into(&mut c, &mut st, "prod/db", "test").expect("ok");
    let text = dump(&c);
    let back = cfg(&text);
    assert_eq!(groups(&back), groups(&c));
    assert_eq!(
        all_group_names(&back),
        ["prod", "prod/web", "test", "test/empty"]
    );
    assert_eq!(settings(&back), settings(&sample()));
}
