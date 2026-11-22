//! Local shells for a new terminal tab: what the "+" / shell button menu lists and which one the
//! default "Local shell" runs. Linux lists /etc/shells; Windows detects Command Prompt, Windows
//! PowerShell, PowerShell 7 and the installed WSL distributions (by default: WSL, else cmd.exe).

use super::ssh::{available_shells, default_interactive_shell};
use crate::config::doc::Value;
use crate::shell::{join, quote};

/// One entry of the shell menu. `inner` is the session command line (what a tab runs); `None`
/// when the shell is not installed here, so the menu shows it greyed out.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LocalShell {
    /// Stable name stored in `launch.shell` (Windows: `cmd`, `powershell`, `pwsh`, `wsl:<distro>`;
    /// Linux: the shell's path).
    pub id: String,
    pub label: String,
    pub inner: Option<String>,
}

impl LocalShell {
    pub fn available(&self) -> bool {
        self.inner.is_some()
    }
}

/// `launch.shell` values that mean "pick for me" on Windows.
fn is_auto(name: &str) -> bool {
    name.is_empty() || name.eq_ignore_ascii_case("auto")
}

/// The Windows shells in menu order. `which` finds an executable on PATH (`cmd` -> `...\cmd.exe`),
/// `wsl_distros` are the installed WSL distributions, the default one first.
pub fn windows_shells(
    which: &dyn Fn(&str) -> Option<String>,
    comspec: Option<&str>,
    wsl_distros: &[String],
) -> Vec<LocalShell> {
    let exe = |id: &str, label: &str, found: Option<String>| LocalShell {
        id: id.to_owned(),
        label: label.to_owned(),
        inner: found.map(|e| join(&[e])),
    };
    let cmd = which("cmd").or_else(|| comspec.filter(|c| !c.is_empty()).map(str::to_owned));
    let mut out = vec![
        exe("cmd", "Command Prompt (cmd.exe)", cmd),
        powershell("powershell", "Windows PowerShell", which("powershell")),
        powershell("pwsh", "PowerShell 7", which("pwsh")),
    ];
    let wsl = which("wsl");
    if wsl_distros.is_empty() {
        out.push(LocalShell {
            id: "wsl".to_owned(),
            label: "WSL".to_owned(),
            inner: None,
        });
    }
    for d in wsl_distros {
        out.push(LocalShell {
            id: format!("wsl:{d}"),
            label: format!("WSL — {d}"),
            // The distribution's own login shell, through the OSC 7 bootstrap (bash, zsh, fish,
            // sh): the Files panel follows its folder like an SSH session's.
            inner: wsl.as_ref().map(|w| {
                let mut argv: Vec<String> = [w.as_str(), "-d", d.as_str(), "--cd", "~", "-e"]
                    .map(str::to_owned)
                    .to_vec();
                argv.extend(crate::text::osc7::tracking_local_argv());
                join(&argv)
            }),
        });
    }
    out
}

/// Wraps the user's own `prompt` so every prompt reports the folder with OSC 7 (the Files panel
/// follows it, like cmd's PROMPT in terminal_qobj.rs). Only for file system locations.
const PS_PROMPT_HOOK: &str = "$global:__tgPrompt = $function:prompt\n\
function global:prompt { $e = [char]27; $o = ''; \
if ($PWD.Provider.Name -eq 'FileSystem') { $o = \"$e]7;file://localhost/$($PWD.ProviderPath)$e\\\" }; \
$o + (& $global:__tgPrompt) }";

/// `<powershell> -NoLogo -NoExit -EncodedCommand <hook>`: base64 of UTF-16LE, so no quoting of
/// the script survives or breaks on the way through the Windows command line.
fn powershell(id: &str, label: &str, found: Option<String>) -> LocalShell {
    use base64::Engine as _;
    let utf16: Vec<u8> = PS_PROMPT_HOOK
        .encode_utf16()
        .flat_map(u16::to_le_bytes)
        .collect();
    let encoded = base64::engine::general_purpose::STANDARD.encode(utf16);
    LocalShell {
        id: id.to_owned(),
        label: label.to_owned(),
        inner: found.map(|e| {
            join(&[
                e.as_str(),
                "-NoLogo",
                "-NoExit",
                "-EncodedCommand",
                &encoded,
            ])
        }),
    }
}

