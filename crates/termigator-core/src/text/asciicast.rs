//! asciicast v2 recorder.
//!
//! A JSON header line, then one JSON array per event: `[seconds, "o", text]` for output and
//! `[seconds, "r", "COLSxROWS"]` for a resize. Bytes are exactly those of
//! `json.dumps(obj, ensure_ascii=False)` (separators `", "` / `": "`) and the seconds are
//! `round(t, 6)` printed as a shortest round-trip float. Raw PTY bytes are decoded as UTF-8
//! incrementally with U+FFFD replacement, so a character split across two reads is not mangled.
//!
//! Time is passed in by the caller: `now` is the wall clock (header `timestamp`, whole seconds)
//! and `mono` a monotonic reading in seconds, given to every call that may write an event.
//! Golden: `compat/golden/text_asciicast.json`, tests in `tests/text_asciicast.rs`.

use std::io::{self, Write};

/// Shortest round-trip float repr (e.g. `1.0`, `1e-05`); non-finite values are spelled `NaN` /
/// `Infinity` / `-Infinity`.
pub(crate) fn shortest_float_repr(x: f64) -> String {
    if x.is_nan() {
        return "NaN".to_owned();
    }
    if x.is_infinite() {
        return if x > 0.0 { "Infinity" } else { "-Infinity" }.to_owned();
    }
    if x == 0.0 {
        return if x.is_sign_negative() { "-0.0" } else { "0.0" }.to_owned();
    }
    // `{:e}` gives the shortest round-trip digits as "d.ddde<exp>".
    let sci = format!("{:e}", x.abs());
    let (mant, exp) = sci.split_once('e').unwrap_or((&sci, "0"));
    let exp: i32 = exp.parse().unwrap_or(0);
    let digits: String = mant.chars().filter(|c| *c != '.').collect();
    let sign = if x < 0.0 { "-" } else { "" };
    let n = i32::try_from(digits.len()).unwrap_or(i32::MAX);
    let body = if (-4..16).contains(&exp) {
        if exp < 0 {
            let zeros = "0".repeat(usize::try_from(-exp - 1).unwrap_or(0));
            format!("0.{zeros}{digits}")
        } else if exp + 1 >= n {
            let zeros = "0".repeat(usize::try_from(exp + 1 - n).unwrap_or(0));
            format!("{digits}{zeros}.0")
        } else {
            let at = usize::try_from(exp + 1).unwrap_or(0);
            format!("{}.{}", &digits[..at], &digits[at..])
        }
    } else {
        let frac = if n > 1 {
            format!(".{}", &digits[1..])
        } else {
            String::new()
        };
        let esign = if exp < 0 { '-' } else { '+' };
        format!("{}{frac}e{esign}{:02}", &digits[..1], exp.abs())
    };
    format!("{sign}{body}")
}

/// Rounds to 6 decimals, as a float (correctly rounded from the exact binary value).
fn round6(x: f64) -> f64 {
    if !x.is_finite() {
        return x;
    }
    format!("{x:.6}").parse().unwrap_or(x)
}

/// Incremental UTF-8 decoder with `errors="replace"` (each invalid sequence becomes U+FFFD).
#[derive(Default)]
struct Utf8Decoder {
    pending: Vec<u8>,
}

impl Utf8Decoder {
    fn decode(&mut self, raw: &[u8], last: bool) -> String {
        self.pending.extend_from_slice(raw);
        let buf = std::mem::take(&mut self.pending);
        let mut out = String::new();
        let mut rest = buf.as_slice();
        while !rest.is_empty() {
            match std::str::from_utf8(rest) {
                Ok(s) => {
                    out.push_str(s);
                    break;
                }
                Err(e) => {
                    let (good, bad) = rest.split_at(e.valid_up_to());
                    out.push_str(&String::from_utf8_lossy(good));
                    match e.error_len() {
                        Some(n) => {
                            out.push('\u{FFFD}');
                            rest = &bad[n..];
                        }
                        None if last => {
                            out.push('\u{FFFD}');
                            break;
                        }
                        None => {
                            self.pending = bad.to_vec();
                            break;
                        }
                    }
                }
            }
        }
        out
    }
}

/// Writes an asciicast v2 stream to `W`.
pub struct CastRecorder<W: Write> {
    out: W,
    start: f64,
    size: (i64, i64),
    decoder: Utf8Decoder,
}

fn json_str(s: &str) -> String {
    // serde_json escapes exactly like json.dumps(ensure_ascii=False): `\" \\ \b \f \n \r \t`
    // and `\u00xx` (lower-case hex) for other control characters.
    serde_json::to_string(s).unwrap_or_else(|_| "\"\"".to_owned())
}

impl<W: Write> CastRecorder<W> {
    /// Writes the header. `env` is filtered to `TERM` and `SHELL` (in the given order); `now` is
    /// the wall clock in seconds (truncated), `mono` the monotonic start reading.
    pub fn new(
        out: W,
        width: i64,
        height: i64,
        title: &str,
        env: &[(String, String)],
        now: f64,
        mono: f64,
    ) -> io::Result<Self> {
        let size = (width.max(1), height.max(1));
        let mut rec = Self {
            out,
            start: mono,
            size,
            decoder: Utf8Decoder::default(),
        };
        // `int(now())` truncates toward zero, as `as` does.
        let mut header = format!(
            "{{\"version\": 2, \"width\": {}, \"height\": {}, \"timestamp\": {}",
            size.0, size.1, now as i64
        );
        if !title.is_empty() {
            header.push_str(&format!(", \"title\": {}", json_str(title)));
        }
        let kept: Vec<String> = env
            .iter()
            .filter(|(k, _)| k == "TERM" || k == "SHELL")
            .map(|(k, v)| format!("{}: {}", json_str(k), json_str(v)))
            .collect();
        header.push_str(&format!(", \"env\": {{{}}}}}", kept.join(", ")));
        rec.write_line(&header)?;
        Ok(rec)
    }

    fn write_line(&mut self, line: &str) -> io::Result<()> {
        self.out.write_all(line.as_bytes())?;
        self.out.write_all(b"\n")
    }

    fn t(&self, mono: f64) -> String {
        shortest_float_repr(round6(mono - self.start))
    }

    /// Records raw terminal output read at monotonic time `mono`; nothing is written while the
    /// bytes only form an incomplete UTF-8 character.
    pub fn output(&mut self, raw: &[u8], mono: f64) -> io::Result<()> {
        let text = self.decoder.decode(raw, false);
        if text.is_empty() {
            return Ok(());
        }
        let line = format!("[{}, \"o\", {}]", self.t(mono), json_str(&text));
        self.write_line(&line)
    }

    /// Records a resize; an unchanged size writes nothing.
    pub fn resize(&mut self, width: i64, height: i64, mono: f64) -> io::Result<()> {
        let size = (width.max(1), height.max(1));
        if size == self.size {
            return Ok(());
        }
        self.size = size;
        let line = format!("[{}, \"r\", \"{}x{}\"]", self.t(mono), size.0, size.1);
        self.write_line(&line)
    }

    /// Flushes a truncated trailing character (as U+FFFD) and returns the writer.
    pub fn close(mut self, mono: f64) -> io::Result<W> {
        let tail = self.decoder.decode(b"", true);
        if !tail.is_empty() {
            let line = format!("[{}, \"o\", {}]", self.t(mono), json_str(&tail));
            self.write_line(&line)?;
        }
        self.out.flush()?;
        Ok(self.out)
    }
}
