//! Pane tree rules and the `last_session` snapshot format, checked against golden files
//! recorded by driving the window offscreen (compat/golden/panes_layout.json,
//! panes_restore.json).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use std::collections::HashMap;

use serde_json::Value as J;
use termigator_core::config::doc::Value;
use termigator_core::panes::{
    Dock, Location, Orientation, PaneError, PaneId, Panes, RestoreSource, TabId, TabRecord,
    hit_zone, restore,
};

const SHELL: &str = "exec bash --norc -i";

fn from_golden(v: &J) -> Value {
    match v {
        J::Null => Value::Null,
        J::Bool(b) => Value::Bool(*b),
        J::String(s) => Value::Str(s.clone()),
        J::Array(a) => Value::Seq(a.iter().map(from_golden).collect()),
        J::Number(n) => n
            .as_i64()
            .map_or_else(|| Value::Float(n.as_f64().unwrap()), Value::Int),
        J::Object(_) => match common::tagged(v) {
            Some(("$int", p)) => Value::Int(p.as_str().unwrap().parse().unwrap()),
            Some(("$float", p)) => Value::Float(p.as_str().unwrap().parse().unwrap()),
            Some(("$map", p)) => Value::Map(
                p.as_array()
                    .unwrap()
                    .iter()
                    .map(|kv| (from_golden(&kv[0]), from_golden(&kv[1])))
                    .collect(),
            ),
            _ => panic!("untagged object: {v}"),
        },
    }
}

/// Drop each splitter's `sizes` after checking their shape: they are real widget geometry in the
/// recorded run, so only "one integer per child" is comparable.
fn strip_sizes(v: &Value) -> Value {
    let Value::Map(m) = v else {
        return v.clone();
    };
    let kids = m.iter().find(|(k, _)| *k == Value::Str("children".into()));
    let sizes = m.iter().find(|(k, _)| *k == Value::Str("sizes".into()));
    if let (Some((_, Value::Seq(kids))), Some((_, Value::Seq(sizes)))) = (kids, sizes) {
        assert_eq!(kids.len(), sizes.len(), "sizes per child: {v:?}");
        assert!(sizes.iter().all(|s| matches!(s, Value::Int(_))));
    }
    Value::Map(
        m.iter()
            .filter(|(k, _)| *k != Value::Str("sizes".into()))
            .map(|(k, x)| match x {
                Value::Seq(xs) => (k.clone(), Value::Seq(xs.iter().map(strip_sizes).collect())),
                // The recorded window sorts keys of the saved entries it loads; key order of a
                // connection is not pane logic.
                Value::Map(c) if *k == Value::Str("conn".into()) => {
                    let mut c = c.clone();
                    c.sort_by_key(|(k, _)| format!("{k:?}"));
                    (k.clone(), Value::Map(c))
                }
                _ => (k.clone(), x.clone()),
            })
            .collect(),
    )
}

fn plain_json(v: &J) -> Value {
    match v {
        J::Object(o) => Value::Map(
            o.iter()
                .map(|(k, x)| (Value::Str(k.clone()), plain_json(x)))
                .collect(),
        ),
        other => from_golden(other),
    }
}

struct Win {
    p: Panes,
    names: HashMap<String, TabId>,
    by_id: HashMap<TabId, String>,
    conns: HashMap<TabId, Value>,
}

impl Win {
    fn new() -> Self {
        Self {
            p: Panes::new(),
            names: HashMap::new(),
            by_id: HashMap::new(),
            conns: HashMap::new(),
        }
    }

    fn tab(&mut self, title: &str) -> TabId {
        let id = self.p.new_tab_id();
        let active = self.p.active();
        self.p.add_tab(active, id, None).unwrap();
        self.names.insert(title.to_owned(), id);
        self.by_id.insert(id, title.to_owned());
        id
    }

    fn name(&self, id: TabId) -> &str {
        &self.by_id[&id]
    }

    fn titles(&self, tabs: &[TabId]) -> Vec<String> {
        tabs.iter().map(|&t| self.name(t).to_owned()).collect()
    }

