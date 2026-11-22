//! Windows implementation of the platform layer. Parts not needed by the app yet are stubs.

use std::path::{Path, PathBuf};

pub fn can_exec(p: &Path) -> bool {
    p.is_file()
}

pub fn can_read(p: &Path) -> bool {
    std::fs::File::open(p).is_ok()
}

pub fn can_read_write(p: &Path) -> bool {
    std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(p)
        .is_ok()
}

pub fn is_executable(p: &Path) -> bool {
    p.is_file()
}

/// First `dir\name` on `PATH` that exists, trying the `PATHEXT` extensions (`.EXE` ...).
pub fn find_in_path(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    let pathext = std::env::var("PATHEXT").unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".to_owned());
    let names = super::pathext_names(name, &pathext);
    std::env::split_paths(&path)
        .flat_map(|dir| names.iter().map(move |n| dir.join(n)))
        .find(|p| is_executable(p))
}

pub fn current_user_name() -> Option<String> {
    std::env::var("USERNAME").ok()
}

pub fn current_user_home() -> Option<PathBuf> {
    std::env::var_os("USERPROFILE").map(PathBuf::from)
}

pub fn user_home_by_name(_name: &str) -> Option<PathBuf> {
    None
}

pub fn errno_text(code: i32) -> String {
    std::io::Error::from_raw_os_error(code).to_string()
}

// ACLs instead of modes (icacls, see `restrict_to_user`); names are UTF-8 (lossy for unpaired
// surrogates).
use std::borrow::Cow;
use std::ffi::{OsStr, OsString};
use std::fs::{DirBuilder, File, Metadata, OpenOptions};
use std::io;

/// A new file inherits its folder's ACL; the private folders (config, runtime) grant only the
/// user, and files elsewhere get [`set_private_file`] from their callers.
pub fn private_file(opts: &mut OpenOptions) -> &mut OpenOptions {
    opts
}

/// Creates `dir` (and, with `recursive`, its parents) and makes it private like `chmod 700`.
pub fn private_dir_create(dir: &Path, recursive: bool) -> io::Result<()> {
    DirBuilder::new().recursive(recursive).create(dir)?;
    set_private_dir(dir)
}

/// Only the current user, nothing inherited (the `chmod 600` of Windows).
pub fn set_private_file(p: &Path) -> io::Result<()> {
    restrict_to_user(p, "F")
}

/// Only the current user, inherited by everything created inside (the `chmod 700` of Windows).
pub fn set_private_dir(p: &Path) -> io::Result<()> {
    restrict_to_user(p, "(OI)(CI)F")
}

/// The current user's SID (`S-1-5-21-…`), from `whoami /user`; a SID, not a name, so localized or
/// renamed accounts cannot confuse `icacls`.
fn user_sid() -> io::Result<&'static str> {
    use std::os::windows::process::CommandExt;
    static SID: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();
    SID.get_or_init(|| {
        let out = Command::new("whoami")
            .args(["/user", "/fo", "csv", "/nh"])
            .creation_flags(0x0800_0000) // CREATE_NO_WINDOW
            .output()
            .ok()?;
        let text = String::from_utf8_lossy(&out.stdout);
        let sid = text.trim().rsplit(',').next()?.trim_matches('"').to_owned();
        sid.starts_with("S-1-").then_some(sid)
    })
    .as_deref()
    .ok_or_else(|| io::Error::other("cannot read the user's SID (whoami /user)"))
}

/// Well-known groups an explicit entry may name: SYSTEM, Administrators, Users, Authenticated Users,
/// Everyone, CREATOR OWNER.
const BROAD_SIDS: [&str; 6] = [
    "S-1-5-18",
    "S-1-5-32-544",
    "S-1-5-32-545",
    "S-1-5-11",
    "S-1-1-0",
    "S-1-3-0",
];

// shortcut: one icacls run per call (~50 ms); the callers set up a handful of paths at start, not
// per write. Upgrade to SetNamedSecurityInfoW in a crate that may use `unsafe` if that changes.
fn restrict_to_user(p: &Path, rights: &str) -> io::Result<()> {
    use std::os::windows::process::CommandExt;
    let sid = user_sid()?;
    // `/inheritance:r` drops only inherited entries; an object created where nothing is inherited
    // (an elevated process, some temp dirs) carries explicit ones from the token's default DACL.
    let mut args = vec!["/inheritance:r".to_owned()];
    for broad in BROAD_SIDS {
        args.extend(["/remove:g".to_owned(), format!("*{broad}")]);
    }
    args.extend([
        "/grant:r".to_owned(),
        format!("*{sid}:{rights}"),
        "/q".to_owned(),
    ]);
    let out = Command::new("icacls")
        .arg(p)
        .args(&args)
        .creation_flags(0x0800_0000) // CREATE_NO_WINDOW
        .output()?;
    if out.status.success() {
        Ok(())
    } else {
        Err(io::Error::other(format!(
            "icacls {}: {}",
            p.display(),
            String::from_utf8_lossy(&out.stderr).trim()
        )))
    }
}

