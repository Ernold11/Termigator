//! Environment categories: the names and colours an entry or folder can be marked with.
//!
//! The categories live in config.yaml (`environments:`) as a list of `{id, name, color}` maps.
//! `id` is what entries store in `environment:` (stable across renames), `name` is what the menus
//! show and `color` is a CSS/Qt colour string. A config without the key gets the four built-in
//! categories; an empty list means the user removed them all.

use super::doc::Value;
use super::model::{strip_space, truthy, value_str};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Environment {
    /// Stable key stored in entries and folders.
    pub id: String,
    /// What the UI shows.
    pub name: String,
    /// A Qt/CSS colour (`#rrggbb` from the picker).
    pub color: String,
}

/// `(id, name, color)` of the built-in categories. The ids are the values older configs used.
pub const DEFAULTS: [(&str, &str, &str); 4] = [
    ("production", "Production", "#da4453"),
    ("staging", "Staging", "#f67400"),
    ("test", "Test", "#3daee9"),
    ("development", "Development", "#27ae60"),
];

fn text(v: &Value, key: &str) -> String {
    match v.get(key) {
        Some(x) if truthy(x) => value_str(x),
        _ => String::new(),
    }
}

fn environment_value(e: &Environment) -> Value {
    Value::Map(vec![
        (Value::Str("id".into()), Value::Str(e.id.clone())),
        (Value::Str("name".into()), Value::Str(e.name.clone())),
        (Value::Str("color".into()), Value::Str(e.color.clone())),
    ])
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

/// The default list as a config value (used by `default_config`).
pub fn default_value() -> Value {
    Value::Seq(
        DEFAULTS
            .iter()
            .map(|(id, name, color)| {
                environment_value(&Environment {
                    id: (*id).to_owned(),
                    name: (*name).to_owned(),
                    color: (*color).to_owned(),
                })
            })
            .collect(),
    )
}

/// The built-in categories.
pub fn defaults() -> Vec<Environment> {
    DEFAULTS
        .iter()
        .map(|(id, name, color)| Environment {
            id: (*id).to_owned(),
            name: (*name).to_owned(),
            color: (*color).to_owned(),
        })
        .collect()
}

fn slug(name: &str) -> String {
    let mut out = String::new();
    for c in name.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.ends_with('-') {
            out.push('-');
        }
    }
    let out = out.trim_matches('-');
    if out.is_empty() {
        "environment".to_owned()
    } else {
        out.to_owned()
    }
}

fn parse_item(v: &Value) -> Option<Environment> {
    let name = strip_space(&text(v, "name")).to_owned();
    let id = strip_space(&text(v, "id")).to_owned();
    let color = strip_space(&text(v, "color")).to_owned();
    let id = if id.is_empty() { slug(&name) } else { id };
    let name = if name.is_empty() { id.clone() } else { name };
    if id.is_empty() {
        return None;
    }
    Some(Environment { id, name, color })
}

fn parse_list(items: &[Value]) -> Vec<Environment> {
    let mut out: Vec<Environment> = Vec::new();
    for item in items {
        let Some(env) = parse_item(item) else {
            continue;
        };
        if out.iter().any(|e| e.id.eq_ignore_ascii_case(&env.id)) {
            continue;
        }
        out.push(env);
    }
    out
}

/// The categories of `cfg`: `environments` when it is a list (empty = none), else the defaults.
pub fn list(cfg: &Value) -> Vec<Environment> {
    match cfg.get("environments") {
        Some(Value::Seq(items)) => parse_list(items),
        Some(_) => Vec::new(),
        None => defaults(),
    }
}

/// The category matching `value` by id or display name (case-insensitive).
pub fn find(cfg: &Value, value: &str) -> Option<Environment> {
    let needle = strip_space(value);
    if needle.is_empty() {
        return None;
    }
    list(cfg)
        .into_iter()
        .find(|e| e.id.eq_ignore_ascii_case(needle) || e.name.eq_ignore_ascii_case(needle))
}

/// The canonical id for `value` (an id or a display name), or `None` when unknown.
pub fn canonical_id(cfg: &Value, value: &str) -> Option<String> {
    find(cfg, value).map(|e| e.id)
}

/// The colour of `value`, `""` when it is not a known category.
pub fn color(cfg: &Value, value: &str) -> String {
    find(cfg, value).map(|e| e.color).unwrap_or_default()
}

/// The display name of `value`, or `value` itself when it is not a known category.
pub fn name(cfg: &Value, value: &str) -> String {
    find(cfg, value)
        .map(|e| e.name)
        .unwrap_or_else(|| strip_space(value).to_owned())
}

/// The categories as JSON for QML: `[{id, name, color}, …]`.
pub fn to_json(envs: &[Environment]) -> serde_json::Value {
    serde_json::Value::Array(
        envs.iter()
            .map(|e| {
                serde_json::json!({
                    "id": e.id,
                    "name": e.name,
                    "color": e.color,
                })
            })
            .collect(),
    )
}

