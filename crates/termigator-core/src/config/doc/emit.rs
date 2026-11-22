//! Port of PyYAML's emitter (`yaml/emitter.py`) for what
//! `safe_dump(data, allow_unicode=True, default_flow_style=False, sort_keys=False)`
//! with aliases disabled produces: block collections, flow only for empty ones,
//! plain / single-quoted / double-quoted scalars, width 80, indent 2.
//!
//! Every represented scalar resolves back to its own tag (see `represent`), so no
//! tag is ever written; strings that would not get the `str` tag are quoted.

use super::Value;
use super::resolve::{self, Tag};

const INDENT: usize = 2;
const BREAKS: [char; 4] = ['\n', '\u{85}', '\u{2028}', '\u{2029}'];

fn is_break(c: char) -> bool {
    BREAKS.contains(&c)
}

fn is_space_or_break(c: char) -> bool {
    matches!(c, '\0' | ' ' | '\t' | '\r') || is_break(c)
}

/// How scalars are laid out.
#[derive(Clone, Copy)]
pub(super) struct Layout {
    width: usize,
    /// Never split a scalar over lines (multi-line strings are double-quoted with
    /// `\n` escapes): for values spliced into an existing document at any indent.
    one_line: bool,
}

pub(super) const PYYAML: Layout = Layout {
    width: 80,
    one_line: false,
};
pub(super) const ONE_LINE: Layout = Layout {
    width: usize::MAX,
    one_line: true,
};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Style {
    Plain,
    Single,
    Double,
}

struct Analysis {
    empty: bool,
    multiline: bool,
    allow_flow_plain: bool,
    allow_block_plain: bool,
    allow_single_quoted: bool,
}

/// `Emitter.analyze_scalar` with `allow_unicode=True`.
fn analyze(text: &[char]) -> Analysis {
    if text.is_empty() {
        return Analysis {
            empty: true,
            multiline: false,
            allow_flow_plain: false,
            allow_block_plain: true,
            allow_single_quoted: true,
        };
    }
    let starts = |p: &str| text.iter().copied().take(3).eq(p.chars());
    let mut block_indicators = starts("---") || starts("...");
    let mut flow_indicators = block_indicators;
    let (mut line_breaks, mut special) = (false, false);
    let (mut leading_space, mut leading_break, mut trailing_space, mut trailing_break) =
        (false, false, false, false);
    let (mut break_space, mut space_break) = (false, false);
    let mut preceded_by_ws = true;
    let mut followed_by_ws = text.len() == 1 || is_space_or_break(text[1]);
    let (mut previous_space, mut previous_break) = (false, false);
    let last = text.len() - 1;
    for (index, &ch) in text.iter().enumerate() {
        if index == 0 {
            if "#,[]{}&*!|>'\"%@`".contains(ch) {
                flow_indicators = true;
                block_indicators = true;
            }
            if ch == '?' || ch == ':' {
                flow_indicators = true;
                block_indicators |= followed_by_ws;
            }
            if ch == '-' && followed_by_ws {
                flow_indicators = true;
                block_indicators = true;
            }
        } else {
            if ",?[]{}".contains(ch) {
                flow_indicators = true;
            }
            if ch == ':' {
                flow_indicators = true;
                block_indicators |= followed_by_ws;
            }
            if ch == '#' && preceded_by_ws {
                flow_indicators = true;
                block_indicators = true;
            }
        }
        if is_break(ch) {
            line_breaks = true;
        }
        if !(ch == '\n' || ('\x20'..='\x7e').contains(&ch)) {
            let unicode = (ch == '\u{85}'
                || ('\u{a0}'..='\u{d7ff}').contains(&ch)
                || ('\u{e000}'..='\u{fffd}').contains(&ch)
                || ('\u{10000}'..'\u{10ffff}').contains(&ch))
                && ch != '\u{feff}';
            special |= !unicode;
        }
        if ch == ' ' {
            leading_space |= index == 0;
            trailing_space |= index == last;
            break_space |= previous_break;
            previous_space = true;
            previous_break = false;
        } else if is_break(ch) {
            leading_break |= index == 0;
            trailing_break |= index == last;
            space_break |= previous_space;
            previous_space = false;
            previous_break = true;
        } else {
            previous_space = false;
            previous_break = false;
        }
        preceded_by_ws = is_space_or_break(ch);
        followed_by_ws = index + 2 >= text.len() || is_space_or_break(text[index + 2]);
    }
    let mut a = Analysis {
        empty: false,
        multiline: line_breaks,
        allow_flow_plain: true,
        allow_block_plain: true,
        allow_single_quoted: true,
    };
    if leading_space || leading_break || trailing_space || trailing_break {
        a.allow_flow_plain = false;
        a.allow_block_plain = false;
    }
    if break_space {
        a.allow_flow_plain = false;
        a.allow_block_plain = false;
        a.allow_single_quoted = false;
    }
    if space_break || special {
        a.allow_flow_plain = false;
        a.allow_block_plain = false;
        a.allow_single_quoted = false;
    }
    if line_breaks {
        a.allow_flow_plain = false;
        a.allow_block_plain = false;
    }
    if flow_indicators {
        a.allow_flow_plain = false;
    }
    if block_indicators {
        a.allow_block_plain = false;
    }
    a
}

