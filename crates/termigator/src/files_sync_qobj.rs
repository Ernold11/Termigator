//! `FileSyncCore`: the Synchronize dialog (the sync dialog), bound to the host it was
//! opened for.

use std::path::PathBuf;
use std::pin::Pin;
use std::time::Duration;

use cxx_qt::{CxxQtType, Threading};
use cxx_qt_lib::QString;
use serde_json::{Value as J, json};
use termigator_core::config::doc::Value;
use termigator_core::proc::runner::{self, CancelHandle, Command};
use termigator_core::proto::ssh::{Env, expand_user, quote_repr};
use termigator_core::remote::listing::{RemoteOps, display_name, human_size};
use termigator_core::remote::sync::{
    Direction, RunPlan, SyncAction, SyncOp, parse_remote_tree, plan_sync, remote_tree_argv,
    scan_local,
};

use crate::files_qobj::{
    build, command_env, err_text, int_list, launch, qs, run_blocking, stripped,
};
use crate::transfers_qobj::{QueueError, ScpRequest, queue_scp};

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
        #[qproperty(bool, busy)]
        type FileSyncCore = super::FileSyncCoreRust;

        /// A message for the dialog: `level` is error | info | positive | warning.
        #[qsignal]
        fn message(self: Pin<&mut FileSyncCore>, level: QString, text: QString);

        /// The compared plan: `{rows: [...]}`.
        #[qsignal]
        fn plan_ready(self: Pin<&mut FileSyncCore>, json: QString);

        /// The copies were queued (the dialog clears its table).
        #[qsignal]
        fn queued(self: Pin<&mut FileSyncCore>);

        /// `binding`: `FilesCore.bindingJson()`; `queue`: the name
        /// of the panel's transfer queue.
        #[qinvokable]
        fn bind(self: Pin<&mut FileSyncCore>, binding: &QString, queue: &QString);

        /// Compare (`direction`: upload | download | both).
        #[qinvokable]
        fn compare(
            self: Pin<&mut FileSyncCore>,
            local: &QString,
            remote: &QString,
            direction: &QString,
        );

        /// Synchronize the ticked rows (`checked_json`: row numbers of the plan).
        #[qinvokable]
        fn run(
            self: Pin<&mut FileSyncCore>,
            checked_json: &QString,
            local: &QString,
            remote: &QString,
        );

        /// The text of the compare / synchronize inputs changed: the plan is void.
        #[qinvokable]
        fn invalidate(self: Pin<&mut FileSyncCore>);

        #[qinvokable]
        fn shutdown(self: Pin<&mut FileSyncCore>);
    }

    impl cxx_qt::Threading for FileSyncCore {}
}

#[derive(Default)]
pub struct FileSyncCoreRust {
    busy: bool,
    conn: Option<Value>,
    label: String,
    queue: String,
    plan: Vec<SyncAction>,
    cancel: Option<CancelHandle>,
}

/// Display label of each sync operation.
fn op_label(op: SyncOp) -> &'static str {
    match op {
        SyncOp::Upload => "↑ Upload",
        SyncOp::Download => "↓ Download",
        SyncOp::MkdirRemote => "+ Folder on server",
        SyncOp::MkdirLocal => "+ Folder here",
        SyncOp::Conflict => "⚠ Skip",
    }
}

/// The rows of the dialog's table and the message under the form.
pub fn plan_json(plan: &[SyncAction], rejected: &[Vec<u8>]) -> (J, &'static str, String) {
    let rows: Vec<J> = plan
        .iter()
        .map(|a| {
            let copy = matches!(a.op, SyncOp::Upload | SyncOp::Download);
            json!({
                "op": a.op.as_str(),
                "opLabel": op_label(a.op),
                "rel": display_name(&a.rel),
                "size": if copy { human_size(a.size) } else { String::new() },
                "reason": a.reason,
                "checkable": a.op != SyncOp::Conflict,
                // A copy over a newer file starts unticked.
                "checked": a.op != SyncOp::Conflict && !a.replaces_newer(),
                "risky": a.replaces_newer(),
                "replaces": a.replaces_existing(),
            })
        })
        .collect();
    let copies = plan
        .iter()
        .filter(|a| matches!(a.op, SyncOp::Upload | SyncOp::Download))
        .count();
    let conflicts = plan.iter().filter(|a| a.op == SyncOp::Conflict).count();
    let older = plan.iter().filter(|a| a.replaces_newer()).count();
    let (mut level, mut text) = if plan.is_empty() {
        ("positive", "Both folders are already in sync.".to_owned())
    } else {
        (
            if conflicts > 0 || older > 0 {
                "warning"
            } else {
                "info"
            },
            format!(
                "{copies} file(s) to copy{}{} — untick what should not be copied.",
                if conflicts > 0 {
                    format!(", {conflicts} conflict(s) left alone")
                } else {
                    String::new()
                },
                if older > 0 {
                    format!(", {older} would replace a newer file (unticked)")
                } else {
                    String::new()
                }
            ),
        )
    };
    if let Some(first) = rejected.first() {
        // Never silently — the server sent paths that leave the folder.
        level = "error";
        text = format!(
            "The server listed {} path(s) that point outside the folder (e.g. {}) — left out.",
            rejected.len(),
            quote_repr(&display_name(first))
        );
    }
    (json!({"rows": rows}), level, text)
}

