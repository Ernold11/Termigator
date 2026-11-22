//! Spike: exercises the neutral engine facade without Qt.
//!
//! `cargo run --release --example ghostty_spike` runs the correctness checks and prints one line
//! per check. `--bench` adds a feed-throughput and snapshot benchmark over synthetic output.
//!
//! The checks cover what matters for the engine: wide characters and emoji, reflow on resize,
//! truecolor and OSC 7/8/52, DECSCUSR, key and mouse encoding, scrollback, history text and
//! `clear()`. Drawing and RSS with a real Qt view are measured separately.

use std::time::Instant;

use termigator_term_bridge::engine::{
    Engine, EngineEvent, Key, KeyInput, Mods, MouseAction, MouseButton, MouseInput, TextFormat,
};

fn main() {
    let mut failures = 0;
    for (name, check) in CHECKS {
        match check() {
            Ok(detail) => println!("PASS {name}: {detail}"),
            Err(detail) => {
                failures += 1;
                println!("FAIL {name}: {detail}");
            }
        }
    }
    if std::env::args().any(|arg| arg == "--bench") {
        bench();
    }
    if failures > 0 {
        eprintln!("{failures} check(s) failed");
        std::process::exit(1);
    }
    println!("all checks passed");
}

type Check = fn() -> Result<String, String>;

const CHECKS: &[(&str, Check)] = &[
    ("wide-and-emoji", wide_and_emoji),
    ("reflow", reflow),
    ("truecolor", truecolor),
    ("osc-7-8-52", osc_events),
    ("decscusr", decscusr),
    ("keys", keys),
    ("mouse", mouse),
    ("scrollback", scrollback),
    ("clear", clear),
];

fn engine() -> Result<Engine, String> {
    Engine::new(40, 8, Some(1_000), (8, 16)).map_err(|e| e.to_string())
}

fn row_text(engine: &mut Engine, row: usize) -> Result<String, String> {
    let snapshot = engine.snapshot().map_err(|e| e.to_string())?;
    Ok(snapshot
        .row_data
        .get(row)
        .map(|r| r.cells.iter().map(|c| c.text.as_str()).collect())
        .unwrap_or_default())
}

fn wide_and_emoji() -> Result<String, String> {
    let mut e = engine()?;
    // Half-width, Polish, CJK (2 columns), emoji with ZWJ and a variation selector.
    e.feed("aą漢🙂👨‍👩‍👧‍👦b".as_bytes());
    let snapshot = e.snapshot().map_err(|e| e.to_string())?;
    let cells = &snapshot.row_data[0].cells;
    let text: String = cells.iter().map(|c| c.text.as_str()).collect();
    if !text.starts_with("aą漢🙂👨‍👩‍👧‍👦b") {
        return Err(format!("graphemes lost: {text:?}"));
    }
    let han = cells.iter().find(|c| c.text == "漢").ok_or("no CJK cell")?;
    if han.width != 2 {
        return Err(format!("CJK width {} != 2", han.width));
    }
    let emoji = cells
        .iter()
        .find(|c| c.text.starts_with('🙂'))
        .ok_or("no emoji cell")?;
    if emoji.width != 2 {
        return Err(format!("emoji width {} != 2", emoji.width));
    }
    let family = cells
        .iter()
        .find(|c| c.text.starts_with('👨'))
        .ok_or("no ZWJ family cell")?;
    if family.text.chars().count() < 7 {
        return Err(format!("ZWJ cluster split: {:?}", family.text));
    }
    Ok(format!(
        "{} cells, CJK={}px emoji={}px, cluster={} codepoints",
        cells.len(),
        han.width,
        emoji.width,
        family.text.chars().count()
    ))
}

