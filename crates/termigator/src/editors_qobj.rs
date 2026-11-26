//! Connection editors: `EditorBackend`, the Rust half of the SSH / RDP / VNC / Telnet /
//! Serial / Favorite dialogs in `qml/editors/`.
//!
//! The dialogs hold only widgets. Everything the dialogs decide lives here:
//! [`prepare`] is what the fields show for an entry and [`build`] makes the entry or favorite
//! (the form laid over the *original* entry, so keys the dialog does not own survive, and keys
//! come out in insertion order). The form travels as JSON
//! between QML and Rust; the entry as `Value::to_json()`.
//!
//! "Test connection" probes on a worker thread (`probe_connection`) and answers with a signal.

use std::pin::Pin;
use std::time::Duration;

use cxx_qt::Threading;
use cxx_qt_lib::QString;
use serde_json::{Value as J, json};
use termigator_core::config::doc::Value;
use termigator_core::config::environments;
use termigator_core::config::groups::normalize_group_path;
use termigator_core::config::model::{
    PROTOCOL_SSH, connection_protocol, default_terminal_from_cfg, section, strip_space,
    terminal_mode_for_entry, truthy, value_int, value_str,
};
use termigator_core::connectivity::{DEFAULT_TIMEOUT, ProbeReport, probe_connection};
use termigator_core::hosttree::{all_group_names, environment_of};
use termigator_core::proto::rdp::{RDP_DEFAULT_PORT, RDP_DIALOG_MANAGED_KEYS};
use termigator_core::proto::serial::{BAUD_RATES, detect_serial_devices, serial_access_problem};
use termigator_core::proto::ssh::{
    SSH_DIALOG_MANAGED_KEYS, format_proxy_jump_for_ui, forward_specs_from_yaml,
};
use termigator_core::proto::telnet::TELNET_DEFAULT_PORT;
use termigator_core::proto::vnc::VNC_DEFAULT_PORT;
use termigator_core::shell;
use termigator_core::vault::is_sealed;

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
        type EditorBackend = super::EditorBackendRust;

        /// A "Test connection" probe started with `start_test` finished: the `token` it was
        /// started with and the report as JSON (`{ok, summary, via_note, steps: [...]}`).
        #[qsignal]
        fn test_finished(self: Pin<&mut EditorBackend>, token: QString, report_json: QString);

        /// What the dialog of `kind` shows for `entry_json` ("" = a new entry) as JSON:
        /// `{title, form, groups, gateways, credentials, devices}`; "" for an unknown kind.
        #[qinvokable]
        fn prepare(
            self: &EditorBackend,
            kind: &QString,
            entry_json: &QString,
            default_group: &QString,
        ) -> QString;

        /// `connection_dict()` / `favorite_dict()`: the entry to store, as JSON; "" when a
        /// favorite lacks its label or command.
        #[qinvokable]
        fn build(
            self: &EditorBackend,
            kind: &QString,
            entry_json: &QString,
            form_json: &QString,
        ) -> QString;

        /// `window.default_terminal` as "embedded" / "external" (Quick SSH starts on it).
        #[qinvokable]
        fn default_terminal(self: &EditorBackend) -> QString;

        /// The warning under a serial device ("" = none).
        #[qinvokable]
        fn serial_problem(self: &EditorBackend, device: &QString) -> QString;

        /// The warning under the SSH key field when the file looks like a public key or a PuTTY
        /// `.ppk` ("" = fine, empty or not a file).
        #[qinvokable]
        fn key_problem(self: &EditorBackend, path: &QString) -> QString;

        /// Probes the entry in `conn_json` off the GUI thread; answers with `test_finished`.
        #[qinvokable]
        fn start_test(self: &EditorBackend, token: &QString, conn_json: &QString);

        /// The warning under the jump host key field: [`key_problem`] plus "not found".
        #[qinvokable]
        fn key_file_problem(self: &EditorBackend, path: &QString) -> QString;

        /// Whether the key file is protected by a passphrase.
        #[qinvokable]
        fn key_needs_passphrase(self: &EditorBackend, path: &QString) -> bool;

        /// The key path with `~` expanded, as ssh-add and the key list name it.
        #[qinvokable]
        fn key_path(self: &EditorBackend, path: &QString) -> QString;
    }

    impl cxx_qt::Threading for EditorBackend {}
}

#[derive(Default)]
pub struct EditorBackendRust;

impl qobject::EditorBackend {
    fn prepare(&self, kind: &QString, entry_json: &QString, default_group: &QString) -> QString {
        let Some(kind) = Kind::parse(&kind.to_string()) else {
            return QString::default();
        };
        let entry = parse_entry("prepare", &entry_json.to_string());
        let out = with_app(|app| {
            prepare(
                &app.cfg,
                kind,
                entry.as_ref(),
                &default_group.to_string(),
                &detect_serial_devices(),
            )
        });
        QString::from(out.to_string().as_str())
    }