/// The shell for `launch.shell = name` among `shells`: an id (`wsl` = the default distribution)
/// that is installed; "" / `auto` = the first WSL distribution, else cmd.exe, else anything there.
pub fn pick_windows<'a>(shells: &'a [LocalShell], name: &str) -> Option<&'a LocalShell> {
    let usable = |s: &&LocalShell| s.available();
    if is_auto(name) {
        return shells
            .iter()
            .filter(usable)
            .find(|s| s.id.starts_with("wsl:"))
            .or_else(|| shells.iter().filter(usable).find(|s| s.id == "cmd"))
            .or_else(|| shells.iter().find(usable));
    }
    let name = name.to_ascii_lowercase();
    shells.iter().filter(usable).find(|s| {
        let id = s.id.to_ascii_lowercase();
        id == name || (name == "wsl" && id.starts_with("wsl:"))
    })
}

/// The command line of the default local shell on Windows: `launch.shell_executable` (absolute),
/// then `launch.shell` as a shell id or a program on PATH, else automatic (WSL, else cmd.exe).
pub fn windows_default_inner(
    cfg: &Value,
    shells: &[LocalShell],
    which: &dyn Fn(&str) -> Option<String>,
) -> String {
    let launch = |k: &str| {
        cfg.get("launch")
            .and_then(|l| l.get(k))
            .and_then(|v| match v {
                Value::Str(s) => Some(s.trim().to_owned()),
                _ => None,
            })
            .unwrap_or_default()
    };
    let se = launch("shell_executable");
    if !se.is_empty()
        && std::path::Path::new(&se).is_absolute()
        && crate::platform::can_exec(std::path::Path::new(&se))
    {
        return join(&[se]);
    }
    let name = launch("shell");
    if let Some(s) = pick_windows(shells, &name) {
        return s.inner.clone().unwrap_or_default();
    }
    if !is_auto(&name)
        && let Some(exe) = which(&name)
    {
        return join(&[exe]);
    }
    pick_windows(shells, "")
        .and_then(|s| s.inner.clone())
        .unwrap_or_else(|| "cmd.exe".to_owned())
}

/// The distribution a WSL command line runs (`wsl.exe ... -d <name> ...`), if it is one.
fn wsl_distro(inner: &str) -> Option<String> {
    let argv = crate::shell::split(inner).ok()?;
    let exe = argv
        .first()?
        .rsplit(['\\', '/'])
        .next()?
        .to_ascii_lowercase();
    if exe != "wsl.exe" && exe != "wsl" {
        return None;
    }
    let at = argv
        .iter()
        .position(|a| a == "-d" || a == "--distribution")?;
    argv.get(at + 1).cloned()
}

/// The Windows folder of a directory a local shell reported with OSC 7 (`reported` is the path
/// part of the URL): cmd and PowerShell send `/C:\dir`; a WSL shell (`inner` its command line)
/// sends a Linux path, which is `\\wsl.localhost\<distro>\...`, or `X:\...` under `/mnt/x`.
/// "" when there is no such folder.
pub fn windows_local_dir(inner: &str, reported: &str) -> String {
    let b = reported.as_bytes();
    if b.len() >= 3 && b[0] == b'/' && b[1].is_ascii_alphabetic() && b[2] == b':' {
        return reported[1..].replace('/', "\\");
    }
    let Some(distro) = wsl_distro(inner) else {
        return String::new();
    };
    if !reported.starts_with('/') {
        return String::new();
    }
    let mut parts = reported.split('/').skip(1);
    if parts.next() == Some("mnt")
        && let Some(drive) = parts.next()
        && drive.len() == 1
        && drive.as_bytes()[0].is_ascii_alphabetic()
    {
        let rest: Vec<&str> = parts.filter(|p| !p.is_empty()).collect();
        return format!("{}:\\{}", drive.to_ascii_uppercase(), rest.join("\\"));
    }
    format!(
        "\\\\wsl.localhost\\{distro}{}",
        reported.trim_end_matches('/').replace('/', "\\")
    )
}

