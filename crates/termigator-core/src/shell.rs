//! POSIX shell quoting, joining and splitting: `quote`, `join` and `split`.
//!
//! * [`quote`]: empty string becomes `''`; a string made only of ASCII `[A-Za-z0-9_@%+=:,./-]`
//!   is returned unchanged (the safe class is ASCII only, so Polish letters
//!   and other non-ASCII word characters are quoted); anything else is wrapped in single quotes
//!   with each `'` written as `'"'"'`.
//! * [`join`]: quoted arguments separated by one space.
//! * [`split`]: POSIX mode with `comments=False` (what `shlex.split` uses): whitespace is
//!   `' \t\r\n'`, single quotes are literal, in double quotes a backslash only escapes `"` and
//!   `\`, outside quotes a backslash escapes any character (backslash-newline yields a newline,
//!   not a line continuation). Unclosed quotes and a trailing backslash are errors.
//!
//! Golden: `compat/golden/shell.json`, tests in `tests/shell.rs`.

/// Failure of [`split`]; the messages are the texts shown to the user.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum SplitError {
    /// A quote was opened and never closed.
    #[error("No closing quotation")]
    NoClosingQuotation,
    /// The input ends with a lone backslash.
    #[error("No escaped character")]
    NoEscapedCharacter,
}

fn is_safe(c: char) -> bool {
    c.is_ascii_alphanumeric()
        || matches!(c, '_' | '@' | '%' | '+' | '=' | ':' | ',' | '.' | '/' | '-')
}

/// Quotes one argument for a POSIX shell.
pub fn quote(s: &str) -> String {
    if s.is_empty() {
        return "''".to_owned();
    }
    if s.chars().all(is_safe) {
        return s.to_owned();
    }
    format!("'{}'", s.replace('\'', "'\"'\"'"))
}

/// Quotes and joins arguments with spaces.
pub fn join<S: AsRef<str>>(args: &[S]) -> String {
    args.iter()
        .map(|a| quote(a.as_ref()))
        .collect::<Vec<_>>()
        .join(" ")
}

#[derive(Clone, Copy, PartialEq)]
enum State {
    Space,
    Word,
    Quote(char),
    /// Backslash seen; remembers the quote to return to (`None`: outside quotes).
    Escape(Option<char>),
}

/// Splits a command line into arguments (POSIX rules, no comments).
pub fn split(s: &str) -> Result<Vec<String>, SplitError> {
    let mut out = Vec::new();
    let mut token = String::new();
    let mut quoted = false;
    let mut state = State::Space;
    for c in s.chars() {
        match state {
            State::Space | State::Word => match c {
                ' ' | '\t' | '\r' | '\n' => {
                    if !token.is_empty() || quoted {
                        out.push(std::mem::take(&mut token));
                        quoted = false;
                    }
                    state = State::Space;
                }
                '\\' => state = State::Escape(None),
                '\'' | '"' => state = State::Quote(c),
                _ => {
                    token.push(c);
                    state = State::Word;
                }
            },
            State::Quote(q) => {
                if c == q {
                    quoted = true;
                    state = State::Word;
                } else if c == '\\' && q == '"' {
                    state = State::Escape(Some(q));
                } else {
                    token.push(c);
                }
            }
            State::Escape(back) => {
                if let Some(q) = back
                    && c != '\\'
                    && c != q
                {
                    token.push('\\');
                }
                token.push(c);
                state = back.map_or(State::Word, State::Quote);
            }
        }
    }
    match state {
        State::Quote(_) => Err(SplitError::NoClosingQuotation),
        State::Escape(_) => Err(SplitError::NoEscapedCharacter),
        State::Space | State::Word => {
            if !token.is_empty() || quoted {
                out.push(token);
            }
            Ok(out)
        }
    }
}

/// One argument for a Windows command line, so that `CommandLineToArgvW` (and the MSVC runtime)
/// give back exactly `s`: unchanged when it holds no space, tab, newline, vertical tab or `"`;
/// otherwise in double quotes, with every `"` as `\"` and the backslashes before a `"` (or
/// before the closing quote) doubled. The empty string becomes `""`.
pub fn quote_windows(s: &str) -> String {
    if s.is_empty() {
        return "\"\"".to_owned();
    }
    if !s.contains([' ', '\t', '\n', '\x0b', '"']) {
        return s.to_owned();
    }
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    let mut backslashes = 0usize;
    for c in s.chars() {
        match c {
            '\\' => backslashes += 1,
            '"' => {
                out.extend(std::iter::repeat_n('\\', backslashes * 2 + 1));
                out.push('"');
                backslashes = 0;
            }
            _ => {
                out.extend(std::iter::repeat_n('\\', backslashes));
                out.push(c);
                backslashes = 0;
            }
        }
    }
    out.extend(std::iter::repeat_n('\\', backslashes * 2));
    out.push('"');
    out
}

/// [`quote_windows`] arguments separated by one space.
pub fn join_windows<S: AsRef<str>>(args: &[S]) -> String {
    args.iter()
        .map(|a| quote_windows(a.as_ref()))
        .collect::<Vec<_>>()
        .join(" ")
}
