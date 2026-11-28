//! Neutral terminal-engine facade.
//!
//! The rest of the bridge talks to a terminal through the types in this module only: [`Engine`],
//! [`Snapshot`], [`Cell`], [`KeyInput`], [`MouseInput`], [`EngineEvent`]. Ghostty's own types stay
//! private to this module so a different backend (the plan's fallback is `alacritty_terminal`) can
//! replace `ghostty-vt` without touching the view or the QObject layer.
//!
//! Threading: every handle inside `ghostty-vt` is `!Send + !Sync`, so [`Engine`] lives on the GUI
//! thread, next to the Qt objects that feed and draw it.

use std::fmt;

use ghostty_vt::{
    RenderState,
    fmt::{Format, Formatter, FormatterOptions},
    key, mouse, paste,
    render::{self, CursorVisualStyle, Dirty},
    screen::{GridRef, Screen},
    selection::{FormatOptions, SelectLineOptions, SelectWordOptions, Selection},
    style, terminal,
    terminal::{Point, PointCoordinate, ScrollViewport, Terminal},
};

/// Engine result. Ghostty's error type is small and `Copy`; wrap it so callers do not depend on it.
pub type Result<T> = std::result::Result<T, Error>;

/// A terminal error, with the backend's message for logs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Error {
    kind: ghostty_vt::Error,
}

impl Error {
    pub(crate) fn from_ghostty(kind: ghostty_vt::Error) -> Self {
        Self { kind }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "terminal engine: {}", self.kind)
    }
}

impl std::error::Error for Error {}

impl From<ghostty_vt::Error> for Error {
    fn from(value: ghostty_vt::Error) -> Self {
        Self::from_ghostty(value)
    }
}

/// 24-bit colour.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Rgb {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl From<style::RgbColor> for Rgb {
    fn from(value: style::RgbColor) -> Self {
        Self {
            r: value.r,
            g: value.g,
            b: value.b,
        }
    }
}

impl From<Rgb> for style::RgbColor {
    fn from(value: Rgb) -> Self {
        Self {
            r: value.r,
            g: value.g,
            b: value.b,
        }
    }
}

/// Text decoration under a cell.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Underline {
    #[default]
    None,
    Single,
    Double,
    Curly,
    Dotted,
    Dashed,
}

/// A cell's visual attributes. Colours are already resolved through the palette; `None` means the
/// terminal's default foreground/background.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Style {
    pub fg: Option<Rgb>,
    pub bg: Option<Rgb>,
    pub bold: bool,
    pub italic: bool,
    pub faint: bool,
    pub blink: bool,
    pub inverse: bool,
    pub invisible: bool,
    pub strikethrough: bool,
    pub overline: bool,
    pub underline: Underline,
}

/// One cell of a row.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Cell {
    /// The full grapheme cluster; empty for a cell without text.
    pub text: String,
    /// 1 or 2 columns. Zero-width tail cells are skipped by the snapshot.
    pub width: u8,
    pub style: Style,
    pub selected: bool,
}

impl Cell {
    /// True for a cell that has nothing to draw (no text and default styling).
    pub fn is_blank(&self) -> bool {
        self.text.is_empty() && self.style == Style::default()
    }
}

/// A viewport row.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Row {
    pub cells: Vec<Cell>,
    /// The row continues on the next line (soft wrap); drawing must not break here.
    pub wrapped: bool,
}

/// Cursor shape (DECSCUSR).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CursorShape {
    #[default]
    Block,
    Bar,
    Underline,
    HollowBlock,
}

/// Where the cursor is and how to draw it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Cursor {
    pub x: u16,
    pub y: u16,
    pub visible: bool,
    pub blinking: bool,
    pub shape: CursorShape,
    /// The cursor is on the trailing half of a wide character; do not draw it there again.
    pub at_wide_tail: bool,
}

/// How much of the render state changed since the previous snapshot.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Damage {
    #[default]
    Clean,
    Partial,
    Full,
}

/// The terminal's effective colours.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Colors {
    pub foreground: Rgb,
    pub background: Rgb,
    pub cursor: Option<Rgb>,
    pub palette: [Rgb; 256],
}

/// What changed since the last snapshot, plus everything needed to draw the viewport.
#[derive(Clone, Debug)]
pub struct Snapshot {
    pub cols: u16,
    pub rows: u16,
    pub damage: Damage,
    /// Viewport rows, top to bottom. Always `rows` entries.
    pub row_data: Vec<Row>,
    pub cursor: Cursor,
    pub colors: Colors,
}