/// OpenSSH refuses this private key as "UNPROTECTED PRIVATE KEY FILE" (its ACL grants someone
/// besides the owner, SYSTEM and Administrators). Asked of `ssh-keygen -y`, which runs the same
/// check, so localized group names and inherited ACLs need no parsing here; `false` without it.
pub fn key_too_open(p: &Path) -> bool {
    use std::collections::HashMap;
    use std::sync::Mutex;
    // shortcut: one answer per key for the app's life (a fixed key keeps using the copy, which is
    // harmless); the Files panel builds ssh commands often and ssh-keygen takes ~30 ms.
    static SEEN: std::sync::OnceLock<Mutex<HashMap<PathBuf, bool>>> = std::sync::OnceLock::new();
    let seen = SEEN.get_or_init(Mutex::default);
    if let Some(known) = seen.lock().ok().and_then(|m| m.get(p).copied()) {
        return known;
    }
    let open = keygen_says_too_open(p);
    if let Ok(mut m) = seen.lock() {
        m.insert(p.to_owned(), open);
    }
    open
}

fn keygen_says_too_open(p: &Path) -> bool {
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::time::{Duration, Instant};
    static N: AtomicU32 = AtomicU32::new(0);
    let Some(keygen) = find_in_path("ssh-keygen") else {
        return false;
    };
    // stderr into a file, not a pipe: with "bad permissions" Win32-OpenSSH's ssh-keygen never
    // exits while its stderr is a pipe (OpenSSH 9.x, Windows 11). The text is only the warning.
    let log = std::env::temp_dir().join(format!(
        "termigator-keycheck-{}-{}.txt",
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    ));
    let Ok(err) = File::create(&log) else {
        return false;
    };
    let mut cmd = Command::new(keygen);
    cmd.args(["-y", "-P", "", "-f"])
        .arg(p)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(err);
    background_process(&mut cmd);
    let finished = cmd.spawn().is_ok_and(|mut child| {
        let until = Instant::now() + Duration::from_secs(5);
        loop {
            match child.try_wait() {
                Ok(Some(_)) => break true,
                Ok(None) if Instant::now() < until => std::thread::sleep(Duration::from_millis(10)),
                _ => {
                    let _ = child.kill();
                    let _ = child.wait();
                    break false;
                }
            }
        }
    });
    let text = std::fs::read_to_string(&log).unwrap_or_default();
    let _ = std::fs::remove_file(&log);
    finished && (text.contains("UNPROTECTED PRIVATE KEY FILE") || text.contains("bad permissions"))
}

pub fn copy_permissions(_file: &File, _from: &Metadata) -> io::Result<()> {
    Ok(())
}

pub fn current_uid() -> u32 {
    0
}

pub fn owned_by(_md: &Metadata, _uid: u32) -> bool {
    true
}

pub fn open_to_others(_md: &Metadata) -> bool {
    false
}

pub fn mtime_parts(md: &Metadata) -> (i64, i64) {
    use std::time::UNIX_EPOCH;
    match md
        .modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
    {
        Some(d) => (d.as_secs() as i64, i64::from(d.subsec_nanos())),
        None => (0, 0),
    }
}

pub fn os_str_bytes(s: &OsStr) -> Cow<'_, [u8]> {
    match s.to_string_lossy() {
        Cow::Borrowed(b) => Cow::Borrowed(b.as_bytes()),
        Cow::Owned(o) => Cow::Owned(o.into_bytes()),
    }
}

pub fn os_str_from_bytes(b: &[u8]) -> OsString {
    OsString::from(String::from_utf8_lossy(b).into_owned())
}

pub fn os_string_from_vec(v: Vec<u8>) -> OsString {
    OsString::from(String::from_utf8_lossy(&v).into_owned())
}

pub fn os_string_into_vec(s: OsString) -> Vec<u8> {
    s.to_string_lossy().into_owned().into_bytes()
}

