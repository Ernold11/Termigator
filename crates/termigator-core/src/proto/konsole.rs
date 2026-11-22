//! External terminal template (`launch.template`) and xdotool command lines (`konsole_argv`,
//! the argv part of `spawn_konsole` and the `xdotool_*` helpers).
//!
//! [`konsole_argv`] expands the template (default `konsole -e bash -lc {cmd_quoted}`) with
//! `str.format`-style rules for the one name `cmd_quoted` (the inner command through
//! [`shell::quote`]): `{{` / `}}` are literal braces; an unknown name (`{foo}`) makes the whole
//! template fall back to a plain replacement of `{cmd_quoted}`; then [`shell::split`] gives the argv.
//! [`external_terminal_argv`] adds what `spawn_konsole` does before `Popen`: the "no command" and "not found in PATH" messages and
//! the resolved executable. Spawning (new session, null stdio) and the xdotool calls are not part of this module.
//!
//! A positional field (`{}` or `{0}`) in the template is an `Err` ("Replacement index 0 out of
//! range" style text); so is unbalanced braces. Conversions other than `!s` / `!r`, format specs
//! and attribute or index access on `cmd_quoted` (`{cmd_quoted:>9}`, `{cmd_quoted[0]}`) are
//! reported as an error.
//! Golden fixtures: `compat/golden/tools_konsole_argv.json`, `tools_xdotool.json`.

use crate::config::doc::Value;
use crate::config::model::{strip_space, truthy, value_str};
use crate::proto::ssh::quote_repr;
use crate::shell;

/// The template used when `launch.template` is unset or blank.
pub const DEFAULT_TEMPLATE: &str = "konsole -e bash -lc {cmd_quoted}";

const NAME: &str = "cmd_quoted";

/// One replacement field of the template, split into name, conversion and format spec.
struct Field {
    name: String,
    conversion: Option<char>,
    spec: String,
}

/// Parses the field that starts after a `{` at `chars[*i]`; leaves `*i` after its closing `}`.
fn parse_field(chars: &[char], i: &mut usize) -> Result<Field, String> {
    let start = *i;
    let mut sep = None;
    while *i < chars.len() {
        let d = chars[*i];
        *i += 1;
        match d {
            '{' => return Err("unexpected '{' in field name".to_owned()),
            '[' => {
                while *i < chars.len() && chars[*i] != ']' {
                    *i += 1;
                }
            }
            '}' | ':' | '!' => {
                sep = Some(d);
                break;
            }
            _ => {}
        }
    }
    let name_end = if sep.is_some() { *i - 1 } else { *i };
    let mut field = Field {
        name: chars[start..name_end].iter().collect(),
        conversion: None,
        spec: String::new(),
    };
    match sep {
        None => return Err("expected '}' before end of string".to_owned()),
        Some('}') => return Ok(field),
        Some(sep) => {
            if sep == '!' {
                let Some(&conv) = chars.get(*i) else {
                    return Err("end of string while looking for conversion specifier".to_owned());
                };
                *i += 1;
                field.conversion = Some(conv);
                if let Some(&next) = chars.get(*i) {
                    *i += 1;
                    if next == '}' {
                        return Ok(field);
                    }
                    if next != ':' {
                        return Err("expected ':' after conversion specifier".to_owned());
                    }
                }
            }
        }
    }
    let spec_start = *i;
    let mut depth = 1;
    while *i < chars.len() {
        let d = chars[*i];
        *i += 1;
        match d {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    field.spec = chars[spec_start..*i - 1].iter().collect();
                    return Ok(field);
                }
            }
            _ => {}
        }
    }
    Err("unmatched '{' in format spec".to_owned())
}

/// `template.format(cmd_quoted=quoted)` with `str.format` semantics for the supported names; an
/// unknown name falls back to `template.replace("{cmd_quoted}", quoted)`.
fn expand(template: &str, quoted: &str) -> Result<String, String> {
    let chars: Vec<char> = template.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        i += 1;
        match c {
            '{' | '}' if chars.get(i) == Some(&c) => {
                i += 1;
                out.push(c);
            }
            '}' => return Err("Single '}' encountered in format string".to_owned()),
            '{' if i >= chars.len() => {
                return Err("Single '{' encountered in format string".to_owned());
            }
            '{' => {
                let field = parse_field(&chars, &mut i)?;
                let first_end = field.name.find(['.', '[']).unwrap_or(field.name.len());
                let first = &field.name[..first_end];
                if first.is_empty() || first.chars().all(|c| c.is_ascii_digit()) {
                    let index: u64 = first.parse().unwrap_or(0);
                    return Err(format!(
                        "Replacement index {index} out of range for positional args tuple"
                    ));
                }
                if first != NAME {
                    return Ok(template.replace("{cmd_quoted}", quoted)); // KeyError
                }
                if first_end != field.name.len() || !field.spec.is_empty() {
                    return Err(format!(
                        "Unsupported replacement field for {NAME} in launch.template (use {{{NAME}}})."
                    ));
                }
                match field.conversion {
                    None | Some('s') => out.push_str(quoted),
                    Some('r') => out.push_str(&quote_repr(quoted)),
                    Some('a') => {
                        return Err(format!(
                            "Unsupported conversion '!a' for {NAME} in launch.template."
                        ));
                    }
                    Some(other) => return Err(format!("Unknown conversion specifier {other}")),
                }
            }
            c => out.push(c),
        }
    }
    Ok(out)
}

