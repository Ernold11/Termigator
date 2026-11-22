//! Transfer queue of a Files panel: `TransfersCore`, exposed to QML as
//! `Termigator.TransfersCore`; one per panel (the side panel and every Files tab keep their own
//! queue).
//!
//! The model of a job, the FIFO policy and the scp meter parser are core's
//! (`termigator_core::remote::transfers`); the scp / sftp processes are core's too
//! (`termigator_core::proc::transfer`). This object owns the queue and the running
//! [`TransferHandle`]s and tells QML what changed: `jobAdded` / `jobUpdated` (a JSON object, see
//! [`job_json`]), `jobRemoved(id)` and `jobFinished(json)` for every job reaching a final state.
//! QML calls `poll()` from a timer while `busy` (the handles' channels are drained
//! there), so no thread has to reach into this object.
//!
//! Other objects add jobs with [`queue_scp`]: the queue is looked up by the name the QML side
//! registered (`registerQueue`), so the Files object and the sync dialog need no reference to it.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::mpsc::TryRecvError;

use cxx_qt::{CxxQtThread, CxxQtType, Threading};
use cxx_qt_lib::QString;
use serde_json::json;
use termigator_core::config::doc::Value;
use termigator_core::config::model::section;
use termigator_core::proc::transfer::{self, Event, TransferHandle};
use termigator_core::proto::scp::{ScpTransfer, SftpResume, scp_argv, sftp_resume_argv};
use termigator_core::proto::ssh::ssh_password_env;
use termigator_core::remote::listing::display_name;
use termigator_core::remote::transfers::{
    DEFAULT_PARALLEL, JobState, Progress, TransferJob, TransferQueue,
};
use termigator_core::vault::{self, Revealed};

use crate::shell_qobj::with_app;

#[cxx_qt::bridge]
pub mod qobject {
    unsafe extern "C++" {
        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;
    }

    #[auto_cxx_name]
    extern "RustQt" {
        #[qobject]
        #[qml_element]
        #[qproperty(i32, active_count)]
        #[qproperty(i32, job_count)]
        #[qproperty(bool, busy)]
        type TransfersCore = super::TransfersCoreRust;

        /// A job entered the queue (`json`: [`super::job_json`]).
        #[qsignal]
        fn job_added(self: Pin<&mut TransfersCore>, json: QString);

        /// A job's state or progress changed.
        #[qsignal]
        fn job_updated(self: Pin<&mut TransfersCore>, json: QString);

        /// A job left the list (Remove from list, Clear finished, resume / retry replacing it).
        #[qsignal]
        fn job_removed(self: Pin<&mut TransfersCore>, id: i32);

        /// A job reached done / failed / cancelled (the panel refreshes, opens the editor, …).
        #[qsignal]
        fn job_finished(self: Pin<&mut TransfersCore>, json: QString);

        /// Makes this queue findable by `name` for [`super::queue_scp`]; call once.
        #[qinvokable]
        fn register_queue(self: Pin<&mut TransfersCore>, name: &QString);

        /// Drains the running jobs' events (a timer calls it while `busy`).
        #[qinvokable]
        fn poll(self: Pin<&mut TransfersCore>);

        /// Cancel: the jobs in `ids_json` (a JSON array), all active ones when it is empty.
        #[qinvokable]
        fn cancel_ids(self: Pin<&mut TransfersCore>, ids_json: &QString);

        /// Context menu "Resume" of an interrupted download (sftp `reget`).
        #[qinvokable]
        fn resume(self: Pin<&mut TransfersCore>, id: i32);

        /// Context menu "Retry" of a failed or cancelled upload.
        #[qinvokable]
        fn retry(self: Pin<&mut TransfersCore>, id: i32);

        /// Context menu "Remove from list" (finished jobs only).
        #[qinvokable]
        fn remove(self: Pin<&mut TransfersCore>, id: i32);

        #[qinvokable]
        fn clear_finished(self: Pin<&mut TransfersCore>);

        /// The close question of a Files tab with transfers running, else "".
        #[qinvokable]
        fn question(self: &TransfersCore) -> QString;

        /// "Transfers" or "Transfers — N active".
        #[qinvokable]
        fn title(self: &TransfersCore) -> QString;

        /// Every job as a JSON array (for the view when it is created again, and tests).
        #[qinvokable]
        fn jobs_json(self: &TransfersCore) -> QString;

        /// Window or tab closing: cancels everything that is not finished.
        #[qinvokable]
        fn shutdown(self: Pin<&mut TransfersCore>);
    }

