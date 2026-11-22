//! RDP on Windows: `mstsc.exe` opens a short-lived `.rdp` file (`rdp::rdp_file` in core). A saved
//! password goes into it encrypted with DPAPI for this user (`password 51:b:`), so it is never in
//! an argument list; without one mstsc asks for it itself. The file lives in the private runtime
//! folder and is deleted shortly after mstsc has read it.

use termigator_core::config::doc::Value;

/// The command line of an RDP tab on Windows: `<mstsc> <file.rdp>`, or the reason it cannot run.
/// `conn` has its password revealed (plain text, not a sealed token).
pub fn mstsc_command(conn: &Value) -> Result<String, String> {
    imp::mstsc_command(conn)
}

#[cfg(windows)]
mod imp {
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::Duration;

    use termigator_core::config::doc::Value;
    use termigator_core::config::model::value_str;
    use termigator_core::proto::rdp::rdp_file;
    use termigator_core::proto::ssh::which;
    use termigator_core::shell::join;
    use termigator_core::vault::SEALED_PREFIX;

    /// mstsc reads the file when it starts; it is gone well before anyone could pick it up later.
    const KEEP_FILE: Duration = Duration::from_secs(20);

    pub fn mstsc_command(conn: &Value) -> Result<String, String> {
        let password = conn.get("password").map(value_str).unwrap_or_default();
        if password.starts_with(SEALED_PREFIX) {
            return Err("Password is still encrypted — unlock the vault first.".to_owned());
        }
        let blob = if password.is_empty() {
            None
        } else {
            Some(protect(&password).ok_or("Could not encrypt the password for mstsc (DPAPI).")?)
        };
        let home_drive = std::env::var("SystemDrive").ok();
        let text = rdp_file(conn, blob.as_deref(), home_drive.as_deref())?;
        let mstsc = which("mstsc", std::env::var("PATH").ok().as_deref())
            .ok_or("mstsc.exe (Remote Desktop Connection) was not found.")?;
        let file = write_private(&text)?;
        let doomed = file.clone();
        std::thread::spawn(move || {
            std::thread::sleep(KEEP_FILE);
            let _ = std::fs::remove_file(doomed);
        });
        Ok(join(&[mstsc, file.to_string_lossy().into_owned()]))
    }

