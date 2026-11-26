//! Tunnels, snippets, bulk edit and the connection probes of the notice: `ToolsBackend`,
//! the Rust half of `qml/tools/`.
//!
//! * Tunnels: the saved list is `tunnels` in the config; the
//!   running state lives here. A tunnel is a supervised `ssh -N`; `poll()` (a QML timer) turns
//!   its events into `tunnel_changed(name)`. Resolving the ssh command for a tunnel uses
//!   the SSH entry named by `via` with its credential, the password
//!   revealed with the vault's session key. A locked vault answers "locked" from `start_tunnel`;
//!   the dialog asks for the master password through `panes.unlockVault` and tries again.
//!   A tunnel's own `identity_file` (optional) replaces the entry's key.
//! * Snippets: the list, the questions and the text
//!   typed for one session (`text::snippets`).
//! * The probe of "Test connection…" from a tab and of the disconnect notice,
//!   on a worker thread, answered with `test_finished(token, report)` (same shape as the editors').

use std::collections::{BTreeMap, HashMap};
use std::ffi::OsString;
use std::pin::Pin;
use std::time::Duration;

use cxx_qt::{CxxQtType, Threading};
use cxx_qt_lib::QString;
use serde_json::{Value as J, json};
use termigator_core::config::doc::Value;
use termigator_core::config::environments;
use termigator_core::config::model::{section, truthy, value_str};
use termigator_core::connectivity::probe_connection;
use termigator_core::credentials::apply_credential;
use termigator_core::proc::tunnel::{SETTLE, Tunnel, TunnelEvent};
use termigator_core::proto::gateway::find_gateway;
use termigator_core::proto::ssh::{Env, ssh_password_env};
use termigator_core::proto::tunnels::{
    Tunnel as SavedTunnel, forwards_summary, normalize_tunnels, parse_forwards, tunnel_argv,
};
use termigator_core::text::snippets::{
    SnippetConn, expand, normalize_snippets, questions, text_to_send,
};
use termigator_core::vault::{self, Secret};

use crate::editors_qobj::{key_file_problem, key_locked, key_path, report_json, ssh_entry_labels};
use crate::shell_qobj::{set_in, with_app};
use crate::sidebar_qobj::probe_timeout;

