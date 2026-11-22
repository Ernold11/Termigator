//! Files panel logic without Qt.
//!
//! ## Target generation (ARCHITECTURE.md 4.3)
//! [`PanelState`] bumps a generation counter whenever the target host changes. Every command is
//! started with [`PanelState::begin_command`], which issues a [`Ticket`] `(generation, cwd)`; its
//! result is applied only if [`PanelState::accept`] (or, for a listing, which legitimately moves
//! the folder, [`PanelState::accept_listing`]) still matches. The same ticket guards the refresh
//! after an upload, and the DnD payload ([`PanelState::encode_drag`] / [`PanelState::resolve_drop`])
//! carries generation + folder. Sync/compare dialogs keep a [`HostBinding`] taken when they were
//! opened and never look at the panel's current target.
//!
//! ## Rows, bookmarks, edit-in-place
//! [`PanelState::rows`] is the display order (".." pinned first, folders before files in either
//! direction, hidden filter). Bookmarks and the last folder live in the `files_bookmarks` mapping
//! (`{host_key: {bookmarks: [...], last: path}}`) and are read/written on a [`Value`]. An
//! [`EditSession`] is the debounce (700 ms) + (size, sha1) fingerprint state machine; the clock and
//! the fingerprint function are injected.
//!
//! Notes: remote names are bytes, so the DnD JSON carries `cwd` and `name` as
//! lowercase hex; bookmarks and the last folder are stored in YAML only when they are valid UTF-8
//! (a non-UTF-8 path is silently not remembered). Sorting compares the lossily decoded,
//! lowercased name (case-folding approximated by `to_lowercase`), ties broken by the raw bytes.

use std::cmp::Ordering;
use std::path::{Path, PathBuf};

use percent_encoding::percent_decode_str;
use serde_json::{Value as J, json};
use sha1::{Digest, Sha1};

use crate::config::doc::Value;
use crate::remote::listing::{RemoteEntry, browse_conn, display_name, host_key};

/// Shown when a DnD payload or a result no longer matches the host/folder on screen.
pub const DROP_STALE_MSG: &str = "The remote folder changed while dragging — drag again.";
/// Debounce of the edit watcher: one save = one upload.
pub const EDIT_DEBOUNCE_MS: u64 = 700;
/// Pause before a failed save is uploaded again.
pub const EDIT_RETRY_MS: u64 = 5_000;

/// Identity of one command: the target generation and the folder it was started in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ticket {
    pub generation: u64,
    pub cwd: Vec<u8>,
}

/// What [`PanelState::finish_command`] tells the caller.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Finish {
    /// The result belongs to the current target: apply it. `pending_follow` is a directory the
    /// session reported meanwhile; navigate there afterwards via [`PanelState::follow_after`].
    Accepted { pending_follow: Option<Vec<u8>> },
    /// The result belongs to an earlier target: drop it. `relist` = list the current host now.
    Stale { relist: bool },
}

/// What to do with an "active session changed directory" notification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Follow {
    Ignore,
    /// A command is running; it is remembered and offered again by `finish_command`.
    Deferred,
    Navigate(Vec<u8>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortColumn {
    Name,
    Size,
    Modified,
    Mode,
    Owner,
}

/// A row of the listing in display order; `Entry` indexes [`PanelState::entries`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Row {
    Up,
    Entry(usize),
}

#[derive(Debug, Clone, PartialEq)]
struct Target {
    key: String,
    browse: Value,
}

/// The host a dialog (sync, compare) was opened for; independent of later panel switches.
#[derive(Debug, Clone, PartialEq)]
pub struct HostBinding {
    pub host_key: String,
    pub conn: Value,
}

#[derive(Debug, Clone)]
pub struct PanelState {
    target: Option<Target>,
    conn: Option<Value>,
    generation: u64,
    cwd: Vec<u8>,
    entries: Vec<RemoteEntry>,
    stale: bool,
    busy: bool,
    start_dir: Vec<u8>,
    follow_pending: Option<Vec<u8>>,
    pub show_hidden: bool,
    pub sort_column: SortColumn,
    pub ascending: bool,
}

impl Default for PanelState {
    fn default() -> Self {
        Self {
            target: None,
            conn: None,
            generation: 0,
            cwd: Vec::new(),
            entries: Vec::new(),
            stale: false,
            busy: false,
            start_dir: Vec::new(),
            follow_pending: None,
            show_hidden: true,
            sort_column: SortColumn::Name,
            ascending: true,
        }
    }
}