// --- single instance (named pipe `\\.\pipe\termigator-<user>-<layout>`) ---

fn unsupported<T>() -> io::Result<T> {
    Err(io::Error::from(io::ErrorKind::Unsupported))
}

pub use uds_windows::{UnixListener as IpcListener, UnixStream as IpcStream};

/// `md` describes an AF_UNIX socket file, which NTFS stores as a reparse point.
pub fn is_ipc_endpoint(md: &Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    md.file_attributes() & 0x400 != 0 // FILE_ATTRIBUTE_REPARSE_POINT
}

pub fn try_lock_exclusive(_f: &File) -> io::Result<bool> {
    Ok(true)
}

// --- processes (Job Objects) ---

use std::process::{Child, Command, ExitStatus};

pub fn new_process_group(cmd: &mut Command) -> &mut Command {
    use std::os::windows::process::CommandExt;
    cmd.creation_flags(0x0000_0200) // CREATE_NEW_PROCESS_GROUP
}

/// [`new_process_group`] for a process that runs in the background (output captured, no user
/// interaction): CREATE_NO_WINDOW as well, or every console program (ssh.exe, ping, ssh-keygen)
/// started from the GUI app flashes a console window of its own.
pub fn background_process(cmd: &mut Command) -> &mut Command {
    use std::os::windows::process::CommandExt;
    cmd.creation_flags(0x0000_0200 | 0x0800_0000) // CREATE_NEW_PROCESS_GROUP | CREATE_NO_WINDOW
}

// shortcut: `taskkill /T` walks the process tree once, no Job Object yet; fine for ssh/scp.
fn taskkill(pid: u32, force: bool) {
    use std::os::windows::process::CommandExt;
    let mut c = Command::new("taskkill");
    c.args(["/T", "/PID", &pid.to_string()]);
    if force {
        c.arg("/F");
    }
    // CREATE_NO_WINDOW: no console flash from the GUI app.
    let _ = c.creation_flags(0x0800_0000).output();
}

pub fn group_terminate(pid: u32) {
    taskkill(pid, false);
}

pub fn group_kill(pid: u32) {
    taskkill(pid, true);
}

pub fn has_exited(child: &mut Child) -> bool {
    !matches!(child.try_wait(), Ok(None))
}

pub fn exit_signal(_s: &ExitStatus) -> Option<i32> {
    None
}

pub fn exit_status_success() -> ExitStatus {
    use std::os::windows::process::ExitStatusExt;
    ExitStatus::from_raw(0)
}

/// Windows reads and writes the ConPTY through streams the app takes from the master (feed mode),
/// not through a descriptor, so this has nothing to read.
pub struct PtyIo;

impl PtyIo {
    pub fn open(_master: &dyn portable_pty::MasterPty) -> Result<Self, String> {
        Ok(Self)
    }

    pub fn read(&mut self, _buf: &mut [u8]) -> io::Result<usize> {
        unsupported()
    }

    pub fn write(&mut self, _data: &[u8]) -> io::Result<usize> {
        unsupported()
    }

    pub fn write_all(&mut self, _data: &[u8]) -> io::Result<()> {
        unsupported()
    }
}

/// The process on a ConPTY (portable-pty's own child type).
pub struct PtyChild(Box<dyn portable_pty::Child + Send + Sync>);

fn to_std(s: portable_pty::ExitStatus) -> ExitStatus {
    use std::os::windows::process::ExitStatusExt;
    ExitStatus::from_raw(s.exit_code())
}

impl PtyChild {
    pub fn from_portable(
        child: Box<dyn portable_pty::Child + Send + Sync>,
    ) -> Result<Self, String> {
        Ok(Self(child))
    }

    pub fn id(&self) -> u32 {
        self.0.process_id().unwrap_or(0)
    }

    pub fn try_wait(&mut self) -> io::Result<Option<ExitStatus>> {
        Ok(self.0.try_wait()?.map(to_std))
    }

    pub fn kill(&mut self) -> io::Result<()> {
        self.0.kill()
    }

    pub fn wait(&mut self) -> io::Result<ExitStatus> {
        Ok(to_std(self.0.wait()?))
    }
}

pub fn hostname() -> Option<String> {
    std::env::var("COMPUTERNAME").ok().filter(|h| !h.is_empty())
}

/// A directory cannot be opened for fsync on Windows (access denied); NTFS journals the rename.
pub fn sync_dir(_dir: &Path) -> io::Result<()> {
    Ok(())
}

pub fn use_feed() -> bool {
    true
}