    fn build(&self, kind: &QString, entry_json: &QString, form_json: &QString) -> QString {
        let Some(kind) = Kind::parse(&kind.to_string()) else {
            return QString::default();
        };
        let entry = parse_entry("build", &entry_json.to_string());
        let form: J = crate::qml_json::value("build", &form_json.to_string()).unwrap_or(J::Null);
        build(kind, entry.as_ref(), &form)
            .map(|v| QString::from(v.to_json().to_string().as_str()))
            .unwrap_or_default()
    }

    fn default_terminal(&self) -> QString {
        QString::from(with_app(|app| default_terminal_from_cfg(&app.cfg)))
    }

    fn serial_problem(&self, device: &QString) -> QString {
        QString::from(serial_problem(&device.to_string()).as_str())
    }

    fn key_problem(&self, path: &QString) -> QString {
        QString::from(key_problem(&path.to_string()).as_str())
    }

    fn key_file_problem(&self, path: &QString) -> QString {
        QString::from(key_file_problem(&path.to_string()).as_str())
    }

    fn key_needs_passphrase(&self, path: &QString) -> bool {
        key_locked(&path.to_string())
    }

    fn key_path(&self, path: &QString) -> QString {
        QString::from(key_path(&path.to_string()).as_str())
    }

    fn start_test(&self, token: &QString, conn_json: &QString) {
        let Some(conn) = parse_entry("start_test", &conn_json.to_string()) else {
            return;
        };
        let timeout = with_app(|app| probe_timeout(&app.cfg));
        let token = token.to_string();
        let thread = self.qt_thread();
        // A probe cannot be interrupted (blocking connect), so a closed dialog just never reads
        // the answer; the thread ends by itself.
        std::thread::spawn(move || {
            let report = report_json(&probe_connection(&conn, timeout, true)).to_string();
            let _ = thread.queue(move |c: Pin<&mut qobject::EditorBackend>| {
                c.test_finished(
                    QString::from(token.as_str()),
                    QString::from(report.as_str()),
                );
            });
        });
    }
}

// ---- Pure logic (unit-tested below) ------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Ssh,
    Rdp,
    Vnc,
    Telnet,
    Serial,
    Favorite,
}

impl Kind {
    pub fn parse(name: &str) -> Option<Kind> {
        Some(match name {
            "ssh" => Kind::Ssh,
            "rdp" => Kind::Rdp,
            "vnc" => Kind::Vnc,
            "telnet" => Kind::Telnet,
            "serial" => Kind::Serial,
            "favorite" => Kind::Favorite,
            _ => return None,
        })
    }

    fn noun(self) -> &'static str {
        match self {
            Kind::Ssh => "SSH connection",
            Kind::Rdp => "RDP connection",
            Kind::Vnc => "VNC connection",
            Kind::Telnet => "Telnet connection",
            Kind::Serial => "serial connection",
            Kind::Favorite => "favorite command",
        }
    }
}

/// An empty entry counts as a new one.
fn parse_entry(invokable: &str, json: &str) -> Option<Value> {
    let v = Value::from_json(&crate::qml_json::value(invokable, json)?);
    (matches!(v, Value::Map(_)) && truthy(&v)).then_some(v)
}

/// `launch.probe_timeout`.
fn probe_timeout(cfg: &Value) -> Duration {
    let secs = section(cfg, "launch")
        .get("probe_timeout")
        .filter(|v| truthy(v))
        .and_then(|v| match v {
            Value::Float(f) => Some(*f),
            other => value_int(other).map(|i| i as f64),
        })
        .filter(|s| s.is_finite() && *s > 0.0);
    secs.map_or(DEFAULT_TIMEOUT, Duration::from_secs_f64)
}

pub fn report_json(r: &ProbeReport) -> J {
    json!({
        "ok": r.ok,
        "summary": r.summary,
        "via_note": r.via_note,
        "endpoint": r.endpoint(),
        "steps": r.steps.iter().map(|s| json!({
            "name": s.name, "ok": s.ok, "detail": s.detail, "ms": s.ms,
        })).collect::<Vec<_>>(),
    })
}

/// The warning under the SSH key field: the file looks like a public key or a PuTTY `.ppk`.
pub fn key_problem(path: &str) -> String {
    let conn = Value::Map(vec![(
        Value::Str("identity_file".to_owned()),
        Value::Str(path.to_owned()),
    )]);
    let home = termigator_core::proto::ssh::Env::system().home;
    termigator_core::proto::ssh::identity_warning(&conn, home.as_deref()).unwrap_or_default()
}

