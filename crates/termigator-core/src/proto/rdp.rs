//! FreeRDP client selection and command line, password via stdin.
//!
//! Contract: `Ok(command)` or `Err(user-facing message)`, texts are stable. Client
//! resolution: `launch.rdp_client`, else the first of [`RDP_CLIENT_CANDIDATES`] (FreeRDP 3 before
//! 2, Wayland last) that `env.which` finds, else the first candidate. All redirection flags are emitted
//! in the syntax both FreeRDP 2 and 3 parse. A stored password never reaches argv: the line is
//! `printf '%s\n' "$TERMIGATOR_RDP_PW" | client … /from-stdin`, the value travels in
//! [`rdp_password_env`] (which refuses sealed `enc:` tokens) and clipboard copies are masked
//! ([`clipboard_rdp_command`]). Guards (ARCHITECTURE.md §7, guard 5): first character of host,
//! user, domain and gateway values not `/`, `-` or `+`; sub-option values (gateway, shared
//! folders) without `,`; shared folders absolute; resolution `^\d{3,5}x\d{3,5}$`.
//!
//! Machine access goes through [`Env`] (`which`, `$HOME` for `~` in shared folders), shared with
//! `proto::ssh`, so tests are deterministic. Known limit: `~user` in a shared
//! folder is not expanded (only `~` and `~/...`).
//!
//! Golden: `compat/golden/rdp_*.json`, tests in `tests/proto_rdp.rs`.

use regex::Regex;

use crate::config::doc::Value;
use crate::config::model::{strip_space, truthy, value_str};
use crate::proto::ssh::{Env, quote_repr};
use crate::proto::vnc::{parse_port, text, text_stripped};
use crate::shell;
use crate::vault::{SEALED_PREFIX, Secret, VaultError};

pub const RDP_DEFAULT_PORT: i64 = 3389;

/// FreeRDP binaries, in preference order (FreeRDP 3 first, SDL frontends included, 2.x next,
/// Wayland last).
pub const RDP_CLIENT_CANDIDATES: [&str; 6] = [
    "xfreerdp3",
    "sdl-freerdp3",
    "xfreerdp",
    "sdl-freerdp",
    "wlfreerdp3",
    "wlfreerdp",
];

/// Keys fully owned by the RDP dialog; other keys on a connection are preserved on save.
pub const RDP_DIALOG_MANAGED_KEYS: &[&str] = &[
    "label",
    "protocol",
    "host",
    "user",
    "password",
    "domain",
    "port",
    "resolution",
    "fullscreen",
    "extra_rdp_args",
    "terminal",
    "group",
    "credential",
    "clipboard",
    "home_drive",
    "shared_folders",
    "sound",
    "microphone",
    "multimon",
    "dynamic_resolution",
    "gateway",
    "gateway_user",
    "gateway_domain",
    "ssh_gateway",
];

/// FreeRDP executable: `launch.rdp_client` if set, else the first candidate found.
pub fn resolve_rdp_client(launch: &Value, env: &Env) -> String {
    let over = text_stripped(launch.get("rdp_client"));
    if !over.is_empty() {
        return over;
    }
    RDP_CLIENT_CANDIDATES
        .iter()
        .find(|n| (env.which)(n).is_some())
        .unwrap_or(&RDP_CLIENT_CANDIDATES[0])
        .to_string()
}

fn value_guard(s: &str) -> Option<&'static str> {
    let t = strip_space(s);
    let Some(first) = t.chars().next() else {
        return Some("empty value");
    };
    if "/-+".contains(first) {
        return Some("must not start with '/', '-' or '+' (pass options via Extra RDP arguments)");
    }
    None
}

/// A value inside a comma-separated FreeRDP sub-option list: a comma starts the next one.
fn sub_value(name: &str, value: &str) -> Option<String> {
    value
        .contains(',')
        .then(|| format!("{name} {}: must not contain ','", quote_repr(value)))
}

/// Splits on line boundaries (`\n`, `\r\n`, `\r`, …); a trailing break adds no empty line.
fn splitlines(t: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut start = 0;
    let mut chars = t.char_indices().peekable();
    while let Some((i, c)) = chars.next() {
        if matches!(
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
        ) {
            out.push(&t[start..i]);
            let mut end = i + c.len_utf8();
            if c == '\r' && chars.peek().is_some_and(|&(_, n)| n == '\n') {
                chars.next();
                end += 1;
            }
            start = end;
        }
    }
    if start < t.len() {
        out.push(&t[start..]);
    }
    out
}