    fn pane_titles(&self) -> Vec<Vec<String>> {
        self.p
            .list_panes()
            .iter()
            .map(|&p| self.titles(self.p.tabs(p).unwrap()))
            .collect()
    }

    fn split(&mut self, o: Orientation) -> PaneId {
        let a = self.p.active();
        self.p.split(a, o, true, 1000).unwrap()
    }

    fn snapshot(&self) -> Value {
        self.p.snapshot(&|t| {
            Some(TabRecord {
                title: self.name(t).to_owned(),
                conn: self.conns.get(&t).cloned(),
                inner: SHELL.to_owned(),
            })
        })
    }
}

fn orient(s: &str) -> Orientation {
    if s == "h" {
        Orientation::Horizontal
    } else {
        Orientation::Vertical
    }
}

fn state_json(w: &Win) -> J {
    let panes = w.p.list_panes();
    let cur: Vec<i64> = panes
        .iter()
        .map(|&p| w.p.current(p).unwrap().map_or(-1, |c| c as i64))
        .collect();
    let windows: Vec<Vec<String>> =
        w.p.windows()
            .iter()
            .map(|&id| w.titles(w.p.window_tabs(id).unwrap()))
            .collect();
    serde_json::json!({
        "panes": w.pane_titles(), "current": cur,
        "active": panes.iter().position(|&p| p == w.p.active()).unwrap(),
        "windows": windows,
    })
}

fn check_state(name: &str, w: &Win, expect: &J) {
    let got = state_json(w);
    for k in ["panes", "current", "active", "windows"] {
        assert_eq!(got[k], expect[k], "{name}: {k}");
    }
    assert_eq!(
        strip_sizes(&w.snapshot()),
        strip_sizes(&from_golden(&expect["snapshot"])),
        "{name}: snapshot"
    );
}

#[test]
fn golden_layouts_match_golden() {
    let cases = common::golden("panes_layout");
    assert!(cases.len() >= 20);
    for case in &cases {
        let name = common::name(case);
        // Deliberate deviation: the recorded window could never close an emptied main pane (see
        // `emptied_main_pane_goes_when_others_remain`).
        if name == "main_pane_emptied_other_stays" {
            continue;
        }
        let mut w = Win::new();
        for op in case["ops"].as_array().unwrap() {
            let title = op["title"].as_str().unwrap_or("");
            match op["op"].as_str().unwrap() {
                "tab" => {
                    let id = w.tab(title);
                    if let Some(c) = op.get("conn") {
                        w.conns.insert(id, plain_json(c));
                    }
                }
                "split" => {
                    w.split(orient(op["o"].as_str().unwrap()));
                }
                "focus" => {
                    let p = w.p.list_panes()[op["pane"].as_u64().unwrap() as usize];
                    w.p.set_active(p).unwrap();
                }
                "close_tab" => w.p.close_tab(w.names[title], false).unwrap(),
                "close_pane" => {
                    let a = w.p.active();
                    w.p.close_pane(a).unwrap();
                }
                "detach" => {
                    w.p.detach_tab(w.names[title]).unwrap();
                }
                "attach" => w.p.attach_tab(w.names[title]).unwrap(),
                other => panic!("{name}: op {other}"),
            }
        }
        check_state(name, &w, &case["expect"]);
    }
}