/// An effect queued by the VT stream; drain after every [`Engine::feed`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EngineEvent {
    /// Bytes the program expects back on the pty (query answers, mode reports).
    PtyWrite(Vec<u8>),
    Bell,
    TitleChanged(String),
    /// OSC 7 (usually a `file://` URI) or the equivalent.
    PwdChanged(String),
    /// OSC 52 / Kitty clipboard write: destination and decoded text representations.
    ClipboardWrite {
        selection: bool,
        contents: Vec<(String, Vec<u8>)>,
    },
    /// OSC 9 / OSC 777 desktop notification.
    Notification {
        title: String,
        body: String,
    },
}

/// A key press, already translated from Qt into backend-neutral terms.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeyInput {
    pub key: Key,
    pub mods: Mods,
    pub action: KeyAction,
    /// The unmodified character for the current layout, if the key produces one.
    pub text: Option<String>,
    /// The unshifted code point, needed by the Kitty keyboard protocol.
    pub unshifted: Option<char>,
}

impl Default for KeyInput {
    fn default() -> Self {
        Self {
            key: Key::Unidentified,
            mods: Mods::default(),
            action: KeyAction::Press,
            text: None,
            unshifted: None,
        }
    }
}

/// Modifier keys.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Mods {
    pub shift: bool,
    pub ctrl: bool,
    pub alt: bool,
    pub super_key: bool,
    pub caps_lock: bool,
    pub num_lock: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum KeyAction {
    #[default]
    Press,
    Release,
    Repeat,
}

/// Physical keys the bridge may see. Only what Qt's `QKeyEvent` can report; the mapping lives in
/// one place so a backend swap stays local.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum Key {
    #[default]
    Unidentified,
    Backquote,
    Backslash,
    BracketLeft,
    BracketRight,
    Comma,
    Digit0,
    Digit1,
    Digit2,
    Digit3,
    Digit4,
    Digit5,
    Digit6,
    Digit7,
    Digit8,
    Digit9,
    Equal,
    IntlBackslash,
    A,
    B,
    C,
    D,
    E,
    F,
    G,
    H,
    I,
    J,
    K,
    L,
    M,
    N,
    O,
    P,
    Q,
    R,
    S,
    T,
    U,
    V,
    W,
    X,
    Y,
    Z,
    Minus,
    Period,
    Quote,
    Semicolon,
    Slash,
    Backspace,
    CapsLock,
    ContextMenu,
    Enter,
    Space,
    Tab,
    Delete,
    End,
    Home,
    Insert,
    PageDown,
    PageUp,
    ArrowDown,
    ArrowLeft,
    ArrowRight,
    ArrowUp,
    Escape,
    F1,
    F2,
    F3,
    F4,
    F5,
    F6,
    F7,
    F8,
    F9,
    F10,
    F11,
    F12,
    Numpad0,
    Numpad1,
    Numpad2,
    Numpad3,
    Numpad4,
    Numpad5,
    Numpad6,
    Numpad7,
    Numpad8,
    Numpad9,
    NumpadAdd,
    NumpadDecimal,
    NumpadDivide,
    NumpadEnter,
    NumpadEqual,
    NumpadMultiply,
    NumpadSubtract,
}