fn shared_folders(raw: Option<&Value>) -> Vec<String> {
    match raw {
        Some(Value::Seq(items)) => items
            .iter()
            .map(|x| strip_space(&value_str(x)).to_owned())
            .filter(|s| !s.is_empty())
            .collect(),
        other => splitlines(&text(other))
            .into_iter()
            .map(|l| strip_space(l).to_owned())
            .filter(|s| !s.is_empty())
            .collect(),
    }
}

fn is_true(v: Option<&Value>) -> bool {
    matches!(v, Some(Value::Bool(true)))
}

/// `re.sub(r"[^A-Za-z0-9_-]+", "_", s)`.
fn non_word_runs(s: &str) -> String {
    let mut out = String::new();
    let mut in_run = false;
    for c in s.chars() {
        if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
            out.push(c);
            in_run = false;
        } else if !in_run {
            out.push('_');
            in_run = true;
        }
    }
    out
}

/// Redirections, multimon, dynamic resolution, RD Gateway.
fn append_extras(conn: &Value, argv: &mut Vec<String>, home: Option<&str>) -> Result<(), String> {
    match conn.get("clipboard") {
        Some(Value::Bool(true)) => argv.push("+clipboard".to_owned()),
        Some(Value::Bool(false)) => argv.push("-clipboard".to_owned()),
        _ => {}
    }
    if is_true(conn.get("home_drive")) {
        argv.push("+home-drive".to_owned());
    }
    for path in shared_folders(conn.get("shared_folders")) {
        let path = super::ssh::expand_user(&path, home);
        if !path.starts_with('/') {
            return Err(format!(
                "shared folder {}: use an absolute path.",
                quote_repr(&path)
            ));
        }
        if let Some(e) = sub_value("shared folder", &path) {
            return Err(e);
        }
        let base = path.trim_end_matches('/').rsplit('/').next().unwrap_or("");
        let name = non_word_runs(base);
        let name = if name.is_empty() { "root" } else { &name };
        argv.push(format!("/drive:{name},{path}"));
    }
    for (key, flag) in [
        ("sound", "/sound"),
        ("microphone", "/microphone"),
        ("multimon", "/multimon"),
    ] {
        if is_true(conn.get(key)) {
            argv.push(flag.to_owned());
        }
    }
    if is_true(conn.get("dynamic_resolution")) && !is_true(conn.get("fullscreen")) {
        argv.push("+dynamic-resolution".to_owned());
    }
    let gw = text_stripped(conn.get("gateway"));
    if !gw.is_empty() {
        let mut parts = vec![format!("g:{gw}")];
        for (key, prefix) in [("gateway_user", "u"), ("gateway_domain", "d")] {
            let v = text_stripped(conn.get(key));
            if !v.is_empty() {
                parts.push(format!("{prefix}:{v}"));
            }
        }
        for key in ["gateway", "gateway_user", "gateway_domain"] {
            let v = text_stripped(conn.get(key));
            if !v.is_empty() {
                if let Some(e) = value_guard(&v) {
                    return Err(format!("{key} {}: {e}", quote_repr(&v)));
                }
                if let Some(e) = sub_value(key, &v) {
                    return Err(e);
                }
            }
        }
        argv.push(format!("/gateway:{}", parts.join(",")));
    }
    Ok(())
}

/// `launch.rdp_resolution` when unset: the size of the screen the window is on.
pub const RDP_RESOLUTION_DEFAULT: &str = "screen";

/// Whether `text` is a usable `launch.rdp_resolution` / host `resolution`: empty (the client's own
/// default), `screen`, `fullscreen` or `WxH`.
pub fn resolution_setting_is_valid(text: &str) -> bool {
    matches!(text, "" | "screen" | "fullscreen")
        || Regex::new(r"^\d{3,5}x\d{3,5}$").is_ok_and(|re| re.is_match(text))
}

