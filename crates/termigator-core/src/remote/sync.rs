//! Folder sync planner and path-traversal guards, plus the pure part of the Synchronize dialog.
//!
//! WinSCP-style "Synchronize": both trees are listed recursively, compared by size and mtime and
//! turned into a plan the user reviews before anything runs. Deliberately conservative:
//! nothing is ever deleted (missing files are copied, never removed); a file is changed only when
//! its size differs or its mtime differs by more than [`MTIME_SLACK`] seconds; in two-way mode
//! the newer side wins and same mtime with a different size is a conflict left alone; folders
//! are planned before files so uploads find their parent.
//!
//! ## Guards (ARCHITECTURE.md section 7, guard 10)
//! The server's `find` output is attacker-controlled, so a relative path like `../../x` would
//! make a download write outside the chosen local folder:
//! - [`safe_rel`] rejects empty paths, a leading `/` and empty, `.` or `..` segments;
//! - [`parse_remote_tree`] leaves such paths out and reports them;
//! - [`local_target`] re-checks that the joined path stays inside the root and is not the root;
//! - [`RunPlan::prepare`] checks every action's local target before anything is created or
//!   queued, and refuses the whole plan if one fails.
//!
//! Remote paths are bytes (ADR-12), local paths are `Path`/`PathBuf`. The
//! remote root is quoted literally, so a typed `~` is not expanded.
//!
//! Golden fixtures: `compat/golden/sync_*.json`, tests in `tests/remote_sync.rs`.
//!
//! Limits (none occur with real `find` output or sane filesystems):
//! - Sizes and mtimes are `i64` (huge float mtimes saturate); comparisons are done in `i128`.
//! - Numbers in `find` records are parsed like `int()` / `float()` for ASCII text only; a
//!   non-ASCII digit makes the record skipped. `inf` / `nan` mtimes skip the record.
//! - [`local_target`] also refuses a path containing NUL, and a leading `//` of the root is
//!   collapsed to `/`.
//! - [`scan_local`] skips entries it cannot stat instead of raising, like it skips unreadable
//!   folders (`os.walk` does the latter too).

use crate::platform;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Component, Path, PathBuf};

use crate::config::doc::Value;
use crate::config::model::{strip_space, value_int};
use crate::remote::listing::{Argv, RemoteOps, quote_bytes, remote_join};

/// FAT / SMB round mtimes to 2 s.
pub const MTIME_SLACK: i64 = 2;
pub const SYNC_MARKER: &str = "@@TERMIGATOR-SYNC@@";

/// Shown when a plan contains a path that leaves the local folder.
pub const OUTSIDE_MSG: &str =
    "A path in the plan points outside the local folder — nothing was copied.";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Node {
    pub is_dir: bool,
    pub size: i64,
    pub mtime: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Upload,
    Download,
    Both,
}

impl Direction {
    /// `"upload"`, `"download"` or `"both"`.
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "upload" => Some(Self::Upload),
            "download" => Some(Self::Download),
            "both" => Some(Self::Both),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncOp {
    Upload,
    Download,
    MkdirRemote,
    MkdirLocal,
    Conflict,
}

impl SyncOp {
    /// The op string.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Upload => "upload",
            Self::Download => "download",
            Self::MkdirRemote => "mkdir-remote",
            Self::MkdirLocal => "mkdir-local",
            Self::Conflict => "conflict",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncAction {
    pub op: SyncOp,
    /// Path relative to both roots, `/`-separated.
    pub rel: Vec<u8>,
    pub reason: &'static str,
    pub size: i64,
}

/// One-way copies of a file that is older than the one it replaces.
pub const OLDER_HERE: &str = "older here — replaces a newer file on the server";
pub const OLDER_THERE: &str = "older on the server — replaces a newer file here";
const MISSING_THERE: &str = "missing on the server";
const MISSING_HERE: &str = "missing here";

impl SyncAction {
    /// The copy overwrites a newer file at its destination (left unticked in the plan).
    pub fn replaces_newer(&self) -> bool {
        self.reason == OLDER_HERE || self.reason == OLDER_THERE
    }

