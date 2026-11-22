use super::*;

#[test]
fn find_next_selects_the_match_in_the_screen() {
    let mut engine = SessionEngine::new(20, 5, None).expect("engine");
    engine.feed(b"one\r\ntwo beta here\r\nthree");
    assert!(engine.find_next("beta", false, false, true, false));
    assert_eq!(engine.selected_text().trim(), "beta");
    assert!(engine.find_next("beta", false, false, true, false));
    assert_eq!(engine.selected_text().trim(), "beta");
    assert!(!engine.find_next("missing", false, false, true, false));
}

#[test]
fn binary_snapshot_has_the_documented_header_and_arena() {
    let mut engine = SessionEngine::new(10, 3, None).expect("engine");
    engine.feed(b"hello");
    let bytes = engine.snapshot_binary();
    assert!(bytes.len() > 20);
    assert_eq!(&bytes[0..4], &0x4753_5031u32.to_be_bytes());
    assert_eq!(u16::from_be_bytes([bytes[4], bytes[5]]), 10);
    assert_eq!(u16::from_be_bytes([bytes[6], bytes[7]]), 3);
    assert!(bytes.windows(5).any(|w| w == b"hello"));
}

#[test]
fn binary_snapshot_keeps_the_columns_a_prompt_skips_with_cursor_movement() {
    let mut engine = SessionEngine::new(20, 2, None).expect("engine");
    // A shell's right prompt: CSI 10 C jumps over empty cells before printing.
    engine.feed(b"L> %\x1b[10C[right]");
    let bytes = engine.snapshot_binary();
    let mut at = 34; // header (see snapshot_binary's layout)
    let mut cells = Vec::new();
    for _ in 0..2 {
        let count =
            u32::from_be_bytes([bytes[at + 1], bytes[at + 2], bytes[at + 3], bytes[at + 4]]);
        at += 5;
        for _ in 0..count {
            let x = u16::from_be_bytes([bytes[at], bytes[at + 1]]);
            let mut p = at + 5;
            let has_fg = bytes[p];
            p += 1 + usize::from(has_fg != 0) * 3;
            let has_bg = bytes[p];
            p += 1 + usize::from(has_bg != 0) * 3;
            let off =
                u32::from_be_bytes([bytes[p], bytes[p + 1], bytes[p + 2], bytes[p + 3]]) as usize;
            let len = u16::from_be_bytes([bytes[p + 4], bytes[p + 5]]) as usize;
            at = p + 6;
            cells.push((x, off, len));
        }
    }
    let arena = &bytes[at..];
    let texts: Vec<(u16, String)> = cells
        .iter()
        .map(|(x, off, len)| {
            (
                *x,
                String::from_utf8_lossy(&arena[*off..*off + *len]).to_string(),
            )
        })
        .collect();
    // "L> %" is 4 cells, so the CSI 10 C gap puts the next glyph in column 14.
    assert!(
        texts.iter().any(|(x, text)| text == "[" && *x == 14),
        "{texts:?}"
    );
}

#[test]
fn konsole_scheme_sets_defaults_and_the_ansi_palette() {
    let mut engine = SessionEngine::new(20, 4, None).expect("engine");
    let ini = konsole::parse_ini(
        "[Background]\nColor=35,38,39\n[Foreground]\nColor=252,252,252\n\
             [Color1]\nColor=237,21,21\n[Color1Intense]\nColor=192,57,43\n",
    )
    .expect("ini");
    assert!(engine.apply_ini(&ini));
    let snapshot = engine.engine.snapshot().expect("snapshot");
    assert_eq!(
        snapshot.colors.background,
        Rgb {
            r: 35,
            g: 38,
            b: 39
        }
    );
    assert_eq!(
        snapshot.colors.foreground,
        Rgb {
            r: 252,
            g: 252,
            b: 252
        }
    );
    assert_eq!(
        snapshot.colors.palette[1],
        Rgb {
            r: 237,
            g: 21,
            b: 21
        }
    );
    assert_eq!(
        snapshot.colors.palette[9],
        Rgb {
            r: 192,
            g: 57,
            b: 43
        }
    );
}

/// Every Ctrl+letter (and the punctuation that makes C0 bytes) reaches the program as its
/// control byte, with the text Qt really sends (the byte itself): Ctrl+D ends a session,
/// Ctrl+Z suspends, Ctrl+R searches history, k9s/vim/tmux use many of them.
#[test]
fn every_ctrl_key_sends_its_control_byte() {
    let ctrl = 0x0400_0000;
    let mut engine = SessionEngine::new(20, 4, None).expect("engine");
    for (i, letter) in (b'A'..=b'Z').enumerate() {
        let byte = u8::try_from(i + 1).expect("1..=26");
        let text = char::from(byte).to_string();
        assert_eq!(
            engine.encode_qt_key(i32::from(letter), ctrl, &text),
            vec![byte],
            "Ctrl+{}",
            char::from(letter)
        );
        // The same with Shift held (Qt still sends the byte).
        assert_eq!(
            engine.encode_qt_key(i32::from(letter), ctrl | 0x0200_0000, &text),
            vec![byte],
            "Ctrl+Shift+{}",
            char::from(letter)
        );
    }
    for (key, text, byte) in [
        (0x20, "\0", 0x00),
        (0x5b, "\x1b", 0x1b),
        (0x5c, "\x1c", 0x1c),
        (0x5d, "\x1d", 0x1d),
        (0x5e, "\x1e", 0x1e),
        (0x5f, "\x1f", 0x1f),
    ] {
        assert_eq!(
            engine.encode_qt_key(key, ctrl, text),
            vec![byte],
            "Ctrl+key {key:#x}"
        );
    }
}

#[test]
fn qt_keymapper_handles_text_modifiers_and_special_keys() {
    let mut engine = SessionEngine::new(20, 4, None).expect("engine");
    // Ctrl+C is a control byte, not the letter.
    assert_eq!(engine.encode_qt_key(0x43, 0x0400_0000, "c"), b"\x03");
    // Qt delivers the control byte itself as the event text for Ctrl+letter.
    assert_eq!(engine.encode_qt_key(0x43, 0x0400_0000, "\x03"), b"\x03");
    assert_eq!(engine.encode_qt_key(0x44, 0x0400_0000, "\x04"), b"\x04");
    assert_eq!(engine.encode_qt_key(0x5a, 0x0400_0000, "\x1a"), b"\x1a");
    // Lowercase and uppercase letters keep the user's text.
    assert_eq!(engine.encode_qt_key(0x41, 0, "a"), b"a");
    assert_eq!(engine.encode_qt_key(0x41, 0x0200_0000, "A"), b"A");
    // Special keys come from the application-cursor table after the mode is set.
    assert_eq!(engine.encode_qt_key(0x0100_0013, 0, ""), b"\x1b[A");
    engine.feed(b"\x1b[?1h");
    assert_eq!(engine.encode_qt_key(0x0100_0013, 0, ""), b"\x1bOA");
    // Return and backward delete.
    assert_eq!(engine.encode_qt_key(0x0100_0004, 0, "\r"), b"\r");
    assert_eq!(engine.encode_qt_key(0x0100_0003, 0, ""), b"\x7f");
    // Tab and Escape arrive with a control character as text and still encode as themselves.
    assert_eq!(engine.encode_qt_key(0x0100_0001, 0, "\t"), b"\t");
    assert_eq!(engine.encode_qt_key(0x0100_0000, 0, "\x1b"), b"\x1b");
}
