//! Flattened host tree: folders, filter, Recent, drop zones, mutations (ADR-10).
//!
//! The sidebar tree logic (list refresh and helpers, the folder and entry handlers) and the drop
//! semantics of the tree widget, without Qt.
//!
//! - [`build`] turns the config (`connections`, `favorites`, `groups`, `recent`), the expanded
//!   set ([`TreeState`]) and the filter text into the visible rows ([`Row`]: depth, kind, index,
//!   expanded, label, subtitle, badge, icon, error) plus the status line and empty-list hint.
//!   Order: Recent, folders (per-segment casefold, subfolders before entries), top-level entries.
//! - [`drop_zone`] maps the y position inside a row to above / on / below; [`resolve_drop`]
//!   turns a drop into a [`DropAction`] (what `SidebarTree.dropEvent` emits); [`apply_drop`]
//!   executes it on the config.
//! - The mutations (`entry_moved`, `entries_placed`, `apply_group_rename`, `ungroup_group`,
//!   `delete_group`, `duplicate_entry`, ...) change `connections` / `favorites` / `groups` /
//!   `recent` in place; persisting and re-selecting is the
//!   caller's job (some return the entry to select afterwards).
//!
//! What needs other modules is injected through [`Hooks`]: the launch-command error of an entry
//! (the command builders) and the serial summary. The default hooks report no error and the
//! bare device name. Quirks kept: the RDP / VNC subtitle loses its `RDP ·` / `VNC ·`
//! prefix unless an SSH gateway is set, and an error never shows as subtitle. The tooltip is not
//! produced here. Golden fixtures: `compat/golden/hosttree_*.json`.

use std::collections::{BTreeMap, BTreeSet};

use crate::config::doc::Value;
use crate::config::groups::{
    group_path_ancestors, map_group_on_relocate, map_group_on_rename, normalize_group_path,
    normalize_group_value,
};
use crate::config::model::{
    PROTOCOL_RDP, PROTOCOL_SERIAL, PROTOCOL_TELNET, PROTOCOL_VNC, TERMINAL_EXTERNAL,
    connection_protocol, proxy_jump_value, section, strip_space, terminal_mode_for_entry, truthy,
    value_str,
};

/// Number of Recent keys kept in `recent`.
pub const RECENT_KEEP: usize = 10;
/// Number of Recent entries shown.
pub const RECENT_SHOWN: usize = 5;
/// Fraction of the row height at the top / bottom that counts as "above" / "below".
pub const DROP_EDGE: f64 = 0.18;

const SELF_MOVE: &str = "Cannot move a folder into itself.";

/// Kind of a config entry (`connections` or `favorites`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntryKind {
    Connection,
    Favorite,
}

impl EntryKind {
    fn key(self) -> &'static str {
        match self {
            Self::Connection => "connections",
            Self::Favorite => "favorites",
        }
    }

    /// `"connection"` / `"favorite"` as in the drop payloads.
    pub fn name(self) -> &'static str {
        match self {
            Self::Connection => "connection",
            Self::Favorite => "favorite",
        }
    }

    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "connection" => Some(Self::Connection),
            "favorite" => Some(Self::Favorite),
            _ => None,
        }
    }
}

/// An entry of the config: kind and index in its list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EntryRef {
    pub kind: EntryKind,
    pub index: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RowKind {
    Recent,
    Group,
    Connection,
    Favorite,
}

/// Which icon the delegate draws.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Icon {
    Group,
    Recent,
    Connection,
    ConnectionError,
    /// RDP and VNC.
    Remote,
    Serial,
    Favorite,
    Distro(String),
    /// The user's pick (`icon:` of the entry), a theme icon name.
    Custom(String),
}