    /// The copy overwrites an existing file at its destination (the run asks first).
    pub fn replaces_existing(&self) -> bool {
        matches!(self.op, SyncOp::Upload | SyncOp::Download)
            && self.reason != MISSING_THERE
            && self.reason != MISSING_HERE
    }
}

/// A tree keyed by relative path (byte order, same as sorting the UTF-8 text).
pub type Tree = BTreeMap<Vec<u8>, Node>;

/// Every file and folder under `root`: `type NUL size NUL mtime NUL relpath NUL`, after a marker
/// line and a GNU `find` check (BusyBox / BSD hosts exit 3 with a clear message).
pub fn remote_tree_argv(ops: &RemoteOps<'_>, root: &[u8]) -> Result<Argv, String> {
    let script = [
        b"cd -- ".as_slice(),
        &quote_bytes(root),
        b" || exit 1; find . -maxdepth 0 -printf '' >/dev/null 2>&1 || ",
        b"{ echo 'GNU find is needed on the server' >&2; exit 3; }; echo ",
        SYNC_MARKER.as_bytes(),
        br"; find . -mindepth 1 \( -type f -o -type d \) -printf '%y\0%s\0%T@\0%P\0'",
    ]
    .concat();
    ops.exec_argv(&[b"exec sh -c ".as_slice(), &quote_bytes(&script)].concat())
}

/// A relative path the server may hand us: no `..` / `.` / empty parts, not absolute.
pub fn safe_rel(rel: &[u8]) -> bool {
    !rel.is_empty()
        && !rel.starts_with(b"/")
        && rel
            .split(|b| *b == b'/')
            .all(|p| !p.is_empty() && p != b"." && p != b"..")
}

/// `os.path.abspath`: absolute (relative to the cwd) with `.`, `..` and `//` folded lexically.
fn abspath(p: &Path) -> Option<PathBuf> {
    let full = if p.is_absolute() {
        p.to_path_buf()
    } else {
        std::env::current_dir().ok()?.join(p)
    };
    // The drive / UNC prefix stays: without it a Windows path names the current drive instead.
    let mut out = PathBuf::new();
    for c in full.components() {
        match c {
            Component::Prefix(_) | Component::RootDir => out.push(c.as_os_str()),
            Component::Normal(n) => out.push(n),
            // `pop` stops at the root, as `..` does.
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
        }
    }
    Some(out)
}

/// `root/rel` on this machine, or `None` when `rel` is unsafe or the result would leave `root`
/// (or be `root` itself).
pub fn local_target(root: &Path, rel: &[u8]) -> Option<PathBuf> {
    if !safe_rel(rel) || rel.contains(&0) {
        return None;
    }
    // On Windows `\` separates and `C:x` replaces the whole path when joined; neither is a valid
    // file name character there anyway.
    if cfg!(windows) && rel.iter().any(|b| matches!(b, b'\\' | b':')) {
        return None;
    }
    let root_abs = abspath(root)?;
    let mut joined = root_abs.clone();
    joined.extend(rel.split(|b| *b == b'/').map(platform::os_str_from_bytes));
    let path = abspath(&joined)?;
    (path.starts_with(&root_abs) && path != root_abs).then_some(path)
}

/// Parses a float for the shapes `find -printf %T@` produces and a few more.
fn parse_float(s: &[u8]) -> Option<f64> {
    let t = strip_space(std::str::from_utf8(s).ok()?);
    let b = t.as_bytes();
    let underscore_ok = b.iter().enumerate().all(|(i, c)| {
        *c != b'_'
            || (i > 0
                && b.get(i + 1).is_some()
                && b[i - 1].is_ascii_digit()
                && b[i + 1].is_ascii_digit())
    });
    if !underscore_ok {
        return None;
    }
    t.replace('_', "").parse::<f64>().ok()
}

/// Parses a decimal integer.
fn parse_int_bytes(s: &[u8]) -> Option<i64> {
    value_int(&Value::Str(std::str::from_utf8(s).ok()?.to_owned()))
}

/// The remote listing (the output after [`SYNC_MARKER`] and a newline); unsafe paths (see
/// [`safe_rel`]) are left out of the tree and returned as the second value. Records with an
/// unknown type or unparsable numbers are skipped silently.
pub fn parse_remote_tree(out: &[u8]) -> (Tree, Vec<Vec<u8>>) {
    let marker = [SYNC_MARKER.as_bytes(), b"\n"].concat();
    let mut tree = Tree::new();
    let mut rejected = Vec::new();
    let Some(at) = out.windows(marker.len()).position(|w| w == marker) else {
        return (tree, rejected);
    };
    let fields: Vec<&[u8]> = out[at + marker.len()..].split(|b| *b == 0).collect();
    for &[kind, size_s, mtime_s, rel] in fields.as_chunks::<4>().0 {
        if rel.is_empty() || (kind != b"f" && kind != b"d") {
            continue;
        }
        if !safe_rel(rel) {
            rejected.push(rel.to_vec());
            continue;
        }
        let is_dir = kind == b"d";
        let size = if is_dir {
            Some(0)
        } else {
            parse_int_bytes(size_s)
        };
        let mtime = parse_float(mtime_s)
            .filter(|f| f.is_finite())
            .map(|f| f.trunc() as i64);
        if let (Some(size), Some(mtime)) = (size, mtime) {
            tree.insert(
                rel.to_vec(),
                Node {
                    is_dir,
                    size,
                    mtime,
                },
            );
        }
    }
    (tree, rejected)
}

/// Whole-second mtime: the float is built as `sec + nsec * 1e-9` and truncated.
fn mtime_of(md: &fs::Metadata) -> i64 {
    let (sec, nsec) = platform::mtime_parts(md);
    (sec as f64 + nsec as f64 * 1e-9).trunc() as i64
}

/// Files and folders under `root`; symlinks are not followed (nor listed), and only regular
/// files and folders are. A `root` that is not a folder gives an empty tree.
pub fn scan_local(root: &Path) -> Tree {
    let mut tree = Tree::new();
    walk(root, &[], &mut tree);
    tree
}

fn walk(dir: &Path, rel: &[u8], tree: &mut Tree) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let Ok(md) = fs::symlink_metadata(entry.path()) else {
            continue;
        };
        let name = entry.file_name();
        let mut key = rel.to_vec();
        if !key.is_empty() {
            key.push(b'/');
        }
        key.extend_from_slice(&platform::os_str_bytes(&name));
        if md.is_dir() {
            tree.insert(
                key.clone(),
                Node {
                    is_dir: true,
                    size: 0,
                    mtime: mtime_of(&md),
                },
            );
            walk(&entry.path(), &key, tree);
        } else if md.is_file() {
            tree.insert(
                key,
                Node {
                    is_dir: false,
                    size: i64::try_from(md.len()).unwrap_or(i64::MAX),
                    mtime: mtime_of(&md),
                },
            );
        }
    }
}

