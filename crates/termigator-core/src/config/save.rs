//! config.yaml + session.yaml: load, merge/strip
//! defaults, save.
//!
//! - [`load`]: `load_config` — reads config.yaml, else the example (anything that is not
//!   a mapping reads as `{}`, like `_load_yaml_dict`), then lets a non-empty session.yaml
//!   (list or `{connections: [...]}`) replace `connections`.
//! - [`merge_defaults`] / [`strip_defaults_for_save`]: loose equality via [`Value::loose_eq`]
//!   (`true == 1 == 1.0`); unknown keys kept; an empty `window` / `launch` is dropped.
//! - [`save`]: `save_config` — each file is rewritten only when its parsed content
//!   changes; config.yaml keeps its comments ([`update_preserving_comments`]); every write
//!   is atomic ([`write_atomic`]).
//!
//! Differences by design: a truthy non-mapping `window` / `launch` merges as `{}`; the caller says
//! whether a vault is configured (`vault_configured`) instead of `save` reading `cfg["vault"]`;
//! config.yaml edits keep values equal to what the file holds verbatim (see `doc.rs`).
//! [`strip_plaintext_passwords`] is `crate::vault`'s. [`default_config`] lives in `model.rs` and
//! is re-exported here.

use crate::platform;
use std::fs;
use std::hash::{BuildHasher, RandomState};
use std::io::{self, ErrorKind, Write};
use std::path::{Path, PathBuf};

use super::doc::{self, Value, YamlError, dump, update_preserving_comments};
pub use super::model::default_config;
use super::model::{s, truthy};
use crate::paths::Paths;

/// First line of every session.yaml the app writes.
pub const SESSION_HEADER: &str = "# Termigator SSH connections — edited by the app or by hand.\n";

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("{}: {source}", path.display())]
    Io { path: PathBuf, source: io::Error },
    #[error("{}: {source}", path.display())]
    Yaml { path: PathBuf, source: YamlError },
}

fn io_err(path: &Path) -> impl FnOnce(io::Error) -> ConfigError + '_ {
    move |source| ConfigError::Io {
        path: path.to_owned(),
        source,
    }
}

fn pairs(v: &Value) -> &[(Value, Value)] {
    match v {
        Value::Map(p) => p,
        _ => &[],
    }
}

/// `{**a, **b}`: keys of `a` keep their place, new keys of `b` are appended.
fn update(out: &mut Vec<(Value, Value)>, from: &[(Value, Value)]) {
    for (k, v) in from {
        match out.iter_mut().find(|(k2, _)| k2.loose_eq(k)) {
            Some(slot) => slot.1 = v.clone(),
            None => out.push((k.clone(), v.clone())),
        }
    }
}

fn set(m: &mut Vec<(Value, Value)>, key: &str, v: Value) {
    update(m, &[(s(key), v)]);
}

fn get<'a>(m: &'a [(Value, Value)], key: &Value) -> Option<&'a Value> {
    m.iter().find(|(k, _)| k.loose_eq(key)).map(|(_, v)| v)
}

/// `merge_defaults`: defaults under `cfg`, `window` / `launch` merged one level deep,
/// non-list `connections` / `favorites` reset to `[]`.
pub fn merge_defaults(cfg: &Value) -> Value {
    Value::Map(merge_pairs(cfg))
}

/// [`merge_defaults`] for an owned document: the `connections` / `favorites` lists are moved,
/// not cloned (the app loads them as owned values).
pub fn merge_defaults_owned(mut cfg: Value) -> Value {
    let (mut conns, mut favs) = (None, None);
    if let Value::Map(pairs) = &mut cfg {
        for (k, v) in pairs.iter_mut() {
            match k {
                Value::Str(s) if s == "connections" => {
                    if matches!(v, Value::Seq(_)) {
                        conns = Some(std::mem::replace(v, Value::Seq(Vec::new())));
                    } else {
                        *v = Value::Seq(Vec::new());
                    }
                }
                Value::Str(s) if s == "favorites" => {
                    if matches!(v, Value::Seq(_)) {
                        favs = Some(std::mem::replace(v, Value::Seq(Vec::new())));
                    } else {
                        *v = Value::Seq(Vec::new());
                    }
                }
                _ => {}
            }
        }
    }
    let mut out = merge_pairs(&cfg);
    if let Some(v) = conns {
        set_owned(&mut out, "connections", v);
    }
    if let Some(v) = favs {
        set_owned(&mut out, "favorites", v);
    }
    Value::Map(out)
}

