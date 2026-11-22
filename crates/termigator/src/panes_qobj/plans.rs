use super::*;

/// `shutil.which` with the process's `PATH`.
pub(super) fn path_which(cmd: &str) -> Option<String> {
    which(cmd, std::env::var("PATH").ok().as_deref())
}

pub fn is_external(mode: &str) -> bool {
    normalize_terminal_mode(Some(&Value::Str(mode.to_owned()))) == TERMINAL_EXTERNAL
}

/// A picked shell (an id of `shells`, or a path run as `<path> -l`) is
/// titled by its name, else the configured default shell runs as "Local shell"; both in the
/// default terminal mode.
pub fn local_shell(
    cfg: &Value,
    shell_path: &str,
    shell_env: &str,
    which: &dyn Fn(&str) -> Option<String>,
    shells: &[LocalShell],
) -> (String, String, &'static str) {
    let (inner, title) = if shell_path.is_empty() {
        (
            local_shell::default_inner(cfg, shell_env, which),
            "Local shell".to_owned(),
        )
    } else if let Some((s, inner)) = shells
        .iter()
        .find(|s| s.id == shell_path)
        .and_then(|s| Some((s, s.inner.clone()?)))
    {
        (inner, s.label.clone())
    } else {
        let name = shell_path.rsplit('/').next().unwrap_or_default();
        (format!("{} -l", quote(shell_path)), name.to_owned())
    };
    (inner, title, default_terminal_from_cfg(cfg))
}

/// The shell menu's entries as JSON: `[{id, label, available}]` (Windows lists missing shells too).
pub fn local_shells_json(shells: &[LocalShell]) -> String {
    let list: Vec<J> = shells
        .iter()
        .map(|s| json!({"id": s.id, "label": s.label, "available": s.available()}))
        .collect();
    J::Array(list).to_string()
}

/// The extra environment of a spawn: a JSON object of strings (anything else is ignored).
pub(super) fn env_map(invokable: &str, json: &str) -> BTreeMap<OsString, OsString> {
    match crate::qml_json::value(invokable, json) {
        Some(J::Object(m)) => m
            .into_iter()
            .filter_map(|(k, v)| Some((k.into(), v.as_str()?.to_owned().into())))
            .collect(),
        _ => BTreeMap::new(),
    }
}

/// `{"<tab id>": {"title", "conn", "inner"}}` from QML; malformed entries are skipped.
pub fn parse_records(invokable: &str, json: &str) -> HashMap<TabId, TabRecord> {
    let Some(J::Object(map)) = crate::qml_json::value(invokable, json) else {
        return HashMap::new();
    };
    let text = |v: &J, k: &str| v.get(k).and_then(J::as_str).unwrap_or("").to_owned();
    map.iter()
        .filter_map(|(id, v)| {
            let conn = v
                .get("conn")
                .filter(|c| c.is_object())
                .map(Value::from_json);
            Some((
                id.parse().ok()?,
                TabRecord {
                    title: text(v, "title"),
                    conn,
                    inner: text(v, "inner"),
                },
            ))
        })
        .collect()
}

/// `cfg["last_session"] = value` keeping the key's place, or removing it for `None`.
fn set_last_session(cfg: &mut Value, value: Option<Value>) {
    let Value::Map(pairs) = cfg else { return };
    let key = Value::Str("last_session".to_owned());
    let at = pairs.iter().position(|(k, _)| *k == key);
    match (at, value) {
        (Some(i), Some(v)) => pairs[i].1 = v,
        (None, Some(v)) => pairs.push((key, v)),
        (Some(i), None) => {
            pairs.remove(i);
        }
        (None, None) => {}
    }
}

/// On close: the snapshot when `window.session_restore`, else no `last_session`.
pub fn save_snapshot(cfg: &mut Value, panes: &Panes, records: &HashMap<TabId, TabRecord>) {
    let snap =
        crate::shell_qobj::keep_session(cfg).then(|| panes.snapshot(&|t| records.get(&t).cloned()));
    set_last_session(cfg, snap);
}

