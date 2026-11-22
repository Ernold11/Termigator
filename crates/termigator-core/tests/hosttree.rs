//! Host tree against the golden fixtures (compat/golden/hosttree_*.json), plus the
//! sidebar, drag-and-drop, bulk drag and duplicate tests.
mod common;

use serde_json::Value as J;
use termigator_core::config::doc::{Value, load as yaml_load};
use termigator_core::hosttree::{
    DropAction, EntryKind, EntryRef, Hooks, Icon, Row, RowKind, Tree, TreeState, Zone, apply_drop,
    build, drop_zone, entry_moved, resolve_drop,
};

// ---- golden decoding -------------------------------------------------------------------

fn from_golden(v: &J) -> Value {
    if let Some((tag, p)) = common::tagged(v) {
        let s = || p.as_str().unwrap_or_default().to_owned();
        return match tag {
            "$int" => Value::Int(s().parse().expect("i64")),
            "$float" => Value::Float(s().parse().expect("f64")),
            "$map" => Value::Map(
                p.as_array()
                    .into_iter()
                    .flatten()
                    .map(|kv| (from_golden(&kv[0]), from_golden(&kv[1])))
                    .collect(),
            ),
            other => panic!("unknown tag {other}"),
        };
    }
    match v {
        J::Null => Value::Null,
        J::Bool(b) => Value::Bool(*b),
        J::String(s) => Value::Str(s.clone()),
        J::Array(a) => Value::Seq(a.iter().map(from_golden).collect()),
        other => panic!("not a golden value: {other}"),
    }
}

fn strings(v: &J) -> Vec<String> {
    v.as_array()
        .into_iter()
        .flatten()
        .map(|s| s.as_str().unwrap_or_default().to_owned())
        .collect()
}

fn state(case: &J) -> TreeState {
    TreeState {
        expanded_groups: strings(&case["expanded"]).into_iter().collect(),
        recent_expanded: case["recent_expanded"].as_bool().unwrap_or(true),
    }
}

/// The launch-builder stand-ins the golden fixtures use.
struct GoldenHooks;

impl Hooks for GoldenHooks {
    fn build_error(&self, conn: &Value) -> Option<String> {
        let label = match conn.get("label") {
            Some(Value::Str(s)) => s.clone(),
            _ => String::new(),
        };
        label.starts_with("bad").then(|| format!("boom {label}"))
    }

    fn serial_summary(&self, conn: &Value) -> String {
        match conn.get("device") {
            Some(Value::Str(s)) => format!("SER:{s}"),
            _ => "SER:".to_owned(),
        }
    }
}

fn icon_token(i: &Icon) -> String {
    match i {
        Icon::Group => "group".into(),
        Icon::Recent => "recent".into(),
        Icon::Connection => "connection".into(),
        Icon::ConnectionError => "connection_err".into(),
        Icon::Remote => "rdp".into(),
        Icon::Serial => "serial".into(),
        Icon::Favorite => "favorite".into(),
        Icon::Distro(d) => format!("distro:{d}"),
        Icon::Custom(c) => format!("custom:{c}"),
    }
}

fn kind_token(k: RowKind) -> &'static str {
    match k {
        RowKind::Recent => "recent",
        RowKind::Group => "group",
        RowKind::Connection => "connection",
        RowKind::Favorite => "favorite",
    }
}

fn row_json(r: &Row) -> J {
    let mut o = serde_json::Map::new();
    o.insert("depth".into(), r.depth.into());
    o.insert("kind".into(), kind_token(r.kind).into());
    if r.kind == RowKind::Group {
        o.insert("path".into(), r.path.clone().into());
    } else {
        o.insert("index".into(), r.index.into());
    }
    o.insert("label".into(), r.label.clone().into());
    o.insert("subtitle".into(), r.subtitle.clone().into());
    o.insert("badge".into(), r.badge.clone().into());
    o.insert("icon".into(), icon_token(&r.icon).into());
    o.insert("error".into(), r.error.into());
    o.insert("expanded".into(), r.expanded.into());
    J::Object(o)
}

