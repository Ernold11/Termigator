//! scp progress meter parser and transfer queue policy (the pty spawn and cancelling are in
//! `proc::transfer`).
//!
//! scp draws its meter only when stdout is a terminal and it is that terminal's foreground
//! process, so each job runs under a pty. The meter looks like
//! `big.bin    45%  171MB  12.3MB/s   00:05 ETA` and is redrawn with `\r`:
//! [`split_updates`] cuts raw tty output into lines (keeping the unfinished tail) and
//! [`parse_progress`] turns a line into [`Progress`].
//!
//! [`TransferQueue`] is the policy only: FIFO, at most `parallel` jobs running. It never spawns
//! anything; the caller asks [`TransferQueue::startable`], starts those jobs and sets their
//! state.
//!
//! Golden fixtures: `compat/golden/transfers_*.json`, tests in
//! `tests/remote_transfers.rs`.
//!
//! Notes:
//! - Job ids come from the queue (1, 2, ...), not a process-wide counter, and queue methods
//!   return ids instead of job objects (the queue owns the jobs).
//! - `TransferJob` has no free-form `meta` dict and `tag` is a `u64`: the caller keeps whatever
//!   else it needs (connection, recursion) in its own map keyed by job id.
//! - The meter percentage accepts ASCII digits only (not other Unicode
//!   digits).
//! - `local` is a `PathBuf`, `remote` and `label` are bytes (ADR-12), `argv` elements are bytes.

use std::fmt;
use std::path::PathBuf;
use std::sync::LazyLock;

use regex::Regex;

use crate::config::model::strip_space;
use crate::remote::listing::Argv;
use crate::vault::Secret;

pub const DEFAULT_PARALLEL: usize = 2;
/// Wide enough that scp never truncates the meter (it sizes the line to the tty).
pub const PTY_COLUMNS: u16 = 200;

/// Whitespace / non-whitespace classes: Unicode whitespace plus `\x1c`-`\x1f`.
const SP: &str = r"[\s\x1c-\x1f]";
const NSP: &str = r"[^\s\x1c-\x1f]";

static METER_RE: LazyLock<Option<Regex>> = LazyLock::new(|| {
    Regex::new(&format!(
        "([0-9]{{1,3}})%{SP}+({NSP}+){SP}+({NSP}+/s){SP}+({NSP}+)(?:{SP}+ETA)?{SP}*$"
    ))
    .ok()
});

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobState {
    Queued,
    Running,
    Done,
    Failed,
    Cancelled,
}

impl JobState {
    pub fn finished(self) -> bool {
        matches!(self, Self::Done | Self::Failed | Self::Cancelled)
    }

    /// The state's name.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Running => "running",
            Self::Done => "done",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        [
            Self::Queued,
            Self::Running,
            Self::Done,
            Self::Failed,
            Self::Cancelled,
        ]
        .into_iter()
        .find(|st| st.as_str() == s)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Progress {
    pub percent: u8,
    pub size: String,
    pub rate: String,
    pub eta: String,
}

/// One upload or download. `argv`/`env` are ready to spawn; `env` may carry a decrypted password
/// for sshpass (in memory only, never shown by `Debug`).
#[derive(Clone)]
pub struct TransferJob {
    /// Assigned by [`TransferQueue::add`].
    pub id: u64,
    pub upload: bool,
    pub local: PathBuf,
    pub remote: Vec<u8>,
    pub label: Vec<u8>,
    pub argv: Argv,
    pub env: Vec<(String, Secret)>,
    pub state: JobState,
    pub progress: Option<Progress>,
    pub error: String,
    /// Opaque tag set by the caller (e.g. the panel's target generation), so a finished upload
    /// only refreshes the listing it belongs to.
    pub tag: u64,
}

impl TransferJob {
    pub fn new(
        upload: bool,
        local: PathBuf,
        remote: Vec<u8>,
        label: Vec<u8>,
        argv: Argv,
        env: Vec<(String, Secret)>,
    ) -> Self {
        Self {
            id: 0,
            upload,
            local,
            remote,
            label,
            argv,
            env,
            state: JobState::Queued,
            progress: None,
            error: String::new(),
            tag: 0,
        }
    }

    /// The arrow shown next to the job.
    pub fn direction(&self) -> &'static str {
        if self.upload { "↑" } else { "↓" }
    }
}

