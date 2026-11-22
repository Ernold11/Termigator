//! Row views of the host tree: icons, tooltips, search collapse, reveal, drag checks.

use super::*;
use termigator_core::config::environments;

/// The command builders and the serial summary for [`hosttree::build`].
struct SidebarHooks<'a> {
    cfg: &'a Value,
}

impl Hooks for SidebarHooks<'_> {
    fn build_error(&self, conn: &Value) -> Option<String> {
        build_conn_command(self.cfg, conn)
            .err()
            .filter(|e| !e.is_empty())
    }

    fn serial_summary(&self, conn: &Value) -> String {
        serial::serial_summary(conn)
    }
}

/// `user@host[:port]` as the row shows it (the port only when not the protocol's default).
fn target_of(c: &Value, proto: &str) -> String {
    let (host, user) = (text(c, "host"), text(c, "user"));
    if host.is_empty() {
        return String::new();
    }
    let target = if user.is_empty() {
        host
    } else {
        format!("{user}@{host}")
    };
    let default_port = match proto {
        PROTOCOL_RDP => "3389",
        PROTOCOL_VNC => "5900",
        PROTOCOL_TELNET => "23",
        _ => "22",
    };
    if let Some(p) = c.get("port").filter(|p| !matches!(p, Value::Null)) {
        let port = value_str(p);
        let bare = strip_space(&port);
        if !bare.is_empty() && bare != default_port {
            return format!("{target}:{port}");
        }
    }
    target
}

/// The tooltip of a connection row.
pub fn connection_tooltip(c: &Value, err: Option<&str>) -> String {
    let proto = connection_protocol(c);
    let target = target_of(c, proto);
    let mut tip = if target.is_empty() {
        entry_label(c)
    } else {
        target
    };
    let distro = strip_space(&text(c, "distro")).to_owned();
    match proto {
        PROTOCOL_RDP => tip.push_str("\n\nRDP (FreeRDP)"),
        PROTOCOL_VNC => tip.push_str("\n\nVNC"),
        PROTOCOL_TELNET => tip.push_str("\n\nTelnet (not encrypted)"),
        PROTOCOL_SERIAL => tip = format!("{}\n\nSerial console", serial::serial_summary(c)),
        _ if !distro.is_empty() && err.is_none() => {
            tip = format!("{tip}\n\n{}", distro_label(&distro));
        }
        _ => {}
    }
    if c.get("use_mosh") == Some(&Value::Bool(true)) {
        tip.push_str("\nmosh");
    }

    if let Some(Value::Seq(tags)) = c.get("tags")
        && !tags.is_empty()
    {
        let tags: Vec<String> = tags.iter().map(value_str).collect();
        tip = format!("{tip}\nTags: {}", tags.join(", "));
    }
    if let Some(e) = err {
        tip = format!("{tip}\n\n{e}");
    }
    tip
}

fn row_view(cfg: &Value, row: Row) -> RowView {
    let entry = row.entry().and_then(|e| entry_at(cfg, e));
    let tooltip = match (row.kind, entry) {
        (RowKind::Connection, Some(c)) => {
            let err = SidebarHooks { cfg }.build_error(c);
            connection_tooltip(c, err.as_deref())
        }
        (RowKind::Favorite, Some(f)) => format!(
            "Command:\n{}\n\nTerminal: {}",
            row.subtitle,
            terminal_mode_for_entry(Some(f), cfg)
        ),
        _ => String::new(),
    };
    let distro = match &row.icon {
        Icon::Distro(id) => {
            let (color, glyph) = distro_badge(id);
            Some((color, glyph, distro_logo_url(id)))
        }
        _ => None,
    };
    // The entry's own environment, else the one its folder has or inherits.
    let (environment, from) = match (row.kind, entry) {
        (RowKind::Connection, Some(c)) if !hosttree::environment_of(c).is_empty() => {
            (hosttree::environment_of(c), String::new())
        }
        (RowKind::Connection, Some(c)) => hosttree::folder_environment(cfg, &group_of(c)),
        (RowKind::Group, _) => hosttree::folder_environment(cfg, &row.path),
        _ => (String::new(), String::new()),
    };
    let environment_name = environments::name(cfg, &environment);
    let environment_color = environments::color(cfg, &environment);
    let mut tooltip = tooltip;
    if !environment.is_empty() {
        let line = if from.is_empty() || (row.kind == RowKind::Group && from == row.path) {
            format!("Label: {environment_name}")
        } else {
            format!("Label: {environment_name} (from folder {from})")
        };
        tooltip = if tooltip.is_empty() {
            line
        } else {
            format!("{tooltip}\n{line}")
        };
    }
    RowView {
        icon: icon_name(&row.icon),
        distro,
        tooltip,
        environment,
        environment_name,
        environment_color,
        row,
    }
}

