//! Comment-preserving update of config.yaml: `config._sync_mapping` applied to the
//! file (recurse into mappings, replace other values whole, drop keys missing from
//! the target, append new keys).
//!
//! - Deleting a block-mapping entry cuts its lines out by byte range: yaml-edit 0.3.6
//!   leaves the removed entry's indentation in front of the next key of an indented
//!   mapping, which breaks the document.
//! - Replacing and inserting go through yaml-edit, but every new key and value is first
//!   rendered by our PyYAML-compatible emitter and parsed back, so YAML 1.1 quoting (`'yes'`,
//!   `'1:30'`, `'0777'`, `'='`) comes from us, not from yaml-edit's YAML 1.2 rules.
//! - The result is reloaded and must equal the target; otherwise the caller gets a
//!   plain dump.

use std::ops::Range;
use std::str::FromStr;

use yaml_edit::{Mapping, YamlFile, YamlNode};

use super::emit::{self, ONE_LINE};
use super::{Value, load};

pub(super) fn update(original: &str, target: &Value) -> String {
    // Line endings are normalised: the loader only ever sees `\n`.
    let original = original.replace("\r\n", "\n").replace('\r', "\n");
    let edited = match (target, load(&original)) {
        (Value::Map(new), Ok(Value::Map(old))) => edit(&original, &old, new),
        _ => None,
    };
    match edited {
        Some(text) if load(&text).is_ok_and(|v| v.loose_eq(target)) => text,
        _ => emit::dump(target, emit::PYYAML),
    }
}

enum Pass<'a> {
    /// Collect the line ranges of block entries to delete; change nothing.
    Delete(&'a str, &'a mut Vec<Range<usize>>),
    /// Remove what is left to remove, replace changed values, append new keys.
    Edit,
}

fn root(text: &str) -> Option<(YamlFile, Mapping)> {
    let file = YamlFile::from_str(text).ok()?;
    let map = file.document()?.as_mapping()?;
    Some((file, map))
}

fn edit(original: &str, old: &[(Value, Value)], target: &[(Value, Value)]) -> Option<String> {
    let mut cut = Vec::new();
    sync(
        &root(original)?.1,
        old,
        target,
        &mut Pass::Delete(original, &mut cut),
    )?;
    let mut text = original.to_owned();
    for range in cut.into_iter().rev() {
        text.replace_range(range, "");
    }
    let (file, map) = root(&text)?;
    sync(&map, old, target, &mut Pass::Edit)?;
    let mut text = file.to_string();
    if target.is_empty() && matches!(load(&text), Ok(Value::Null)) {
        // Every key went; keep the comments and write the empty mapping like ruamel.
        if !text.is_empty() && !text.ends_with('\n') {
            text.push('\n');
        }
        text.push_str("{}\n");
    }
    Some(text)
}

/// The whole lines of an entry, when nothing else shares them (block style).
fn entry_lines(text: &str, entry: &yaml_edit::MappingEntry) -> Option<Range<usize>> {
    let span = entry.byte_range();
    let start = usize::try_from(span.start).ok()?;
    // The span runs on over the line break and the next line's indentation.
    let end = start
        + text
            .get(start..usize::try_from(span.end).ok()?)?
            .trim_end()
            .len();
    let line_start = text[..start].rfind('\n').map_or(0, |i| i + 1);
    let line_end = text[end..].find('\n').map_or(text.len(), |i| end + i + 1);
    let alone = text[line_start..start].trim_matches(' ').is_empty()
        && text[end..line_end].trim().is_empty();
    alone.then_some(line_start..line_end)
}

fn lookup<'v>(pairs: &'v [(Value, Value)], key: &Value) -> Option<&'v Value> {
    pairs.iter().find(|(k, _)| k.loose_eq(key)).map(|(_, v)| v)
}

/// `None`: the document has something this cannot map back (merge keys, alias keys).
fn sync(
    map: &Mapping,
    old: &[(Value, Value)],
    target: &[(Value, Value)],
    pass: &mut Pass,
) -> Option<()> {
    for entry in map.entries().collect::<Vec<_>>() {
        // The key as PyYAML reads it, not as yaml-edit compares it.
        let key = load(&entry.key_node()?.to_string()).ok()?;
        let Some(new) = lookup(target, &key) else {
            match pass {
                Pass::Delete(text, cut) => cut.extend(entry_lines(text, &entry)),
                Pass::Edit => entry.remove(),
            }
            continue;
        };
        let current = lookup(old, &key)?;
        let nested = entry.value_node().and_then(|n| n.as_mapping().cloned());
        match (new, current, nested) {
            (Value::Map(n), Value::Map(c), Some(nested)) => sync(&nested, c, n, pass)?,
            // Equal values stay as written, with their comments and spelling.
            _ if new == current => {}
            _ if matches!(pass, Pass::Delete(..)) => {}
            _ => entry.set_value(fragment(&key, new)?.1, false),
        }
    }
    if matches!(pass, Pass::Edit) {
        for (k, v) in target {
            if lookup(old, k).is_none() {
                let (key, value) = fragment(k, v)?;
                map.set(key, value);
            }
        }
    }
    Some(())
}

/// Key and value nodes of `{key: value}` as our emitter writes it, on single lines.
fn fragment(key: &Value, value: &Value) -> Option<(YamlNode, YamlNode)> {
    let text = emit::dump(&Value::Map(vec![(key.clone(), value.clone())]), ONE_LINE);
    let file = YamlFile::from_str(&text).ok()?;
    let entry = file.document()?.as_mapping()?.entries().next()?;
    Some((entry.key_node()?, entry.value_node()?))
}
