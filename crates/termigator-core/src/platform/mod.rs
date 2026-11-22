//! Platform layer: the only place in core that may touch OS-specific APIs.
//! Domain code calls these free functions, no `cfg` outside.

/// Names to try for `name` on Windows: itself when it already ends in one of `pathext`
/// (`;`-separated, like `%PATHEXT%`), else `name` plus each extension, in that order.
pub fn pathext_names(name: &str, pathext: &str) -> Vec<String> {
    let exts: Vec<&str> = pathext.split(';').filter(|e| !e.is_empty()).collect();
    let lower = name.to_ascii_lowercase();
    if exts
        .iter()
        .any(|e| lower.ends_with(&e.to_ascii_lowercase()))
    {
        return vec![name.to_owned()];
    }
    exts.iter().map(|e| format!("{name}{e}")).collect()
}

/// One value of a registry key read by [`registry_keys`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RegData {
    Str(String),
    Dword(u32),
}

/// A registry key and its values; `name` is the subkey under the key read ("" for that key).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RegKeyValues {
    pub name: String,
    pub values: Vec<(String, RegData)>,
}

#[cfg(unix)]
mod unix;
#[cfg(unix)]
pub use unix::*;

#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub use windows::*;
