use super::*;

fn engine() -> Engine {
    Engine::new(20, 5, None, (8, 16)).expect("engine")
}

const CELL: (u16, u16) = (8, 16);

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
    // Windows line ends are one Enter each, not two.
    assert_eq!(engine.encode_paste("one\r\ntwo\r\n", false), b"one\rtwo\r");
    let bytes = engine.encode_paste("one\r\ntwo", true);
    assert_eq!(bytes, b"\x1b[200~one\ntwo\x1b[201~");
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

#[test]
fn wheel_scrolls_the_scrollback_on_the_primary_screen() {
    let mut engine = engine();
    assert_eq!(engine.wheel_target(false), WheelTarget::Scrollback);
    assert!(
        engine
            .encode_wheel(5, 2, -3, Mods::default(), CELL)
            .is_empty()
    );
    // Mouse tracking on the primary screen (a shell with mouse support) still gets the wheel.
    engine.feed(b"\x1b[?1000h");
    assert_eq!(engine.wheel_target(false), WheelTarget::Report);
}

#[test]
fn wheel_on_the_alternate_screen_without_tracking_sends_cursor_keys() {
    // less, man, vim without 'mouse', k9s with the mouse off: the alternate screen has no
    // scrollback, so the wheel becomes Up/Down keys (xterm's alternateScroll, DECSET 1007).
    for enter in [&b"\x1b[?1049h"[..], b"\x1b[?1047h", b"\x1b[?47h"] {
        let mut engine = engine();
        engine.feed(enter);
        assert_eq!(engine.wheel_target(false), WheelTarget::Arrows, "{enter:?}");
        assert_eq!(
            engine.encode_wheel(0, 0, -2, Mods::default(), CELL),
            b"\x1b[A\x1b[A"
        );
        assert_eq!(
            engine.encode_wheel(0, 0, 3, Mods::default(), CELL),
            b"\x1b[B\x1b[B\x1b[B"
        );
        // Application cursor keys (DECCKM), as vim and less set them.
        engine.feed(b"\x1b[?1h");
        assert_eq!(
            engine.encode_wheel(0, 0, -1, Mods::default(), CELL),
            b"\x1bOA"
        );
    }
}

#[test]
fn alternate_scroll_off_keeps_the_wheel_local() {
    let mut engine = engine();
    engine.feed(b"\x1b[?1049h\x1b[?1007l");
    assert_eq!(engine.wheel_target(false), WheelTarget::Scrollback);
    assert!(
        engine
            .encode_wheel(0, 0, -1, Mods::default(), CELL)
            .is_empty()
    );
}

#[test]
fn wheel_is_reported_in_the_format_the_program_chose() {
    let mut engine = engine();
    // k9s (tcell) and neovim with 'mouse=a': tracking plus SGR coordinates.
    for tracking in [&b"\x1b[?1000h"[..], b"\x1b[?1002h", b"\x1b[?1003h"] {
        engine.feed(b"\x1b[?1049h\x1b[?1006h");
        engine.feed(tracking);
        assert_eq!(
            engine.wheel_target(false),
            WheelTarget::Report,
            "{tracking:?}"
        );
        // One report per notch, a press only (buttons 4 and 5 have no release), 1-based cells.
        assert_eq!(
            engine.encode_wheel(4, 2, -1, Mods::default(), CELL),
            b"\x1b[<64;5;3M"
        );
        assert_eq!(
            engine.encode_wheel(4, 2, 2, Mods::default(), CELL),
            b"\x1b[<65;5;3M\x1b[<65;5;3M"
        );
        engine.feed(b"\x1b[?1000l\x1b[?1002l\x1b[?1003l\x1b[?1006l\x1b[?1049l");
    }
    // The X11 format without 1006: button 64 + 32, column and row + 33.
    engine.feed(b"\x1b[?1000h");
    assert_eq!(
        engine.encode_wheel(4, 2, -1, Mods::default(), CELL),
        b"\x1b[M`%#"
    );
    // Ctrl is part of the button code (16).
    engine.feed(b"\x1b[?1006h");
    let ctrl = Mods {
        ctrl: true,
        ..Mods::default()
    };
    assert_eq!(engine.encode_wheel(0, 0, 1, ctrl, CELL), b"\x1b[<81;1;1M");
}