/// [`set`] without cloning the value (takes ownership).
fn set_owned(m: &mut Vec<(Value, Value)>, key: &str, v: Value) {
    if let Some(slot) = m
        .iter_mut()
        .find(|(k, _)| matches!(k, Value::Str(s) if s == key))
    {
        slot.1 = v;
    } else {
        m.push((s(key), v));
    }
}

fn merge_pairs(cfg: &Value) -> Vec<(Value, Value)> {
    let base = default_config();
    let cfg = pairs(cfg);
    let mut out = pairs(&base).to_vec();
    update(&mut out, cfg);
    for sec in ["window", "launch"] {
        let mut m = base.get(sec).map(pairs).unwrap_or_default().to_vec();
        if let Some(v) = get(cfg, &s(sec)).filter(|v| truthy(v)) {
            update(&mut m, pairs(v));
        }
        set(&mut out, sec, Value::Map(m));
    }
    for list in ["connections", "favorites"] {
        let v = match get(cfg, &s(list)) {
            Some(v @ Value::Seq(_)) => v.clone(),
            _ => Value::Seq(Vec::new()),
        };
        set(&mut out, list, v);
    }
    out
}

/// `strip_defaults_for_save`: drop values equal (loose equality) to the defaults.
pub fn strip_defaults_for_save(cfg: &Value) -> Value {
    Value::Map(strip_pairs(cfg))
}

fn strip_pairs(cfg: &Value) -> Vec<(Value, Value)> {
    let base = default_config();
    let base = pairs(&base);
    let mut out = Vec::new();
    for (k, v) in pairs(cfg) {
        let is_section = matches!(k, Value::Str(t) if t == "window" || t == "launch");
        if is_section && let Value::Map(sec) = v {
            let defaults = get(base, k).map(pairs).unwrap_or_default();
            let kept: Vec<_> = sec
                .iter()
                .filter(|(sk, sv)| !get(defaults, sk).is_some_and(|d| d.loose_eq(sv)))
                .cloned()
                .collect();
            if !kept.is_empty() {
                out.push((k.clone(), Value::Map(kept)));
            }
        } else if !get(base, k).is_some_and(|d| d.loose_eq(v)) {
            out.push((k.clone(), v.clone()));
        }
    }
    out
}

pub use crate::vault::strip_plaintext_passwords;

fn read_yaml(path: &Path) -> Result<Option<Value>, ConfigError> {
    if !path.is_file() {
        return Ok(None);
    }
    let text = fs::read_to_string(path).map_err(io_err(path))?;
    doc::load(&text)
        .map(Some)
        .map_err(|source| ConfigError::Yaml {
            path: path.to_owned(),
            source,
        })
}

/// `_load_yaml_dict`: a missing file, a falsy or a non-mapping document is `{}`.
fn load_yaml_dict(path: &Path) -> Result<Value, ConfigError> {
    Ok(match read_yaml(path)? {
        Some(v @ Value::Map(_)) => v,
        _ => Value::Map(Vec::new()),
    })
}

fn only_maps(v: &Value) -> Vec<Value> {
    match v {
        Value::Seq(x) => x
            .iter()
            .filter(|c| matches!(c, Value::Map(_)))
            .cloned()
            .collect(),
        _ => Vec::new(),
    }
}

/// `_load_session_connections`: mappings from a list or from `{connections: [...]}`.
fn load_session_connections(paths: &Paths) -> Result<Vec<Value>, ConfigError> {
    Ok(match read_yaml(&paths.session_path)? {
        Some(v @ Value::Seq(_)) => only_maps(&v),
        Some(v @ Value::Map(_)) => v.get("connections").map(only_maps).unwrap_or_default(),
        _ => Vec::new(),
    })
}