fn rows_json(t: &Tree) -> Vec<J> {
    t.rows.iter().map(row_json).collect()
}

#[test]
fn golden_build() {
    let cases = common::golden("hosttree_build");
    assert!(cases.len() >= 43);
    for c in &cases {
        let cfg = from_golden(&c["cfg"]);
        let filter = c["filter"].as_str().unwrap_or_default();
        let t = build(&cfg, &state(c), filter, &GoldenHooks);
        let e = &c["expect"];
        let name = common::name(c);
        let mut want: Vec<J> = e["rows"].as_array().cloned().unwrap_or_default();
        // Windows has no external terminal (terminal_mode_for_entry), so no "ext" badge.
        if cfg!(windows) {
            for row in &mut want {
                if row["badge"] == "ext" {
                    row["badge"] = "".into();
                }
            }
        }
        assert_eq!(rows_json(&t), want, "{name}: rows");
        assert_eq!(
            t.entry_count as u64,
            e["entries"].as_u64().unwrap_or(99),
            "{name}: entries"
        );
        assert_eq!(t.hint.as_deref(), e["hint"].as_str(), "{name}: hint");
        assert_eq!(
            t.status,
            e["status"].as_str().unwrap_or_default(),
            "{name}: status"
        );
    }
}

// ---- operations ------------------------------------------------------------------------

fn s(v: &Value) -> String {
    match v {
        Value::Str(t) => t.clone(),
        other => panic!("not a string: {other:?}"),
    }
}

fn n(v: &Value) -> i64 {
    match v {
        Value::Int(i) => *i,
        other => panic!("not an int: {other:?}"),
    }
}

fn seq(v: &Value) -> &[Value] {
    match v {
        Value::Seq(x) => x,
        other => panic!("not a list: {other:?}"),
    }
}

fn entry(kind: &Value, idx: &Value) -> EntryRef {
    EntryRef {
        kind: EntryKind::parse(&s(kind)).expect("kind"),
        index: usize::try_from(n(idx)).expect("index"),
    }
}

fn pair(v: &Value) -> EntryRef {
    entry(&seq(v)[0], &seq(v)[1])
}

fn sel_json(e: Option<EntryRef>) -> J {
    e.map_or(J::Null, |e| serde_json::json!([e.kind.name(), e.index]))
}

fn err_json(r: Result<(), String>) -> J {
    r.err().map_or(J::Null, J::String)
}