fn ghostty_key(key: Key) -> key::Key {
    use key::Key as K;
    match key {
        Key::Unidentified => K::Unidentified,
        Key::Backquote => K::Backquote,
        Key::Backslash => K::Backslash,
        Key::BracketLeft => K::BracketLeft,
        Key::BracketRight => K::BracketRight,
        Key::Comma => K::Comma,
        Key::Digit0 => K::Digit0,
        Key::Digit1 => K::Digit1,
        Key::Digit2 => K::Digit2,
        Key::Digit3 => K::Digit3,
        Key::Digit4 => K::Digit4,
        Key::Digit5 => K::Digit5,
        Key::Digit6 => K::Digit6,
        Key::Digit7 => K::Digit7,
        Key::Digit8 => K::Digit8,
        Key::Digit9 => K::Digit9,
        Key::Equal => K::Equal,
        Key::IntlBackslash => K::IntlBackslash,
        Key::A => K::A,
        Key::B => K::B,
        Key::C => K::C,
        Key::D => K::D,
        Key::E => K::E,
        Key::F => K::F,
        Key::G => K::G,
        Key::H => K::H,
        Key::I => K::I,
        Key::J => K::J,
        Key::K => K::K,
        Key::L => K::L,
        Key::M => K::M,
        Key::N => K::N,
        Key::O => K::O,
        Key::P => K::P,
        Key::Q => K::Q,
        Key::R => K::R,
        Key::S => K::S,
        Key::T => K::T,
        Key::U => K::U,
        Key::V => K::V,
        Key::W => K::W,
        Key::X => K::X,
        Key::Y => K::Y,
        Key::Z => K::Z,
        Key::Minus => K::Minus,
        Key::Period => K::Period,
        Key::Quote => K::Quote,
        Key::Semicolon => K::Semicolon,
        Key::Slash => K::Slash,
        Key::Backspace => K::Backspace,
        Key::CapsLock => K::CapsLock,
        Key::ContextMenu => K::ContextMenu,
        Key::Enter => K::Enter,
        Key::Space => K::Space,
        Key::Tab => K::Tab,
        Key::Delete => K::Delete,
        Key::End => K::End,
        Key::Home => K::Home,
        Key::Insert => K::Insert,
        Key::PageDown => K::PageDown,
        Key::PageUp => K::PageUp,
        Key::ArrowDown => K::ArrowDown,
        Key::ArrowLeft => K::ArrowLeft,
        Key::ArrowRight => K::ArrowRight,
        Key::ArrowUp => K::ArrowUp,
        Key::Escape => K::Escape,
        Key::F1 => K::F1,
        Key::F2 => K::F2,
        Key::F3 => K::F3,
        Key::F4 => K::F4,
        Key::F5 => K::F5,
        Key::F6 => K::F6,
        Key::F7 => K::F7,
        Key::F8 => K::F8,
        Key::F9 => K::F9,
        Key::F10 => K::F10,
        Key::F11 => K::F11,
        Key::F12 => K::F12,
        Key::Numpad0 => K::Numpad0,
        Key::Numpad1 => K::Numpad1,
        Key::Numpad2 => K::Numpad2,
        Key::Numpad3 => K::Numpad3,
        Key::Numpad4 => K::Numpad4,
        Key::Numpad5 => K::Numpad5,
        Key::Numpad6 => K::Numpad6,
        Key::Numpad7 => K::Numpad7,
        Key::Numpad8 => K::Numpad8,
        Key::Numpad9 => K::Numpad9,
        Key::NumpadAdd => K::NumpadAdd,
        Key::NumpadDecimal => K::NumpadDecimal,
        Key::NumpadDivide => K::NumpadDivide,
        Key::NumpadEnter => K::NumpadEnter,
        Key::NumpadEqual => K::NumpadEqual,
        Key::NumpadMultiply => K::NumpadMultiply,
        Key::NumpadSubtract => K::NumpadSubtract,
    }
}

/// A mouse event in cell coordinates.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MouseInput {
    pub action: MouseAction,
    pub button: Option<MouseButton>,
    pub x: u16,
    pub y: u16,
    pub mods: Mods,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MouseAction {
    Press,
    Release,
    Motion,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MouseButton {
    Left,
    Middle,
    Right,
    WheelUp,
    WheelDown,
    Other(u8),
}

/// Where a wheel turn goes, from the program's modes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WheelTarget {
    /// The view scrolls its own scrollback.
    Scrollback,
    /// The program tracks the mouse (DECSET 9/1000/1002/1003): button 4/5 reports, one per
    /// notch, in the format it chose (X11, UTF-8 1005, SGR 1006, urxvt 1015, SGR pixels 1016).
    Report,
    /// The alternate screen (DECSET 47/1047/1049) without mouse tracking: it has no scrollback,
    /// so the wheel sends cursor Up/Down, one per line, like xterm's alternateScroll (DECSET 1007).
    Arrows,
}

/// Scrollbar geometry in rows: `total` is the whole buffer, `offset` the first visible row,
/// `len` the viewport height.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Scrollbar {
    pub total: usize,
    pub offset: usize,
    pub len: usize,
}

/// Output format for [`Engine::text`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TextFormat {
    /// Plain text, line breaks preserved.
    #[default]
    Plain,
    /// VT escape sequences: colours, styles and hyperlinks survive.
    Vt,
}

/// The terminal emulator plus its render state and input encoders.
pub struct Engine {
    term: Terminal,
    render: RenderState,
    rows: render::RowIterator,
    cells: render::CellIterator,
    keys: key::Encoder,
    mouse: mouse::Encoder,
    /// The cell of the last mouse report: a motion is reported only when it reaches another
    /// one. Ghostty's own deduplication (`set_track_last_cell`) cannot be used, as
    /// `set_options_from_terminal`, needed before every event, forgets the cell.
    last_cell: Option<(u16, u16)>,
    cols: u16,
    rows_count: u16,
}

impl fmt::Debug for Engine {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Engine")
            .field("cols", &self.cols)
            .field("rows", &self.rows_count)
            .finish_non_exhaustive()
    }
}

