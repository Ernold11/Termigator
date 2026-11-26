//! Quick-connect history (`termigator_core::quick_history`): order, duplicates, the limit,
//! forgetting, and that credentials are never stored.

use std::fs;
use std::path::PathBuf;

use termigator_core::config::doc::{Value, load as yaml_load};
use termigator_core::config::save::{load, save};
use termigator_core::paths::Paths;
use termigator_core::quick_history::{
    DEFAULT_LIMIT, KEY, clear, entries, forget, limit, remember, scrub,
};

fn cfg(text: &str) -> Value {
    yaml_load(text).expect("yaml")
}

fn stored(c: &Value) -> Vec<Value> {
    match c.get(KEY) {
        Some(Value::Seq(items)) => items.clone(),
        other => panic!("no list: {other:?}"),
    }
}

#[test]
fn newest_first_and_a_reused_target_moves_to_the_front() {
    let mut c = cfg("{}\n");
    for t in ["a@one", "b@two", "c@three"] {
        remember(&mut c, t);
    }
    assert_eq!(entries(&c), ["c@three", "b@two", "a@one"]);
    remember(&mut c, "  a@one ");
    assert_eq!(entries(&c), ["a@one", "c@three", "b@two"]);
    assert_eq!(stored(&c).len(), 3);
}

#[test]
fn the_limit_defaults_to_20_and_is_configurable() {
    let mut c = cfg("{}\n");
    assert_eq!(limit(&c), DEFAULT_LIMIT);
    for i in 0..30 {
        remember(&mut c, &format!("u@h{i}"));
    }
    let hist = entries(&c);
    assert_eq!(hist.len(), 20);
    assert_eq!(hist[0], "u@h29");

    let mut c = cfg("window: {quick_history_limit: 3}\n");
    for i in 0..5 {
        remember(&mut c, &format!("u@h{i}"));
    }
    assert_eq!(entries(&c), ["u@h4", "u@h3", "u@h2"]);
    assert_eq!(stored(&c).len(), 3);

    // Lowering the limit hides the tail at once; 0 keeps nothing.
    let mut c = cfg("window: {quick_history_limit: 0}\nquick_history: [u@a, u@b]\n");
    assert!(entries(&c).is_empty());
    remember(&mut c, "u@c");
    assert!(stored(&c).is_empty());

    assert_eq!(limit(&cfg("window: {quick_history_limit: -4}\n")), 0);
    assert_eq!(limit(&cfg("window: {quick_history_limit: 9999}\n")), 200);
    assert_eq!(limit(&cfg("window: {quick_history_limit: lots}\n")), 20);
}

#[test]
fn credentials_are_never_stored() {
    for secret in [
        "root:hunter2@db",
        ":tok@h",
        "u:@h:22",
        "a:b:c@d",
        "u@h\u{0}pw",
        "u@h\nx",
    ] {
        assert_eq!(scrub(secret), None, "{secret:?}");
        let mut c = cfg("quick_history: [u@ok]\n");
        remember(&mut c, secret);
        assert_eq!(stored(&c), [Value::Str("u@ok".into())], "{secret:?}");
    }
    // A colon after the `@` is a port, and saved entry names are kept as typed.
    assert_eq!(scrub(" u@h:2222 ").as_deref(), Some("u@h:2222"));
    assert_eq!(scrub("web prod").as_deref(), Some("web prod"));
}

#[test]
fn an_old_list_is_cleaned_when_read() {
    let mut c = cfg("quick_history: [u@a, 'root:pw@b', 7, u@a, '  ', u@c]\n");
    assert_eq!(entries(&c), ["u@a", "u@c"]);
    remember(&mut c, "u@d");
    assert_eq!(
        stored(&c),
        ["u@d", "u@a", "u@c"].map(|s| Value::Str(s.into()))
    );
}

#[test]
fn single_entries_can_be_forgotten_and_the_list_cleared() {
    let mut c = cfg("quick_history: [u@a, u@b, u@c]\n");
    forget(&mut c, "u@b");
    assert_eq!(entries(&c), ["u@a", "u@c"]);
    forget(&mut c, "not there");
    assert_eq!(entries(&c), ["u@a", "u@c"]);
    clear(&mut c);
    assert!(entries(&c).is_empty());
    assert!(stored(&c).is_empty());
}

#[test]
fn the_history_survives_saving_and_loading_the_config() {
    let base = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("quick_history_restart");
    let _ = fs::remove_dir_all(&base);
    let dir = base.join("cfg");
    fs::create_dir_all(&dir).expect("mkdir");
    let paths = Paths {
        repo_root: base.clone(),
        config_path: dir.join("config.yaml"),
        session_path: dir.join("session.yaml"),
        config_example_path: dir.join("config.example.yaml"),
        session_example_path: dir.join("session.example.yaml"),
        config_dir: dir,
    };
    let mut c = load(&paths).expect("load");
    remember(&mut c, "u@old");
    remember(&mut c, "root:pw@nope");
    remember(&mut c, "u@new");
    save(&paths, &c, false).expect("save");

    let text = fs::read_to_string(&paths.config_path).expect("config.yaml");
    assert!(!text.contains("pw"), "{text}");
    assert_eq!(entries(&load(&paths).expect("reload")), ["u@new", "u@old"]);
}