fn msg(c: Pin<&mut qobject::FileSyncCore>, level: &str, text: &str) {
    c.message(qs(level), qs(text));
}

impl qobject::FileSyncCore {
    fn bind(mut self: Pin<&mut Self>, binding: &QString, queue: &QString) {
        let j: J = crate::qml_json::value("bind", &binding.to_string()).unwrap_or(J::Null);
        let conn = j.get("conn").map(Value::from_json);
        let label = j.get("label").and_then(J::as_str).unwrap_or_default();
        let mut me = self.as_mut().rust_mut();
        me.conn = conn;
        me.label = label.to_owned();
        me.queue = queue.to_string();
    }

    fn invalidate(mut self: Pin<&mut Self>) {
        self.as_mut().rust_mut().plan.clear();
    }

    fn compare(mut self: Pin<&mut Self>, local: &QString, remote: &QString, direction: &QString) {
        let local_text = stripped(local);
        let local_path = PathBuf::from(expand_user(
            &local_text,
            std::env::var("HOME").ok().as_deref(),
        ));
        let remote = stripped(remote).into_bytes();
        if !local_path.is_dir() {
            msg(
                self,
                "error",
                &format!("{} is not a folder.", local_path.display()),
            );
            return;
        }
        if remote.is_empty() {
            msg(self, "error", "Enter the remote folder.");
            return;
        }
        let Some(conn) = self.conn.clone() else {
            return;
        };
        let argv = match build(&conn, |o| remote_tree_argv(o, &remote)) {
            Ok(a) => a,
            Err(e) => {
                let t = if e.is_empty() {
                    "Cannot list the remote folder.".to_owned()
                } else {
                    e
                };
                msg(self, "error", &t);
                return;
            }
        };
        let env = match command_env(&conn, &self.label) {
            Ok(e) => e,
            Err(_) => {
                msg(
                    self,
                    "error",
                    "The saved password is locked — enter the master password.",
                );
                return;
            }
        };
        msg(self.as_mut(), "info", "Comparing…");
        self.as_mut().set_busy(true);
        let direction = Direction::parse(&direction.to_string()).unwrap_or(Direction::Upload);
        let thread = self.qt_thread();
        let shown_remote = display_name(&remote);
        let mut cmd = Command::new(
            argv.iter()
                .map(|a| termigator_core::platform::os_string_from_vec(a.clone())),
        );
        cmd.env = env;
        cmd.timeout = Some(Duration::from_secs(300));
        let running = match runner::spawn(cmd) {
            Ok(r) => r,
            Err(e) => {
                self.as_mut().set_busy(false);
                msg(
                    self,
                    "error",
                    &format!(
                        "Listing {shown_remote} failed: {}",
                        err_text(e.to_string().as_bytes(), 127)
                    ),
                );
                return;
            }
        };
        self.as_mut().rust_mut().cancel = Some(running.cancel_handle());
        std::thread::spawn(move || {
            let Some(out) = running.wait() else { return };
            if out.cancelled {
                return;
            }
            let (rc, stdout, stderr) = if out.timed_out {
                (124, Vec::new(), b"Timed out after 300s.".to_vec())
            } else {
                (out.code(), out.stdout, out.stderr)
            };
            let result = if rc != 0 {
                Err(format!(
                    "Listing {shown_remote} failed: {}",
                    err_text(&stderr, rc)
                ))
            } else {
                let (tree, rejected) = parse_remote_tree(&stdout);
                let plan = plan_sync(&scan_local(&local_path), &tree, direction);
                Ok((plan, rejected))
            };
            let _ = thread.queue(move |mut c: Pin<&mut qobject::FileSyncCore>| {
                c.as_mut().set_busy(false);
                match result {
                    Err(text) => msg(c, "error", &text),
                    Ok((plan, rejected)) => {
                        let (rows, level, text) = plan_json(&plan, &rejected);
                        c.as_mut().rust_mut().plan = plan;
                        c.as_mut().plan_ready(qs(&rows.to_string()));
                        msg(c, level, &text);
                    }
                }
            });
        });
    }