#[test]
fn golden_restore_matches_golden() {
    let cases = common::golden("panes_restore");
    assert!(cases.len() >= 20);
    for case in &cases {
        let name = common::name(case);
        let snap = from_golden(&case["snapshot"]);
        let Value::Seq(conns) = from_golden(&case["connections"]) else {
            panic!("{name}: connections");
        };
        let r = restore(&snap, &conns);
        let mut w = Win {
            p: r.panes,
            names: HashMap::new(),
            by_id: HashMap::new(),
            conns: HashMap::new(),
        };
        let panes = w.p.list_panes();
        let mut opened = Vec::new();
        for t in &r.tabs {
            w.names.insert(t.title.clone(), t.tab);
            w.by_id.insert(t.tab, t.title.clone());
            let mut pane = panes.iter().position(|&p| p == t.pane).unwrap();
            // The recorded window opens the tabs while the main pane is still detached from the splitter
            // when the snapshot never marks one (it is put in front afterwards).
            if !t.detached && !format!("{snap:?}").contains("(Str(\"main\"), Bool(true))") {
                pane -= 1;
            }
            let (inner, conn) = match &t.source {
                RestoreSource::Inner(i) => (J::String(i.clone()), J::Null),
                RestoreSource::Connection { entry, matched } => {
                    if let Some(i) = matched {
                        assert!(entry.loose_eq(&conns[*i]), "{name}");
                    }
                    w.conns.insert(t.tab, entry.clone());
                    (J::Null, J::Null)
                }
            };
            opened.push((t.title.clone(), pane, inner, conn, t.detached));
        }
        let expect = case["expect"]["opened"].as_array().unwrap();
        assert_eq!(opened.len(), expect.len(), "{name}: opened count");
        for ((title, pane, inner, _, _), e) in opened.iter().zip(expect) {
            assert_eq!(title, e["title"].as_str().unwrap(), "{name}");
            assert_eq!(*pane as u64, e["pane"].as_u64().unwrap(), "{name}: {title}");
            assert_eq!(*inner, e["inner"], "{name}: {title}");
        }
        for (t, e) in r.tabs.iter().zip(expect) {
            match &t.source {
                RestoreSource::Connection { entry, .. } => {
                    assert!(
                        entry.loose_eq(&from_golden(&e["conn"])),
                        "{name}: {}",
                        t.title
                    );
                }
                RestoreSource::Inner(_) => assert!(e["conn"].is_null(), "{name}"),
            }
        }
        check_state(name, &w, &case["expect"]);
    }
}

#[test]
fn starts_with_single_pane() {
    let p = Panes::new();
    assert_eq!(p.list_panes(), vec![p.main_pane()]);
    assert_eq!(p.active(), p.main_pane());
}

#[test]
fn split_right_moves_current_tab() {
    let mut w = Win::new();
    w.tab("t1");
    w.tab("t2");
    let new = w.split(Orientation::Horizontal);
    assert_eq!(w.pane_titles(), [["t1"], ["t2"]]);
    assert_eq!(w.p.active(), new);
}

#[test]
fn split_single_tab_creates_empty_pane() {
    let mut w = Win::new();
    w.tab("only");
    w.split(Orientation::Horizontal);
    assert_eq!(w.p.list_panes().len(), 2);
    assert_eq!(w.pane_titles(), [vec!["only".to_owned()], vec![]]);
}

#[test]
fn cross_orientation_split_nests_a_splitter() {
    let mut w = Win::new();
    w.tab("t1");
    w.tab("t2");
    w.split(Orientation::Horizontal);
    w.tab("t3");
    w.split(Orientation::Vertical);
    assert_eq!(w.p.list_panes().len(), 3);
    let j = w.p.to_json();
    let root = &j["root"]["children"];
    assert_eq!(root.as_array().unwrap().len(), 2);
    assert_eq!(root[1]["type"], "split");
    assert_eq!(root[1]["orientation"], "v");
    assert_eq!(root[1]["children"][1]["tabs"][0], 3);
    // a fresh nested splitter divides max(200, extent) in two, rounding the first half up
    assert_eq!(root[1]["sizes"], serde_json::json!([500, 500]));
}

#[test]
fn nested_split_uses_at_least_200() {
    let mut p = Panes::new();
    p.split(0, Orientation::Vertical, false, 51).unwrap();
    assert_eq!(
        p.to_json()["root"]["children"][0]["sizes"],
        serde_json::json!([100, 100])
    );
}

#[test]
fn same_orientation_split_extends_splitter_and_halves_size() {
    let mut p = Panes::new();
    p.set_sizes(&[], vec![900]).unwrap();
    let b = p.split(0, Orientation::Horizontal, false, 0).unwrap();
    p.split(b, Orientation::Horizontal, false, 0).unwrap();
    let j = p.to_json();
    assert_eq!(j["root"]["children"].as_array().unwrap().len(), 3);
    assert_eq!(j["root"]["sizes"], serde_json::json!([450, 225, 225]));
}

