use super::*;

fn engine() -> Engine {
    Engine::new(20, 5, None, (8, 16)).expect("engine")
}

fn text_at(snapshot: &Snapshot, row: usize) -> String {
    snapshot.row_data[row]
        .cells
        .iter()
        .map(|cell| cell.text.as_str())
        .collect()
}

#[test]
fn wide_lines_keep_the_configured_scrollback() {
    // 300 columns of text per line: the old 512-byte budget per line kept far fewer.
    let mut engine = Engine::new(300, 5, Some(2000), (8, 16)).expect("engine");
    let line = "x".repeat(290); // 295 characters: one physical row each
    for i in 0..3000 {
        engine.feed(format!("{i:05}{line}\r\n").as_bytes());
    }
    let text = engine.history_text(false).expect("history");
    let first = text.lines().next().unwrap_or_default();
    let oldest: usize = first[..5].parse().expect("line number");
    // The emulator trims whole pages, so a little less than the limit may stay.
    let kept = 3000 - oldest;
    assert!(
        (1800..=2010).contains(&kept),
        "{kept} lines kept, want about 2000"
    );
}

#[test]
fn plain_text_and_cursor() {
    let mut engine = engine();
    engine.feed(b"hello\r\nworld");
    let snapshot = engine.snapshot().expect("snapshot");
    assert_eq!(text_at(&snapshot, 0), "hello");
    assert_eq!(text_at(&snapshot, 1), "world");
    assert_eq!((snapshot.cursor.x, snapshot.cursor.y), (5, 1));
}

#[test]
fn wide_characters_take_two_columns() {
    let mut engine = engine();
    engine.feed("aąb漢c".as_bytes());
    let snapshot = engine.snapshot().expect("snapshot");
    let cells = &snapshot.row_data[0].cells;
    let widths: Vec<u8> = cells.iter().take(5).map(|cell| cell.width).collect();
    assert_eq!(widths, vec![1, 1, 1, 2, 1]);
    assert_eq!(text_at(&snapshot, 0), "aąb漢c");
}

#[test]
fn reflow_rewraps_wrapped_lines() {
    let mut engine = Engine::new(10, 4, None, (8, 16)).expect("engine");
    engine.feed(b"0123456789abc");
    let narrow = engine.snapshot().expect("snapshot");
    assert!(narrow.row_data[0].wrapped);
    assert_eq!(text_at(&narrow, 0), "0123456789");
    assert_eq!(text_at(&narrow, 1), "abc");

    engine.resize(20, 4, (8, 16)).expect("resize");
    let wide = engine.snapshot().expect("snapshot");
    assert_eq!(text_at(&wide, 0), "0123456789abc");
    assert!(!wide.row_data[0].wrapped);
}

#[test]
fn bracketed_paste_wraps_and_plain_paste_normalizes_newlines() {
    let mut engine = engine();
    engine.feed(b"\x1b[?2004h");
    assert!(engine.bracketed_paste());
    let bytes = engine.encode_paste("one\ntwo", engine.bracketed_paste());
    assert!(bytes.starts_with(b"\x1b[200~"));
    assert!(bytes.ends_with(b"\x1b[201~"));
    assert!(bytes.windows(3).any(|w| w == b"one"));
    engine.feed(b"\x1b[?2004l");
    assert!(!engine.bracketed_paste());
    let bytes = engine.encode_paste("one\ntwo", engine.bracketed_paste());
    assert_eq!(bytes, b"one\rtwo");
}

#[test]
fn selection_marks_cells_and_yields_text() {
    let mut engine = engine();
    engine.feed(b"hello world");
    engine.select((0, 0), (4, 0), false).expect("select");
    let snapshot = engine.snapshot().expect("snapshot");
    assert!(snapshot.row_data[0].cells[0].selected);
    assert!(!snapshot.row_data[0].cells[6].selected);
    let text = engine.selected_text().expect("text");
    assert_eq!(text.trim_end(), "hello");
    engine.clear_selection().expect("clear");
    let snapshot = engine.snapshot().expect("snapshot");
    assert!(!snapshot.row_data[0].cells[0].selected);
}

#[test]
fn osc_events_are_reported() {
    let mut engine = engine();
    engine.feed(b"\x1b]2;title\x1b\\\x1b]7;file:///tmp\x1b\\\x07");
    let events = engine.events();
    assert!(events.contains(&EngineEvent::TitleChanged("title".into())));
    assert!(events.contains(&EngineEvent::PwdChanged("file:///tmp".into())));
    assert!(events.contains(&EngineEvent::Bell));
}

#[test]
fn dsr_answer_is_a_pty_write() {
    let mut engine = engine();
    engine.feed(b"\x1b[5n");
    let events = engine.events();
    assert!(matches!(events.as_slice(), [EngineEvent::PtyWrite(_)]));
}

#[test]
fn key_encoder_follows_application_cursor_mode() {
    let mut engine = engine();
    let up = KeyInput {
        key: Key::ArrowUp,
        ..KeyInput::default()
    };
    assert_eq!(engine.encode_key(&up).expect("encode"), b"\x1b[A");
    engine.feed(b"\x1b[?1h");
    assert_eq!(engine.encode_key(&up).expect("encode"), b"\x1bOA");
}

#[test]
fn ctrl_letter_is_a_control_byte() {
    let mut engine = engine();
    let ctrl_c = KeyInput {
        key: Key::C,
        mods: Mods {
            ctrl: true,
            ..Mods::default()
        },
        text: Some("c".into()),
        ..KeyInput::default()
    };
    assert_eq!(engine.encode_key(&ctrl_c).expect("encode"), b"\x03");
}

#[test]
fn scrollback_and_history_text() {
    let mut engine = Engine::new(10, 2, Some(100), (8, 16)).expect("engine");
    for i in 0..10 {
        engine.feed(format!("line{i}\r\n").as_bytes());
    }
    let bar = engine.scrollbar().expect("scrollbar");
    assert!(bar.total > bar.len);
    let text = engine.history_text(false).expect("history");
    assert!(text.contains("line0"));
    assert!(text.contains("line9"));
    engine.scroll_to_top();
    assert_eq!(engine.scrollbar().expect("scrollbar").offset, 0);
}

#[test]
fn input_does_not_grow_scrollback_without_output() {
    let mut engine = engine();
    engine.feed(b"\x1b[999;999H");
    let bar = engine.scrollbar().expect("scrollbar");
    assert_eq!(bar.total, usize::from(engine.size().1));
}
