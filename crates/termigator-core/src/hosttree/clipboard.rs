//! Rename of an entry and the host tree's own clipboard (Copy / Cut / Paste of entries and
//! folders). The clipboard lives in memory only: entries never reach the system clipboard, so a
//! copied password stays where the config keeps it (a copy holds what Duplicate would).

use super::*;
use crate::proto::gateway::find_gateway;

/// Rename entry `entry` to `name` (its `label`). Its place in the list, its folder and every
/// other setting stay; RDP / VNC entries that use it as their `ssh_gateway` follow the new name.
/// Refused: a blank name, and one that another entry of the same folder already shows.
pub fn rename_entry(cfg: &mut Value, entry: EntryRef, name: &str) -> Result<EntryRef, String> {
    let name = strip_space(name);
    if name.is_empty() {
        return Err("A name cannot be empty.".to_owned());
    }
    let key = entry.kind.key();
    let mut lst = list(cfg, key);
    let Some(cur) = lst.get(entry.index).filter(|e| is_dict(e)).cloned() else {
        return Err("The entry no longer exists.".to_owned());
    };
    let old = entry_label(&cur);
    if old == name && strip_space(&text(&cur, "label")) == name {
        return Ok(entry);
    }
    let group = group_of(&cur);
    let taken = [EntryKind::Connection, EntryKind::Favorite]
        .into_iter()
        .flat_map(|k| {
            list(cfg, k.key())
                .into_iter()
                .enumerate()
                .map(move |(i, e)| (EntryRef { kind: k, index: i }, e))
        })
        .any(|(r, e)| {
            r != entry && is_dict(&e) && group_of(&e) == group && entry_label(&e) == name
        });
    if taken {
        return Err(format!("“{name}” already exists in {}.", place(&group)));
    }
    let conns = list(cfg, "connections");
    // Gateways are looked up by label: the entries reaching this one keep reaching it.
    let gateway_users: Vec<usize> = if entry.kind == EntryKind::Connection {
        let me = &conns[entry.index];
        conns
            .iter()
            .enumerate()
            .filter(|(_, c)| {
                find_gateway(&conns, c.get("ssh_gateway").filter(|g| truthy(g)))
                    .is_some_and(|g| std::ptr::eq(g, me))
            })
            .map(|(i, _)| i)
            .collect()
    } else {
        Vec::new()
    };
    let new_label = Value::Str(name.to_owned());
    let other_ssh_named = conns.iter().enumerate().any(|(i, c)| {
        i != entry.index && find_gateway(std::slice::from_ref(c), Some(&new_label)).is_some()
    });
    if !gateway_users.is_empty() && other_ssh_named {
        return Err(format!(
            "Another SSH entry is already called “{name}”; the entries using “{old}” as their SSH gateway would reach it instead."
        ));
    }
    if let Some(e) = lst.get_mut(entry.index) {
        set_key(e, "label", Value::Str(name.to_owned()));
    }
    set_list(cfg, key, lst);
    if !gateway_users.is_empty() {
        let mut conns = list(cfg, "connections");
        for i in gateway_users {
            if let Some(c) = conns.get_mut(i) {
                set_key(c, "ssh_gateway", Value::Str(name.to_owned()));
            }
        }
        set_list(cfg, "connections", conns);
    }
    Ok(entry)
}

/// "the top level" or “folder”, for messages.
fn place(group: &str) -> String {
    if group.is_empty() {
        "the top level".to_owned()
    } else {
        format!("“{group}”")
    }
}

/// One copied or cut item.
#[derive(Debug, Clone, PartialEq)]
pub enum ClipItem {
    /// The entry as it was copied; `at` is where it was then (a cut finds it again by content).
    Entry { at: EntryRef, value: Value },
    /// A folder path; its contents are taken when pasting.
    Folder(String),
}

/// What Copy / Cut put on the tree's clipboard.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Clip {
    pub items: Vec<ClipItem>,
    /// Cut: Paste moves (keeping each entry's place in its list and every setting) and empties
    /// the clipboard; Copy: Paste adds copies and can be repeated.
    pub cut: bool,
}