/// Session restore: only with `window.session_restore` and a mapping.
/// `forced`: the restart after a style change reopens the tabs either way.
pub fn restore_session(cfg: &Value, forced: bool) -> Option<termigator_core::panes::Restored> {
    let snap = cfg.get("last_session")?;
    if !(forced || window_bool(cfg, "session_restore", false)) || !matches!(snap, Value::Map(_)) {
        return None;
    }
    let connections = match cfg.get("connections") {
        Some(Value::Seq(c)) => c.as_slice(),
        _ => &[],
    };
    Some(restore(snap, connections))
}

pub(super) fn parse_entry(invokable: &str, json: &QString) -> Option<Value> {
    crate::qml_json::value(invokable, &json.to_string())
        .filter(J::is_object)
        .map(|j| Value::from_json(&j))
}

fn seq<'a>(cfg: &'a Value, key: &str) -> &'a [Value] {
    match cfg.get(key) {
        Some(Value::Seq(s)) => s,
        _ => &[],
    }
}

/// `str(conn.get(key) or "")`.
fn text(c: &Value, key: &str) -> String {
    c.get(key)
        .filter(|v| truthy(v))
        .map(value_str)
        .unwrap_or_default()
}

/// The entry with its named credential's user and password.
fn with_credential(cfg: &Value, c: &Value) -> Value {
    apply_credential(c, seq(cfg, "credentials"))
}

/// A warning instead of opening. `locked`: the vault must be unlocked first (the unlock prompt).
#[derive(Debug, Clone, PartialEq)]
pub struct Stop {
    pub text: String,
    pub locked: bool,
}

impl From<VaultError> for Stop {
    fn from(e: VaultError) -> Stop {
        Stop {
            text: e.to_string(),
            locked: false,
        }
    }
}

/// What opening an entry needs from the user or gives to `_open_inner`.
#[derive(Debug, Clone, PartialEq)]
pub enum Plan {
    /// A file-browser-only SSH entry opens the Files tab.
    Files,
    Stop(Stop),
    Open {
        inner: String,
        title: String,
        mode: &'static str,
        /// `_open_inner`'s `err`: shown instead of opening.
        error: String,
        env: Vec<(String, Secret)>,
        /// Shown, then the tab opens anyway (X11 without a display).
        notice: Option<String>,
        /// The tab keeps the entry (`conn=c`).
        conn: bool,
        /// Distro detection (SSH entries from the list).
        distro: bool,
    },
}

/// Reveals an entry without the unlock prompt: the entry with its credential applied
/// and its password decrypted with `key`.
fn reveal(cfg: &Value, c: &Value, key: Option<&Secret>) -> Result<Revealed<'static>, Stop> {
    let c = with_credential(cfg, c);
    let locked = key.is_none() && cfg.get("vault").is_some_and(vault::is_configured);
    vault::reveal_connection(&c, key)
        .map(Revealed::into_static)
        .map_err(|e| Stop {
            text: format!("Cannot decrypt the saved password:\n{e}"),
            locked,
        })
}

/// `openConnection(entry)`: a favorite (the only entries with a
/// `command`) or a connection. `key`: the vault's session key; `display`: `$DISPLAY`; `screen`: `WxH` of the window's display
/// (the RDP start size).
pub fn connection_plan(
    cfg: &Value,
    c: &Value,
    key: Option<&Secret>,
    display: Option<&str>,
    screen: Option<&str>,
) -> Plan {
    if c.get("command").is_some() {
        let title = c
            .get("label")
            .filter(|v| truthy(v))
            .map_or_else(|| "Favorite".to_owned(), value_str);
        return Plan::Open {
            inner: strip_space(&text(c, "command")).to_owned(),
            title,
            mode: terminal_mode_for_entry(Some(c), cfg),
            error: String::new(),
            env: Vec::new(),
            notice: None,
            conn: false,
            distro: false,
        };
    }
    let proto = connection_protocol(c);
    if proto == PROTOCOL_SSH && c.get("files_only") == Some(&Value::Bool(true)) {
        return Plan::Files;
    }
    open_connection(cfg, c, proto, key, display, screen).unwrap_or_else(Plan::Stop)
}