impl Engine {
    /// Creates a terminal of `cols` x `rows` cells.
    ///
    /// `scrollback_lines` follows the Konsole setting (`historySize`); `None` means unlimited,
    /// like a negative `historySize` in the QML API. `cell_px` is the rendered cell size; the
    /// emulator only uses it for pixel-based reports (CSI 14/16 t, SGR-Pixels mouse).
    pub fn new(
        cols: u16,
        rows: u16,
        scrollback_lines: Option<usize>,
        cell_px: (u16, u16),
    ) -> Result<Self> {
        let cols = cols.max(1);
        let rows = rows.max(1);
        let mut term = Terminal::new(terminal::Options {
            cols,
            rows,
            ..Default::default()
        })?;
        apply_history_limit(&mut term, scrollback_lines)?;
        term.resize(cols, rows, cell_px.0.into(), cell_px.1.into())?;

        Ok(Self {
            term,
            render: RenderState::new()?,
            rows: render::RowIterator::new()?,
            cells: render::CellIterator::new()?,
            keys: key::Encoder::new()?,
            mouse: mouse::Encoder::new()?,
            last_cell: None,
            cols,
            rows_count: rows,
        })
    }

    /// Feeds raw pty bytes. Effects are queued; call [`Engine::events`] afterwards.
    pub fn feed(&mut self, data: &[u8]) {
        self.term.vt_write(data);
    }

    /// Drains effects queued by the last feed.
    pub fn events(&mut self) -> Vec<EngineEvent> {
        let mut out = Vec::new();
        for event in self.term.events() {
            match event {
                terminal::Event::PtyWrite(bytes) => out.push(EngineEvent::PtyWrite(bytes)),
                terminal::Event::Bell => out.push(EngineEvent::Bell),
                terminal::Event::TitleChanged(title) => {
                    out.push(EngineEvent::TitleChanged(title));
                }
                terminal::Event::PwdChanged(pwd) => out.push(EngineEvent::PwdChanged(pwd)),
                terminal::Event::ClipboardWrite(write) => {
                    out.push(EngineEvent::ClipboardWrite {
                        selection: write.location == terminal::ClipboardLocation::Selection,
                        contents: write
                            .contents
                            .into_iter()
                            .map(|content| (content.mime, content.data))
                            .collect(),
                    });
                }
                terminal::Event::DesktopNotification { title, body } => {
                    out.push(EngineEvent::Notification { title, body });
                }
                // Progress reports and render hold are view-level concerns, not handled here.
                _ => {}
            }
        }
        out
    }

    /// Resizes the grid and the reported cell size. The primary screen reflows wrapped lines.
    pub fn resize(&mut self, cols: u16, rows: u16, cell_px: (u16, u16)) -> Result<()> {
        let cols = cols.max(1);
        let rows = rows.max(1);
        if (cols, rows) == (self.cols, self.rows_count) {
            // Pixel size may still have changed (font size, HiDPI scale).
            self.term
                .resize(cols, rows, cell_px.0.into(), cell_px.1.into())?;
            return Ok(());
        }
        self.term
            .resize(cols, rows, cell_px.0.into(), cell_px.1.into())?;
        self.cols = cols;
        self.rows_count = rows;
        Ok(())
    }

    /// Terminal size in cells.
    pub fn size(&self) -> (u16, u16) {
        (self.cols, self.rows_count)
    }

    /// The default (not OSC-overridden) colours the view should use for blank cells.
    pub fn set_colors(&mut self, foreground: Option<Rgb>, background: Option<Rgb>) -> Result<()> {
        self.term.set_default_fg_color(foreground.map(Into::into))?;
        self.term.set_default_bg_color(background.map(Into::into))?;
        Ok(())
    }

    /// Replaces the 256-colour palette (the Konsole `.colorscheme` palette).
    pub fn set_palette(&mut self, palette: [Rgb; 256]) -> Result<()> {
        let palette = style::Palette(palette.map(Into::into));
        self.term.set_default_color_palette(Some(palette))?;
        Ok(())
    }

    /// Ghostty's built-in 256-colour palette, the base for a Konsole scheme's 16 colours.
    pub fn default_palette() -> [Rgb; 256] {
        style::Palette::default().0.map(Into::into)
    }