/// `load_config`: the config document (not merged with defaults).
pub fn load(paths: &Paths) -> Result<Value, ConfigError> {
    let source = [&paths.config_path, &paths.config_example_path]
        .into_iter()
        .find(|p| p.is_file());
    let mut data = match source {
        Some(p) => load_yaml_dict(p)?,
        None => Value::Map(Vec::new()),
    };
    let session = load_session_connections(paths)?;
    if let Value::Map(m) = &mut data {
        if !session.is_empty() {
            set(m, "connections", Value::Seq(session));
        } else if !matches!(get(m, &s("connections")), Some(Value::Seq(_))) {
            set(m, "connections", Value::Seq(Vec::new()));
        }
    }
    Ok(data)
}

/// `save_config`. `vault_configured`: a master password exists, so unsealed passwords
/// in `connections` and `credentials` are not written.
pub fn save(paths: &Paths, cfg: &Value, vault_configured: bool) -> Result<(), ConfigError> {
    let mut merged = merge_pairs(cfg);
    fs::create_dir_all(&paths.config_dir).map_err(io_err(&paths.config_dir))?;

    let mut conns = get(&merged, &s("connections"))
        .map(only_maps)
        .unwrap_or_default();
    if vault_configured {
        conns = strip_plaintext_passwords(&conns);
        if let Some(Value::Seq(creds)) = get(&merged, &s("credentials")) {
            let creds = strip_plaintext_passwords(creds);
            set(&mut merged, "credentials", Value::Seq(creds));
        }
    }
    let main = Value::Map(
        strip_pairs(&Value::Map(merged))
            .into_iter()
            .filter(|(k, _)| !matches!(k, Value::Str(t) if t == "connections"))
            .collect(),
    );

    let conns = Value::Seq(conns);
    let session_text = (!Value::Seq(load_session_connections(paths)?).loose_eq(&conns))
        .then(|| SESSION_HEADER.to_owned() + &dump(&conns));

    let cfg_path = &paths.config_path;
    let cfg_text = if load_yaml_dict(cfg_path)?.loose_eq(&main) {
        None
    } else if cfg_path.is_file() {
        let original = fs::read_to_string(cfg_path).map_err(io_err(cfg_path))?;
        Some(update_preserving_comments(&original, &main))
    } else {
        Some(dump(&main))
    };

    let Some(session_text) = session_text else {
        return match cfg_text {
            Some(t) => write_atomic(cfg_path, &t).map_err(io_err(cfg_path)),
            None => Ok(()),
        };
    };
    // Both files change (vault operation): if the second write fails, the first is rolled
    // back, or the passwords would be sealed under a key whose salt was never saved.
    // shortcut: a kill between the two writes is not recovered; the `.bak` stays beside the file.
    let sp = &paths.session_path;
    let before = cfg_text.as_ref().and_then(|_| fs::read_to_string(sp).ok());
    let bak = sp.with_extension("yaml.bak");
    if let Some(b) = &before {
        write_atomic(&bak, b).map_err(io_err(&bak))?;
    }
    write_atomic(sp, &session_text).map_err(io_err(sp))?;
    if let Some(t) = cfg_text {
        if let Err(e) = write_atomic(cfg_path, &t) {
            if let Some(b) = &before {
                let _ = write_atomic(sp, b);
            }
            let _ = fs::remove_file(&bak);
            return Err(io_err(cfg_path)(e));
        }
        let _ = fs::remove_file(&bak);
    }
    Ok(())
}

/// Once at start: an installed layout's config directory becomes 0700 and its YAML files
/// 0600 (they can hold plaintext passwords). Failures (not our files) are ignored.
pub fn tighten_modes(paths: &Paths) {
    if paths.config_dir.is_dir() {
        let _ = platform::set_private_dir(&paths.config_dir);
    }
    for f in [&paths.config_path, &paths.session_path] {
        if f.is_file() {
            let _ = platform::set_private_file(f);
        }
    }
}

/// The file a write to `path` lands in: symlinks followed (dangling ones too).
fn resolve_symlinks(path: &Path) -> io::Result<PathBuf> {
    let mut target = path.to_owned();
    for _ in 0..40 {
        match fs::symlink_metadata(&target) {
            Ok(md) if md.file_type().is_symlink() => {
                let link = fs::read_link(&target)?;
                target = match target.parent() {
                    Some(dir) => dir.join(link),
                    None => link,
                };
            }
            _ => return Ok(target),
        }
    }
    Err(io::Error::other("too many levels of symbolic links"))
}