/// The external terminal's argv for `inner_command`; `launch` is the `launch` config section
/// (`template` read from it).
pub fn konsole_argv(launch: &Value, inner_command: &str) -> Result<Vec<String>, String> {
    let configured = launch.get("template").filter(|v| truthy(v)).map(value_str);
    let mut tpl = strip_space(configured.as_deref().unwrap_or(DEFAULT_TEMPLATE)).to_owned();
    if tpl.is_empty() {
        tpl = DEFAULT_TEMPLATE.to_owned();
    }
    let line = expand(&tpl, &shell::quote(inner_command))?;
    shell::split(&line).map_err(|e| e.to_string())
}

/// [`konsole_argv`] with the executable resolved as `spawn_konsole` does before starting it
/// (`which` is `shutil.which`: the resolved path or `None`).
pub fn external_terminal_argv(
    launch: &Value,
    inner_command: &str,
    which: &dyn Fn(&str) -> Option<String>,
) -> Result<Vec<String>, String> {
    let argv = konsole_argv(launch, inner_command)?;
    let Some(exe0) = argv.first() else {
        return Err("After expanding launch.template there is no command.".to_owned());
    };
    let Some(exe) = which(exe0) else {
        return Err(format!(
            "`{exe0}` not found in PATH. Install Konsole (KDE) \
             or set launch.template to another terminal emulator."
        ));
    };
    let mut out = vec![exe];
    out.extend(argv.into_iter().skip(1));
    Ok(out)
}

/// `xdotool search --pid PID --onlyvisible .`: the windows of one process.
pub fn xdotool_search_pid_argv(pid: u32) -> Vec<String> {
    [
        "xdotool",
        "search",
        "--pid",
        &pid.to_string(),
        "--onlyvisible",
        ".",
    ]
    .map(str::to_owned)
    .into()
}

/// `xdotool search --name --onlyvisible Konsole`: the old name heuristic (several Konsole windows
/// make it unreliable).
pub fn xdotool_search_name_argv() -> Vec<String> {
    ["xdotool", "search", "--name", "--onlyvisible", "Konsole"]
        .map(str::to_owned)
        .into()
}

/// `xdotool getwindowgeometry --shell WIN`.
pub fn xdotool_geometry_argv(win_id: &str) -> Vec<String> {
    ["xdotool", "getwindowgeometry", "--shell", win_id]
        .map(str::to_owned)
        .into()
}

/// The last all-digit (ASCII) line of an `xdotool search` output (the topmost match); `None` if
/// none.
pub fn parse_window_id(output: &str) -> Option<String> {
    strip_space(output)
        .lines()
        .rev()
        .find(|l| {
            let t = strip_space(l);
            !t.is_empty() && t.chars().all(|c| c.is_ascii_digit())
        })
        .map(str::to_owned)
}

/// `(x, y, width, height)` from `xdotool getwindowgeometry --shell` (`X=`, `Y=`, `WIDTH=`,
/// `HEIGHT=` lines); `None` when one is missing.
pub fn parse_window_geometry(output: &str) -> Option<(i64, i64, i64, i64)> {
    let (mut x, mut y, mut w, mut h) = (None, None, None, None);
    for line in output.lines() {
        let Some((key, val)) = strip_space(line).split_once('=') else {
            continue;
        };
        if val.is_empty() || !val.chars().all(|c| c.is_ascii_digit()) {
            continue;
        }
        let slot = match key {
            "X" => &mut x,
            "Y" => &mut y,
            "WIDTH" => &mut w,
            "HEIGHT" => &mut h,
            _ => continue,
        };
        *slot = val.parse().ok();
    }
    Some((x?, y?, w?, h?))
}
