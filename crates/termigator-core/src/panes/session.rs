//! The `last_session` snapshot values and [`restore`]: which tabs to open from a snapshot.

use super::*;

pub(super) fn map(pairs: Vec<(&str, Value)>) -> Value {
    Value::Map(
        pairs
            .into_iter()
            .map(|(k, v)| (Value::Str(k.into()), v))
            .collect(),
    )
}

pub(super) fn leaf_value(tabs: Vec<Value>, current: usize, main: bool) -> Value {
    map(vec![
        ("tabs", Value::Seq(tabs)),
        ("current", Value::Int(current as i64)),
        ("main", Value::Bool(main)),
    ])
}

/// What a snapshot stores about one embedded terminal tab.
#[derive(Debug, Clone, PartialEq)]
pub struct TabRecord {
    /// The tab's title (`"Session"` when empty).
    pub title: String,
    /// The tab's connection mapping, if it has one; its `password` key is dropped.
    pub conn: Option<Value>,
    /// The inner shell command, used when there is no connection.
    pub inner: String,
}

impl TabRecord {
    pub(super) fn to_value(&self) -> Value {
        let title = if self.title.is_empty() {
            "Session"
        } else {
            &self.title
        };
        let mut pairs = vec![("title", Value::Str(title.to_owned()))];
        match &self.conn {
            Some(Value::Map(m)) => {
                let kept = m
                    .iter()
                    .filter(|(k, _)| !matches!(k, Value::Str(s) if s == "password"))
                    .cloned()
                    .collect();
                pairs.push(("conn", Value::Map(kept)));
            }
            _ => pairs.push(("inner", Value::Str(self.inner.clone()))),
        }
        map(pairs)
    }
}

/// How to open a restored tab.
#[derive(Debug, Clone, PartialEq)]
pub enum RestoreSource {
    /// Build the command from `entry`: the matching saved connection (`matched` is its index in
    /// the list passed to [`restore`]; it still holds the password) or else the snapshot's own.
    Connection {
        entry: Value,
        matched: Option<usize>,
    },
    /// A plain inner shell command.
    Inner(String),
}

/// A tab [`restore`] has already placed in the tree and the caller must now open.
#[derive(Debug, Clone, PartialEq)]
pub struct RestoreTab {
    pub tab: TabId,
    /// The pane it was reopened into (the main pane for detached ones, before detaching).
    pub pane: PaneId,
    pub title: String,
    pub source: RestoreSource,
    /// Already moved into a window of its own.
    pub detached: bool,
}

/// Result of [`restore`]. When opening a tab fails, call [`Panes::close_tab`] with `keep_pane`
/// (the pane stays in place).
#[derive(Debug, Clone)]
pub struct Restored {
    pub panes: Panes,
    pub tabs: Vec<RestoreTab>,
}

/// Rebuild the layout of `snap` and list the tabs to open, in this order:
/// pane by pane depth-first, then the detached ones. `connections` are the saved entries.
pub fn restore(snap: &Value, connections: &[Value]) -> Restored {
    let mut panes = Panes::new();
    let mut tabs = Vec::new();
    if !matches!(snap, Value::Map(_)) {
        return Restored { panes, tabs };
    }
    // The root starts empty; the main pane goes where the snapshot marks it (else first).
    panes.root.children.clear();
    panes.root.sizes.clear();
    panes.panes.clear();
    let mut placed_main = false;
    let mut root = std::mem::replace(
        &mut panes.root,
        Split {
            orientation: Orientation::Horizontal,
            sizes: Vec::new(),
            children: Vec::new(),
        },
    );
    let mut cx = Build {
        panes: &mut panes,
        tabs: &mut tabs,
        connections,
        placed_main: &mut placed_main,
    };
    cx.node(snap, &mut root, true);
    if !placed_main {
        root.children.insert(0, Node::Pane(0));
        root.sizes.insert(0, 0);
        panes.panes.insert(
            0,
            Pane {
                id: 0,
                main: true,
                list: TabList::default(),
            },
        );
    }
    panes.root = root;
    panes.active = 0;
    if let Some(Value::Seq(det)) = snap.get("detached") {
        for r in det {
            if let Some(t) = reopen(&mut panes, r, 0, connections)
                && panes.detach_tab(t.tab).is_ok()
            {
                tabs.push(RestoreTab {
                    detached: true,
                    ..t
                });
            }
        }
    }
    Restored { panes, tabs }
}

struct Build<'a> {
    panes: &'a mut Panes,
    tabs: &'a mut Vec<RestoreTab>,
    connections: &'a [Value],
    placed_main: &'a mut bool,
}

