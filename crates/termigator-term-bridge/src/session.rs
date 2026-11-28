//! Session bridge: one engine per terminal session, shared by the view.
//!
//! The snapshot goes to the view as JSON (simple, fast enough so far; a packed row buffer would be
//! the upgrade). PTY ownership stays in Rust (`termigator_core::proc::pty`); this object only
//! parses bytes, encodes input and reports effects.

use std::fs;

use serde_json::{Value, json};
use termigator_core::konsole::{self, XdgDirs};

use crate::engine::{CursorShape, Engine, Rgb, cell_span};

/// Engine handle owned by the session object the terminal view draws.
pub struct SessionEngine {
    engine: Engine,
    scrollback: Option<usize>,
    last_match: Option<(usize, u16)>,
}

impl SessionEngine {
    /// Creates an engine; errors are returned as strings because the C++ side only logs them.
    pub fn new(cols: u16, rows: u16, scrollback: Option<usize>) -> Result<Self, String> {
        // The view reports its real cell size with its first resize.
        let engine = Engine::new(cols, rows, scrollback, (8, 16)).map_err(|e| e.to_string())?;
        Ok(Self {
            engine,
            scrollback,
            last_match: None,
        })
    }

    pub fn feed(&mut self, data: &[u8]) {
        self.engine.feed(data);
    }

    /// Loads the colour scheme from the user's Konsole profile (Breeze when it names none) and
    /// returns the scheme name. This is what makes the terminal look like Konsole by default.
    pub fn apply_konsole_default_scheme(&mut self) -> String {
        let appearance = konsole::default_appearance(&XdgDirs::from_env());
        let name = appearance
            .color_scheme
            .unwrap_or_else(|| konsole::DEFAULT_COLOR_SCHEME.to_owned());
        if self.set_scheme(&name) {
            name
        } else {
            // A profile can name a scheme that was uninstalled; fall back to Konsole's default.
            let fallback = konsole::DEFAULT_COLOR_SCHEME;
            if self.set_scheme(fallback) {
                fallback.to_owned()
            } else {
                String::new()
            }
        }
    }

    /// Applies a scheme from its file contents (used for compiled-in resources).
    pub fn set_scheme_text(&mut self, text: &str) -> bool {
        konsole::parse_ini(text)
            .map(|ini| self.apply_ini(&ini))
            .unwrap_or(false)
    }

    /// Loads `<name>.colorscheme` from the Konsole scheme directories: 16 ANSI colours plus the
    /// default foreground/background. The 256-colour cube keeps Ghostty's defaults because
    /// Konsole schemes only define the first 16 slots.
    pub fn set_scheme(&mut self, name: &str) -> bool {
        let name = name.strip_suffix(".colorscheme").unwrap_or(name);
        for dir in konsole::scheme_dirs(&XdgDirs::from_env()) {
            let path = dir.join(format!("{name}.colorscheme"));
            let Ok(text) = fs::read_to_string(&path) else {
                continue;
            };
            let Some(ini) = konsole::parse_ini(&text) else {
                continue;
            };
            return self.apply_ini(&ini);
        }
        false
    }

    fn apply_ini(&mut self, ini: &konsole::Ini) -> bool {
        let color = |section: &str| -> Option<Rgb> {
            parse_rgb(ini.get(section, "Color").unwrap_or_default())
        };
        let (Some(background), Some(foreground)) = (color("Background"), color("Foreground"))
        else {
            return false;
        };
        let mut palette = Engine::default_palette();
        for i in 0..8 {
            if let Some(value) = color(&format!("Color{i}")) {
                palette[i] = value;
            }
            if let Some(value) = color(&format!("Color{i}Intense")) {
                palette[i + 8] = value;
            }
        }
        if self.engine.set_palette(palette).is_err() {
            return false;
        }
        self.engine
            .set_colors(Some(foreground), Some(background))
            .is_ok()
    }

    /// The grid in cells and the view's cell size in pixels, which the pixel reports
    /// (CSI 14 t / 16 t, in-band size reports) give the program.
    pub fn resize(&mut self, cols: u16, rows: u16, cell_width: u16, cell_height: u16) {
        let cell_px = (cell_width.max(1), cell_height.max(1));
        // Errors would only mean an invalid size, which the caller clamps already.
        let _ = self.engine.resize(cols, rows, cell_px);
    }