/// A key path with `~` expanded ("" when blank).
pub fn key_path(path: &str) -> String {
    let path = path.trim();
    if path.is_empty() {
        return String::new();
    }
    termigator_core::proto::ssh::expand_user(
        path,
        termigator_core::proto::ssh::Env::system().home.as_deref(),
    )
}

/// [`key_problem`] plus "not found", for the keys used without a terminal to read ssh's own
/// error in (the jump host key, a tunnel's key).
pub fn key_file_problem(path: &str) -> String {
    let full = key_path(path);
    if full.is_empty() {
        return String::new();
    }
    if !std::path::Path::new(&full).is_file() {
        return format!("SSH key file not found: {full}");
    }
    key_problem(path.trim())
}

/// Whether the key file is protected by a passphrase.
pub fn key_locked(path: &str) -> bool {
    let full = key_path(path);
    !full.is_empty()
        && termigator_core::proto::ssh_keys::key_needs_passphrase(std::path::Path::new(&full))
}

/// `serial_access_problem` for a device path (`SerialConnectionDialog._update_hint`).
pub fn serial_problem(device: &str) -> String {
    let dev = strip_space(device);
    if dev.starts_with('/') {
        serial_access_problem(dev).unwrap_or_default()
    } else {
        String::new()
    }
}

// -- an insertion-ordered dict -----------------------------------------------------------

/// `dict(initial)`: assignment keeps the position of an existing key, a new or popped-and-set key
/// goes last.
struct Dict(Vec<(Value, Value)>);

impl Dict {
    fn from_entry(entry: Option<&Value>) -> Dict {
        match entry {
            Some(Value::Map(pairs)) => Dict(pairs.clone()),
            _ => Dict(Vec::new()),
        }
    }

    fn pop(&mut self, key: &str) {
        self.0
            .retain(|(k, _)| !matches!(k, Value::Str(s) if s == key));
    }

    fn set(&mut self, key: &str, value: Value) {
        match self
            .0
            .iter_mut()
            .find(|(k, _)| matches!(k, Value::Str(s) if s == key))
        {
            Some(slot) => slot.1 = value,
            None => self.0.push((Value::Str(key.to_owned()), value)),
        }
    }

    fn set_str(&mut self, key: &str, value: &str) {
        self.set(key, Value::Str(value.to_owned()));
    }

    fn pop_managed(&mut self, keys: &[&str], keep: &[&str]) {
        for k in keys.iter().filter(|k| !keep.contains(k)) {
            self.pop(k);
        }
    }

    fn into_value(self) -> Value {
        Value::Map(self.0)
    }
}

// -- reading the entry (the dialogs' `__init__`) -----------------------------------------------

/// `str(entry.get(key) or "")`.
fn sx(entry: Option<&Value>, key: &str) -> String {
    entry
        .and_then(|e| e.get(key))
        .filter(|v| truthy(v))
        .map(value_str)
        .unwrap_or_default()
}

/// `entry.get(key) is True`.
fn is_true(entry: Option<&Value>, key: &str) -> bool {
    matches!(entry.and_then(|e| e.get(key)), Some(Value::Bool(true)))
}

/// The port field: a stored port that `int()` accepts, clamped like a QSpinBox, else `default`.
fn port_of(entry: Option<&Value>, default: i64) -> i64 {
    let Some(p) = entry
        .and_then(|e| e.get("port"))
        .filter(|p| !matches!(p, Value::Null) && !strip_space(&value_str(p)).is_empty())
    else {
        return default;
    };
    value_int(p).map_or(default, |v| v.clamp(1, 65535))
}

/// Extra arguments: a list is shown shell-joined, anything else as its text.
fn args_text(entry: Option<&Value>, key: &str) -> String {
    match entry.and_then(|e| e.get(key)) {
        None | Some(Value::Null) => String::new(),
        Some(Value::Seq(items)) => shell::join(&items.iter().map(value_str).collect::<Vec<_>>()),
        Some(v) => value_str(v),
    }
}

/// The folder shown: the entry's own (stripped) or the folder the editor was opened for.
fn group_of_entry(entry: Option<&Value>, default_group: &str) -> String {
    let own = strip_space(&sx(entry, "group")).to_owned();
    if own.is_empty() {
        default_group.to_owned()
    } else {
        own
    }
}

/// `-J` text, tags and the other SSH-only fields.
fn dynamic_text(entry: Option<&Value>) -> String {
    match entry.and_then(|e| e.get("dynamic_forward")) {
        None | Some(Value::Null | Value::Bool(_)) => String::new(),
        Some(v) => {
            let text = value_str(v);
            let t = strip_space(&text);
            if t.is_empty() || matches!(t.to_lowercase().as_str(), "none" | "false" | "no") {
                String::new()
            } else {
                t.to_owned()
            }
        }
    }
}