#[test]
fn closing_last_tab_removes_pane_and_unwraps() {
    let mut w = Win::new();
    w.tab("t1");
    w.tab("t2");
    w.split(Orientation::Horizontal);
    w.tab("t3");
    w.split(Orientation::Vertical);
    let t3 = w.names["t3"];
    w.p.close_tab(t3, false).unwrap();
    assert_eq!(w.p.list_panes().len(), 2);
    let j = w.p.to_json();
    assert!(
        j["root"]["children"]
            .as_array()
            .unwrap()
            .iter()
            .all(|c| c["type"] == "pane")
    );
    w.p.close_tab(w.names["t2"], false).unwrap();
    assert_eq!(w.p.list_panes(), vec![0]);
    assert_eq!(w.p.active(), 0);
}

#[test]
fn close_pane_action() {
    let mut w = Win::new();
    w.tab("t1");
    w.tab("t2");
    w.split(Orientation::Horizontal);
    let a = w.p.active();
    assert_eq!(w.p.close_pane(a).unwrap(), vec![w.names["t2"]]);
    assert_eq!(w.pane_titles(), [["t1"]]);
}

#[test]
fn main_pane_never_removed() {
    let mut w = Win::new();
    w.tab("t1");
    assert_eq!(w.p.close_pane(0).unwrap().len(), 1);
    assert_eq!(w.p.list_panes(), vec![0]);
    assert!(w.p.tabs(0).unwrap().is_empty());
    assert_eq!(w.p.current(0).unwrap(), None);
}

#[test]
fn keep_pane_leaves_emptied_pane() {
    let mut w = Win::new();
    w.tab("a");
    w.tab("b");
    w.split(Orientation::Horizontal);
    w.p.close_tab(w.names["b"], true).unwrap();
    assert_eq!(w.p.list_panes().len(), 2);
}

#[test]
fn removed_pane_hands_its_size_to_the_neighbour() {
    let mut p = Panes::new();
    p.set_sizes(&[], vec![800]).unwrap();
    let b = p.split(0, Orientation::Horizontal, false, 0).unwrap();
    p.close_pane(b).unwrap();
    assert_eq!(p.to_json()["root"]["sizes"], serde_json::json!([800]));
}

#[test]
fn detach_and_attach() {
    let mut w = Win::new();
    let a = w.tab("alpha");
    let b = w.tab("beta");
    w.split(Orientation::Horizontal);
    let win = w.p.detach_tab(b).unwrap();
    assert_eq!(
        w.p.list_panes().len(),
        1,
        "the emptied split pane went away"
    );
    assert_eq!(w.p.locate(b), Some(Location::Window(win)));
    assert_eq!(w.p.locate(a), Some(Location::Pane(0)));
    w.p.attach_tab(b).unwrap();
    assert_eq!(w.p.locate(b), Some(Location::Pane(0)));
    assert!(w.p.windows().is_empty());
    assert_eq!(w.p.current(0).unwrap(), Some(1));
}

#[test]
fn detach_errors() {
    let mut w = Win::new();
    let a = w.tab("a");
    assert_eq!(w.p.attach_tab(a), Err(PaneError::NotDetached(a)));
    assert_eq!(w.p.detach_tab(99), Err(PaneError::NoSuchTab(99)));
    assert_eq!(w.p.set_active(42), Err(PaneError::NoSuchPane(42)));
}

#[test]
fn closing_a_tab_selects_the_right_neighbour_like_qt() {
    let mut w = Win::new();
    for t in ["a", "b", "c", "d"] {
        w.tab(t);
    }
    w.p.set_current(0, 1).unwrap();
    w.p.close_tab(w.names["b"], false).unwrap();
    assert_eq!(w.p.current(0).unwrap(), Some(1)); // "c" took the slot
    w.p.close_tab(w.names["d"], false).unwrap();
    assert_eq!(w.p.current(0).unwrap(), Some(1));
    w.p.close_tab(w.names["a"], false).unwrap();
    assert_eq!(w.p.current(0).unwrap(), Some(0)); // current "c" shifted left
}

#[test]
fn active_pane_falls_back_to_the_first_when_removed() {
    let mut w = Win::new();
    w.tab("a");
    w.tab("b");
    let new = w.split(Orientation::Horizontal);
    assert_eq!(w.p.active(), new);
    w.p.close_tab(w.names["b"], false).unwrap();
    assert_eq!(w.p.active(), 0);
}

