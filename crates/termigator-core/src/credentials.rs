//! Named credentials and their references from connections.
//!
//! One login shared by many entries: `config.yaml` → `credentials` holds
//! `{name, user, password?, domain?}` and an entry points at one with `credential: <name>`.
//! The entry keeps a copy of the credential's user (and RDP domain); the password is only
//! lent to a copy at reveal time ([`apply_credential`]).
//!
//! Everything works on [`Value`] maps and keeps insertion key order (an existing
//! key is overwritten in place, a new one is appended). Text conversion follows
//! [`value_str`], so containers used where text is expected are outside its limits.
//! Golden fixtures: `compat/golden/credentials.json`.

use crate::config::doc::Value;
use crate::config::model::{strip_space, truthy, value_str};

/// `dict[key] = val`: overwrite in place or append.
pub(crate) fn set_key(map: &mut Value, key: &str, val: Value) {
    if let Value::Map(pairs) = map {
        match pairs
            .iter_mut()
            .find(|(k, _)| matches!(k, Value::Str(s) if s == key))
        {
            Some((_, v)) => *v = val,
            None => pairs.push((Value::Str(key.to_owned()), val)),
        }
    }
}

/// `dict.pop(key, None)`.
pub(crate) fn remove_key(map: &mut Value, key: &str) {
    if let Value::Map(pairs) = map {
        pairs.retain(|(k, _)| !matches!(k, Value::Str(s) if s == key));
    }
}

/// `str(v or "").strip()`.
fn stripped(v: Option<&Value>) -> String {
    match v {
        Some(v) if truthy(v) => strip_space(&value_str(v)).to_owned(),
        _ => String::new(),
    }
}

fn row(pairs: Vec<(&str, String)>) -> Value {
    Value::Map(
        pairs
            .into_iter()
            .map(|(k, v)| (Value::Str(k.to_owned()), Value::Str(v)))
            .collect(),
    )
}

/// Credentials from YAML, tolerant of hand edits: named, unique by name (first wins).
pub fn normalize_credentials(raw: &Value) -> Vec<Value> {
    let Value::Seq(items) = raw else {
        return Vec::new();
    };
    let mut seen: Vec<String> = Vec::new();
    let mut out = Vec::new();
    for c in items {
        if !matches!(c, Value::Map(_)) {
            continue;
        }
        let name = stripped(c.get("name"));
        if name.is_empty() || seen.contains(&name) {
            continue;
        }
        let mut pairs = vec![("name", name.clone()), ("user", stripped(c.get("user")))];
        seen.push(name);
        if let Some(p) = c.get("password").filter(|p| truthy(p)) {
            pairs.push(("password", value_str(p)));
        }
        let domain = stripped(c.get("domain"));
        if !domain.is_empty() {
            pairs.push(("domain", domain));
        }
        out.push(row(pairs));
    }
    out
}

/// The credential called `name` (`str(name or "").strip()`), if any.
pub fn find_credential<'a>(creds: &'a [Value], name: &Value) -> Option<&'a Value> {
    let name = stripped(Some(name));
    if name.is_empty() {
        return None;
    }
    let name = Value::Str(name);
    creds
        .iter()
        .find(|c| matches!(c, Value::Map(_)) && c.get("name").is_some_and(|n| n.loose_eq(&name)))
}

/// `conn` with the user / domain / password of its named credential (a copy), or a
/// plain copy when it has none. A dangling name leaves the entry as it is.
pub fn apply_credential(conn: &Value, creds: &[Value]) -> Value {
    let cred = conn
        .get("credential")
        .and_then(|name| find_credential(creds, name));
    let Some(cred) = cred else {
        return conn.clone();
    };
    let mut out = conn.clone();
    remove_key(&mut out, "password");
    for key in ["user", "domain", "password"] {
        if let Some(v) = cred.get(key).filter(|v| truthy(v)) {
            set_key(&mut out, key, v.clone());
        }
    }
    out
}

/// How many entries use the credential `name`.
pub fn usage_count(conns: &[Value], name: &str) -> usize {
    let name = Value::Str(name.to_owned());
    conns
        .iter()
        .filter(|c| {
            matches!(c, Value::Map(_)) && c.get("credential").is_some_and(|v| v.loose_eq(&name))
        })
        .count()
}

/// After the credentials were edited: follow `renames` (old → new name), refresh the user /
/// domain copies, and detach entries whose credential was deleted (they keep the user, lose
/// the shared password). Returns how many entries changed.
pub fn sync_references(
    conns: &mut [Value],
    renames: &[(String, String)],
    creds: &[Value],
) -> usize {
    let mut changed = 0;
    for c in conns {
        let Some(current) = c.get("credential").filter(|v| truthy(v)).cloned() else {
            continue;
        };
        let before = c.clone();
        let name = match &current {
            Value::Str(s) => renames
                .iter()
                .find(|(old, _)| old == s)
                .map_or(current, |(_, new)| Value::Str(new.clone())),
            _ => current,
        };
        match find_credential(creds, &name) {
            None => remove_key(c, "credential"),
            Some(cred) => {
                set_key(c, "credential", name);
                remove_key(c, "password");
                if let Some(u) = cred.get("user").filter(|v| truthy(v)) {
                    set_key(c, "user", u.clone());
                }
                if matches!(c.get("protocol"), Some(Value::Str(p)) if p == "rdp") {
                    match cred.get("domain").filter(|v| truthy(v)) {
                        Some(d) => set_key(c, "domain", d.clone()),
                        None => remove_key(c, "domain"),
                    }
                }
            }
        }
        if !c.loose_eq(&before) {
            changed += 1;
        }
    }
    changed
}
