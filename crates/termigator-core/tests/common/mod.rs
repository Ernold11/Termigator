//! Golden fixtures from `compat/golden/*.json` (convention: compat/golden/README.md).
#![allow(dead_code)] // each test binary uses its own subset

use std::path::{Path, PathBuf};

use serde_json::Value as J;

/// The repository root (two levels above this crate).
pub fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// `canonicalize` without the `\\?\` prefix Windows adds (it would break paths joined with `/`).
pub fn canon(p: impl AsRef<Path>) -> PathBuf {
    let c = p.as_ref().canonicalize().expect("canonicalize");
    match c.to_str().and_then(|s| s.strip_prefix(r"\\?\")) {
        Some(plain) => PathBuf::from(plain),
        None => c,
    }
}

/// The `cases` array of `compat/golden/<name>.json`.
pub fn golden(name: &str) -> Vec<J> {
    let path = repo_root()
        .join("compat/golden")
        .join(format!("{name}.json"));
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let doc: J = serde_json::from_str(&text).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    doc["cases"]
        .as_array()
        .cloned()
        .unwrap_or_else(|| panic!("{}: no cases", path.display()))
}

/// A case's `name`.
pub fn name(case: &J) -> &str {
    case["name"].as_str().unwrap_or("<unnamed>")
}

/// A case's text input: `"file"` (relative to the repo root) or inline `"input"`.
pub fn input_text(case: &J) -> String {
    match case.get("file").and_then(J::as_str) {
        Some(rel) => {
            std::fs::read_to_string(repo_root().join(rel)).unwrap_or_else(|e| panic!("{rel}: {e}"))
        }
        None => case["input"]
            .as_str()
            .unwrap_or_else(|| panic!("{}: no input", name(case)))
            .to_owned(),
    }
}

/// Bytes from a JSON string (UTF-8) or `{"$bytes": "<hex>"}`.
pub fn bytes(v: &J) -> Vec<u8> {
    if let Some(s) = v.as_str() {
        return s.as_bytes().to_vec();
    }
    let hex = v["$bytes"]
        .as_str()
        .unwrap_or_else(|| panic!("not bytes: {v}"));
    (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap_or_else(|e| panic!("{hex}: {e}")))
        .collect()
}

/// The single `$tag` of a tagged golden value, e.g. `("$int", "90")`.
pub fn tagged(v: &J) -> Option<(&str, &J)> {
    let obj = v.as_object()?;
    if obj.len() != 1 {
        return None;
    }
    obj.iter()
        .next()
        .map(|(k, v)| (k.as_str(), v))
        .filter(|(k, _)| k.starts_with('$'))
}

/// Golden import rows (MobaXterm, PuTTY) hold the Linux key-path mapping. On Windows a key path
/// stays a Windows path by design (`import::windows::moba_identity_to_windows`, tested there), so
/// there `identity_file` is left out of the comparison with the golden; elsewhere `v` is unchanged.
pub fn without_win_key_paths(
    v: &termigator_core::config::doc::Value,
) -> termigator_core::config::doc::Value {
    use termigator_core::config::doc::Value;
    if !cfg!(windows) {
        return v.clone();
    }
    match v {
        Value::Seq(s) => Value::Seq(s.iter().map(without_win_key_paths).collect()),
        Value::Map(m) => Value::Map(
            m.iter()
                .filter(|(k, _)| !matches!(k, Value::Str(t) if t == "identity_file"))
                .cloned()
                .collect(),
        ),
        other => other.clone(),
    }
}