    pub fn scroll(&mut self, delta: i32) {
        self.engine.scroll(delta as isize);
    }

    /// The hyperlink or URL-looking token at a viewport cell.
    pub fn url_at(&mut self, col: u16, row: u16) -> String {
        self.engine.url_at(col, row).unwrap_or_default()
    }

    /// Names of the installed Konsole colour schemes, sorted and unique.
    pub fn available_schemes() -> Vec<String> {
        let mut names = Vec::new();
        for dir in konsole::scheme_dirs(&XdgDirs::from_env()) {
            let Ok(entries) = fs::read_dir(dir) else {
                continue;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().is_some_and(|ext| ext == "colorscheme")
                    && let Some(stem) = path.file_stem().and_then(|stem| stem.to_str())
                    && !names.iter().any(|name| name == stem)
                {
                    names.push(stem.to_owned());
                }
            }
        }
        names.sort();
        names
    }

    /// Finds `needle` in the scrollback and screen, scrolls to it and selects it. Literal or
    /// regular-expression matching; `forward` and `current` follow the find bar's buttons/typing.
    pub fn find_next(
        &mut self,
        needle: &str,
        case_sensitive: bool,
        regex: bool,
        forward: bool,
        current: bool,
    ) -> bool {
        let text = self.engine.history_text(false).unwrap_or_default();
        if needle.is_empty() {
            let _ = self.engine.clear_selection();
            self.last_match = None;
            return false;
        }
        // A literal needle is an escaped pattern: matching stays on the original line, so offsets
        // never come from a lower-cased copy whose bytes differ (İ, K: a panic, then an abort).
        let pattern = if regex {
            needle.to_owned()
        } else {
            regex::escape(needle)
        };
        let Ok(re) = regex::RegexBuilder::new(&pattern)
            .case_insensitive(!case_sensitive)
            .build()
        else {
            return false;
        };
        // (line, first column, last column), in cells: wide and combined characters count.
        let mut matches: Vec<(usize, u16, u16)> = Vec::new();
        for (line_no, line) in text.lines().enumerate() {
            for found in re.find_iter(line) {
                if found.start() == found.end() {
                    continue;
                }
                let (start, end) = cell_span(line, found.start(), found.end());
                matches.push((line_no, start, end));
            }
        }
        if matches.is_empty() {
            let _ = self.engine.clear_selection();
            self.last_match = None;
            return false;
        }

        let current_index = self
            .last_match
            .and_then(|(line, col)| matches.iter().position(|(l, c, _)| *l == line && *c == col));
        let index = if current && current_index.is_some() {
            current_index.unwrap_or(0)
        } else if let Some(index) = current_index {
            if forward {
                (index + 1) % matches.len()
            } else if index == 0 {
                matches.len() - 1
            } else {
                index - 1
            }
        } else if forward {
            0
        } else {
            matches.len() - 1
        };

        let (line, start, end) = matches[index];
        let rows = u32::from(self.engine.size().1);
        let top = line.saturating_sub((rows / 2) as usize);
        self.engine.scroll_to_row(top);
        let _ = self
            .engine
            .select_screen((start, line as u32), (end, line as u32), false);
        self.last_match = Some((line, start));
        true
    }

    /// Encodes clipboard text for the pty using the terminal's bracketed-paste mode.
    pub fn paste(&self, text: &str) -> Vec<u8> {
        let bracketed = self.engine.bracketed_paste();
        self.engine.encode_paste(text, bracketed)
    }

    /// Encodes a paste with an explicit bracketed-paste decision (settings override).
    pub fn paste_mode(&self, text: &str, bracketed: bool) -> Vec<u8> {
        self.engine.encode_paste(text, bracketed)
    }

    pub fn bracketed_paste(&self) -> bool {
        self.engine.bracketed_paste()
    }

    pub fn paste_is_safe(text: &str) -> bool {
        Engine::paste_is_safe(text)
    }

    pub fn history_text(&self) -> String {
        self.engine.history_text(false).unwrap_or_default()
    }

