//! The key matrix of the Ghostty-backed session against `infocmp xterm-256color`.
//!
//! Every session gets `TERM=xterm-256color`; a program that turns on the application keypad and
//! cursor keys (`smkx`) expects the capabilities terminfo declares. The engine has to produce the
//! same bytes as the old xterm-compatible keytab did (port of
//! `keytab_matches_xterm_terminfo`, which drove the vendored QMLTermWidget through QML).
//!
//! The control keys are guaranteed: with the terminal focused, Ctrl+letter and the
//! punctuation that types a C0 byte always reach the pty. That part needs no terminfo — it is
//! ASCII, and Qt delivers the character in `text`.

use std::process::Command;

use termigator_term_bridge::session::SessionEngine;

/// Qt's key codes (Qt::Key_*): the C++ view passes them straight through `encode_qt_key`.
const QT_KEY_HOME: i32 = 0x0100_0010;
const QT_KEY_END: i32 = 0x0100_0011;
const QT_KEY_F1: i32 = 0x0100_0030;
const QT_KEY_F5: i32 = 0x0100_0034;
const QT_KEY_F12: i32 = 0x0100_003b;
const QT_CTRL: u32 = 0x0400_0000;
const QT_SHIFT: u32 = 0x0200_0000;

/// What `infocmp -1 xterm-256color` declares for the capabilities, or `None` without infocmp.
/// Only `\E` occurs in these values.
fn terminfo(caps: &[&str]) -> Option<Vec<Vec<u8>>> {
    let out = Command::new("infocmp")
        .args(["-1", "xterm-256color"])
        .output()
        .ok()?;
    assert!(out.status.success(), "infocmp xterm-256color failed");
    let text = String::from_utf8_lossy(&out.stdout);
    Some(
        caps.iter()
            .map(|cap| {
                let prefix = format!("{cap}=");
                let value = text
                    .lines()
                    .find_map(|line| line.trim().strip_prefix(&prefix)?.strip_suffix(','))
                    .unwrap_or_else(|| panic!("infocmp xterm-256color has no {cap}"));
                value.replace("\\E", "\x1b").into_bytes()
            })
            .collect(),
    )
}

#[test]
fn function_and_navigation_keys_follow_xterm_terminfo() {
    let Some(expected) = terminfo(&["kf1", "kf5", "kf12", "khome", "kend"]) else {
        eprintln!("key matrix: skipped, infocmp (ncurses) is not installed");
        return;
    };
    let mut engine = SessionEngine::new(80, 24, None).expect("engine");
    // smkx: application cursor keys (DECCKM) and keypad (DECKPAM), as every terminfo program sends.
    engine.feed(b"\x1b[?1h\x1b=");
    let keys = [
        (QT_KEY_F1, "F1"),
        (QT_KEY_F5, "F5"),
        (QT_KEY_F12, "F12"),
        (QT_KEY_HOME, "Home"),
        (QT_KEY_END, "End"),
    ];
    for ((key, name), want) in keys.into_iter().zip(&expected) {
        assert_eq!(
            &engine.encode_qt_key(key, 0, ""),
            want,
            "{name}: {} != terminfo {}",
            String::from_utf8_lossy(&engine.encode_qt_key(key, 0, "")),
            String::from_utf8_lossy(want)
        );
    }
}

#[test]
fn control_keys_reach_the_pty() {
    let mut engine = SessionEngine::new(80, 24, None).expect("engine");
    // Ctrl+A..Ctrl+Z are the C0 control bytes; text carries what the keyboard layout produced.
    // Ghostty keeps Ctrl+I, Ctrl+M and Ctrl+[ out of that map on purpose (fixterms, like Kitty):
    // they arrive as CSI-u so a program can tell them from Tab, Enter and Esc. Konsole sends
    // 0x09/0x0d/0x1b there, so this is a known difference — the named keys still send the C0
    // bytes (see `plain_text_and_cursor` and the keypad checks above).
    for letter in b'a'..=b'z' {
        let qt_key = i32::from(letter.to_ascii_uppercase());
        let text = char::from(letter).to_string();
        let got = engine.encode_qt_key(qt_key, QT_CTRL, &text);
        let want: Vec<u8> = match letter {
            b'i' => b"\x1b[105;5u".to_vec(),
            b'm' => b"\x1b[109;5u".to_vec(),
            other => vec![other & 0x1f],
        };
        assert_eq!(got, want, "Ctrl+{}", char::from(letter));
    }
    // The punctuation that must reach the pty (never taken by window shortcuts); Ctrl+[ is the
    // third fixterms exception.
    for (text, mods, want) in [
        ("@", QT_CTRL, b"\x00".to_vec()),
        ("\\", QT_CTRL, b"\x1c".to_vec()),
        ("]", QT_CTRL, b"\x1d".to_vec()),
        ("^", QT_CTRL, b"\x1e".to_vec()),
        ("_", QT_CTRL | QT_SHIFT, b"\x1f".to_vec()),
        ("[", QT_CTRL, b"\x1b[91;5u".to_vec()),
    ] {
        let qt_key = i32::from(text.as_bytes()[0].to_ascii_uppercase());
        assert_eq!(
            engine.encode_qt_key(qt_key, mods, text),
            want,
            "Ctrl+{text}"
        );
    }
}