/// Drops `environment` from entries and folder marks that pointed at a removed category.
fn clear_references(cfg: &mut Value, old: &Environment) {
    let matches = |v: &Value| {
        let raw = value_str(v);
        let text = strip_space(&raw);
        text.eq_ignore_ascii_case(&old.id) || text.eq_ignore_ascii_case(&old.name)
    };
    for list in ["connections", "favorites"] {
        if let Some(Value::Seq(items)) = cfg.get_mut(list) {
            for item in items {
                if item.get("environment").is_some_and(&matches) {
                    remove_key(&mut *item, "environment");
                }
            }
        }
    }
    if let Some(Value::Map(pairs)) = cfg.get_mut("group_environments") {
        pairs.retain(|(_, v)| !matches(v));
    }
}

/// Writes the categories and clears references to categories that were removed. Names are not
/// touched: entries store the stable id, so a rename keeps every mark.
pub fn set(cfg: &mut Value, envs: &[Environment]) {
    let old = list(cfg);
    set_key(
        cfg,
        "environments",
        Value::Seq(envs.iter().map(environment_value).collect()),
    );
    for env in &old {
        if !envs.iter().any(|e| e.id.eq_ignore_ascii_case(&env.id)) {
            clear_references(cfg, env);
        }
    }
}

/// Applies an edited list from QML (a JSON array of `{id, name, color}` objects).
pub fn apply(cfg: &mut Value, draft: &serde_json::Value) {
    let Some(items) = draft.as_array() else {
        return;
    };
    let items: Vec<Value> = items.iter().map(Value::from_json).collect();
    let envs = parse_list(&items);
    set(cfg, &envs);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::doc::load;

    fn cfg(yaml: &str) -> Value {
        load(yaml).expect("yaml")
    }

    #[test]
    fn missing_key_gives_the_defaults_and_an_empty_list_none() {
        let c = cfg("window: {}");
        let envs = list(&c);
        assert_eq!(envs.len(), 4);
        assert_eq!(envs[0].id, "production");
        assert_eq!(envs[0].name, "Production");
        assert!(list(&cfg("environments: []")).is_empty());
    }

    #[test]
    fn ids_names_and_colors_are_read_and_derived() {
        let c = cfg("environments:\n\
             - {name: Prod, color: '#ff0000'}\n\
             - {id: qa, name: 'QA env', color: '#00ff00'}\n\
             - {id: broken, name: '', color: ''}\n\
             - {id: PROD, name: Duplicate}\n");
        let envs = list(&c);
        assert_eq!(envs.len(), 3);
        assert_eq!(envs[0].id, "prod");
        assert_eq!(envs[0].name, "Prod");
        assert_eq!(envs[1].id, "qa");
        assert_eq!(envs[2].name, "broken");
    }

    #[test]
    fn find_matches_id_or_name_ignoring_case() {
        let c = cfg("{}");
        assert_eq!(
            find(&c, "PRODUCTION").map(|e| e.id).as_deref(),
            Some("production")
        );
        assert_eq!(
            find(&c, "staging").map(|e| e.name).as_deref(),
            Some("Staging")
        );
        assert_eq!(
            find(&c, "Staging").map(|e| e.id).as_deref(),
            Some("staging")
        );
        assert!(find(&c, "nope").is_none());
    }

    #[test]
    fn removing_a_category_clears_entries_and_folders() {
        let mut c = cfg("connections:\n\
             - {label: a, environment: production}\n\
             - {label: b, environment: Production}\n\
             - {label: c, environment: test}\n\
             group_environments: {Prod: production, Lab: test}\n");
        let envs = vec![Environment {
            id: "test".into(),
            name: "Test".into(),
            color: "#3daee9".into(),
        }];
        set(&mut c, &envs);
        let conns = match c.get("connections") {
            Some(Value::Seq(items)) => items,
            _ => panic!("connections"),
        };
        assert!(conns[0].get("environment").is_none());
        assert!(conns[1].get("environment").is_none());
        assert_eq!(
            conns[2].get("environment"),
            Some(&Value::Str("test".into()))
        );
        let groups = match c.get("group_environments") {
            Some(Value::Map(pairs)) => pairs,
            _ => panic!("groups"),
        };
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].1, Value::Str("test".into()));
    }

    #[test]
    fn rename_keeps_marks_because_entries_store_the_id() {
        let mut c = cfg("connections:\n- {label: a, environment: prod}\n");
        let envs = vec![Environment {
            id: "prod".into(),
            name: "Production".into(),
            color: "#ff0000".into(),
        }];
        set(&mut c, &envs);
        let conns = match c.get("connections") {
            Some(Value::Seq(items)) => items,
            _ => panic!("connections"),
        };
        assert_eq!(
            conns[0].get("environment"),
            Some(&Value::Str("prod".into()))
        );
    }
}