/// The notice probes the host quickly and without ping.
const NOTICE_TIMEOUT: Duration = Duration::from_secs(4);

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
        type ToolsBackend = super::ToolsBackendRust;

        /// The status of a tunnel changed (its name).
        #[qsignal]
        fn tunnel_changed(self: Pin<&mut ToolsBackend>, name: QString);

        /// A probe started with `start_test` / `start_notice_probe` finished.
        #[qsignal]
        fn test_finished(self: Pin<&mut ToolsBackend>, token: QString, report_json: QString);

        /// The saved tunnels with their live status as JSON:
        /// `[{name, via, forwards, summary, autostart, status, statusText, color, detail}]`.
        #[qinvokable]
        fn tunnels_json(self: &ToolsBackend) -> QString;

        /// Labels of the SSH entries (the choices of "Through SSH entry").
        #[qinvokable]
        fn ssh_labels_json(self: &ToolsBackend) -> QString;

        /// The warning for the form (`original` = the name
        /// being edited, "" for a new tunnel), or "" when it may be saved.
        #[qinvokable]
        fn tunnel_error(self: &ToolsBackend, original: &QString, form_json: &QString) -> QString;

        /// Stores the form (replacing `original`, or appended when empty); the caller persists.
        #[qinvokable]
        fn save_tunnel(self: &ToolsBackend, original: &QString, form_json: &QString);

        /// Removes the tunnel from the config and forgets its state; the caller persists.
        #[qinvokable]
        fn delete_tunnel(self: Pin<&mut ToolsBackend>, name: &QString);

        /// The Autostart check box; the caller persists.
        #[qinvokable]
        fn set_autostart(self: &ToolsBackend, name: &QString, on: bool);

        /// Starts a saved tunnel. "" = started, failed (the status says why) or already live;
        /// "locked" = the vault must be unlocked first.
        #[qinvokable]
        fn start_tunnel(self: Pin<&mut ToolsBackend>, name: &QString) -> QString;

        /// Marks a tunnel failed with `text` (an unlock that was not given).
        #[qinvokable]
        fn fail_tunnel(self: Pin<&mut ToolsBackend>, name: &QString, text: &QString);

        /// Asks a tunnel to end; its status turns "stopped" once the process is gone.
        #[qinvokable]
        fn stop_tunnel(self: Pin<&mut ToolsBackend>, name: &QString);

        /// Stops a tunnel and drops its state (edit, delete).
        #[qinvokable]
        fn forget_tunnel(self: Pin<&mut ToolsBackend>, name: &QString);

        /// The warning under the tunnel's key field ("" = none): not found, a public key, a
        /// `.ppk`, readable by others.
        #[qinvokable]
        fn key_file_problem(self: &ToolsBackend, path: &QString) -> QString;

        /// Whether the tunnel's key file is protected by a passphrase (the tunnel has no terminal
        /// to ask for it, so the key has to be in the SSH agent).
        #[qinvokable]
        fn key_needs_passphrase(self: &ToolsBackend, path: &QString) -> bool;

        /// The tunnel's key path with `~` expanded, as ssh-add and the key list name it.
        #[qinvokable]
        fn key_path(self: &ToolsBackend, path: &QString) -> QString;

        /// Ends every tunnel and waits until the processes are gone (quitting).
        #[qinvokable]
        fn stop_all(self: Pin<&mut ToolsBackend>);

        /// Tunnels that are starting or running (the close question).
        #[qinvokable]
        fn running_count(self: &ToolsBackend) -> i32;

        /// Names of the tunnels with autostart on, as JSON.
        #[qinvokable]
        fn autostart_json(self: &ToolsBackend) -> QString;

        /// Applies the events of the tunnel processes (called by a timer).
        #[qinvokable]
        fn poll(self: Pin<&mut ToolsBackend>);

        /// The snippets (normalized) as JSON `[{name, command, enter}]`.
        #[qinvokable]
        fn snippets_json(self: &ToolsBackend) -> QString;

        /// Stores the edited list (normalized like `SnippetsDialog.snippets()`); the caller persists.
        #[qinvokable]
        fn save_snippets(self: &ToolsBackend, list_json: &QString);

        /// The `{?Question}` prompts of a command, as JSON.
        #[qinvokable]
        fn snippet_questions(self: &ToolsBackend, command: &QString) -> QString;

        /// What is typed into one session: the command expanded for `conn_json` (the tab's
        /// entry, or "" / "null"), `title` as the label when the entry has none.
        #[qinvokable]
        fn snippet_text(
            self: &ToolsBackend,
            command: &QString,
            enter: bool,
            conn_json: &QString,
            title: &QString,
            answers_json: &QString,
        ) -> QString;

        /// Names of the named credentials (bulk edit).
        #[qinvokable]
        fn credential_names_json(self: &ToolsBackend) -> QString;

        /// Environment categories for the bulk-edit combo: `[{id, name, color}, …]`.
        #[qinvokable]
        fn environments_json(self: &ToolsBackend) -> QString;

        /// `window.notify_on_disconnect` off ("Don't notify me…"); the caller persists.
        #[qinvokable]
        fn mute_disconnect_notices(self: &ToolsBackend);

        /// Test connection with ping and `launch.probe_timeout`; answers with `test_finished`.
        #[qinvokable]
        fn start_test(self: &ToolsBackend, token: &QString, conn_json: &QString);

        /// The disconnect notice's quick probe (4 s, no ping); answers with `test_finished`.
        #[qinvokable]
        fn start_notice_probe(self: &ToolsBackend, token: &QString, conn_json: &QString);
    }

    impl cxx_qt::Threading for ToolsBackend {}
}

#[derive(Default)]
pub struct ToolsBackendRust {
    runs: Runs,
}

fn qs(s: &str) -> QString {
    QString::from(s)
}

fn parse_json(invokable: &str, text: &QString) -> J {
    crate::qml_json::value(invokable, &text.to_string()).unwrap_or(J::Null)
}

impl qobject::ToolsBackend {
    fn tunnels_json(&self) -> QString {
        let saved = with_app(|app| saved_tunnels(&app.cfg));
        qs(&tunnels_view(&saved, &self.runs).to_string())
    }

    fn ssh_labels_json(&self) -> QString {
        qs(&json!(with_app(|app| ssh_entry_labels(&app.cfg))).to_string())
    }

    fn tunnel_error(&self, original: &QString, form_json: &QString) -> QString {
        let taken: Vec<String> = with_app(|app| saved_tunnels(&app.cfg))
            .into_iter()
            .map(|t| t.name)
            .filter(|n| *n != original.to_string())
            .collect();
        qs(&form_error(&parse_json("tunnel_error", form_json), &taken).unwrap_or_default())
    }

