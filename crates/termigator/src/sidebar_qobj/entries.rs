//! Entry helpers: config access, launch commands, add / edit validation, Copy command, probe.

use super::*;

/// `str(v.get(key) or "")`.
pub(super) fn text(v: &Value, key: &str) -> String {
    match v.get(key) {
        Some(x) if truthy(x) => value_str(x),
        _ => String::new(),
    }
}

pub(super) fn seq<'a>(cfg: &'a Value, key: &str) -> &'a [Value] {
    match cfg.get(key) {
        Some(Value::Seq(items)) => items,
        _ => &[],
    }
}

pub(super) fn entry_at(cfg: &Value, e: EntryRef) -> Option<&Value> {
    let key = match e.kind {
        EntryKind::Connection => "connections",
        EntryKind::Favorite => "favorites",
    };
    seq(cfg, key)
        .get(e.index)
        .filter(|v| matches!(v, Value::Map(_)))
}

pub(super) fn connection_at(cfg: &Value, index: i32) -> Option<&Value> {
    entry_at(
        cfg,
        EntryRef {
            kind: EntryKind::Connection,
            index: usize::try_from(index).ok()?,
        },
    )
}

fn set_key(map: &mut Value, key: &str, value: Value) {
    let Value::Map(pairs) = map else { return };
    match pairs
        .iter_mut()
        .find(|(k, _)| matches!(k, Value::Str(s) if s == key))
    {
        Some(slot) => slot.1 = value,
        None => pairs.push((Value::Str(key.to_owned()), value)),
    }
}

/// `self._cfg[key] = [*list, item]`.
fn push_entry(cfg: &mut Value, key: &str, item: Value) {
    let mut items = seq(cfg, key).to_vec();
    items.push(item);
    set_key(cfg, key, Value::Seq(items));
}

/// The entry with its named credential's user and password.
pub(super) fn with_credential(cfg: &Value, c: &Value) -> Value {
    apply_credential(c, seq(cfg, "credentials"))
}

/// `launch.track_remote_folder` (Preferences → Terminal, on unless turned off) as the flag
/// `build_ssh_command` reads: the SSH tab then starts its shell through the OSC 7 bootstrap.
fn with_folder_tracking(lo: &Value) -> Value {
    let mut lo = lo.clone();
    let on = lo.get("track_remote_folder").is_none_or(truthy);
    if let Value::Map(m) = &mut lo {
        m.retain(|(k, _)| !matches!(k, Value::Str(k) if k == "track_remote_folder"));
        m.push((
            Value::Str("track_remote_folder".to_owned()),
            Value::Bool(on),
        ));
    }
    lo
}

/// The launch command of an entry, by its protocol.
pub fn build_conn_command(cfg: &Value, c: &Value) -> Result<String, String> {
    let c = with_credential(cfg, c);
    let proto = connection_protocol(&c);
    let lo = section(cfg, "launch");
    let gateway_set = c
        .get("ssh_gateway")
        .is_some_and(|g| truthy(g) && !strip_space(&value_str(g)).is_empty());
    if (proto == PROTOCOL_RDP || proto == PROTOCOL_VNC) && gateway_set {
        return build_via_gateway(cfg, &c, proto);
    }
    match proto {
        PROTOCOL_RDP => rdp::build_rdp_command(&c, lo, &Env::system()),
        PROTOCOL_VNC => vnc::build_vnc_command(&c, lo, &vnc::which_in_path),
        PROTOCOL_TELNET => telnet::build_telnet_command(&c, &vnc::which_in_path),
        PROTOCOL_SERIAL => serial::build_serial_command(&c, lo, &vnc::which_in_path),
        _ => ssh::build_ssh_command(&c, &with_folder_tracking(lo)),
    }
}

/// RDP / VNC through an SSH entry.
fn build_via_gateway(cfg: &Value, c: &Value, proto: &str) -> Result<String, String> {
    let name = c.get("ssh_gateway");
    let Some(gw) = gateway::find_gateway(seq(cfg, "connections"), name) else {
        let shown = match name {
            Some(Value::Str(s)) => quote_repr(s),
            Some(v) => value_str(v),
            None => "None".to_owned(),
        };
        return Err(format!("SSH gateway {shown} is not a saved SSH entry."));
    };
    let gw = with_credential(cfg, gw);
    let lo = section(cfg, "launch");
    let env = Env::system();
    let port = free_local_port().map_err(|e| e.to_string())?;
    let client = |cc: &Value| {
        if proto == PROTOCOL_RDP {
            rdp::build_rdp_command(cc, lo, &env)
        } else {
            vnc::build_vnc_command(cc, lo, &vnc::which_in_path)
        }
    };
    gateway::build_via_gateway(c, &gw, &client, lo, port, &env)
}

