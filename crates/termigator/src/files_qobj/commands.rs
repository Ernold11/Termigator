//! `FilesCore` methods that are not invokables: state helpers, the command runner and its
//! completion, navigation, transfer queueing, operations and Edit.

use super::*;

impl qobject::FilesCore {
    // ---- state helpers

    pub(super) fn entry_at(&self, row: usize) -> Option<&RemoteEntry> {
        match self.rows.get(row)? {
            Row::Entry(i) => self.panel.entries().get(*i),
            Row::Up => None,
        }
    }

    /// The entries behind `rows` (the ".." row has none).
    pub(super) fn entries_of(&self, rows: &[usize]) -> Vec<RemoteEntry> {
        rows.iter()
            .filter_map(|r| self.entry_at(*r).cloned())
            .collect()
    }

    pub(super) fn host_label(&self) -> String {
        self.label.to_string()
    }

    pub(super) fn say(mut self: Pin<&mut Self>, text: &str) {
        self.as_mut().set_status(qs(text));
    }

    pub(super) fn warn(mut self: Pin<&mut Self>, text: &str) {
        self.as_mut().warning(qs(text));
    }

    /// Rebuilds the rows from the state; the status line follows.
    pub(super) fn repopulate(mut self: Pin<&mut Self>) {
        self.as_mut().begin_reset_model();
        let rows = self.panel.rows();
        self.as_mut().rust_mut().rows = rows;
        self.as_mut().end_reset_model();
        let cwd = self.panel.cwd().to_vec();
        self.as_mut().set_cwd(qs(&display_name(&cwd)));
        if !cwd.is_empty() {
            let shown = self
                .rows
                .iter()
                .filter(|r| matches!(r, Row::Entry(_)))
                .count();
            let text = listing_status(&self.host_label(), &cwd, shown, self.panel.entries());
            self.say(&text);
        }
    }

    /// Starts `argv` for `pending`; the result comes back through [`Self::command_done`].
    pub(super) fn run(mut self: Pin<&mut Self>, argv: Argv, pending: Pending) {
        let Some(conn) = self.panel.conn().cloned() else {
            return;
        };
        let Some(ticket) = self.as_mut().rust_mut().panel.begin_command() else {
            return;
        };
        let env = match command_env(&conn, &self.host_label()) {
            Ok(env) => env,
            Err(text) => {
                self.as_mut().rust_mut().panel.finish_command(&ticket);
                self.say(&text);
                return;
            }
        };
        self.as_mut().set_busy(true);
        let token = self.next_token + 1;
        self.as_mut().rust_mut().next_token = token;
        let mut cmd = Command::new(
            argv.iter()
                .map(|a| termigator_core::platform::os_string_from_vec(a.clone())),
        );
        cmd.env = env;
        cmd.timeout = Some(Duration::from_secs(LIST_TIMEOUT));
        let running = match runner::spawn(cmd) {
            Ok(r) => r,
            Err(e) => {
                let cancel = CancelHandle::default();
                self.as_mut()
                    .rust_mut()
                    .pending
                    .insert(token, (pending, ticket, cancel));
                self.command_done(token, 127, Vec::new(), e.to_string().into_bytes());
                return;
            }
        };
        let cancel = running.cancel_handle();
        self.as_mut()
            .rust_mut()
            .pending
            .insert(token, (pending, ticket, cancel));
        let thread = self.qt_thread();
        let spawned = std::thread::Builder::new()
            .name("termigator-files-cmd".to_owned())
            .spawn(move || {
                let Some(out) = running.wait() else { return };
                if out.cancelled {
                    return;
                }
                let (rc, o, e) = if out.timed_out {
                    (
                        124,
                        Vec::new(),
                        format!("Timed out after {LIST_TIMEOUT}s.").into_bytes(),
                    )
                } else {
                    (out.code(), out.stdout, out.stderr)
                };
                let _ = thread.queue(move |c: Pin<&mut qobject::FilesCore>| {
                    c.command_done(token, rc, o, e);
                });
            });
        if spawned.is_err() {
            self.command_done(token, 1, Vec::new(), b"cannot start a thread".to_vec());
        }
    }

