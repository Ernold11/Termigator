//! Golden tests and unit tests for `termigator_core::text::osc7`.

mod common;

use termigator_core::text::osc7::{OSC7_SHELL_SNIPPET, Osc7Scanner};

#[test]
fn matches_golden() {
    let cases = common::golden("text_osc7");
    assert!(cases.len() > 150);
    for c in &cases {
        let n = common::name(c);
        if c["op"] == "snippet" {
            assert_eq!(Some(OSC7_SHELL_SNIPPET), c["expect"].as_str(), "{n}");
            continue;
        }
        let mut sc = Osc7Scanner::new();
        let chunks = c["chunks"].as_array().cloned().unwrap_or_default();
        let expect = c["expect"].as_array().cloned().unwrap_or_default();
        assert_eq!(chunks.len(), expect.len(), "{n}");
        for (i, (ch, ex)) in chunks.iter().zip(&expect).enumerate() {
            let want: Vec<&str> = ex
                .as_array()
                .map(|a| a.iter().filter_map(|s| s.as_str()).collect())
                .unwrap_or_default();
            assert_eq!(
                sc.feed(ch.as_str().unwrap_or_default()),
                want,
                "{n} chunk {i}"
            );
        }
    }
}

#[test]
fn handles_terminators_splits_and_escapes() {
    let mut sc = Osc7Scanner::new();
    assert_eq!(
        sc.feed("hello \x1b]7;file://h/var/log\x07 prompt$ "),
        ["/var/log"]
    );
    assert_eq!(
        sc.feed("\x1b]7;file://h/srv/my%20dir\x1b\\"),
        ["/srv/my dir"]
    );
    assert!(sc.feed("out\x1b]7;file://h/ta").is_empty());
    assert_eq!(sc.feed("il/x\x07"), ["/tail/x"]);
    assert!(sc.feed("abc\x1b").is_empty());
    assert_eq!(sc.feed("]7;file://h/z\x07"), ["/z"]);
    assert!(sc.feed("\x1b]7;relative\x07").is_empty());
}

#[test]
fn tracking_command_carries_the_bootstrap_as_base64() {
    use base64::Engine as _;
    let cmd = termigator_core::text::osc7::tracking_remote_command();
    assert!(cmd.starts_with("sh -c '") && cmd.ends_with("-l'"), "{cmd}");
    // Nothing a fish or csh single-quoted string would treat specially, and one line.
    assert!(!cmd.contains('\\') && !cmd.contains('\n'), "{cmd}");
    assert_eq!(
        cmd.matches('\'').count(),
        2,
        "only the two outer quotes: {cmd}"
    );
    let b64 = cmd
        .split("echo ")
        .nth(1)
        .unwrap()
        .split(' ')
        .next()
        .unwrap();
    let script = base64::engine::general_purpose::STANDARD
        .decode(b64)
        .unwrap();
    assert_eq!(script, include_bytes!("../src/text/osc7_bootstrap.sh"));
}