#[test]
fn snapshot_never_holds_a_password_and_skips_foreign_tabs() {
    let mut w = Win::new();
    let a = w.tab("db");
    w.tab("files");
    w.conns.insert(
        a,
        Value::Map(vec![
            (Value::Str("host".into()), Value::Str("h".into())),
            (Value::Str("password".into()), Value::Str("s3cret".into())),
        ]),
    );
    let files = w.names["files"];
    let snap = w.p.snapshot(&|t| {
        (t != files).then(|| TabRecord {
            title: "db".into(),
            conn: w.conns.get(&t).cloned(),
            inner: String::new(),
        })
    });
    assert!(!format!("{snap:?}").contains("s3cret"));
    let tabs = snap.get("tabs").unwrap();
    assert!(matches!(tabs, Value::Seq(t) if t.len() == 1));
}

#[test]
fn json_for_qml_lists_panes_and_windows() {
    let mut w = Win::new();
    let a = w.tab("a");
    w.tab("b");
    w.split(Orientation::Vertical);
    w.p.detach_tab(a).unwrap();
    // `a` was alone in the main pane, which now goes; the other pane takes over as main.
    let j = w.p.to_json();
    assert_eq!(j["root"]["orientation"], "h");
    let main = &j["root"]["children"][0];
    assert_eq!(main["main"], true);
    assert_eq!(main["current"], 0);
    assert_eq!(j["detached"][0]["tabs"], serde_json::json!([a]));
    assert_eq!(j["active"], 1);
}

#[test]
fn emptied_main_pane_goes_when_others_remain() {
    let mut w = Win::new();
    w.tab("a");
    w.tab("b");
    w.split(Orientation::Horizontal);
    w.p.close_tab(w.names["a"], false).unwrap();
    assert_eq!(w.p.list_panes().len(), 1);
    assert_eq!(w.pane_titles(), [["b"]]);
    assert_eq!(w.p.main_pane(), w.p.list_panes()[0]);
}

#[test]
fn set_sizes_ignores_wrong_length_and_rejects_bad_path() {
    let mut p = Panes::new();
    p.set_sizes(&[], vec![1, 2]).unwrap();
    assert_eq!(p.to_json()["root"]["sizes"], serde_json::json!([0]));
    assert_eq!(p.set_sizes(&[0], vec![1]), Err(PaneError::BadPath));
}

/// A tab goes into an existing detached window and keeps its slot on a restart;
/// the window's current tab.
#[test]
fn detached_window_takes_new_tabs_in_place() {
    let mut p = Panes::new();
    let a = p.new_tab_id();
    p.add_tab(0, a, None).expect("main");
    let w = p.detach_tab(a).expect("detach");
    let (b, c) = (p.new_tab_id(), p.new_tab_id());
    p.add_window_tab(w, b, None).expect("window");
    assert_eq!(p.window_tabs(w), Some(&[a, b][..]));
    p.set_window_current(w, 0).expect("current");
    p.close_tab(a, true).expect("restart keeps the window");
    p.add_window_tab(w, c, Some(0)).expect("same slot");
    assert_eq!(p.window_tabs(w), Some(&[c, b][..]));
    assert_eq!(p.locate(c), Some(Location::Window(w)));
    assert_eq!(p.active(), 0);
    assert_eq!(
        p.add_window_tab(99, 7, None),
        Err(PaneError::NoSuchWindow(99))
    );
    assert_eq!(p.set_window_current(w, 5), Err(PaneError::BadIndex));
}

#[test]
fn dock_names_round_trip() {
    for d in [
        Dock::Center,
        Dock::Left,
        Dock::Right,
        Dock::Top,
        Dock::Bottom,
    ] {
        assert_eq!(Dock::parse(d.name()), Some(d));
    }
    assert_eq!(Dock::parse("middle"), None);
    assert_eq!(Dock::parse(""), None);
}