fn reflow() -> Result<String, String> {
    let mut e = Engine::new(10, 4, None, (8, 16)).map_err(|e| e.to_string())?;
    e.feed(b"0123456789abc");
    let narrow = row_text(&mut e, 0)?;
    let wrapped = e.snapshot().map_err(|e| e.to_string())?.row_data[0].wrapped;
    if narrow != "0123456789" || !wrapped {
        return Err(format!("10 cols: {narrow:?} wrapped={wrapped}"));
    }
    e.resize(20, 4, (8, 16)).map_err(|e| e.to_string())?;
    let wide = row_text(&mut e, 0)?;
    if wide != "0123456789abc" {
        return Err(format!("20 cols after reflow: {wide:?}"));
    }
    // Reflow of a full line that was pushed into scrollback, too.
    for i in 0..20 {
        e.feed(format!("line-{i:02}\r\n").as_bytes());
    }
    e.resize(14, 4, (8, 16)).map_err(|e| e.to_string())?;
    let text = e.history_text(false).map_err(|e| e.to_string())?;
    if !text.contains("0123456789abc") || !text.contains("line-19") {
        return Err(format!("history lost lines on shrink: {text:?}"));
    }
    Ok("10 -> 20 columns unwraps; shrink keeps history".into())
}

fn truecolor() -> Result<String, String> {
    let mut e = engine()?;
    e.feed(b"\x1b[38;2;1;2;3;48;2;250;251;252mX\x1b[0m");
    let snapshot = e.snapshot().map_err(|e| e.to_string())?;
    let cell = snapshot.row_data[0].cells.first().ok_or("no cell")?;
    let fg = cell.style.fg.ok_or("no foreground")?;
    let bg = cell.style.bg.ok_or("no background")?;
    if (fg.r, fg.g, fg.b) != (1, 2, 3) || (bg.r, bg.g, bg.b) != (250, 251, 252) {
        return Err(format!("fg={fg:?} bg={bg:?}"));
    }
    // Palette colours resolve too: 256-colour index 196 is bright red.
    e.feed(b"\x1b[38;5;196mY\x1b[0m");
    let snapshot = e.snapshot().map_err(|e| e.to_string())?;
    let y = snapshot.row_data[0].cells.get(1).ok_or("no second cell")?;
    let fg = y.style.fg.ok_or("no palette foreground")?;
    if fg.r < 200 || fg.g > 40 || fg.b > 40 {
        return Err(format!("palette 196 resolved to {fg:?}"));
    }
    Ok(format!(
        "truecolor {:?}/{:?}, palette 196 {:?}",
        (1, 2, 3),
        (250, 251, 252),
        fg
    ))
}

fn osc_events() -> Result<String, String> {
    let mut e = engine()?;
    e.feed(b"\x1b]7;file:///home/user\x1b\\");
    e.feed(b"\x1b]8;;https://example.com/x\x1b\\link\x1b]8;;\x1b\\");
    e.feed(b"\x1b]52;c;SGVsbG8=\x07");
    let events = e.events();
    let pwd = events
        .iter()
        .find_map(|event| match event {
            EngineEvent::PwdChanged(pwd) => Some(pwd.clone()),
            _ => None,
        })
        .ok_or("no OSC 7 event")?;
    if pwd != "file:///home/user" {
        return Err(format!("OSC 7 = {pwd:?}"));
    }
    let clipboard = events
        .iter()
        .find_map(|event| match event {
            EngineEvent::ClipboardWrite { contents, .. } => contents.first().cloned(),
            _ => None,
        })
        .ok_or("no OSC 52 event")?;
    if clipboard.1 != b"Hello" {
        return Err(format!(
            "OSC 52 = {:?}",
            String::from_utf8_lossy(&clipboard.1)
        ));
    }
    let vt = e.text(TextFormat::Vt, false).map_err(|e| e.to_string())?;
    if !vt.contains("https://example.com/x") {
        return Err(format!("OSC 8 link missing from VT text: {vt:?}"));
    }
    Ok("OSC 7 pwd, OSC 8 link in VT text, OSC 52 Hello".into())
}

fn decscusr() -> Result<String, String> {
    let mut e = engine()?;
    e.feed(b"\x1b[2 q");
    let block = e.snapshot().map_err(|e| e.to_string())?.cursor;
    e.feed(b"\x1b[5 q");
    let bar = e.snapshot().map_err(|e| e.to_string())?.cursor;
    e.feed(b"\x1b[4 q");
    let underline = e.snapshot().map_err(|e| e.to_string())?.cursor;
    if block.shape != termigator_term_bridge::engine::CursorShape::Block
        || block.blinking
        || bar.shape != termigator_term_bridge::engine::CursorShape::Bar
        || !bar.blinking
        || underline.shape != termigator_term_bridge::engine::CursorShape::Underline
        || underline.blinking
    {
        return Err(format!(
            "block={block:?} bar={bar:?} underline={underline:?}"
        ));
    }
    Ok("steady block, blinking bar, steady underline".into())
}

