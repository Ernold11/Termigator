//! scp and sftp command lines, glob escaping, sftp batch quoting.
//!
//! [`build_scp_command_with`] gives the exact shell line the context-menu file actions use
//! (guard 4 of ARCHITECTURE.md §7: paths never start with `-`, a *download* source goes
//! through [`scp_glob_escape`], IPv6 hosts are bracketed, `sshpass -e` is prefixed when a
//! password is set). [`scp_argv_with`] turns it into the Files panel's argv (shared
//! ControlMaster, keepalive, `ConnectTimeout`, translated `extra_ssh_args`), and
//! [`sftp_resume_argv_with`] derives the `sftp reget` invocation plus its batch text from the
//! download argv. Binary lookup, the ControlPath directory and `$HOME` come from
//! [`Env`], so everything is deterministic in tests. Golden fixtures: `compat/golden/scp_*.json`.
//!
//! [`sftp_resume_argv_with`] refuses a path with `\n`, `\r` or NUL, which would
//! otherwise add a line (`!cmd`) to the sftp batch file.
//!
//! Known limits (none occur with sane input):
//! - `browse_conn` is not applied: [`build_scp_command_with`] reads only keys it keeps.
//! - `-p<digits>` in `extra_ssh_args` takes ASCII digits only.
//! - A non-finite or out-of-range `ssh_keepalive_seconds` falls back to 15.
//! - `~user` in a local path is not expanded (see [`crate::proto::ssh::expand_user`]).

use std::path::Path;

use crate::config::doc::Value;
use crate::config::model::{strip_space, truthy, value_int, value_str};
use crate::proto::ssh::{
    Env, conn_port, expand_user, identity_flags, keepalive_ssh_opts, normalize_proxy_jump_hops,
    password_via_askpass, quote_repr, ssh_control_opts, ssh_target_guard,
};
use crate::shell::{join, split};

/// `str(v or "")` for a mapping value.
fn text(v: Option<&Value>) -> String {
    match v {
        Some(x) if truthy(x) => value_str(x),
        _ => String::new(),
    }
}