/// Shortest round-trip float repr (e.g. `1.0`, `1e-05`) as `SafeRepresenter.represent_float`
/// adjusts it.
pub(super) fn float_repr(f: f64) -> String {
    if f.is_nan() {
        return ".nan".to_owned();
    }
    if f.is_infinite() {
        return if f > 0.0 { ".inf" } else { "-.inf" }.to_owned();
    }
    // `{:e}` gives the same shortest round-trip digits as the repr.
    let sci = format!("{f:e}");
    let (mantissa, exp) = sci.split_once('e').unwrap_or((&sci, "0"));
    let exp: i32 = exp.parse().unwrap_or(0);
    let (sign, mantissa) = match mantissa.strip_prefix('-') {
        Some(m) => ("-", m),
        None => ("", mantissa),
    };
    let digits: String = mantissa.chars().filter(|c| *c != '.').collect();
    let decpt = exp + 1;
    let body = if !(-3..=16).contains(&decpt) {
        let (first, rest) = digits.split_at(1);
        // repr has no `.0` here ("1e+17"); PyYAML inserts it so the float regex matches.
        let rest = if rest.is_empty() { "0" } else { rest };
        format!(
            "{first}.{rest}e{}{:02}",
            if exp < 0 { '-' } else { '+' },
            exp.abs()
        )
    } else if decpt <= 0 {
        format!("0.{}{digits}", "0".repeat(decpt.unsigned_abs() as usize))
    } else {
        let point = decpt.unsigned_abs() as usize;
        if point >= digits.len() {
            format!("{digits}{}.0", "0".repeat(point - digits.len()))
        } else {
            format!("{}.{}", &digits[..point], &digits[point..])
        }
    };
    format!("{sign}{body}")
}

/// `SafeRepresenter` + `Serializer`: the scalar text and whether a plain scalar
/// would resolve back to the same tag (`implicit[0]`).
fn represent(v: &Value) -> Option<(String, bool)> {
    Some(match v {
        Value::Null => ("null".to_owned(), true),
        Value::Bool(b) => (b.to_string(), true),
        Value::Int(i) => (i.to_string(), true),
        Value::Float(f) => (float_repr(*f), true),
        Value::Timestamp(t) => (
            super::canonical_timestamp(t).unwrap_or_else(|| t.clone()),
            true,
        ),
        Value::Str(s) => (s.clone(), resolve::resolve_plain(s) == Tag::Str),
        Value::Seq(_) | Value::Map(_) => return None,
    })
}