impl Clip {
    /// The clipboard for `entries` and `folders`; what a picked folder holds already is left out.
    pub fn new(cfg: &Value, entries: &[EntryRef], folders: &[String], cut: bool) -> Self {
        let folders: Vec<String> = folders
            .iter()
            .map(|f| normalize_group_path(f))
            .filter(|f| !f.is_empty())
            .collect();
        let covered = |g: &str, own: Option<&str>| {
            folders
                .iter()
                .any(|f| Some(f.as_str()) != own && is_within(g, f))
        };
        let mut items = Vec::new();
        for f in &folders {
            let item = ClipItem::Folder(f.clone());
            if !covered(f, Some(f)) && !items.contains(&item) {
                items.push(item);
            }
        }
        for &at in entries {
            let Some(value) = list(cfg, at.kind.key())
                .get(at.index)
                .filter(|e| is_dict(e))
                .cloned()
            else {
                continue;
            };
            if covered(&group_of(&value), None)
                || items
                    .iter()
                    .any(|i| matches!(i, ClipItem::Entry { at: a, .. } if *a == at))
            {
                continue;
            }
            items.push(ClipItem::Entry { at, value });
        }
        Self { items, cut }
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// The cut entry `e` (by its place, as long as it is still the same entry there).
    pub fn holds_entry(&self, cfg: &Value, e: EntryRef) -> bool {
        self.items.iter().any(|i| match i {
            ClipItem::Entry { .. } => locate(cfg, i) == Some(e),
            ClipItem::Folder(_) => false,
        })
    }

    pub fn holds_folder(&self, path: &str) -> bool {
        self.items
            .iter()
            .any(|i| matches!(i, ClipItem::Folder(f) if f == path))
    }
}

/// Where a clipboard entry is now: its old place if it still holds the same entry, else the
/// first equal one.
fn locate(cfg: &Value, item: &ClipItem) -> Option<EntryRef> {
    let ClipItem::Entry { at, value } = item else {
        return None;
    };
    let lst = list(cfg, at.kind.key());
    if lst.get(at.index) == Some(value) {
        return Some(*at);
    }
    lst.iter().position(|e| e == value).map(|index| EntryRef {
        kind: at.kind,
        index,
    })
}

/// What Paste did, to select afterwards.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Pasted {
    /// The first pasted entry (when no folder was pasted).
    pub entry: Option<EntryRef>,
    /// The first pasted folder's new path.
    pub folder: Option<String>,
    /// Items pasted.
    pub count: usize,
}

/// `base`, else `base (copy)`, `base (copy 2)`, … — the first that `taken` does not refuse.
fn free_name(base: &str, taken: impl Fn(&str) -> bool) -> String {
    if !taken(base) {
        return base.to_owned();
    }
    let mut n = 1;
    loop {
        let name = if n == 1 {
            format!("{base} (copy)")
        } else {
            format!("{base} (copy {n})")
        };
        if !taken(&name) {
            return name;
        }
        n += 1;
    }
}

fn join(parent: &str, leaf: &str) -> String {
    if parent.is_empty() {
        leaf.to_owned()
    } else {
        format!("{parent}/{leaf}")
    }
}