/// Linux: the login shells of /etc/shells, each run as `<path> -l`.
pub fn unix_shells(paths: &[String]) -> Vec<LocalShell> {
    paths
        .iter()
        .map(|p| LocalShell {
            id: p.clone(),
            label: p.rsplit('/').next().unwrap_or_default().to_owned(),
            inner: Some(format!("{} -l", quote(p))),
        })
        .collect()
}

/// The shells of this machine for the menu.
pub fn detect(which: &dyn Fn(&str) -> Option<String>) -> Vec<LocalShell> {
    if cfg!(windows) {
        let comspec = std::env::var("COMSPEC").ok();
        windows_shells(
            which,
            comspec.as_deref(),
            &crate::platform::wsl_distributions(),
        )
    } else {
        unix_shells(&available_shells("/etc/shells"))
    }
}

/// The default local shell's command line on this machine (Windows: [`windows_default_inner`],
/// elsewhere [`default_interactive_shell`] with `$SHELL` = `shell_env`).
pub fn default_inner(
    cfg: &Value,
    shell_env: &str,
    which: &dyn Fn(&str) -> Option<String>,
) -> String {
    if cfg!(windows) {
        windows_default_inner(cfg, &detect(which), which)
    } else {
        default_interactive_shell(cfg, shell_env, which)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn which_of(found: &'static [&'static str]) -> impl Fn(&str) -> Option<String> {
        move |n: &str| {
            found
                .contains(&n)
                .then(|| format!("C:\\Windows\\System32\\{n}.exe"))
        }
    }

    fn ids(shells: &[LocalShell]) -> Vec<(&str, bool)> {
        shells
            .iter()
            .map(|s| (s.id.as_str(), s.available()))
            .collect()
    }

    #[test]
    fn windows_lists_all_and_greys_missing() {
        let w = which_of(&["cmd", "powershell"]);
        let s = windows_shells(&w, None, &[]);
        assert_eq!(
            ids(&s),
            [
                ("cmd", true),
                ("powershell", true),
                ("pwsh", false),
                ("wsl", false)
            ]
        );
        assert_eq!(
            s[0].inner.as_deref(),
            Some("'C:\\Windows\\System32\\cmd.exe'")
        );
    }

    #[test]
    fn wsl_distros_each_get_an_entry() {
        let w = which_of(&["cmd", "wsl"]);
        let s = windows_shells(&w, None, &["Ubuntu".into(), "Debian".into()]);
        assert_eq!(ids(&s)[3..], [("wsl:Ubuntu", true), ("wsl:Debian", true)]);
        assert!(
            s[3].inner
                .as_deref()
                .expect("wsl")
                .starts_with("'C:\\Windows\\System32\\wsl.exe' -d Ubuntu --cd '~' -e sh -c ")
        );
        // Registered distributions without wsl.exe cannot start.
        let s = windows_shells(&which_of(&["cmd"]), None, &["Ubuntu".into()]);
        assert!(!s[3].available());
    }

    #[test]
    fn powershell_reports_its_folder() {
        use base64::Engine as _;
        let s = windows_shells(&which_of(&["powershell"]), None, &[]);
        let argv = crate::shell::split(s[1].inner.as_deref().expect("found")).expect("split");
        assert_eq!(argv[1..4], ["-NoLogo", "-NoExit", "-EncodedCommand"]);
        let raw = base64::engine::general_purpose::STANDARD
            .decode(&argv[4])
            .expect("base64");
        let units: Vec<u16> = raw
            .chunks(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .collect();
        let script = String::from_utf16(&units).expect("utf-16");
        assert!(script.contains("\"$e]7;file://localhost/$($PWD.ProviderPath)$e\\\""));
        assert!(script.contains("(& $global:__tgPrompt)"));
    }

    #[test]
    fn reported_folders_on_windows() {
        let s = windows_shells(&which_of(&["wsl"]), None, &["Debian".into()]);
        let wsl = s[3].inner.clone().expect("wsl");
        assert_eq!(
            windows_local_dir(&wsl, "/home/tg/src"),
            "\\\\wsl.localhost\\Debian\\home\\tg\\src"
        );
        assert_eq!(windows_local_dir(&wsl, "/"), "\\\\wsl.localhost\\Debian");
        assert_eq!(windows_local_dir(&wsl, "/mnt/c/Users/x"), "C:\\Users\\x");
        assert_eq!(windows_local_dir(&wsl, "/mnt/d"), "D:\\");
        // cmd / PowerShell report the drive path; any shell may.
        assert_eq!(
            windows_local_dir("'C:\\cmd.exe'", "/C:\\Windows"),
            "C:\\Windows"
        );
        assert_eq!(windows_local_dir("'C:\\cmd.exe'", "/home/x"), "");
        assert_eq!(windows_local_dir("", ""), "");
    }

    #[test]
    fn wsl_runs_the_folder_bootstrap() {
        let s = windows_shells(&which_of(&["wsl"]), None, &["Ubuntu".into()]);
        let argv = crate::shell::split(s[3].inner.as_deref().expect("wsl")).expect("split");
        assert_eq!(argv[1..8], ["-d", "Ubuntu", "--cd", "~", "-e", "sh", "-c"]);
        assert!(argv[8].starts_with("ERN_NO_MOTD=1; eval "));
    }

    #[test]
    fn comspec_when_cmd_is_not_on_path() {
        let s = windows_shells(&which_of(&[]), Some("C:\\W\\cmd.exe"), &[]);
        assert_eq!(s[0].inner.as_deref(), Some("'C:\\W\\cmd.exe'"));
    }

    #[test]
    fn auto_prefers_wsl_then_cmd() {
        let w = which_of(&["cmd", "powershell", "wsl"]);
        let with_wsl = windows_shells(&w, None, &["Ubuntu".into()]);
        assert_eq!(
            pick_windows(&with_wsl, "").map(|s| s.id.as_str()),
            Some("wsl:Ubuntu")
        );
        assert_eq!(
            pick_windows(&with_wsl, "auto").map(|s| s.id.as_str()),
            Some("wsl:Ubuntu")
        );
        let no_wsl = windows_shells(&w, None, &[]);
        assert_eq!(
            pick_windows(&no_wsl, "").map(|s| s.id.as_str()),
            Some("cmd")
        );
        let only_ps = windows_shells(&which_of(&["powershell"]), None, &[]);
        assert_eq!(
            pick_windows(&only_ps, "").map(|s| s.id.as_str()),
            Some("powershell")
        );
    }

    #[test]
    fn configured_shell_wins_when_installed() {
        let w = which_of(&["cmd", "powershell", "wsl"]);
        let s = windows_shells(&w, None, &["Ubuntu".into()]);
        assert_eq!(
            pick_windows(&s, "PowerShell").map(|s| s.id.as_str()),
            Some("powershell")
        );
        assert_eq!(
            pick_windows(&s, "wsl").map(|s| s.id.as_str()),
            Some("wsl:Ubuntu")
        );
        assert_eq!(pick_windows(&s, "pwsh"), None);
    }

    #[test]
    fn default_inner_falls_back_to_auto_or_path() {
        let w = which_of(&["cmd", "nu"]);
        let s = windows_shells(&w, None, &[]);
        let cfg = |shell: &str| Value::from_json(&serde_json::json!({"launch": {"shell": shell}}));
        assert_eq!(
            windows_default_inner(&cfg(""), &s, &w),
            "'C:\\Windows\\System32\\cmd.exe'"
        );
        // Not installed: automatic.
        assert_eq!(
            windows_default_inner(&cfg("pwsh"), &s, &w),
            "'C:\\Windows\\System32\\cmd.exe'"
        );
        // Any other program on PATH.
        assert_eq!(
            windows_default_inner(&cfg("nu"), &s, &w),
            "'C:\\Windows\\System32\\nu.exe'"
        );
    }

    #[test]
    fn unix_entries_match_the_old_menu() {
        let s = unix_shells(&["/bin/bash".into(), "/usr/bin/fish".into()]);
        assert_eq!(s[1].label, "fish");
        assert_eq!(s[1].inner.as_deref(), Some("/usr/bin/fish -l"));
        assert!(s.iter().all(LocalShell::available));
    }
}