fn tags_text(entry: Option<&Value>) -> String {
    match entry.and_then(|e| e.get("tags")) {
        Some(Value::Seq(items)) => items
            .iter()
            .map(value_str)
            .filter(|t| !strip_space(t).is_empty())
            .collect::<Vec<_>>()
            .join(", "),
        Some(v) if truthy(v) => value_str(v),
        _ => String::new(),
    }
}

/// The select the stored value maps to: "" (default), "on" or "off".
fn keepalive_of(entry: Option<&Value>) -> &'static str {
    match entry.and_then(|e| e.get("ssh_keepalive")) {
        Some(Value::Bool(true)) => "on",
        Some(Value::Bool(false)) => "off",
        _ => "",
    }
}

fn strict_host_of(entry: Option<&Value>) -> String {
    match entry.and_then(|e| e.get("strict_host_key_checking")) {
        None | Some(Value::Null) => String::new(),
        Some(v) => {
            let t = strip_space(&value_str(v)).to_owned();
            if STRICT_HOST.contains(&t.as_str()) {
                t
            } else {
                String::new()
            }
        }
    }
}

const STRICT_HOST: [&str; 5] = ["accept-new", "yes", "no", "ask", "off"];

/// What each dialog shows for `entry` (None = a new entry), plus the lists its combos offer.
pub fn prepare(
    cfg: &Value,
    kind: Kind,
    entry: Option<&Value>,
    default_group: &str,
    devices: &[String],
) -> J {
    let e = entry;
    let terminal = terminal_mode_for_entry(e, cfg);
    let group = group_of_entry(e, default_group);
    let sealed = e.and_then(|x| x.get("password")).is_some_and(is_sealed);
    // A typed password is stored in plain text without a master password: the dialog says so.
    let vault_on = cfg
        .get("vault")
        .is_some_and(termigator_core::vault::is_configured);
    let common = |port_default: i64| {
        json!({
            "label": sx(e, "label"), "host": sx(e, "host"), "user": sx(e, "user"),
            "port": port_of(e, port_default), "group": group, "terminal": terminal,
        })
    };
    let form = match kind {
        Kind::Ssh => {
            let mut f = common(22);
            merge(
                &mut f,
                json!({
                    "password": password_text(e), "passwordSealed": sealed, "vaultOn": vault_on,
                    "credential": sx(e, "credential"),
                    "identityFile": sx(e, "identity_file"), "extra": args_text(e, "extra_ssh_args"),
                    "tags": tags_text(e), "useMosh": is_true(e, "use_mosh"),
                    "autoReconnect": is_true(e, "auto_reconnect"),
                    "proxyJump": e.map(format_proxy_jump_for_ui).unwrap_or_default(),
                    "jumpIdentityFile": sx(e, "jump_identity_file"),
                    "forwardAgent": is_true(e, "forward_agent"),
                    "x11": if is_true(e, "x11_forwarding") {
                        if is_true(e, "x11_trusted") { "y" } else { "x" }
                    } else { "" },
                    "localFwd": forward_specs_from_yaml(e.and_then(|x| x.get("local_forwards"))).join("\n"),
                    "remoteFwd": forward_specs_from_yaml(e.and_then(|x| x.get("remote_forwards"))).join("\n"),
                    "dynamic": dynamic_text(e), "remoteCmd": sx(e, "remote_command"),
                    "strictHost": strict_host_of(e),
                    "filesOnly": is_true(e, "files_only"), "compression": is_true(e, "compression"),
                    "requestTty": is_true(e, "request_tty"), "keepalive": keepalive_of(e),
                    "locale": sx(e, "remote_locale"), "fontFamily": sx(e, "font_family"),
                    "fontSize": font_size_of(e), "colorScheme": sx(e, "color_scheme"),
                    "startup": sx(e, "startup_commands"),
                }),
            );
            f
        }
        Kind::Rdp => {
            let mut f = common(RDP_DEFAULT_PORT);
            merge(
                &mut f,
                json!({
                    "password": password_text(e), "passwordSealed": sealed, "vaultOn": vault_on,
                    "credential": sx(e, "credential"), "domain": sx(e, "domain"),
                    "sshGateway": sx(e, "ssh_gateway"),
                    "clipboard": !matches!(e.and_then(|x| x.get("clipboard")), Some(Value::Bool(false))),
                    "sound": is_true(e, "sound"), "microphone": is_true(e, "microphone"),
                    "homeDrive": is_true(e, "home_drive"), "multimon": is_true(e, "multimon"),
                    "dynamicRes": is_true(e, "dynamic_resolution"),
                    "folders": shared_folders_text(e), "gateway": sx(e, "gateway"),
                    "gwUser": sx(e, "gateway_user"), "gwDomain": sx(e, "gateway_domain"),
                    "fullscreen": is_true(e, "fullscreen"),
                    "resolution": strip_space(&sx(e, "resolution")),
                    "extra": args_text(e, "extra_rdp_args"),
                }),
            );
            f
        }
        Kind::Vnc => {
            let mut f = common(VNC_DEFAULT_PORT);
            merge(
                &mut f,
                json!({"sshGateway": sx(e, "ssh_gateway"), "extra": args_text(e, "extra_vnc_args")}),
            );
            f
        }
        Kind::Telnet => {
            // Telnet reads `int(port or 23)` rather than the guarded spin-box load.
            let port = e
                .and_then(|x| x.get("port"))
                .filter(|p| truthy(p))
                .and_then(value_int)
                .map_or(TELNET_DEFAULT_PORT, |v| v.clamp(1, 65535));
            json!({
                "label": sx(e, "label"), "host": sx(e, "host"), "port": port,
                "user": sx(e, "user"), "group": group, "terminal": terminal,
            })
        }
        Kind::Serial => {
            let device = {
                let own = sx(e, "device");
                if own.is_empty() {
                    devices.first().cloned().unwrap_or_default()
                } else {
                    own
                }
            };
            let or = |key: &str, default: &str| {
                let v = sx(e, key);
                if v.is_empty() { default.to_owned() } else { v }
            };
            // A fixed list ignores a value it does not offer and stays on its first item.
            let pick = |key: &str, items: &[&str]| {
                let v = sx(e, key);
                items
                    .iter()
                    .find(|i| **i == v)
                    .unwrap_or(&items[0])
                    .to_string()
            };
            json!({
                "label": sx(e, "label"), "device": device, "baud": or("baud", "115200"),
                "dataBits": pick("data_bits", &["8", "7", "6", "5"]),
                "parity": pick("parity", &["none", "even", "odd"]),
                "stopBits": pick("stop_bits", &["1", "2"]),
                "flow": pick("flow", &["none", "hardware", "software"]),
                "group": group, "terminal": terminal,
            })
        }
        Kind::Favorite => json!({
            "label": sx(e, "label"), "command": sx(e, "command"), "group": group,
            "terminal": terminal,
        }),
    };
    let mut form = form;
    if let Some(e) = entry {
        let own = environment_of(e);
        let own = environments::canonical_id(cfg, &own).unwrap_or(own);
        merge(&mut form, json!({ "environment": own }));
    }
    json!({
        "title": format!("{} {}", if entry.is_some() { "Edit" } else { "Add" }, kind.noun()),
        "isNew": entry.is_none(),
        "form": form,
        "groups": group_choices(cfg),
        "gateways": ssh_entry_labels(cfg),
        "credentials": credential_choices(cfg),
        "devices": devices,
        "baudRates": BAUD_RATES,
        "environments": environments::to_json(&environments::list(cfg)),
    })
}