    fn save_tunnel(&self, original: &QString, form_json: &QString) {
        let form = form_tunnel(&parse_json("save_tunnel", form_json));
        let original = original.to_string();
        with_app(|app| {
            let mut all = saved_tunnels(&app.cfg);
            match all.iter_mut().find(|t| t.name == original) {
                Some(slot) if !original.is_empty() => *slot = form,
                _ => all.push(form),
            }
            store_tunnels(&mut app.cfg, &all);
        });
    }

    fn delete_tunnel(mut self: Pin<&mut Self>, name: &QString) {
        let name = name.to_string();
        with_app(|app| {
            let mut all = saved_tunnels(&app.cfg);
            all.retain(|t| t.name != name);
            store_tunnels(&mut app.cfg, &all);
        });
        self.as_mut().rust_mut().runs.forget(&name);
        self.tunnel_changed(qs(&name));
    }

    fn set_autostart(&self, name: &QString, on: bool) {
        let name = name.to_string();
        with_app(|app| {
            let mut all = saved_tunnels(&app.cfg);
            for t in all.iter_mut().filter(|t| t.name == name) {
                t.autostart = on;
            }
            store_tunnels(&mut app.cfg, &all);
        });
    }

    fn start_tunnel(mut self: Pin<&mut Self>, name: &QString) -> QString {
        let name = name.to_string();
        if self.runs.live(&name) {
            return QString::default();
        }
        let key = vault::session_key();
        let resolved = with_app(|app| {
            let saved = saved_tunnels(&app.cfg).into_iter().find(|t| t.name == name);
            match saved {
                Some(t) => resolve_tunnel(&app.cfg, &tunnel_value(&t), key.as_ref()),
                None => Err(Resolve::Failed(format!("No tunnel named {name:?}."))),
            }
        });
        match resolved {
            Err(Resolve::Locked) => return qs("locked"),
            Err(Resolve::Failed(text)) => self.as_mut().rust_mut().runs.fail(&name, &text),
            Ok((argv, env)) => self
                .as_mut()
                .rust_mut()
                .runs
                .start(&name, &argv, &env, SETTLE),
        }
        self.tunnel_changed(qs(&name));
        QString::default()
    }

    fn fail_tunnel(mut self: Pin<&mut Self>, name: &QString, text: &QString) {
        self.as_mut()
            .rust_mut()
            .runs
            .fail(&name.to_string(), &text.to_string());
        self.tunnel_changed(name.clone());
    }

    fn stop_tunnel(mut self: Pin<&mut Self>, name: &QString) {
        let changed = self.as_mut().rust_mut().runs.stop(&name.to_string());
        if changed {
            self.tunnel_changed(name.clone());
        }
    }

    fn forget_tunnel(mut self: Pin<&mut Self>, name: &QString) {
        self.as_mut().rust_mut().runs.forget(&name.to_string());
        self.tunnel_changed(name.clone());
    }

    fn key_file_problem(&self, path: &QString) -> QString {
        qs(&key_file_problem(&path.to_string()))
    }

    fn key_needs_passphrase(&self, path: &QString) -> bool {
        key_locked(&path.to_string())
    }

    fn key_path(&self, path: &QString) -> QString {
        qs(&key_path(&path.to_string()))
    }

    fn stop_all(mut self: Pin<&mut Self>) {
        let names = self.as_mut().rust_mut().runs.stop_all();
        for n in names {
            self.as_mut().tunnel_changed(qs(&n));
        }
    }

    fn running_count(&self) -> i32 {
        i32::try_from(self.runs.running_count()).unwrap_or(i32::MAX)
    }

    fn autostart_json(&self) -> QString {
        let names: Vec<String> = with_app(|app| saved_tunnels(&app.cfg))
            .into_iter()
            .filter(|t| t.autostart)
            .map(|t| t.name)
            .collect();
        qs(&json!(names).to_string())
    }

    fn poll(mut self: Pin<&mut Self>) {
        let changed = self.as_mut().rust_mut().runs.poll();
        for n in changed {
            self.as_mut().tunnel_changed(qs(&n));
        }
    }

    fn snippets_json(&self) -> QString {
        qs(&snippet_list(&with_app(|app| app.cfg.to_json())).to_string())
    }

