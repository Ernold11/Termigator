//! Mutations of `connections` / `favorites` / `groups` / `recent`.

use super::*;

/// `_on_entry_dropped`: move an entry into a folder (`""` = top level). `Some` = changed.
pub fn entry_moved(
    cfg: &mut Value,
    state: &mut TreeState,
    entry: EntryRef,
    group: &str,
) -> Option<EntryRef> {
    let key = entry.kind.key();
    let mut lst = list(cfg, key);
    let e = lst.get_mut(entry.index).filter(|e| is_dict(e))?;
    let group = normalize_group_path(group);
    if group_of(e) == group {
        return None;
    }
    set_group(e, &group);
    set_list(cfg, key, lst);
    state.reveal(&group);
    Some(entry)
}

/// `_on_entries_placed`: the entries go into `group` and take the place next to `anchor`.
/// Returns the entry to select (the first moved block).
pub fn entries_placed(
    cfg: &mut Value,
    state: &mut TreeState,
    moving: &[EntryRef],
    group: &str,
    anchor: EntryRef,
    after: bool,
) -> Option<EntryRef> {
    let group = normalize_group_path(group);
    let mut select = None;
    for kind in [EntryKind::Connection, EntryKind::Favorite] {
        let key = kind.key();
        let lst = list(cfg, key);
        let idxs: BTreeSet<usize> = moving
            .iter()
            .filter(|m| m.kind == kind && m.index < lst.len())
            .map(|m| m.index)
            .collect();
        if idxs.is_empty() {
            continue;
        }
        let moved: Vec<Value> = idxs
            .iter()
            .map(|&i| {
                let mut e = lst[i].clone();
                set_group(&mut e, &group);
                e
            })
            .collect();
        let rest: Vec<usize> = (0..lst.len()).filter(|i| !idxs.contains(i)).collect();
        let pos = if anchor.kind == kind {
            rest.iter()
                .position(|&i| i == anchor.index)
                .map_or(rest.len(), |n| n + usize::from(after))
        } else {
            rest.len()
        };
        let mut out: Vec<Value> = rest[..pos].iter().map(|&i| lst[i].clone()).collect();
        out.extend(moved);
        out.extend(rest[pos..].iter().map(|&i| lst[i].clone()));
        set_list(cfg, key, out);
        select.get_or_insert(EntryRef { kind, index: pos });
    }
    if !group.is_empty() {
        state.reveal(&group);
    }
    select
}

/// `_apply_group_rename`: rename / move folder `old` to path `new` with its subtree.
pub fn apply_group_rename(
    cfg: &mut Value,
    state: &mut TreeState,
    old: &str,
    new: &str,
) -> Result<(), String> {
    let (old, new) = (normalize_group_path(old), normalize_group_path(new));
    if new.is_empty() || old.is_empty() || new == old {
        return Ok(());
    }
    if new.starts_with(&format!("{old}/")) {
        return Err(SELF_MOVE.to_owned());
    }
    for key in ["connections", "favorites"] {
        let mut lst = list(cfg, key);
        for e in lst.iter_mut().filter(|e| is_dict(e)) {
            let g = group_of(e);
            let ng = map_group_on_rename(&g, &old, &new);
            if ng != g {
                set_key(e, "group", Value::Str(ng));
            }
        }
        set_list(cfg, key, lst);
    }
    let mapped: Vec<String> = explicit_groups(cfg)
        .iter()
        .map(|g| map_group_on_rename(g, &old, &new))
        .collect();
    set_explicit_groups(cfg, &mapped);
    remap_group_environments(cfg, |g| Some(map_group_on_rename(g, &old, &new)));
    state.expanded_groups = state
        .expanded_groups
        .iter()
        .map(|g| map_group_on_rename(g, &old, &new))
        .collect();
    let mut anc = group_path_ancestors(&new);
    anc.pop();
    state.expanded_groups.extend(anc);
    Ok(())
}

/// `_on_folder_moved`: folder `old` dragged onto folder `parent` (`""` = top level).
pub fn folder_moved(
    cfg: &mut Value,
    state: &mut TreeState,
    old: &str,
    parent: &str,
) -> Result<(), String> {
    let old = normalize_group_path(old);
    let parent = normalize_group_path(parent);
    let leaf = old.rsplit('/').next().unwrap_or_default();
    let new = if parent.is_empty() {
        leaf.to_owned()
    } else {
        format!("{parent}/{leaf}")
    };
    apply_group_rename(cfg, state, &old, &new)
}

/// `_add_group`: create a folder (possibly empty). `false` when the path is empty.
pub fn add_group(cfg: &mut Value, state: &mut TreeState, parent: &str, name: &str) -> bool {
    let path = normalize_group_path(&if parent.is_empty() {
        name.to_owned()
    } else {
        format!("{parent}/{name}")
    });
    if path.is_empty() {
        return false;
    }
    let mut groups = explicit_groups(cfg);
    groups.push(path.clone());
    set_explicit_groups(cfg, &groups);
    state.reveal(&path);
    true
}