fn merge(into: &mut J, more: J) {
    if let (J::Object(a), J::Object(b)) = (into, more) {
        a.extend(b);
    }
}

/// `_fill_password_field`: an encrypted token never goes into the field.
fn password_text(entry: Option<&Value>) -> String {
    match entry.and_then(|e| e.get("password")) {
        Some(v) if is_sealed(v) => String::new(),
        _ => sx(entry, "password"),
    }
}

fn font_size_of(entry: Option<&Value>) -> i64 {
    entry
        .and_then(|e| e.get("font_size"))
        .filter(|v| truthy(v))
        .and_then(value_int)
        .map_or(0, |v| v.clamp(0, 72))
}

fn shared_folders_text(entry: Option<&Value>) -> String {
    match entry.and_then(|e| e.get("shared_folders")) {
        Some(Value::Seq(items)) => items.iter().map(value_str).collect::<Vec<_>>().join("\n"),
        _ => sx(entry, "shared_folders"),
    }
}

/// `_make_group_combo`: the typed-or-picked folder choices, without blanks and repeats.
fn group_choices(cfg: &Value) -> Vec<String> {
    let mut seen: Vec<String> = Vec::new();
    for g in all_group_names(cfg) {
        let g = strip_space(&g).to_owned();
        if !g.is_empty() && !seen.contains(&g) {
            seen.push(g);
        }
    }
    seen
}

