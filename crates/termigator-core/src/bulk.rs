//! Bulk edit of host entries and session icons.
//!
//! [`apply_bulk`] changes many entries at once with only the fields the user ticked; fields
//! that do not apply to an entry's protocol (a jump host on RDP, a credential on VNC) are
//! left alone on that entry and counted, never forced. Entries are [`Value`] maps and keep
//! insertion key order. Golden fixtures: `compat/golden/bulk.json`, `bulk_icons.json`.

use crate::config::doc::Value;
use crate::config::groups::normalize_group_value;
use crate::config::model::{strip_space, truthy, value_str};
use crate::credentials::{find_credential, remove_key, set_key};

/// (label, freedesktop icon name) — what "Set icon" offers.
pub const ICON_CHOICES: [(&str, &str); 12] = [
    ("Server", "network-server"),
    ("Database", "server-database"),
    ("Network device", "network-wired"),
    ("Firewall", "security-high"),
    ("Desktop", "computer"),
    ("Laptop", "computer-laptop"),
    ("Cloud", "folder-cloud"),
    ("Container", "package-x-generic"),
    ("Terminal", "utilities-terminal"),
    ("Web", "applications-internet"),
    ("Star", "starred"),
    ("Warning", "dialog-warning"),
];

/// Whether `name` is one of the [`ICON_CHOICES`] (`apply_bulk`
/// itself accepts any icon text).
pub fn is_icon_name(name: &str) -> bool {
    ICON_CHOICES.iter().any(|(_, n)| *n == name)
}

/// `str(c.get("protocol") or "ssh").lower()`.
fn proto(c: &Value) -> String {
    match c.get("protocol").filter(|v| truthy(v)) {
        Some(v) => value_str(v).to_lowercase(),
        None => "ssh".to_owned(),
    }
}

/// Tags of an entry: the non-blank `str(x).strip()` of its list, else none.
fn tags_of(c: &Value) -> Vec<Value> {
    match c.get("tags") {
        Some(Value::Seq(items)) => items
            .iter()
            .map(|x| strip_space(&value_str(x)).to_owned())
            .filter(|t| !t.is_empty())
            .map(Value::Str)
            .collect(),
        _ => Vec::new(),
    }
}

/// `changes.get(key) or []` as a list (shortcut: a text value counts as empty here, the
/// dialog only produces lists).
fn list_of<'a>(changes: &'a Value, key: &str) -> &'a [Value] {
    match changes.get(key) {
        Some(Value::Seq(items)) => items,
        _ => &[],
    }
}

fn contains(items: &[Value], t: &Value) -> bool {
    items.iter().any(|x| x.loose_eq(t))
}

/// Apply `changes` to `conns[i]` for each index (in place). Keys, all optional: `group`
/// (`""` = top level), `tags_set` / `tags_add` / `tags_remove` (lists), `terminal`, `environment`
/// (`""` clears),
/// `proxy_jump` (`""` clears; SSH only), `credential` (`""` detaches; SSH / RDP only), `icon`
/// (`""` = default). Returns `(changed, not_applicable)` — entries where some ticked field did
/// not apply to their protocol.
pub fn apply_bulk(
    conns: &mut [Value],
    indices: &[i64],
    changes: &Value,
    credentials: &[Value],
) -> (usize, usize) {
    let mut sorted = indices.to_vec();
    sorted.sort_unstable();
    sorted.dedup();
    let (mut changed, mut skipped) = (0, 0);
    for i in sorted {
        let Some(c) = usize::try_from(i).ok().and_then(|i| conns.get_mut(i)) else {
            continue;
        };
        if !matches!(c, Value::Map(_)) {
            continue;
        }
        let before = c.clone();
        let mut na = false;
        if let Some(g) = changes.get("group") {
            let g = normalize_group_value(g);
            if g.is_empty() {
                remove_key(c, "group");
            } else {
                set_key(c, "group", Value::Str(g));
            }
        }
        if ["tags_set", "tags_add", "tags_remove"]
            .iter()
            .any(|k| changes.get(k).is_some())
        {
            let mut tags = match changes.get("tags_set") {
                Some(Value::Null) | None => tags_of(c),
                Some(_) => list_of(changes, "tags_set").to_vec(),
            };
            for t in list_of(changes, "tags_add") {
                if !contains(&tags, t) {
                    tags.push(t.clone());
                }
            }
            let remove = list_of(changes, "tags_remove");
            tags.retain(|t| !contains(remove, t));
            if tags.is_empty() {
                remove_key(c, "tags");
            } else {
                set_key(c, "tags", Value::Seq(tags));
            }
        }
        if let Some(t) = changes.get("terminal") {
            set_key(c, "terminal", t.clone());
        }
        // A category id (or "" to clear it). The dialog offers the configured ids; an
        // unknown text is still stored, so a hand-edited config is not silently trimmed.
        if let Some(e) = changes.get("environment") {
            let raw = value_str(e);
            let env = strip_space(&raw);
            if env.is_empty() {
                remove_key(c, "environment");
            } else {
                set_key(c, "environment", Value::Str(env.to_owned()));
            }
        }
        if let Some(pj) = changes.get("proxy_jump") {
            if proto(c) != "ssh" {
                na = true;
            } else if truthy(pj) {
                set_key(c, "proxy_jump", pj.clone());
                remove_key(c, "jumphost");
                remove_key(c, "jump_host");
            } else {
                for k in ["proxy_jump", "jumphost", "jump_host"] {
                    remove_key(c, k);
                }
            }
        }
        if let Some(name) = changes.get("credential") {
            let p = proto(c);
            if p != "ssh" && p != "rdp" {
                na = true;
            } else if !truthy(name) {
                remove_key(c, "credential");
            } else if let Some(cred) = find_credential(credentials, name) {
                set_key(c, "credential", name.clone());
                remove_key(c, "password"); // the shared one replaces the entry's own
                if let Some(u) = cred.get("user").filter(|v| truthy(v)) {
                    set_key(c, "user", u.clone());
                }
                if p == "rdp"
                    && let Some(d) = cred.get("domain").filter(|v| truthy(v))
                {
                    set_key(c, "domain", d.clone());
                }
            } else {
                na = true;
            }
        }
        if let Some(icon) = changes.get("icon") {
            if truthy(icon) {
                set_key(c, "icon", icon.clone());
            } else {
                remove_key(c, "icon");
            }
        }
        skipped += usize::from(na);
        changed += usize::from(!c.loose_eq(&before));
    }
    (changed, skipped)
}
