//! `proc::external`: detached terminal spawn (new session, null stdio, env not argv) and the
//! xdotool follow with a stub `xdotool` on a temp PATH.
// Unix-only for now: uses modes, symlinks, sockets or sh stubs.
#![cfg(unix)]
#![allow(clippy::unwrap_used)]

use std::{
    collections::BTreeMap,
    ffi::OsString,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

mod live;

use termigator_core::proc::external::{follow_window_with_env, spawn_external_terminal};

fn dir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("termigator-ext-{tag}-{}", std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn script(path: &Path, body: &str) {
    live::script(path, &format!("#!/bin/bash\n{body}\n"));
}

fn wait_for(path: &Path) -> String {
    let t0 = Instant::now();
    loop {
        if let Ok(s) = std::fs::read_to_string(path)
            && s.ends_with('\n')
        {
            return s;
        }
        assert!(
            t0.elapsed() < Duration::from_secs(5),
            "{} never written",
            path.display()
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn spawn_returns_pid_new_session_env_and_args() {
    let d = dir("spawn");
    let (term, out) = (d.join("term"), d.join("out"));
    script(
        &term,
        &format!(
            "{{ echo \"$TERMIGATOR_SECRET\"; printf '%s\\n' \"$*\"; \
             awk '{{print $6}}' /proc/$$/stat; echo $$; }} > {o}.tmp; mv {o}.tmp {o}",
            o = out.display()
        ),
    );
    let env = BTreeMap::from([(
        OsString::from("TERMIGATOR_SECRET"),
        OsString::from("s3cret"),
    )]);
    let argv = vec![term.display().to_string(), "-e".into(), "bash".into()];
    let pid = spawn_external_terminal(&argv, &env).unwrap();
    let lines: Vec<String> = wait_for(&out).lines().map(str::to_owned).collect();
    assert_eq!(lines[0], "s3cret");
    assert_eq!(lines[1], "-e bash");
    assert_eq!(
        lines[2],
        pid.to_string(),
        "session id is the terminal's own pid"
    );
    assert_eq!(lines[3], pid.to_string(), "pid returned is the terminal's");
    // The terminal ended and was reaped: no zombie is left behind.
    let t0 = Instant::now();
    while Path::new(&format!("/proc/{pid}")).exists() {
        assert!(
            t0.elapsed() < Duration::from_secs(5),
            "terminal never reaped"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    std::fs::remove_dir_all(&d).unwrap();
}

#[test]
fn spawn_errors() {
    assert_eq!(
        spawn_external_terminal(&[], &BTreeMap::new()),
        Err("After expanding launch.template there is no command.".to_owned())
    );
    let e = spawn_external_terminal(&["/nonexistent/term".into()], &BTreeMap::new()).unwrap_err();
    assert!(
        e.contains("No such file") && e.contains("/nonexistent/term"),
        "{e}"
    );
}

fn xdotool_env(d: &Path) -> BTreeMap<OsString, OsString> {
    let mut path = OsString::from(d);
    path.push(":");
    path.push(std::env::var_os("PATH").unwrap_or_default());
    BTreeMap::from([("PATH".into(), path)])
}

const XDOTOOL: &str = r#"
case "$1" in
  search)
    if [ "$2" = "--pid" ]; then
      case "$3" in
        4242) printf '10\n20\n' ;;
        5) printf '30\n' ;;
        *) exit 1 ;;
      esac
    else
      printf 'garbage\n77\n'
    fi ;;
  getwindowgeometry)
    case "$3" in
      20) printf 'WINDOW=20\nX=5\nY=6\nWIDTH=700\nHEIGHT=400\nSCREEN=0\n' ;;
      77) printf 'X=1\nY=2\nWIDTH=3\nHEIGHT=4\n' ;;
      30) printf 'X=1\nY=2\nWIDTH=3\n' ;;
      *) exit 1 ;;
    esac ;;
esac
"#;

#[test]
fn follow_by_pid_then_name_fallback_and_failures() {
    let d = dir("xdo");
    script(&d.join("xdotool"), XDOTOOL);
    let env = xdotool_env(&d);
    // The last id of the pid search wins.
    assert_eq!(follow_window_with_env(4242, &env), Some((5, 6, 700, 400)));
    // pid search fails -> the name heuristic; pid 0 goes there directly.
    assert_eq!(follow_window_with_env(1, &env), Some((1, 2, 3, 4)));
    assert_eq!(follow_window_with_env(0, &env), Some((1, 2, 3, 4)));
    // Incomplete geometry.
    assert_eq!(follow_window_with_env(5, &env), None);
    std::fs::remove_dir_all(&d).unwrap();
}

#[test]
fn follow_geometry_call_times_out_after_two_seconds() {
    let d = dir("xdo-slow");
    // The search yields window 99; its geometry call hangs.
    script(
        &d.join("xdotool"),
        "case \"$1\" in search) echo 99 ;; *) sleep 10 ;; esac",
    );
    let t0 = Instant::now();
    assert_eq!(follow_window_with_env(7, &xdotool_env(&d)), None);
    let e = t0.elapsed();
    assert!(
        e >= Duration::from_secs(2) && e < Duration::from_secs(4),
        "{e:?}"
    );
    std::fs::remove_dir_all(&d).unwrap();
}

#[test]
fn follow_without_xdotool_is_none() {
    let d = dir("xdo-none");
    let env = BTreeMap::from([(OsString::from("PATH"), OsString::from(&d))]);
    assert_eq!(follow_window_with_env(1, &env), None);
    std::fs::remove_dir_all(&d).unwrap();
}