    /// Builds a full viewport snapshot. Rows that did not change are still copied; a dirty-row
    /// variant is meant for when the view starts using partial updates.
    pub fn snapshot(&mut self) -> Result<Snapshot> {
        let snap = self.render.update(&self.term)?;
        let damage = match snap.dirty()? {
            Dirty::Clean => Damage::Clean,
            Dirty::Partial => Damage::Partial,
            Dirty::Full => Damage::Full,
        };
        let colors = snap.colors()?;
        let palette: [Rgb; 256] = colors.palette.map(Into::into);
        let mut row_data = Vec::with_capacity(self.rows_count as usize);
        {
            let mut iter = self.rows.update(&snap)?;
            while let Some(row) = iter.next() {
                let raw = row.raw_row()?;
                let wrapped = raw.is_wrapped()?;
                let mut cells = Vec::with_capacity(self.cols as usize);
                {
                    let mut cell_iter = self.cells.update(row)?;
                    while let Some(cell) = cell_iter.next() {
                        let raw_cell = cell.raw_cell()?;
                        let width = match raw_cell.wide()? {
                            ghostty_vt::screen::CellWide::Wide => 2,
                            ghostty_vt::screen::CellWide::SpacerTail
                            | ghostty_vt::screen::CellWide::SpacerHead => 0,
                            _ => 1,
                        };
                        if width == 0 {
                            continue;
                        }
                        let mut text = String::new();
                        cell.graphemes_utf8(&mut text)?;
                        let st = cell.style()?;
                        let mut fg = cell.fg_color()?.map(Into::into);
                        // Bold in one of the 8 ANSI colours is drawn in its bright twin, as xterm,
                        // Konsole and Windows Terminal do (bold black is grey, not invisible).
                        if st.bold
                            && let style::StyleColor::Palette(index) = st.fg_color
                            && usize::from(index.0) < 8
                        {
                            fg = Some(palette[usize::from(index.0) + 8]);
                        }
                        cells.push(Cell {
                            text,
                            width,
                            style: Style {
                                fg,
                                bg: cell.bg_color()?.map(Into::into),
                                bold: st.bold,
                                italic: st.italic,
                                faint: st.faint,
                                blink: st.blink,
                                inverse: st.inverse,
                                invisible: st.invisible,
                                strikethrough: st.strikethrough,
                                overline: st.overline,
                                underline: match st.underline {
                                    style::Underline::None => Underline::None,
                                    style::Underline::Single => Underline::Single,
                                    style::Underline::Double => Underline::Double,
                                    style::Underline::Curly => Underline::Curly,
                                    style::Underline::Dotted => Underline::Dotted,
                                    style::Underline::Dashed => Underline::Dashed,
                                    _ => Underline::None,
                                },
                            },
                            selected: cell.is_selected()?,
                        });
                    }
                }
                row_data.push(Row { cells, wrapped });
            }
        }

        let cursor = snap.cursor()?;
        let (x, y, at_wide_tail) = match cursor.viewport {
            Some(v) => (v.x, v.y, v.at_wide_tail),
            None => (0, 0, false),
        };
        Ok(Snapshot {
            cols: snap.cols()?,
            rows: snap.rows()?,
            damage,
            row_data,
            cursor: Cursor {
                x,
                y,
                // Scrolled back into the history, the cursor is off the screen: not at (0, 0).
                visible: cursor.visible && cursor.viewport.is_some(),
                blinking: cursor.blinking,
                shape: match cursor.visual_style {
                    CursorVisualStyle::Bar => CursorShape::Bar,
                    CursorVisualStyle::Underline => CursorShape::Underline,
                    CursorVisualStyle::BlockHollow => CursorShape::HollowBlock,
                    _ => CursorShape::Block,
                },
                at_wide_tail,
            },
            colors: Colors {
                foreground: colors.foreground.into(),
                background: colors.background.into(),
                cursor: colors.cursor.map(Into::into),
                palette,
            },
        })
    }

    /// Marks the current frame as drawn, so later updates can be partial.
    ///
    /// Ghostty keeps the "row dirty" flags on its own snapshot; taking a fresh one here is cheap
    /// when nothing changed. This is meant to become a snapshot guard that avoids the extra update.
    pub fn clean(&mut self) -> Result<()> {
        let snap = self.render.update(&self.term)?;
        snap.clean()?;
        Ok(())
    }

    /// Encodes one key using the terminal's current modes (application cursor keys, modifyOtherKeys,
    /// Kitty keyboard protocol, backarrow mode).
    pub fn encode_key(&mut self, input: &KeyInput) -> Result<Vec<u8>> {
        self.keys.set_options_from_terminal(&self.term);
        let mut event = key::Event::new()?;
        event
            .set_action(match input.action {
                KeyAction::Press => key::Action::Press,
                KeyAction::Release => key::Action::Release,
                KeyAction::Repeat => key::Action::Repeat,
            })
            .set_key(ghostty_key(input.key))
            .set_mods(ghostty_mods(input.mods))
            .set_utf8(input.text.clone());
        if let Some(unshifted) = input.unshifted {
            event.set_unshifted_codepoint(unshifted);
        }
        let mut out = Vec::new();
        self.keys.encode_to_vec(&event, &mut out)?;
        Ok(out)
    }