    #[allow(clippy::too_many_arguments, reason = "flat cxx bridge signature")]
    pub fn select(
        &mut self,
        start_col: u16,
        start_row: u16,
        end_col: u16,
        end_row: u16,
        rectangle: bool,
    ) {
        let _ = self
            .engine
            .select((start_col, start_row), (end_col, end_row), rectangle);
    }

    /// Selects between two screen rows (0 = the oldest scrollback row, as the scrollbar offset
    /// counts them), so a selection keeps its anchor while the view scrolls.
    #[allow(clippy::too_many_arguments, reason = "flat cxx bridge signature")]
    pub fn select_screen(
        &mut self,
        start_col: u16,
        start_row: u32,
        end_col: u16,
        end_row: u32,
        rectangle: bool,
    ) {
        let _ = self
            .engine
            .select_screen((start_col, start_row), (end_col, end_row), rectangle);
    }

    pub fn select_word(&mut self, col: u16, row: u16) {
        let _ = self.engine.select_word((col, row));
    }

    pub fn select_line(&mut self, col: u16, row: u16) {
        let _ = self.engine.select_line((col, row));
    }

    pub fn clear_selection(&mut self) {
        let _ = self.engine.clear_selection();
    }

    pub fn selected_text(&self) -> String {
        self.engine.selected_text().unwrap_or_default()
    }

    /// `(total, offset, len)` for the scrollbar.
    pub fn scrollbar_ints(&self) -> (i32, i32, i32) {
        match self.engine.scrollbar() {
            Ok(bar) => (bar.total as i32, bar.offset as i32, bar.len as i32),
            Err(_) => (0, 0, 0),
        }
    }

    pub fn scrollbar_total(&self) -> u32 {
        self.scrollbar_ints().0.max(0) as u32
    }

    pub fn scrollbar_offset(&self) -> u32 {
        self.scrollbar_ints().1.max(0) as u32
    }

    pub fn scrollbar_len(&self) -> u32 {
        self.scrollbar_ints().2.max(0) as u32
    }

    pub fn scroll_to_row(&mut self, row: i32) {
        self.engine.scroll_to_row(row.max(0) as usize);
    }

    /// Encodes a mouse event with the session's live modes; empty when reporting is off.
    /// `button` is the button pressed or released; for a motion it is the buttons held (Qt's
    /// `buttons()` mask), so a drag reaches a program that tracks only drags (DECSET 1002).
    #[allow(clippy::too_many_arguments, reason = "flat cxx bridge signature")]
    pub fn encode_mouse(
        &mut self,
        x: u16,
        y: u16,
        button: i32,
        action: i32,
        modifiers: u32,
        cell_size: u32,
    ) -> Vec<u8> {
        use crate::engine::{MouseAction, MouseButton, MouseInput};
        let action = match action {
            1 => MouseAction::Release,
            2 => MouseAction::Motion,
            _ => MouseAction::Press,
        };
        let button = if action == MouseAction::Motion {
            // Several held: the lowest-numbered one (left, middle, right), as xterm reports it.
            [
                (1, MouseButton::Left),
                (4, MouseButton::Middle),
                (2, MouseButton::Right),
            ]
            .into_iter()
            .find(|(bit, _)| button & bit != 0)
            .map(|(_, b)| b)
        } else {
            match button {
                1 => Some(MouseButton::Left),
                2 => Some(MouseButton::Right),
                4 => Some(MouseButton::Middle),
                8 => Some(MouseButton::WheelUp),
                16 => Some(MouseButton::WheelDown),
                _ => None,
            }
        };
        let mods = qt_mods(modifiers);
        self.engine
            .encode_mouse(
                MouseInput {
                    action,
                    button,
                    x,
                    y,
                    mods,
                },
                ((cell_size >> 16) as u16, (cell_size & 0xffff) as u16),
            )
            .unwrap_or_default()
    }

    /// Where a wheel turn goes with these Qt modifiers: 0 the scrollback, 1 mouse reports (a step
    /// is a notch), 2 cursor keys (a step is a line). See [`crate::engine::WheelTarget`].
    pub fn wheel_target(&self, modifiers: u32) -> u8 {
        use crate::engine::WheelTarget;
        match self.engine.wheel_target(qt_mods(modifiers).shift) {
            WheelTarget::Scrollback => 0,
            WheelTarget::Report => 1,
            WheelTarget::Arrows => 2,
        }
    }

