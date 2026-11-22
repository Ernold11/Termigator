//! `qml/files/FilesSmoke.qml` offscreen, against a "remote host" that is this machine. A
//! stand-in `ssh` first in PATH runs the remote command (or sftp-server for scp's `-s`), so the
//! real ssh / scp / sftp argv of the Files panel run end to end: listing, sorting, uploads and
//! downloads with clashes, mkdir / rename / delete / chmod, edit in place, the transfer queue
//! (cancel, resume with sftp, retry, two at a time), drag and drop entry points, bookmarks,
//! follow, two panes, Synchronize, a Files tab and the host menu's one-shot scp. This side builds
//! the folders the check works on and then looks at the disk.
// Unix file modes and symlinks throughout; the Windows build is checked on the VM by hand.
#![cfg(unix)]

mod common;

use std::fs::{self, File};
use std::os::unix::fs::{PermissionsExt, symlink};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use termigator_core::config::doc::{self, Value};
use termigator_core::config::model::value_str;

// cxx-qt's generated C++ is linked into this test too; it needs the library's Rust half.
use termigator as _;

// Where the distributions put OpenSSH's sftp-server: Arch, Debian / Ubuntu, Fedora.
const SFTP_SERVERS: [&str; 3] = [
    "/usr/lib/ssh/sftp-server",
    "/usr/lib/openssh/sftp-server",
    "/usr/libexec/openssh/sftp-server",
];

fn script(path: &Path, text: &str) {
    common::write_executable(path, text);
}

fn pseudo_random(n: usize) -> Vec<u8> {
    let mut x: u32 = 0x1234_5678;
    (0..n)
        .map(|_| {
            x = x.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            x.to_be_bytes()[0]
        })
        .collect()
}

fn write(path: impl AsRef<Path>, data: impl AsRef<[u8]>) {
    let path = path.as_ref();
    fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
    fs::write(path, data).expect("write");
}

