//! Session log path template and streaming ANSI-stripping formatter.
//!
//! [`render_log_path`] expands `{label} {host} {user} {date} {time}` (each made filename-safe)
//! and a leading `~`; unknown placeholders stay as they are. [`LogFormatter`] post-processes the
//! raw terminal byte stream: it can strip escape sequences and stamp the start of every line, and
//! it handles sequences and lines split across chunks.
//!
//! A defect is kept on purpose: with `strip_ansi` the byte 0x9B is
//! treated as a C1 CSI, which eats the second byte of UTF-8 characters such as `ś` (C5 9B)
//! together with the text that follows up to a CSI final byte.
//!
//! Shortcut: `~user/...` is not expanded (no passwd lookup), only `~` and `~/...`.
//!
//! Time is passed in ([`LocalTime`]). Golden: `compat/golden/text_session_log.json`,
//! tests in `tests/text_session_log.rs`.

use std::sync::LazyLock;

use regex::{Captures, Regex};

use super::snippets::strip_space;

/// Template used when the configured one is empty.
pub const DEFAULT_TEMPLATE: &str = "~/termigator-logs/{label}/{date}_{time}.log";

/// A local wall-clock time (the local clock reading).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LocalTime {
    pub year: u32,
    pub month: u32,
    pub day: u32,
    pub hour: u32,
    pub minute: u32,
    pub second: u32,
}

static UNSAFE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"[^\p{L}\p{N}_.@+-]+").unwrap_or_else(|e| unreachable!("static regex: {e}"))
});
static PLACEHOLDER: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\{([\p{L}\p{N}_]+)\}").unwrap_or_else(|e| unreachable!("static regex: {e}"))
});

fn safe(part: &str) -> String {
    let s = UNSAFE.replace_all(strip_space(part), "_");
    let s: String = s.trim_matches(['.', '_']).chars().take(80).collect();
    if s.is_empty() {
        "session".to_owned()
    } else {
        s
    }
}

/// `os.path.expanduser` for `~` and `~/...` with an explicit home directory.
fn expand_home(path: &str, home: Option<&str>) -> String {
    let Some(home) = home else {
        return path.to_owned();
    };
    if !path.starts_with('~') {
        return path.to_owned();
    }
    let i = path[1..].find('/').map_or(path.len(), |k| k + 1);
    if i != 1 {
        return path.to_owned(); // "~user": needs the passwd database
    }
    let out = format!("{}{}", home.trim_end_matches('/'), &path[i..]);
    if out.is_empty() { "/".to_owned() } else { out }
}

/// [`render_log_path`] with an explicit home directory (`None`: leave `~` alone).
pub fn render_log_path_with_home(
    template: &str,
    label: &str,
    host: &str,
    user: &str,
    now: LocalTime,
    home: Option<&str>,
) -> String {
    let date = format!("{:04}-{:02}-{:02}", now.year, now.month, now.day);
    let time = format!("{:02}{:02}{:02}", now.hour, now.minute, now.second);
    let tpl = if template.is_empty() {
        DEFAULT_TEMPLATE
    } else {
        template
    };
    let out = PLACEHOLDER.replace_all(tpl, |m: &Captures| match &m[1] {
        "label" => safe(label),
        "host" => safe(if host.is_empty() { "local" } else { host }),
        "user" => safe(if user.is_empty() { "user" } else { user }),
        "date" => date.clone(),
        "time" => time.clone(),
        _ => m[0].to_owned(),
    });
    expand_home(&out, home)
}

/// Expands the auto-log template for one session; `~` uses `$HOME`.
pub fn render_log_path(
    template: &str,
    label: &str,
    host: &str,
    user: &str,
    now: LocalTime,
) -> String {
    let home = std::env::var("HOME").ok();
    render_log_path_with_home(template, label, host, user, now, home.as_deref())
}

/// Supplies the time for line stamps.
pub type Clock = Box<dyn FnMut() -> LocalTime + Send>;

/// Streaming log post-processor.
pub struct LogFormatter {
    strip_ansi: bool,
    timestamps: bool,
    clock: Clock,
    at_line_start: bool,
    esc: Vec<u8>,
    in_string: bool,
}

impl LogFormatter {
    /// `clock` is called once per stamped line.
    pub fn new(strip_ansi: bool, timestamps: bool, clock: Clock) -> Self {
        Self {
            strip_ansi,
            timestamps,
            clock,
            at_line_start: true,
            esc: Vec::new(),
            in_string: false,
        }
    }

    pub fn feed(&mut self, data: &[u8]) -> Vec<u8> {
        if !self.strip_ansi && !self.timestamps {
            return data.to_vec();
        }
        let mut out = Vec::with_capacity(data.len());
        for &b in data {
            if self.strip_ansi && self.consume_escape(b) {
                continue;
            }
            if self.strip_ansi && b == 0x0D {
                continue; // CR of CRLF; a bare CR redraw is dropped too
            }
            if self.timestamps && self.at_line_start && b != 0x0A {
                let t = (self.clock)();
                out.extend_from_slice(
                    format!(
                        "[{:04}-{:02}-{:02} {:02}:{:02}:{:02}] ",
                        t.year, t.month, t.day, t.hour, t.minute, t.second
                    )
                    .as_bytes(),
                );
                self.at_line_start = false;
            }
            out.push(b);
            if b == 0x0A {
                self.at_line_start = true;
            }
        }
        out
    }

    /// True when `b` belongs to an escape sequence (and is therefore dropped).
    fn consume_escape(&mut self, b: u8) -> bool {
        if self.in_string {
            if b == 0x07 || (self.esc.last() == Some(&0x1B) && b == b'\\') {
                self.reset();
            } else {
                self.esc = vec![if b == 0x1B { b } else { b'x' }];
            }
            return true;
        }
        if self.esc.is_empty() {
            if b == 0x1B {
                self.esc.push(b);
                return true;
            }
            if b == 0x9B {
                self.esc.extend_from_slice(b"\x1b["); // C1 CSI
                return true;
            }
            return b < 0x20 && !matches!(b, 0x0A | 0x0D | 0x09 | 0x08);
        }
        if self.esc.len() == 1 {
            match b {
                b'[' => self.esc.push(b),
                b']' | b'P' | b'X' | b'^' | b'_' => {
                    self.in_string = true;
                    self.esc = vec![b'x'];
                }
                b'(' | b')' | b'*' | b'+' | b'#' | b'%' => self.esc.push(b),
                _ => self.reset(),
            }
            return true;
        }
        if self.esc.get(1) == Some(&b'[') {
            if (0x40..=0x7E).contains(&b) {
                self.reset();
            }
            return true;
        }
        self.reset(); // the byte after a charset designator
        true
    }

    fn reset(&mut self) {
        self.esc.clear();
        self.in_string = false;
    }
}