    /// The bytes for `steps` wheel steps (negative is up) at a viewport cell; empty when the view
    /// scrolls its scrollback.
    pub fn encode_wheel(
        &mut self,
        x: u16,
        y: u16,
        steps: i32,
        modifiers: u32,
        cell_size: u32,
    ) -> Vec<u8> {
        self.engine.encode_wheel(
            x,
            y,
            steps,
            qt_mods(modifiers),
            ((cell_size >> 16) as u16, (cell_size & 0xffff) as u16),
        )
    }

    /// Encodes a Qt key event (`QKeyEvent::key()` and `modifiers()`) with the engine's current
    /// modes. `text` is the already-translated UTF-8 text; empty when the key only produces a
    /// control sequence.
    pub fn encode_qt_key(&mut self, key: i32, modifiers: u32, text: &str) -> Vec<u8> {
        let mut mods = qt_mods(modifiers);
        // Qt reports Shift+Tab as Key_Backtab (xkb ISO_Left_Tab): Tab with Shift, `CSI Z`.
        if key == QT_KEY_BACKTAB {
            mods.shift = true;
        }
        // `text` is the unmodified character. With Ctrl Qt reports the control byte itself
        // ("\x03" for Ctrl+C); passing that on makes the encoder emit `CSI 3;5u` instead of the
        // byte, and the shell never sees SIGINT. A control character is no text: the key and the
        // modifier alone encode it.
        let text =
            (!text.is_empty() && !text.chars().all(|c| c.is_control())).then(|| text.to_owned());
        let input = crate::engine::KeyInput {
            key: qt_key(key),
            mods,
            text,
            ..crate::engine::KeyInput::default()
        };
        let bytes = self.engine.encode_key(&input).unwrap_or_default();
        // Legacy mode sends nothing for Ctrl+Shift+letter, nor for Ctrl+@ [ \ ] ^ _ (no key
        // mapping); xterm sends the control byte (NUL for Ctrl+@).
        if bytes.is_empty() && mods.ctrl && !mods.alt && (0x40..=0x5f).contains(&key) {
            return vec![u8::try_from(key - 0x40).unwrap_or_default()];
        }
        bytes
    }

    pub fn scrollback(&self) -> Option<usize> {
        self.scrollback
    }

    /// Drops the scrollback and resets the emulator (`clear()` in the QML session).
    pub fn clear(&mut self) {
        self.engine.clear();
    }

    /// Applies a new `historySize` (negative in QML means unlimited, the caller passes `None`).
    pub fn set_history(&mut self, lines: i64) {
        let limit = usize::try_from(lines).ok();
        if self.engine.set_history_lines(limit).is_ok() {
            self.scrollback = limit;
        }
    }

    /// Drains effects queued by the last feed as a compact JSON array. `pty` carries hex bytes;
    pub fn events_json(&mut self) -> String {
        let events = self.engine.events();
        let list: Vec<Value> = events
            .into_iter()
            .map(|event| match event {
                crate::engine::EngineEvent::PtyWrite(bytes) => {
                    let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
                    json!({"t": "pty", "d": hex})
                }
                crate::engine::EngineEvent::Bell => json!({"t": "bell"}),
                crate::engine::EngineEvent::TitleChanged(title) => {
                    json!({"t": "title", "v": title})
                }
                crate::engine::EngineEvent::PwdChanged(pwd) => json!({"t": "pwd", "v": pwd}),
                crate::engine::EngineEvent::ClipboardWrite {
                    selection,
                    contents,
                } => {
                    let text = contents
                        .iter()
                        .find(|(mime, _)| {
                            mime == "text/plain;charset=utf-8" || mime == "text/plain"
                        })
                        .or_else(|| contents.first())
                        .map(|(_, data)| String::from_utf8_lossy(data).into_owned())
                        .unwrap_or_default();
                    json!({"t": "clip", "sel": selection, "v": text})
                }
                crate::engine::EngineEvent::Notification { title, body } => {
                    json!({"t": "notify", "v": format!("{title}\n{body}")})
                }
            })
            .collect();
        Value::Array(list).to_string()
    }

