//! Where Windows programs keep their sessions, for File → Import without picking a file:
//! MobaXterm (installed edition: the registry; portable / older: `MobaXterm.ini`), PuTTY (the
//! registry) and SuperPuTTY (`Documents\SuperPuTTY\Sessions.xml`). Registry keys become the text
//! the existing parsers read (an INI for MobaXterm, a regedit export for PuTTY), so the parsing
//! stays in one place. The builders are pure; the `local_*` functions read this machine.

use std::path::{Path, PathBuf};

use crate::platform::{RegData, RegKeyValues};

/// MobaXterm's settings, installed edition (`[Bookmarks*]` are subkeys there).
pub const MOBA_REG: &str = r"Software\Mobatek\MobaXterm";
/// PuTTY's saved sessions.
pub const PUTTY_REG: &str = r"Software\SimonTatham\PuTTY\Sessions";
/// Shell folders of the user (the Documents folder may be moved, e.g. to OneDrive).
const SHELL_FOLDERS_REG: &str =
    r"Software\Microsoft\Windows\CurrentVersion\Explorer\User Shell Folders";

/// The folders the sessions files live in.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WinDirs {
    pub profile: PathBuf,
    pub appdata: PathBuf,
    pub documents: PathBuf,
    pub desktop: PathBuf,
}

/// The subkeys of MobaXterm's key as INI text: `[name]` and `key=value` (string values).
pub fn moba_ini_from_registry(keys: &[RegKeyValues]) -> String {
    let mut out = String::new();
    for k in keys.iter().filter(|k| !k.name.is_empty()) {
        out.push_str(&format!("[{}]\r\n", k.name));
        for (name, data) in &k.values {
            if let RegData::Str(v) = data {
                out.push_str(&format!("{name}={v}\r\n"));
            }
        }
    }
    out
}

/// PuTTY's session subkeys as a regedit export (what `parse_putty_reg` reads).
pub fn putty_reg_from_registry(keys: &[RegKeyValues]) -> String {
    let mut out = String::from("Windows Registry Editor Version 5.00\r\n");
    for k in keys.iter().filter(|k| !k.name.is_empty()) {
        out.push_str(&format!(
            "\r\n[HKEY_CURRENT_USER\\{PUTTY_REG}\\{}]\r\n",
            k.name
        ));
        for (name, data) in &k.values {
            match data {
                RegData::Str(v) => {
                    out.push_str(&format!("\"{name}\"=\"{}\"\r\n", v.replace('\\', "\\\\")))
                }
                RegData::Dword(d) => out.push_str(&format!("\"{name}\"=dword:{d:08x}\r\n")),
            }
        }
    }
    out
}

/// True when MobaXterm text has a bookmarks section (sessions to import).
pub fn has_moba_bookmarks(text: &str) -> bool {
    text.lines().any(|l| {
        let l = l.trim().to_ascii_lowercase();
        l.starts_with("[bookmarks")
    })
}

/// `%NAME%` expanded with `lookup` (unknown names stay as written).
pub fn expand_env(s: &str, lookup: &dyn Fn(&str) -> Option<String>) -> String {
    let mut out = String::new();
    let mut rest = s;
    while let Some(start) = rest.find('%') {
        let Some(len) = rest[start + 1..].find('%') else {
            break;
        };
        let name = &rest[start + 1..start + 1 + len];
        out.push_str(&rest[..start]);
        match lookup(name).filter(|_| !name.is_empty()) {
            Some(v) => out.push_str(&v),
            None => out.push_str(&rest[start..start + len + 2]),
        }
        rest = &rest[start + len + 2..];
    }
    out.push_str(rest);
    out
}

/// The Documents folder: the shell folder `Personal` (expanded), else `<profile>\Documents`.
pub fn documents_dir(
    shell_folders: &[RegKeyValues],
    profile: &Path,
    lookup: &dyn Fn(&str) -> Option<String>,
) -> PathBuf {
    shell_folder(shell_folders, "Personal", "Documents", profile, lookup)
}