    impl cxx_qt::Threading for TransfersCore {}
}

/// What the queue remembers of a job besides what core's [`TransferJob`] holds.
#[derive(Clone)]
pub struct Meta {
    /// The stored entry the job runs against (credential applied, password as stored): resume
    /// rebuilds the command from it.
    pub conn: Value,
    pub recursive: bool,
    /// Who wants to hear about the end: "" nobody, `edit:<n>` the Files object's working copy.
    pub purpose: String,
    /// Resume the download with sftp `reget` instead of running `argv`.
    pub resume: bool,
}

thread_local! {
    static QUEUES: RefCell<HashMap<String, CxxQtThread<qobject::TransfersCore>>> =
        RefCell::new(HashMap::new());
}

/// Hands a job to the queue registered as `queue`; false when there is none (any more).
fn enqueue(queue: &str, job: TransferJob, meta: Meta) -> bool {
    let handle = QUEUES.with(|q| q.borrow().get(queue).cloned());
    handle.is_some_and(|h| h.queue(move |c| c.add_job(job, meta)).is_ok())
}

/// Why a job could not be queued; `Status` goes to the panel's status line, `Warn` to a box.
#[derive(Debug, PartialEq, Eq)]
pub enum QueueError {
    Status(String),
    Warn(String),
}

/// One scp job for [`queue_scp`].
pub struct ScpRequest<'a> {
    pub queue: &'a str,
    /// The panel's label, for the status messages.
    pub host_label: &'a str,
    /// The entry the job runs against: credential applied, password as stored.
    pub conn: &'a Value,
    pub upload: bool,
    pub local: PathBuf,
    pub remote: Vec<u8>,
    pub label: Vec<u8>,
    pub recursive: bool,
    pub preserve: bool,
    pub purpose: String,
}

/// The entry with its password decrypted (the vault's session key), `None` while it is locked.
pub fn reveal_conn(conn: &Value) -> Option<Revealed<'_>> {
    let key = vault::session_key();
    vault::reveal_connection(conn, key.as_ref()).ok()
}

/// Builds the scp command now (the credentials are resolved now, so the job runs fine even when
/// the panel moves on) and queues the job. Returns the tag-less outcome.
pub fn queue_scp(r: ScpRequest<'_>) -> Result<(), QueueError> {
    let locked = || {
        QueueError::Status(format!(
            "{}: saved password is locked — enter the master password.",
            r.host_label
        ))
    };
    let revealed = reveal_conn(r.conn).ok_or_else(locked)?;
    let launch = with_app(|app| section(&app.cfg, "launch").clone());
    let (Some(local), Ok(remote)) = (r.local.to_str(), std::str::from_utf8(&r.remote)) else {
        return Err(QueueError::Warn(NON_UTF8_MSG.to_owned()));
    };
    let argv = scp_argv(
        r.conn,
        &launch,
        &ScpTransfer {
            local_path: local,
            remote_path: remote,
            upload: r.upload,
            recursive: r.recursive,
            preserve: r.preserve,
        },
    )
    .map_err(|e| {
        QueueError::Warn(if e.is_empty() {
            "Cannot build scp command.".to_owned()
        } else {
            e
        })
    })?;
    let env = ssh_password_env(&revealed)
        .map_err(|e| QueueError::Status(format!("{}: {e}", r.host_label)))?;
    let job = TransferJob::new(
        r.upload,
        r.local.clone(),
        r.remote.clone(),
        r.label,
        argv.into_iter().map(String::into_bytes).collect(),
        env,
    );
    let meta = Meta {
        conn: r.conn.clone(),
        recursive: r.recursive,
        purpose: r.purpose,
        resume: false,
    };
    if enqueue(r.queue, job, meta) {
        Ok(())
    } else {
        Err(QueueError::Status(
            "The transfer queue is not available.".to_owned(),
        ))
    }
}

/// Core's scp / sftp builders take text; a name that is not UTF-8 cannot be handed to them yet.
const NON_UTF8_MSG: &str = "This path is not valid UTF-8 and cannot be transferred.";

pub struct TransfersCoreRust {
    active_count: i32,
    job_count: i32,
    busy: bool,
    queue: TransferQueue,
    meta: HashMap<u64, Meta>,
    handles: HashMap<u64, TransferHandle>,
}

impl Default for TransfersCoreRust {
    fn default() -> Self {
        Self {
            active_count: 0,
            job_count: 0,
            busy: false,
            queue: TransferQueue::new(DEFAULT_PARALLEL),
            meta: HashMap::new(),
            handles: HashMap::new(),
        }
    }
}

