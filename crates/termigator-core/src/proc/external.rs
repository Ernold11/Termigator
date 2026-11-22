//! External terminal spawn and xdotool window follow.
//!
//! * [`spawn_external_terminal`] starts the (already resolved, see
//!   `proto::konsole::external_terminal_argv`) terminal detached: stdio is `/dev/null`, the
//!   environment is the current one overlaid with `env` (secrets travel there, never in argv),
//!   and it leads a new session so closing Termigator does not hang it up. The crate forbids
//!   `unsafe`, so the session is made by util-linux `setsid(1)` exec'ing the terminal in place
//!   (the pid is the terminal's own). A thread reaps the
//!   terminal when it exits, so no zombie is left.
//! * [`follow_window`] runs `xdotool` through [`super::runner`] with 2 s timeouts (a hung
//!   X server cannot hang the caller beyond that) and blocks, so call it from a worker thread.
//!   A missing `xdotool` (spawn fails) or any failure gives `None`.

use std::{
    collections::BTreeMap,
    ffi::OsString,
    process::{Command, Stdio},
    thread,
    time::Duration,
};

use super::{
    runner::{self, Outcome},
    tunnel::with_system_env,
};
use crate::proto::konsole::{
    parse_window_geometry, parse_window_id, xdotool_geometry_argv, xdotool_search_name_argv,
    xdotool_search_pid_argv,
};

/// Timeout of every xdotool call.
pub const XDOTOOL_TIMEOUT: Duration = Duration::from_secs(2);

/// Start `argv` as a detached terminal; returns its pid.
pub fn spawn_external_terminal(
    argv: &[String],
    env: &BTreeMap<OsString, OsString>,
) -> Result<u32, String> {
    let Some(exe) = argv.first() else {
        return Err("After expanding launch.template there is no command.".to_owned());
    };
    // setsid(1) would swallow a failed exec; an explicit path can be checked up front.
    if exe.contains('/') {
        std::fs::metadata(exe).map_err(|e| format!("{e}: '{exe}'"))?;
    }
    let mut full_env = with_system_env(env);
    let spawned = Command::new("setsid")
        .arg("--") // else getopt would eat the terminal's own options such as -e
        .arg(exe)
        .args(&argv[1..])
        .env_clear()
        .envs(&full_env)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
    // shortcut: the temporary std Command frees its env copy unwiped, upgrade with proc::runner.
    crate::vault::wipe_env(&mut full_env);
    let mut child = spawned.map_err(|e| e.to_string())?;
    let pid = child.id();
    thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(pid)
}

/// `(x, y, width, height)` of the terminal window of `pid` (falls back to the newest visible
/// window titled Konsole). Blocks for up to a few seconds: worker thread only.
pub fn follow_window(pid: u32) -> Option<(i64, i64, i64, i64)> {
    follow_window_with_env(pid, &BTreeMap::new())
}

/// [`follow_window`] with extra environment (tests point `PATH` at a stub `xdotool`).
pub fn follow_window_with_env(
    pid: u32,
    env: &BTreeMap<OsString, OsString>,
) -> Option<(i64, i64, i64, i64)> {
    let env = with_system_env(env);
    let by_pid = if pid == 0 {
        None
    } else {
        xdotool(&xdotool_search_pid_argv(pid), &env).and_then(|o| parse_window_id(&o))
    };
    let win = match by_pid {
        Some(w) => w,
        None => parse_window_id(&xdotool(&xdotool_search_name_argv(), &env)?)?,
    };
    parse_window_geometry(&xdotool(&xdotool_geometry_argv(&win), &env)?)
}

/// stdout of a successful run, else `None` (non-zero exit, timeout, missing binary).
fn xdotool(argv: &[String], env: &BTreeMap<OsString, OsString>) -> Option<String> {
    let mut cmd = runner::Command::new(argv);
    cmd.env = env.clone();
    cmd.timeout = Some(XDOTOOL_TIMEOUT);
    let o: Outcome = runner::spawn(cmd).ok()?.wait()?;
    (o.status.success() && !o.timed_out).then(|| String::from_utf8_lossy(&o.stdout).into_owned())
}
