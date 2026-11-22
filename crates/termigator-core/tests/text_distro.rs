//! Golden tests for `termigator_core::text::distro::detect_distro`.

mod common;

use termigator_core::text::distro::detect_distro;

#[test]
fn matches_golden() {
    let cases = common::golden("text_distro");
    assert!(cases.len() > 100);
    for c in &cases {
        let n = common::name(c);
        assert_eq!(
            detect_distro(c["input"].as_str().unwrap_or_default()),
            c["expect"].as_str(),
            "{n}"
        );
    }
}

#[test]
fn os_release_id_wins_over_names() {
    assert_eq!(
        detect_distro("Welcome to Debian\nID=ubuntu"),
        Some("ubuntu")
    );
    assert_eq!(detect_distro("ID=weird\nWelcome to Debian"), Some("debian"));
    assert_eq!(detect_distro(""), None);
}