    fn save_snippets(&self, list_json: &QString) {
        let list = snippet_list(&json!({"snippets": parse_json("save_snippets", list_json)}));
        with_app(|app| set_in(&mut app.cfg, &["snippets"], Value::from_json(&list)));
    }

    fn snippet_questions(&self, command: &QString) -> QString {
        qs(&json!(questions(&command.to_string())).to_string())
    }

    fn snippet_text(
        &self,
        command: &QString,
        enter: bool,
        conn_json: &QString,
        title: &QString,
        answers_json: &QString,
    ) -> QString {
        let answers: HashMap<String, String> = match parse_json("snippet_text", answers_json) {
            J::Object(o) => o
                .into_iter()
                .map(|(k, v)| (k, v.as_str().unwrap_or_default().to_owned()))
                .collect(),
            _ => HashMap::new(),
        };
        let conn = snippet_conn(&parse_json("snippet_text", conn_json), &title.to_string());
        qs(&text_to_send(
            &expand(&command.to_string(), &conn, &answers),
            enter,
        ))
    }

    fn credential_names_json(&self) -> QString {
        let names: Vec<String> = with_app(|app| match app.cfg.get("credentials") {
            Some(Value::Seq(items)) => items
                .iter()
                .filter(|c| matches!(c, Value::Map(_)))
                .filter_map(|c| c.get("name").filter(|n| truthy(n)).map(value_str))
                .collect(),
            _ => Vec::new(),
        });
        qs(&json!(names).to_string())
    }

    fn environments_json(&self) -> QString {
        with_app(|app| qs(&environments::to_json(&environments::list(&app.cfg)).to_string()))
    }

    fn mute_disconnect_notices(&self) {
        with_app(|app| {
            set_in(
                &mut app.cfg,
                &["window", "notify_on_disconnect"],
                Value::Bool(false),
            )
        });
    }

    fn start_test(&self, token: &QString, conn_json: &QString) {
        let timeout = with_app(|app| probe_timeout(&app.cfg));
        self.probe(token, conn_json, timeout, true);
    }

    fn start_notice_probe(&self, token: &QString, conn_json: &QString) {
        self.probe(token, conn_json, NOTICE_TIMEOUT, false);
    }

    /// A plain thread; a probe cannot be interrupted, and once the object is gone the queued
    /// answer is dropped.
    fn probe(&self, token: &QString, conn_json: &QString, timeout: Duration, ping: bool) {
        let conn = Value::from_json(&parse_json("probe", conn_json));
        if !matches!(conn, Value::Map(_)) {
            return;
        }
        let token = token.to_string();
        let thread = self.qt_thread();
        std::thread::spawn(move || {
            let report = report_json(&probe_connection(&conn, timeout, ping)).to_string();
            let _ = thread.queue(move |t: Pin<&mut qobject::ToolsBackend>| {
                t.test_finished(qs(&token), qs(&report));
            });
        });
    }
}

// ---- Pure logic (unit-tested below) ------------------------------------------------------

/// `normalize_tunnels(cfg["tunnels"])`.
fn saved_tunnels(cfg: &Value) -> Vec<SavedTunnel> {
    normalize_tunnels(cfg.get("tunnels").unwrap_or(&Value::Null))
}

/// A tunnel as the mapping the config and `tunnel_argv` use.
fn tunnel_value(t: &SavedTunnel) -> Value {
    let s = |text: &str| Value::Str(text.to_owned());
    let mut pairs = vec![
        (s("name"), s(&t.name)),
        (s("via"), s(&t.via)),
        (
            s("forwards"),
            Value::Seq(t.forwards.iter().map(|f| s(f)).collect()),
        ),
        (s("autostart"), Value::Bool(t.autostart)),
    ];
    // Only when set, so the tunnels saved before the field existed are written unchanged.
    if !t.identity_file.is_empty() {
        pairs.push((s("identity_file"), s(&t.identity_file)));
    }
    Value::Map(pairs)
}

/// An empty list removes the key.
fn store_tunnels(cfg: &mut Value, all: &[SavedTunnel]) {
    if all.is_empty() {
        if let Value::Map(pairs) = cfg {
            pairs.retain(|(k, _)| !matches!(k, Value::Str(s) if s == "tunnels"));
        }
    } else {
        let list = Value::Seq(all.iter().map(tunnel_value).collect());
        set_in(cfg, &["tunnels"], list);
    }
}