/// One visible row of the flattened tree.
#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    pub depth: usize,
    pub kind: RowKind,
    /// Config index for entries, `0` otherwise.
    pub index: usize,
    /// Folder path of a group row, `""` otherwise.
    pub path: String,
    /// Folder holding the row (`""` = top level; Recent and its children: `""`).
    pub folder: String,
    /// The Recent row and its children: neither a drag source nor a drop target.
    pub in_recent: bool,
    pub label: String,
    pub subtitle: String,
    /// Folder: entry count of the subtree; entry: protocol, `ext` or `""`; Recent: count.
    pub badge: String,
    pub icon: Icon,
    pub error: bool,
    pub expanded: bool,
    pub has_children: bool,
}

impl Row {
    pub fn entry(&self) -> Option<EntryRef> {
        let kind = match self.kind {
            RowKind::Connection => EntryKind::Connection,
            RowKind::Favorite => EntryKind::Favorite,
            _ => return None,
        };
        Some(EntryRef {
            kind,
            index: self.index,
        })
    }
}

/// Expansion state kept by the window (`_expanded_groups`, `_recent_expanded`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TreeState {
    pub expanded_groups: BTreeSet<String>,
    pub recent_expanded: bool,
}

impl Default for TreeState {
    fn default() -> Self {
        Self {
            expanded_groups: BTreeSet::new(),
            recent_expanded: true,
        }
    }
}

impl TreeState {
    /// `_on_group_expanded` / `_on_group_collapsed`: ignored while a filter is active.
    pub fn set_expanded(&mut self, row: &Row, expanded: bool, filter_active: bool) {
        if filter_active {
            return;
        }
        match row.kind {
            RowKind::Group if expanded => {
                self.expanded_groups.insert(row.path.clone());
            }
            RowKind::Group => {
                self.expanded_groups.remove(&row.path);
            }
            RowKind::Recent => self.recent_expanded = expanded,
            _ => {}
        }
    }

    fn reveal(&mut self, group: &str) {
        self.expanded_groups.extend(group_path_ancestors(group));
    }
}

/// What depends on other modules.
pub trait Hooks {
    /// The error of building the entry's launch command (`_build_conn_command(c)[1]`).
    fn build_error(&self, _conn: &Value) -> Option<String> {
        None
    }

    /// `serial_summary(conn)`.
    fn serial_summary(&self, conn: &Value) -> String {
        text(conn, "device")
    }
}

impl Hooks for () {}

/// The built tree.
#[derive(Debug, Clone, PartialEq)]
pub struct Tree {
    pub rows: Vec<Row>,
    /// Entries passing the filter (also those in collapsed folders).
    pub entry_count: usize,
    pub folder_count: usize,
    /// Text of the empty-list hint; `None` when the hint is hidden.
    pub hint: Option<String>,
    pub status: String,
}

// ---- value helpers ---------------------------------------------------------------------

/// `str(entry.get(key) or "")`.
fn text(v: &Value, key: &str) -> String {
    match v.get(key) {
        Some(x) if truthy(x) => value_str(x),
        _ => String::new(),
    }
}

fn set_key(map: &mut Value, key: &str, value: Value) {
    let Value::Map(pairs) = map else { return };
    match pairs
        .iter_mut()
        .find(|(k, _)| matches!(k, Value::Str(s) if s == key))
    {
        Some(slot) => slot.1 = value,
        None => pairs.push((Value::Str(key.to_owned()), value)),
    }
}

fn remove_key(map: &mut Value, key: &str) {
    if let Value::Map(pairs) = map {
        pairs.retain(|(k, _)| !matches!(k, Value::Str(s) if s == key));
    }
}

fn list(cfg: &Value, key: &str) -> Vec<Value> {
    match cfg.get(key) {
        Some(Value::Seq(v)) => v.clone(),
        _ => Vec::new(),
    }
}

fn set_list(cfg: &mut Value, key: &str, items: Vec<Value>) {
    set_key(cfg, key, Value::Seq(items));
}

fn is_dict(v: &Value) -> bool {
    matches!(v, Value::Map(_))
}

fn set_group(entry: &mut Value, group: &str) {
    if group.is_empty() {
        remove_key(entry, "group");
    } else {
        set_key(entry, "group", Value::Str(group.to_owned()));
    }
}