/// While searching, the folders collapsed by hand hide their rows again.
fn collapse_rows(rows: Vec<Row>, collapsed: &BTreeSet<String>) -> Vec<Row> {
    let mut out: Vec<Row> = Vec::with_capacity(rows.len());
    let mut hide_below: Option<usize> = None;
    for mut r in rows {
        if let Some(d) = hide_below {
            if r.depth > d {
                continue;
            }
            hide_below = None;
        }
        if r.kind == RowKind::Group && collapsed.contains(&r.path) {
            r.expanded = false;
            hide_below = Some(r.depth);
        }
        out.push(r);
    }
    out
}

/// Rows, status line and empty-list hint (`""` = hidden).
pub(super) fn build_views(
    cfg: &Value,
    state: &TreeState,
    filter: &str,
    filter_collapsed: &BTreeSet<String>,
) -> (Vec<RowView>, String, String) {
    let tree = hosttree::build(cfg, state, filter, &SidebarHooks { cfg });
    let rows = if strip_space(filter).is_empty() {
        tree.rows
    } else {
        collapse_rows(tree.rows, filter_collapsed)
    };
    let views = rows.into_iter().map(|r| row_view(cfg, r)).collect();
    (views, tree.status, tree.hint.unwrap_or_default())
}

/// The first item for the entry in the whole tree (Recent first,
/// collapsed folders too) gets every level above it expanded.
pub fn reveal_entry(cfg: &Value, state: &mut TreeState, filter: &str, entry: EntryRef) {
    let all = TreeState {
        expanded_groups: hosttree::all_group_names(cfg).into_iter().collect(),
        recent_expanded: true,
    };
    let filtering = !strip_space(filter).is_empty();
    // Without a search the rows do not depend on the builders' errors.
    let tree = if filtering {
        hosttree::build(cfg, &all, filter, &SidebarHooks { cfg })
    } else {
        hosttree::build(cfg, &all, filter, &())
    };
    let Some(found) = tree.rows.iter().find(|r| r.entry() == Some(entry)) else {
        return;
    };
    if filtering {
        return; // everything is expanded while searching
    }
    if found.in_recent {
        state.recent_expanded = true;
    } else {
        state
            .expanded_groups
            .extend(group_path_ancestors(&found.folder));
    }
}

/// Qt's `droppingOnItself`: a drop on a dragged row, or anywhere below a dragged folder (or a
/// selected Recent), is refused.
pub fn drop_blocked(rows: &[Row], current: usize, selected: &[usize], target: usize) -> bool {
    let Some(t) = rows.get(target) else {
        return false;
    };
    let dragged: BTreeSet<usize> = selected.iter().copied().chain([current]).collect();
    let ancestors = group_path_ancestors(&t.folder);
    rows.iter().enumerate().any(|(i, r)| {
        dragged.contains(&i)
            && (i == target
                || (t.in_recent && r.kind == RowKind::Recent)
                || (!t.in_recent
                    && r.kind == RowKind::Group
                    && !r.in_recent
                    && ancestors.contains(&r.path)))
    })
}

/// Where the rows `keep` of `old` are in `new` (by kind, index, folder path and Recent).
pub fn remap_rows(old: &[Row], new: &[Row], keep: &[usize]) -> Vec<Option<usize>> {
    let same = |a: &Row, b: &Row| {
        a.kind == b.kind && a.index == b.index && a.path == b.path && a.in_recent == b.in_recent
    };
    keep.iter()
        .map(|&k| {
            let r = old.get(k)?;
            new.iter().position(|n| same(n, r))
        })
        .collect()
}

pub fn delete_group_question(path: &str, doomed: usize) -> String {
    let what = match doomed {
        0 => "no entries".to_owned(),
        1 => "its 1 entry".to_owned(),
        n => format!("its {n} entries"),
    };
    format!("Delete the “{path}” folder, all its subfolders and {what}?\nThis cannot be undone.")
}

pub fn dissolve_question(path: &str) -> String {
    let dest = match path.rsplit_once('/') {
        Some((parent, _)) => format!("“{parent}”"),
        None => "the top level".to_owned(),
    };
    format!(
        "Dissolve the “{path}” folder and move its contents to {dest}?\n(The entries themselves \
         are kept.)"
    )
}