fn keys() -> Result<String, String> {
    let mut e = engine()?;
    let key = |key, mods: Mods, text: Option<&str>| KeyInput {
        key,
        mods,
        text: text.map(str::to_owned),
        ..KeyInput::default()
    };
    let cases = vec![
        (
            key(
                Key::C,
                Mods {
                    ctrl: true,
                    ..Mods::default()
                },
                Some("c"),
            ),
            b"\x03".to_vec(),
        ),
        (key(Key::Enter, Mods::default(), None), b"\r".to_vec()),
        (key(Key::Tab, Mods::default(), None), b"\t".to_vec()),
        (key(Key::Backspace, Mods::default(), None), b"\x7f".to_vec()),
        (key(Key::ArrowUp, Mods::default(), None), b"\x1b[A".to_vec()),
        (key(Key::F1, Mods::default(), None), b"\x1bOP".to_vec()),
        (
            key(
                Key::X,
                Mods {
                    alt: true,
                    ..Mods::default()
                },
                Some("x"),
            ),
            b"\x1bx".to_vec(),
        ),
    ];
    for (input, want) in &cases {
        let got = e.encode_key(input).map_err(|e| e.to_string())?;
        if &got != want {
            return Err(format!("{:?} -> {got:?}, want {want:?}", input.key));
        }
    }
    // Application cursor keys change the encoding without rebuilding the engine.
    e.feed(b"\x1b[?1h");
    let app_up = e
        .encode_key(&key(Key::ArrowUp, Mods::default(), None))
        .map_err(|e| e.to_string())?;
    if app_up != b"\x1bOA" {
        return Err(format!("application ArrowUp -> {app_up:?}"));
    }
    Ok(format!(
        "{} sequences and application cursor mode",
        cases.len()
    ))
}

fn mouse() -> Result<String, String> {
    let mut e = engine()?;
    // SGR encoding, button-event tracking.
    e.feed(b"\x1b[?1002h\x1b[?1006h");
    let press = e
        .encode_mouse(
            MouseInput {
                action: MouseAction::Press,
                button: Some(MouseButton::Left),
                x: 2,
                y: 3,
                mods: Mods::default(),
            },
            (8, 16),
        )
        .map_err(|e| e.to_string())?;
    let release = e
        .encode_mouse(
            MouseInput {
                action: MouseAction::Release,
                button: Some(MouseButton::Left),
                x: 2,
                y: 3,
                mods: Mods::default(),
            },
            (8, 16),
        )
        .map_err(|e| e.to_string())?;
    if press != b"\x1b[<0;3;4M" || release != b"\x1b[<0;3;4m" {
        return Err(format!(
            "press={:?} release={:?}",
            String::from_utf8_lossy(&press),
            String::from_utf8_lossy(&release)
        ));
    }
    // No tracking mode: the view must handle the click itself.
    let mut plain = engine()?;
    let none = plain
        .encode_mouse(
            MouseInput {
                action: MouseAction::Press,
                button: Some(MouseButton::Left),
                x: 1,
                y: 1,
                mods: Mods::default(),
            },
            (8, 16),
        )
        .map_err(|e| e.to_string())?;
    if !none.is_empty() {
        return Err(format!("mouse encoded without tracking: {none:?}"));
    }
    Ok("SGR press/release, disabled when tracking is off".into())
}

