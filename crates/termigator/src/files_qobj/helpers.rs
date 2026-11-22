//! The pure helpers of the files panel: question and status texts, config access, the argv
//! builder, the command environment and the blocking runner.

use super::*;

// ---- helpers (pure) ------------------------------------------------------------------------

pub(crate) fn qs(s: &str) -> QString {
    QString::from(s)
}

/// `str.strip()` text of a QString.
pub(crate) fn stripped(q: &QString) -> String {
    strip_space(&q.to_string()).to_owned()
}

pub(super) const ROLES: [&str; 10] = [
    "kind", "name", "isDir", "isLink", "iconName", "sizeText", "mtime", "mode", "owner", "tooltip",
];
/// `Qt::UserRole`.
pub(super) const USER_ROLE: i32 = 0x0100;
pub(super) const LIST_TIMEOUT: u64 = 30;
pub(super) const NO_TARGET_TEXT: &str = "Open an SSH session — its files will show up here.";

/// The last line of stderr, else `failed (exit rc)`.
pub fn err_text(stderr: &[u8], rc: i32) -> String {
    let text = String::from_utf8_lossy(stderr);
    match strip_space(&text).lines().next_back() {
        Some(l) => l.to_owned(),
        None => format!("failed (exit {rc})"),
    }
}

/// The yes / no / cancel question about names that already exist.
pub fn clash_question(names: &[String], place: &str) -> String {
    let mut shown: Vec<String> = names.iter().take(10).map(|n| format!("  • {n}")).collect();
    if names.len() > 10 {
        shown.push("  …".to_owned());
    }
    format!(
        "{} item(s) already exist in {place}:\n{}\n\nYes = overwrite them, No = skip them.",
        names.len(),
        shown.join("\n")
    )
}

/// The delete confirmation.
pub fn delete_question_text(entries: &[&RemoteEntry]) -> String {
    if let [e] = entries {
        let what = if e.is_dir {
            "directory (recursively!)"
        } else {
            "file"
        };
        return format!(
            "Delete remote {what} “{}”?\nThis cannot be undone.",
            display_name(&e.name)
        );
    }
    let n_dirs = entries.iter().filter(|e| e.is_dir).count();
    let names: Vec<String> = entries
        .iter()
        .take(10)
        .map(|e| {
            format!(
                "  • {}{}",
                display_name(&e.name),
                if e.is_dir { "/" } else { "" }
            )
        })
        .collect();
    let more = if entries.len() > 10 { "\n  …" } else { "" };
    let folders = if n_dirs > 0 {
        format!(" ({n_dirs} folder(s), recursively!)")
    } else {
        String::new()
    };
    format!(
        "Delete {} remote item(s){folders}?\n{}{more}\nThis cannot be undone.",
        entries.len(),
        names.join("\n")
    )
}

/// The status line under a listing.
pub(super) fn listing_status(
    label: &str,
    cwd: &[u8],
    shown: usize,
    entries: &[RemoteEntry],
) -> String {
    let n_dirs = entries.iter().filter(|e| e.is_dir).count();
    format!(
        "{label} · {} — {shown} shown ({} total, {n_dirs} dir(s))",
        display_name(cwd),
        entries.len()
    )
}

pub(crate) fn base_name(p: &Path) -> String {
    p.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

pub(crate) fn int_list(invokable: &str, json: &QString) -> Vec<usize> {
    let text = json.to_string();
    crate::qml_json::logged(invokable, &text, serde_json::from_str(&text)).unwrap_or_default()
}

pub(crate) fn string_list(invokable: &str, json: &QString) -> Vec<String> {
    let text = json.to_string();
    crate::qml_json::logged(invokable, &text, serde_json::from_str(&text)).unwrap_or_default()
}

pub(crate) fn text_json(v: &J) -> QString {
    qs(&v.to_string())
}

/// The value at `key`, created as an empty mapping when missing or of another type.
pub(super) fn slot<'a>(map: &'a mut Value, key: &str) -> &'a mut Value {
    if !matches!(map, Value::Map(_)) {
        *map = Value::Map(Vec::new());
    }
    match map {
        Value::Map(pairs) => {
            let at = pairs
                .iter()
                .position(|(k, _)| matches!(k, Value::Str(s) if s == key))
                .unwrap_or_else(|| {
                    pairs.push((Value::Str(key.to_owned()), Value::Map(Vec::new())));
                    pairs.len() - 1
                });
            let value = &mut pairs[at].1;
            if !matches!(value, Value::Map(_)) {
                *value = Value::Map(Vec::new());
            }
            value
        }
        other => other, // not reached: a mapping was just put there
    }
}