#[test]
fn golden_ops() {
    use termigator_core::hosttree as h;
    let cases = common::golden("hosttree_ops");
    assert!(cases.len() >= 54);
    for c in &cases {
        let mut cfg = from_golden(&c["cfg"]);
        let mut st = state(c);
        let a = from_golden(&c["args"]);
        let a = seq(&a);
        let (mut select, mut ret) = (None, J::Null);
        match c["op"].as_str().unwrap_or_default() {
            "entry_dropped" => {
                select = h::entry_moved(&mut cfg, &mut st, entry(&a[0], &a[1]), &s(&a[2]))
            }
            "entries_placed" => {
                let moving: Vec<EntryRef> = seq(&a[0]).iter().map(pair).collect();
                let after = a[3] == Value::Bool(true);
                select =
                    h::entries_placed(&mut cfg, &mut st, &moving, &s(&a[1]), pair(&a[2]), after);
            }
            "folder_moved" => {
                ret = err_json(h::folder_moved(&mut cfg, &mut st, &s(&a[0]), &s(&a[1])))
            }
            "group_rename" => {
                ret = err_json(h::apply_group_rename(
                    &mut cfg,
                    &mut st,
                    &s(&a[0]),
                    &s(&a[1]),
                ));
            }
            "ungroup" => h::ungroup_group(&mut cfg, &mut st, &s(&a[0])),
            "delete_group" => h::delete_group(&mut cfg, &mut st, &s(&a[0])),
            "add_group" => {
                h::add_group(&mut cfg, &mut st, &s(&a[0]), &s(&a[1]));
            }
            "duplicate" => select = h::duplicate_entry(&mut cfg, entry(&a[0], &a[1])),
            "move" => {
                let d = isize::try_from(n(&a[2])).expect("delta");
                select = h::move_entry(&mut cfg, entry(&a[0], &a[1]), d);
            }
            "move_to_group" => {
                select = h::move_to_group(&mut cfg, &mut st, entry(&a[0], &a[1]), &s(&a[2]));
            }
            "remove" => h::remove_entry(&mut cfg, entry(&a[0], &a[1])),
            "bulk_delete" => {
                let idx: Vec<usize> = seq(&a[0]).iter().map(|i| n(i) as usize).collect();
                h::bulk_delete(&mut cfg, &idx);
            }
            "touch_recent" => {
                let conn =
                    seq(cfg.get("connections").expect("connections"))[n(&a[0]) as usize].clone();
                h::touch_recent(&mut cfg, &conn);
            }
            other => panic!("op {other}"),
        }
        let name = common::name(c);
        let e = &c["expect"];
        assert_eq!(cfg, from_golden(&e["cfg"]), "{name}: cfg");
        let got: Vec<String> = st.expanded_groups.iter().cloned().collect();
        assert_eq!(got, strings(&e["expanded"]), "{name}: expanded");
        // Only ops that re-select are compared (the others never set `select`).
        assert_eq!(sel_json(select), e["select"], "{name}: select");
        assert_eq!(ret, e["ret"], "{name}: ret");
    }
}

// ---- drops -----------------------------------------------------------------------------

fn payload_row(rows: &[Row], p: &J) -> usize {
    let key = p[0].as_str().unwrap_or_default();
    rows.iter()
        .position(|r| {
            kind_token(r.kind) == key
                && if key == "group" {
                    p[1].as_str() == Some(r.path.as_str())
                } else {
                    p[1].as_u64() == Some(r.index as u64)
                }
        })
        .unwrap_or_else(|| panic!("no row {p}"))
}

fn signals(a: &DropAction) -> Vec<J> {
    let m = |e: &EntryRef| serde_json::json!([e.kind.name(), e.index]);
    match a {
        DropAction::Ignore => vec![],
        DropAction::FolderMoved { path, parent } => {
            vec![serde_json::json!(["folder", path, parent])]
        }
        DropAction::EntriesPlaced {
            moving,
            group,
            anchor,
            after,
        } => vec![serde_json::json!([
            "place",
            moving.iter().map(m).collect::<Vec<_>>(),
            group,
            m(anchor),
            after
        ])],
        DropAction::EntryMoves { moving, group } => moving
            .iter()
            .map(|e| serde_json::json!(["move", e.kind.name(), e.index, group]))
            .collect(),
    }
}

#[test]
fn golden_drop() {
    let cases = common::golden("hosttree_drop");
    assert!(cases.len() >= 20);
    for c in &cases {
        let cfg = from_golden(&c["cfg"]);
        let t = build(&cfg, &state(c), "", &());
        let cur = payload_row(&t.rows, &c["current"]);
        let sel: Vec<usize> = c["selected"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|p| payload_row(&t.rows, p))
            .collect();
        let target = c["target"].is_array().then(|| {
            let r = payload_row(&t.rows, &c["target"]);
            let y = match c["zone"].as_str() {
                Some("above") => 0.05,
                Some("below") => 0.95,
                _ => 0.5,
            };
            (r, drop_zone(&t.rows[r], y))
        });
        let target = target.and_then(|(r, z)| z.map(|z| (r, z)));
        let got = signals(&resolve_drop(&t.rows, cur, &sel, target));
        assert_eq!(
            got,
            c["expect"].as_array().cloned().unwrap_or_default(),
            "{}",
            common::name(c)
        );
    }
}

// ---- sidebar tests ---------------------------------------------------------------

fn cfg(text: &str) -> Value {
    yaml_load(text).expect("yaml")
}

