//! OSC 7 (current directory) scanner over split chunks (`Osc7Scanner`
//! and `OSC7_SHELL_SNIPPET`).
//!
//! Finds `ESC ] 7 ; file://host/path` reports ended by BEL or `ESC \` in terminal output,
//! reassembles sequences split across chunks and percent-decodes the path. The
//! scanner works on decoded text; bytes that are not valid UTF-8 after percent-decoding become
//! U+FFFD (a Rust `String` cannot hold raw invalid bytes).
//!
//! Golden: `compat/golden/text_osc7.json`, tests in `tests/text_osc7.rs`.

const START: &str = "\x1b]7;";
const MAX_PENDING: usize = 4096;

/// One line for bash and zsh: report `$PWD` via OSC 7 before every prompt.
pub const OSC7_SHELL_SNIPPET: &str = concat!(
    "__termigator_osc7(){ printf '\\033]7;file://%s%s\\007' \"${HOSTNAME:-$HOST}\" \"$PWD\"; }; ",
    "if [ -n \"$ZSH_VERSION\" ]; then precmd_functions+=(__termigator_osc7); ",
    "else PROMPT_COMMAND=\"__termigator_osc7${PROMPT_COMMAND:+;$PROMPT_COMMAND}\"; fi; __termigator_osc7"
);

/// The remote command of an SSH tab that reports its folder without typing anything into the
/// terminal: `sh -c` (valid under bash, zsh and fish alike — the script travels as base64, so the
/// single quotes hold no quoting of their own) runs [`osc7_bootstrap.sh`](osc7_bootstrap.sh), and
/// falls back to a plain login shell when the host cannot decode it. `ssh` runs a command without
/// the MOTD and the "Last login" line, which only an interactive login prints.
pub fn tracking_remote_command() -> String {
    use base64::Engine as _;
    let script =
        base64::engine::general_purpose::STANDARD.encode(include_str!("osc7_bootstrap.sh"));
    format!("sh -c 'eval \"$(echo {script} | base64 -d 2>/dev/null)\"; exec \"${{SHELL:-sh}}\" -l'")
}

/// [`tracking_remote_command`] for a local Linux shell (WSL): `sh -c <script>` as argv, without the
/// MOTD and "Last login" (nothing logged in).
pub fn tracking_local_argv() -> Vec<String> {
    use base64::Engine as _;
    let script =
        base64::engine::general_purpose::STANDARD.encode(include_str!("osc7_bootstrap.sh"));
    vec![
        "sh".to_owned(),
        "-c".to_owned(),
        format!(
            "ERN_NO_MOTD=1; eval \"$(echo {script} | base64 -d 2>/dev/null)\"; exec \"${{SHELL:-sh}}\" -l"
        ),
    ]
}

/// Streaming scanner; keep one per terminal.
#[derive(Default)]
pub struct Osc7Scanner {
    pending: String,
}

fn hex(b: Option<&u8>) -> Option<u8> {
    let d = char::from(*b?).to_digit(16)?;
    u8::try_from(d).ok()
}

/// `urllib.parse.unquote`: `%XX` becomes a byte, anything else is untouched.
fn unquote(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%'
            && let (Some(h), Some(l)) = (hex(b.get(i + 1)), hex(b.get(i + 2)))
        {
            out.push(h * 16 + l);
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn path_of(uri: &str) -> Option<String> {
    let path = match uri.strip_prefix("file://") {
        Some(after) => after.find('/').map_or("", |i| &after[i..]),
        None => uri,
    };
    let path = unquote(path);
    path.starts_with('/').then_some(path)
}

impl Osc7Scanner {
    pub fn new() -> Self {
        Self::default()
    }

    /// Directories reported in `chunk` (plus any sequence completed from earlier chunks).
    pub fn feed(&mut self, chunk: &str) -> Vec<String> {
        let all = std::mem::take(&mut self.pending) + chunk;
        let mut data = all.as_str();
        let mut found = Vec::new();
        loop {
            let Some(i) = data.find(START) else {
                // keep a possible partial "ESC ] 7" at the very end for the next chunk
                let skip = data.chars().count().saturating_sub(3);
                let tail: String = data.chars().skip(skip).collect();
                self.pending = tail
                    .rfind('\x1b')
                    .map_or_else(String::new, |k| tail[k..].to_owned());
                return found;
            };
            let rest = &data[i + START.len()..];
            let end = match (rest.find('\x07'), rest.find("\x1b\\")) {
                (Some(a), Some(b)) => a.min(b),
                (Some(a), None) | (None, Some(a)) => a,
                (None, None) => {
                    self.pending = data[i..].chars().take(MAX_PENDING).collect();
                    return found;
                }
            };
            if let Some(p) = path_of(&rest[..end]) {
                found.push(p);
            }
            // both terminators start with an ASCII byte: skipping one byte stays on a boundary
            data = &rest[end + 1..];
        }
    }
}
