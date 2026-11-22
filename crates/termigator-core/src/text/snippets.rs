//! Snippet placeholders and prompt detection.
//!
//! A snippet is `{name, command, enter}`. In the command `{host}`, `{user}`, `{label}` and
//! `{port}` are filled per target session, `{?Question}` is asked once before sending, and
//! `{{` / `}}` are literal braces. [`waiting_for_user_input`] tells whether the terminal's last
//! line is a credential / confirmation prompt.
//!
//! Shortcut: [`normalize_snippets`] stringifies list/dict scalars of a hand-edited YAML like
//! compact JSON; only scalars appear in real configs.
//!
//! Golden: `compat/golden/text_snippets.json`, tests in `tests/text_snippets.rs`.

use std::sync::LazyLock;

use regex::{Captures, Regex};
use serde_json::Value;

use super::asciicast::shortest_float_repr;

static FIELD: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\{\{|\}\}|\{(\?[^{}]+|host|user|label|port)\}")
        .unwrap_or_else(|e| unreachable!("static regex: {e}"))
});

// `[\s\x1c-\x1f]` is the whitespace class including the separators U+001C-U+001F.
static ASKS_INPUT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)(password|passphrase|passcode|verification code|one-time|\botp\b|\bpin\b|\btoken\b|\(yes/no[^)]*\)|continue connecting)[^\n]*[:?][\s\x1c-\x1f]*$",
    )
    .unwrap_or_else(|e| unreachable!("static regex: {e}"))
});

static ANSI: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\x1b(\[[0-?]*[ -/]*[@-~]|\][^\x07\x1b]*(\x07|\x1b\\)|.)")
        .unwrap_or_else(|e| unreachable!("static regex: {e}"))
});

/// A snippet as kept in `config.yaml` -> `snippets`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Snippet {
    pub name: String,
    pub command: String,
    pub enter: bool,
}

/// What `{host}`, `{user}`, `{label}` and `{port}` expand to; an empty field is "missing".
#[derive(Debug, Clone, Default)]
pub struct SnippetConn {
    pub host: String,
    pub user: String,
    pub label: String,
    pub port: String,
}

/// Whitespace test (Rust's `char::is_whitespace` plus the separators U+001C-U+001F).
pub(crate) fn is_space(c: char) -> bool {
    c.is_whitespace() || ('\x1c'..='\x1f').contains(&c)
}

/// Strips leading/trailing Unicode whitespace (including U+001C-U+001F).
pub(crate) fn strip_space(s: &str) -> &str {
    s.trim_matches(is_space)
}

fn truthy(v: &Value) -> bool {
    match v {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64().is_some_and(|f| f != 0.0),
        Value::String(s) => !s.is_empty(),
        Value::Array(a) => !a.is_empty(),
        Value::Object(o) => !o.is_empty(),
    }
}

/// String form of the values a YAML scalar can take.
fn value_str(v: &Value) -> String {
    match v {
        Value::Null => "None".to_owned(),
        Value::Bool(true) => "True".to_owned(),
        Value::Bool(false) => "False".to_owned(),
        Value::Number(n) => match (n.as_i64(), n.as_u64()) {
            (Some(i), _) => i.to_string(),
            (_, Some(u)) => u.to_string(),
            _ => shortest_float_repr(n.as_f64().unwrap_or(0.0)),
        },
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

/// `str(d.get(key) or "")`.
fn str_or_empty(d: &serde_json::Map<String, Value>, key: &str) -> String {
    d.get(key)
        .filter(|v| truthy(v))
        .map(value_str)
        .unwrap_or_default()
}

/// First line of `s`, split at any line separator (empty when there is none).
fn first_line(s: &str) -> &str {
    let end = s
        .find(|c: char| {
            matches!(
                c,
                '\n' | '\r'
                    | '\x0b'
                    | '\x0c'
                    | '\x1c'
                    | '\x1d'
                    | '\x1e'
                    | '\u{85}'
                    | '\u{2028}'
                    | '\u{2029}'
            )
        })
        .unwrap_or(s.len());
    &s[..end]
}

/// Snippets from YAML, tolerant of hand edits: entries without a command are dropped.
pub fn normalize_snippets(raw: &Value) -> Vec<Snippet> {
    let Some(list) = raw.as_array() else {
        return Vec::new();
    };
    list.iter()
        .filter_map(Value::as_object)
        .filter_map(|s| {
            let command = str_or_empty(s, "command");
            if strip_space(&command).is_empty() {
                return None;
            }
            let name = match s.get("name").filter(|v| truthy(v)) {
                Some(v) => value_str(v),
                None => first_line(&command).to_owned(),
            };
            Some(Snippet {
                name: name.chars().take(80).collect(),
                enter: s.get("enter").is_none_or(truthy),
                command,
            })
        })
        .collect()
}

/// `{?…}` prompts in order of first appearance, without duplicates.
pub fn questions(command: &str) -> Vec<String> {
    let mut seen: Vec<String> = Vec::new();
    for m in FIELD.captures_iter(command) {
        if let Some(q) = m.get(1).and_then(|g| g.as_str().strip_prefix('?'))
            && !seen.iter().any(|s| s == q)
        {
            seen.push(q.to_owned());
        }
    }
    seen
}

/// Fills placeholders for one target. Unanswered questions become empty strings.
pub fn expand(
    command: &str,
    conn: &SnippetConn,
    answers: &std::collections::HashMap<String, String>,
) -> String {
    FIELD
        .replace_all(command, |m: &Captures| {
            match (&m[0], m.get(1).map(|g| g.as_str())) {
                ("{{", _) => "{".to_owned(),
                ("}}", _) => "}".to_owned(),
                (_, Some(g)) => match g.strip_prefix('?') {
                    Some(q) => answers.get(q).cloned().unwrap_or_default(),
                    None => match g {
                        "port" if conn.port.is_empty() => "22".to_owned(),
                        "port" => conn.port.clone(),
                        "host" => conn.host.clone(),
                        "user" => conn.user.clone(),
                        _ => conn.label.clone(),
                    },
                },
                _ => m[0].to_owned(),
            }
        })
        .into_owned()
}

/// What is typed into the terminal: the lines as they are, then Enter if asked.
pub fn text_to_send(command: &str, enter: bool) -> String {
    let body = command.replace("\r\n", "\n");
    format!(
        "{}{}",
        body.trim_end_matches('\n'),
        if enter { "\n" } else { "" }
    )
}

/// True when the last line of terminal output is a credential / confirmation prompt (ssh
/// password, key passphrase, OTP, host-key yes/no).
pub fn waiting_for_user_input(output_tail: &str) -> bool {
    let text = ANSI.replace_all(output_tail, "").replace('\r', "\n");
    let last = text
        .trim_end_matches(' ')
        .rsplit('\n')
        .next()
        .unwrap_or_default();
    ASKS_INPUT.is_match(last)
}