fn motion(engine: &mut Engine, x: u16, y: u16, button: Option<MouseButton>) -> Vec<u8> {
    engine
        .encode_mouse(
            MouseInput {
                action: MouseAction::Motion,
                button,
                x,
                y,
                mods: Mods::default(),
            },
            CELL,
        )
        .expect("encode")
}

#[test]
fn a_motion_is_reported_once_per_cell() {
    // Qt sends a move per pixel; a program tracking every motion (DECSET 1003) is told only when
    // the pointer reaches another cell, as xterm and Ghostty do.
    let mut engine = engine();
    engine.feed(b"\x1b[?1003h\x1b[?1006h");
    assert_eq!(motion(&mut engine, 3, 1, None), b"\x1b[<35;4;2M");
    assert!(motion(&mut engine, 3, 1, None).is_empty());
    assert_eq!(motion(&mut engine, 4, 1, None), b"\x1b[<35;5;2M");
    // A drag (DECSET 1002) right after the press, still in the pressed cell, is no news either.
    engine.feed(b"\x1b[?1003l\x1b[?1002h");
    let press = MouseInput {
        action: MouseAction::Press,
        button: Some(MouseButton::Left),
        x: 6,
        y: 2,
        mods: Mods::default(),
    };
    assert_eq!(
        engine.encode_mouse(press, CELL).expect("press"),
        b"\x1b[<0;7;3M"
    );
    let left = Some(MouseButton::Left);
    assert!(motion(&mut engine, 6, 2, left).is_empty());
    assert_eq!(motion(&mut engine, 7, 2, left), b"\x1b[<32;8;3M");
    // Tracking switched off and on again: the first motion counts, wherever it is.
    engine.feed(b"\x1b[?1002l");
    assert!(motion(&mut engine, 7, 2, left).is_empty());
    engine.feed(b"\x1b[?1002h");
    assert_eq!(motion(&mut engine, 7, 2, left), b"\x1b[<32;8;3M");
}

#[test]
fn pixel_reports_keep_every_motion() {
    // SGR-Pixels (DECSET 1016) reports positions inside a cell, so no motion is dropped.
    let mut engine = engine();
    engine.feed(b"\x1b[?1003h\x1b[?1016h");
    assert!(!motion(&mut engine, 3, 1, None).is_empty());
    assert!(!motion(&mut engine, 3, 1, None).is_empty());
}

#[test]
fn shift_wheel_bypasses_the_program() {
    let mut engine = engine();
    engine.feed(b"\x1b[?1049h\x1b[?1000h\x1b[?1006h");
    assert_eq!(engine.wheel_target(true), WheelTarget::Scrollback);
    let shift = Mods {
        shift: true,
        ..Mods::default()
    };
    assert!(engine.encode_wheel(0, 0, -1, shift, CELL).is_empty());
}

#[test]
fn bold_ansi_colours_are_drawn_bright() {
    let mut engine = engine();
    engine.feed(
        b"\x1b[1;30mA\x1b[0m\x1b[30mB\x1b[0m\x1b[1;37mC\x1b[0m\x1b[1;91mD\x1b[1;38;2;1;2;3mE",
    );
    let snapshot = engine.snapshot().expect("snapshot");
    let palette = snapshot.colors.palette;
    let fg = |x: usize| snapshot.row_data[0].cells[x].style.fg;
    assert_eq!(fg(0), Some(palette[8]), "bold black is bright black");
    assert_eq!(fg(1), Some(palette[0]), "plain black stays");
    assert_eq!(fg(2), Some(palette[15]), "bold white is bright white");
    assert_eq!(fg(3), Some(palette[9]), "bright red stays bright red");
    assert_eq!(
        fg(4),
        Some(Rgb { r: 1, g: 2, b: 3 }),
        "truecolor is left alone"
    );
}

#[test]
fn the_cursor_is_hidden_while_the_view_shows_the_history() {
    let mut engine = engine();
    for i in 0..20 {
        engine.feed(format!("line{i}\r\n").as_bytes());
    }
    assert!(engine.snapshot().expect("snapshot").cursor.visible);
    engine.scroll(-5);
    assert!(
        !engine.snapshot().expect("snapshot").cursor.visible,
        "drawn at (0, 0)"
    );
    engine.scroll_to_bottom();
    assert!(engine.snapshot().expect("snapshot").cursor.visible);
}