/// Entry's folder as a canonical path.
pub fn group_of(entry: &Value) -> String {
    normalize_group_value(entry.get("group").unwrap_or(&Value::Null))
}

/// Folders created explicitly (`groups:` in the config).
pub fn explicit_groups(cfg: &Value) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    if let Some(Value::Seq(items)) = cfg.get("groups") {
        for g in items {
            let p = normalize_group_value(g);
            if !p.is_empty() && !out.contains(&p) {
                out.push(p);
            }
        }
    }
    out
}

/// Write the explicit-folder list back; an empty list removes the key.
pub fn set_explicit_groups(cfg: &mut Value, paths: &[String]) {
    let mut uniq: Vec<String> = Vec::new();
    for p in paths {
        let p = normalize_group_path(p);
        if !p.is_empty() && !uniq.contains(&p) {
            uniq.push(p);
        }
    }
    if uniq.is_empty() {
        remove_key(cfg, "groups");
    } else {
        set_key(
            cfg,
            "groups",
            Value::Seq(uniq.into_iter().map(Value::Str).collect()),
        );
    }
}

/// Case folding approximated by lowercasing plus `ß` → `ss`.
fn casefold(s: &str) -> String {
    s.to_lowercase().replace('ß', "ss")
}

/// Per-segment casefold: parents sort before their children.
fn group_sort_key(path: &str) -> Vec<String> {
    path.split('/').map(casefold).collect()
}

/// Every folder path in play (entries with ancestors, explicit folders), sorted.
pub fn all_group_names(cfg: &Value) -> Vec<String> {
    let mut paths: BTreeSet<String> = BTreeSet::new();
    for key in ["connections", "favorites"] {
        for e in list(cfg, key).iter().filter(|e| is_dict(e)) {
            paths.extend(group_path_ancestors(&group_of(e)));
        }
    }
    for g in explicit_groups(cfg) {
        paths.extend(group_path_ancestors(&g));
    }
    let mut out: Vec<String> = paths.into_iter().collect();
    out.sort_by_key(|p| group_sort_key(p));
    out
}

/// Row label of a connection.
pub fn entry_label(c: &Value) -> String {
    let lbl = strip_space(&text(c, "label")).to_owned();
    if !lbl.is_empty() {
        return lbl;
    }
    let user = strip_space(&text(c, "user")).to_owned();
    let mut host = strip_space(&text(c, "host")).to_owned();
    if host.is_empty() {
        host = strip_space(&text(c, "device")).to_owned();
    }
    if !user.is_empty() {
        format!("{user}@{host}")
    } else if host.is_empty() {
        "connection".to_owned()
    } else {
        host
    }
}

/// Key of a connection in `recent`: `host|user|port`, lowercase.
pub fn recent_key(c: &Value) -> String {
    let mut host = text(c, "host");
    if host.is_empty() {
        host = text(c, "device");
    }
    format!(
        "{}|{}|{}",
        strip_space(&host).to_lowercase(),
        strip_space(&text(c, "user")).to_lowercase(),
        strip_space(&text(c, "port"))
    )
}

/// `_touch_recent` without the persist: the connection becomes the newest Recent key.
pub fn touch_recent(cfg: &mut Value, conn: &Value) {
    let key = recent_key(conn);
    let mut rec: Vec<Value> = list(cfg, "recent")
        .into_iter()
        .filter(|k| matches!(k, Value::Str(s) if *s != key))
        .collect();
    rec.insert(0, Value::Str(key));
    rec.truncate(RECENT_KEEP);
    set_list(cfg, "recent", rec);
}

/// `_forward_specs_from_yaml`.
fn forward_specs(val: Option<&Value>) -> Vec<String> {
    match val {
        None | Some(Value::Null) => Vec::new(),
        Some(Value::Seq(items)) => items
            .iter()
            .map(|x| strip_space(&value_str(x)).to_owned())
            .filter(|s| !s.is_empty())
            .collect(),
        Some(v) => {
            let owned = value_str(v);
            let s = strip_space(&owned);
            if s.is_empty() {
                Vec::new()
            } else if s.contains('\n') {
                s.lines()
                    .map(|l| strip_space(l).to_owned())
                    .filter(|l| !l.is_empty())
                    .collect()
            } else {
                vec![s.to_owned()]
            }
        }
    }
}

