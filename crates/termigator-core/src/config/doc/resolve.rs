//! PyYAML's implicit resolver (`yaml/resolver.py`) and SafeConstructor scalar
//! constructors (`yaml/constructor.py`), written out by hand instead of regexes.
//!
//! A regex `$` may also match before a final `\n`; that never matters here, because
//! plain scalars cannot end with a line break and multi-line strings are never
//! written plain.

use super::Value;

/// The YAML 1.1 type tags PyYAML's SafeLoader knows for scalars.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Tag {
    Null,
    Bool,
    Int,
    Float,
    Timestamp,
    Str,
    /// `<<` — only meaningful as a mapping key.
    Merge,
    /// `=` — a plain string when it is a mapping key, an error elsewhere.
    Value,
    /// `!`, `&`, `*` — cannot come from the parser, only stops the emitter writing them plain.
    Yaml,
}

pub(super) const NS: &str = "tag:yaml.org,2002:";

impl Tag {
    pub(super) fn name(self) -> &'static str {
        match self {
            Tag::Null => "null",
            Tag::Bool => "bool",
            Tag::Int => "int",
            Tag::Float => "float",
            Tag::Timestamp => "timestamp",
            Tag::Str => "str",
            Tag::Merge => "merge",
            Tag::Value => "value",
            Tag::Yaml => "yaml",
        }
    }

    /// `tag:yaml.org,2002:int` → `Int`; `None` for tags without a scalar constructor.
    pub(super) fn from_uri(uri: &str) -> Option<Tag> {
        let name = uri.strip_prefix(NS)?;
        [
            Tag::Null,
            Tag::Bool,
            Tag::Int,
            Tag::Float,
            Tag::Timestamp,
            Tag::Str,
            Tag::Merge,
            Tag::Value,
            Tag::Yaml,
        ]
        .into_iter()
        .find(|t| t.name() == name)
    }
}

/// The tag PyYAML gives an untagged plain scalar (`Resolver.resolve(ScalarNode, s, (True, False))`).
pub(super) fn resolve_plain(s: &str) -> Tag {
    if bool_word(s).is_some() {
        Tag::Bool
    } else if is_float(s) {
        Tag::Float
    } else if is_int(s) {
        Tag::Int
    } else if s == "<<" {
        Tag::Merge
    } else if matches!(s, "~" | "null" | "Null" | "NULL" | "") {
        Tag::Null
    } else if is_timestamp(s) {
        Tag::Timestamp
    } else if s == "=" {
        Tag::Value
    } else if matches!(s, "!" | "&" | "*") {
        Tag::Yaml
    } else {
        Tag::Str
    }
}

fn bool_word(s: &str) -> Option<bool> {
    match s {
        "yes" | "Yes" | "YES" | "true" | "True" | "TRUE" | "on" | "On" | "ON" => Some(true),
        "no" | "No" | "NO" | "false" | "False" | "FALSE" | "off" | "Off" | "OFF" => Some(false),
        _ => None,
    }
}

fn strip_sign(s: &str) -> &str {
    s.strip_prefix(['-', '+']).unwrap_or(s)
}

fn all(s: &str, f: impl Fn(char) -> bool) -> bool {
    s.chars().all(f)
}

fn digit_or_underscore(c: char) -> bool {
    c.is_ascii_digit() || c == '_'
}

/// `(?::[0-5]?[0-9])+` matching the whole of `s`.
fn sexagesimal_tail(s: &str) -> bool {
    let Some(rest) = s.strip_prefix(':') else {
        return false;
    };
    rest.split(':').all(|seg| match seg.as_bytes() {
        [d] => d.is_ascii_digit(),
        [a, b] => (b'0'..=b'5').contains(a) && b.is_ascii_digit(),
        _ => false,
    })
}

/// `[1-9][0-9_]*` followed by an optional sexagesimal tail; `tail_required` for floats.
fn base60(s: &str, first: impl Fn(char) -> bool, tail_required: bool) -> bool {
    if !s.starts_with(first) {
        return false;
    }
    let head = s.find(|c: char| !digit_or_underscore(c)).unwrap_or(s.len());
    if head == s.len() {
        !tail_required
    } else {
        sexagesimal_tail(&s[head..])
    }
}

/// `(?:[eE][-+][0-9]+)?` matching the whole of `s`.
fn exponent(s: &str) -> bool {
    s.is_empty()
        || s.strip_prefix(['e', 'E'])
            .and_then(|r| r.strip_prefix(['-', '+']))
            .is_some_and(|d| !d.is_empty() && all(d, |c| c.is_ascii_digit()))
}