/// The bulk apply, without the persist.
pub(super) fn bulk_apply(
    cfg: &mut Value,
    state: &mut TreeState,
    indices: &[i64],
    changes: &Value,
) -> Outcome {
    let mut conns = seq(cfg, "connections").to_vec();
    let creds = seq(cfg, "credentials").to_vec();
    let (changed, na) = apply_bulk(&mut conns, indices, changes, &creds);
    let mut out = Outcome::default();
    if changed > 0 {
        set_key(cfg, "connections", Value::Seq(conns));
        if let Some(g) = changes.get("group").filter(|g| truthy(g)) {
            let group = normalize_group_path(&value_str(g));
            state.expanded_groups.extend(group_path_ancestors(&group));
        }
        out.changed = true;
        out.notice = Some(format!(
            "Changed {changed} entr{}.",
            if changed == 1 { "y" } else { "ies" }
        ));
    }
    if na > 0 {
        out.info = Some(format!(
            "{na} entr{} left partly unchanged — a jump host applies only to SSH, a credential \
             only to SSH and RDP.",
            if na == 1 { "y was" } else { "ies were" }
        ));
    }
    out
}

fn has(data: &Value, keys: &[&str]) -> bool {
    keys.iter().all(|k| data.get(k).is_some_and(truthy))
}

/// The add / edit flows after the editor's OK: validation, the default
/// label, the launch-command check, then the entry goes into the list. `Ok(false)`: the edited
/// entry is gone; nothing changed.
pub fn store_entry(
    cfg: &mut Value,
    kind: &str,
    target: Option<EntryRef>,
    mut data: Value,
) -> Result<bool, Msg> {
    let lo = section(cfg, "launch").clone();
    let info = |t: &str| Msg::Info(t.to_owned());
    if kind == "favorite" || target.is_some_and(|t| t.kind == EntryKind::Favorite) {
        // FavoriteDialog.favorite_dict() is None without both.
        let ok = matches!(data, Value::Map(_))
            && !strip_space(&text(&data, "label")).is_empty()
            && !strip_space(&text(&data, "command")).is_empty();
        if !ok {
            return Err(info("Enter label and command."));
        }
        return Ok(match target {
            None => {
                push_entry(cfg, "favorites", data);
                true
            }
            Some(t) => replace_entry(cfg, "favorites", t.index, data),
        });
    }
    if !matches!(data, Value::Map(_)) {
        data = Value::Map(Vec::new());
    }
    let Some(t) = target else {
        // `_add_ssh` / `_add_rdp` / `_add_vnc` check with the bare builder (no credential, no
        // gateway); `_add_simple_entry` (Telnet, serial) with `_build_conn_command`.
        let built = match kind {
            "rdp" | "vnc" | "telnet" => {
                if !has(&data, &["host"]) {
                    return Err(info("Enter a host."));
                }
                let label = entry_label(&data);
                label_default(&mut data, label);
                match kind {
                    "rdp" => rdp::build_rdp_command(&data, &lo, &Env::system()),
                    "vnc" => vnc::build_vnc_command(&data, &lo, &vnc::which_in_path),
                    _ => build_conn_command(cfg, &data),
                }
            }
            "serial" => {
                if !has(&data, &["device"]) {
                    return Err(info("Enter a device."));
                }
                let label = entry_label(&data);
                label_default(&mut data, label);
                build_conn_command(cfg, &data)
            }
            _ => {
                if !has(&data, &["host", "user"]) {
                    return Err(info("Enter host and user."));
                }
                let label = format!("{}@{}", text(&data, "user"), text(&data, "host"));
                label_default(&mut data, label);
                ssh::build_ssh_command(&data, &lo)
            }
        };
        built.map_err(Msg::Warning)?;
        push_entry(cfg, "connections", data);
        return Ok(true);
    };
    let Some(cur) = entry_at(cfg, t) else {
        return Ok(false);
    };
    match connection_protocol(cur) {
        PROTOCOL_SERIAL if !has(&data, &["device"]) => return Err(info("Enter a device.")),
        PROTOCOL_RDP | PROTOCOL_VNC | PROTOCOL_TELNET if !has(&data, &["host"]) => {
            return Err(info("Enter a host."));
        }
        PROTOCOL_SSH if !has(&data, &["host", "user"]) => {
            return Err(info("Enter host and user."));
        }
        _ => {}
    }
    let label = entry_label(&data);
    label_default(&mut data, label);
    build_conn_command(cfg, &data).map_err(Msg::Warning)?;
    Ok(replace_entry(cfg, "connections", t.index, data))
}

