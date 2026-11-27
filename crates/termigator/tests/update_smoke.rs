//! Update check against a local "latest release" file (`TERMIGATOR_UPDATE_URL`): banner, skip,
//! the welcome box's check box, and nothing at all with `window.check_updates: false`.

mod common;

const LATEST: &str = r#"{"tag_name": "v9.9.9", "html_url": "https://github.com/Ernold11/Termigator/releases/tag/v9.9.9", "draft": false, "prerelease": false}"#;

fn run(name: &str, config_yaml: &str) -> (String, std::path::PathBuf) {
    let file =
        std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("{name}-latest.json"));
    std::fs::write(&file, LATEST).expect("latest.json");
    let url = format!("file://{}", file.display());
    let mut run = common::launch_env(
        name,
        config_yaml,
        &["--smoke-check=shell/UpdateCheck.qml"],
        &[
            ("TERMIGATOR_UPDATE_URL", &url),
            ("QML_XHR_ALLOW_FILE_READ", "1"),
        ],
    );
    let status = run.wait(60, |_| {});
    let log = run.log();
    assert!(status.success(), "exit {status}, stderr:\n{log}");
    (log, run.config.clone())
}

#[test]
fn newer_release_shows_the_banner_and_skip_is_remembered() {
    let (log, config) = run("update-on", "window: {confirm_close: false}\n");
    assert!(log.contains("update: shown 9.9.9"), "{log}");
    assert!(log.contains("update: checkbox ok"), "{log}");
    let state = std::fs::read_to_string(config.join("update-check.json")).expect("state");
    assert!(state.contains(r#""skipped":"9.9.9""#), "{state}");
    // The status bar shows the latest release between checks.
    assert!(
        state.contains(r#""latest":{"#) && state.contains(r#""version":"9.9.9""#),
        "{state}"
    );
}

#[test]
fn switched_off_asks_nothing() {
    let (log, config) = run(
        "update-off",
        "window: {confirm_close: false, check_updates: false}\n",
    );
    assert!(log.contains("update: none"), "{log}");
    assert!(
        !config.join("update-check.json").exists(),
        "checked although switched off"
    );
}