    pub(super) fn command_done(
        mut self: Pin<&mut Self>,
        token: u64,
        rc: i32,
        out: Vec<u8>,
        err: Vec<u8>,
    ) {
        let Some((pending, ticket, _)) = self.as_mut().rust_mut().pending.remove(&token) else {
            return;
        };
        self.as_mut().set_busy(false);
        let finish = self.as_mut().rust_mut().panel.finish_command(&ticket);
        let pending_follow = match finish {
            Finish::Stale { relist } => {
                // Host A's result, panel on host B now: drop it, list B if it waits.
                if relist && self.active {
                    self.as_mut().refresh();
                }
                return;
            }
            Finish::Accepted { pending_follow } => pending_follow,
        };
        match pending {
            Pending::List(path) => self.as_mut().listing_done(&ticket, &path, rc, &out, &err),
            Pending::UploadCheck { dir, paths } => {
                self.as_mut().upload_check_done(&dir, paths, rc, &out, &err);
            }
            Pending::Op(prefix) => {
                if rc == 0 {
                    self.as_mut().refresh();
                } else {
                    self.as_mut()
                        .say(&format!("{prefix}: {}", err_text(&err, rc)));
                }
            }
        }
        if let Some(p) = pending_follow
            && let Some(path) = self.panel.follow_after(p)
        {
            self.as_mut().navigate_bytes(path);
        }
        if self.refresh_after && !self.panel.is_busy() {
            self.as_mut().rust_mut().refresh_after = false;
            self.refresh();
        }
    }

    pub(super) fn listing_done(
        mut self: Pin<&mut Self>,
        ticket: &Ticket,
        path: &[u8],
        rc: i32,
        out: &[u8],
        err: &[u8],
    ) {
        if rc != 0 {
            let retry = self.as_mut().rust_mut().panel.listing_failed(ticket, path);
            if let Some(home) = retry {
                self.navigate_bytes(home); // the remembered folder is gone — start at home
            } else if is_auth_failure(rc, err) {
                // Typed in the terminal, not saved: Files cannot log in by itself (yet).
                self.as_mut().set_auth_required(true);
                self.say("Authentication required — Files has no password for this host.");
            } else {
                self.say(&format!("Listing failed: {}", err_text(err, rc)));
            }
            return;
        }
        self.as_mut().set_auth_required(false);
        match parse_ls_output(out) {
            Err(text) => self.say(&text),
            Ok((cwd, entries)) => {
                let applied =
                    self.as_mut()
                        .rust_mut()
                        .panel
                        .apply_listing(ticket, cwd.clone(), entries);
                if applied {
                    self.as_mut().repopulate();
                    if let Some(key) = self.panel.host_key().map(str::to_owned) {
                        with_store(|s| panel::remember_last(s, &key, &cwd));
                    }
                }
            }
        }
    }

    pub(super) fn navigate_bytes(mut self: Pin<&mut Self>, path: Vec<u8>) {
        if self.panel.conn().is_none() || self.panel.is_busy() {
            return;
        }
        let Some(conn) = self.panel.conn().cloned() else {
            return;
        };
        let argv = match build(&conn, |o| o.list_dir_argv(&path)) {
            Ok(a) => a,
            Err(e) => {
                let text = if e.is_empty() {
                    "Cannot build ssh command.".to_owned()
                } else {
                    e
                };
                self.say(&text);
                return;
            }
        };
        self.as_mut().rust_mut().panel.listing_started();
        let shown = if path == b"." {
            "home".to_owned()
        } else {
            display_name(&path)
        };
        self.as_mut().say(&format!("Listing {shown}…"));
        self.run(argv, Pending::List(path));
    }

    /// Queues one scp job against `conn` (default: the panel's host).
    pub(super) fn enqueue(
        self: Pin<&mut Self>,
        conn: Option<Value>,
        job: Transfer,
    ) -> Result<(), ()> {
        let Some(conn) = conn.or_else(|| self.panel.conn().cloned()) else {
            return Err(());
        };
        let queue = self.transfers_id.to_string();
        let host_label = self.host_label();
        let result = queue_scp(ScpRequest {
            queue: &queue,
            host_label: &host_label,
            conn: &conn,
            upload: job.upload,
            local: job.local,
            remote: job.remote,
            label: job.label,
            recursive: job.recursive,
            preserve: false,
            purpose: job.purpose,
        });
        match result {
            Ok(()) => Ok(()),
            Err(QueueError::Status(t)) => {
                self.say(&t);
                Err(())
            }
            Err(QueueError::Warn(t)) => {
                self.warn(&t);
                Err(())
            }
        }
    }

    /// Lists `dir` (not the folder shown) before uploading `paths` into it.
    pub(super) fn check_upload(self: Pin<&mut Self>, dir: Vec<u8>, paths: Vec<String>) {
        let Some(conn) = self.panel.conn().cloned() else {
            return;
        };
        if self.panel.is_busy() {
            self.say("Busy — drop the files again in a moment.");
            return;
        }
        match build(&conn, |o| o.list_dir_argv(&dir)) {
            Ok(argv) => self.run(argv, Pending::UploadCheck { dir, paths }),
            Err(e) => self.say(&format!("Upload: {e}")),
        }
    }