/// `TunnelEditDialog.tunnel()`: the form's trimmed fields.
fn form_tunnel(form: &J) -> SavedTunnel {
    let text = |k: &str| form[k].as_str().unwrap_or_default().trim().to_owned();
    SavedTunnel {
        name: text("name"),
        via: text("via"),
        forwards: form["forwards"]
            .as_str()
            .unwrap_or_default()
            .lines()
            .map(|l| l.trim().to_owned())
            .filter(|l| !l.is_empty())
            .collect(),
        autostart: form["autostart"].as_bool() == Some(true),
        identity_file: text("identityFile"),
    }
}

/// `TunnelEditDialog._accept`: the first problem with the form.
fn form_error(form: &J, taken: &[String]) -> Option<String> {
    let t = form_tunnel(form);
    if t.name.is_empty() || taken.contains(&t.name) {
        return Some("Give the tunnel a name not used by another tunnel.".to_owned());
    }
    if t.via.is_empty() {
        return Some("Pick the SSH entry to tunnel through (add one first).".to_owned());
    }
    let lines = Value::Str(form["forwards"].as_str().unwrap_or_default().to_owned());
    parse_forwards(Some(&lines)).err()
}

/// The ssh argv and the environment (the password) of a tunnel.
type Resolved = (Vec<String>, Vec<(String, Secret)>);

enum Resolve {
    Locked,
    Failed(String),
}

/// The ssh argv and environment of a tunnel.
fn resolve_tunnel(cfg: &Value, tunnel: &Value, key: Option<&Secret>) -> Result<Resolved, Resolve> {
    let list = |k: &str| match cfg.get(k) {
        Some(Value::Seq(items)) => items.clone(),
        _ => Vec::new(),
    };
    let creds = list("credentials");
    let via =
        find_gateway(&list("connections"), tunnel.get("via")).map(|v| apply_credential(v, &creds));
    let argv = tunnel_argv(tunnel, via.as_ref(), section(cfg, "launch"), &Env::system())
        .map_err(Resolve::Failed)?;
    let Some(via) = via else {
        return Err(Resolve::Failed("cannot build the ssh command".to_owned()));
    };
    let sealed = via.get("password").is_some_and(vault::is_sealed);
    if sealed && key.is_none() && cfg.get("vault").is_some_and(vault::is_configured) {
        return Err(Resolve::Locked);
    }
    let revealed = vault::reveal_connection(&via, key).map_err(|_| {
        Resolve::Failed("the saved password is locked (master password not entered)".to_owned())
    })?;
    let env = ssh_password_env(&revealed).map_err(|e| Resolve::Failed(e.to_string()))?;
    Ok((argv, env))
}

const LOOK: [(&str, &str, &str); 4] = [
    ("running", "Running", "#27ae60"),
    ("starting", "Starting…", "#f67400"),
    ("failed", "Failed", "#da4453"),
    ("stopped", "Stopped", "#7f8c8d"),
];

/// The rows of the Tunnels window.
fn tunnels_view(saved: &[SavedTunnel], runs: &Runs) -> J {
    let rows: Vec<J> = saved
        .iter()
        .map(|t| {
            let (status, detail) = runs.state(&t.name);
            let (_, text, color) = LOOK
                .iter()
                .find(|(k, _, _)| *k == status)
                .copied()
                .unwrap_or(("", status, "#7f8c8d"));
            json!({
                "name": t.name, "via": t.via, "forwards": t.forwards, "autostart": t.autostart,
                "identityFile": t.identity_file,
                "summary": forwards_summary(&tunnel_value(t)),
                "status": status, "statusText": text, "color": color, "detail": detail,
            })
        })
        .collect();
    J::Array(rows)
}

/// The config's snippets, normalized, as JSON.
fn snippet_list(cfg: &J) -> J {
    let list = normalize_snippets(&cfg["snippets"]);
    J::Array(
        list.iter()
            .map(|s| json!({"name": s.name, "command": s.command, "enter": s.enter}))
            .collect(),
    )
}

/// The placeholders' values for one session: `str(conn.get(f) or "")`, the label falling back to
/// the tab title when the entry has none.
fn snippet_conn(conn: &J, title: &str) -> SnippetConn {
    let conn = Value::from_json(conn);
    let field = |k: &str| match conn.get(k) {
        Some(v) if truthy(v) => value_str(v),
        _ => String::new(),
    };
    SnippetConn {
        host: field("host"),
        user: field("user"),
        label: if conn.get("label").is_some() {
            field("label")
        } else {
            title.to_owned()
        },
        port: field("port"),
    }
}