#[test]
fn hit_zone_cross_sectors_and_small_panes() {
    // Pane 400 x 300 (the guide cross fits).
    assert_eq!(
        hit_zone(400.0, 300.0, 200.0, 150.0, 40.0),
        Some(Dock::Center)
    );
    assert_eq!(hit_zone(400.0, 300.0, 150.0, 150.0, 40.0), Some(Dock::Left));
    assert_eq!(
        hit_zone(400.0, 300.0, 250.0, 150.0, 40.0),
        Some(Dock::Right)
    );
    assert_eq!(hit_zone(400.0, 300.0, 200.0, 100.0, 40.0), Some(Dock::Top));
    assert_eq!(
        hit_zone(400.0, 300.0, 200.0, 200.0, 40.0),
        Some(Dock::Bottom)
    );
    // Sectors outside the cross.
    assert_eq!(hit_zone(400.0, 300.0, 10.0, 150.0, 40.0), Some(Dock::Left));
    assert_eq!(
        hit_zone(400.0, 300.0, 390.0, 150.0, 40.0),
        Some(Dock::Right)
    );
    assert_eq!(hit_zone(400.0, 300.0, 200.0, 10.0, 40.0), Some(Dock::Top));
    assert_eq!(
        hit_zone(400.0, 300.0, 200.0, 290.0, 40.0),
        Some(Dock::Bottom)
    );
    assert_eq!(hit_zone(400.0, 300.0, 100.0, 100.0, 40.0), Some(Dock::Left));
    assert_eq!(hit_zone(400.0, 300.0, 100.0, 50.0, 40.0), Some(Dock::Top));
    assert_eq!(
        hit_zone(400.0, 300.0, 150.0, 110.0, 40.0),
        Some(Dock::Center)
    );
    // Outside the pane.
    assert_eq!(hit_zone(400.0, 300.0, -1.0, 5.0, 40.0), None);
    assert_eq!(hit_zone(400.0, 300.0, 401.0, 5.0, 40.0), None);
    assert_eq!(hit_zone(400.0, 300.0, 5.0, 301.0, 40.0), None);
    // Pane 100 x 100 (the cross does not fit, only sectors).
    assert_eq!(hit_zone(100.0, 100.0, 50.0, 50.0, 40.0), Some(Dock::Center));
    assert_eq!(hit_zone(100.0, 100.0, 45.0, 50.0, 40.0), Some(Dock::Center));
    assert_eq!(hit_zone(100.0, 100.0, 5.0, 50.0, 40.0), Some(Dock::Left));
    // Degenerate.
    assert_eq!(hit_zone(0.0, 0.0, 0.0, 0.0, 40.0), None);
}

// --- dragging tabs (move_tab, dock_at_edge) ---------------------------------------------

fn id(w: &Win, t: &str) -> TabId {
    w.names[t]
}

fn pane_of(l: Location) -> PaneId {
    match l {
        Location::Pane(p) => p,
        Location::Window(_) => panic!("expected a pane"),
    }
}

#[test]
fn move_tab_reorders_inside_a_pane() {
    let mut w = Win::new();
    for t in ["a", "b", "c"] {
        w.tab(t);
    }
    let c = id(&w, "c");
    let r =
        w.p.move_tab(c, Location::Pane(0), Dock::Center, Some(0), 1000);
    assert_eq!(r, Ok(Location::Pane(0)));
    assert_eq!(w.pane_titles(), [["c", "a", "b"]]);
    assert_eq!(w.p.current(0).unwrap(), Some(0));
}

#[test]
fn move_tab_to_the_middle_of_another_pane_removes_an_emptied_source() {
    let mut w = Win::new();
    w.tab("a");
    let other = w.split(Orientation::Horizontal);
    w.tab("b");
    assert_eq!(w.pane_titles(), [["a"], ["b"]]);
    let a = id(&w, "a");
    let r =
        w.p.move_tab(a, Location::Pane(other), Dock::Center, None, 1000);
    assert_eq!(r, Ok(Location::Pane(other)));
    assert_eq!(w.pane_titles(), [["b", "a"]]);
    assert_eq!(w.p.list_panes(), [other]);
    assert_eq!(
        w.p.main_pane(),
        other,
        "the main flag moves to the survivor"
    );
    assert_eq!(w.p.current(other).unwrap(), Some(1));
    assert_eq!(w.p.active(), other);
}