fn label_default(data: &mut Value, label: String) {
    if !data.get("label").is_some_and(truthy) {
        set_key(data, "label", Value::Str(label));
    }
}

fn replace_entry(cfg: &mut Value, key: &str, index: usize, data: Value) -> bool {
    let mut items = seq(cfg, key).to_vec();
    let Some(slot) = items.get_mut(index) else {
        return false;
    };
    *slot = data;
    set_key(cfg, key, Value::Seq(items));
    true
}

/// The clipboard text and the status-bar notice.
pub fn copy_command(cfg: &Value, c: &Value) -> Result<(String, String), String> {
    let proto = connection_protocol(c);
    let name = match proto {
        PROTOCOL_RDP => "RDP",
        PROTOCOL_VNC => "VNC",
        PROTOCOL_TELNET => "Telnet",
        PROTOCOL_SERIAL => "serial",
        _ => "SSH",
    };
    let cmd =
        build_conn_command(cfg, c).map_err(|e| format!("Cannot build {name} command:\n{e}"))?;
    if strip_space(&cmd).is_empty() {
        return Err(format!("Empty {name} command."));
    }
    let cmd = match proto {
        PROTOCOL_SSH => ssh::clipboard_ssh_command(&cmd, &with_credential(cfg, c)),
        PROTOCOL_RDP => rdp::clipboard_rdp_command(&cmd, &with_credential(cfg, c)),
        _ => cmd,
    };
    let notice = format!(
        "Copied the {name} command of “{}” to the clipboard.",
        entry_label(c)
    );
    Ok((cmd, notice))
}

/// Before the tab opens: `(command, title)` or the warning.
pub fn nettool_line(c: &Value, tool: &str) -> Result<(String, String), String> {
    let (cmd, title) = nettools::tool_for_entry(tool, c, &vnc::which_in_path)?;
    if cmd.is_empty() {
        return Err("Nothing to run.".to_owned());
    }
    Ok((cmd, title))
}

/// `launch.probe_timeout` as seconds, 5 s when unusable.
pub fn probe_timeout(cfg: &Value) -> Duration {
    let secs = match section(cfg, "launch").get("probe_timeout") {
        Some(Value::Int(i)) if *i != 0 => Some(*i as f64),
        Some(Value::Float(f)) if *f != 0.0 => Some(*f),
        Some(Value::Bool(true)) => Some(1.0),
        Some(Value::Str(s)) if !s.is_empty() => strip_space(s).parse::<f64>().ok(),
        Some(v) if truthy(v) => None,
        _ => Some(5.0),
    };
    secs.and_then(|s| Duration::try_from_secs_f64(s).ok())
        .unwrap_or(DEFAULT_TIMEOUT)
}

/// The Test connection dialog's data.
pub fn report_json(r: &ProbeReport) -> String {
    let steps: Vec<_> = r
        .steps
        .iter()
        .map(|s| {
            let mark = match s.ok {
                Some(true) => "✔",
                Some(false) => "✘",
                None => "•",
            };
            json!({
                "check": format!("{mark}  {}", s.name),
                "detail": s.detail,
                "time": s.ms.map(|ms| format!("{ms:.0} ms")).unwrap_or_default(),
            })
        })
        .collect();
    json!({
        "headline": format!("{}  {}", if r.ok { "✔" } else { "✘" }, r.summary),
        "note": r.via_note,
        "steps": steps,
    })
    .to_string()
}