/// One tunnel's live state.
#[derive(Default)]
struct Run {
    tunnel: Option<Tunnel>,
    status: &'static str,
    detail: String,
}

/// The running tunnels (no Qt).
#[derive(Default)]
pub struct Runs {
    map: HashMap<String, Run>,
}

impl Runs {
    fn state(&self, name: &str) -> (&'static str, &str) {
        self.map
            .get(name)
            .map_or(("stopped", ""), |r| (r.status, r.detail.as_str()))
    }

    fn live(&self, name: &str) -> bool {
        matches!(self.state(name).0, "starting" | "running")
    }

    fn running_count(&self) -> usize {
        self.map
            .values()
            .filter(|r| matches!(r.status, "starting" | "running"))
            .count()
    }

    fn set(&mut self, name: &str, status: &'static str, detail: &str) {
        let run = self.map.entry(name.to_owned()).or_default();
        run.status = status;
        run.detail = detail.to_owned();
    }

    fn fail(&mut self, name: &str, text: &str) {
        self.map.entry(name.to_owned()).or_default().tunnel = None;
        self.set(name, "failed", text);
    }

    fn start(&mut self, name: &str, argv: &[String], env: &[(String, Secret)], settle: Duration) {
        let mut env: BTreeMap<OsString, OsString> = env
            .iter()
            .map(|(k, v)| {
                (
                    OsString::from(k),
                    termigator_core::platform::os_string_from_vec(v.expose().to_vec()),
                )
            })
            .collect();
        let tunnel = Tunnel::start_with_settle(argv, &env, settle);
        vault::wipe_env(&mut env);
        self.set(name, "starting", "connecting…");
        if let Some(run) = self.map.get_mut(name) {
            run.tunnel = Some(tunnel);
        }
    }

    /// Asks the process to end. True when the status changed at once (nothing was running).
    fn stop(&mut self, name: &str) -> bool {
        match self.map.get(name).and_then(|r| r.tunnel.as_ref()) {
            Some(t) => {
                t.stop();
                false
            }
            None if self.state(name).0 != "stopped" => {
                self.set(name, "stopped", "");
                true
            }
            None => false,
        }
    }

    /// Dropping a `Tunnel` stops it.
    fn forget(&mut self, name: &str) {
        self.map.remove(name);
    }

    /// Ends every process and waits for the supervisors (SIGTERM, SIGKILL after 500 ms), so no
    /// `ssh -N` outlives the application. Returns the names whose status changed.
    fn stop_all(&mut self) -> Vec<String> {
        let mut changed = Vec::new();
        for run in self.map.values() {
            if let Some(t) = &run.tunnel {
                t.stop();
            }
        }
        for (name, run) in &mut self.map {
            if let Some(t) = run.tunnel.take() {
                while let Ok(ev) = t.events().recv_timeout(Duration::from_secs(3)) {
                    if matches!(ev, TunnelEvent::Stopped | TunnelEvent::Failed { .. }) {
                        break;
                    }
                }
            }
            if run.status != "stopped" {
                run.status = "stopped";
                run.detail.clear();
                changed.push(name.clone());
            }
        }
        changed
    }

    /// Applies the pending events; the names that changed.
    fn poll(&mut self) -> Vec<String> {
        let mut changed = Vec::new();
        for (name, run) in &mut self.map {
            let Some(t) = &run.tunnel else { continue };
            let events: Vec<TunnelEvent> = t.events().try_iter().collect();
            for ev in events {
                match &ev {
                    TunnelEvent::Starting => {
                        run.status = "starting";
                        run.detail = "connecting…".to_owned();
                    }
                    TunnelEvent::Running => {
                        run.status = "running";
                        run.detail.clear();
                    }
                    TunnelEvent::Failed { .. } => {
                        run.status = "failed";
                        run.detail = ev.detail().unwrap_or_default();
                        run.tunnel = None;
                    }
                    TunnelEvent::Stopped => {
                        run.status = "stopped";
                        run.detail.clear();
                        run.tunnel = None;
                    }
                }
                changed.push(name.clone());
                if run.tunnel.is_none() {
                    break;
                }
            }
        }
        changed
    }
}

#[cfg(test)]
mod tests;