/// `_ungroup_group`: dissolve a folder, its contents move up one level.
pub fn ungroup_group(cfg: &mut Value, state: &mut TreeState, name: &str) {
    let path = normalize_group_path(name);
    if path.is_empty() {
        return;
    }
    for key in ["connections", "favorites"] {
        let mut lst = list(cfg, key);
        for e in lst.iter_mut().filter(|e| is_dict(e)) {
            let g = group_of(e);
            let ng = map_group_on_dissolve(&g, &path);
            if ng != g {
                set_group(e, &ng);
            }
        }
        set_list(cfg, key, lst);
    }
    let mapped: Vec<String> = explicit_groups(cfg)
        .iter()
        .filter(|g| **g != path)
        .map(|g| map_group_on_dissolve(g, &path))
        .collect();
    set_explicit_groups(cfg, &mapped);
    // The dissolved folder's own environment goes with it; its contents now inherit from above.
    remap_group_environments(cfg, |g| {
        (g != path).then(|| map_group_on_dissolve(g, &path))
    });
    state.expanded_groups = state
        .expanded_groups
        .iter()
        .filter(|g| **g != path)
        .map(|g| map_group_on_dissolve(g, &path))
        .filter(|g| !g.is_empty())
        .collect();
}

fn inside(entry: &Value, path: &str) -> bool {
    let g = group_of(entry);
    g == path || g.starts_with(&format!("{path}/"))
}

/// Entries (connections and favorites) that `delete_group` would remove.
pub fn group_entry_count(cfg: &Value, name: &str) -> usize {
    let path = normalize_group_path(name);
    ["connections", "favorites"]
        .iter()
        .flat_map(|k| list(cfg, k))
        .filter(|e| is_dict(e) && inside(e, &path))
        .count()
}

/// `_delete_group`: the folder with every entry and subfolder under it.
pub fn delete_group(cfg: &mut Value, state: &mut TreeState, name: &str) {
    let path = normalize_group_path(name);
    if path.is_empty() {
        return;
    }
    for key in ["connections", "favorites"] {
        let kept = list(cfg, key)
            .into_iter()
            .filter(|e| !(is_dict(e) && inside(e, &path)))
            .collect();
        set_list(cfg, key, kept);
    }
    let prefix = format!("{path}/");
    let groups: Vec<String> = explicit_groups(cfg)
        .into_iter()
        .filter(|g| *g != path && !g.starts_with(&prefix))
        .collect();
    set_explicit_groups(cfg, &groups);
    remap_group_environments(cfg, |g| {
        (g != path && !g.starts_with(&prefix)).then(|| g.to_owned())
    });
    state
        .expanded_groups
        .retain(|g| *g != path && !g.starts_with(&prefix));
}

/// `_duplicate_selected`: deep copy right after the original, label `<label> (copy)`.
pub fn duplicate_entry(cfg: &mut Value, entry: EntryRef) -> Option<EntryRef> {
    let key = entry.kind.key();
    let mut lst = list(cfg, key);
    let mut clone = lst.get(entry.index).filter(|e| is_dict(e))?.clone();
    let base = strip_space(&text(&clone, "label")).to_owned();
    let label = if base.is_empty() {
        "copy".to_owned()
    } else {
        format!("{base} (copy)")
    };
    set_key(&mut clone, "label", Value::Str(label));
    lst.insert(entry.index + 1, clone);
    set_list(cfg, key, lst);
    Some(EntryRef {
        kind: entry.kind,
        index: entry.index + 1,
    })
}

/// `_move_selected`: swap with the sibling `delta` places away in the same folder.
pub fn move_entry(cfg: &mut Value, entry: EntryRef, delta: isize) -> Option<EntryRef> {
    let key = entry.kind.key();
    let mut lst = list(cfg, key);
    let cur = group_of(lst.get(entry.index).filter(|e| is_dict(e))?);
    let sibs: Vec<usize> = (0..lst.len())
        .filter(|&k| is_dict(&lst[k]) && group_of(&lst[k]) == cur)
        .collect();
    let pos = sibs.iter().position(|&k| k == entry.index)?;
    let j = *sibs.get(pos.checked_add_signed(delta)?)?;
    lst.swap(entry.index, j);
    set_list(cfg, key, lst);
    Some(EntryRef {
        kind: entry.kind,
        index: j,
    })
}

/// `_move_to_group` after the dialog: `name` is the typed folder (`(top level)` = top).
pub fn move_to_group(
    cfg: &mut Value,
    state: &mut TreeState,
    entry: EntryRef,
    name: &str,
) -> Option<EntryRef> {
    let key = entry.kind.key();
    let mut lst = list(cfg, key);
    let e = lst.get_mut(entry.index).filter(|e| is_dict(e))?;
    let group = if strip_space(name) == "(top level)" {
        String::new()
    } else {
        normalize_group_path(name)
    };
    set_group(e, &group);
    set_list(cfg, key, lst);
    if !group.is_empty() {
        state.reveal(&group);
    }
    Some(entry)
}

/// `_remove_selected` after confirmation.
pub fn remove_entry(cfg: &mut Value, entry: EntryRef) {
    let key = entry.kind.key();
    let mut lst = list(cfg, key);
    if entry.index < lst.len() {
        lst.remove(entry.index);
        set_list(cfg, key, lst);
    }
}

/// `_bulk_delete` after confirmation: drop the connections at `indices`.
pub fn bulk_delete(cfg: &mut Value, indices: &[usize]) {
    let kept = list(cfg, "connections")
        .into_iter()
        .enumerate()
        .filter(|(i, _)| !indices.contains(i))
        .map(|(_, c)| c)
        .collect();
    set_list(cfg, "connections", kept);
}

/// `_selected_connection_indices`: config indices of the selected connection rows, each once,
/// in config order (Recent duplicates count once).
pub fn selected_connection_indices(rows: &[Row], selected: &[usize]) -> Vec<usize> {
    let set: BTreeSet<usize> = selected
        .iter()
        .filter_map(|&i| rows.get(i))
        .filter(|r| r.kind == RowKind::Connection)
        .map(|r| r.index)
        .collect();
    set.into_iter().collect()
}