/// Installed WSL distributions from the registry (`HKCU\...\Lxss`), the default one first.
/// Empty when WSL is not set up.
pub fn wsl_distributions() -> Vec<String> {
    use winreg::RegKey;
    use winreg::enums::HKEY_CURRENT_USER;
    let Ok(lxss) = RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey(r"Software\Microsoft\Windows\CurrentVersion\Lxss")
    else {
        return Vec::new();
    };
    let default: String = lxss.get_value("DefaultDistribution").unwrap_or_default();
    let mut out: Vec<(bool, String)> = lxss
        .enum_keys()
        .filter_map(Result::ok)
        .filter_map(|guid| {
            let key = lxss.open_subkey(&guid).ok()?;
            let name: String = key.get_value("DistributionName").ok()?;
            // State 1 = installed (others: installing, uninstalling).
            let state: u32 = key.get_value("State").unwrap_or(1);
            (state == 1 && !name.is_empty()).then(|| (guid.eq_ignore_ascii_case(&default), name))
        })
        .collect();
    out.sort_by_key(|(is_default, _)| !*is_default);
    out.into_iter().map(|(_, n)| n).collect()
}

/// `HKEY_CURRENT_USER\<hkcu_path>` and its direct subkeys with their string (REG_SZ,
/// REG_EXPAND_SZ, unexpanded) and DWORD values; other types are skipped. Empty when the key is missing.
pub fn registry_keys(hkcu_path: &str) -> Vec<super::RegKeyValues> {
    use winreg::RegKey;
    use winreg::enums::{HKEY_CURRENT_USER, REG_DWORD, REG_EXPAND_SZ, REG_SZ};
    use winreg::types::FromRegValue;
    let Ok(root) = RegKey::predef(HKEY_CURRENT_USER).open_subkey(hkcu_path) else {
        return Vec::new();
    };
    let values = |key: &RegKey| -> Vec<(String, super::RegData)> {
        key.enum_values()
            .filter_map(Result::ok)
            .filter_map(|(name, v)| {
                let data = match v.vtype {
                    REG_SZ | REG_EXPAND_SZ => super::RegData::Str(String::from_reg_value(&v).ok()?),
                    REG_DWORD => super::RegData::Dword(u32::from_reg_value(&v).ok()?),
                    _ => return None,
                };
                Some((name, data))
            })
            .collect()
    };
    let mut out = vec![super::RegKeyValues {
        name: String::new(),
        values: values(&root),
    }];
    for sub in root.enum_keys().filter_map(Result::ok) {
        if let Ok(key) = root.open_subkey(&sub) {
            out.push(super::RegKeyValues {
                name: sub,
                values: values(&key),
            });
        }
    }
    out
}

#[cfg(test)]
mod acl_tests {
    use super::*;

    fn acl(p: &Path) -> String {
        let out = Command::new("icacls").arg(p).output().expect("icacls");
        String::from_utf8_lossy(&out.stdout).into_owned()
    }

    #[test]
    fn private_dir_and_file_grant_only_the_user() {
        let base = std::env::temp_dir().join(format!("tg-acl-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let dir = base.join("cfg");
        private_dir_create(&dir, true).expect("private dir");
        let inherited = dir.join("new.yaml");
        std::fs::write(&inherited, "x").expect("write");
        let other = base.join("export.txt");
        std::fs::write(&other, "x").expect("write");
        // Explicit entries (not inherited) must go too, as on a CI runner's elevated temp dir.
        let explicit = Command::new("icacls")
            .arg(&other)
            .args(["/grant", "*S-1-5-18:F", "/grant", "*S-1-5-32-545:R", "/q"])
            .status()
            .expect("icacls");
        assert!(explicit.success());
        set_private_file(&other).expect("private file");
        let sid = user_sid().expect("sid");
        for p in [&dir, &inherited, &other] {
            // icacls starts with the path itself (`C:\Users\…`): look only at the grants after it.
            let text = acl(p).replace(&p.display().to_string(), "");
            for group in ["BUILTIN\\", "NT AUTHORITY\\", "Everyone", "CREATOR OWNER"] {
                assert!(
                    !text.contains(group),
                    "{} still grants {group}:\n{text}",
                    p.display()
                );
            }
            assert!(
                text.contains("(F)"),
                "{} has no full-control grant:\n{text}",
                p.display()
            );
        }
        assert!(sid.starts_with("S-1-5-"));
        let _ = std::fs::remove_dir_all(&base);
    }
}
