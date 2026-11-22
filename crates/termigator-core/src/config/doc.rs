//! YAML as the config files are read and written (ADR-5).
//!
//! - [`load`]: PyYAML `safe_load` — YAML 1.1 resolution (`yes`/`on` → bool, `0777` octal,
//!   `1:30` base 60, `~` null, timestamps), anchors, aliases and `<<` merge keys, one
//!   document. Parser: libyaml (safe Rust port `libyaml-safer`), the algorithm PyYAML's
//!   scanner was ported from.
//! - [`dump`]: byte-identical to `yaml.safe_dump(data, allow_unicode=True,
//!   default_flow_style=False, sort_keys=False)` with aliases off (session.yaml).
//! - [`update_preserving_comments`]: config.yaml synced to the target mapping
//!   with a comment-preserving round trip, through yaml-edit (deletions by byte range),
//!   keeping comments and the spelling of unchanged values; new values are quoted
//!   for YAML 1.1 readers. Unlike ruamel, values equal to what the file holds are not
//!   rewritten, so `yes`, `1:30` or a commented list stay exactly as written.
//!
//! Not supported (rejected with an error): recursive aliases, integers outside `i64`,
//! the `!!binary`, `!!set`, `!!omap` and `!!pairs` tags. Golden fixtures:
//! `compat/golden/yaml_*.json`.

mod edit;
mod emit;
mod load;
mod resolve;

/// A loaded YAML value; mappings keep insertion order.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    Str(String),
    /// `datetime.date` / `datetime.datetime`, kept as written; see [`canonical_timestamp`].
    Timestamp(String),
    Seq(Vec<Value>),
    Map(Vec<(Value, Value)>),
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum YamlError {
    /// Scanner or parser error from libyaml; the message carries line and column.
    #[error("{0}")]
    Syntax(String),
    /// The text parses but PyYAML's SafeLoader would not build a value from it.
    #[error("line {line} column {column}: {message}")]
    Construct {
        line: u64,
        column: u64,
        message: String,
    },
}

/// Parse `text` like PyYAML `safe_load`; an empty document is [`Value::Null`].
pub fn load(text: &str) -> Result<Value, YamlError> {
    // libyaml-safer 0.3.0 panics on some malformed text (`"\` at the very end, found by
    // tests/fuzz_parsers.rs): a damaged config is an error to show, not a crash.
    std::panic::catch_unwind(|| load::load(text)).unwrap_or_else(|_| {
        Err(YamlError::Syntax(
            "malformed YAML the parser cannot read, likely an unfinished quote or `\\` near the end of the file".to_owned(),
        ))
    })
}

/// Serialise like `yaml.safe_dump(data, allow_unicode=True, default_flow_style=False,
/// sort_keys=False)`.
pub fn dump(value: &Value) -> String {
    emit::dump(value, emit::PYYAML)
}

/// New config.yaml text: `original` edited to hold `target`, comments kept. Falls back
/// to [`dump`] when `original` is not a mapping or cannot be edited faithfully.
pub fn update_preserving_comments(original: &str, target: &Value) -> String {
    // The same guard for the editing library: a plain dump loses comments, not data.
    std::panic::catch_unwind(|| edit::update(original, target)).unwrap_or_else(|_| dump(target))
}

/// ISO form (`isoformat`-style) of a YAML timestamp (`2026-1-2T3:04:05Z` →
/// `2026-01-02 03:04:05+00:00`); `None` if it is not a valid timestamp.
pub fn canonical_timestamp(text: &str) -> Option<String> {
    resolve::Timestamp::parse(text)?.iso().ok()
}

enum Num {
    I(i64),
    F(f64),
}

impl Value {
    fn number(&self) -> Option<Num> {
        match *self {
            Value::Bool(b) => Some(Num::I(i64::from(b))),
            Value::Int(i) => Some(Num::I(i)),
            Value::Float(f) => Some(Num::F(f)),
            _ => None,
        }
    }

    /// Loose equality: `true == 1 == 1.0`, maps compare without order, NaN is unequal.
    pub fn loose_eq(&self, other: &Value) -> bool {
        match (self, other) {
            (Value::Null, Value::Null) => true,
            (Value::Str(a), Value::Str(b)) => a == b,
            (Value::Timestamp(a), Value::Timestamp(b)) => {
                canonical_timestamp(a).unwrap_or_else(|| a.clone())
                    == canonical_timestamp(b).unwrap_or_else(|| b.clone())
            }
            (Value::Seq(a), Value::Seq(b)) => {
                a.len() == b.len() && a.iter().zip(b).all(|(x, y)| x.loose_eq(y))
            }
            (Value::Map(a), Value::Map(b)) => {
                a.len() == b.len()
                    && a.iter()
                        .all(|(k, v)| b.iter().any(|(k2, v2)| k.loose_eq(k2) && v.loose_eq(v2)))
            }
            _ => match (self.number(), other.number()) {
                (Some(Num::I(a)), Some(Num::I(b))) => a == b,
                (Some(Num::F(a)), Some(Num::F(b))) => a == b,
                (Some(Num::I(i)), Some(Num::F(f))) | (Some(Num::F(f)), Some(Num::I(i))) => {
                    // Exact: no rounding of large integers through f64.
                    f.fract() == 0.0
                        && (-9.223_372_036_854_776e18..9.223_372_036_854_776e18).contains(&f)
                        && f as i64 == i
                }
                _ => false,
            },
        }
    }