fn leaf(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

fn parent_of(path: &str) -> &str {
    path.rsplit_once('/').map_or("", |(p, _)| p)
}

/// Labels shown in folder `group` (either list).
fn labels_in(cfg: &Value, group: &str) -> Vec<String> {
    ["connections", "favorites"]
        .iter()
        .flat_map(|k| list(cfg, k))
        .filter(|e| is_dict(e) && group_of(e) == group)
        .map(|e| entry_label(&e))
        .collect()
}

/// Paste `clip` into folder `dest` (`""` = top level). Checks every item before changing
/// anything: a folder into itself or below it, a cut folder whose name the destination has
/// already, and an item that is gone. Copies get a free name (`… (copy)`); moves keep theirs.
/// A copied entry is pasted as it was copied (also when it has changed or gone since); a cut
/// one must still be there.
pub fn paste(
    cfg: &mut Value,
    state: &mut TreeState,
    clip: &Clip,
    dest: &str,
) -> Result<Pasted, String> {
    let dest = normalize_group_path(dest);
    let groups = all_group_names(cfg);
    let mut entries = Vec::new();
    for item in &clip.items {
        match item {
            ClipItem::Folder(f) => {
                if !groups.contains(f) {
                    return Err(format!("The folder “{f}” no longer exists."));
                }
                if is_within(&dest, f) {
                    return Err(format!(
                        "Cannot paste the folder “{f}” into itself or one of its subfolders."
                    ));
                }
                let target = join(&dest, leaf(f));
                if clip.cut && target != *f && groups.contains(&target) {
                    return Err(format!(
                        "A folder “{}” already exists in {}.",
                        leaf(f),
                        place(&dest)
                    ));
                }
            }
            ClipItem::Entry { value, .. } if clip.cut => match locate(cfg, item) {
                Some(e) => entries.push(e),
                None => {
                    return Err(format!(
                        "“{}” no longer exists or has changed since it was cut.",
                        entry_label(value)
                    ));
                }
            },
            ClipItem::Entry { .. } => {}
        }
    }
    let mut out = Pasted::default();
    let mut entries = entries.into_iter();
    for item in &clip.items {
        match item {
            ClipItem::Folder(f) => {
                let path = if clip.cut {
                    if parent_of(f) != dest {
                        folder_moved(cfg, state, f, &dest)?;
                    }
                    join(&dest, leaf(f))
                } else {
                    copy_folder(cfg, f, &dest)
                };
                out.folder.get_or_insert(path);
            }
            ClipItem::Entry { at, value } => {
                let pasted = if clip.cut {
                    let Some(e) = entries.next() else { continue };
                    move_to_folder(cfg, e, &dest)
                } else {
                    copy_entry(cfg, at.kind, value, &dest)
                };
                out.entry.get_or_insert(pasted);
            }
        }
        out.count += 1;
    }
    if !dest.is_empty() {
        state.reveal(&dest);
    }
    if let Some(f) = &out.folder {
        state.reveal(parent_of(f));
    }
    Ok(out)
}

/// A cut entry into `dest`: only its `group` changes.
fn move_to_folder(cfg: &mut Value, e: EntryRef, dest: &str) -> EntryRef {
    let mut lst = list(cfg, e.kind.key());
    if let Some(v) = lst.get_mut(e.index) {
        set_group(v, dest);
    }
    set_list(cfg, e.kind.key(), lst);
    e
}

/// A copy of entry `value` at the end of its list, in `dest`, with a name free there.
fn copy_entry(cfg: &mut Value, kind: EntryKind, value: &Value, dest: &str) -> EntryRef {
    let key = kind.key();
    let mut lst = list(cfg, key);
    let mut copy = value.clone();
    let taken = labels_in(cfg, dest);
    let label = free_name(&entry_label(&copy), |n| taken.iter().any(|t| t == n));
    set_group(&mut copy, dest);
    set_key(&mut copy, "label", Value::Str(label));
    lst.push(copy);
    let index = lst.len() - 1;
    set_list(cfg, key, lst);
    EntryRef { kind, index }
}

/// A copy of folder `src` with everything in it (entries, subfolders, environments) inside
/// `dest`, under a free name. Returns the new path.
fn copy_folder(cfg: &mut Value, src: &str, dest: &str) -> String {
    let groups = all_group_names(cfg);
    let path = free_name(&join(dest, leaf(src)), |p| groups.iter().any(|g| g == p));
    let map = |g: &str| map_group_on_rename(g, src, &path);
    for key in ["connections", "favorites"] {
        let mut lst = list(cfg, key);
        let copies: Vec<Value> = lst
            .iter()
            .filter(|e| is_dict(e) && is_within(&group_of(e), src))
            .map(|e| {
                let mut c = e.clone();
                set_group(&mut c, &map(&group_of(e)));
                c
            })
            .collect();
        if !copies.is_empty() {
            lst.extend(copies);
            set_list(cfg, key, lst);
        }
    }
    // The explicit (possibly empty) folders in `src`; an empty `src` itself is one of them.
    let mut explicit = explicit_groups(cfg);
    let below: Vec<String> = explicit
        .iter()
        .filter(|g| is_within(g, src))
        .map(|g| map(g))
        .collect();
    explicit.extend(below);
    set_explicit_groups(cfg, &explicit);
    let mut envs = group_environments(cfg);
    let copied: Vec<(String, String)> = envs
        .iter()
        .filter(|(g, _)| is_within(g, src))
        .map(|(g, e)| (map(g), e.clone()))
        .collect();
    if !copied.is_empty() {
        envs.extend(copied);
        set_group_environments(cfg, &envs);
    }
    path
}