impl PanelState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }
    pub fn cwd(&self) -> &[u8] {
        &self.cwd
    }
    pub fn entries(&self) -> &[RemoteEntry] {
        &self.entries
    }
    pub fn conn(&self) -> Option<&Value> {
        self.conn.as_ref()
    }
    pub fn host_key(&self) -> Option<&str> {
        self.target.as_ref().map(|t| t.key.as_str())
    }
    pub fn is_busy(&self) -> bool {
        self.busy
    }
    /// The target changed and has not been listed yet.
    pub fn is_stale(&self) -> bool {
        self.stale
    }

    /// Bind the panel to the active tab's connection (`None` = no SSH tab). `last_folder` is the
    /// remembered folder of the new host. Returns `true` when the panel was reset (the caller
    /// should clear the view and, if visible, list); `false` = same host, directory kept.
    pub fn set_target(&mut self, conn: Option<&Value>, last_folder: &[u8]) -> bool {
        let Some(conn) = conn else {
            if self.target.is_some() {
                self.generation += 1;
            }
            self.target = None;
            self.conn = None;
            self.cwd.clear();
            self.entries.clear();
            self.stale = false;
            self.follow_pending = None;
            return true;
        };
        let browse = browse_conn(conn);
        let same = self.target.as_ref().is_some_and(|t| t.browse == browse);
        self.conn = Some(conn.clone());
        if same && !self.cwd.is_empty() {
            return false;
        }
        if !same {
            self.generation += 1;
        }
        self.target = Some(Target {
            key: host_key(conn),
            browse,
        });
        self.start_dir = last_folder.to_vec();
        self.cwd.clear();
        self.entries.clear();
        self.stale = true;
        true
    }

    /// Snapshot of the current host for a dialog; `None` without a target.
    pub fn binding(&self) -> Option<HostBinding> {
        let t = self.target.as_ref()?;
        Some(HostBinding {
            host_key: t.key.clone(),
            conn: self.conn.clone()?,
        })
    }

    /// Start a command: `None` when there is no target or one is already running.
    pub fn begin_command(&mut self) -> Option<Ticket> {
        if self.target.is_none() || self.busy {
            return None;
        }
        self.busy = true;
        Some(self.ticket())
    }

    /// A ticket for work that does not occupy the panel (an upload job, a drag).
    pub fn ticket(&self) -> Ticket {
        Ticket {
            generation: self.generation,
            cwd: self.cwd.clone(),
        }
    }

    /// Result guard for everything but a listing: same target and same folder.
    pub fn accept(&self, t: &Ticket) -> bool {
        self.target.is_some() && t.generation == self.generation && t.cwd == self.cwd
    }

    /// Result guard for a listing (it moves the folder, so only the generation counts).
    pub fn accept_listing(&self, t: &Ticket) -> bool {
        self.target.is_some() && t.generation == self.generation
    }

    /// A command finished: free the panel and decide whether its result may be used.
    pub fn finish_command(&mut self, t: &Ticket) -> Finish {
        self.busy = false;
        let pending = self.follow_pending.take();
        if t.generation != self.generation {
            return Finish::Stale {
                relist: self.stale && self.target.is_some(),
            };
        }
        Finish::Accepted {
            pending_follow: pending,
        }
    }

    /// After applying a result: where to go for a deferred follow, if anywhere.
    pub fn follow_after(&self, pending: Vec<u8>) -> Option<Vec<u8>> {
        (pending != self.cwd && !self.busy).then_some(pending)
    }

    /// The path `refresh` lists: the current folder, else the remembered one, else home.
    pub fn refresh_path(&self) -> Vec<u8> {
        if !self.cwd.is_empty() {
            self.cwd.clone()
        } else if !self.start_dir.is_empty() {
            self.start_dir.clone()
        } else {
            b".".to_vec()
        }
    }

    /// Mark the listing as requested (clears the stale flag).
    pub fn listing_started(&mut self) {
        self.stale = false;
    }

    /// Apply a successful listing. Returns `false` (nothing changed) for a stale ticket.
    pub fn apply_listing(&mut self, t: &Ticket, cwd: Vec<u8>, entries: Vec<RemoteEntry>) -> bool {
        if !self.accept_listing(t) {
            return false;
        }
        self.cwd = cwd;
        self.entries = entries;
        true
    }

    /// A listing of `path` failed. If it was the remembered start folder (nothing listed yet) the
    /// folder is forgotten and `Some(".")` says to list home instead.
    pub fn listing_failed(&mut self, t: &Ticket, path: &[u8]) -> Option<Vec<u8>> {
        if !self.accept_listing(t) {
            return None;
        }
        if path == self.start_dir.as_slice() && self.cwd.is_empty() {
            self.start_dir.clear();
            return Some(b".".to_vec());
        }
        None
    }

    /// The active session reported a new directory (OSC 7).
    pub fn follow_directory(&mut self, path: &[u8], follow_enabled: bool) -> Follow {
        if !follow_enabled || self.target.is_none() || path.is_empty() || path == self.cwd {
            return Follow::Ignore;
        }
        if self.busy {
            self.follow_pending = Some(path.to_vec());
            return Follow::Deferred;
        }
        Follow::Navigate(path.to_vec())
    }

    /// Sort by `column`; clicking the current column flips the direction.
    pub fn sort_by(&mut self, column: SortColumn) {
        if self.sort_column == column {
            self.ascending = !self.ascending;
        } else {
            self.sort_column = column;
            self.ascending = true;
        }
    }

    /// Rows in display order: ".." (when not at `/`) first whichever way the column sorts,
    /// folders before files in either direction, hidden files filtered by `show_hidden`.
    pub fn rows(&self) -> Vec<Row> {
        let mut idx: Vec<usize> = (0..self.entries.len())
            .filter(|&i| self.show_hidden || !self.entries[i].name.starts_with(b"."))
            .collect();
        idx.sort_by(|&a, &b| {
            let (a, b) = (&self.entries[a], &self.entries[b]);
            // Dirs first regardless of direction.
            let dirs = b.is_dir.cmp(&a.is_dir);
            if dirs != Ordering::Equal {
                return dirs;
            }
            let ord = self.compare(a, b);
            if self.ascending { ord } else { ord.reverse() }
        });
        let mut rows = Vec::with_capacity(idx.len() + 1);
        if !self.cwd.is_empty() && self.cwd != b"/" {
            rows.push(Row::Up);
        }
        rows.extend(idx.into_iter().map(Row::Entry));
        rows
    }

    fn compare(&self, a: &RemoteEntry, b: &RemoteEntry) -> Ordering {
        let fold = |e: &RemoteEntry| display_name(&e.name).to_lowercase();
        let by_name = || fold(a).cmp(&fold(b)).then_with(|| a.name.cmp(&b.name));
        match self.sort_column {
            SortColumn::Name => by_name(),
            SortColumn::Size => a.size.cmp(&b.size).then_with(by_name),
            SortColumn::Modified => a.mtime.cmp(&b.mtime).then_with(by_name),
            SortColumn::Mode => a.mode.cmp(&b.mode).then_with(by_name),
            SortColumn::Owner => a.owner.cmp(&b.owner).then_with(by_name),
        }
    }

    // ---- DnD ----

    /// JSON for rows dragged out of the list (`.` and `..` are never carried).
    pub fn encode_drag(&self, entries: &[&RemoteEntry]) -> Vec<u8> {
        let list: Vec<J> = entries
            .iter()
            .filter(|e| e.name != b"." && e.name != b"..")
            .map(|e| json!({"name": hex(&e.name), "is_dir": e.is_dir, "is_link": e.is_link}))
            .collect();
        json!({"gen": self.generation, "cwd": hex(&self.cwd), "entries": list})
            .to_string()
            .into_bytes()
    }

    /// Entries of a dropped payload that still exist in the folder on screen. Refuses (with
    /// [`DROP_STALE_MSG`]) a payload of another generation or folder, or when there is no target.
    pub fn resolve_drop(&self, payload: &DragPayload) -> Result<Vec<RemoteEntry>, &'static str> {
        if self.target.is_none() || payload.generation != self.generation || payload.cwd != self.cwd
        {
            return Err(DROP_STALE_MSG);
        }
        Ok(self
            .entries
            .iter()
            .filter(|e| payload.entries.iter().any(|p| p.name == e.name))
            .cloned()
            .collect())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DragEntry {
    pub name: Vec<u8>,
    pub is_dir: bool,
    pub is_link: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DragPayload {
    pub generation: u64,
    pub cwd: Vec<u8>,
    pub entries: Vec<DragEntry>,
}

/// Parse a DnD payload; anything malformed is `None`.
pub fn parse_drag(bytes: &[u8]) -> Option<DragPayload> {
    let v: J = serde_json::from_slice(bytes).ok()?;
    let entries = v
        .get("entries")?
        .as_array()?
        .iter()
        .filter_map(|e| {
            Some(DragEntry {
                name: unhex(e.get("name")?.as_str()?)?,
                is_dir: e.get("is_dir").and_then(J::as_bool).unwrap_or(false),
                is_link: e.get("is_link").and_then(J::as_bool).unwrap_or(false),
            })
        })
        .collect();
    Some(DragPayload {
        generation: v.get("gen")?.as_u64()?,
        cwd: unhex(v.get("cwd")?.as_str()?)?,
        entries,
    })
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn unhex(s: &str) -> Option<Vec<u8>> {
    if !s.is_ascii() || !s.len().is_multiple_of(2) {
        return None;
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).ok())
        .collect()
}

/// Local paths of files dropped from a file manager: only `file://` URLs of existing paths;
/// a single remote/web URL refuses the whole drop (empty result).
pub fn local_drop_paths(urls: &[&str], exists: impl Fn(&Path) -> bool) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for url in urls {
        let Some(rest) = url.strip_prefix("file://") else {
            return Vec::new();
        };
        // `file://host/path` is not local; `file:///path` and `file://localhost/path` are.
        let path = match rest.find('/') {
            Some(0) => rest,
            Some(i) if &rest[..i] == "localhost" => &rest[i..],
            _ => return Vec::new(),
        };
        let decoded: Vec<u8> = percent_decode_str(path).collect();
        let decoded = String::from_utf8_lossy(&decoded);
        let trimmed = decoded.trim_end_matches('/');
        // Windows: `file:///C:/dir` is `C:/dir`, not `/C:/dir`.
        let b = trimmed.as_bytes();
        let trimmed = if cfg!(windows) && b.len() >= 3 && b[0] == b'/' && b[2] == b':' {
            &trimmed[1..]
        } else {
            trimmed
        };
        let p = Path::new(trimmed);
        if !trimmed.is_empty() && exists(p) {
            out.push(p.to_path_buf());
        }
    }
    out
}