/// Text of the state column.
pub fn state_text(job: &TransferJob) -> String {
    match job.state {
        JobState::Queued => "queued".to_owned(),
        JobState::Running => match &job.progress {
            None => "starting…".to_owned(),
            Some(p) if !p.eta.is_empty() && p.eta != "--:--" => format!("{} · {}", p.size, p.eta),
            Some(p) => p.size.clone(),
        },
        JobState::Done => "done".to_owned(),
        JobState::Cancelled => "cancelled".to_owned(),
        JobState::Failed => format!("failed: {}", job.error),
    }
}

/// The JSON QML shows and `jobFinished` reports.
pub fn job_json(job: &TransferJob, purpose: &str) -> String {
    let percent = match (&job.progress, job.state) {
        (_, JobState::Done) => 100,
        (Some(Progress { percent, .. }), _) => i32::from(*percent),
        _ => 0,
    };
    let speed = if job.state == JobState::Running {
        job.progress.as_ref().map(|p| p.rate.clone())
    } else {
        None
    };
    let label = display_name(&job.label);
    json!({
        "id": job.id,
        "upload": job.upload,
        "label": format!("{} {}", job.direction(), label),
        "plainLabel": label,
        "tooltip": format!(
            "{}  {}  {}",
            job.local.display(),
            if job.upload { "→" } else { "←" },
            display_name(&job.remote)
        ),
        "local": job.local.display().to_string(),
        "state": job.state.as_str(),
        "percent": percent,
        "speed": speed.unwrap_or_default(),
        "stateText": state_text(job),
        "error": job.error,
        "purpose": purpose,
    })
    .to_string()
}

fn qs(s: &str) -> QString {
    QString::from(s)
}

impl qobject::TransfersCore {
    fn register_queue(self: Pin<&mut Self>, name: &QString) {
        let handle = self.qt_thread();
        let name = name.to_string();
        QUEUES.with(|q| q.borrow_mut().insert(name, handle));
    }

    fn poll(mut self: Pin<&mut Self>) {
        let running: Vec<u64> = self.handles.keys().copied().collect();
        for id in running {
            let mut events = Vec::new();
            let mut gone = false;
            if let Some(h) = self.handles.get(&id) {
                loop {
                    match h.events().try_recv() {
                        Ok(ev) => events.push(ev),
                        Err(TryRecvError::Empty) => break,
                        Err(TryRecvError::Disconnected) => {
                            gone = true;
                            break;
                        }
                    }
                }
            }
            for ev in events {
                self.as_mut().apply_event(id, ev);
            }
            if gone && self.handles.contains_key(&id) {
                // The worker always ends with a final event; a vanished channel is a failure.
                let message = "exit -1".to_owned();
                self.as_mut().apply_event(id, Event::Failed { message });
            }
        }
    }

    fn apply_event(mut self: Pin<&mut Self>, id: u64, ev: Event) {
        let final_state = match ev {
            Event::Progress {
                percent,
                size,
                speed,
                eta,
            } => {
                if let Some(job) = self.as_mut().rust_mut().queue.get_mut(id)
                    && !job.state.finished()
                {
                    job.progress = Some(Progress {
                        percent,
                        size,
                        rate: speed,
                        eta,
                    });
                    self.as_mut().updated(id);
                }
                return;
            }
            Event::Done => JobState::Done,
            Event::Failed { message } => {
                if let Some(job) = self.as_mut().rust_mut().queue.get_mut(id) {
                    job.error = message;
                }
                JobState::Failed
            }
            Event::Cancelled => JobState::Cancelled,
        };
        self.as_mut().rust_mut().handles.remove(&id);
        if let Some(job) = self.as_mut().rust_mut().queue.get_mut(id) {
            // A job cancelled by the user stays cancelled whatever the process did last.
            if job.state != JobState::Cancelled {
                job.state = final_state;
            }
        }
        self.as_mut().updated(id);
        self.as_mut().finished(id);
        self.pump();
    }

    /// Emits `jobUpdated` for `id`.
    fn updated(mut self: Pin<&mut Self>, id: u64) {
        let text = self.json_of(id);
        if let Some(text) = text {
            self.as_mut().job_updated(qs(&text));
        }
    }

    fn finished(mut self: Pin<&mut Self>, id: u64) {
        if let Some(text) = self.json_of(id) {
            self.as_mut().job_finished(qs(&text));
        }
    }

    fn json_of(&self, id: u64) -> Option<String> {
        let job = self.queue.get(id)?;
        let purpose = self.meta.get(&id).map_or("", |m| m.purpose.as_str());
        Some(job_json(job, purpose))
    }