/// The Desktop folder: the shell folder `Desktop` (expanded; OneDrive moves it), else
/// `<profile>\Desktop`.
pub fn desktop_dir(
    shell_folders: &[RegKeyValues],
    profile: &Path,
    lookup: &dyn Fn(&str) -> Option<String>,
) -> PathBuf {
    shell_folder(shell_folders, "Desktop", "Desktop", profile, lookup)
}

fn shell_folder(
    shell_folders: &[RegKeyValues],
    value: &str,
    default: &str,
    profile: &Path,
    lookup: &dyn Fn(&str) -> Option<String>,
) -> PathBuf {
    shell_folders
        .iter()
        .filter(|k| k.name.is_empty())
        .flat_map(|k| &k.values)
        .find_map(|(n, d)| match d {
            RegData::Str(v) if n.eq_ignore_ascii_case(value) && !v.is_empty() => {
                Some(PathBuf::from(expand_env(v, lookup)))
            }
            _ => None,
        })
        .unwrap_or_else(|| profile.join(default))
}

/// `MobaXterm.ini` candidates, in the order MobaXterm editions use them.
pub fn moba_ini_candidates(d: &WinDirs) -> Vec<PathBuf> {
    vec![
        d.appdata.join("MobaXterm").join("MobaXterm.ini"),
        d.documents.join("MobaXterm").join("MobaXterm.ini"),
    ]
}

/// SuperPuTTY's `Sessions.xml` candidates (its default settings folder is Documents\SuperPuTTY).
pub fn superputty_candidates(d: &WinDirs) -> Vec<PathBuf> {
    vec![
        d.documents.join("SuperPuTTY").join("Sessions.xml"),
        d.appdata.join("SuperPuTTY").join("Sessions.xml"),
    ]
}

/// A MobaXterm key path (`_ProfileDir_\.ssh\id`, `_MyDocuments_\...`, `_CurrentDrive_\...`,
/// `_AppDataDir_\...`) as a Windows path; other paths keep their text with backslashes.
pub fn moba_identity_to_windows(path: Option<&str>, d: &WinDirs) -> Option<String> {
    let p = path
        .map(str::trim)
        .filter(|p| !p.is_empty())?
        .replace('/', "\\");
    let drive = d
        .profile
        .to_string_lossy()
        .get(..2)
        .filter(|s| s.ends_with(':'))
        .unwrap_or("C:")
        .to_owned();
    for (mark, base) in [
        ("_ProfileDir_", d.profile.to_string_lossy().into_owned()),
        ("_MyDocuments_", d.documents.to_string_lossy().into_owned()),
        ("_DesktopDir_", d.desktop.to_string_lossy().into_owned()),
        ("_AppDataDir_", d.appdata.to_string_lossy().into_owned()),
        ("_CurrentDrive_", drive),
    ] {
        if let Some(tail) = p.strip_prefix(mark) {
            // `_CurrentDrive_:\k` would otherwise become `C::\k`.
            let tail = tail.strip_prefix(':').unwrap_or(tail);
            return Some(format!("{}{}", base.trim_end_matches('\\'), tail));
        }
    }
    Some(p)
}