fn is_int(s: &str) -> bool {
    let u = strip_sign(s);
    if let Some(r) = u.strip_prefix("0b") {
        return !r.is_empty() && all(r, |c| matches!(c, '0' | '1' | '_'));
    }
    if let Some(r) = u.strip_prefix("0x") {
        return !r.is_empty() && all(r, |c| c.is_ascii_hexdigit() || c == '_');
    }
    if let Some(r) = u.strip_prefix('0') {
        return r.is_empty() || all(r, |c| matches!(c, '0'..='7' | '_'));
    }
    base60(u, |c| ('1'..='9').contains(&c), false)
}

fn is_float(s: &str) -> bool {
    if matches!(s, ".nan" | ".NaN" | ".NAN") {
        return true;
    }
    if let Some(r) = s.strip_prefix('.') {
        // `\.[0-9][0-9_]*(exp)?` — no sign allowed here.
        if r.starts_with(|c: char| c.is_ascii_digit()) {
            let end = r.find(|c: char| !digit_or_underscore(c)).unwrap_or(r.len());
            return exponent(&r[end..]);
        }
    }
    let u = strip_sign(s);
    if matches!(u, ".inf" | ".Inf" | ".INF") {
        return true;
    }
    let Some((int_part, frac)) = u.split_once('.') else {
        return false;
    };
    // `[0-9][0-9_]*\.[0-9_]*(exp)?`
    if int_part.starts_with(|c: char| c.is_ascii_digit()) && all(int_part, digit_or_underscore) {
        let end = frac
            .find(|c: char| !digit_or_underscore(c))
            .unwrap_or(frac.len());
        return exponent(&frac[end..]);
    }
    // `[0-9][0-9_]*(?::[0-5]?[0-9])+\.[0-9_]*`
    base60(int_part, |c| c.is_ascii_digit(), true) && all(frac, digit_or_underscore)
}

fn is_timestamp(s: &str) -> bool {
    // The resolver wants two-digit month and day when there is no time part.
    Timestamp::parse(s).is_some_and(|t| t.time.is_some() || s.len() == 10)
}

/// SafeConstructor for `tag` applied to the scalar text (`construct_yaml_*`).
pub(super) fn construct(tag: Tag, text: &str) -> Result<Value, String> {
    match tag {
        Tag::Null => Ok(Value::Null),
        Tag::Str => Ok(Value::Str(text.to_owned())),
        Tag::Bool => bool_word(&text.to_lowercase())
            .map(Value::Bool)
            .ok_or_else(|| format!("invalid boolean {text:?}")),
        Tag::Int => construct_int(text).map(Value::Int),
        Tag::Float => construct_float(text).map(Value::Float),
        Tag::Timestamp => Timestamp::parse(text)
            .ok_or_else(|| format!("invalid timestamp {text:?}"))?
            .iso()
            .map(|_| Value::Timestamp(text.to_owned())),
        Tag::Merge | Tag::Value | Tag::Yaml => Err(format!(
            "could not determine a constructor for the tag '{NS}{}'",
            tag.name()
        )),
    }
}

fn construct_int(text: &str) -> Result<i64, String> {
    let bad = || format!("invalid integer {text:?}");
    let value = text.replace('_', "");
    let sign: i128 = if value.starts_with('-') { -1 } else { 1 };
    let v = value.strip_prefix(['-', '+']).unwrap_or(&value);
    let radix = |digits: &str, base: u32| i128::from_str_radix(digits, base).map_err(|_| bad());
    let magnitude = if v == "0" {
        0
    } else if let Some(d) = v.strip_prefix("0b") {
        radix(d, 2)?
    } else if let Some(d) = v.strip_prefix("0x") {
        radix(d, 16)?
    } else if v.starts_with('0') {
        radix(v, 8)?
    } else if v.contains(':') {
        let mut total: i128 = 0;
        for part in v.split(':') {
            let digit = radix(part, 10)?;
            total = total
                .checked_mul(60)
                .and_then(|t| t.checked_add(digit))
                .ok_or_else(bad)?;
        }
        total
    } else {
        radix(v, 10)?
    };
    i64::try_from(sign * magnitude).map_err(|_| format!("integer {text:?} is out of range"))
}