    pub(crate) fn add_job(mut self: Pin<&mut Self>, job: TransferJob, meta: Meta) {
        let id = self.as_mut().rust_mut().queue.add(job);
        self.as_mut().rust_mut().meta.insert(id, meta);
        if let Some(text) = self.json_of(id) {
            self.as_mut().job_added(qs(&text));
        }
        self.pump();
    }

    /// Starts what the policy allows and refreshes the counters.
    fn pump(mut self: Pin<&mut Self>) {
        for id in self.queue.startable() {
            self.as_mut().start(id);
        }
        let (active, total) = {
            let q = &self.queue;
            let active = q.jobs().iter().filter(|j| !j.state.finished()).count();
            (active, q.jobs().len())
        };
        self.as_mut()
            .set_active_count(i32::try_from(active).unwrap_or(i32::MAX));
        self.as_mut()
            .set_job_count(i32::try_from(total).unwrap_or(i32::MAX));
        // A cancelled job still has a process to reap: its end is read by `poll()` too.
        let attached = !self.handles.is_empty();
        self.as_mut().set_busy(attached);
    }

    fn start(mut self: Pin<&mut Self>, id: u64) {
        let mut me = self.as_mut().rust_mut();
        let Some(job) = me.queue.get_mut(id) else {
            return;
        };
        job.state = JobState::Running;
        let job = job.clone();
        let meta = self.meta.get(&id).cloned();
        let handle = match meta {
            Some(m) if m.resume => {
                let launch = with_app(|app| section(&app.cfg, "launch").clone());
                let (local, remote) = (job.local.clone(), job.remote.clone());
                transfer::start_resume(job, move |batch| {
                    let (Some(local), Ok(remote)) = (local.to_str(), std::str::from_utf8(&remote))
                    else {
                        return Err(NON_UTF8_MSG.to_owned());
                    };
                    sftp_resume_argv(
                        &m.conn,
                        &launch,
                        &SftpResume {
                            local_path: local,
                            remote_path: remote,
                            batch_path: batch,
                            recursive: m.recursive,
                        },
                    )
                })
            }
            _ => transfer::start(job),
        };
        self.as_mut().rust_mut().handles.insert(id, handle);
        self.updated(id);
    }

    fn cancel_ids(mut self: Pin<&mut Self>, ids_json: &QString) {
        let ids_json = ids_json.to_string();
        let parsed = serde_json::from_str(&ids_json);
        let ids: Vec<u64> =
            crate::qml_json::logged("cancel_ids", &ids_json, parsed).unwrap_or_default();
        let targets: Vec<u64> = if ids.is_empty() {
            self.queue.jobs().iter().map(|j| j.id).collect()
        } else {
            ids
        };
        for id in targets {
            let mut me = self.as_mut().rust_mut();
            let Some(job) = me.queue.get_mut(id) else {
                continue;
            };
            let was = job.state;
            if matches!(was, JobState::Queued | JobState::Running) {
                job.state = JobState::Cancelled;
            }
            match was {
                JobState::Queued => {
                    self.as_mut().updated(id);
                    self.as_mut().finished(id);
                }
                JobState::Running => {
                    if let Some(h) = self.handles.get(&id) {
                        h.cancel();
                    }
                    self.as_mut().updated(id);
                }
                _ => {}
            }
        }
        self.pump();
    }

    fn resume(mut self: Pin<&mut Self>, id: i32) {
        let Some((old, meta)) = self.interrupted(id).filter(|(j, _)| !j.upload) else {
            return;
        };
        let mut label = old.label.clone();
        label.extend_from_slice(" (resume)".as_bytes());
        let job = TransferJob::new(
            false,
            old.local.clone(),
            old.remote.clone(),
            label,
            Vec::new(),
            old.env.clone(),
        );
        let meta = Meta {
            resume: true,
            ..meta
        };
        self.as_mut().add_job(job, meta);
        self.remove(id);
    }

    fn retry(mut self: Pin<&mut Self>, id: i32) {
        let Some((old, meta)) = self.interrupted(id).filter(|(j, _)| j.upload) else {
            return;
        };
        let job = TransferJob::new(
            true,
            old.local.clone(),
            old.remote.clone(),
            old.label.clone(),
            old.argv.clone(),
            old.env.clone(),
        );
        self.as_mut().add_job(job, meta);
        self.remove(id);
    }