fn differs(a: &Node, b: &Node) -> bool {
    a.size != b.size || (i128::from(a.mtime) - i128::from(b.mtime)).abs() > i128::from(MTIME_SLACK)
}

fn act(op: SyncOp, rel: &[u8], reason: &'static str, size: i64) -> SyncAction {
    SyncAction {
        op,
        rel: rel.to_vec(),
        reason,
        size,
    }
}

/// What to copy so the target side(s) catch up: folders first, then files, each by path.
pub fn plan_sync(local: &Tree, remote: &Tree, direction: Direction) -> Vec<SyncAction> {
    let up = matches!(direction, Direction::Upload | Direction::Both);
    let down = matches!(direction, Direction::Download | Direction::Both);
    let slack = i128::from(MTIME_SLACK);
    let mut dirs = Vec::new();
    let mut files = Vec::new();
    let rels: BTreeSet<&Vec<u8>> = local.keys().chain(remote.keys()).collect();
    for rel in rels {
        let (lo, re) = (local.get(rel), remote.get(rel));
        if let (Some(l), Some(r)) = (lo, re)
            && l.is_dir != r.is_dir
        {
            files.push(act(
                SyncOp::Conflict,
                rel,
                "a folder on one side, a file on the other",
                0,
            ));
            continue;
        }
        if lo.is_some_and(|n| n.is_dir) || re.is_some_and(|n| n.is_dir) {
            if lo.is_some() && re.is_none() && up {
                dirs.push(act(SyncOp::MkdirRemote, rel, "missing on the server", 0));
            } else if re.is_some() && lo.is_none() && down {
                dirs.push(act(SyncOp::MkdirLocal, rel, "missing here", 0));
            }
            continue;
        }
        match (lo, re) {
            (Some(l), None) => {
                if up {
                    files.push(act(SyncOp::Upload, rel, MISSING_THERE, l.size));
                }
            }
            (None, Some(r)) => {
                if down {
                    files.push(act(SyncOp::Download, rel, MISSING_HERE, r.size));
                }
            }
            (Some(l), Some(r)) if differs(l, r) => {
                let (lm, rm) = (i128::from(l.mtime), i128::from(r.mtime));
                let newer_local = lm > rm + slack;
                let newer_remote = rm > lm + slack;
                files.push(match direction {
                    Direction::Upload => act(
                        SyncOp::Upload,
                        rel,
                        if newer_local {
                            "newer here"
                        } else if newer_remote {
                            OLDER_HERE
                        } else {
                            "differs"
                        },
                        l.size,
                    ),
                    Direction::Download => act(
                        SyncOp::Download,
                        rel,
                        if newer_remote {
                            "newer on the server"
                        } else if newer_local {
                            OLDER_THERE
                        } else {
                            "differs"
                        },
                        r.size,
                    ),
                    Direction::Both if newer_local => {
                        act(SyncOp::Upload, rel, "newer here", l.size)
                    }
                    Direction::Both if newer_remote => {
                        act(SyncOp::Download, rel, "newer on the server", r.size)
                    }
                    Direction::Both => act(SyncOp::Conflict, rel, "same time, different size", 0),
                });
            }
            _ => {}
        }
    }
    dirs.extend(files);
    dirs
}

