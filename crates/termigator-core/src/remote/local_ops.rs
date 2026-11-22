//! File operations of the local Files pane (right-click menu): New folder, Rename, Delete.
//! Pure `std::fs`, so the same code serves Linux and Windows; the QML only asks and shows the error.

use std::fs;
use std::path::{Path, PathBuf};

/// Characters Windows refuses in a name (also refused elsewhere: a name typed on one system
/// should be valid on the other).
const WINDOWS_INVALID: &[char] = &['<', '>', ':', '"', '|', '?', '*'];

/// The trimmed `name` when it is one plain file name.
pub fn valid_name(name: &str) -> Result<&str, String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("The name is empty.".to_owned());
    }
    if name == "." || name == ".." {
        return Err(format!("\"{name}\" is not a usable name."));
    }
    if name.contains(['/', '\\', '\0']) {
        return Err("A name cannot contain / or \\.".to_owned());
    }
    if cfg!(windows) && name.contains(WINDOWS_INVALID) {
        return Err("A name cannot contain any of < > : \" | ? *".to_owned());
    }
    Ok(name)
}

/// Creates the folder `name` in `parent`.
pub fn mkdir(parent: &Path, name: &str) -> Result<PathBuf, String> {
    let name = valid_name(name)?;
    let path = parent.join(name);
    if path.symlink_metadata().is_ok() {
        return Err(format!("\"{name}\" already exists."));
    }
    fs::create_dir(&path).map_err(|e| format!("Cannot create \"{name}\": {e}"))?;
    Ok(path)
}

/// Renames `path` to `new_name` in the same folder. Never replaces an existing entry.
pub fn rename(path: &Path, new_name: &str) -> Result<PathBuf, String> {
    let new_name = valid_name(new_name)?;
    let parent = path.parent().ok_or("This cannot be renamed.")?;
    let target = parent.join(new_name);
    if target == path {
        return Ok(target);
    }
    if target.symlink_metadata().is_ok() {
        return Err(format!("\"{new_name}\" already exists."));
    }
    fs::rename(path, &target).map_err(|e| format!("Cannot rename to \"{new_name}\": {e}"))?;
    Ok(target)
}

/// Deletes each path (a folder with its content; a link is removed, not followed). Keeps going
/// after a failure and reports every one.
pub fn delete(paths: &[PathBuf]) -> Result<(), String> {
    let mut errors = Vec::new();
    for p in paths {
        if p.parent().is_none() {
            errors.push(format!("{}: a root cannot be deleted", p.display()));
            continue;
        }
        let result = match p.symlink_metadata() {
            Ok(m) if m.is_dir() => fs::remove_dir_all(p),
            Ok(_) => fs::remove_file(p),
            Err(e) => Err(e),
        };
        if let Err(e) = result {
            errors.push(format!("{}: {e}", p.display()));
        }
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join("\n"))
    }
}