    /// The value of string key `key` in a mapping.
    pub fn get(&self, key: &str) -> Option<&Value> {
        match self {
            Value::Map(pairs) => pairs
                .iter()
                .find(|(k, _)| matches!(k, Value::Str(s) if s == key))
                .map(|(_, v)| v),
            _ => None,
        }
    }

    /// Mutable [`Value::get`].
    pub fn get_mut(&mut self, key: &str) -> Option<&mut Value> {
        match self {
            Value::Map(pairs) => pairs
                .iter_mut()
                .find(|(k, _)| matches!(k, Value::Str(s) if s == key))
                .map(|(_, v)| v),
            _ => None,
        }
    }

    /// JSON for the QML side. Keys become text
    /// (`value_str`), timestamps become strings, NaN/infinity become `null`.
    pub fn to_json(&self) -> serde_json::Value {
        use serde_json::Value as J;
        match self {
            Value::Null => J::Null,
            Value::Bool(b) => J::Bool(*b),
            Value::Int(i) => J::from(*i),
            Value::Float(f) => serde_json::Number::from_f64(*f).map_or(J::Null, J::Number),
            Value::Str(s) | Value::Timestamp(s) => J::String(s.clone()),
            Value::Seq(items) => J::Array(items.iter().map(Value::to_json).collect()),
            Value::Map(pairs) => J::Object(
                pairs
                    .iter()
                    .map(|(k, v)| (super::model::value_str(k), v.to_json()))
                    .collect(),
            ),
        }
    }

    /// The inverse of [`Value::to_json`]; key order is kept. Integers outside `i64` become floats.
    pub fn from_json(j: &serde_json::Value) -> Value {
        use serde_json::Value as J;
        match j {
            J::Null => Value::Null,
            J::Bool(b) => Value::Bool(*b),
            J::Number(n) => n
                .as_i64()
                .map_or_else(|| Value::Float(n.as_f64().unwrap_or(f64::NAN)), Value::Int),
            J::String(s) => Value::Str(s.clone()),
            J::Array(items) => Value::Seq(items.iter().map(Value::from_json).collect()),
            J::Object(map) => Value::Map(
                map.iter()
                    .map(|(k, v)| (Value::Str(k.clone()), Value::from_json(v)))
                    .collect(),
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loose_eq_follows_golden() {
        assert!(Value::Bool(true).loose_eq(&Value::Int(1)));
        assert!(Value::Int(1).loose_eq(&Value::Float(1.0)));
        assert!(!Value::Float(f64::NAN).loose_eq(&Value::Float(f64::NAN)));
        assert!(!Value::Int(i64::MAX).loose_eq(&Value::Float(9.223_372_036_854_776e18)));
        let a = Value::Map(vec![
            (Value::Str("a".into()), Value::Int(1)),
            (Value::Str("b".into()), Value::Null),
        ]);
        let b = Value::Map(vec![
            (Value::Str("b".into()), Value::Null),
            (Value::Str("a".into()), Value::Bool(true)),
        ]);
        assert!(a.loose_eq(&b));
    }

    #[test]
    fn json_round_trip_keeps_order_and_types() {
        let v = load("z: 1\na: [1.5, true, ~, text]\nm: {k: -3}\n").expect("yaml");
        let j = v.to_json();
        assert_eq!(
            j.to_string(),
            r#"{"z":1,"a":[1.5,true,null,"text"],"m":{"k":-3}}"#
        );
        assert_eq!(Value::from_json(&j), v);
        let odd = load("1: 2001-02-03\nn: .nan\n").expect("yaml").to_json();
        assert_eq!(odd.to_string(), r#"{"1":"2001-02-03","n":null}"#);
    }

    #[test]
    fn rejects_what_value_cannot_hold() {
        assert!(load("a: &a [*a]\n").is_err()); // recursive alias: PyYAML builds a cycle
        let bomb = "a: &a [x, x, x, x, x, x, x, x, x, x]\n".to_owned()
            + &(b'b'..=b'h')
                .map(|c| {
                    let prev = (c - 1) as char;
                    format!(
                        "{0}: &{0} [*{1}, *{1}, *{1}, *{1}, *{1}, *{1}, *{1}, *{1}, *{1}, *{1}]\n",
                        c as char,
                        if prev == 'a' { 'a' } else { prev }
                    )
                })
                .collect::<String>();
        assert!(load(&bomb).is_err());
        assert!(load(&("[".repeat(600) + &"]".repeat(600))).is_err());
        assert!(load("a: \"\\ud83d\\ude80\"\n").is_err()); // lone surrogates: not a Rust String
    }
}