/// `conn` with the program-wide start size applied (`launch.rdp_resolution`, default `screen`),
/// unless the host chose its own (`fullscreen` or a `resolution`). `screen` is the `WxH` of the
/// display the window is on; without it nothing is set and the client picks. Pure, so the
/// FreeRDP line and the Windows `.rdp` file both see the result.
pub fn with_default_geometry(conn: &Value, launch: &Value, screen: Option<&str>) -> Value {
    if is_true(conn.get("fullscreen")) || !text_stripped(conn.get("resolution")).is_empty() {
        return conn.clone();
    }
    let setting = match launch.get("rdp_resolution") {
        None | Some(Value::Null) => RDP_RESOLUTION_DEFAULT.to_owned(),
        Some(v) => value_str(v).trim().to_lowercase(),
    };
    let (key, value) = match setting.as_str() {
        "screen" => match screen.filter(|s| resolution_setting_is_valid(s) && !s.is_empty()) {
            Some(size) => ("resolution", Value::Str(size.to_owned())),
            None => return conn.clone(),
        },
        "fullscreen" => ("fullscreen", Value::Bool(true)),
        size if resolution_setting_is_valid(size) && !size.is_empty() => {
            ("resolution", Value::Str(size.to_owned()))
        }
        _ => return conn.clone(),
    };
    let mut out = conn.clone();
    if let Value::Map(pairs) = &mut out {
        pairs.retain(|(k, _)| !matches!(k, Value::Str(s) if s == key));
        pairs.push((Value::Str(key.to_owned()), value));
    }
    out
}

/// The shell line launching FreeRDP for `conn`.
pub fn build_rdp_command(conn: &Value, launch: &Value, env: &Env) -> Result<String, String> {
    let host = text_stripped(conn.get("host"));
    if host.is_empty() {
        return Err("Host is missing.".to_owned());
    }
    if let Some(g) = value_guard(&host) {
        return Err(format!("host {}: {g}", quote_repr(&host)));
    }
    let mut argv = vec![resolve_rdp_client(launch, env)];
    let port = parse_port(conn, RDP_DEFAULT_PORT)?;
    if port != RDP_DEFAULT_PORT {
        argv.push(format!("/port:{port}"));
    }
    argv.push(format!("/v:{host}"));

    let user = text_stripped(conn.get("user"));
    if !user.is_empty() {
        if let Some(e) = value_guard(&user) {
            return Err(format!("user {}: {e}", quote_repr(&user)));
        }
        argv.push(format!("/u:{user}"));
    }
    let domain = text_stripped(conn.get("domain"));
    if !domain.is_empty() {
        if let Some(e) = value_guard(&domain) {
            return Err(format!("domain {}: {e}", quote_repr(&domain)));
        }
        argv.push(format!("/d:{domain}"));
    }

    if is_true(conn.get("fullscreen")) {
        argv.push("/f".to_owned());
    } else {
        let resolution = text_stripped(conn.get("resolution"));
        if !resolution.is_empty() {
            // Unicode-aware `\d`.
            let ok = Regex::new(r"^\d{3,5}x\d{3,5}$").is_ok_and(|re| re.is_match(&resolution));
            if !ok {
                return Err("resolution must look like 1920x1080.".to_owned());
            }
            argv.push(format!("/size:{resolution}"));
        }
    }

    append_extras(conn, &mut argv, env.home.as_deref())?;

    match conn.get("extra_rdp_args") {
        Some(Value::Seq(items)) => argv.extend(items.iter().map(value_str)),
        Some(x) if truthy(x) => argv.extend(
            shell::split(&value_str(x))
                .map_err(|_| "Could not parse extra_rdp_args.".to_owned())?,
        ),
        _ => {}
    }

    if !text(conn.get("password")).is_empty() {
        if user.is_empty() {
            return Err(
                "Password is set but user is empty. FreeRDP would read the piped \
                        password as the username — set user as well."
                    .to_owned(),
            );
        }
        // /from-stdin + a pipe from $TERMIGATOR_RDP_PW keeps the password out of `ps`.
        argv.push("/from-stdin".to_owned());
        return Ok(format!(
            "printf '%s\\n' \"$TERMIGATOR_RDP_PW\" | {}",
            shell::join(&argv)
        ));
    }
    Ok(shell::join(&argv))
}