/// `cfg["files_bookmarks"]` (created on first use).
pub(super) fn with_store<R>(f: impl FnOnce(&mut Value) -> R) -> R {
    with_app(|app| f(slot(&mut app.cfg, "files_bookmarks")))
}

pub(crate) fn launch() -> Value {
    with_app(|app| section(&app.cfg, "launch").clone())
}

pub(crate) fn credentials(cfg: &Value) -> &[Value] {
    match cfg.get("credentials") {
        Some(Value::Seq(items)) => items,
        _ => &[],
    }
}

/// The argv of a remote command for `conn`.
pub(crate) fn build(
    conn: &Value,
    f: impl FnOnce(&RemoteOps<'_>) -> Result<Argv, String>,
) -> Result<Argv, String> {
    let launch = launch();
    let env = Env::system();
    f(&RemoteOps {
        conn,
        launch: &launch,
        env: &env,
    })
}

/// The process environment plus `SSHPASS` for a revealed entry; the error is the status text.
pub(crate) fn command_env(
    conn: &Value,
    host_label: &str,
) -> Result<BTreeMap<OsString, OsString>, String> {
    let Some(revealed) = reveal_conn(conn) else {
        return Err(format!(
            "{host_label}: saved password is locked — enter the master password."
        ));
    };
    let pw = ssh_password_env(&revealed).map_err(|e| format!("{host_label}: {e}"))?;
    let mut env: BTreeMap<OsString, OsString> = std::env::vars_os().collect();
    env.extend(pw.into_iter().map(|(k, v)| {
        (
            k.into(),
            termigator_core::platform::os_string_from_vec(v.expose().to_vec()),
        )
    }));
    Ok(env)
}

/// What a worker reports.
pub(crate) struct Done {
    pub rc: i32,
    pub err: Vec<u8>,
}

/// Runs `argv` and returns its result; `None` when it was cancelled. Blocks (worker threads).
pub(crate) fn run_blocking(
    argv: &Argv,
    env: BTreeMap<OsString, OsString>,
    timeout: u64,
) -> Option<Done> {
    let mut cmd = Command::new(
        argv.iter()
            .map(|a| termigator_core::platform::os_string_from_vec(a.clone())),
    );
    cmd.env = env;
    cmd.timeout = Some(Duration::from_secs(timeout));
    cmd.max_output = DEFAULT_MAX_OUTPUT;
    let running = match runner::spawn(cmd) {
        Ok(r) => r,
        Err(e) => {
            return Some(Done {
                rc: 127,
                err: e.to_string().into_bytes(),
            });
        }
    };
    let out = running.wait()?;
    if out.cancelled {
        return None;
    }
    if out.timed_out {
        return Some(Done {
            rc: 124,
            err: format!("Timed out after {timeout}s.").into_bytes(),
        });
    }
    Some(Done {
        rc: out.code(),
        err: out.stderr,
    })
}

pub(super) fn column_of(i: i32) -> SortColumn {
    match i {
        1 => SortColumn::Size,
        2 => SortColumn::Modified,
        3 => SortColumn::Mode,
        4 => SortColumn::Owner,
        _ => SortColumn::Name,
    }
}

/// `map[key] = value` in a mapping.
pub(super) fn set_in(map: &mut Value, key: &str, value: Value) {
    let Value::Map(pairs) = map else { return };
    match pairs
        .iter_mut()
        .find(|(k, _)| matches!(k, Value::Str(s) if s == key))
    {
        Some(p) => p.1 = value,
        None => pairs.push((Value::Str(key.to_owned()), value)),
    }
}

/// The base names of `paths` that `entries` (a remote folder) already has.
pub(crate) fn upload_clash_names(paths: &[String], entries: &[RemoteEntry]) -> Vec<String> {
    paths
        .iter()
        .map(|p| base_name(Path::new(p)))
        .filter(|b| entries.iter().any(|e| display_name(&e.name) == *b))
        .collect()
}
