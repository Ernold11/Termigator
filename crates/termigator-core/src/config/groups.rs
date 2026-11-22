//! Folder (group) paths of the sidebar tree.
//!
//! Paths are `/`-separated. Backslashes count as separators too (MobaXterm session paths),
//! blank segments and whitespace around segments are dropped, `""` is the top level. All
//! functions but [`normalize_group_value`] take text; normalise raw config values first.

use super::doc::Value;
use super::model::{strip_space, truthy, value_str};

/// Canonical `a/b/c` form of a path.
pub fn normalize_group_path(raw: &str) -> String {
    raw.replace('\\', "/")
        .split('/')
        .map(strip_space)
        .filter(|p| !p.is_empty())
        .collect::<Vec<_>>()
        .join("/")
}

/// [`normalize_group_path`] of `str(raw or "")` for an arbitrary config value.
pub fn normalize_group_value(raw: &Value) -> String {
    if truthy(raw) {
        normalize_group_path(&value_str(raw))
    } else {
        String::new()
    }
}

/// `["a", "a/b", "a/b/c"]` for `a/b/c`: every folder the path lives in, itself included.
pub fn group_path_ancestors(path: &str) -> Vec<String> {
    let path = normalize_group_path(path);
    path.match_indices('/')
        .map(|(i, _)| path[..i].to_owned())
        .chain((!path.is_empty()).then_some(path.clone()))
        .collect()
}

/// An entry's group after folder `old` (and its subtree) is renamed to `new`.
pub fn map_group_on_rename(group: &str, old: &str, new: &str) -> String {
    let g = normalize_group_path(group);
    let (old, new) = (normalize_group_path(old), normalize_group_path(new));
    if old.is_empty() || new.is_empty() {
        return g;
    }
    if g == old {
        return new;
    }
    match g.strip_prefix(&old).filter(|rest| rest.starts_with('/')) {
        Some(rest) => format!("{new}{rest}"),
        None => g,
    }
}

/// An entry's group after folder `path` is dissolved: its contents move up one level.
pub fn map_group_on_dissolve(group: &str, path: &str) -> String {
    let g = normalize_group_path(group);
    let path = normalize_group_path(path);
    if path.is_empty() {
        return g;
    }
    let parent = path.rsplit_once('/').map_or("", |(p, _)| p);
    if g == path {
        return parent.to_owned();
    }
    match g.strip_prefix(&path).and_then(|r| r.strip_prefix('/')) {
        Some(rest) if parent.is_empty() => rest.to_owned(),
        Some(rest) => format!("{parent}/{rest}"),
        None => g,
    }
}