/// `_atomic_write_text`: the file holds either the old or the new text, never a
/// mix. Writes the symlink target; temp `.<name>.<random>.tmp` beside it (0600), fsync,
/// the old file's mode copied, rename, fsync of the directory. The temp file is removed
/// on any error.
pub fn write_atomic(path: &Path, text: &str) -> io::Result<()> {
    let target = resolve_symlinks(path)?;
    let dir = match target.parent() {
        Some(d) if !d.as_os_str().is_empty() => d.to_owned(),
        _ => PathBuf::from("."),
    };
    let name = target
        .file_name()
        .ok_or_else(|| io::Error::new(ErrorKind::InvalidInput, "no file name"))?
        .to_string_lossy()
        .into_owned();
    fs::create_dir_all(&dir)?;
    let (tmp, mut file) = loop {
        let tmp = dir.join(format!(
            ".{name}.{:016x}.tmp",
            RandomState::new().hash_one(std::process::id())
        ));
        match platform::private_file(fs::OpenOptions::new().write(true).create_new(true)).open(&tmp)
        {
            Err(e) if e.kind() == ErrorKind::AlreadyExists => continue,
            r => break (tmp.clone(), r?),
        }
    };
    let result = (|| {
        file.write_all(text.as_bytes())?;
        file.sync_all()?;
        match fs::metadata(&target) {
            Ok(md) => platform::copy_permissions(&file, &md)?,
            Err(e) if e.kind() == ErrorKind::NotFound => {} // new file: stays 0600
            Err(e) => return Err(e),
        }
        drop(file);
        fs::rename(&tmp, &target)
    })();
    if let Err(e) = result {
        // Best effort: the original error is what the caller needs to see.
        let _ = fs::remove_file(&tmp);
        return Err(e);
    }
    platform::sync_dir(&dir)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn owned_merge_matches_the_borrowed_one() {
        let text = "connections: [{host: a, user: u}]\nfavorites: nope\nwindow: {width: 1}\n";
        let parsed = doc::load(text).expect("yaml");
        assert_eq!(
            merge_defaults_owned(parsed.clone()),
            merge_defaults(&parsed)
        );
    }

    #[test]
    fn truthiness_and_password_strip() {
        let entries = doc::load(
            "- {host: a, password: plain}\n- {host: b, password: 'enc:v1:x'}\n- {host: c, password: ''}\n- {host: d, password: 0}\n- not a map\n",
        )
        .expect("yaml");
        let Value::Seq(entries) = entries else {
            panic!("seq")
        };
        let out = strip_plaintext_passwords(&entries);
        assert_eq!(out[0].get("password"), None);
        assert_eq!(out[0].get("host"), Some(&s("a")));
        assert_eq!(&out[1..], &entries[1..]);
    }

    #[cfg(unix)] // needs a symlink
    #[test]
    fn failed_second_write_rolls_the_session_file_back() {
        let dir = std::env::temp_dir().join(format!("termigator-save-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        // config.yaml -> a path below a regular file: its write must fail.
        fs::write(dir.join("blocker"), "x").unwrap();
        std::os::unix::fs::symlink(dir.join("blocker/x"), dir.join("config.yaml")).unwrap();
        let session = dir.join("session.yaml");
        fs::write(&session, "- {host: old}\n").unwrap();
        let paths = Paths {
            repo_root: dir.clone(),
            config_dir: dir.clone(),
            config_path: dir.join("config.yaml"),
            session_path: session.clone(),
            config_example_path: dir.join("none1"),
            session_example_path: dir.join("none2"),
        };
        let cfg = doc::load("connections: [{host: new}]\nvault: {n: 2}\n").unwrap();
        assert!(save(&paths, &cfg, false).is_err());
        assert_eq!(fs::read_to_string(&session).unwrap(), "- {host: old}\n");
        assert!(!dir.join("session.yaml.bak").exists());
        let _ = fs::remove_dir_all(&dir);
    }
}