impl fmt::Debug for TransferJob {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TransferJob")
            .field("id", &self.id)
            .field("upload", &self.upload)
            .field("local", &self.local)
            .field("remote", &String::from_utf8_lossy(&self.remote))
            .field("state", &self.state)
            .field("progress", &self.progress)
            .field("error", &self.error)
            .field("env", &format_args!("<{} vars redacted>", self.env.len()))
            .finish_non_exhaustive()
    }
}

/// Strips trailing Unicode whitespace.
fn rstrip_space(s: &str) -> &str {
    s.trim_end_matches(|c: char| c.is_whitespace() || ('\x1c'..='\x1f').contains(&c))
}

/// The scp meter line gives a [`Progress`]; anything else (errors, banners) gives `None`.
pub fn parse_progress(line: &str) -> Option<Progress> {
    let caps = METER_RE.as_ref()?.captures(rstrip_space(line))?;
    let percent = caps[1].parse::<u8>().unwrap_or(100).min(100);
    let mut rate = caps[3].to_owned();
    if rate.starts_with('-') {
        // scp prints garbage rates for near-instant transfers
        rate.clear();
    }
    Some(Progress {
        percent,
        size: caps[2].to_owned(),
        rate,
        eta: caps[4].to_owned(),
    })
}

/// Feed raw tty output: the complete lines (`\r` or `\n` ended, blank ones dropped) and the
/// unfinished tail to keep for the next chunk.
pub fn split_updates(pending: &str, chunk: &str) -> (Vec<String>, String) {
    let data = format!("{pending}{chunk}");
    let mut parts: Vec<&str> = data.split(['\r', '\n']).collect();
    let tail = parts.pop().unwrap_or_default().to_owned();
    let lines = parts
        .into_iter()
        .filter(|p| !strip_space(p).is_empty())
        .map(str::to_owned)
        .collect();
    (lines, tail)
}

/// Job list and the "what to start next" policy (FIFO, at most `parallel` running).
#[derive(Debug)]
pub struct TransferQueue {
    parallel: usize,
    jobs: Vec<TransferJob>,
    next_id: u64,
}

impl Default for TransferQueue {
    fn default() -> Self {
        Self::new(DEFAULT_PARALLEL)
    }
}

impl TransferQueue {
    /// `parallel` below 1 is raised to 1.
    pub fn new(parallel: usize) -> Self {
        Self {
            parallel: parallel.max(1),
            jobs: Vec::new(),
            next_id: 1,
        }
    }

    pub fn parallel(&self) -> usize {
        self.parallel
    }

    pub fn jobs(&self) -> &[TransferJob] {
        &self.jobs
    }

    /// Appends the job and returns its new id.
    pub fn add(&mut self, mut job: TransferJob) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        job.id = id;
        self.jobs.push(job);
        id
    }

    pub fn get(&self, id: u64) -> Option<&TransferJob> {
        self.jobs.iter().find(|j| j.id == id)
    }

    pub fn get_mut(&mut self, id: u64) -> Option<&mut TransferJob> {
        self.jobs.iter_mut().find(|j| j.id == id)
    }

    /// Ids of the running jobs.
    pub fn running(&self) -> Vec<u64> {
        self.ids_in(JobState::Running, usize::MAX)
    }

    /// Ids of the queued jobs to start now: FIFO, as many as there are free slots.
    pub fn startable(&self) -> Vec<u64> {
        let free = self.parallel.saturating_sub(self.running().len());
        self.ids_in(JobState::Queued, free)
    }

    fn ids_in(&self, state: JobState, limit: usize) -> Vec<u64> {
        self.jobs
            .iter()
            .filter(|j| j.state == state)
            .take(limit)
            .map(|j| j.id)
            .collect()
    }

    /// "Remove from list" a finished job leaves the queue;
    /// a running or queued one stays (`None`).
    pub fn remove_finished(&mut self, id: u64) -> Option<TransferJob> {
        let i = self.jobs.iter().position(|j| j.id == id)?;
        self.jobs[i].state.finished().then(|| self.jobs.remove(i))
    }

    /// Removes and returns the finished jobs.
    pub fn clear_finished(&mut self) -> Vec<TransferJob> {
        let (gone, kept) = std::mem::take(&mut self.jobs)
            .into_iter()
            .partition(|j| j.state.finished());
        self.jobs = kept;
        gone
    }

    /// Any job not finished yet.
    pub fn active(&self) -> bool {
        self.jobs.iter().any(|j| !j.state.finished())
    }
}