/// This user's folders: `%USERPROFILE%`, `%APPDATA%` and Documents (from the registry).
/// `None` outside Windows or without a profile.
pub fn win_dirs() -> Option<WinDirs> {
    if !cfg!(windows) {
        return None;
    }
    let env = |n: &str| std::env::var(n).ok().filter(|v| !v.is_empty());
    let profile = PathBuf::from(env("USERPROFILE")?);
    let appdata = env("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| profile.join("AppData").join("Roaming"));
    let folders = crate::platform::registry_keys(SHELL_FOLDERS_REG);
    let documents = documents_dir(&folders, &profile, &env);
    let desktop = desktop_dir(&folders, &profile, &env);
    Some(WinDirs {
        profile,
        appdata,
        documents,
        desktop,
    })
}

/// This machine's MobaXterm sessions: `(where, INI text)` from the registry, else the first
/// `MobaXterm.ini` with bookmarks.
pub fn local_moba() -> Option<(String, String)> {
    let dirs = win_dirs()?;
    let reg = moba_ini_from_registry(&crate::platform::registry_keys(MOBA_REG));
    if has_moba_bookmarks(&reg) {
        return Some((format!("HKEY_CURRENT_USER\\{MOBA_REG}"), reg));
    }
    moba_ini_candidates(&dirs).into_iter().find_map(|p| {
        let raw = std::fs::read(&p).ok()?;
        let text = super::moba::decode_moba_bytes(&raw).ok()?;
        has_moba_bookmarks(&text).then(|| (p.to_string_lossy().into_owned(), text))
    })
}

/// This machine's PuTTY sessions as a regedit export ("" outside Windows or without sessions).
pub fn local_putty_reg() -> String {
    let keys = crate::platform::registry_keys(PUTTY_REG);
    if keys.iter().all(|k| k.name.is_empty()) {
        return String::new();
    }
    putty_reg_from_registry(&keys)
}

/// SuperPuTTY's `Sessions.xml` on this machine.
pub fn local_superputty() -> Option<PathBuf> {
    superputty_candidates(&win_dirs()?)
        .into_iter()
        .find(|p| p.is_file())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(name: &str, values: &[(&str, RegData)]) -> RegKeyValues {
        RegKeyValues {
            name: name.to_owned(),
            values: values
                .iter()
                .map(|(n, d)| ((*n).to_owned(), d.clone()))
                .collect(),
        }
    }

    fn dirs() -> WinDirs {
        WinDirs {
            profile: PathBuf::from(r"C:\Users\ann"),
            appdata: PathBuf::from(r"C:\Users\ann\AppData\Roaming"),
            documents: PathBuf::from(r"D:\OneDrive\Docs"),
            desktop: PathBuf::from(r"D:\OneDrive\Desktop"),
        }
    }

    #[test]
    fn moba_registry_reads_like_its_ini() {
        let keys = [
            key("", &[("SessionP", RegData::Str("x".into()))]),
            key(
                "Bookmarks",
                &[
                    ("SubRep", RegData::Str(String::new())),
                    ("ImgNum", RegData::Str("42".into())),
                    (
                        "web",
                        RegData::Str("#109#0%10.0.0.5%22%root%%-1%-1%%%".into()),
                    ),
                ],
            ),
        ];
        let ini = moba_ini_from_registry(&keys);
        assert!(has_moba_bookmarks(&ini));
        assert!(!ini.contains("SessionP"));
        let (rows, _) = super::super::moba::parse_moba_export(&ini, "me");
        assert_eq!(rows.len(), 1);
        assert_eq!(
            rows[0].get("host"),
            Some(&crate::config::doc::Value::Str("10.0.0.5".into()))
        );
    }

    #[test]
    fn putty_registry_reads_like_a_reg_export() {
        let keys = [
            key("", &[]),
            key(
                "My%20Box",
                &[
                    ("HostName", RegData::Str("box.lan".into())),
                    ("Protocol", RegData::Str("ssh".into())),
                    ("PortNumber", RegData::Dword(2222)),
                    ("UserName", RegData::Str("ann".into())),
                    ("PublicKeyFile", RegData::Str(r"C:\keys\id_ed25519".into())),
                ],
            ),
            key(
                "Telnet",
                &[
                    ("HostName", RegData::Str("t".into())),
                    ("Protocol", RegData::Str("telnet".into())),
                ],
            ),
        ];
        let text = putty_reg_from_registry(&keys);
        assert!(text.contains("\"PortNumber\"=dword:000008ae"));
        let rows = super::super::putty::parse_putty_reg(&text, "me");
        assert_eq!(rows.len(), 1);
        let get = |k: &str| rows[0].get(k).cloned();
        assert_eq!(
            get("label"),
            Some(crate::config::doc::Value::Str("My Box".into()))
        );
        assert_eq!(get("port"), Some(crate::config::doc::Value::Int(2222)));
    }

    #[test]
    fn documents_follow_the_shell_folder() {
        let lookup = |n: &str| (n == "USERPROFILE").then(|| r"C:\Users\ann".to_owned());
        let moved = [key(
            "",
            &[(
                "Personal",
                RegData::Str(r"%USERPROFILE%\OneDrive\Documents".into()),
            )],
        )];
        assert_eq!(
            documents_dir(&moved, Path::new(r"C:\Users\ann"), &lookup),
            PathBuf::from(r"C:\Users\ann\OneDrive\Documents")
        );
        assert_eq!(
            documents_dir(&[], Path::new(r"C:\Users\ann"), &lookup),
            Path::new(r"C:\Users\ann").join("Documents")
        );
        assert_eq!(expand_env("%NOPE%\\x", &lookup), "%NOPE%\\x");
        let desktop = [key(
            "",
            &[(
                "Desktop",
                RegData::Str(r"%USERPROFILE%\OneDrive\Desktop".into()),
            )],
        )];
        assert_eq!(
            desktop_dir(&desktop, Path::new(r"C:\Users\ann"), &lookup),
            PathBuf::from(r"C:\Users\ann\OneDrive\Desktop")
        );
        assert_eq!(
            desktop_dir(&[], Path::new(r"C:\Users\ann"), &lookup),
            Path::new(r"C:\Users\ann").join("Desktop")
        );
    }

    #[test]
    fn moba_key_paths_on_windows() {
        let d = dirs();
        assert_eq!(
            moba_identity_to_windows(Some(r"_ProfileDir_\.ssh\id_rsa"), &d).as_deref(),
            Some(r"C:\Users\ann\.ssh\id_rsa")
        );
        assert_eq!(
            moba_identity_to_windows(Some("_MyDocuments_/MobaXterm/home/.ssh/k"), &d).as_deref(),
            Some(r"D:\OneDrive\Docs\MobaXterm\home\.ssh\k")
        );
        assert_eq!(
            moba_identity_to_windows(Some(r"_CurrentDrive_\keys\a"), &d).as_deref(),
            Some(r"C:\keys\a")
        );
        assert_eq!(
            moba_identity_to_windows(Some(r"E:\k\id"), &d).as_deref(),
            Some(r"E:\k\id")
        );
        assert_eq!(
            moba_identity_to_windows(Some(r"_DesktopDir_\mykey.ppk"), &d).as_deref(),
            Some(r"D:\OneDrive\Desktop\mykey.ppk")
        );
        assert_eq!(
            moba_identity_to_windows(Some(r"_AppDataDir_\k"), &d).as_deref(),
            Some(r"C:\Users\ann\AppData\Roaming\k")
        );
        // A colon after the token must not double up.
        assert_eq!(
            moba_identity_to_windows(Some(r"_CurrentDrive_:\keys\a"), &d).as_deref(),
            Some(r"C:\keys\a")
        );
        assert_eq!(moba_identity_to_windows(Some(" "), &d), None);
    }

    #[test]
    fn candidates() {
        let d = dirs();
        assert_eq!(
            superputty_candidates(&d)[0],
            PathBuf::from(r"D:\OneDrive\Docs")
                .join("SuperPuTTY")
                .join("Sessions.xml")
        );
        assert_eq!(
            moba_ini_candidates(&d)[0],
            PathBuf::from(r"C:\Users\ann\AppData\Roaming")
                .join("MobaXterm")
                .join("MobaXterm.ini")
        );
    }
}