/// Windows: the `.rdp` file `mstsc.exe` opens for `conn` (there is no FreeRDP). `password_blob`
/// is the password encrypted for this user with DPAPI, hex (`password 51:b:`), or `None`: mstsc
/// then asks for it itself. `home_drive` is the drive letter (`C:`) of the user's profile, for
/// `home_drive`. Shared folders map to their drives (mstsc redirects drives, not folders).
/// FreeRDP's own extra arguments and the SSH gateway have no `.rdp` form: refused, not ignored.
pub fn rdp_file(
    conn: &Value,
    password_blob: Option<&str>,
    home_drive: Option<&str>,
) -> Result<String, String> {
    let host = text_stripped(conn.get("host"));
    if host.is_empty() {
        return Err("Host is missing.".to_owned());
    }
    // One value per line: a line break would start another setting.
    let clean = |name: &str, v: &str| -> Result<(), String> {
        if v.contains(['\r', '\n']) {
            return Err(format!("{name} {}: must be one line", quote_repr(v)));
        }
        Ok(())
    };
    clean("host", &host)?;
    if conn
        .get("ssh_gateway")
        .is_some_and(|g| truthy(g) && !strip_space(&value_str(g)).is_empty())
    {
        return Err("An SSH gateway for RDP is not supported on Windows yet.".to_owned());
    }
    if conn.get("extra_rdp_args").is_some_and(truthy) {
        return Err(
            "Extra RDP arguments are FreeRDP options; Windows uses mstsc. Clear them in the entry."
                .to_owned(),
        );
    }
    let port = parse_port(conn, RDP_DEFAULT_PORT)?;
    let mut lines = vec![if port == RDP_DEFAULT_PORT {
        format!("full address:s:{host}")
    } else {
        format!("full address:s:{host}:{port}")
    }];
    let user = text_stripped(conn.get("user"));
    let domain = text_stripped(conn.get("domain"));
    clean("user", &user)?;
    clean("domain", &domain)?;
    if !user.is_empty() {
        lines.push(if domain.is_empty() {
            format!("username:s:{user}")
        } else {
            format!("username:s:{domain}\\{user}")
        });
    }
    if let Some(blob) = password_blob {
        if user.is_empty() {
            return Err("Password is set but user is empty — set user as well.".to_owned());
        }
        lines.push(format!("password 51:b:{blob}"));
        lines.push("prompt for credentials:i:0".to_owned());
    }
    if is_true(conn.get("fullscreen")) {
        lines.push("screen mode id:i:2".to_owned());
    } else {
        lines.push("screen mode id:i:1".to_owned());
        let resolution = text_stripped(conn.get("resolution"));
        if !resolution.is_empty() {
            let size = resolution
                .split_once('x')
                .and_then(|(w, h)| Some((w.parse::<u32>().ok()?, h.parse::<u32>().ok()?)))
                .filter(|(w, h)| (100..=99_999).contains(w) && (100..=99_999).contains(h));
            let Some((w, h)) = size else {
                return Err("resolution must look like 1920x1080.".to_owned());
            };
            lines.push(format!("desktopwidth:i:{w}"));
            lines.push(format!("desktopheight:i:{h}"));
        }
        if is_true(conn.get("dynamic_resolution")) {
            lines.push("dynamic resolution:i:1".to_owned());
        }
    }
    if let Some(Value::Bool(b)) = conn.get("clipboard") {
        lines.push(format!("redirectclipboard:i:{}", i32::from(*b)));
    }
    let mut drives: Vec<String> = Vec::new();
    if is_true(conn.get("home_drive"))
        && let Some(d) = home_drive
    {
        drives.push(format!("{}\\", d.trim_end_matches('\\')));
    }
    for path in shared_folders(conn.get("shared_folders")) {
        let b = path.as_bytes();
        if b.len() < 2 || !b[0].is_ascii_alphabetic() || b[1] != b':' {
            return Err(format!(
                "shared folder {}: on Windows give a folder on a drive (mstsc shares whole drives).",
                quote_repr(&path)
            ));
        }
        let drive = format!("{}:\\", path[..1].to_ascii_uppercase());
        if !drives.contains(&drive) {
            drives.push(drive);
        }
    }
    if !drives.is_empty() {
        lines.push(format!("drivestoredirect:s:{};", drives.join(";")));
    }
    lines.push(format!(
        "audiomode:i:{}",
        if is_true(conn.get("sound")) { 0 } else { 2 }
    ));
    if is_true(conn.get("microphone")) {
        lines.push("audiocapturemode:i:1".to_owned());
    }
    if is_true(conn.get("multimon")) {
        lines.push("use multimon:i:1".to_owned());
    }
    let gw = text_stripped(conn.get("gateway"));
    if !gw.is_empty() {
        clean("gateway", &gw)?;
        lines.push(format!("gatewayhostname:s:{gw}"));
        lines.push("gatewayusagemethod:i:1".to_owned());
        lines.push("gatewayprofileusagemethod:i:1".to_owned());
    }
    let mut out = lines.join("\r\n");
    out.push_str("\r\n");
    Ok(out)
}