    pub fn scrollbar_json(&self) -> String {
        match self.engine.scrollbar() {
            Ok(bar) => {
                json!({"total": bar.total, "offset": bar.offset, "len": bar.len}).to_string()
            }
            Err(_) => "{}".to_owned(),
        }
    }

    /// Packed snapshot for the view. Layout, all big-endian:
    /// `u32 magic, u16 cols, u16 rows, u16 cursor_x, u16 cursor_y, u8 visible, u8 shape,
    /// u8 blinking, u8 damage, 3x u8 fg, 3x u8 bg, u8 has_cursor, 3x u8 cursor, u32 arena_len,
    /// u32 row_count`, then per row `u8 wrapped, u32 cells` and per cell
    /// `u16 x, u8 width, u16 flags, u8 has_fg, rgb?, u8 has_bg, rgb?, u32 text_off, u16 text_len`,
    /// then the UTF-8 arena. Only non-blank cells are listed; each carries the column it occupies,
    /// so the empty cells a prompt skips with `CSI n C` keep their place.
    pub fn snapshot_binary(&mut self) -> Vec<u8> {
        let Ok(snapshot) = self.engine.snapshot() else {
            return Vec::new();
        };
        let mut body = Vec::new();
        let mut arena = Vec::new();
        push_u32(&mut body, snapshot.row_data.len() as u32);
        for row in &snapshot.row_data {
            body.push(u8::from(row.wrapped));
            let cells = placed_cells(row);
            push_u32(&mut body, cells.len() as u32);
            for (x, cell) in cells {
                push_u16(&mut body, x);
                body.push(cell.width);
                body.extend_from_slice(&style_flags(&cell.style, cell.selected).to_be_bytes());
                if let Some(fg) = cell.style.fg {
                    body.push(1);
                    body.extend_from_slice(&[fg.r, fg.g, fg.b]);
                } else {
                    body.push(0);
                }
                if let Some(bg) = cell.style.bg {
                    body.push(1);
                    body.extend_from_slice(&[bg.r, bg.g, bg.b]);
                } else {
                    body.push(0);
                }
                push_u32(&mut body, arena.len() as u32);
                push_u16(&mut body, cell.text.len() as u16);
                arena.extend_from_slice(cell.text.as_bytes());
            }
        }

        let mut out = Vec::with_capacity(40 + body.len() + arena.len());
        push_u32(&mut out, 0x4753_5031); // "GSP1"
        push_u16(&mut out, snapshot.cols);
        push_u16(&mut out, snapshot.rows);
        push_u16(&mut out, snapshot.cursor.x);
        push_u16(&mut out, snapshot.cursor.y);
        out.push(u8::from(snapshot.cursor.visible));
        out.push(match snapshot.cursor.shape {
            CursorShape::Block => 0,
            CursorShape::Bar => 1,
            CursorShape::Underline => 2,
            CursorShape::HollowBlock => 3,
        });
        out.push(u8::from(snapshot.cursor.blinking));
        out.push(match snapshot.damage {
            crate::engine::Damage::Clean => 0,
            crate::engine::Damage::Partial => 1,
            crate::engine::Damage::Full => 2,
        });
        for rgb in [snapshot.colors.foreground, snapshot.colors.background] {
            out.extend_from_slice(&[rgb.r, rgb.g, rgb.b]);
        }
        out.push(u8::from(snapshot.colors.cursor.is_some()));
        let cursor = snapshot.colors.cursor.unwrap_or_default();
        out.extend_from_slice(&[cursor.r, cursor.g, cursor.b]);
        push_u32(&mut out, arena.len() as u32);
        out.extend_from_slice(&body);
        out.extend_from_slice(&arena);
        out
    }