#[test]
fn move_tab_to_a_side_in_the_same_orientation_adds_a_sibling() {
    let mut w = Win::new();
    for t in ["a", "b", "c"] {
        w.tab(t);
    }
    let right = w.split(Orientation::Horizontal);
    assert_eq!(w.pane_titles(), [vec!["a", "b"], vec!["c"]]);
    let a = id(&w, "a");
    let r =
        w.p.move_tab(a, Location::Pane(right), Dock::Right, None, 1000);
    let new = pane_of(r.unwrap());
    assert_eq!(w.pane_titles(), [vec!["b"], vec!["c"], vec!["a"]]);
    assert_eq!(w.p.active(), new);
    let j = w.p.to_json();
    assert_eq!(j["root"]["orientation"], "h");
    assert_eq!(j["root"]["children"].as_array().unwrap().len(), 3);
    // Left puts it before the target instead.
    w.p.move_tab(a, Location::Pane(right), Dock::Left, None, 1000)
        .unwrap();
    assert_eq!(w.pane_titles(), [vec!["b"], vec!["a"], vec!["c"]]);
}

#[test]
fn move_tab_to_a_side_in_the_other_orientation_wraps_the_target() {
    let mut w = Win::new();
    for t in ["a", "b", "c"] {
        w.tab(t);
    }
    let right = w.split(Orientation::Horizontal);
    let a = id(&w, "a");
    w.p.move_tab(a, Location::Pane(right), Dock::Top, None, 600)
        .unwrap();
    assert_eq!(w.pane_titles(), [vec!["b"], vec!["a"], vec!["c"]]);
    let j = w.p.to_json();
    let nested = &j["root"]["children"][1];
    assert_eq!(nested["type"], "split");
    assert_eq!(nested["orientation"], "v");
    assert_eq!(nested["sizes"], serde_json::json!([300, 300]));
    assert_eq!(nested["children"][0]["tabs"][0], a);
}

#[test]
fn the_only_tab_of_a_pane_dropped_on_its_own_side_changes_nothing() {
    let mut w = Win::new();
    w.tab("a");
    let a = id(&w, "a");
    for d in [Dock::Left, Dock::Right, Dock::Top, Dock::Bottom] {
        let r = w.p.move_tab(a, Location::Pane(0), d, None, 1000);
        assert_eq!(r, Ok(Location::Pane(0)));
        assert_eq!(w.p.list_panes(), [0]);
    }
    // With a second tab the same drop splits the pane.
    w.tab("b");
    let r = w.p.move_tab(a, Location::Pane(0), Dock::Bottom, None, 1000);
    assert_ne!(r, Ok(Location::Pane(0)));
    assert_eq!(w.pane_titles(), [vec!["b"], vec!["a"]]);
}

#[test]
fn tabs_move_between_a_pane_and_a_detached_window_only_through_the_middle() {
    let mut w = Win::new();
    w.tab("a");
    w.tab("b");
    let b = id(&w, "b");
    let win = w.p.detach_tab(b).unwrap();
    let a = id(&w, "a");
    let r =
        w.p.move_tab(a, Location::Window(win), Dock::Center, Some(0), 1000);
    assert_eq!(r, Ok(Location::Window(win)));
    assert_eq!(w.titles(w.p.window_tabs(win).unwrap()), ["a", "b"]);
    assert!(
        w.p.tabs(0).unwrap().is_empty(),
        "the last pane stays, emptied"
    );
    // A side of a window is no split.
    let r =
        w.p.move_tab(b, Location::Window(win), Dock::Left, None, 1000);
    assert_eq!(r, Err(PaneError::BadDock));
    // Reorder inside the window, then back into the pane; the emptied window goes away.
    w.p.move_tab(b, Location::Window(win), Dock::Center, Some(0), 1000)
        .unwrap();
    assert_eq!(w.titles(w.p.window_tabs(win).unwrap()), ["b", "a"]);
    w.p.move_tab(a, Location::Pane(0), Dock::Center, None, 1000)
        .unwrap();
    w.p.move_tab(b, Location::Pane(0), Dock::Center, None, 1000)
        .unwrap();
    assert!(w.p.windows().is_empty());
    assert_eq!(w.pane_titles(), [["a", "b"]]);
    assert_eq!(
        w.p.move_tab(999, Location::Pane(0), Dock::Center, None, 1000),
        Err(PaneError::NoSuchTab(999))
    );
    assert_eq!(
        w.p.move_tab(a, Location::Pane(77), Dock::Center, None, 1000),
        Err(PaneError::NoSuchPane(77))
    );
}

