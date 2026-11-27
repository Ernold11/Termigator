//! Single instance (`window.single_instance`, `src/lib.rs` + `single_instance::claim`) with the real
//! binary offscreen: a second start hands `show` to the running window and quits before any Qt
//! (no second window or tray icon), whether that window is visible, minimized or in the tray, its
//! local shell untouched; several starts at once all hand over; after a crash the next start gets
//! the window; with the setting off a second start runs on its own.

mod common;

use std::fs::{self, File};
use std::path::Path;
use std::process::{Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};

// cxx-qt's generated C++ is linked into this test too; it needs the library's Rust half.
use termigator as _;

const CHECK: &str = "--smoke-check=shell/SingleInstanceSmoke.qml";

/// Another start with the first one's config and control socket; returns its exit status, stderr
/// and how long it ran.
fn second_start(
    first: &common::Launched,
    name: &str,
    args: &[&str],
) -> (ExitStatus, String, Duration) {
    let log_path = first.config.with_file_name(format!("{name}.log"));
    let started = Instant::now();
    let mut child = Command::new(env!("CARGO_BIN_EXE_termigator"))
        .args(args)
        .env("QT_QPA_PLATFORM", "offscreen")
        .env("QT_QPA_PLATFORMTHEME", "")
        .env("QT_QUICK_BACKEND", "software")
        .env("QT_FORCE_STDERR_LOGGING", "1")
        .env(
            "TERMIGATOR_SMOKE_TEST",
            first.shot.with_file_name(format!("{name}.png")),
        )
        .env("TERMIGATOR_CONFIG_DIR", &first.config)
        .env("TERMIGATOR_CONTROL_SOCKET", &first.socket)
        .stdout(Stdio::null())
        .stderr(File::create(&log_path).expect("log"))
        .spawn()
        .expect("spawn second termigator");
    let status = loop {
        if let Some(s) = child.try_wait().expect("wait") {
            break s;
        }
        if started.elapsed() > Duration::from_secs(60) {
            let _ = child.kill();
            panic!(
                "second start did not quit: {}",
                fs::read_to_string(&log_path).unwrap_or_default()
            );
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    (
        status,
        fs::read_to_string(&log_path).unwrap_or_default(),
        started.elapsed(),
    )
}

fn assert_handed_over(what: &str, (status, log, took): &(ExitStatus, String, Duration)) {
    assert!(status.success(), "{what}: exit {status}, stderr:\n{log}");
    // Quit before Qt: no window, no tray icon, nothing of the app in its log.
    assert!(
        !log.contains("termigator single:"),
        "{what}: started a window:\n{log}"
    );
    assert!(*took < Duration::from_secs(10), "{what}: took {took:?}");
}

fn wait_for(run: &mut common::Launched, line: &str) {
    let deadline = Instant::now() + Duration::from_secs(60);
    while !run.log().contains(line) {
        if let Some(s) = run.child.try_wait().expect("wait") {
            panic!("quit ({s}) before {line:?}, stderr:\n{}", run.log());
        }
        assert!(
            Instant::now() < deadline,
            "no {line:?}, stderr:\n{}",
            run.log()
        );
        std::thread::sleep(Duration::from_millis(100));
    }
}

#[test]
fn second_start_restores_the_running_window_in_every_state() {
    let mut run = common::launch("single", "window: {confirm_close: false}\n", &[CHECK]);
    wait_for(&mut run, "termigator single: ready visible");
    // Several starts at once: all hand over, none becomes a second window.
    let together: Vec<_> = std::thread::scope(|s| {
        let starts: Vec<_> = (0..4)
            .map(|i| {
                let run = &run;
                s.spawn(move || second_start(run, &format!("together{i}"), &[]))
            })
            .collect();
        starts
            .into_iter()
            .map(|t| t.join().expect("join"))
            .collect()
    });
    for (i, r) in together.iter().enumerate() {
        assert_handed_over(&format!("start {i} of 4"), r);
    }
    wait_for(&mut run, "termigator single: shown visible");
    for state in ["minimized", "tray"] {
        wait_for(&mut run, &format!("termigator single: ready {state}"));
        assert_handed_over(state, &second_start(&run, state, &[]));
        wait_for(&mut run, &format!("termigator single: shown {state}"));
    }
    let status = run.wait(60, |_| {});
    let log = run.log();
    assert!(status.success(), "exit {status}, stderr:\n{log}");
    assert!(log.contains("termigator single: done"), "stderr:\n{log}");
    assert_eq!(
        log.matches("termigator single: primary").count(),
        1,
        "stderr:\n{log}"
    );
}

#[test]
fn after_a_crash_the_next_start_gets_the_window() {
    let mut run = common::launch("single-crash", "window: {confirm_close: false}\n", &[CHECK]);
    wait_for(&mut run, "termigator single: ready visible");
    run.child.kill().expect("kill"); // SIGKILL: no cleanup, the socket file stays
    let _ = run.child.wait();
    assert!(
        run.socket.exists(),
        "a killed process leaves its socket file"
    );
    let (status, log, _) = second_start(&run, "after-crash", &[CHECK, "--single-quick"]);
    assert!(status.success(), "exit {status}, stderr:\n{log}");
    assert!(log.contains("termigator single: primary"), "stderr:\n{log}");
    // And once that one closed normally, the place is free again.
    assert!(!run.socket.exists(), "socket left behind");
    let (status, log, _) = second_start(&run, "after-close", &[CHECK, "--single-quick"]);
    assert!(
        status.success() && log.contains("termigator single: primary"),
        "stderr:\n{log}"
    );
}

#[test]
fn with_the_setting_off_a_second_start_runs_on_its_own_until_turned_on() {
    let mut run = common::launch(
        "single-off",
        "window: {confirm_close: false, single_instance: false}\n",
        &[CHECK, "--single-once"],
    );
    wait_for(&mut run, "termigator single: ready visible");
    let (status, log, _) = second_start(&run, "independent", &[CHECK, "--single-quick"]);
    assert!(status.success(), "exit {status}, stderr:\n{log}");
    assert!(
        log.contains("termigator single: primary"),
        "a second window, stderr:\n{log}"
    );
    assert!(!run.log().contains("remote="), "stderr:\n{}", run.log());
    assert!(run.socket.exists(), "the first one keeps its socket");
    // Turned on (Settings writes the same key): the next start goes to the running one.
    set_single_instance(&run.config, true);
    assert_handed_over("after turning on", &second_start(&run, "turned-on", &[]));
    let status = run.wait(60, |_| {});
    assert!(status.success(), "exit {status}, stderr:\n{}", run.log());
    assert!(run.log().contains("termigator single: shown visible"));
}

fn set_single_instance(config: &Path, on: bool) {
    let path = config.join("config.yaml");
    let text = fs::read_to_string(&path).expect("config.yaml");
    let text = text.replace(
        &format!("single_instance: {}", !on),
        &format!("single_instance: {on}"),
    );
    assert!(text.contains(&format!("single_instance: {on}")), "{text}");
    fs::write(path, text).expect("write config.yaml");
}