pub(super) struct Emitter {
    out: String,
    layout: Layout,
    column: usize,
    whitespace: bool,
    indention: bool,
    open_ended: bool,
    indent: Option<usize>,
    indents: Vec<Option<usize>>,
    flow_level: usize,
    root_context: bool,
    mapping_context: bool,
    simple_key_context: bool,
}

pub(super) fn dump(value: &Value, layout: Layout) -> String {
    let mut e = Emitter {
        out: String::new(),
        layout,
        column: 0,
        whitespace: true,
        indention: true,
        open_ended: false,
        indent: None,
        indents: Vec::new(),
        flow_level: 0,
        root_context: false,
        mapping_context: false,
        simple_key_context: false,
    };
    // The root is never an empty untagged scalar event, so the document start stays implicit.
    e.node(value, true, false, false);
    e.write_indent(); // expect_document_end
    if e.open_ended {
        e.write_indicator("...", true, false, false);
        e.write_indent();
    }
    e.out
}

impl Emitter {
    fn write(&mut self, data: &str) {
        self.column += data.chars().count();
        self.out.push_str(data);
    }

    fn increase_indent(&mut self, flow: bool, indentless: bool) {
        self.indents.push(self.indent);
        self.indent = match self.indent {
            None => Some(if flow { INDENT } else { 0 }),
            Some(i) if !indentless => Some(i + INDENT),
            same => same,
        };
    }

    fn pop_indent(&mut self) {
        self.indent = self.indents.pop().flatten();
    }

    fn write_indicator(
        &mut self,
        indicator: &str,
        need_ws: bool,
        whitespace: bool,
        indention: bool,
    ) {
        if !self.whitespace && need_ws {
            self.write(" ");
        }
        self.write(indicator);
        self.whitespace = whitespace;
        self.indention = self.indention && indention;
        self.open_ended = false;
    }

    fn write_indent(&mut self) {
        let indent = self.indent.unwrap_or(0);
        if !self.indention || self.column > indent || (self.column == indent && !self.whitespace) {
            self.write_line_break('\n');
        }
        if self.column < indent {
            self.whitespace = true;
            let pad = " ".repeat(indent - self.column);
            self.write(&pad);
        }
    }

    fn write_line_break(&mut self, br: char) {
        self.whitespace = true;
        self.indention = true;
        self.column = 0;
        self.out.push(br);
    }

    fn node(&mut self, v: &Value, root: bool, mapping: bool, simple_key: bool) {
        self.root_context = root;
        self.mapping_context = mapping;
        self.simple_key_context = simple_key;
        match v {
            Value::Seq(items) if self.flow_level > 0 || items.is_empty() => {
                self.empty_flow("[", "]")
            }
            Value::Map(pairs) if self.flow_level > 0 || pairs.is_empty() => {
                self.empty_flow("{", "}")
            }
            Value::Seq(items) => {
                let indentless = self.mapping_context && !self.indention;
                self.increase_indent(false, indentless);
                for item in items {
                    self.write_indent();
                    self.write_indicator("-", true, false, true);
                    self.node(item, false, false, false);
                }
                self.pop_indent();
            }
            Value::Map(pairs) => {
                self.increase_indent(false, false);
                for (k, val) in pairs {
                    self.write_indent();
                    if simple_key_ok(k) {
                        self.node(k, false, true, true);
                        self.write_indicator(":", false, false, false);
                    } else {
                        self.write_indicator("?", true, false, true);
                        self.node(k, false, true, false);
                        self.write_indent();
                        self.write_indicator(":", true, false, true);
                    }
                    self.node(val, false, true, false);
                }
                self.pop_indent();
            }
            scalar => {
                if let Some((text, implicit_plain)) = represent(scalar) {
                    self.increase_indent(true, false);
                    self.scalar(&text.chars().collect::<Vec<_>>(), implicit_plain);
                    self.pop_indent();
                }
            }
        }
    }