    fn run(self: Pin<&mut Self>, checked_json: &QString, local: &QString, remote: &QString) {
        let todo: Vec<SyncAction> = int_list("run", checked_json)
            .iter()
            .filter_map(|i| self.plan.get(*i))
            .filter(|a| a.op != SyncOp::Conflict)
            .cloned()
            .collect();
        if todo.is_empty() {
            return;
        }
        let local_root = PathBuf::from(expand_user(
            &stripped(local),
            std::env::var("HOME").ok().as_deref(),
        ));
        let remote_root = stripped(remote).into_bytes();
        // Defence in depth: every local path must stay inside the chosen folder.
        let plan = match RunPlan::prepare(&local_root, &remote_root, &todo) {
            Ok(p) => p,
            Err(text) => {
                msg(self, "error", &text);
                return;
            }
        };
        for d in &plan.local_dirs {
            if let Err(e) = std::fs::create_dir_all(d) {
                msg(
                    self,
                    "error",
                    &format!("Cannot create {}: {e}", d.display()),
                );
                return;
            }
        }
        let Some(conn) = self.conn.clone() else {
            return;
        };
        let launch = launch();
        let sys_env = Env::system();
        let ops = RemoteOps {
            conn: &conn,
            launch: &launch,
            env: &sys_env,
        };
        let mkdir = match plan.mkdir_argv(&ops).transpose() {
            Ok(m) => m,
            Err(e) => {
                let t = if e.is_empty() {
                    "Cannot create the remote folders.".to_owned()
                } else {
                    e
                };
                msg(self, "error", &t);
                return;
            }
        };
        let Some(argv) = mkdir else {
            self.enqueue_copies(&plan);
            return;
        };
        let env = match command_env(&conn, &self.label) {
            Ok(e) => e,
            Err(_) => {
                msg(
                    self,
                    "error",
                    "The saved password is locked — enter the master password.",
                );
                return;
            }
        };
        let thread = self.qt_thread();
        std::thread::spawn(move || {
            let done = run_blocking(&argv, env, 120);
            let _ = thread.queue(move |c: Pin<&mut qobject::FileSyncCore>| match done {
                None => {}
                Some(d) if d.rc != 0 => msg(
                    c,
                    "error",
                    &format!("Creating remote folders failed: {}", err_text(&d.err, d.rc)),
                ),
                Some(_) => c.enqueue_copies(&plan),
            });
        });
    }

    /// The copies go to the panel's queue with the mtime kept (`-p`).
    fn enqueue_copies(mut self: Pin<&mut Self>, plan: &RunPlan) {
        let Some(conn) = self.conn.clone() else {
            return;
        };
        let (queue, label) = (self.queue.clone(), self.label.clone());
        for c in &plan.copies {
            if !c.upload
                && let Some(dir) = c.local.parent()
            {
                let _ = std::fs::create_dir_all(dir);
            }
            let result = queue_scp(ScpRequest {
                queue: &queue,
                host_label: &label,
                conn: &conn,
                upload: c.upload,
                local: c.local.clone(),
                remote: c.remote.clone(),
                label: c.label.clone(),
                recursive: false,
                preserve: true,
                purpose: String::new(),
            });
            if let Err(QueueError::Status(t) | QueueError::Warn(t)) = result {
                msg(self.as_mut(), "error", &t);
                return;
            }
        }
        msg(
            self.as_mut(),
            "positive",
            &format!(
                "{} copy job(s) queued — see the transfer list.",
                plan.copies.len()
            ),
        );
        self.as_mut().rust_mut().plan.clear();
        self.queued();
    }

    fn shutdown(mut self: Pin<&mut Self>) {
        if let Some(c) = self.as_mut().rust_mut().cancel.take() {
            c.cancel();
        }
    }
}