#[test]
fn dock_at_edge_extends_wraps_or_flips_the_root() {
    let mut w = Win::new();
    w.tab("a");
    w.tab("b");
    // A root with one child takes the orientation of the edge.
    let b = id(&w, "b");
    let n = w.p.dock_at_edge(b, Dock::Left, 1000).unwrap();
    assert_eq!(w.pane_titles(), [["b"], ["a"]]);
    assert_eq!(w.p.active(), n);
    let j = w.p.to_json();
    assert_eq!(j["root"]["orientation"], "h");
    assert_eq!(j["root"]["sizes"][0], 500);
    // Same orientation: another sibling at the far end.
    w.tab("c");
    let c = id(&w, "c");
    w.p.dock_at_edge(c, Dock::Right, 1000).unwrap();
    assert_eq!(w.pane_titles(), [vec!["b"], vec!["a"], vec!["c"]]);
    assert_eq!(
        w.p.to_json()["root"]["children"].as_array().unwrap().len(),
        3
    );
    // Other orientation: the old root becomes the second child of a new one.
    let a = id(&w, "a");
    w.p.dock_at_edge(a, Dock::Top, 1000).unwrap();
    assert_eq!(w.pane_titles(), [vec!["a"], vec!["b"], vec!["c"]]);
    let j = w.p.to_json();
    assert_eq!(j["root"]["orientation"], "v");
    assert_eq!(j["root"]["children"][0]["type"], "pane");
    assert_eq!(j["root"]["children"][1]["type"], "split");
    assert_eq!(j["root"]["children"][1]["orientation"], "h");
    assert_eq!(j["root"]["sizes"], serde_json::json!([500, 500]));
}

#[test]
fn dock_at_edge_rejects_the_middle_and_ignores_a_lone_tab() {
    let mut w = Win::new();
    w.tab("a");
    let a = id(&w, "a");
    assert_eq!(
        w.p.dock_at_edge(a, Dock::Center, 1000),
        Err(PaneError::BadDock)
    );
    assert_eq!(w.p.dock_at_edge(a, Dock::Left, 1000), Ok(0));
    assert_eq!(w.p.list_panes(), [0]);
    assert_eq!(
        w.p.dock_at_edge(404, Dock::Left, 1000),
        Err(PaneError::NoSuchTab(404))
    );
    // From a detached window to an edge.
    w.tab("b");
    let b = id(&w, "b");
    w.p.detach_tab(b).unwrap();
    w.p.dock_at_edge(b, Dock::Bottom, 1000).unwrap();
    assert!(w.p.windows().is_empty());
    assert_eq!(w.pane_titles(), [vec!["a"], vec!["b"]]);
}

#[test]
fn the_snapshot_after_moves_restores_to_the_same_shape() {
    let mut w = Win::new();
    for t in ["a", "b", "c"] {
        w.tab(t);
    }
    let right = w.split(Orientation::Horizontal);
    let a = id(&w, "a");
    w.p.move_tab(a, Location::Pane(right), Dock::Top, None, 600)
        .unwrap();
    let b = id(&w, "b");
    w.p.dock_at_edge(b, Dock::Bottom, 800).unwrap();
    let snap = w.snapshot();
    let restored = restore(&snap, &[]);
    let titles: Vec<Vec<String>> = restored
        .panes
        .list_panes()
        .iter()
        .map(|&p| {
            restored
                .panes
                .tabs(p)
                .unwrap()
                .iter()
                .map(|t| {
                    restored
                        .tabs
                        .iter()
                        .find(|r| r.tab == *t)
                        .unwrap()
                        .title
                        .clone()
                })
                .collect()
        })
        .collect();
    assert_eq!(titles, w.pane_titles());
}