impl Build<'_> {
    fn node(&mut self, node: &Value, into: &mut Split, is_root: bool) {
        let kids = match (node.get("split"), node.get("children")) {
            (Some(_), Some(Value::Seq(k))) => Some(k),
            _ => None,
        };
        if let Some(kids) = kids {
            let vertical_or_h = match node.get("split") {
                Some(Value::Str(s)) if s == "h" => Orientation::Horizontal,
                _ => Orientation::Vertical,
            };
            let mut sub = Split {
                orientation: vertical_or_h,
                sizes: Vec::new(),
                children: Vec::new(),
            };
            if is_root && into.children.is_empty() {
                into.orientation = vertical_or_h;
                // Quirk: while the root is still empty a nested split flattens into it.
                for c in kids.iter().filter(|c| matches!(c, Value::Map(_))) {
                    self.node(c, into, true);
                }
                apply_sizes(node, into);
            } else {
                for c in kids.iter().filter(|c| matches!(c, Value::Map(_))) {
                    self.node(c, &mut sub, false);
                }
                apply_sizes(node, &mut sub);
                into.children.push(Node::Split(sub));
                into.sizes.push(0);
            }
            return;
        }
        let main = truthy(node.get("main").unwrap_or(&Value::Null)) && !*self.placed_main;
        let id = if main {
            *self.placed_main = true;
            0
        } else {
            let id = self.panes.next_pane;
            self.panes.next_pane += 1;
            id
        };
        self.panes.panes.push(Pane {
            id,
            main,
            list: TabList::default(),
        });
        into.children.push(Node::Pane(id));
        into.sizes.push(0);
        if let Some(Value::Seq(recs)) = node.get("tabs") {
            for r in recs {
                if let Some(t) = reopen(self.panes, r, id, self.connections) {
                    self.tabs.push(t);
                }
            }
        }
        if let Some(Value::Int(cur)) = node.get("current")
            && let Ok(cur) = usize::try_from(*cur)
        {
            let _ = self.panes.pane_mut(id).map(|p| {
                if cur < p.list.tabs.len() {
                    p.list.current = Some(cur);
                }
            });
        }
    }
}

fn apply_sizes(node: &Value, split: &mut Split) {
    if let Some(Value::Seq(sizes)) = node.get("sizes")
        && sizes.len() == split.children.len()
    {
        let ints: Option<Vec<i64>> = sizes
            .iter()
            .map(|v| match v {
                Value::Int(i) => Some(*i),
                Value::Float(f) if f.is_finite() => Some(f.trunc() as i64),
                _ => None,
            })
            .collect();
        if let Some(ints) = ints {
            split.sizes = ints;
        }
    }
}

/// `_reopen_tab_record`: a tab added to `pane`, or `None` for records that are not reopened.
fn reopen(
    panes: &mut Panes,
    rec: &Value,
    pane: PaneId,
    connections: &[Value],
) -> Option<RestoreTab> {
    if !matches!(rec, Value::Map(_)) {
        return None;
    }
    let title = match rec.get("title") {
        Some(t) if truthy(t) => value_str(t),
        _ => "Session".to_owned(),
    };
    let source = match rec.get("conn") {
        Some(conn @ Value::Map(_)) => {
            let key = |c: &Value| ["host", "user", "port", "label"].map(|k| c.get(k).cloned());
            let want = key(conn);
            let matched = connections.iter().position(|c| {
                matches!(c, Value::Map(_))
                    && key(c).iter().zip(&want).all(|(a, b)| match (a, b) {
                        (Some(a), Some(b)) => a.loose_eq(b),
                        (None, None) => true,
                        (Some(v), None) | (None, Some(v)) => matches!(v, Value::Null),
                    })
            });
            let entry = matched.map_or_else(|| conn.clone(), |i| connections[i].clone());
            if ![PROTOCOL_SSH, PROTOCOL_TELNET, PROTOCOL_SERIAL]
                .contains(&connection_protocol(&entry))
            {
                return None;
            }
            RestoreSource::Connection { entry, matched }
        }
        _ => {
            let inner = rec
                .get("inner")
                .filter(|v| truthy(v))
                .map(value_str)
                .unwrap_or_default();
            if strip_space(&inner).is_empty() {
                return None;
            }
            RestoreSource::Inner(inner)
        }
    };
    let tab = panes.new_tab_id();
    panes.add_tab(pane, tab, None).ok()?;
    Some(RestoreTab {
        tab,
        pane,
        title,
        source,
        detached: false,
    })
}