fn open_connection(
    cfg: &Value,
    c: &Value,
    proto: &'static str,
    key: Option<&Secret>,
    display: Option<&str>,
    screen: Option<&str>,
) -> Result<Plan, Stop> {
    // The program-wide RDP start size (Settings → Connections), unless the host has its own.
    let sized;
    let c = if proto == PROTOCOL_RDP {
        sized = termigator_core::proto::rdp::with_default_geometry(
            c,
            cfg.get("launch").unwrap_or(&Value::Null),
            screen,
        );
        &sized
    } else {
        c
    };
    let revealed = reveal(cfg, c, key)?;
    // Windows has no FreeRDP: mstsc opens an .rdp file with the password DPAPI-encrypted in it.
    let built = if cfg!(windows) && proto == PROTOCOL_RDP {
        crate::rdp_windows::mstsc_command(&revealed)
    } else {
        build_conn_command(cfg, c)
    };
    if proto == PROTOCOL_SERIAL
        && built.is_ok()
        && let Some(problem) = serial_access_problem(&text(c, "device"))
    {
        return Err(Stop {
            text: problem,
            locked: false,
        });
    }
    let mut env = Vec::new();
    let mut notice = None;
    if proto == PROTOCOL_SSH {
        env = secret_pairs(ssh_session_env(c));
        env.extend(ssh_password_env(&revealed)?);
        if built.is_ok() {
            let key = termigator_core::proto::ssh::identity_warning(
                c,
                termigator_core::proto::ssh::Env::system().home.as_deref(),
            );
            notice = [x11_display_warning(c, display), key]
                .into_iter()
                .flatten()
                .reduce(|a, b| format!("{a}\n{b}"));
        }
    } else if proto == PROTOCOL_RDP && !cfg!(windows) {
        env.extend(rdp_password_env(&revealed)?);
    }
    let via_gw = (proto == PROTOCOL_RDP || proto == PROTOCOL_VNC)
        && c.get("ssh_gateway").is_some_and(truthy);
    if via_gw
        && built.is_ok()
        && let Some(gw) = gateway::find_gateway(seq(cfg, "connections"), c.get("ssh_gateway"))
    {
        // The gateway hop's password (sshpass reads $SSHPASS).
        let gw = reveal(cfg, gw, key)?;
        env.extend(ssh_password_env(&gw)?);
    }
    let (inner, error) = match built {
        Ok(cmd) => (cmd, String::new()),
        Err(e) => (String::new(), e),
    };
    Ok(Plan::Open {
        inner,
        title: entry_label(c),
        mode: terminal_mode_for_entry(Some(c), cfg),
        error,
        env,
        notice,
        conn: true,
        distro: proto == PROTOCOL_SSH,
    })
}

/// Non-secret pairs in the type of the secret ones, for one env list.
fn secret_pairs(env: Vec<(String, String)>) -> Vec<(String, Secret)> {
    env.into_iter()
        .map(|(k, v)| (k, Secret::new(v.into_bytes())))
        .collect()
}

/// `{"KEY": "value"}`; a later key wins.
/// shortcut: the password leaves zeroizing memory here (JSON, QString, QML), upgrade if QML
/// stops carrying the env and the spawn takes it from Rust directly.
pub(super) fn env_json(env: &[(String, Secret)]) -> J {
    J::Object(
        env.iter()
            .map(|(k, v)| (k.clone(), json!(String::from_utf8_lossy(v.expose()))))
            .collect(),
    )
}

pub(super) fn plan_json(plan: &Plan, recent: bool) -> J {
    match plan {
        Plan::Files => json!({"files": true}),
        Plan::Stop(Stop { text, locked }) => json!({"stop": text, "locked": locked}),
        Plan::Open {
            inner,
            title,
            mode,
            error,
            env,
            notice,
            conn,
            distro,
        } => json!({"inner": inner, "title": title, "mode": mode, "error": error,
                    "env": env_json(env), "notice": notice, "conn": conn, "distro": distro,
                    "recent": recent}),
    }
}