/// `format_proxy_jump_for_ui`.
fn proxy_jump_for_ui(conn: &Value) -> String {
    match proxy_jump_value(conn) {
        None | Some(Value::Null) => String::new(),
        Some(Value::Seq(items)) => items
            .iter()
            .map(|x| strip_space(&value_str(x)).to_owned())
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join(","),
        Some(v) => strip_space(&value_str(v)).to_owned(),
    }
}

fn matches_filter(needle: &str, parts: &[String]) -> bool {
    needle.is_empty()
        || casefold(
            &parts
                .iter()
                .filter(|p| !p.is_empty())
                .cloned()
                .collect::<Vec<_>>()
                .join(" "),
        )
        .contains(needle)
}

// ---- rows ------------------------------------------------------------------------------

struct Entry {
    group: String,
    row: Row,
}

fn blank_row(kind: RowKind) -> Row {
    Row {
        depth: 0,
        kind,
        index: 0,
        path: String::new(),
        folder: String::new(),
        in_recent: false,
        label: String::new(),
        subtitle: String::new(),
        badge: String::new(),
        icon: Icon::Connection,
        error: false,
        expanded: false,
        has_children: false,
    }
}

fn connection_row(cfg: &Value, c: &Value, i: usize, err: Option<&str>, hooks: &dyn Hooks) -> Row {
    let proto = connection_protocol(c);
    let label = entry_label(c);
    let host = text(c, "host");
    let user = text(c, "user");
    let default_port = match proto {
        PROTOCOL_RDP => "3389",
        PROTOCOL_VNC => "5900",
        PROTOCOL_TELNET => "23",
        _ => "22",
    };
    let mut target = if host.is_empty() {
        String::new()
    } else if user.is_empty() {
        host.clone()
    } else {
        format!("{user}@{host}")
    };
    if let Some(port) = c.get("port").filter(|p| !matches!(p, Value::Null)) {
        let port = value_str(port);
        let p = strip_space(&port);
        if !p.is_empty() && p != default_port && !target.is_empty() {
            target = format!("{target}:{port}");
        }
    }
    let gateway = c.get("ssh_gateway").filter(|g| truthy(g));
    let remote = proto == PROTOCOL_RDP || proto == PROTOCOL_VNC;
    // Two consecutive if-chains: the second one's `else` overwrites the first.
    let mut subtitle = String::new();
    if let Some(e) = err {
        subtitle = e.to_owned();
    } else if proto == PROTOCOL_RDP || proto == PROTOCOL_VNC {
        let name = if proto == PROTOCOL_RDP { "RDP" } else { "VNC" };
        subtitle = if target.is_empty() {
            name.to_owned()
        } else {
            format!("{name} · {target}")
        };
    }
    if let (None, true, Some(gw)) = (err, remote, gateway) {
        subtitle = format!("{subtitle} · via {}", value_str(gw));
    } else if proto == PROTOCOL_TELNET {
        subtitle = if target.is_empty() {
            "Telnet".to_owned()
        } else {
            format!("Telnet · {target}")
        };
    } else if proto == PROTOCOL_SERIAL {
        subtitle = format!("Serial · {}", hooks.serial_summary(c));
    } else {
        subtitle = target;
    }
    let distro = strip_space(&text(c, "distro")).to_owned();
    let icon = if err.is_some() {
        Icon::ConnectionError
    } else if let Some(i) = c.get("icon").filter(|i| truthy(i)) {
        Icon::Custom(value_str(i))
    } else if remote {
        Icon::Remote
    } else if proto == PROTOCOL_SERIAL {
        Icon::Serial
    } else if !distro.is_empty() {
        Icon::Distro(distro)
    } else {
        Icon::Connection
    };
    let badge = if matches!(
        proto,
        PROTOCOL_RDP | PROTOCOL_VNC | PROTOCOL_TELNET | PROTOCOL_SERIAL
    ) {
        proto.to_owned()
    } else if terminal_mode_for_entry(Some(c), cfg) == TERMINAL_EXTERNAL {
        "ext".to_owned()
    } else {
        String::new()
    };
    Row {
        index: i,
        label,
        subtitle,
        badge,
        icon,
        error: err.is_some(),
        ..blank_row(RowKind::Connection)
    }
}

