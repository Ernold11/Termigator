//! File → Import / Export and the backup in the real binary, offscreen
//! (`qml/import/ImportSmoke.qml`). HOME and XDG_DATA_HOME point into a fixture folder holding a
//! `~/.ssh/config`, a PuTTY session, a Remmina profile and a MobaXterm export; this side checks
//! what the check wrote: the merged `session.yaml`, the exported ssh_config (mode 0600) and the
//! backup zip.
// Unix file modes and symlinks throughout; the Windows build is checked on the VM by hand.
#![cfg(unix)]

mod common;

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;

// cxx-qt's generated C++ is linked into this test too; it needs the library's Rust half.
use termigator as _;

const CONFIG: &str = "\
connections:
- {label: web, host: 10.0.0.1, user: bob, password: secret-pw}
";

const MOBA: &str = "[Bookmarks]\nSubRep=\nImgNum=42\nmoba=#109#0%10.0.0.5%22%admin%%-1%-1%%%%%0%0%0%%%-1%0%0%0%%1080%%0%0%1#MobaFont%10%0%0%-1%15%236,236,236%30,30,30%180,180,192%0%-1%0%%xterm%-1%-1%_Std_Colors_0_%80%24%0%1%-1%<none>%%0%0%-1#0# #-1\n";

fn write(path: PathBuf, text: &str) {
    fs::create_dir_all(path.parent().expect("parent")).expect("dir");
    fs::write(path, text).expect("write fixture");
}

#[test]
fn import_export_and_backup_offscreen() {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("termigator-u10-fixtures");
    let _ = fs::remove_dir_all(&dir);
    let home = dir.join("home");
    write(
        home.join(".ssh/config"),
        "Host web\n  HostName 10.0.0.1\n  User bob\nHost db\n  HostName 10.0.0.2\n  User bob\n",
    );
    write(
        home.join(".putty/sessions/Box"),
        "HostName=10.0.0.9\nProtocol=ssh\nPortNumber=22\n",
    );
    write(
        home.join("xdg/remmina/a.remmina"),
        "[remmina]\nname=box\nprotocol=SSH\nserver=10.0.0.3\nusername=u\n",
    );
    write(
        home.join("xdg/remmina/b.remmina"),
        "[remmina]\nname=x\nprotocol=SPICE\nserver=10.0.0.4\n",
    );
    write(dir.join("s.mxtsessions"), MOBA);

    let dir_s = dir.to_string_lossy().into_owned();
    let home_s = home.to_string_lossy().into_owned();
    let xdg_s = home.join("xdg").to_string_lossy().into_owned();
    let mut run = common::launch_env(
        "u10-import",
        CONFIG,
        &[
            "--smoke-check=import/ImportSmoke.qml",
            &format!("--u10-dir={dir_s}"),
        ],
        &[("HOME", &home_s), ("XDG_DATA_HOME", &xdg_s)],
    );
    let status = run.wait(60, |_| {});
    let log = run.log();
    assert!(status.success(), "exit {status}, stderr:\n{log}");
    assert!(
        log.contains("termigator import smoke: done"),
        "stderr:\n{log}"
    );
    for bad in ["TypeError", "ReferenceError", "qml/import/"] {
        assert!(!log.contains(bad), "{bad} in stderr:\n{log}");
    }

    let session = fs::read_to_string(run.config.join("session.yaml")).expect("session.yaml");
    for host in ["10.0.0.2", "10.0.0.3", "10.0.0.9", "10.0.0.5"] {
        assert!(session.contains(host), "{host} missing:\n{session}");
    }
    assert_eq!(session.matches("host: 10.0.0.1").count(), 1, "{session}");

    let out = dir.join("out.conf");
    let conf = fs::read_to_string(&out).expect("out.conf");
    assert!(
        conf.contains("Host web") && !conf.contains("secret-pw"),
        "{conf}"
    );
    assert_eq!(
        fs::metadata(&out).expect("meta").permissions().mode() & 0o777,
        0o600
    );
    let zip = dir.join("backup.zip");
    assert_eq!(
        fs::metadata(&zip).expect("zip").permissions().mode() & 0o777,
        0o600
    );
    assert!(
        fs::metadata(&run.shot).map(|m| m.len()).unwrap_or(0) > 0,
        "no grab"
    );
}