    /// A failed or cancelled job with what it was started with.
    fn interrupted(&self, id: i32) -> Option<(TransferJob, Meta)> {
        let id = u64::try_from(id).ok()?;
        let job = self.queue.get(id)?;
        matches!(job.state, JobState::Failed | JobState::Cancelled)
            .then(|| self.meta.get(&id).map(|m| (job.clone(), m.clone())))
            .flatten()
    }

    fn remove(mut self: Pin<&mut Self>, id: i32) {
        let Ok(id) = u64::try_from(id) else { return };
        if self.as_mut().rust_mut().queue.remove_finished(id).is_some() {
            self.as_mut().rust_mut().meta.remove(&id);
            self.as_mut().job_removed(i32::try_from(id).unwrap_or(0));
            self.pump();
        }
    }

    fn clear_finished(mut self: Pin<&mut Self>) {
        let gone = self.as_mut().rust_mut().queue.clear_finished();
        for job in gone {
            self.as_mut().rust_mut().meta.remove(&job.id);
            self.as_mut()
                .job_removed(i32::try_from(job.id).unwrap_or(0));
        }
        self.pump();
    }

    fn question(&self) -> QString {
        let n = self.active_count;
        if n > 0 {
            qs(&format!(
                "{n} file transfer(s) are still in progress in this tab. Stop them and close it?"
            ))
        } else {
            QString::default()
        }
    }

    fn title(&self) -> QString {
        qs(&title_text(self.active_count))
    }

    fn jobs_json(&self) -> QString {
        let rows: Vec<serde_json::Value> = self
            .queue
            .jobs()
            .iter()
            .filter_map(|j| serde_json::from_str(&self.json_of(j.id)?).ok())
            .collect();
        qs(&serde_json::Value::Array(rows).to_string())
    }

    fn shutdown(mut self: Pin<&mut Self>) {
        let ids: Vec<u64> = self
            .queue
            .jobs()
            .iter()
            .filter(|j| !j.state.finished())
            .map(|j| j.id)
            .collect();
        for id in ids {
            if let Some(job) = self.as_mut().rust_mut().queue.get_mut(id) {
                job.state = JobState::Cancelled;
            }
            if let Some(h) = self.as_mut().rust_mut().handles.remove(&id) {
                h.cancel();
            }
        }
        self.pump();
    }
}

fn title_text(active: i32) -> String {
    if active > 0 {
        format!("Transfers — {active} active")
    } else {
        "Transfers".to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn job(state: JobState) -> TransferJob {
        let mut j = TransferJob::new(
            true,
            PathBuf::from("/tmp/a b"),
            b"/r/a b".to_vec(),
            b"a b".to_vec(),
            Vec::new(),
            Vec::new(),
        );
        j.id = 7;
        j.state = state;
        j
    }

    #[test]
    fn state_texts_follow_golden() {
        let mut j = job(JobState::Queued);
        assert_eq!(state_text(&j), "queued");
        j.state = JobState::Running;
        assert_eq!(state_text(&j), "starting…");
        j.progress = Some(Progress {
            percent: 40,
            size: "171MB".into(),
            rate: "12MB/s".into(),
            eta: "00:05".into(),
        });
        assert_eq!(state_text(&j), "171MB · 00:05");
        if let Some(p) = j.progress.as_mut() {
            p.eta = "--:--".into();
        }
        assert_eq!(state_text(&j), "171MB");
        j.state = JobState::Failed;
        j.error = "Permission denied".into();
        assert_eq!(state_text(&j), "failed: Permission denied");
    }

    #[test]
    fn job_json_has_label_arrow_and_hides_speed_once_finished() {
        let mut j = job(JobState::Running);
        j.progress = Some(Progress {
            percent: 40,
            size: "1MB".into(),
            rate: "2MB/s".into(),
            eta: "00:01".into(),
        });
        let v: serde_json::Value =
            serde_json::from_str(&job_json(&j, "edit:1")).unwrap_or_default();
        assert_eq!(v["label"], "↑ a b");
        assert_eq!(v["tooltip"], "/tmp/a b  →  /r/a b");
        assert_eq!(v["percent"], 40);
        assert_eq!(v["speed"], "2MB/s");
        assert_eq!(v["purpose"], "edit:1");
        j.state = JobState::Done;
        let v: serde_json::Value = serde_json::from_str(&job_json(&j, "")).unwrap_or_default();
        assert_eq!(v["percent"], 100);
        assert_eq!(v["speed"], "");
    }

    #[test]
    fn title_counts_active_jobs() {
        assert_eq!(title_text(0), "Transfers");
        assert_eq!(title_text(3), "Transfers — 3 active");
    }
}