    /// A compact JSON snapshot. Only non-blank cells are listed; each carries its column index.
    pub fn snapshot_json(&mut self) -> String {
        let Ok(snapshot) = self.engine.snapshot() else {
            return "{}".to_owned();
        };
        let rows: Vec<Value> = snapshot
            .row_data
            .iter()
            .map(|row| {
                let cells: Vec<Value> = placed_cells(row)
                    .into_iter()
                    .map(|(x, cell)| {
                        let mut flags = 0u16;
                        flags |= u16::from(cell.style.bold);
                        flags |= u16::from(cell.style.italic) << 1;
                        flags |= u16::from(cell.style.faint) << 2;
                        flags |= u16::from(cell.style.blink) << 3;
                        flags |= u16::from(cell.style.inverse) << 4;
                        flags |= u16::from(cell.style.invisible) << 5;
                        flags |= u16::from(cell.style.strikethrough) << 6;
                        flags |= u16::from(cell.style.overline) << 7;
                        flags |=
                            u16::from(cell.style.underline != crate::engine::Underline::None) << 8;
                        flags |= u16::from(cell.selected) << 9;
                        let mut value = json!({
                            "x": x,
                            "t": cell.text,
                            "w": cell.width,
                            "a": flags,
                        });
                        if let Some(fg) = cell.style.fg {
                            value["f"] = color(fg);
                        }
                        if let Some(bg) = cell.style.bg {
                            value["b"] = color(bg);
                        }
                        value
                    })
                    .collect();
                json!({"w": row.wrapped, "c": cells})
            })
            .collect();
        let cursor = snapshot.cursor;
        let value = json!({
            "cols": snapshot.cols,
            "rows": snapshot.rows,
            "fg": color(snapshot.colors.foreground),
            "bg": color(snapshot.colors.background),
            "cursor": {
                "x": cursor.x,
                "y": cursor.y,
                "visible": cursor.visible,
                "blink": cursor.blinking,
                "shape": match cursor.shape {
                    CursorShape::Block => 0,
                    CursorShape::Bar => 1,
                    CursorShape::Underline => 2,
                    CursorShape::HollowBlock => 3,
                },
                "color": snapshot.colors.cursor.map(color),
            },
            "rows_data": rows,
        });
        value.to_string()
    }
}

fn color(rgb: Rgb) -> Value {
    json!([rgb.r, rgb.g, rgb.b])
}

/// `Qt::Key` -> neutral key. Qt constants are stable and this is the only place that knows them;
/// Qt's `KeyboardModifiers` bits as engine modifiers.
fn qt_mods(modifiers: u32) -> crate::engine::Mods {
    crate::engine::Mods {
        shift: modifiers & 0x0200_0000 != 0,
        ctrl: modifiers & 0x0400_0000 != 0,
        alt: modifiers & 0x0800_0000 != 0,
        super_key: modifiers & 0x1000_0000 != 0,
        ..crate::engine::Mods::default()
    }
}

/// `Qt::Key_Backtab`.
const QT_KEY_BACKTAB: i32 = 0x0100_0002;