    /// default_flow_style=False: only empty collections are written in flow style.
    fn empty_flow(&mut self, open: &str, close: &str) {
        self.write_indicator(open, true, true, false);
        self.flow_level += 1;
        self.increase_indent(true, false);
        self.pop_indent();
        self.flow_level -= 1;
        self.write_indicator(close, false, false, false);
    }

    fn scalar(&mut self, text: &[char], implicit_plain: bool) {
        let a = analyze(text);
        let sk = self.simple_key_context;
        let plain_ok = if self.flow_level > 0 {
            a.allow_flow_plain
        } else {
            a.allow_block_plain
        };
        let multiline_bad = a.multiline && (sk || self.layout.one_line);
        let style = if implicit_plain && !(sk && (a.empty || a.multiline)) && plain_ok {
            Style::Plain
        } else if a.allow_single_quoted && !multiline_bad {
            Style::Single
        } else {
            Style::Double
        };
        let split = !sk && !self.layout.one_line;
        match style {
            Style::Plain => self.write_plain(text, split),
            Style::Single => self.write_single_quoted(text, split),
            Style::Double => self.write_double_quoted(text, split),
        }
    }

    fn write_chars(&mut self, text: &[char]) {
        let s: String = text.iter().collect();
        self.write(&s);
    }

    fn write_breaks(&mut self, text: &[char]) {
        if text.first() == Some(&'\n') {
            self.write_line_break('\n');
        }
        for &br in text {
            self.write_line_break(br);
        }
        self.write_indent();
    }

    fn write_plain(&mut self, text: &[char], split: bool) {
        if self.root_context {
            self.open_ended = true;
        }
        if text.is_empty() {
            return;
        }
        if !self.whitespace {
            self.write(" ");
        }
        self.whitespace = false;
        self.indention = false;
        let (mut spaces, mut breaks) = (false, false);
        let mut start = 0;
        for end in 0..=text.len() {
            let ch = text.get(end).copied();
            if spaces {
                if ch != Some(' ') {
                    if start + 1 == end && self.column > self.layout.width && split {
                        self.write_indent();
                        self.whitespace = false;
                        self.indention = false;
                    } else {
                        self.write_chars(&text[start..end]);
                    }
                    start = end;
                }
            } else if breaks {
                if !ch.is_some_and(is_break) {
                    self.write_breaks(&text[start..end]);
                    self.whitespace = false;
                    self.indention = false;
                    start = end;
                }
            } else if ch.is_none_or(|c| c == ' ' || is_break(c)) {
                self.write_chars(&text[start..end]);
                start = end;
            }
            if let Some(c) = ch {
                spaces = c == ' ';
                breaks = is_break(c);
            }
        }
    }

    fn write_single_quoted(&mut self, text: &[char], split: bool) {
        self.write_indicator("'", true, false, false);
        let (mut spaces, mut breaks) = (false, false);
        let mut start = 0;
        for end in 0..=text.len() {
            let ch = text.get(end).copied();
            if spaces {
                if ch != Some(' ') {
                    if start + 1 == end
                        && self.column > self.layout.width
                        && split
                        && start != 0
                        && end != text.len()
                    {
                        self.write_indent();
                    } else {
                        self.write_chars(&text[start..end]);
                    }
                    start = end;
                }
            } else if breaks {
                if !ch.is_some_and(is_break) {
                    self.write_breaks(&text[start..end]);
                    start = end;
                }
            } else if ch.is_none_or(|c| c == ' ' || c == '\'' || is_break(c)) && start < end {
                self.write_chars(&text[start..end]);
                start = end;
            }
            if ch == Some('\'') {
                self.write("''");
                start = end + 1;
            }
            if let Some(c) = ch {
                spaces = c == ' ';
                breaks = is_break(c);
            }
        }
        self.write_indicator("'", false, false, false);
    }