fn scrollback() -> Result<String, String> {
    let mut e = Engine::new(10, 2, Some(1_000), (8, 16)).map_err(|e| e.to_string())?;
    for i in 0..50 {
        e.feed(format!("line{i:02} with enough text to wrap\r\n").as_bytes());
    }
    let bar = e.scrollbar().map_err(|e| e.to_string())?;
    if bar.total <= bar.len || bar.offset + bar.len != bar.total {
        return Err(format!("scrollbar {bar:?}"));
    }
    e.scroll_to_top();
    let top = e.scrollbar().map_err(|e| e.to_string())?;
    if top.offset != 0 {
        return Err(format!("scroll to top offset {}", top.offset));
    }
    let text = e.history_text(false).map_err(|e| e.to_string())?;
    if !text.contains("line00") || !text.contains("line49") {
        return Err("history missing first or last line".into());
    }
    e.scroll_to_bottom();
    let wrapped = e.text(TextFormat::Plain, true).map_err(|e| e.to_string())?;
    if wrapped.matches('\n').count() >= text.matches('\n').count() {
        return Err("unwrapped text has no fewer line breaks".into());
    }
    Ok(format!(
        "{} rows total, {} visible, history {} bytes",
        bar.total,
        bar.len,
        text.len()
    ))
}

fn clear() -> Result<String, String> {
    let mut e = engine()?;
    for i in 0..30 {
        e.feed(format!("x{i}\r\n").as_bytes());
    }
    e.clear();
    let bar = e.scrollbar().map_err(|e| e.to_string())?;
    let text = e.history_text(false).map_err(|e| e.to_string())?;
    if bar.total != bar.len || text.contains("x0") {
        return Err(format!("after clear: {bar:?}, text {text:?}"));
    }
    Ok(format!("scrollback dropped, {} rows left", bar.total))
}

/// Synthetic output: numbered lines with some SGR changes, like a build log.
fn make_output(mb: usize) -> Vec<u8> {
    let line = b"\x1b[32mok\x1b[0m line with some text 0123456789 abcdefghijklmnopqrstuvwxyz\r\n";
    let target = mb.saturating_mul(1024 * 1024);
    let mut data = Vec::with_capacity(target);
    while data.len() < target {
        data.extend_from_slice(line);
    }
    data.truncate(target);
    data
}

fn rss(field: &str) -> f64 {
    let status = std::fs::read_to_string("/proc/self/status").unwrap_or_default();
    status
        .lines()
        .find_map(|line| {
            let value = line.strip_prefix(field)?.trim().strip_suffix(" kB")?;
            value.parse::<f64>().ok()
        })
        .map_or(f64::NAN, |kb| kb / 1024.0)
}

fn bench() {
    let args: Vec<String> = std::env::args().collect();
    let mb = args
        .iter()
        .position(|arg| arg == "--mb")
        .and_then(|i| args.get(i + 1))
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(100);
    let sessions = args
        .iter()
        .position(|arg| arg == "--sessions")
        .and_then(|i| args.get(i + 1))
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(8);
    let data = make_output(mb);
    let base_rss = rss("VmRSS:");
    let create_start = Instant::now();
    let mut engines: Vec<Engine> = (0..sessions)
        .map(|_| Engine::new(120, 40, Some(10_000), (8, 16)).expect("engine"))
        .collect();
    let create = create_start.elapsed();
    let start = Instant::now();
    for engine in &mut engines {
        for chunk in data.chunks(64 * 1024) {
            engine.feed(chunk);
            while !engine.events().is_empty() {}
        }
    }
    let feed = start.elapsed();
    let snapshot_start = Instant::now();
    let mut cells = 0usize;
    for engine in &mut engines {
        let snapshot = engine.snapshot().expect("snapshot");
        cells += snapshot
            .row_data
            .iter()
            .map(|row| row.cells.len())
            .sum::<usize>();
    }
    let snap = snapshot_start.elapsed();
    let total_mb = (data.len() as f64 / (1024.0 * 1024.0)) * sessions as f64;
    println!(
        "BENCH engine=ghostty sessions={sessions} input_mb={mb} total_mb={total_mb:.0} \
         create_ms={:.1} feed_s={:.2} throughput_mb_s={:.1} snapshot_ms={:.2} cells={cells} \
         base_rss_mb={base_rss:.1} peak_rss_mb={:.1} end_rss_mb={:.1}",
        create.as_secs_f64() * 1000.0,
        feed.as_secs_f64(),
        total_mb / feed.as_secs_f64(),
        snap.as_secs_f64() * 1000.0,
        rss("VmHWM:"),
        rss("VmRSS:"),
    );
}