/// `_ssh_entry_labels`: labels of SSH entries, the choices for an SSH gateway.
pub(crate) fn ssh_entry_labels(cfg: &Value) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    if let Some(Value::Seq(conns)) = cfg.get("connections") {
        for c in conns {
            if !matches!(c, Value::Map(_)) || connection_protocol(c) != PROTOCOL_SSH {
                continue;
            }
            let label = strip_space(&sx(Some(c), "label")).to_owned();
            if !label.is_empty() && !out.contains(&label) {
                out.push(label);
            }
        }
    }
    out
}

/// The credential combo's choices: `[{name, label, user, domain}]` (named dicts only).
fn credential_choices(cfg: &Value) -> Vec<J> {
    let Some(Value::Seq(items)) = cfg.get("credentials") else {
        return Vec::new();
    };
    items
        .iter()
        .filter(|c| matches!(c, Value::Map(_)) && c.get("name").is_some_and(truthy))
        .map(|c| {
            let name = sx(Some(c), "name");
            let user = sx(Some(c), "user");
            let shown = if user.is_empty() {
                "no user"
            } else {
                user.as_str()
            };
            json!({
                "name": name, "label": format!("{name}  ({shown})"),
                "user": user, "domain": sx(Some(c), "domain"),
            })
        })
        .collect()
}

// -- the form (`connection_dict`) --------------------------------------------------------------

fn fs(form: &J, key: &str) -> String {
    match form.get(key) {
        Some(J::String(s)) => s.clone(),
        Some(J::Number(n)) => n.to_string(),
        _ => String::new(),
    }
}

fn ft(form: &J, key: &str) -> String {
    strip_space(&fs(form, key)).to_owned()
}

fn fb(form: &J, key: &str) -> bool {
    form.get(key).and_then(J::as_bool).unwrap_or(false)
}

fn fi(form: &J, key: &str, default: i64) -> i64 {
    form.get(key).and_then(J::as_i64).unwrap_or(default)
}

/// `[ln.strip() for ln in text.splitlines() if ln.strip()]`.
fn lines_of(text: &str) -> Vec<Value> {
    text.split(['\n', '\r'])
        .map(strip_space)
        .filter(|l| !l.is_empty())
        .map(|l| Value::Str(l.to_owned()))
        .collect()
}

/// `shlex.split`, or the raw text when it has an unbalanced quote.
fn split_args(raw: &str) -> Value {
    match shell::split(raw) {
        Ok(parts) => Value::Seq(parts.into_iter().map(Value::Str).collect()),
        Err(_) => Value::Str(raw.to_owned()),
    }
}

/// `int(text)`, or the text itself when it is not an integer.
fn int_or_text(text: String) -> Value {
    match value_int(&Value::Str(text.clone())) {
        Some(i) => Value::Int(i),
        None => Value::Str(text),
    }
}

/// Folder (canonical path), terminal; shared by every dialog.
fn store_group(out: &mut Dict, form: &J) {
    let group = normalize_group_path(&fs(form, "group"));
    if group.is_empty() {
        out.pop("group");
    } else {
        out.set_str("group", &group);
    }
    // Only a dialog with the Environment field sends it; the others keep the entry's.
    if form.get("environment").is_some() {
        let env = fs(form, "environment");
        if env.is_empty() {
            out.pop("environment");
        } else {
            out.set_str("environment", &env);
        }
    }
}

fn store_port(out: &mut Dict, form: &J, default: i64) {
    let port = fi(form, "port", default);
    out.set(
        "port",
        if port == default {
            Value::Null
        } else {
            Value::Int(port)
        },
    );
}

/// The password to store, or None to drop it. A typed one wins; an encrypted one stays while
/// the field is empty unless "Remove the saved password" is ticked.
fn resolve_password(form: &J, initial: Option<&Value>) -> Option<String> {
    let typed = fs(form, "password");
    if !typed.is_empty() {
        return Some(typed);
    }
    let stored = initial.and_then(|e| e.get("password"));
    match stored {
        Some(v) if is_sealed(v) && !fb(form, "forgetPassword") => Some(value_str(v)),
        _ => None,
    }
}

/// `credential` and `password` after everything else.
fn store_login(out: &mut Dict, form: &J, initial: Option<&Value>) {
    let pwd = resolve_password(form, initial);
    let credential = fs(form, "credential");
    if !credential.is_empty() {
        out.set_str("credential", &credential);
        out.pop("password");
    } else {
        out.pop("credential");
        match pwd {
            Some(p) if !p.is_empty() => out.set_str("password", &p),
            _ => out.pop("password"),
        }
    }
}