fn construct_float(text: &str) -> Result<f64, String> {
    let value = text.replace('_', "").to_lowercase();
    let sign = if value.starts_with('-') { -1.0 } else { 1.0 };
    let v = value.strip_prefix(['-', '+']).unwrap_or(&value);
    let parse = |s: &str| {
        s.parse::<f64>()
            .map_err(|_| format!("invalid float {text:?}"))
    };
    if v == ".inf" {
        Ok(sign * f64::INFINITY)
    } else if v == ".nan" {
        Ok(f64::NAN)
    } else if v.contains(':') {
        // Same accumulation order as PyYAML, so rounding matches too.
        let (mut total, mut base) = (0.0, 1.0);
        for part in v.rsplit(':') {
            total += parse(part)? * base;
            base *= 60.0;
        }
        Ok(sign * total)
    } else {
        Ok(sign * parse(v)?)
    }
}

/// The parts of a timestamp as `SafeConstructor.timestamp_regexp` reads them.
pub(super) struct Timestamp<'a> {
    year: u32,
    month: u32,
    day: u32,
    time: Option<Time<'a>>,
}

struct Time<'a> {
    hour: u32,
    minute: u32,
    second: u32,
    fraction: &'a str,
    /// Offset in minutes; `Some(0)` for `Z`.
    tz: Option<i32>,
}

/// Leading run of 1..=`max` ASCII digits.
fn digits(s: &str, max: usize) -> Option<(u32, &str)> {
    let n = s.bytes().take(max).take_while(u8::is_ascii_digit).count();
    if n == 0 {
        return None;
    }
    Some((s[..n].parse().ok()?, &s[n..]))
}

fn exact(s: &str, n: usize) -> Option<(u32, &str)> {
    digits(s, n).filter(|(_, r)| s.len() - r.len() == n)
}

impl<'a> Timestamp<'a> {
    pub(super) fn parse(s: &'a str) -> Option<Self> {
        let (year, r) = exact(s, 4)?;
        let (month, r) = digits(r.strip_prefix('-')?, 2)?;
        let (day, r) = digits(r.strip_prefix('-')?, 2)?;
        if r.is_empty() {
            return Some(Timestamp {
                year,
                month,
                day,
                time: None,
            });
        }
        let r = match r.strip_prefix(['T', 't']) {
            Some(r) => r,
            None => r.strip_prefix([' ', '\t'])?.trim_start_matches([' ', '\t']),
        };
        let (hour, r) = digits(r, 2)?;
        let (minute, r) = exact(r.strip_prefix(':')?, 2)?;
        let (second, mut r) = exact(r.strip_prefix(':')?, 2)?;
        let mut fraction = "";
        if let Some(f) = r.strip_prefix('.') {
            let n = f.bytes().take_while(u8::is_ascii_digit).count();
            (fraction, r) = f.split_at(n);
        }
        let tz = if r.is_empty() {
            None
        } else {
            let r = r.trim_start_matches([' ', '\t']);
            if r == "Z" {
                Some(0)
            } else {
                let neg = r.starts_with('-');
                let (h, r) = digits(r.strip_prefix(['-', '+'])?, 2)?;
                let m = match r.strip_prefix(':') {
                    Some(m) => exact(m, 2).filter(|(_, rest)| rest.is_empty())?.0,
                    None if r.is_empty() => 0,
                    None => return None,
                };
                let minutes = i32::try_from(h * 60 + m).ok()?;
                Some(if neg { -minutes } else { minutes })
            }
        };
        let time = Time {
            hour,
            minute,
            second,
            fraction,
            tz,
        };
        Some(Timestamp {
            year,
            month,
            day,
            time: Some(time),
        })
    }