    /// The listing for [`Self::check_upload`]: no clash uploads at once, else the user is asked.
    /// A failed listing uploads nothing (an unknown folder could hold anything).
    pub(super) fn upload_check_done(
        mut self: Pin<&mut Self>,
        dir: &[u8],
        paths: Vec<String>,
        rc: i32,
        out: &[u8],
        err: &[u8],
    ) {
        let listed = if rc == 0 {
            parse_ls_output(out)
        } else {
            Err(err_text(err, rc))
        };
        let entries = match listed {
            Ok((_, entries)) => entries,
            Err(text) => {
                self.say(&format!(
                    "Upload stopped: cannot list {}: {text}",
                    display_name(dir)
                ));
                return;
            }
        };
        let clashes = upload_clash_names(&paths, &entries);
        if clashes.is_empty() {
            self.enqueue_uploads(dir, &paths, &[]);
            return;
        }
        let question = clash_question(&clashes, &display_name(dir));
        self.as_mut().rust_mut().upload_check = Some((dir.to_vec(), paths, clashes));
        self.upload_clashes_found(qs(&question));
    }

    /// Queues `paths` for upload into `dir`, leaving out the base names in `skip`.
    pub(super) fn enqueue_uploads(
        mut self: Pin<&mut Self>,
        dir: &[u8],
        paths: &[String],
        skip: &[String],
    ) {
        for p in paths {
            let base = base_name(Path::new(p));
            if skip.contains(&base) {
                continue;
            }
            let mut remote = dir.to_vec();
            while remote.len() > 1 && remote.ends_with(b"/") {
                remote.pop();
            }
            if !remote.ends_with(b"/") {
                remote.push(b'/');
            }
            let tag = self.next_tag + 1;
            self.as_mut().rust_mut().next_tag = tag;
            let is_dir = Path::new(p).is_dir();
            let _ = self.as_mut().enqueue(
                None,
                Transfer {
                    upload: true,
                    local: PathBuf::from(p),
                    remote,
                    label: format!("{base}{}", if is_dir { "/" } else { "" }).into_bytes(),
                    recursive: false,
                    purpose: format!("upload:{tag}"),
                },
            );
            let ticket = self.panel.ticket();
            self.as_mut().rust_mut().upload_tickets.insert(tag, ticket);
        }
    }

    /// `stat`s `remote` on `conn` in the background (not the panel's command: the edit
    /// may belong to another host); the answer goes to [`Self::side_stat_done`]. Anything that
    /// keeps the stat from running counts as "unknown", which never blocks a save.
    pub(super) fn side_stat(mut self: Pin<&mut Self>, conn: Value, remote: &[u8], what: SideStat) {
        let token = self.next_token + 1;
        self.as_mut().rust_mut().next_token = token;
        self.as_mut().rust_mut().side_pending.insert(token, what);
        let started = command_env(&conn, &self.host_label())
            .and_then(|env| Ok((env, build(&conn, |o| o.stat_argv(remote))?)))
            .and_then(|(env, argv)| {
                let mut cmd = Command::new(
                    argv.iter()
                        .map(|a| termigator_core::platform::os_string_from_vec(a.clone())),
                );
                cmd.env = env;
                cmd.timeout = Some(Duration::from_secs(LIST_TIMEOUT));
                runner::spawn(cmd).map_err(|e| e.to_string())
            });
        let Ok(running) = started else {
            self.side_stat_done(token, 1, Vec::new());
            return;
        };
        let thread = self.qt_thread();
        let spawned = std::thread::Builder::new()
            .name("termigator-files-stat".to_owned())
            .spawn(move || {
                let (rc, out) = match running.wait() {
                    Some(o) if !o.cancelled && !o.timed_out => (o.code(), o.stdout),
                    _ => (1, Vec::new()),
                };
                let _ = thread.queue(move |c: Pin<&mut qobject::FilesCore>| {
                    c.side_stat_done(token, rc, out);
                });
            });
        if spawned.is_err() {
            self.side_stat_done(token, 1, Vec::new());
        }
    }

    pub(super) fn side_stat_done(mut self: Pin<&mut Self>, token: u64, rc: i32, out: Vec<u8>) {
        let Some(what) = self.as_mut().rust_mut().side_pending.remove(&token) else {
            return;
        };
        let now = Some(String::from_utf8_lossy(&out).trim().to_owned())
            .filter(|t| rc == 0 && !t.is_empty());
        match what {
            SideStat::Base(local) => match now {
                Some(n) => {
                    self.as_mut().rust_mut().edit_base.insert(local, n);
                }
                None => {
                    self.as_mut().rust_mut().edit_base.remove(&local);
                }
            },
            SideStat::Check(up) => {
                let base = self.edit_base.get(&up.local);
                if base.is_some() && now.is_some() && base != now.as_ref() {
                    let name =
                        display_name(up.remote.rsplit(|b| *b == b'/').next().unwrap_or_default());
                    let question = format!(
                        "{name} changed on the server since you opened it. Upload your save over it?\n\nNo keeps the server copy; your next save asks again."
                    );
                    let local = up.local.to_string_lossy().into_owned();
                    self.as_mut()
                        .rust_mut()
                        .edit_conflicts
                        .insert(up.local.clone(), up);
                    self.as_mut()
                        .say(&format!("Not uploaded yet: {name} changed on the server."));
                    self.edit_conflict(qs(&question), qs(&local));
                } else {
                    self.enqueue_save(up);
                }
            }
        }
    }

