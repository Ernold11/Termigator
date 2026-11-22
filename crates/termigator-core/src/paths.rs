//! Config directory (XDG / `TERMIGATOR_CONFIG_DIR`) and private runtime directories.
//!
//! Design notes:
//!
//! * Nothing here reads or writes the process environment on its own: every lookup goes through
//!   an [`EnvSource`], so tests (and the golden checks) pass a closure instead of calling `set_var`.
//! * The app lives in `$XDG_CONFIG_HOME/termigator` ([`APP_DIR`]); there is no repo-checkout
//!   config mode (ADR-13).
//! * `~user` (another user's home) is not expanded in `XDG_CONFIG_HOME` / `TERMIGATOR_CONFIG_DIR`;
//!   `~` and `~/...` are, using `$HOME`.
//! * Intermediate directories of a private subdirectory are created 0700 too (they sit under a
//!   0700 base, so this is never weaker than the umask default).

use crate::platform;
use std::ffi::{OsStr, OsString};
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

/// Application directory name from the cutover on.
pub const CUTOVER_APP_DIR: &str = "termigator";

/// Directory name under the XDG config and runtime dirs.
pub const APP_DIR: &str = CUTOVER_APP_DIR;
/// The directory name below `$XDG_CONFIG_HOME` / `$XDG_RUNTIME_DIR` / the tmp dir, as data so tests can vary it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Layout {
    pub app_dir: &'static str,
}

impl Layout {
    /// What the running app uses.
    pub const ACTIVE: Layout = Layout { app_dir: APP_DIR };
}

/// Environment lookups. Closures `Fn(&str) -> Option<OsString>` work, as does [`ProcessEnv`].
pub trait EnvSource {
    fn var(&self, key: &str) -> Option<OsString>;
}

impl<F: Fn(&str) -> Option<OsString>> EnvSource for F {
    fn var(&self, key: &str) -> Option<OsString> {
        self(key)
    }
}

/// The real process environment.
pub struct ProcessEnv;

impl EnvSource for ProcessEnv {
    fn var(&self, key: &str) -> Option<OsString> {
        std::env::var_os(key)
    }
}

/// Resolved file layout.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Paths {
    /// The candidate root (the source tree of a development build).
    pub repo_root: PathBuf,
    pub config_dir: PathBuf,
    pub config_path: PathBuf,
    pub session_path: PathBuf,
    pub config_example_path: PathBuf,
    pub session_example_path: PathBuf,
}

/// Lexical normalisation like `pathlib` (drops `.`, repeated and trailing slashes).
fn norm(p: impl AsRef<Path>) -> PathBuf {
    p.as_ref().components().collect()
}

fn home(env: &dyn EnvSource) -> Option<PathBuf> {
    env.var("HOME")
        .filter(|h| !h.is_empty())
        .map(PathBuf::from)
        .or_else(std::env::home_dir)
}

/// `Path.expanduser` for `~` and `~/...` only.
fn expand_user(p: &OsStr, env: &dyn EnvSource) -> PathBuf {
    let bytes = platform::os_str_bytes(p);
    let b = &*bytes;
    if (b == b"~" || b.starts_with(b"~/"))
        && let Some(h) = home(env)
    {
        let rest = &b[1..];
        let skip = rest.iter().take_while(|&&c| c == b'/').count();
        return norm(h.join(platform::os_str_from_bytes(&rest[skip..])));
    }
    norm(p)
}

/// Strips leading/trailing whitespace on an env value (non-UTF-8 values are kept as they are).
fn strip(v: &OsStr) -> OsString {
    v.to_str()
        .map_or_else(|| v.to_owned(), |s| OsString::from(s.trim()))
}

/// `$XDG_CONFIG_HOME`, else `~/.config` (on Windows `%APPDATA%`).
fn config_base(env: &dyn EnvSource) -> PathBuf {
    config_base_for(cfg!(windows), env)
}

/// [`config_base`] with the platform as an argument, so both rules are testable anywhere.
pub fn config_base_for(windows: bool, env: &dyn EnvSource) -> PathBuf {
    if let Some(x) = env.var("XDG_CONFIG_HOME").filter(|x| !x.is_empty()) {
        return expand_user(&x, env);
    }
    if windows {
        let appdata = env.var("APPDATA").filter(|x| !x.is_empty());
        return appdata.map_or_else(
            || home(env).map_or_else(|| PathBuf::from("."), |h| h.join("AppData").join("Roaming")),
            PathBuf::from,
        );
    }
    home(env).map_or_else(|| PathBuf::from(".config"), |h| norm(h).join(".config"))
}

impl Paths {
    /// Resolve the layout. `root_candidate` is the directory that would be the repo root.
    pub fn resolve(layout: Layout, env: &dyn EnvSource, root_candidate: &Path) -> Paths {
        let repo_root = norm(root_candidate);
        let mut config_dir = config_base(env).join(layout.app_dir);
        if let Some(o) = env.var("TERMIGATOR_CONFIG_DIR") {
            let o = strip(&o);
            if !o.is_empty() {
                config_dir = expand_user(&o, env);
            }
        }
        Paths {
            config_path: config_dir.join("config.yaml"),
            session_path: config_dir.join("session.yaml"),
            config_example_path: config_dir.join("config.example.yaml"),
            session_example_path: config_dir.join("session.example.yaml"),
            config_dir,
            repo_root,
        }
    }
}

/// [`private_runtime_dir_in`] with the process environment, the system tmp dir, the real uid and
/// [`Layout::ACTIVE`].
pub fn private_runtime_dir(sub: &[&str]) -> Option<PathBuf> {
    private_runtime_dir_in(
        Layout::ACTIVE,
        &ProcessEnv,
        &std::env::temp_dir(),
        platform::current_uid(),
        sub,
    )
}

/// A directory only this user can use, for sockets and generated files.
///
/// `$XDG_RUNTIME_DIR/<app>` when that variable names an existing directory, else
/// `<tmpdir>/<app>-<uid>` - a shared place, so it is accepted only if it is a real directory
/// (not a symlink) owned by `uid`; our own, if too open, is tightened to 0700. `None` when nothing
/// safe is available (any I/O error included): callers then go without the feature rather than
/// use a shared directory.
pub fn private_runtime_dir_in(
    layout: Layout,
    env: &dyn EnvSource,
    tmpdir: &Path,
    uid: u32,
    sub: &[&str],
) -> Option<PathBuf> {
    let base = match env.var("XDG_RUNTIME_DIR").filter(|x| !x.is_empty()) {
        Some(x) if Path::new(&x).is_dir() => norm(x).join(layout.app_dir),
        _ => norm(tmpdir).join(format!("{}-{uid}", layout.app_dir)),
    };
    if let Some(parent) = base.parent() {
        fs::create_dir_all(parent).ok()?;
    }
    match platform::private_dir_create(&base, false) {
        // An existing entry (even a symlink to a directory) is judged by the lstat below.
        Err(e) if e.kind() == ErrorKind::AlreadyExists => {}
        r => r.ok()?,
    }
    let md = fs::symlink_metadata(&base).ok()?;
    if !md.is_dir() || !platform::owned_by(&md, uid) {
        return None;
    }
    if platform::open_to_others(&md) {
        platform::set_private_dir(&base).ok()?;
    }
    let d = sub.iter().fold(base, |p, s| p.join(s));
    platform::private_dir_create(&d, true).ok()?;
    Some(d)
}