    fn write_double_quoted(&mut self, text: &[char], split: bool) {
        self.write_indicator("\"", true, false, false);
        let mut start = 0;
        for end in 0..=text.len() {
            let ch = text.get(end).copied();
            let needs_escape = ch.is_none_or(|c| {
                "\"\\\u{85}\u{2028}\u{2029}\u{feff}".contains(c)
                    || !(('\x20'..='\x7e').contains(&c)
                        || ('\u{a0}'..='\u{d7ff}').contains(&c)
                        || ('\u{e000}'..='\u{fffd}').contains(&c))
            });
            if needs_escape {
                if start < end {
                    self.write_chars(&text[start..end]);
                    start = end;
                }
                if let Some(c) = ch {
                    let esc = match c {
                        '\0' => "\\0".to_owned(),
                        '\x07' => "\\a".to_owned(),
                        '\x08' => "\\b".to_owned(),
                        '\t' => "\\t".to_owned(),
                        '\n' => "\\n".to_owned(),
                        '\x0b' => "\\v".to_owned(),
                        '\x0c' => "\\f".to_owned(),
                        '\r' => "\\r".to_owned(),
                        '\x1b' => "\\e".to_owned(),
                        '"' => "\\\"".to_owned(),
                        '\\' => "\\\\".to_owned(),
                        '\u{85}' => "\\N".to_owned(),
                        '\u{a0}' => "\\_".to_owned(),
                        '\u{2028}' => "\\L".to_owned(),
                        '\u{2029}' => "\\P".to_owned(),
                        c if (c as u32) <= 0xff => format!("\\x{:02X}", c as u32),
                        c if (c as u32) <= 0xffff => format!("\\u{:04X}", c as u32),
                        c => format!("\\U{:08X}", c as u32),
                    };
                    self.write(&esc);
                    start = end + 1;
                }
            }
            if 0 < end
                && end + 1 < text.len()
                && (ch == Some(' ') || start >= end)
                && split
                // `end - start` is -1 right after an escape, as in the PyYAML emitter.
                && self.column as i128 + end as i128 - start as i128 > self.layout.width as i128
            {
                let mut data: String = text[start.min(end)..end].iter().collect();
                data.push('\\');
                if start < end {
                    start = end;
                }
                self.write(&data);
                self.write_indent();
                self.whitespace = false;
                self.indention = false;
                if text.get(start) == Some(&' ') {
                    self.write("\\");
                }
            }
        }
        self.write_indicator("\"", false, false, false);
    }
}

/// `Emitter.check_simple_key` for a key about to be written. The serializer always
/// sets the tag, so its short form (`!!str`, …) counts towards the 128 limit too.
fn simple_key_ok(k: &Value) -> bool {
    let tag = match k {
        Value::Seq(items) => return items.is_empty(),
        Value::Map(pairs) => return pairs.is_empty(),
        Value::Null => "null",
        Value::Bool(_) => "bool",
        Value::Int(_) => "int",
        Value::Float(_) => "float",
        Value::Str(_) => "str",
        Value::Timestamp(_) => "timestamp",
    };
    represent(k).is_some_and(|(text, _)| {
        let chars: Vec<char> = text.chars().collect();
        let a = analyze(&chars);
        chars.len() + 2 + tag.len() < 128 && !a.empty && !a.multiline
    })
}

#[cfg(test)]
mod tests {
    use super::float_repr;

    #[test]
    fn float_repr_matches_golden() {
        let cases = [
            (0.0, "0.0"),
            (-0.0, "-0.0"),
            (1.5, "1.5"),
            (0.1, "0.1"),
            (1e16, "1.0e+16"),
            (1e17, "1.0e+17"),
            (1e-4, "0.0001"),
            (1e-5, "1.0e-05"),
            (1.2345e-7, "1.2345e-07"),
            (1234567890123456.0, "1234567890123456.0"),
            (5e-324, "5.0e-324"),
            (1.5e300, "1.5e+300"),
            (100.0, "100.0"),
        ];
        for (f, s) in cases {
            assert_eq!(float_repr(f), s, "{f:e}");
        }
    }
}