    /// Encodes a mouse event if the program enabled mouse reporting; an empty vector means the view
    /// should handle it locally (selection, scrolling).
    pub fn encode_mouse(&mut self, input: MouseInput, cell_px: (u16, u16)) -> Result<Vec<u8>> {
        if !self.term.is_mouse_tracking().unwrap_or(false) {
            self.last_cell = None; // tracking turned on again starts afresh, as in Ghostty
        }
        let cell = (input.x, input.y);
        // SGR-Pixels reports where inside the cell the pointer is: every motion is news there.
        let pixels = self
            .term
            .mode(terminal::Mode::SGR_PIXELS_MOUSE)
            .unwrap_or(false);
        if input.action == MouseAction::Motion && !pixels && self.last_cell == Some(cell) {
            return Ok(Vec::new());
        }
        self.mouse.set_options_from_terminal(&self.term);
        self.mouse.set_size(mouse::EncoderSize {
            screen_width: u32::from(self.cols) * u32::from(cell_px.0.max(1)),
            screen_height: u32::from(self.rows_count) * u32::from(cell_px.1.max(1)),
            cell_width: u32::from(cell_px.0.max(1)),
            cell_height: u32::from(cell_px.1.max(1)),
            padding_top: 0,
            padding_bottom: 0,
            padding_right: 0,
            padding_left: 0,
        });
        let mut event = mouse::Event::new()?;
        let button = match input.button {
            None => None,
            Some(MouseButton::Left) => Some(mouse::Button::Left),
            Some(MouseButton::Middle) => Some(mouse::Button::Middle),
            Some(MouseButton::Right) => Some(mouse::Button::Right),
            Some(MouseButton::WheelUp) => Some(mouse::Button::Four),
            Some(MouseButton::WheelDown) => Some(mouse::Button::Five),
            Some(MouseButton::Other(n)) => match n {
                0 => Some(mouse::Button::Six),
                1 => Some(mouse::Button::Seven),
                2 => Some(mouse::Button::Eight),
                3 => Some(mouse::Button::Nine),
                4 => Some(mouse::Button::Ten),
                _ => Some(mouse::Button::Eleven),
            },
        };
        event
            .set_action(match input.action {
                MouseAction::Press => mouse::Action::Press,
                MouseAction::Release => mouse::Action::Release,
                MouseAction::Motion => mouse::Action::Motion,
            })
            .set_button(button)
            .set_mods(ghostty_mods(input.mods))
            .set_position(mouse::Position {
                // MouseInput is in cells; Ghostty expects pixels of the rendered surface.
                x: (f32::from(input.x) + 0.5) * f32::from(cell_px.0.max(1)),
                y: (f32::from(input.y) + 0.5) * f32::from(cell_px.1.max(1)),
            });
        let mut out = Vec::new();
        self.mouse.encode_to_vec(&event, &mut out)?;
        if !out.is_empty() {
            self.last_cell = Some(cell);
        }
        Ok(out)
    }

    /// Whether the alternate screen (vim, less, mc: no scrollback) is showing.
    pub fn alternate_screen(&self) -> bool {
        matches!(self.term.active_screen(), Ok(Screen::Alternate))
    }

    /// Where a wheel turn goes now. Shift keeps it in the view (the way Shift+drag selects in a
    /// program that tracks the mouse).
    pub fn wheel_target(&self, shift: bool) -> WheelTarget {
        if shift {
            return WheelTarget::Scrollback;
        }
        if self.term.is_mouse_tracking().unwrap_or(false) {
            return WheelTarget::Report;
        }
        let alternate = matches!(self.term.active_screen(), Ok(Screen::Alternate));
        if alternate && self.term.mode(terminal::Mode::ALT_SCROLL).unwrap_or(false) {
            WheelTarget::Arrows
        } else {
            WheelTarget::Scrollback
        }
    }

    /// The bytes for `steps` wheel steps at a viewport cell (negative is up): reports (a step is a
    /// notch) or cursor keys (a step is a line). Empty when the view scrolls the scrollback.
    pub fn encode_wheel(
        &mut self,
        x: u16,
        y: u16,
        steps: i32,
        mods: Mods,
        cell_px: (u16, u16),
    ) -> Vec<u8> {
        let up = steps < 0;
        let count = steps.unsigned_abs() as usize;
        let one = match self.wheel_target(mods.shift) {
            WheelTarget::Scrollback => return Vec::new(),
            // A press only: xterm sends no release for buttons 4 and 5.
            WheelTarget::Report => self.encode_mouse(
                MouseInput {
                    action: MouseAction::Press,
                    button: Some(if up {
                        MouseButton::WheelUp
                    } else {
                        MouseButton::WheelDown
                    }),
                    x,
                    y,
                    mods,
                },
                cell_px,
            ),
            WheelTarget::Arrows => self.encode_key(&KeyInput {
                key: if up { Key::ArrowUp } else { Key::ArrowDown },
                ..KeyInput::default()
            }),
        };
        one.map(|bytes| bytes.repeat(count)).unwrap_or_default()
    }

    /// The whole scrollback plus the screen as plain text ("Save terminal output", `history`).
    pub fn history_text(&self, unwrap: bool) -> Result<String> {
        self.text(TextFormat::Plain, unwrap)
    }