fn labels(t: &Tree) -> Vec<(usize, String)> {
    t.rows.iter().map(|r| (r.depth, r.label.clone())).collect()
}

fn ui_sidebar_cfg() -> Value {
    cfg(r#"
window: {show_recent: true}
connections:
  - {label: web1, host: w1, user: u, group: prod/web}
  - {label: db1, host: d1, user: u, group: prod/db}
  - {label: solo, host: s1, user: u}
  - {label: moba, host: m1, user: u, group: 'prod\web\eu'}
favorites: [{label: fav, command: htop, group: tools}]
groups: [empty/sub]
recent: []
"#)
}

fn all_open() -> TreeState {
    TreeState {
        expanded_groups: [
            "empty",
            "empty/sub",
            "prod",
            "prod/web",
            "prod/web/eu",
            "prod/db",
            "tools",
        ]
        .iter()
        .map(|s| (*s).to_owned())
        .collect(),
        recent_expanded: true,
    }
}

#[test]
fn folders_sorted_first_then_ungrouped_entries() {
    let t = build(&ui_sidebar_cfg(), &TreeState::default(), "", &());
    let tops: Vec<(RowKind, &str)> = t.rows.iter().map(|r| (r.kind, r.label.as_str())).collect();
    assert_eq!(
        tops,
        [
            (RowKind::Group, "empty"),
            (RowKind::Group, "prod"),
            (RowKind::Group, "tools"),
            (RowKind::Connection, "solo")
        ]
    );
    assert_eq!(t.rows[1].path, "prod");
}

#[test]
fn nested_structure_and_backslash_import() {
    let t = build(&ui_sidebar_cfg(), &all_open(), "", &());
    let paths: Vec<(&str, &str)> = t
        .rows
        .iter()
        .skip(2)
        .take(5)
        .map(|r| (r.path.as_str(), r.label.as_str()))
        .collect();
    assert_eq!(
        paths,
        [
            ("prod", "prod"),
            ("prod/db", "db"),
            ("", "db1"),
            ("prod/web", "web"),
            ("prod/web/eu", "eu")
        ]
    );
    assert_eq!(t.rows[7].label, "moba");
    assert_eq!(t.rows[7].depth, 3);
}

#[test]
fn explicit_empty_folder_exists() {
    let t = build(&ui_sidebar_cfg(), &all_open(), "", &());
    assert_eq!(
        (
            t.rows[1].path.as_str(),
            t.rows[1].depth,
            t.rows[1].has_children
        ),
        ("empty/sub", 1, false)
    );
}

#[test]
fn badge_counts_whole_subtree() {
    let t = build(&ui_sidebar_cfg(), &TreeState::default(), "", &());
    assert_eq!(t.rows[1].badge, "3"); // web1 + db1 + moba
}

#[test]
fn drop_moves_entry_between_folders() {
    let mut c = ui_sidebar_cfg();
    let mut st = TreeState::default();
    let solo = EntryRef {
        kind: EntryKind::Connection,
        index: 2,
    };
    entry_moved(&mut c, &mut st, solo, "prod/web/eu");
    let conns = |c: &Value| seq(c.get("connections").expect("c"))[2].clone();
    assert_eq!(
        conns(&c).get("group"),
        Some(&Value::Str("prod/web/eu".into()))
    );
    entry_moved(&mut c, &mut st, solo, "");
    assert_eq!(conns(&c).get("group"), None);
}

#[test]
fn filter_shows_only_matching_branches() {
    let t = build(&ui_sidebar_cfg(), &TreeState::default(), "db1", &());
    assert!(
        t.rows
            .iter()
            .any(|r| r.kind == RowKind::Group && r.path == "prod")
    );
    assert!(!t.rows.iter().any(|r| r.path == "empty")); // an empty folder cannot match a search
    assert_eq!(t.status, "1 result · search active");
}

fn dnd_cfg() -> Value {
    cfg(r#"
window: {show_recent: false}
connections:
  - {label: a, host: a.lan, user: u, group: prod}
  - {label: b, host: b.lan, user: u, group: prod}
  - {label: c, host: c.lan, user: u, group: prod}
  - {label: x, host: x.lan, user: u, group: infra}
  - {label: top, host: top.lan, user: u, group: ''}
favorites: []
"#)
}

fn dnd_state() -> TreeState {
    TreeState {
        expanded_groups: ["prod", "infra"].iter().map(|s| (*s).to_string()).collect(),
        recent_expanded: true,
    }
}

fn conn(i: usize) -> EntryRef {
    EntryRef {
        kind: EntryKind::Connection,
        index: i,
    }
}

/// (label, group) of every connection in config order.
fn order(c: &Value) -> Vec<(String, String)> {
    seq(c.get("connections").expect("connections"))
        .iter()
        .map(|e| {
            (
                s(e.get("label").expect("label")),
                e.get("group").map_or(String::new(), s),
            )
        })
        .collect()
}

fn row_of(rows: &[Row], e: EntryRef) -> usize {
    rows.iter().position(|r| r.entry() == Some(e)).expect("row")
}

fn group_row(rows: &[Row], p: &str) -> usize {
    rows.iter()
        .position(|r| r.kind == RowKind::Group && r.path == p)
        .expect("group row")
}

/// A drop as the sidebar does it: rebuild the tree, resolve, apply.
fn drop_on(
    c: &mut Value,
    st: &mut TreeState,
    src: Option<EntryRef>,
    group_src: Option<&str>,
    target: Option<(&str, Option<EntryRef>)>,
    zone: Zone,
    extra: &[EntryRef],
) -> Result<Option<EntryRef>, String> {
    let t = build(c, st, "", &());
    let cur = src.map_or_else(
        || group_row(&t.rows, group_src.expect("src")),
        |e| row_of(&t.rows, e),
    );
    let sel: Vec<usize> = std::iter::once(cur)
        .chain(extra.iter().map(|e| row_of(&t.rows, *e)))
        .collect();
    let tgt = target.map(|(g, e)| {
        (
            e.map_or_else(|| group_row(&t.rows, g), |e| row_of(&t.rows, e)),
            zone,
        )
    });
    apply_drop(c, st, &resolve_drop(&t.rows, cur, &sel, tgt))
}

fn in_folder(c: &Value, folder: &str) -> Vec<String> {
    order(c)
        .into_iter()
        .filter(|(_, g)| g == folder)
        .map(|(n, _)| n)
        .collect()
}

#[test]
fn reorder_inside_a_folder() {
    let (mut c, mut st) = (dnd_cfg(), dnd_state());
    drop_on(
        &mut c,
        &mut st,
        Some(conn(2)),
        None,
        Some(("", Some(conn(0)))),
        Zone::Above,
        &[],
    )
    .expect("ok");
    assert_eq!(in_folder(&c, "prod"), ["c", "a", "b"]);
    let sel = drop_on(
        &mut c,
        &mut st,
        Some(conn(0)),
        None,
        Some(("", Some(conn(2)))),
        Zone::Below,
        &[],
    );
    assert_eq!(in_folder(&c, "prod"), ["a", "b", "c"]);
    assert_eq!(sel, Ok(Some(conn(2)))); // still the dragged one
}

#[test]
fn place_into_another_folder_with_a_selection() {
    let (mut c, mut st) = (dnd_cfg(), dnd_state());
    drop_on(
        &mut c,
        &mut st,
        Some(conn(0)),
        None,
        Some(("", Some(conn(3)))),
        Zone::Above,
        &[conn(1)],
    )
    .expect("ok");
    let want: Vec<(String, String)> = [
        ("c", "prod"),
        ("a", "infra"),
        ("b", "infra"),
        ("x", "infra"),
        ("top", ""),
    ]
    .iter()
    .map(|(a, b)| ((*a).to_owned(), (*b).to_owned()))
    .collect();
    assert_eq!(order(&c), want);
}

#[test]
fn drop_on_an_entry_still_just_changes_the_folder() {
    let (mut c, mut st) = (dnd_cfg(), dnd_state());
    drop_on(
        &mut c,
        &mut st,
        Some(conn(4)),
        None,
        Some(("", Some(conn(1)))),
        Zone::On,
        &[],
    )
    .expect("ok");
    assert_eq!(
        order(&c).last(),
        Some(&("top".to_owned(), "prod".to_owned()))
    ); // order kept
}

#[test]
fn folder_moves_with_its_contents() {
    let (mut c, mut st) = (dnd_cfg(), dnd_state());
    let abc = |c: &Value| -> Vec<String> {
        let mut g: Vec<String> = order(c)
            .into_iter()
            .filter(|(n, _)| "abc".contains(n.as_str()))
            .map(|(_, g)| g)
            .collect();
        g.dedup();
        g
    };
    drop_on(
        &mut c,
        &mut st,
        None,
        Some("prod"),
        Some(("infra", None)),
        Zone::On,
        &[],
    )
    .expect("ok");
    assert_eq!(abc(&c), ["infra/prod"]);
    drop_on(
        &mut c,
        &mut st,
        None,
        Some("infra/prod"),
        None,
        Zone::On,
        &[],
    )
    .expect("ok"); // empty space
    assert_eq!(abc(&c), ["prod"]);
    drop_on(
        &mut c,
        &mut st,
        None,
        Some("prod"),
        Some(("infra", None)),
        Zone::Above,
        &[],
    )
    .expect("ok");
    assert_eq!(abc(&c), ["prod"]); // next to infra: top level
}

#[test]
fn folder_cannot_go_into_itself() {
    let mut c = cfg(r#"
window: {show_recent: false}
connections:
  - {label: a, host: a.lan, group: prod/sub}
  - {label: b, host: b.lan, group: prod}
  - {label: c, host: c.lan, group: prod}
favorites: []
"#);
    let mut st = dnd_state();
    st.expanded_groups.insert("prod/sub".into());
    let r = drop_on(
        &mut c,
        &mut st,
        None,
        Some("prod"),
        Some(("prod/sub", None)),
        Zone::On,
        &[],
    );
    assert_eq!(r, Err("Cannot move a folder into itself.".to_owned()));
    let rest: Vec<String> = order(&c).into_iter().skip(1).map(|(_, g)| g).collect();
    assert_eq!(rest, ["prod", "prod"]);
}

#[test]
fn dragging_a_selection_moves_every_selected_entry() {
    let mut c = cfg(r#"
connections:
  - {label: a, host: a, user: u}
  - {label: b, protocol: rdp, host: b, user: x}
  - {label: c, protocol: vnc, host: c, group: lab}
favorites: []
"#);
    let mut st = TreeState::default();
    st.expanded_groups.insert("lab".into());
    drop_on(
        &mut c,
        &mut st,
        Some(conn(0)),
        None,
        Some(("lab", None)),
        Zone::On,
        &[conn(0), conn(1)],
    )
    .expect("ok");
    let groups: Vec<String> = order(&c).into_iter().map(|(_, g)| g).collect();
    assert_eq!(groups, ["lab", "lab", "lab"]);
}

#[test]
fn duplicate_entry_does_not_share_lists() {
    let mut c = cfg(
        "connections: [{label: a, host: h, user: u, tags: [prod], local_forwards: ['1:x:2']}]\nfavorites: []\n",
    );
    let sel = termigator_core::hosttree::duplicate_entry(&mut c, conn(0));
    assert_eq!(sel, Some(conn(1)));
    let l = seq(c.get("connections").expect("c")).to_vec();
    assert_eq!(l[1].get("label"), Some(&Value::Str("a (copy)".into())));
    assert_eq!(l[1].get("tags"), l[0].get("tags"));
    // Values own their data in Rust: mutating the clone cannot touch the original.
    let mut copy = l[1].clone();
    if let Value::Map(p) = &mut copy {
        p.clear();
    }
    assert!(l[0].get("tags").is_some());
}

// ---- Rust-only checks ------------------------------------------------------------------

#[test]
fn drop_zone_thresholds_and_recent() {
    let t = build(
        &cfg("connections: [{label: a, host: h}, {label: b, host: k}]\nrecent: ['h||']\n"),
        &TreeState::default(),
        "",
        &(),
    );
    let (recent, entry) = (&t.rows[0], &t.rows[2]);
    assert_eq!(recent.kind, RowKind::Recent);
    assert_eq!(drop_zone(recent, 0.5), None);
    assert_eq!(drop_zone(&t.rows[1], 0.5), None); // Recent child
    assert_eq!(drop_zone(entry, 0.0), Some(Zone::Above));
    assert_eq!(drop_zone(entry, 0.5), Some(Zone::On));
    assert_eq!(drop_zone(entry, 1.0), Some(Zone::Below));
    assert_eq!(
        labels(&t),
        [
            (0, "Recent".to_owned()),
            (1, "a".to_owned()),
            (0, "a".to_owned()),
            (0, "b".to_owned())
        ]
    );
}

#[test]
fn expand_collapse_ignored_while_filtering() {
    let t = build(&ui_sidebar_cfg(), &TreeState::default(), "", &());
    let mut st = TreeState::default();
    st.set_expanded(&t.rows[1], true, true);
    assert!(st.expanded_groups.is_empty());
    st.set_expanded(&t.rows[1], true, false);
    assert!(st.expanded_groups.contains("prod"));
    st.set_expanded(&t.rows[1], false, false);
    assert!(st.expanded_groups.is_empty());
}

// ---- folder environments ----------------------------------------------------------

#[test]
fn folder_environment_is_inherited_and_follows_the_folder() {
    use termigator_core::hosttree::{
        apply_group_rename, delete_group, effective_environment, folder_environment,
        group_environments, set_group_environment, ungroup_group,
    };
    let mut c = cfg("connections:\n\
         - {label: a, host: h, user: u, group: Prod/db}\n\
         - {label: b, host: h, user: u, group: Prod, environment: test}\n\
         - {label: c, host: h, user: u}\n");
    let entry = |c: &Value, i: usize| match c.get("connections") {
        Some(Value::Seq(items)) => items[i].clone(),
        _ => Value::Null,
    };
    set_group_environment(&mut c, "Prod", "production");
    // Inherited down the tree; an entry's own value wins; outside the folder: none.
    assert_eq!(effective_environment(&c, &entry(&c, 0)), "production");
    assert_eq!(effective_environment(&c, &entry(&c, 1)), "test");
    assert_eq!(effective_environment(&c, &entry(&c, 2)), "");
    assert_eq!(
        folder_environment(&c, "Prod/db"),
        ("production".to_owned(), "Prod".to_owned())
    );
    // The nearest marked folder wins.
    set_group_environment(&mut c, "Prod/db", "staging");
    assert_eq!(effective_environment(&c, &entry(&c, 0)), "staging");
    // A rename carries the marks along.
    let mut st = TreeState::default();
    apply_group_rename(&mut c, &mut st, "Prod", "Live").expect("rename");
    let map: Vec<(String, String)> = group_environments(&c).into_iter().collect();
    assert_eq!(
        map,
        [
            ("Live".to_owned(), "production".to_owned()),
            ("Live/db".to_owned(), "staging".to_owned())
        ]
    );
    // Dissolving Live/db drops its own mark; its entries now inherit Live's.
    ungroup_group(&mut c, &mut st, "Live/db");
    assert_eq!(effective_environment(&c, &entry(&c, 0)), "production");
    // Clearing and deleting remove the key.
    set_group_environment(&mut c, "Live", "");
    assert!(group_environments(&c).is_empty());
    set_group_environment(&mut c, "Live", "production");
    delete_group(&mut c, &mut st, "Live");
    assert!(c.get("group_environments").is_none());
}