/// The entry's own environment id (`environment:` in session.yaml), or "" for none.
/// A value that is no longer a configured category is still returned, so an old config keeps
/// its text until the user changes it.
pub fn environment_of(c: &Value) -> String {
    strip_space(&text(c, "environment")).to_owned()
}

/// Folder environments (`group_environments:` in config.yaml, folder path → environment id).
pub fn group_environments(cfg: &Value) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    if let Some(Value::Map(pairs)) = cfg.get("group_environments") {
        for (k, v) in pairs {
            let path = normalize_group_value(k);
            let raw = value_str(v);
            let raw = strip_space(&raw);
            if path.is_empty() || raw.is_empty() {
                continue;
            }
            let env = crate::config::environments::canonical_id(cfg, raw)
                .unwrap_or_else(|| raw.to_owned());
            out.insert(path, env);
        }
    }
    out
}

fn set_group_environments(cfg: &mut Value, map: &BTreeMap<String, String>) {
    if map.is_empty() {
        remove_key(cfg, "group_environments");
    } else {
        let pairs = map
            .iter()
            .map(|(k, v)| (Value::Str(k.clone()), Value::Str(v.clone())))
            .collect();
        set_key(cfg, "group_environments", Value::Map(pairs));
    }
}

/// Sets (or with "" / an unknown value clears) the environment of folder `path`.
pub fn set_group_environment(cfg: &mut Value, path: &str, env: &str) {
    let path = normalize_group_path(path);
    if path.is_empty() {
        return;
    }
    let mut map = group_environments(cfg);
    match crate::config::environments::canonical_id(cfg, env) {
        Some(e) => map.insert(path, e),
        None => map.remove(&path),
    };
    set_group_environments(cfg, &map);
}

/// Rewrites the folder keys after a rename, a dissolve or a delete (`None` drops the key).
pub(crate) fn remap_group_environments(cfg: &mut Value, f: impl Fn(&str) -> Option<String>) {
    let map = group_environments(cfg);
    if map.is_empty() {
        return;
    }
    let mut out = BTreeMap::new();
    for (k, v) in map {
        if let Some(n) = f(&k).filter(|n| !n.is_empty()) {
            out.entry(n).or_insert(v);
        }
    }
    set_group_environments(cfg, &out);
}

/// The environment folder `path` has or inherits from the nearest marked parent, with that
/// folder's path; `("", "")` when none is marked.
pub fn folder_environment(cfg: &Value, path: &str) -> (String, String) {
    let map = group_environments(cfg);
    let mut anc = group_path_ancestors(&normalize_group_path(path));
    anc.reverse(); // nearest first
    anc.into_iter()
        .find_map(|p| map.get(&p).map(|e| (e.clone(), p)))
        .unwrap_or_else(|| (String::new(), String::new()))
}

/// The environment an entry is shown and opened with: its own, else its folder's
/// (inherited down the folder tree).
pub fn effective_environment(cfg: &Value, c: &Value) -> String {
    match environment_of(c) {
        own if !own.is_empty() => own,
        _ => folder_environment(cfg, &group_of(c)).0,
    }
}