    fn write_private(text: &str) -> Result<PathBuf, String> {
        static N: AtomicU64 = AtomicU64::new(0);
        let dir = termigator_core::paths::private_runtime_dir(&["rdp"])
            .ok_or("No private folder for the .rdp file.")?;
        let file = dir.join(format!(
            "{}-{}.rdp",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        // UTF-16 with a BOM, as mstsc writes its own files (non-ASCII host and user names).
        let mut bytes = vec![0xff, 0xfe];
        bytes.extend(text.encode_utf16().flat_map(u16::to_le_bytes));
        std::fs::write(&file, bytes)
            .map_err(|e| format!("Cannot write {}: {e}", file.display()))?;
        Ok(file)
    }

    /// `password` encrypted with DPAPI for the current user, as mstsc's `password 51:b:` wants it:
    /// the UTF-16LE text, upper-case hex of the blob.
    fn protect(password: &str) -> Option<String> {
        use windows_sys::Win32::Foundation::LocalFree;
        use windows_sys::Win32::Security::Cryptography::{CRYPT_INTEGER_BLOB, CryptProtectData};
        let mut plain: Vec<u8> = password.encode_utf16().flat_map(u16::to_le_bytes).collect();
        let input = CRYPT_INTEGER_BLOB {
            cbData: u32::try_from(plain.len()).ok()?,
            pbData: plain.as_mut_ptr(),
        };
        let mut output = CRYPT_INTEGER_BLOB {
            cbData: 0,
            pbData: std::ptr::null_mut(),
        };
        // SAFETY: `input` points at `plain`, alive for the call; the optional arguments are null;
        // on success `output` is a LocalAlloc'd buffer of `cbData` bytes, freed below.
        let ok = unsafe {
            CryptProtectData(
                &input,
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null(),
                0,
                &mut output,
            )
        };
        plain.iter_mut().for_each(|b| *b = 0);
        if ok == 0 || output.pbData.is_null() {
            return None;
        }
        // SAFETY: CryptProtectData succeeded: `pbData` holds `cbData` initialised bytes.
        let blob = unsafe { std::slice::from_raw_parts(output.pbData, output.cbData as usize) };
        let hex = blob.iter().map(|b| format!("{b:02X}")).collect();
        // SAFETY: the buffer came from CryptProtectData (LocalAlloc) and is not used after this.
        unsafe { LocalFree(output.pbData.cast()) };
        Some(hex)
    }
}

#[cfg(not(windows))]
mod imp {
    use termigator_core::config::doc::Value;

    pub fn mstsc_command(_conn: &Value) -> Result<String, String> {
        Err("mstsc is a Windows program.".to_owned())
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    fn unprotect(hex: &str) -> String {
        use windows_sys::Win32::Foundation::LocalFree;
        use windows_sys::Win32::Security::Cryptography::{CRYPT_INTEGER_BLOB, CryptUnprotectData};
        let mut bytes: Vec<u8> = (0..hex.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).expect("hex"))
            .collect();
        let input = CRYPT_INTEGER_BLOB {
            cbData: u32::try_from(bytes.len()).expect("size"),
            pbData: bytes.as_mut_ptr(),
        };
        let mut out = CRYPT_INTEGER_BLOB {
            cbData: 0,
            pbData: std::ptr::null_mut(),
        };
        // SAFETY: test helper; same contract as CryptProtectData in `protect`.
        let ok = unsafe {
            CryptUnprotectData(
                &input,
                std::ptr::null_mut(),
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null(),
                0,
                &mut out,
            )
        };
        assert!(ok != 0, "CryptUnprotectData failed");
        // SAFETY: success: `pbData` holds `cbData` bytes, freed right after.
        let plain = unsafe { std::slice::from_raw_parts(out.pbData, out.cbData as usize) }.to_vec();
        // SAFETY: LocalAlloc'd by CryptUnprotectData.
        unsafe { LocalFree(out.pbData.cast()) };
        let units: Vec<u16> = plain
            .chunks(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .collect();
        String::from_utf16(&units).expect("utf-16")
    }

    #[test]
    fn mstsc_gets_a_dpapi_password_in_a_private_file() {
        let conn = termigator_core::config::doc::load(
            "host: win.lan\nuser: ann\npassword: 'zażółć P@ss'\n",
        )
        .expect("yaml");
        let cmd = mstsc_command(&conn).expect("command");
        let argv = termigator_core::shell::split(&cmd).expect("split");
        assert!(argv[0].to_lowercase().ends_with("mstsc.exe"), "{cmd}");
        let raw = std::fs::read(&argv[1]).expect("rdp file");
        assert_eq!(raw[..2], [0xff, 0xfe]);
        let units: Vec<u16> = raw[2..]
            .chunks(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .collect();
        let text = String::from_utf16(&units).expect("utf-16");
        assert!(text.contains("full address:s:win.lan\r\n") && text.contains("username:s:ann\r\n"));
        assert!(!text.contains("P@ss"), "plain password in the file");
        let hex = text
            .lines()
            .find_map(|l| l.strip_prefix("password 51:b:"))
            .expect("password line");
        assert_eq!(unprotect(hex), "zażółć P@ss");
        let _ = std::fs::remove_file(&argv[1]);
        // No password: mstsc asks.
        let conn = termigator_core::config::doc::load("host: win.lan\n").expect("yaml");
        let argv =
            termigator_core::shell::split(&mstsc_command(&conn).expect("cmd")).expect("split");
        let raw = std::fs::read(&argv[1]).expect("rdp file");
        assert!(!String::from_utf8_lossy(&raw).contains("p\0a\0s\0s\0w\0o\0r\0d"));
        let _ = std::fs::remove_file(&argv[1]);
    }
}
