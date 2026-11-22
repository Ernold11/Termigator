//! Drag & drop: drop zones, resolving a drop to a [`DropAction`], applying it.

use super::*;

/// Where in a row the pointer is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Zone {
    Above,
    On,
    Below,
}

/// Drop zone from the y position inside the row (`0.0` top, `1.0` bottom); `None` for rows
/// that accept no drops (Recent and its children).
pub fn drop_zone(row: &Row, y_fraction: f64) -> Option<Zone> {
    if row.in_recent {
        return None;
    }
    Some(if y_fraction < DROP_EDGE {
        Zone::Above
    } else if y_fraction > 1.0 - DROP_EDGE {
        Zone::Below
    } else {
        Zone::On
    })
}

/// What a drop asks for (`entryMoved` / `entriesPlaced` / `folderMoved` of `SidebarTree`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DropAction {
    /// Rejected or nothing to do.
    Ignore,
    FolderMoved {
        path: String,
        parent: String,
    },
    EntriesPlaced {
        moving: Vec<EntryRef>,
        group: String,
        anchor: EntryRef,
        after: bool,
    },
    EntryMoves {
        moving: Vec<EntryRef>,
        group: String,
    },
}

/// `SidebarTree.dropEvent`: `current` is the dragged (current) row, `selected` the selected
/// rows in tree order, `target` the row under the pointer and the zone (`None` = empty space).
pub fn resolve_drop(
    rows: &[Row],
    current: usize,
    selected: &[usize],
    target: Option<(usize, Zone)>,
) -> DropAction {
    let Some(cur) = rows.get(current) else {
        return DropAction::Ignore;
    };
    let tgt = target.and_then(|(i, z)| rows.get(i).map(|r| (r, z)));
    if cur.in_recent || tgt.is_some_and(|(r, _)| r.in_recent) {
        return DropAction::Ignore;
    }
    let between = tgt.is_some_and(|(_, z)| z != Zone::On);
    let folder_at = tgt.map_or("", |(r, _)| {
        if r.kind == RowKind::Group {
            r.path.as_str()
        } else {
            r.folder.as_str()
        }
    });
    let parent_of = |p: &str| p.rsplit_once('/').map_or("", |(a, _)| a).to_owned();
    let tgt_group = tgt.filter(|(r, _)| r.kind == RowKind::Group);
    if cur.kind == RowKind::Group {
        let parent = match tgt_group {
            Some((r, _)) if between => parent_of(&r.path),
            _ => folder_at.to_owned(),
        };
        return DropAction::FolderMoved {
            path: cur.path.clone(),
            parent,
        };
    }
    let Some(first) = cur.entry() else {
        return DropAction::Ignore;
    };
    let group = match tgt_group {
        Some((r, _)) if between => parent_of(&r.path),
        _ => folder_at.to_owned(),
    };
    let mut moving = vec![first];
    for &i in selected {
        if let Some(q) = rows.get(i).filter(|r| !r.in_recent).and_then(Row::entry)
            && !moving.contains(&q)
        {
            moving.push(q);
        }
    }
    if let Some((r, zone)) = tgt
        && let Some(anchor) = r.entry().filter(|a| between && !moving.contains(a))
    {
        return DropAction::EntriesPlaced {
            moving,
            group,
            anchor,
            after: zone == Zone::Below,
        };
    }
    DropAction::EntryMoves { moving, group }
}

/// Execute a [`DropAction`] on the config. `Ok(Some(entry))` is the entry to select afterwards.
pub fn apply_drop(
    cfg: &mut Value,
    state: &mut TreeState,
    action: &DropAction,
) -> Result<Option<EntryRef>, String> {
    match action {
        DropAction::Ignore => Ok(None),
        DropAction::FolderMoved { path, parent } => {
            folder_moved(cfg, state, path, parent)?;
            Ok(None)
        }
        DropAction::EntriesPlaced {
            moving,
            group,
            anchor,
            after,
        } => Ok(entries_placed(cfg, state, moving, group, *anchor, *after)),
        DropAction::EntryMoves { moving, group } => {
            let mut select = None;
            for m in moving {
                select = entry_moved(cfg, state, *m, group).or(select);
            }
            Ok(select)
        }
    }
}