fn connection_matches(needle: &str, c: &Value, label: &str, err: Option<&str>, env: &str) -> bool {
    let host = text(c, "host");
    let user = text(c, "user");
    let port = match c.get("port") {
        None | Some(Value::Null) => String::new(),
        Some(p) if strip_space(&value_str(p)).is_empty() => String::new(),
        Some(p) => value_str(p),
    };
    let tags = match c.get("tags") {
        Some(Value::Seq(t)) => t.iter().map(value_str).collect::<Vec<_>>().join(" "),
        _ => text(c, "tags"),
    };
    let parts = [
        label.to_owned(),
        host.clone(),
        user.clone(),
        format!("{user}@{host}"),
        err.unwrap_or_default().to_owned(),
        port,
        text(c, "identity_file"),
        group_of(c),
        connection_protocol(c).to_owned(),
        text(c, "domain"),
        text(c, "resolution"),
        proxy_jump_for_ui(c),
        forward_specs(c.get("local_forwards")).join(" "),
        forward_specs(c.get("remote_forwards")).join(" "),
        text(c, "remote_command"),
        text(c, "dynamic_forward"),
        text(c, "strict_host_key_checking"),
        tags,
        env.to_owned(),
    ];
    matches_filter(needle, &parts)
}

fn favorite_row(cfg: &Value, f: &Value, j: usize, label: String, cmd: String) -> Row {
    let ext = terminal_mode_for_entry(Some(f), cfg) == TERMINAL_EXTERNAL;
    Row {
        index: j,
        label,
        subtitle: cmd,
        badge: if ext { "ext".to_owned() } else { String::new() },
        icon: Icon::Favorite,
        ..blank_row(RowKind::Favorite)
    }
}

struct Emit<'a> {
    needle_active: bool,
    state: &'a TreeState,
    children: BTreeMap<String, Vec<String>>,
    entries: BTreeMap<String, Vec<Row>>,
    totals: BTreeMap<String, usize>,
}

impl Emit<'_> {
    fn emit(&self, parent: &str, depth: usize, out: &mut Vec<Row>) {
        for path in self.children.get(parent).into_iter().flatten() {
            let expanded = self.needle_active || self.state.expanded_groups.contains(path);
            let sub = self.children.contains_key(path.as_str());
            let own = self.entries.get(path).map_or(0, Vec::len);
            out.push(Row {
                depth,
                path: path.clone(),
                folder: parent.to_owned(),
                label: path.rsplit('/').next().unwrap_or_default().to_owned(),
                badge: self.totals.get(path).copied().unwrap_or(0).to_string(),
                icon: Icon::Group,
                expanded,
                has_children: sub || own > 0,
                ..blank_row(RowKind::Group)
            });
            if expanded {
                self.emit(path, depth + 1, out);
                self.push_entries(path, depth + 1, out);
            }
        }
    }

    fn push_entries(&self, folder: &str, depth: usize, out: &mut Vec<Row>) {
        for row in self.entries.get(folder).into_iter().flatten() {
            out.push(Row {
                depth,
                folder: folder.to_owned(),
                ..row.clone()
            });
        }
    }
}