fn qt_key(key: i32) -> crate::engine::Key {
    use crate::engine::Key;
    match key {
        0x41..=0x5a => match key - 0x41 {
            0 => Key::A,
            1 => Key::B,
            2 => Key::C,
            3 => Key::D,
            4 => Key::E,
            5 => Key::F,
            6 => Key::G,
            7 => Key::H,
            8 => Key::I,
            9 => Key::J,
            10 => Key::K,
            11 => Key::L,
            12 => Key::M,
            13 => Key::N,
            14 => Key::O,
            15 => Key::P,
            16 => Key::Q,
            17 => Key::R,
            18 => Key::S,
            19 => Key::T,
            20 => Key::U,
            21 => Key::V,
            22 => Key::W,
            23 => Key::X,
            24 => Key::Y,
            _ => Key::Z,
        },
        0x30..=0x39 => match key - 0x30 {
            0 => Key::Digit0,
            1 => Key::Digit1,
            2 => Key::Digit2,
            3 => Key::Digit3,
            4 => Key::Digit4,
            5 => Key::Digit5,
            6 => Key::Digit6,
            7 => Key::Digit7,
            8 => Key::Digit8,
            _ => Key::Digit9,
        },
        0x20 => Key::Space,
        0x21 => Key::Digit1,
        0x22 => Key::Quote,
        0x23 => Key::Digit3,
        0x24 => Key::Digit4,
        0x25 => Key::Digit5,
        0x26 => Key::Digit7,
        0x27 => Key::Quote,
        0x28 => Key::Digit9,
        0x29 => Key::Digit0,
        0x2a => Key::Digit8,
        0x2b => Key::Equal,
        0x2c => Key::Comma,
        0x2d => Key::Minus,
        0x2e => Key::Period,
        0x2f => Key::Slash,
        0x3a => Key::Semicolon,
        0x3b => Key::Semicolon,
        0x3c => Key::Comma,
        0x3d => Key::Equal,
        0x3e => Key::Period,
        0x3f => Key::Slash,
        0x40 => Key::Digit2,
        0x5b => Key::BracketLeft,
        0x5c => Key::Backslash,
        0x5d => Key::BracketRight,
        0x5e => Key::Digit6,
        0x5f => Key::Minus,
        0x60 => Key::Backquote,
        0x7b => Key::BracketLeft,
        0x7c => Key::Backslash,
        0x7d => Key::BracketRight,
        0x7e => Key::Backquote,
        0x0100_0000 => Key::Escape,
        0x0100_0001 | QT_KEY_BACKTAB => Key::Tab,
        0x0100_0003 => Key::Backspace,
        0x0100_0004 | 0x0100_0005 => Key::Enter,
        0x0100_0006 => Key::Insert,
        0x0100_0007 => Key::Delete,
        0x0100_0010 => Key::Home,
        0x0100_0011 => Key::End,
        0x0100_0012 => Key::ArrowLeft,
        0x0100_0013 => Key::ArrowUp,
        0x0100_0014 => Key::ArrowRight,
        0x0100_0015 => Key::ArrowDown,
        0x0100_0016 => Key::PageUp,
        0x0100_0017 => Key::PageDown,
        0x0100_0030..=0x0100_003b => match key - 0x0100_0030 {
            0 => Key::F1,
            1 => Key::F2,
            2 => Key::F3,
            3 => Key::F4,
            4 => Key::F5,
            5 => Key::F6,
            6 => Key::F7,
            7 => Key::F8,
            8 => Key::F9,
            9 => Key::F10,
            10 => Key::F11,
            _ => Key::F12,
        },
        _ => Key::Unidentified,
    }
}

fn push_u16(out: &mut Vec<u8>, value: u16) {
    out.extend_from_slice(&value.to_be_bytes());
}

fn push_u32(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&value.to_be_bytes());
}

fn style_flags(style: &crate::engine::Style, selected: bool) -> u16 {
    let mut flags = 0u16;
    flags |= u16::from(style.bold);
    flags |= u16::from(style.italic) << 1;
    flags |= u16::from(style.faint) << 2;
    flags |= u16::from(style.blink) << 3;
    flags |= u16::from(style.inverse) << 4;
    flags |= u16::from(style.invisible) << 5;
    flags |= u16::from(style.strikethrough) << 6;
    flags |= u16::from(style.overline) << 7;
    flags |= u16::from(style.underline != crate::engine::Underline::None) << 8;
    flags |= u16::from(selected) << 9;
    flags
}

/// `(column, cell)` for the cells a snapshot lists. Blank cells are left out but still advance
/// the column: when a prompt jumps ahead with `CSI n C` (a shell's right prompt, `RPROMPT`), the
/// gap is empty cells and the text after it has to keep its place.
fn placed_cells(row: &crate::engine::Row) -> Vec<(u16, &crate::engine::Cell)> {
    let mut out = Vec::with_capacity(row.cells.len());
    let mut column = 0u16;
    for cell in &row.cells {
        if !cell.is_blank() || cell.width == 2 {
            out.push((column, cell));
        }
        column = column.saturating_add(u16::from(cell.width));
    }
    out
}

/// Parses Konsole's `r,g,b` colour value.
fn parse_rgb(value: &str) -> Option<Rgb> {
    let mut parts = value.split(',').map(|part| part.trim().parse::<u8>().ok());
    Some(Rgb {
        r: parts.next()??,
        g: parts.next()??,
        b: parts.next()??,
    })
}

#[cfg(test)]
mod tests;