    /// The whole scrollback plus the screen in the requested format. `Vt` keeps colours, styles
    /// and OSC 8 hyperlinks (useful for logs and for the spike's link checks).
    pub fn text(&self, format: TextFormat, unwrap: bool) -> Result<String> {
        let mut formatter = Formatter::new(
            &self.term,
            FormatterOptions::new()
                .with_format(match format {
                    TextFormat::Plain => Format::Plain,
                    TextFormat::Vt => Format::Vt,
                })
                .with_unwrap(unwrap),
        )?;
        let bytes = formatter.format_alloc()?;
        Ok(String::from_utf8_lossy(&bytes).into_owned())
    }

    /// Replaces the scrollback line limit (`historySize`); `None` means unlimited.
    pub fn set_history_lines(&mut self, lines: Option<usize>) -> Result<()> {
        apply_history_limit(&mut self.term, lines)
    }

    /// Starts or updates a linear/block selection between two viewport cells.
    pub fn select(&self, start: (u16, u16), end: (u16, u16), rectangle: bool) -> Result<()> {
        let start = self.viewport_ref(start)?;
        let end = self.viewport_ref(end)?;
        let selection = Selection::new(start, end, rectangle);
        self.term.set_selection(Some(&selection))?;
        Ok(())
    }

    /// Selects between two screen coordinates (0,0 = top of scrollback).
    pub fn select_screen(&self, start: (u16, u32), end: (u16, u32), rectangle: bool) -> Result<()> {
        let start = self.screen_ref(start)?;
        let end = self.screen_ref(end)?;
        let selection = Selection::new(start, end, rectangle);
        self.term.set_selection(Some(&selection))?;
        Ok(())
    }

    /// The OSC 8 hyperlink at a viewport cell, or a URL-looking token on that row.
    pub fn url_at(&mut self, col: u16, row: u16) -> Option<String> {
        if let Ok(grid_ref) = self.viewport_ref((col, row)) {
            let mut buf = vec![0u8; 4096];
            if let Ok(len) = grid_ref.hyperlink_uri(&mut buf)
                && len > 0
            {
                return String::from_utf8(buf[..len].to_vec()).ok();
            }
        }
        let snapshot = self.snapshot().ok()?;
        let row_data = snapshot.row_data.get(row as usize)?;
        // An empty cell (a gap a prompt skips with `CSI n C`) still takes its column.
        let text: String = row_data
            .cells
            .iter()
            .map(|cell| {
                if cell.text.is_empty() {
                    " "
                } else {
                    cell.text.as_str()
                }
            })
            .collect();
        let mut start = 0usize;
        while start < text.len() {
            let rest = &text[start..];
            let Some(scheme) = ["https://", "http://", "ftp://", "mailto:", "file://"]
                .into_iter()
                .find(|scheme| rest.starts_with(scheme))
            else {
                let step = rest.chars().next().map_or(1, char::len_utf8);
                start += step;
                continue;
            };
            let end = rest
                .find(|c: char| c.is_whitespace() || "\"'<>()[]{}".contains(c))
                .unwrap_or(rest.len());
            if end > scheme.len() {
                let (start_col, end_col) = cell_span(&text, start, start + end);
                if col >= start_col && col <= end_col {
                    return Some(rest[..end].to_owned());
                }
            }
            start += end.max(1);
        }
        None
    }

    /// Selects the word at a viewport cell (double click).
    pub fn select_word(&self, point: (u16, u16)) -> Result<()> {
        let grid_ref = self.viewport_ref(point)?;
        let selection = self.term.select_word(SelectWordOptions::new(grid_ref))?;
        self.term.set_selection(selection.as_ref())?;
        Ok(())
    }

    /// Selects the whole line at a viewport cell (triple click).
    pub fn select_line(&self, point: (u16, u16)) -> Result<()> {
        let grid_ref = self.viewport_ref(point)?;
        let selection = self.term.select_line(SelectLineOptions::new(grid_ref))?;
        self.term.set_selection(selection.as_ref())?;
        Ok(())
    }

    pub fn clear_selection(&self) -> Result<()> {
        self.term.set_selection(None)?;
        Ok(())
    }

    /// Whether the program enabled bracketed paste (DEC mode 2004).
    pub fn bracketed_paste(&self) -> bool {
        self.term
            .mode(terminal::Mode::BRACKETED_PASTE)
            .unwrap_or(false)
    }

