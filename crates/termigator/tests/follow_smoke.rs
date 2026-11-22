//! `window.follow_konsole` offscreen (`qml/shell/FollowSmoke.qml`): a stub `xdotool` first in
//! `PATH` reports a terminal window; the Termigator window must move under it, and must not move
//! (nor start the timer) when the key is off.

mod common;

use std::fs;

use termigator as _;

const XDOTOOL: &str = "#!/bin/sh
case \"$1\" in
  search) echo 77 ;;
  getwindowgeometry) printf 'WINDOW=77\\nX=50\\nY=60\\nWIDTH=700\\nHEIGHT=400\\n' ;;
esac
";

fn stub_path(name: &str) -> String {
    let dir = std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("{name}-bin"));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("stub dir");
    let tool = dir.join("xdotool");
    common::write_executable(&tool, XDOTOOL);
    format!(
        "{}:{}",
        dir.display(),
        std::env::var("PATH").unwrap_or_default()
    )
}

fn run(name: &str, config: &str, args: &[&str]) -> String {
    let path = stub_path(name);
    let mut run = common::launch_env(name, config, args, &[("PATH", &path)]);
    let status = run.wait(60, |_| {});
    let log = run.log();
    assert!(status.success(), "exit {status}, stderr:\n{log}");
    log
}

#[test]
fn window_follows_the_terminal() {
    let log = run(
        "follow-on",
        "window:\n  width: 400\n  follow_konsole: true\n  follow_poll_ms: 100\n",
        &["--smoke-check=shell/FollowSmoke.qml"],
    );
    assert!(log.contains("termigator follow: ok"), "stderr:\n{log}");
}

#[test]
fn window_stays_when_follow_is_off() {
    run(
        "follow-off",
        "window:\n  width: 400\n  follow_konsole: false\n",
        &["--smoke-check=shell/FollowSmoke.qml", "--follow-off"],
    );
}