/// The entry the dialog of `kind` stores for `form`, laid over `initial`; None only for a favorite
/// without its label or command.
pub fn build(kind: Kind, initial: Option<&Value>, form: &J) -> Option<Value> {
    let mut out = Dict::from_entry(initial);
    match kind {
        Kind::Ssh => build_ssh(&mut out, initial, form),
        Kind::Rdp => build_rdp(&mut out, initial, form),
        Kind::Vnc => {
            out.pop_managed(
                &[
                    "label",
                    "protocol",
                    "host",
                    "port",
                    "extra_vnc_args",
                    "terminal",
                    "group",
                    "password",
                    "ssh_gateway",
                ],
                &[],
            );
            out.set_str("protocol", "vnc");
            out.set_str("label", &ft(form, "label"));
            out.set_str("host", &ft(form, "host"));
            store_port(&mut out, form, VNC_DEFAULT_PORT);
            let extra = ft(form, "extra");
            if !extra.is_empty() {
                out.set("extra_vnc_args", split_args(&extra));
            }
            store_group(&mut out, form);
            out.set_str("terminal", &fs(form, "terminal"));
            if !fs(form, "sshGateway").is_empty() {
                out.set_str("ssh_gateway", &fs(form, "sshGateway"));
            }
        }
        Kind::Telnet => {
            out.pop_managed(
                &[
                    "label", "protocol", "host", "port", "user", "terminal", "group", "password",
                ],
                &[],
            );
            out.set_str("protocol", "telnet");
            out.set_str("host", &ft(form, "host"));
            let port = fi(form, "port", TELNET_DEFAULT_PORT);
            if port != TELNET_DEFAULT_PORT {
                out.set("port", Value::Int(port));
            }
            if !ft(form, "user").is_empty() {
                out.set_str("user", &ft(form, "user"));
            }
            store_common(&mut out, form);
        }
        Kind::Serial => {
            out.pop_managed(
                &[
                    "label",
                    "protocol",
                    "device",
                    "baud",
                    "data_bits",
                    "parity",
                    "stop_bits",
                    "flow",
                    "terminal",
                    "group",
                    "host",
                ],
                &[],
            );
            out.set_str("protocol", "serial");
            out.set_str("device", &ft(form, "device"));
            let baud = ft(form, "baud");
            out.set("baud", int_or_text(baud));
            for (key, field, default) in [
                ("data_bits", "dataBits", "8"),
                ("stop_bits", "stopBits", "1"),
            ] {
                let v = fs(form, field);
                if v != default {
                    out.set(key, int_or_text(v));
                }
            }
            for (key, field) in [("parity", "parity"), ("flow", "flow")] {
                let v = fs(form, field);
                if v != "none" {
                    out.set_str(key, &v);
                }
            }
            store_common(&mut out, form);
        }
        Kind::Favorite => {
            let (label, command) = (ft(form, "label"), ft(form, "command"));
            if label.is_empty() || command.is_empty() {
                return None;
            }
            // A favorite is built fresh: nothing of the old entry is kept.
            let mut fav = Dict(Vec::new());
            fav.set_str("label", &label);
            fav.set_str("command", &command);
            fav.set_str("terminal", &fs(form, "terminal"));
            let group = normalize_group_path(&fs(form, "group"));
            if !group.is_empty() {
                fav.set_str("group", &group);
            }
            return Some(fav.into_value());
        }
    }
    Some(out.into_value())
}

/// `_store_common`: label, folder, terminal.
fn store_common(out: &mut Dict, form: &J) {
    out.set_str("label", &ft(form, "label"));
    store_group(out, form);
    out.set_str("terminal", &fs(form, "terminal"));
}