/// A restored SSH tab whose sealed password cannot be revealed because the vault is locked: the
/// restore asks for the master password first.
pub fn reopen_locked(cfg: &Value, entry: &Value, key: Option<&Secret>) -> bool {
    connection_protocol(entry) == PROTOCOL_SSH && reveal(cfg, entry, key).is_err_and(|s| s.locked)
}

/// For a restored connection: its command and environment, `None`
/// when it cannot be built. A password that cannot be revealed now (vault locked) is left out,
/// as when the user cancels the unlock prompt.
pub fn reopen_plan(
    cfg: &Value,
    entry: &Value,
    key: Option<&Secret>,
) -> Option<(String, Vec<(String, Secret)>)> {
    let cmd = build_conn_command(cfg, entry)
        .ok()
        .filter(|c| !c.is_empty())?;
    let mut env = Vec::new();
    if connection_protocol(entry) == PROTOCOL_SSH
        && let Ok(revealed) = reveal(cfg, entry, key)
    {
        env = secret_pairs(ssh_session_env(entry));
        env.extend(ssh_password_env(&revealed).unwrap_or_default());
    }
    Some((cmd, env))
}

/// What a quick-connect target opens.
#[derive(Debug, Clone, PartialEq)]
pub enum Quick {
    /// A saved entry of that name (`_open_connection_at`).
    Entry(Value),
    Error(String),
    /// An unsaved SSH session.
    Open {
        inner: String,
        title: String,
        mode: &'static str,
        conn: Value,
    },
}

/// Approximates Unicode case folding (as `hosttree` does).
fn casefold(s: &str) -> String {
    s.to_lowercase().replace('ß', "ss")
}

/// Quick connect: a saved entry's name opens that entry, anything else is
/// `user@host[:port]` for an unsaved SSH session.
pub fn quick_plan(cfg: &Value, text: &str) -> Quick {
    let target = strip_space(text);
    if !target.is_empty() {
        let want = casefold(target);
        let hit = seq(cfg, "connections")
            .iter()
            .find(|c| matches!(c, Value::Map(_)) && casefold(&entry_label(c)) == want);
        if let Some(c) = hit {
            return Quick::Entry(c.clone());
        }
    }
    let t = match parse_quick_ssh_target(target) {
        Ok(t) if !t.user.is_empty() && !t.host.is_empty() => t,
        Ok(_) => return Quick::Error("Invalid target.".to_owned()),
        Err(e) => return Quick::Error(e),
    };
    let s = |x: &str| Value::Str(x.to_owned());
    let conn = Value::Map(vec![
        (s("label"), s(&format!("{}@{}", t.user, t.host))),
        (s("host"), s(&t.host)),
        (s("user"), s(&t.user)),
        (
            s("port"),
            t.port.map_or(Value::Null, |p| Value::Int(p.into())),
        ),
        (s("terminal"), s(default_terminal_from_cfg(cfg))),
    ]);
    match build_ssh_command(&conn, section(cfg, "launch")) {
        Err(e) => Quick::Error(e),
        Ok(inner) => Quick::Open {
            inner,
            title: format!("SSH {}@{}", t.user, t.host),
            mode: terminal_mode_for_entry(Some(&conn), cfg),
            conn,
        },
    }
}

pub fn restore_json(tabs: &[RestoreTab]) -> J {
    let list: Vec<J> = tabs
        .iter()
        .map(|t| {
            let mut o =
                json!({"tab": t.tab, "pane": t.pane, "title": t.title, "detached": t.detached});
            match &t.source {
                RestoreSource::Inner(inner) => o["inner"] = json!(inner),
                RestoreSource::Connection { entry, matched } => {
                    o["entry"] = entry.to_json();
                    o["matched"] = json!(matched);
                }
            }
            o
        })
        .collect();
    J::Array(list)
}