/// Extra environment for spawning a command from [`build_rdp_command`]: the pipeline reads the
/// password from `$TERMIGATOR_RDP_PW`. A sealed token is refused (reveal the connection first).
pub fn rdp_password_env(conn: &Value) -> Result<Vec<(String, Secret)>, VaultError> {
    let password = text(conn.get("password"));
    if password.starts_with(SEALED_PREFIX) {
        return Err(VaultError::Vault(
            "Password is still encrypted — unlock the vault first.".to_owned(),
        ));
    }
    Ok(if password.is_empty() {
        Vec::new()
    } else {
        vec![(
            "TERMIGATOR_RDP_PW".to_owned(),
            Secret::new(password.into_bytes()),
        )]
    })
}

/// Clipboard variant of an RDP command line: a masked `TERMIGATOR_RDP_PW=` prefix.
pub fn clipboard_rdp_command(cmd: &str, conn: &Value) -> String {
    if text(conn.get("password")).is_empty() {
        cmd.to_owned()
    } else {
        format!("TERMIGATOR_RDP_PW='****' {cmd}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn conn(yaml: &str) -> Value {
        crate::config::doc::load(yaml).expect("yaml")
    }

    #[test]
    fn rdp_file_for_mstsc() {
        let c = conn(
            "host: win.lan\nport: 3390\nuser: ann\ndomain: CORP\nresolution: 1600x900\nclipboard: true\n\
             home_drive: true\nshared_folders: ['D:\\\\data', 'd:\\\\more']\nsound: true\nmultimon: true\n\
             gateway: gw.corp\n",
        );
        let f = rdp_file(&c, Some("01ABCD"), Some("C:")).expect("file");
        for line in [
            "full address:s:win.lan:3390",
            "username:s:CORP\\ann",
            "password 51:b:01ABCD",
            "prompt for credentials:i:0",
            "screen mode id:i:1",
            "desktopwidth:i:1600",
            "desktopheight:i:900",
            "redirectclipboard:i:1",
            "drivestoredirect:s:C:\\;D:\\;",
            "audiomode:i:0",
            "use multimon:i:1",
            "gatewayhostname:s:gw.corp",
        ] {
            assert!(f.contains(&format!("{line}\r\n")), "{line} missing in\n{f}");
        }
    }

    #[test]
    fn rdp_file_without_password_lets_mstsc_ask() {
        let f = rdp_file(&conn("host: h\nfullscreen: true\n"), None, None).expect("file");
        assert!(f.starts_with("full address:s:h\r\n"));
        assert!(!f.contains("password") && !f.contains("prompt for credentials"));
        assert!(f.contains("screen mode id:i:2") && f.contains("audiomode:i:2"));
    }

    #[test]
    fn rdp_file_refuses_what_mstsc_cannot_do() {
        let e = |y: &str| rdp_file(&conn(y), None, None).expect_err(y);
        assert!(e("host: h\nssh_gateway: jump\n").contains("SSH gateway"));
        assert!(e("host: h\nextra_rdp_args: /cert:ignore\n").contains("FreeRDP"));
        assert!(e("host: h\nshared_folders: [/home/x]\n").contains("drive"));
        assert!(e("host: h\nresolution: big\n").contains("1920x1080"));
        assert!(rdp_file(&conn("host: h\n"), Some("AA"), None).is_err()); // a password needs a user
        assert!(e("host: ''\n").contains("Host"));
    }

    #[test]
    fn splitlines_like_golden() {
        assert_eq!(splitlines("a\r\nb\n\nc"), ["a", "b", "", "c"]);
        assert_eq!(splitlines("a\u{2028}b\n"), ["a", "b"]);
    }

    #[test]
    fn drive_name_sanitised() {
        assert_eq!(non_word_runs("my dir!!x"), "my_dir_x");
    }
}