fn build_ssh(out: &mut Dict, initial: Option<&Value>, form: &J) {
    out.pop_managed(SSH_DIALOG_MANAGED_KEYS, &["password", "identity_file"]);
    out.set_str("label", &ft(form, "label"));
    out.set_str("host", &ft(form, "host"));
    out.set_str("user", &ft(form, "user"));
    store_port(out, form, 22);
    let extra = ft(form, "extra");
    if !extra.is_empty() {
        out.set("extra_ssh_args", split_args(&extra));
    }
    out.set_str("terminal", &fs(form, "terminal"));
    store_group(out, form);
    let local = lines_of(&fs(form, "localFwd"));
    if !local.is_empty() {
        out.set("local_forwards", Value::Seq(local));
    }
    let remote = lines_of(&fs(form, "remoteFwd"));
    if !remote.is_empty() {
        out.set("remote_forwards", Value::Seq(remote));
    }
    if !ft(form, "proxyJump").is_empty() {
        out.set_str("proxy_jump", &ft(form, "proxyJump"));
    }
    if !ft(form, "jumpIdentityFile").is_empty() {
        out.set_str("jump_identity_file", &ft(form, "jumpIdentityFile"));
    }
    if fb(form, "forwardAgent") {
        out.set("forward_agent", Value::Bool(true));
    }
    let x11 = fs(form, "x11");
    if !x11.is_empty() {
        out.set("x11_forwarding", Value::Bool(true));
        if x11 == "y" {
            out.set("x11_trusted", Value::Bool(true));
        }
    }
    for (key, field) in [
        ("files_only", "filesOnly"),
        ("compression", "compression"),
        ("request_tty", "requestTty"),
    ] {
        if fb(form, field) {
            out.set(key, Value::Bool(true));
        }
    }
    match fs(form, "keepalive").as_str() {
        "on" => out.set("ssh_keepalive", Value::Bool(true)),
        "off" => out.set("ssh_keepalive", Value::Bool(false)),
        _ => {}
    }
    for (key, field) in [
        ("remote_locale", "locale"),
        ("font_family", "fontFamily"),
        ("color_scheme", "colorScheme"),
        ("startup_commands", "startup"),
    ] {
        let v = ft(form, field);
        if !v.is_empty() {
            out.set_str(key, &v);
        }
    }
    let size = fi(form, "fontSize", 0);
    if size != 0 {
        out.set("font_size", Value::Int(size));
    }
    let dynamic = ft(form, "dynamic");
    if !dynamic.is_empty()
        && !matches!(
            dynamic.to_lowercase().as_str(),
            "off" | "none" | "no" | "false" | "0"
        )
    {
        let digits = dynamic.chars().all(|c| c.is_ascii_digit());
        out.set(
            "dynamic_forward",
            dynamic
                .parse::<i64>()
                .ok()
                .filter(|_| digits)
                .map_or_else(|| Value::Str(dynamic.clone()), Value::Int),
        );
    }
    if !ft(form, "remoteCmd").is_empty() {
        out.set_str("remote_command", &ft(form, "remoteCmd"));
    }
    if !ft(form, "strictHost").is_empty() {
        out.set_str("strict_host_key_checking", &ft(form, "strictHost"));
    }
    if fb(form, "useMosh") {
        out.set("use_mosh", Value::Bool(true));
    }
    if fb(form, "autoReconnect") {
        out.set("auto_reconnect", Value::Bool(true));
    }
    let tags: Vec<Value> = fs(form, "tags")
        .split(',')
        .map(strip_space)
        .filter(|t| !t.is_empty())
        .map(|t| Value::Str(t.to_owned()))
        .collect();
    if !tags.is_empty() {
        out.set("tags", Value::Seq(tags));
    }
    // The field is prefilled with the stored key, so clearing it removes the key.
    let key = ft(form, "identityFile");
    if key.is_empty() {
        out.pop("identity_file");
    } else {
        out.set_str("identity_file", &key);
    }
    store_login(out, form, initial);
}

fn build_rdp(out: &mut Dict, initial: Option<&Value>, form: &J) {
    out.pop_managed(RDP_DIALOG_MANAGED_KEYS, &["password"]);
    out.set_str("protocol", "rdp");
    out.set_str("label", &ft(form, "label"));
    out.set_str("host", &ft(form, "host"));
    if !ft(form, "user").is_empty() {
        out.set_str("user", &ft(form, "user"));
    }
    if !ft(form, "domain").is_empty() {
        out.set_str("domain", &ft(form, "domain"));
    }
    store_port(out, form, RDP_DEFAULT_PORT);
    if fb(form, "fullscreen") {
        out.set("fullscreen", Value::Bool(true));
    } else if !ft(form, "resolution").is_empty() {
        out.set_str("resolution", &ft(form, "resolution"));
    }
    let extra = ft(form, "extra");
    if !extra.is_empty() {
        out.set("extra_rdp_args", split_args(&extra));
    }
    store_group(out, form);
    out.set_str("terminal", &fs(form, "terminal"));
    if !fs(form, "sshGateway").is_empty() {
        out.set_str("ssh_gateway", &fs(form, "sshGateway"));
    }
    // The clipboard is on unless switched off — only "off" is worth storing.
    if !fb(form, "clipboard") {
        out.set("clipboard", Value::Bool(false));
    }
    for (key, field) in [
        ("sound", "sound"),
        ("microphone", "microphone"),
        ("home_drive", "homeDrive"),
        ("multimon", "multimon"),
        ("dynamic_resolution", "dynamicRes"),
    ] {
        if fb(form, field) {
            out.set(key, Value::Bool(true));
        }
    }
    let folders = lines_of(&fs(form, "folders"));
    if !folders.is_empty() {
        out.set("shared_folders", Value::Seq(folders));
    }
    for (key, field) in [
        ("gateway", "gateway"),
        ("gateway_user", "gwUser"),
        ("gateway_domain", "gwDomain"),
    ] {
        if !ft(form, field).is_empty() {
            out.set_str(key, &ft(form, field));
        }
    }
    store_login(out, form, initial);
}

#[cfg(test)]
mod tests;