#[test]
fn files_panel_runs_offscreen_against_a_local_host() {
    let (Some(scp), Some(sftp)) = (which("scp"), which("sftp")) else {
        eprintln!("skipped: needs scp and sftp");
        return;
    };
    let Some(sftp_server) = SFTP_SERVERS.into_iter().find(|p| Path::new(p).is_file()) else {
        eprintln!("skipped: needs sftp-server (one of {SFTP_SERVERS:?})");
        return;
    };
    let root = Command::new("id")
        .arg("-u")
        .output()
        .ok()
        .is_some_and(|o| o.stdout == b"0\n");
    // A read-only folder does not stop root (CI runs as root): then the app runs as nobody, in
    // a tree nobody owns.
    if root && which("setpriv").is_none() {
        eprintln!("skipped: running as root without setpriv");
        return;
    }
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("termigator-u9-files");
    // An interrupted run can leave ro/ read-only, which would make remove_dir_all fail.
    let _ = fs::set_permissions(dir.join("remote/ro"), fs::Permissions::from_mode(0o755));
    let _ = fs::remove_dir_all(&dir);
    let stem = format!("{}/", dir.display());
    let (remote, local, bin) = (dir.join("remote"), dir.join("local"), dir.join("bin"));
    fs::create_dir_all(&bin).expect("bin");

    // The stand-in ssh: the remote command runs here; scp / sftp ask for the sftp subsystem with
    // `-s`, which waits 3 s while a remote file called SLOW exists (the check makes it).
    script(
        &bin.join("ssh"),
        &format!(
            "#!/bin/sh\nsub=\nfor a in \"$@\"; do last=\"$a\"; [ \"$a\" = -s ] && sub=1; done\n\
             case \"$*\" in *u@locked*) [ \"$SSHPASS\" = open-sesame ] || {{ echo 'u@locked: Permission denied (publickey,password).' >&2; exit 255; }};; esac\n\
             if [ -n \"$sub\" ]; then\n  [ -e '{stem}remote/SLOW' ] && sleep 3\n  exec {sftp_server}\nfi\n\
             exec sh -c \"$last\"\n"
        ),
    );
    // sshpass only has to hand the command on: the stand-in ssh reads $SSHPASS itself.
    script(&bin.join("sshpass"), "#!/bin/sh\nshift\nexec \"$@\"\n");
    // scp and sftp start /usr/bin/ssh, not the one in PATH: `-S` points them at the stand-in.
    for (name, real) in [("scp", &scp), ("sftp", &sftp)] {
        script(
            &bin.join(name),
            &format!(
                "#!/bin/sh\nexec {} -S {}/ssh \"$@\"\n",
                real.display(),
                bin.display()
            ),
        );
    }
    // The "editor" of Edit: appends a line to the working copy it is given. Later the
    // server copy changes behind its back, then it saves again — that save must ask first.
    script(
        &bin.join("edit"),
        &format!(
            "#!/bin/sh\necho EDITED >> \"$1\"\n\
             (sleep 3; echo SERVER >> '{}'; sleep 2; echo SECOND >> \"$1\") >/dev/null 2>&1 &\n",
            remote.join("a.txt").display()
        ),
    );

    let big = pseudo_random(3 * 1024 * 1024);
    write(remote.join("a.txt"), "alpha\n");
    write(remote.join("big.bin"), &big);
    write(remote.join(".hidden"), "h");
    write(remote.join("sub/in.txt"), "inner");
    write(remote.join("sub/odd\n!touch PWNED"), "odd");
    symlink("a.txt", remote.join("link")).expect("link");
    fs::create_dir_all(remote.join("ro")).expect("ro");
    fs::set_permissions(remote.join("ro"), fs::Permissions::from_mode(0o555)).expect("chmod ro");
    write(dir.join("syncr/r_only.txt"), "from the server");
    write(dir.join("syncr/common.txt"), "same");

    write(local.join("up1.txt"), "one\n");
    write(local.join("up2.txt"), "two\n");
    write(local.join("a.txt"), "LOCAL\n");
    write(local.join("in.txt"), "LOCAL_IN\n"); // clashes with sub/in.txt
    write(local.join("SLOW"), "x");
    write(local.join("lfolder/f1.txt"), "f1");
    write(local.join("lfolder/deep/f2.txt"), "f2");
    // Enough rows for the local pane's wheel check.
    for i in 0..60 {
        write(local.join(format!("many/f{i:02}.txt")), "x");
    }
    for n in 1..=4 {
        write(local.join(format!("bulk{n}.bin")), pseudo_random(64 * 1024));
    }
    // An interrupted download: the first megabyte is already here.
    write(local.join("resume/big.bin"), &big[..1024 * 1024]);
    fs::create_dir_all(local.join("dlmany")).expect("dlmany");
    write(dir.join("syncl/l_only.txt"), "from here");
    write(dir.join("syncl/common.txt"), "same");
    write(dir.join("syncl/sub/deep.txt"), "deep");

    let config = dir.join("config");
    fs::create_dir_all(&config).expect("config dir");
    fs::write(
        config.join("config.yaml"),
        format!(
            "window:\n  confirm_close: false\n  confirm_close_tab: false\n  width: 1100\n  height: 800\n\
             launch:\n  editor: {}/edit\n\
             connections:\n- {{label: box, host: h, user: u}}\n- {{label: locked, host: locked, user: u}}\n",
            bin.display()
        ),
    )
    .expect("config.yaml");
    let socket = std::env::temp_dir().join(format!("termigator-u9-{}.sock", std::process::id()));
    let log_path = dir.join("stderr.log");
    let path = format!(
        "{}:{}",
        bin.display(),
        std::env::var("PATH").unwrap_or_default()
    );
    let mut command = if root {
        let owned = Command::new("chown")
            .args(["-R", "nobody:nobody"])
            .arg(&dir)
            .status()
            .expect("chown");
        assert!(owned.success(), "chown -R nobody {}", dir.display());
        let mut c = Command::new("setpriv");
        c.args(["--reuid=nobody", "--regid=nobody", "--clear-groups", "--"])
            .arg(env!("CARGO_BIN_EXE_termigator"))
            .env("HOME", &dir);
        c
    } else {
        Command::new(env!("CARGO_BIN_EXE_termigator"))
    };
    let mut child = command
        .arg("--smoke-check=files/FilesSmoke.qml")
        .current_dir(&dir) // an sftp batch line that ran a command would leave its file here
        .env("PATH", path)
        .env("QT_QPA_PLATFORM", "offscreen")
        .env("QT_QUICK_BACKEND", "software")
        .env("QT_FORCE_STDERR_LOGGING", "1")
        .env("TERMIGATOR_SMOKE_TEST", dir.join("shot.png"))
        .env("TERMIGATOR_CONFIG_DIR", &config)
        .env("TERMIGATOR_CONTROL_SOCKET", &socket)
        .env("XDG_CACHE_HOME", dir.join("cache"))
        .stdout(Stdio::null())
        .stderr(File::create(&log_path).expect("log"))
        .spawn()
        .expect("spawn termigator");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(180);
    let status = loop {
        if let Some(s) = child.try_wait().expect("wait") {
            break s;
        }
        if std::time::Instant::now() > deadline {
            let _ = child.kill();
            panic!(
                "termigator did not quit in time, stderr:\n{}",
                fs::read_to_string(&log_path).unwrap_or_default()
            );
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    };
    let log = fs::read_to_string(&log_path).unwrap_or_default();
    assert!(status.success(), "exit {status}, stderr:\n{log}");
    assert!(
        log.contains("FILES_DONE"),
        "the check did not finish:\n{log}"
    );
    // The style's empty TabBar of a pane without tabs logs a TypeError at start (the panes and
    // the terminals show it too); anything else is ours.
    let ours: Vec<&str> = log
        .lines()
        .filter(|l| {
            !l.contains("org/kde/desktop/TabBar.qml") && !l.contains("DelegateModel::cancel")
        })
        .collect();
    for bad in ["TypeError", "ReferenceError", "qml/files/"] {
        assert!(
            !ours.iter().any(|l| l.contains(bad)),
            "{bad} in stderr:\n{log}"
        );
    }

    let text =
        |p: PathBuf| fs::read_to_string(&p).unwrap_or_else(|e| panic!("{}: {e}", p.display()));
    // Uploads: new, overwriting, folder, dropped into a folder, the arrow of the local pane.
    assert_eq!(text(remote.join("up1.txt")), "one\n");
    assert_eq!(text(remote.join("lfolder/f1.txt")), "f1");
    assert_eq!(text(remote.join("lfolder/deep/f2.txt")), "f2");
    assert_eq!(text(remote.join("sub/up2.txt")), "two\n");
    assert_eq!(text(remote.join("sub/in.txt")), "inner"); // answered No, kept
    assert_eq!(text(remote.join("up2.txt")), "two\n");
    // The overwrite, then the edited save: the working copy got a line and went back.
    // The second save met a changed server copy and was answered No.
    assert_eq!(text(remote.join("a.txt")), "LOCAL\nEDITED\nSERVER\n");
    // Downloads.
    assert_eq!(text(local.join("dl_a.txt")), "LOCAL\n");
    assert_eq!(text(local.join("dlmany/sub/in.txt")), "inner");
    assert_eq!(fs::read(local.join("dlmany/big.bin")).expect("big"), big);
    assert_eq!(fs::read(local.join("big.bin")).expect("dropped big"), big);
    // Resumed with sftp reget from the first megabyte.
    assert_eq!(
        fs::read(local.join("resume/big.bin")).expect("resumed"),
        big
    );
    // The retried upload into a folder that was read-only at first.
    assert_eq!(text(remote.join("ro/up2.txt")), "two\n");
    // The name with a line break never reached the sftp batch file.
    assert!(!dir.join("PWNED").exists(), "an sftp batch line was run");
    assert!(!remote.join("PWNED").exists() && !local.join("PWNED").exists());
    // Synchronize (both ways): the new files crossed, with their times.
    assert_eq!(text(dir.join("syncr/l_only.txt")), "from here");
    assert_eq!(text(dir.join("syncr/sub/deep.txt")), "deep");
    assert_eq!(text(dir.join("syncl/r_only.txt")), "from the server");
    let mtime = |p: PathBuf| fs::metadata(p).and_then(|m| m.modified()).expect("mtime");
    let (a, b) = (
        mtime(dir.join("syncl/l_only.txt")),
        mtime(dir.join("syncr/l_only.txt")),
    );
    let apart = a
        .duration_since(b)
        .or_else(|_| b.duration_since(a))
        .expect("diff");
    assert!(
        apart.as_secs() <= 2,
        "copies keep their modification time ({apart:?})"
    );
    // The one-shot scp of the host menu ran in a terminal tab.
    assert_eq!(text(remote.join("scp_up.txt")), "one\n");
    // No SLOW left behind, nothing deleted by accident.
    assert!(!remote.join("SLOW").exists(), "SLOW left");
    assert!(!remote.join("renamed").exists() && !remote.join("newdir").exists());
    assert!(remote.join("a.txt").exists() && remote.join("sub/in.txt").exists());

    // What the config remembers: bookmarks, the last folder, optional columns, two panes off.
    let saved = text(config.join("config.yaml"));
    let cfg = doc::load(&saved).expect("saved yaml");
    let marks = cfg
        .get("files_bookmarks")
        .and_then(|s| s.get("u@h:22"))
        .expect("host state");
    let Some(Value::Seq(bookmarks)) = marks.get("bookmarks") else {
        panic!("no bookmarks:\n{saved}");
    };
    assert_eq!(
        bookmarks.iter().map(value_str).collect::<Vec<_>>(),
        [remote.display().to_string()]
    );
    assert!(marks.get("last").is_some(), "no last folder:\n{saved}");
    let launch = cfg.get("launch").expect("launch");
    let Some(Value::Seq(columns)) = launch.get("files_columns") else {
        panic!("no files_columns:\n{saved}");
    };
    assert_eq!(
        columns.iter().map(value_str).collect::<Vec<_>>(),
        ["permissions", "owner"]
    );
    assert_eq!(
        launch.get("files_two_pane_side").map(value_str).as_deref(),
        Some("False")
    );
    // Column widths dragged in the header are remembered next to files_columns (every width, Name
    // included, after the first drag); the check ends with "Automatic column widths", which
    // forgets them: the key stays, empty.
    let widths = launch
        .get("files_column_widths")
        .expect("no files_column_widths");
    assert!(
        matches!(widths, Value::Map(pairs) if pairs.is_empty()),
        "widths not forgotten:\n{saved}"
    );
}

fn which(name: &str) -> Option<PathBuf> {
    std::env::var_os("PATH")?
        .to_string_lossy()
        .split(':')
        .map(|d| Path::new(d).join(name))
        .find(|p| p.is_file())
}