// ---- bookmarks + last folder (cfg["files_bookmarks"]) ----

fn host_state<'a>(store: &'a Value, key: &str) -> Option<&'a Value> {
    store.get(key).filter(|v| matches!(v, Value::Map(_)))
}

/// Set `name` of the host's state mapping to `v`, creating the store/host mappings as needed
/// (a non-mapping store or host state is replaced).
fn set_host_field(store: &mut Value, key: &str, name: &str, v: Value) {
    fn slot<'a>(map: &'a mut Value, key: &str) -> &'a mut Value {
        if !matches!(map, Value::Map(_)) {
            *map = Value::Map(Vec::new());
        }
        let Value::Map(pairs) = map else {
            return map;
        };
        let pos = pairs
            .iter()
            .position(|(k, _)| matches!(k, Value::Str(s) if s == key))
            .unwrap_or_else(|| {
                pairs.push((Value::Str(key.to_owned()), Value::Null));
                pairs.len() - 1
            });
        &mut pairs[pos].1
    }
    let host = slot(store, key);
    *slot(host, name) = v;
}

/// Bookmarked folders of a host (non-string items are skipped).
pub fn bookmarks(store: &Value, key: &str) -> Vec<Vec<u8>> {
    match host_state(store, key).and_then(|s| s.get("bookmarks")) {
        Some(Value::Seq(items)) => items
            .iter()
            .filter_map(|v| match v {
                Value::Str(s) => Some(s.clone().into_bytes()),
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    }
}

/// Replace the bookmarks; the caller persists the config. Non-UTF-8 paths are dropped.
pub fn set_bookmarks(store: &mut Value, key: &str, marks: &[Vec<u8>]) {
    let items = marks
        .iter()
        .filter_map(|m| String::from_utf8(m.clone()).ok().map(Value::Str))
        .collect();
    set_host_field(store, key, "bookmarks", Value::Seq(items));
}

/// "Bookmark this folder": no-op for an empty path or one already bookmarked. Returns whether
/// anything changed (and so whether to persist).
pub fn add_bookmark(store: &mut Value, key: &str, cwd: &[u8]) -> bool {
    let mut marks = bookmarks(store, key);
    if cwd.is_empty() || std::str::from_utf8(cwd).is_err() || marks.iter().any(|m| m == cwd) {
        return false;
    }
    marks.push(cwd.to_vec());
    set_bookmarks(store, key, &marks);
    true
}

/// "Remove bookmark" for `cwd`; returns whether anything changed.
pub fn remove_bookmark(store: &mut Value, key: &str, cwd: &[u8]) -> bool {
    let marks = bookmarks(store, key);
    if !marks.iter().any(|m| m == cwd) {
        return false;
    }
    let kept: Vec<Vec<u8>> = marks.into_iter().filter(|m| m != cwd).collect();
    set_bookmarks(store, key, &kept);
    true
}

/// The folder a host was last listed in (empty = none).
pub fn last_folder(store: &Value, key: &str) -> Vec<u8> {
    match host_state(store, key).and_then(|s| s.get("last")) {
        Some(Value::Str(s)) => s.clone().into_bytes(),
        _ => Vec::new(),
    }
}

/// Remember the listed folder (saved with the next config write, not persisted here).
pub fn remember_last(store: &mut Value, key: &str, cwd: &[u8]) {
    if let Ok(s) = String::from_utf8(cwd.to_vec()) {
        set_host_field(store, key, "last", Value::Str(s));
    }
}

// ---- edit-in-place ----

/// `(size, sha1 hex)` of a working copy.
pub type Fingerprint = (u64, String);

/// Real fingerprint function (inject a fake in tests); `None` if the file cannot be read.
pub fn file_fingerprint(path: &Path) -> Option<Fingerprint> {
    let data = std::fs::read(path).ok()?;
    let digest: String = Sha1::digest(&data)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    Some((data.len() as u64, digest))
}

/// What to upload after a save.
#[derive(Debug, Clone, PartialEq)]
pub struct EditUpload {
    pub local: PathBuf,
    pub conn: Value,
    pub remote: Vec<u8>,
}

/// One working copy opened for editing: bound to the host it was downloaded from, uploads only
/// when the (size, sha1) fingerprint differs from the last one seen, 700 ms after the last
/// change event.
#[derive(Debug, Clone)]
pub struct EditSession {
    local: PathBuf,
    conn: Value,
    remote: Vec<u8>,
    last: Option<Fingerprint>,
    due: Option<u64>,
}

impl EditSession {
    /// `fingerprint` is the one taken right after the download.
    pub fn new(
        local: PathBuf,
        conn: Value,
        remote: Vec<u8>,
        fingerprint: Option<Fingerprint>,
    ) -> Self {
        Self {
            local,
            conn,
            remote,
            last: fingerprint,
            due: None,
        }
    }

    pub fn local(&self) -> &Path {
        &self.local
    }

    /// A file-change event at `now_ms`: (re)start the debounce.
    pub fn changed(&mut self, now_ms: u64) {
        self.due = Some(now_ms + EDIT_DEBOUNCE_MS);
    }

    /// The upload of the last save failed: forget its fingerprint and try again after a pause.
    /// shortcut: retries forever every `EDIT_RETRY_MS` while the file stays; stops when the session ends.
    pub fn upload_failed(&mut self, now_ms: u64) {
        self.last = None;
        self.due = Some(now_ms + EDIT_RETRY_MS);
    }

    /// When the timer should fire, if armed.
    pub fn next_due(&self) -> Option<u64> {
        self.due
    }

    /// Timer tick: if due, fingerprint the file and return an upload when it really changed.
    pub fn poll(
        &mut self,
        now_ms: u64,
        fingerprint: impl Fn(&Path) -> Option<Fingerprint>,
    ) -> Option<EditUpload> {
        if self.due.is_none_or(|d| now_ms < d) {
            return None;
        }
        self.due = None;
        let fp = fingerprint(&self.local)?;
        if self.last.as_ref() == Some(&fp) {
            return None; // touched but unchanged
        }
        self.last = Some(fp);
        Some(EditUpload {
            local: self.local.clone(),
            conn: self.conn.clone(),
            remote: self.remote.clone(),
        })
    }
}