    /// ISO form of the date (`YYYY-MM-DD`) or of the datetime (`YYYY-MM-DD HH:MM:SS...`), or the
    /// error PyYAML's constructor raises for an impossible date.
    pub(super) fn iso(&self) -> Result<String, String> {
        let y = self.year;
        let leap = y.is_multiple_of(4) && (!y.is_multiple_of(100) || y.is_multiple_of(400));
        let days = match self.month {
            1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
            4 | 6 | 9 | 11 => 30,
            2 if leap => 29,
            2 => 28,
            _ => return Err(format!("month {} must be in 1..12", self.month)),
        };
        if self.year == 0 {
            return Err("year 0 is out of range".to_owned());
        }
        if !(1..=days).contains(&self.day) {
            return Err(format!("day {} is out of range for month", self.day));
        }
        let mut out = format!("{:04}-{:02}-{:02}", self.year, self.month, self.day);
        let Some(t) = &self.time else {
            return Ok(out);
        };
        if t.hour > 23 || t.minute > 59 || t.second > 59 {
            return Err(format!(
                "time {}:{}:{} is out of range",
                t.hour, t.minute, t.second
            ));
        }
        out += &format!(" {:02}:{:02}:{:02}", t.hour, t.minute, t.second);
        let micro: String = t.fraction.chars().chain("000000".chars()).take(6).collect();
        if micro != "000000" {
            out += &format!(".{micro}");
        }
        if let Some(tz) = t.tz {
            if tz.abs() >= 24 * 60 {
                return Err(format!("UTC offset of {tz} minutes is out of range"));
            }
            let sign = if tz < 0 { '-' } else { '+' };
            out += &format!("{sign}{:02}:{:02}", tz.abs() / 60, tz.abs() % 60);
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolver_matches_pyyaml_regexes() {
        let cases = [
            ("yes", Tag::Bool),
            ("yEs", Tag::Str),
            ("y", Tag::Str),
            ("0777", Tag::Int),
            ("08", Tag::Str),
            ("0x1F", Tag::Int),
            ("0X1F", Tag::Str),
            ("0b", Tag::Str),
            ("1:30", Tag::Int),
            ("1:60", Tag::Str),
            ("09:30", Tag::Str),
            ("1_000", Tag::Int),
            ("1.", Tag::Float),
            ("1e3", Tag::Str),
            ("1.5e+3", Tag::Float),
            ("1.5e3", Tag::Str),
            ("-.5", Tag::Str),
            (".5", Tag::Float),
            ("._5", Tag::Str),
            ("1:30.5", Tag::Float),
            ("-.inf", Tag::Float),
            ("-.nan", Tag::Str),
            ("2026-01-02", Tag::Timestamp),
            ("2026-1-2", Tag::Str),
            ("2026-1-2 3:04:05.5", Tag::Timestamp),
            ("2026-01-02T10:20:30+0100", Tag::Str),
            ("2026-01-02 10:20:30 ", Tag::Str),
            ("~", Tag::Null),
            ("", Tag::Null),
            ("<<", Tag::Merge),
            ("=", Tag::Value),
            ("*", Tag::Yaml),
        ];
        for (s, tag) in cases {
            assert_eq!(resolve_plain(s), tag, "{s:?}");
        }
    }

    #[test]
    fn constructors_follow_golden() {
        assert_eq!(construct(Tag::Int, "0777"), Ok(Value::Int(511)));
        assert_eq!(construct(Tag::Int, "-1:30"), Ok(Value::Int(-90)));
        assert_eq!(construct(Tag::Int, "0_7"), Ok(Value::Int(7)));
        assert!(construct(Tag::Int, "0b_").is_err());
        assert!(construct(Tag::Int, "9223372036854775808").is_err());
        assert_eq!(
            construct(Tag::Int, "-9223372036854775808"),
            Ok(Value::Int(i64::MIN))
        );
        assert_eq!(construct(Tag::Float, "-1:30.5"), Ok(Value::Float(-90.5)));
        assert_eq!(construct(Tag::Float, "1."), Ok(Value::Float(1.0)));
        assert_eq!(construct(Tag::Bool, "yEs"), Ok(Value::Bool(true)));
    }

    #[test]
    fn timestamps_canonicalise_like_isoformat() {
        let iso = |s| Timestamp::parse(s).map(|t| t.iso());
        assert_eq!(
            iso("2026-1-2T3:04:05Z"),
            Some(Ok("2026-01-02 03:04:05+00:00".into()))
        );
        assert_eq!(
            iso("2026-01-02 10:20:30.5 -5"),
            Some(Ok("2026-01-02 10:20:30.500000-05:00".into()))
        );
        assert_eq!(
            iso("2026-01-02 10:20:30.0000001"),
            Some(Ok("2026-01-02 10:20:30".into()))
        );
        assert_eq!(
            iso("2026-01-02T10:20:30+01:99"),
            Some(Ok("2026-01-02 10:20:30+02:39".into()))
        );
        assert!(matches!(iso("2025-02-29"), Some(Err(_))));
        assert!(matches!(iso("2026-01-02 10:00:00 +24"), Some(Err(_))));
        assert!(iso("2026-01-02T10:20:30+0100").is_none());
    }
}