    /// Prepares clipboard text for the pty: strips unsafe control bytes and wraps it in
    /// bracketed-paste markers when the mode is on (or the caller forces it).
    pub fn encode_paste(&self, text: &str, bracketed: bool) -> Vec<u8> {
        let mut data = text.as_bytes().to_vec();
        let mut out = vec![0u8; data.len().saturating_add(32).max(64)];
        match paste::encode(&mut data, bracketed, &mut out) {
            Ok(len) => {
                out.truncate(len);
                out
            }
            Err(ghostty_vt::Error::OutOfSpace { required }) => {
                let mut grown = vec![0u8; required];
                let mut data = text.as_bytes().to_vec();
                match paste::encode(&mut data, bracketed, &mut grown) {
                    Ok(len) => {
                        grown.truncate(len);
                        grown
                    }
                    Err(_) => Vec::new(),
                }
            }
            Err(_) => Vec::new(),
        }
    }

    /// Conservative check: newlines and the bracketed-paste terminator make a paste unsafe.
    pub fn paste_is_safe(text: &str) -> bool {
        paste::is_safe(text)
    }

    /// The current selection as plain text; empty when nothing is selected.
    pub fn selected_text(&self) -> Result<String> {
        let bytes = self.term.format_selection_alloc(FormatOptions::new())?;
        Ok(bytes
            .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
            .unwrap_or_default())
    }

    fn screen_ref(&self, (x, y): (u16, u32)) -> Result<GridRef<'_>> {
        self.term
            .grid_ref(Point::Screen(PointCoordinate { x, y }))
            .map_err(Into::into)
    }

    fn viewport_ref(&self, (x, y): (u16, u16)) -> Result<GridRef<'_>> {
        self.term
            .grid_ref(Point::Viewport(PointCoordinate { x, y: u32::from(y) }))
            .map_err(Into::into)
    }

    /// Scrollbar geometry for the view.
    pub fn scrollbar(&self) -> Result<Scrollbar> {
        let bar = self.term.scrollbar()?;
        Ok(Scrollbar {
            total: bar.total as usize,
            offset: bar.offset as usize,
            len: bar.len as usize,
        })
    }

    /// Scrolls by `delta` rows (negative is up).
    pub fn scroll(&mut self, delta: isize) {
        self.term.scroll_viewport(ScrollViewport::Delta(delta));
    }

    pub fn scroll_to_top(&mut self) {
        self.term.scroll_viewport(ScrollViewport::Top);
    }

    pub fn scroll_to_bottom(&mut self) {
        self.term.scroll_viewport(ScrollViewport::Bottom);
    }

    /// Scrolls so that `row` is the first visible row.
    pub fn scroll_to_row(&mut self, row: usize) {
        self.term.scroll_viewport(ScrollViewport::Row(row));
    }

    /// Resets the emulator and drops the scrollback (`clear()` in the QML session).
    pub fn clear(&mut self) {
        self.term.reset();
    }
}

/// The cells `line[start..end]` covers, as (first column, last column), for a line laid out the way
/// the terminal prints it: a wide or emoji cluster takes two columns, a combining mark none.
/// `start..end` must be a non-empty range on character boundaries.
pub fn cell_span(line: &str, start: usize, end: usize) -> (u16, u16) {
    let chars: Vec<(usize, char)> = line.char_indices().collect();
    let mut first = 0u16;
    let mut column = 0u16;
    let mut i = 0;
    while i < chars.len() {
        // A cluster is a handful of characters; a bounded window keeps this linear.
        let window: Vec<char> = chars[i..chars.len().min(i + 32)]
            .iter()
            .map(|&(_, c)| c)
            .collect();
        let (consumed, width) = ghostty_vt::unicode::grapheme_width(&window);
        let consumed = consumed.max(1);
        let cluster_start = chars[i].0;
        let cluster_end = chars.get(i + consumed).map_or(line.len(), |&(at, _)| at);
        if cluster_start <= start && start < cluster_end {
            first = column;
        }
        if cluster_start < end && end <= cluster_end {
            return (first, column + u16::from(width.max(1)) - 1);
        }
        column = column.saturating_add(u16::from(width));
        i += consumed;
    }
    (first, column.saturating_sub(1).max(first))
}

fn ghostty_mods(mods: Mods) -> key::Mods {
    let mut out = key::Mods::empty();
    out.set(key::Mods::SHIFT, mods.shift);
    out.set(key::Mods::CTRL, mods.ctrl);
    out.set(key::Mods::ALT, mods.alt);
    out.set(key::Mods::SUPER, mods.super_key);
    out.set(key::Mods::CAPS_LOCK, mods.caps_lock);
    out.set(key::Mods::NUM_LOCK, mods.num_lock);
    out
}

/// The line limit alone bounds the scrollback (a byte budget per line cut wide terminals
/// short); `None` keeps everything in memory, as the Unlimited setting says.
fn apply_history_limit(term: &mut Terminal, lines: Option<usize>) -> Result<()> {
    term.set_scrollback_max_bytes(None)?;
    term.set_scrollback_max_lines(lines)?;
    Ok(())
}

#[cfg(test)]
mod tests;