/// Build the visible rows (`_refresh_list`). `filter` is the raw text of the search box.
pub fn build(cfg: &Value, state: &TreeState, filter: &str, hooks: &dyn Hooks) -> Tree {
    let needle = casefold(strip_space(filter));
    let conns = list(cfg, "connections");
    let favs = list(cfg, "favorites");
    let mut stash: Vec<Entry> = Vec::new();

    for (i, c) in conns.iter().enumerate().filter(|(_, c)| is_dict(c)) {
        let err = hooks.build_error(c);
        let env = effective_environment(cfg, c);
        if !connection_matches(&needle, c, &entry_label(c), err.as_deref(), &env) {
            continue;
        }
        stash.push(Entry {
            group: group_of(c),
            row: connection_row(cfg, c, i, err.as_deref(), hooks),
        });
    }
    for (j, f) in favs.iter().enumerate().filter(|(_, f)| is_dict(f)) {
        let label = {
            let l = text(f, "label");
            if l.is_empty() { "?".to_owned() } else { l }
        };
        let cmd = strip_space(&text(f, "command")).to_owned();
        if cmd.is_empty() || !matches_filter(&needle, &[label.clone(), cmd.clone(), group_of(f)]) {
            continue;
        }
        stash.push(Entry {
            group: group_of(f),
            row: favorite_row(cfg, f, j, label, cmd),
        });
    }
    let entry_count = stash.len();

    let mut rows: Vec<Row> = Vec::new();

    // Recent: shown above the folders, not while searching.
    let show_recent = section(cfg, "window").get("show_recent").is_none_or(truthy);
    if needle.is_empty() && show_recent {
        let mut by_key: BTreeMap<String, usize> = BTreeMap::new();
        for (i, c) in conns.iter().enumerate().filter(|(_, c)| is_dict(c)) {
            by_key.entry(recent_key(c)).or_insert(i);
        }
        let recent: Vec<usize> = list(cfg, "recent")
            .iter()
            .filter_map(|k| match k {
                Value::Str(s) => by_key.get(s).copied(),
                _ => None,
            })
            .take(RECENT_SHOWN)
            .collect();
        if !recent.is_empty() {
            rows.push(Row {
                in_recent: true,
                label: "Recent".to_owned(),
                badge: recent.len().to_string(),
                icon: Icon::Recent,
                expanded: state.recent_expanded,
                has_children: true,
                ..blank_row(RowKind::Recent)
            });
            if state.recent_expanded {
                for i in recent {
                    let c = &conns[i];
                    let err = hooks.build_error(c);
                    rows.push(Row {
                        depth: 1,
                        in_recent: true,
                        ..connection_row(cfg, c, i, err.as_deref(), hooks)
                    });
                }
            }
        }
    }

    // Folders holding visible entries (with ancestors) plus explicit ones when not filtering.
    let mut folder_paths: BTreeSet<String> = BTreeSet::new();
    for e in &stash {
        folder_paths.extend(group_path_ancestors(&e.group));
    }
    if needle.is_empty() {
        for g in explicit_groups(cfg) {
            folder_paths.extend(group_path_ancestors(&g));
        }
    }
    let mut totals: BTreeMap<String, usize> = BTreeMap::new();
    let mut entries: BTreeMap<String, Vec<Row>> = BTreeMap::new();
    for e in stash {
        for anc in group_path_ancestors(&e.group) {
            *totals.entry(anc).or_insert(0) += 1;
        }
        entries.entry(e.group).or_default().push(e.row);
    }
    let mut sorted: Vec<String> = folder_paths.iter().cloned().collect();
    sorted.sort_by_key(|p| group_sort_key(p));
    let mut children: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for p in &sorted {
        let parent = p.rsplit_once('/').map_or("", |(a, _)| a).to_owned();
        children.entry(parent).or_default().push(p.clone());
    }
    let ctx = Emit {
        needle_active: !needle.is_empty(),
        state,
        children,
        entries,
        totals,
    };
    ctx.emit("", 0, &mut rows);
    ctx.push_entries("", 0, &mut rows);

    let any_raw = !conns.is_empty() || !favs.is_empty();
    let hint = if entry_count > 0 {
        None
    } else if !needle.is_empty() {
        Some("No results match the current search.".to_owned())
    } else if !any_raw {
        Some(
            "No saved hosts yet.\n\nUse + above the list (or File → New connection) to add one."
                .to_owned(),
        )
    } else {
        Some("Nothing to show — try adjusting the search, or add a list entry.".to_owned())
    };
    let status = if !needle.is_empty() {
        if entry_count == 1 {
            "1 result · search active".to_owned()
        } else {
            format!("{entry_count} result(s) · search active")
        }
    } else {
        let n = if entry_count == 1 {
            "1 entry".to_owned()
        } else {
            format!("{entry_count} entries")
        };
        match folder_paths.len() {
            0 => n,
            f => format!("{n} · {f} folder(s)"),
        }
    };
    Tree {
        rows,
        entry_count,
        folder_count: folder_paths.len(),
        hint,
        status,
    }
}

mod clipboard;
mod dnd;
mod from_tab;
mod mutations;

pub use clipboard::*;
pub use dnd::*;
pub use from_tab::*;
pub use mutations::*;