/// Backslash-escape `\ * ? [ ]` so scp's SFTP-mode globbing takes a remote *download* path
/// literally; braces are not special there.
pub fn scp_glob_escape(path: &str) -> String {
    let mut out = String::with_capacity(path.len());
    for c in path.chars() {
        if matches!(c, '\\' | '*' | '?' | '[' | ']') {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// A literal path for an sftp batch file: double quotes with `\` and `"` escaped; quoted words
/// are never glob-expanded.
pub fn sftp_quote(path: &str) -> String {
    format!("\"{}\"", path.replace('\\', "\\\\").replace('"', "\\\""))
}

/// [`build_scp_command_with`] on the real machine.
pub fn build_scp_command(
    conn: &Value,
    local_path: &str,
    remote_path: &str,
    upload: bool,
) -> Result<String, String> {
    build_scp_command_with(conn, local_path, remote_path, upload, &Env::system())
}

/// The scp shell line for an upload or download, authenticating like the interactive session
/// (port, identity, jump hosts, host key policy, `sshpass -e`).
pub fn build_scp_command_with(
    conn: &Value,
    local_path: &str,
    remote_path: &str,
    upload: bool,
    env: &Env,
) -> Result<String, String> {
    let host = strip_space(&text(conn.get("host"))).to_owned();
    let user = strip_space(&text(conn.get("user"))).to_owned();
    if host.is_empty() || user.is_empty() {
        return Err("Host or user is missing.".to_owned());
    }
    if let Some(e) = ssh_target_guard(&user, &host) {
        return Err(e);
    }

    let local = expand_user(strip_space(local_path), env.home.as_deref());
    let remote = strip_space(remote_path);
    if local.is_empty() {
        return Err("Local path is missing.".to_owned());
    }
    if remote.is_empty() {
        return Err("Remote path is missing.".to_owned());
    }
    if local.starts_with('-') {
        return Err("Local path must not start with '-'.".to_owned());
    }
    if remote.starts_with('-') {
        return Err("Remote path must not start with '-'.".to_owned());
    }
    if upload && !Path::new(&local).exists() {
        return Err(format!("Local file not found: {}", quote_repr(&local)));
    }

    let mut cmd = vec!["scp".to_owned()];
    if upload && Path::new(&local).is_dir() {
        cmd.push("-r".to_owned());
    }
    if let Some(p) = conn_port(conn)? {
        // scp: uppercase -P (lowercase means "preserve").
        cmd.extend(["-P".to_owned(), p.to_string()]);
    }
    cmd.extend(identity_flags(conn, env.home.as_deref())?);
    let hops = normalize_proxy_jump_hops(conn)?;
    if !hops.is_empty() {
        cmd.extend(["-J".to_owned(), hops.join(",")]);
    }
    if let Some(shk) = conn.get("strict_host_key_checking") {
        let raw = value_str(shk);
        let val = strip_space(&raw);
        if !matches!(shk, Value::Null) && matches!(val, "yes" | "no" | "ask" | "accept-new" | "off")
        {
            cmd.extend(["-o".to_owned(), format!("StrictHostKeyChecking={val}")]);
        }
    }

    // scp requires brackets around IPv6 addresses in the remote spec. Only a download source
    // is escaped: the upload destination is taken literally.
    let remote_host = if host.contains(':') {
        format!("[{host}]")
    } else {
        host
    };
    let remote_path = if upload {
        remote.to_owned()
    } else {
        scp_glob_escape(remote)
    };
    let spec = format!("{user}@{remote_host}:{remote_path}");
    if upload {
        cmd.extend([local, spec]);
    } else {
        cmd.extend([spec, local]);
    }

    if !text(conn.get("password")).is_empty() {
        // Windows, or a jump host: no sshpass, ssh asks the askpass helper (`ssh_password_env`).
        if !password_via_askpass(conn) {
            if (env.which)("sshpass").is_none() {
                return Err("Password is set but sshpass is not in PATH \
                            (e.g. sudo apt install sshpass). Use an SSH key instead."
                    .to_owned());
            }
            cmd.splice(0..0, ["sshpass".to_owned(), "-e".to_owned()]);
        }
    }
    Ok(join(&cmd))
}

/// `extra_ssh_args` as a list of words (empty on unparsable text).
fn extra_args_list(conn: &Value) -> Vec<String> {
    match conn.get("extra_ssh_args") {
        Some(x) if truthy(x) => match x {
            Value::Seq(items) => items.iter().map(value_str).collect(),
            other => split(&value_str(other)).unwrap_or_default(),
        },
        _ => Vec::new(),
    }
}

/// scp speaks a different flag language (`-p` is "preserve times", the port is `-P`), so
/// translate the ssh extra args it can honour and drop the rest.
fn scp_extra_for_files(conn: &Value) -> Vec<String> {
    let args = extra_args_list(conn);
    let mut out = Vec::new();
    let mut i = 0;
    while i < args.len() {
        let a = args[i].as_str();
        let next = args.get(i + 1);
        if let Some(n) = next {
            if matches!(a, "-o" | "-i" | "-J" | "-F") {
                out.extend([a.to_owned(), n.clone()]);
                i += 2;
                continue;
            }
            if a == "-p" {
                out.extend(["-o".to_owned(), format!("Port={n}")]);
                i += 2;
                continue;
            }
        }
        if let Some(rest) = a.strip_prefix("-o").filter(|r| !r.is_empty()) {
            out.extend(["-o".to_owned(), rest.to_owned()]);
        } else if let Some(rest) = a
            .strip_prefix("-p")
            .filter(|r| !r.is_empty() && r.bytes().all(|b| b.is_ascii_digit()))
        {
            out.extend(["-o".to_owned(), format!("Port={rest}")]);
        } else if matches!(a, "-4" | "-6" | "-C" | "-q" | "-v") {
            out.push(a.to_owned());
        }
        i += 1;
    }
    out
}

/// What the Files panel asks of [`scp_argv_with`].
#[derive(Debug, Clone, Copy)]
pub struct ScpTransfer<'a> {
    pub local_path: &'a str,
    pub remote_path: &'a str,
    pub upload: bool,
    /// Add `-r` (when the line has none yet).
    pub recursive: bool,
    /// Add `-p` (keep mtime; directory sync compares by it).
    pub preserve: bool,
}

/// Insert `opts` right after the first `exe` (which may follow a `sshpass -e` prefix).
fn splice_opts(argv: &mut Vec<String>, exe: &str, opts: &[String]) -> bool {
    let Some(i) = argv.iter().position(|a| a == exe) else {
        return false;
    };
    argv.splice(i + 1..i + 1, opts.iter().cloned());
    true
}

/// [`scp_argv_with`] on the real machine.
pub fn scp_argv(conn: &Value, launch: &Value, t: &ScpTransfer) -> Result<Vec<String>, String> {
    scp_argv_with(conn, launch, t, &Env::system())
}

/// scp argv for the panel's upload/download, riding the shared connection: ControlMaster and
/// keepalive per `launch`, `ConnectTimeout`, `BatchMode` without a saved password, and the
/// translated `extra_ssh_args`.
pub fn scp_argv_with(
    conn: &Value,
    launch: &Value,
    t: &ScpTransfer,
    env: &Env,
) -> Result<Vec<String>, String> {
    let line = build_scp_command_with(conn, t.local_path, t.remote_path, t.upload, env)?;
    const SHAPE: &str = "Unexpected scp command shape.";
    let mut argv = split(&line).map_err(|_| SHAPE.to_owned())?;

    let mut opts = Vec::new();
    if launch.get("ssh_control_master").is_none_or(truthy) {
        opts.extend(ssh_control_opts(env.control_dir.as_deref()));
    }
    let keepalive = launch.get("ssh_keepalive").is_none_or(truthy)
        && !matches!(conn.get("ssh_keepalive"), Some(Value::Bool(false)));
    if keepalive {
        let secs = launch
            .get("ssh_keepalive_seconds")
            .filter(|v| truthy(v))
            .and_then(value_int)
            .unwrap_or(15);
        opts.extend(keepalive_ssh_opts(secs));
    }
    opts.extend(["-o".to_owned(), "ConnectTimeout=10".to_owned()]);
    if text(conn.get("password")).is_empty() {
        // A background op must never hang on a prompt; sshpass answers it, so BatchMode stays
        // off with a saved password.
        opts.extend(["-o".to_owned(), "BatchMode=yes".to_owned()]);
    }
    opts.extend(scp_extra_for_files(conn));

    if !splice_opts(&mut argv, "scp", &opts) {
        return Err(SHAPE.to_owned());
    }
    if t.recursive && !argv.iter().any(|a| a == "-r") {
        splice_opts(&mut argv, "scp", &["-r".to_owned()]);
    }
    if t.preserve && !argv.iter().any(|a| a == "-p") {
        splice_opts(&mut argv, "scp", &["-p".to_owned()]);
    }
    Ok(argv)
}

/// Shown instead of resuming a download whose path holds a line break or NUL.
pub const LINE_BREAK_MSG: &str = "Cannot resume: the path contains a line break or NUL character.";

/// What the Files panel asks of [`sftp_resume_argv_with`].
#[derive(Debug, Clone, Copy)]
pub struct SftpResume<'a> {
    pub local_path: &'a str,
    pub remote_path: &'a str,
    /// Where the caller will write the batch text.
    pub batch_path: &'a str,
    pub recursive: bool,
}

/// [`sftp_resume_argv_with`] on the real machine.
pub fn sftp_resume_argv(
    conn: &Value,
    launch: &Value,
    r: &SftpResume,
) -> Result<(Vec<String>, String), String> {
    sftp_resume_argv_with(conn, launch, r, &Env::system())
}

/// `(argv, batch_text)` to resume an interrupted download with `sftp reget` (scp cannot
/// resume): the download's scp argv turned into sftp, same options. The caller writes the
/// batch text to `batch_path`; `progress` turns the meter on, quoted paths are literal.
pub fn sftp_resume_argv_with(
    conn: &Value,
    launch: &Value,
    r: &SftpResume,
    env: &Env,
) -> Result<(Vec<String>, String), String> {
    const SHAPE: &str = "Could not build sftp command.";
    // The batch file is line-oriented (`!cmd` runs a local command) and a quoted word
    // cannot hold a line break, so a server-sent name with one would add a line of its own.
    // Refused rather than escaped: sftp has no escape for it.
    if [r.local_path, r.remote_path]
        .iter()
        .any(|p| p.contains(['\n', '\r', '\0']))
    {
        return Err(LINE_BREAK_MSG.to_owned());
    }
    let t = ScpTransfer {
        local_path: r.local_path,
        remote_path: r.remote_path,
        upload: false,
        recursive: false,
        preserve: false,
    };
    let argv = scp_argv_with(conn, launch, &t, env)?;
    let i = argv.iter().position(|a| a == "scp").ok_or(SHAPE)?;
    let n = argv.len();
    if n < i + 3 {
        return Err(SHAPE.to_owned());
    }
    let opts = argv[i + 1..n - 2].iter().filter(|a| *a != "-r").cloned();
    // user@host:path / user@[v6]:path; sftp wants just the host part.
    let spec = &argv[n - 2];
    let end = if spec.contains("@[") {
        spec.find(']').map(|p| p + 1)
    } else {
        spec.find(':')
    };
    let target = &spec[..end.ok_or(SHAPE)?];
    let flags = if r.recursive { "-R -p" } else { "-p" };
    let batch = format!(
        "progress\nreget {flags} {} {}\n",
        sftp_quote(r.remote_path),
        sftp_quote(r.local_path)
    );
    let mut out: Vec<String> = argv[..i].to_vec();
    out.push("sftp".to_owned());
    out.extend(opts);
    out.extend(["-b".to_owned(), r.batch_path.to_owned(), target.to_owned()]);
    Ok((out, batch))
}