    /// Queues the upload of a saved working copy.
    pub(super) fn enqueue_save(mut self: Pin<&mut Self>, up: EditUpload) {
        let label = format!(
            "✎ {} (save)",
            display_name(up.remote.rsplit(|b| *b == b'/').next().unwrap_or_default())
        );
        let local = up.local.clone();
        self.as_mut()
            .rust_mut()
            .save_targets
            .insert(local.clone(), (up.conn.clone(), up.remote.clone()));
        let queued = self.as_mut().enqueue(
            Some(up.conn),
            Transfer {
                upload: true,
                local: up.local,
                remote: up.remote,
                label: label.into_bytes(),
                recursive: false,
                purpose: format!("save:{}", local.display()),
            },
        );
        if queued.is_err() {
            self.as_mut().rust_mut().save_targets.remove(&local);
            self.as_mut().edit_upload_failed(&local);
        }
    }

    /// The folder an upload goes to: the folder of `row` if that is one, else the current one.
    pub(super) fn upload_dir(&self, row: i32) -> Vec<u8> {
        usize::try_from(row)
            .ok()
            .and_then(|r| self.entry_at(r))
            .filter(|e| e.is_dir)
            .map_or_else(
                || self.panel.cwd().to_vec(),
                |e| remote_join(self.panel.cwd(), &e.name),
            )
    }

    /// Starts an operation: `Err(text)` from the builder is shown as `"<prefix>: <text>"`.
    pub(super) fn operation(
        self: Pin<&mut Self>,
        prefix: &'static str,
        build_argv: impl FnOnce(&RemoteOps<'_>) -> Result<Argv, String>,
    ) {
        let Some(conn) = self.panel.conn().cloned() else {
            return;
        };
        if self.panel.is_busy() {
            return;
        }
        match build(&conn, build_argv) {
            Ok(argv) => self.run(argv, Pending::Op(prefix)),
            Err(e) => {
                let text = if e.is_empty() {
                    "cannot build ssh command".to_owned()
                } else {
                    e
                };
                self.say(&format!("{prefix}: {text}")); // not silent
            }
        }
    }

    pub(super) fn open_for_edit(
        mut self: Pin<&mut Self>,
        local: PathBuf,
        conn: Value,
        remote: Vec<u8>,
    ) {
        let fp = file_fingerprint(&local);
        let host = conn
            .get("host")
            .map(termigator_core::config::model::value_str)
            .unwrap_or_default();
        self.as_mut().rust_mut().edit_conflicts.remove(&local);
        self.as_mut()
            .side_stat(conn.clone(), &remote, SideStat::Base(local.clone()));
        self.as_mut().rust_mut().edits.insert(
            local.clone(),
            EditSession::new(local.clone(), conn, remote.clone(), fp),
        );
        let seen = std::fs::metadata(&local)
            .ok()
            .and_then(|m| Some((m.modified().ok()?, m.len())));
        match seen {
            Some(m) => self.as_mut().rust_mut().edit_seen.insert(local.clone(), m),
            None => self.as_mut().rust_mut().edit_seen.remove(&local),
        };
        self.as_mut().set_editing(true);
        let editor = launch()
            .get("editor")
            .map(termigator_core::config::model::value_str)
            .map(|e| strip_space(&e).to_owned())
            .unwrap_or_default();
        if editor.is_empty() {
            self.as_mut().open_url(qs(&local.to_string_lossy()));
        } else {
            let started = split(&editor)
                .map_err(|e| e.to_string())
                .and_then(|mut argv| {
                    if argv.is_empty() {
                        return Err("empty command".to_owned());
                    }
                    let prog = argv.remove(0);
                    let mut cmd = std::process::Command::new(prog);
                    cmd.args(argv).arg(&local);
                    termigator_core::platform::new_process_group(&mut cmd)
                        .spawn()
                        .map_err(|e| e.to_string())
                });
            match started {
                Ok(mut child) => {
                    std::thread::spawn(move || {
                        let _ = child.wait(); // reaped, not waited for
                    });
                }
                Err(err) => {
                    self.say(&format!("Could not open an editor: {err}"));
                    return;
                }
            }
        }
        let base = remote.rsplit(|b| *b == b'/').next().unwrap_or_default();
        self.say(&format!(
            "Editing {} — every save is uploaded back to {host}:{}",
            display_name(base),
            display_name(&remote)
        ));
    }
}