/// One file copy of a run (the caller adds scp argv, `-p` preserve and the connection).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CopyJob {
    pub upload: bool,
    pub local: PathBuf,
    pub remote: Vec<u8>,
    /// The relative path, shown as the job's label.
    pub label: Vec<u8>,
}

/// What running the user's selection needs, computed before anything is created or queued.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunPlan {
    /// Local folders to create (`mkdir-local` actions), in plan order.
    pub local_dirs: Vec<PathBuf>,
    /// Full remote paths to create (missing folders and the parents of uploads), sorted.
    pub remote_dirs: Vec<Vec<u8>>,
    pub copies: Vec<CopyJob>,
}

impl RunPlan {
    /// Defence in depth: every action's local target must stay inside `local_root`, else
    /// the whole selection is refused with [`OUTSIDE_MSG`]. Conflicts are checked but not run.
    pub fn prepare(
        local_root: &Path,
        remote_root: &[u8],
        todo: &[SyncAction],
    ) -> Result<Self, String> {
        let mut targets = Vec::with_capacity(todo.len());
        for a in todo {
            targets.push(local_target(local_root, &a.rel).ok_or_else(|| OUTSIDE_MSG.to_owned())?);
        }
        let mut plan = RunPlan {
            local_dirs: Vec::new(),
            remote_dirs: Vec::new(),
            copies: Vec::new(),
        };
        let mut rdirs: BTreeSet<&[u8]> = BTreeSet::new();
        for (a, target) in todo.iter().zip(targets) {
            match a.op {
                SyncOp::MkdirLocal => plan.local_dirs.push(target),
                SyncOp::MkdirRemote => {
                    rdirs.insert(&a.rel);
                }
                SyncOp::Upload | SyncOp::Download => {
                    let upload = a.op == SyncOp::Upload;
                    if upload && let Some(i) = a.rel.iter().rposition(|b| *b == b'/') {
                        rdirs.insert(&a.rel[..i]);
                    }
                    plan.copies.push(CopyJob {
                        upload,
                        local: target,
                        remote: remote_join(remote_root, &a.rel),
                        label: a.rel.clone(),
                    });
                }
                SyncOp::Conflict => {}
            }
        }
        plan.remote_dirs = rdirs
            .into_iter()
            .map(|d| remote_join(remote_root, d))
            .collect();
        Ok(plan)
    }

    /// `mkdir -p -- ...` for all remote folders in one ssh call; `None` when there are none.
    pub fn mkdir_argv(&self, ops: &RemoteOps<'_>) -> Option<Result<Argv, String>> {
        if self.remote_dirs.is_empty() {
            return None;
        }
        let mut cmd = b"mkdir -p --".to_vec();
        for d in &self.remote_dirs {
            cmd.push(b' ');
            cmd.extend(quote_bytes(d));
        }
        Some(ops.exec_argv(&cmd))
    }
}

#[cfg(all(test, windows))]
mod windows_tests {
    use super::*;

    #[test]
    fn local_target_keeps_the_drive_and_refuses_windows_tricks() {
        let root = Path::new(r"D:\sync\dst");
        assert_eq!(
            local_target(root, b"a/b.txt"),
            Some(PathBuf::from(r"D:\sync\dst\a\b.txt"))
        );
        for bad in [
            &b"..\\x"[..],
            b"a\\..\\..\\x",
            b"C:evil",
            b"a/C:x",
            b"..",
            b"a/../../x",
        ] {
            assert_eq!(
                local_target(root, bad),
                None,
                "{}",
                String::from_utf8_lossy(bad)
            );
        }
        let unc = Path::new(r"\\srv\share\dst");
        assert_eq!(
            local_target(unc, b"f"),
            Some(PathBuf::from(r"\\srv\share\dst\f"))
        );
    }
}
