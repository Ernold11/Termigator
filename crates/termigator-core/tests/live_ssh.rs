//! OpenSSH tools for real — scp against a local `sftp-server` (transfers through the
//! production worker), ssh-keygen / ssh-agent / ssh-copy-id and sshpass
//! answering the password prompt with the credential's password.
//! The "remote" side is always this machine: a fake `ssh` that execs `sftp-server` or a shell.
// Unix-only for now: uses modes, symlinks, sockets or sh stubs.
#![cfg(unix)]

mod live;

use std::{
    collections::BTreeSet,
    ffi::OsString,
    fs,
    path::{Path, PathBuf},
    process::Command,
    time::Duration,
};

use live::{Term, have, map, s, wait_until};
use termigator_core::{
    config::doc::Value,
    credentials::apply_credential,
    proc::transfer::{Event, TransferHandle, start},
    proto::{
        scp::{ScpTransfer, build_scp_command_with, scp_argv_with},
        ssh::{self, Env, build_ssh_command_with, ssh_password_env},
        ssh_keys::{add_to_agent_argv, agent_fingerprints, copy_id_argv, keygen_argv, list_keys},
    },
    remote::transfers::TransferJob,
    shell,
};

/// `shutil.which` over `path` for the builders.
fn env_on(path: &std::ffi::OsStr) -> Env {
    let path = path.to_string_lossy().into_owned();
    Env {
        which: Box::new(move |n| ssh::which(n, Some(&path))),
        control_dir: None,
        home: None,
    }
}

fn conn() -> Value {
    map(&[("host", s("h")), ("user", s("u"))])
}

fn launch() -> Value {
    map(&[
        ("ssh_control_master", Value::Bool(false)),
        ("ssh_keepalive", Value::Bool(false)),
    ])
}

/// A fake `ssh` in `dir` that runs `body` (e.g. `exec sftp-server`).
fn fake_ssh(dir: &Path, body: &str) -> PathBuf {
    let p = dir.join("ssh");
    live::script(&p, &format!("#!/bin/sh\n{body}\n"));
    p
}

/// Inserts `-S <fake>` right after the `scp` program.
fn with_fake_ssh(mut argv: Vec<String>, fake: &Path) -> Vec<String> {
    let i = argv.iter().position(|a| a == "scp").expect("scp in argv");
    argv.splice(
        i + 1..i + 1,
        ["-S".to_owned(), fake.to_string_lossy().into_owned()],
    );
    argv
}

fn sorted_names(dir: &Path) -> Vec<String> {
    let mut v: Vec<String> = fs::read_dir(dir)
        .expect("read dir")
        .map(|e| e.expect("entry").file_name().to_string_lossy().into_owned())
        .collect();
    v.sort();
    v
}

/// scp's SFTP mode glob-expands a download source — fetching "[ab].log" got a.log and
/// b.log instead. The built command must fetch exactly the named file.
#[test]
fn real_scp_downloads_exactly_the_named_file() {
    let _g = live::serial();
    let test = "real_scp_downloads_exactly_the_named_file";
    if !have(test, &["scp"]) {
        return;
    }
    let Some(srv) = live::sftp_server() else {
        live::skip(test, "needs an sftp-server binary");
        return;
    };
    let d = live::scratch("glob");
    let (remote, local) = (d.join("remote"), d.join("local"));
    fs::create_dir(&remote).expect("remote");
    fs::create_dir(&local).expect("local");
    let names = [
        "[ab].log",
        "a.log",
        "b.log",
        "q?.txt",
        "qx.txt",
        "back\\slash.txt",
        "star*.txt",
    ];
    for n in names {
        fs::write(remote.join(n), n).expect("remote file");
    }
    let fake = fake_ssh(
        &d,
        &format!("exec {}", shell::quote(&srv.to_string_lossy())),
    );
    let env = env_on(&std::env::var_os("PATH").unwrap_or_default());
    for n in ["[ab].log", "q?.txt", "back\\slash.txt", "star*.txt"] {
        for f in fs::read_dir(&local).expect("local").flatten() {
            fs::remove_file(f.path()).expect("clear local");
        }
        let line = build_scp_command_with(
            &conn(),
            local.to_str().expect("utf-8"),
            remote.join(n).to_str().expect("utf-8"),
            false,
            &env,
        )
        .expect("scp line");
        let argv = with_fake_ssh(shell::split(&line).expect("split"), &fake);
        let out = Command::new(&argv[0])
            .args(&argv[1..])
            .output()
            .expect("scp");
        assert_eq!(sorted_names(&local), [n], "{n}: {out:?}");
    }
    let _ = fs::remove_dir_all(&d);
}

fn collect(h: &TransferHandle) -> Vec<Event> {
    let mut out = Vec::new();
    while let Ok(ev) = h.events().recv_timeout(Duration::from_secs(30)) {
        out.push(ev);
    }
    out
}

/// A Files-panel transfer (`scp_argv_with`) run by the production worker with a fake ssh.
fn transfer(t: &ScpTransfer, fake: &Path) -> TransferHandle {
    let env = env_on(&std::env::var_os("PATH").unwrap_or_default());
    let argv = with_fake_ssh(
        scp_argv_with(&conn(), &launch(), t, &env).expect("scp argv"),
        fake,
    );
    start(TransferJob::new(
        t.upload,
        PathBuf::from(t.local_path),
        t.remote_path.as_bytes().to_vec(),
        b"x".to_vec(),
        argv.into_iter().map(String::into_bytes).collect(),
        Vec::new(),
    ))
}

fn scp_transfer<'a>(local: &'a str, remote: &'a str, upload: bool, rec: bool) -> ScpTransfer<'a> {
    ScpTransfer {
        local_path: local,
        remote_path: remote,
        upload,
        recursive: rec,
        preserve: false,
    }
}

/// Transfers with the real scp instead of a stub: a download writes the
/// local file, a folder upload and a folder download are recursive.
#[test]
fn real_scp_transfers_through_the_worker() {
    let _g = live::serial();
    let test = "real_scp_transfers_through_the_worker";
    if !have(test, &["scp"]) {
        return;
    }
    let Some(srv) = live::sftp_server() else {
        live::skip(test, "needs an sftp-server binary");
        return;
    };
    let d = live::scratch("xfer");
    let fake = fake_ssh(
        &d,
        &format!("exec {}", shell::quote(&srv.to_string_lossy())),
    );
    let (remote, local) = (d.join("remote"), d.join("local"));
    fs::create_dir_all(remote.join("logs")).expect("remote");
    fs::create_dir_all(local.join("site/css")).expect("local");
    fs::write(remote.join("file.bin"), "data\n").expect("remote file");
    fs::write(remote.join("logs/x.log"), "log\n").expect("remote log");
    fs::write(local.join("site/css/a.css"), "body{}\n").expect("local css");
    let ps = |p: &Path| p.to_str().expect("utf-8").to_owned();

    // download_writes_the_local_file
    let (dst, src) = (ps(&local.join("out.bin")), ps(&remote.join("file.bin")));
    let evs = collect(&transfer(&scp_transfer(&dst, &src, false, false), &fake));
    assert_eq!(evs.last(), Some(&Event::Done), "{evs:?}");
    assert_eq!(fs::read_to_string(&dst).expect("downloaded"), "data\n");

    // upload_folder_is_recursive
    let (site, into) = (ps(&local.join("site")), format!("{}/", ps(&remote)));
    let evs = collect(&transfer(&scp_transfer(&site, &into, true, true), &fake));
    assert_eq!(evs.last(), Some(&Event::Done), "{evs:?}");
    assert_eq!(
        fs::read_to_string(remote.join("site/css/a.css")).expect("uploaded"),
        "body{}\n"
    );

    // download_many_with_folder
    let target = local.join("dl");
    fs::create_dir(&target).expect("dl");
    let (to, logs) = (ps(&target), ps(&remote.join("logs")));
    let evs = collect(&transfer(&scp_transfer(&to, &logs, false, true), &fake));
    assert_eq!(evs.last(), Some(&Event::Done), "{evs:?}");
    assert_eq!(
        fs::read_to_string(target.join("logs/x.log")).expect("folder"),
        "log\n"
    );
    let _ = fs::remove_dir_all(&d);
}

/// test_cancel_stops_the_running_scp with the real scp: cancelling ends the job as
/// cancelled and takes scp's ssh down with it.
#[test]
fn real_scp_cancel_kills_its_ssh() {
    let _g = live::serial();
    if !have("real_scp_cancel_kills_its_ssh", &["scp"]) {
        return;
    }
    let d = live::scratch("cancel");
    let pidfile = d.join("pid");
    let fake = fake_ssh(
        &d,
        &format!("echo $$ > {}\nexec sleep 30", pidfile.display()),
    );
    let (dst, src) = (d.join("out.bin"), d.join("remote.bin"));
    let h = transfer(
        &scp_transfer(
            dst.to_str().expect("utf-8"),
            src.to_str().expect("utf-8"),
            false,
            false,
        ),
        &fake,
    );
    let mut pid = String::new();
    assert!(
        wait_until(Duration::from_secs(10), || {
            pid = fs::read_to_string(&pidfile).unwrap_or_default();
            pid.ends_with('\n')
        }),
        "scp never started its ssh"
    );
    let pid = pid.trim().to_owned();
    std::thread::sleep(Duration::from_millis(300));
    h.cancel();
    assert_eq!(collect(&h).last(), Some(&Event::Cancelled));
    assert!(
        wait_until(Duration::from_secs(3), || !live::alive(&pid)),
        "ssh {pid} survived the cancel"
    );
    let _ = fs::remove_dir_all(&d);
}

fn keygen(dir: &Path, name: &str, ktype: &str, comment: &str) -> PathBuf {
    let path = dir.join(name);
    let st = Command::new("ssh-keygen")
        .args(["-q", "-t", ktype, "-N", "", "-C", comment, "-f"])
        .arg(&path)
        .status()
        .expect("ssh-keygen");
    assert!(st.success(), "ssh-keygen {ktype}");
    path
}

/// A fresh private key directory.
fn keydir(tag: &str) -> PathBuf {
    let d = live::scratch(tag);
    keygen(&d, "id_ed25519", "ed25519", "me@laptop work");
    keygen(&d, "id_rsa", "rsa", "");
    fs::write(d.join("orphan.pub"), "ssh-ed25519 AAAA nobody\n").expect("orphan"); // no private key
    fs::write(d.join("known_hosts"), "").expect("known_hosts");
    d
}

#[test]
fn list_keys_with_real_ssh_keygen() {
    let _g = live::serial();
    if !have("list_keys_with_real_ssh_keygen", &["ssh-keygen"]) {
        return;
    }
    let d = keydir("keys");
    let keys = list_keys(&d);
    let names: Vec<String> = keys
        .iter()
        .map(|k| {
            k.path
                .file_name()
                .expect("name")
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    assert_eq!(names, ["id_ed25519", "id_rsa"]);
    let (ed, rsa) = (&keys[0], &keys[1]);
    assert_eq!(
        (ed.key_type.as_str(), ed.bits, ed.comment.as_str()),
        ("ED25519", 256, "me@laptop work")
    );
    assert!(ed.fingerprint.starts_with("SHA256:"), "{ed:?}");
    assert_eq!((rsa.key_type.as_str(), rsa.comment.as_str()), ("RSA", ""));
    let _ = fs::remove_dir_all(&d);
}

#[test]
fn keygen_argv_runs_in_ssh_keygen() {
    let _g = live::serial();
    if !have("keygen_argv_runs_in_ssh_keygen", &["ssh-keygen"]) {
        return;
    }
    let d = keydir("keygen");
    let new = d.join("new_rsa");
    let new_s = new.to_str().expect("utf-8");
    let argv = keygen_argv("rsa", new_s, "c d", None).expect("argv");
    assert_eq!(
        argv,
        [
            "ssh-keygen",
            "-t",
            "rsa",
            "-b",
            "4096",
            "-f",
            new_s,
            "-C",
            "c d"
        ]
    );
    // What the terminal runs, minus the passphrase prompt.
    let out = Command::new(&argv[0])
        .args(&argv[1..])
        .args(["-N", ""])
        .output()
        .expect("ssh-keygen");
    assert!(out.status.success(), "{out:?}");
    assert!(d.join("new_rsa.pub").is_file());
    let existing = d.join("id_ed25519");
    let err = keygen_argv("ed25519", existing.to_str().expect("utf-8"), "", None).unwrap_err();
    assert!(err.contains("already exists"), "{err}");
    let missing = d.join("no/dir/k");
    let err = keygen_argv("ed25519", missing.to_str().expect("utf-8"), "", None).unwrap_err();
    assert!(err.contains("does not exist"), "{err}");
    assert!(keygen_argv("dsa", d.join("x").to_str().expect("utf-8"), "", None).is_err());
    assert!(keygen_argv("ed25519", "", "", None).is_err());
    let _ = fs::remove_dir_all(&d);
}

/// `SSH_AUTH_SOCK` is read from the process environment; the checks run in a child copy of
/// this test binary (no `set_var` in a threaded test process).
const AGENT_CHILD: &str = "TERMIGATOR_V3_AGENT_CHILD";

fn agent_child(mode: &str) {
    if mode == "gone" {
        assert_eq!(agent_fingerprints(), None, "no agent behind the socket");
        return;
    }
    let dir = Path::new(mode);
    // An empty agent answers with exit code 1 — not "no agent".
    assert_eq!(agent_fingerprints(), Some(BTreeSet::new()));
    let argv = add_to_agent_argv(&dir.join("id_ed25519"));
    let out = Command::new(&argv[0])
        .args(&argv[1..])
        .output()
        .expect("ssh-add");
    assert!(out.status.success(), "{out:?}");
    let fp = list_keys(dir)
        .into_iter()
        .find(|k| k.path.ends_with("id_ed25519"))
        .expect("key")
        .fingerprint;
    assert_eq!(agent_fingerprints(), Some(BTreeSet::from([fp])));
}

fn run_agent_child(sock: &Path, mode: &str) {
    let out = Command::new(std::env::current_exe().expect("test binary"))
        .args([
            "--exact",
            "agent_fingerprints_with_a_real_agent",
            "--nocapture",
        ])
        .env("SSH_AUTH_SOCK", sock)
        .env(AGENT_CHILD, mode)
        .output()
        .expect("child");
    assert!(
        out.status.success(),
        "child {mode}:\n{}\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn agent_fingerprints_with_a_real_agent() {
    if let Some(mode) = std::env::var_os(AGENT_CHILD) {
        agent_child(&mode.to_string_lossy());
        return;
    }
    let _g = live::serial();
    let test = "agent_fingerprints_with_a_real_agent";
    if !have(test, &["ssh-keygen", "ssh-agent", "ssh-add"]) {
        return;
    }
    let d = keydir("agent");
    let sock = d.join("agent.sock");
    let out = Command::new("ssh-agent")
        .args(["-s", "-a"])
        .arg(&sock)
        .output()
        .expect("ssh-agent");
    let text = String::from_utf8_lossy(&out.stdout);
    let pid: i32 = text
        .split("SSH_AGENT_PID=")
        .nth(1)
        .and_then(|r| r.split(';').next())
        .and_then(|p| p.parse().ok())
        .unwrap_or_else(|| panic!("no agent pid in {text:?}"));
    struct Agent(i32);
    impl Drop for Agent {
        fn drop(&mut self) {
            let _ = nix::sys::signal::kill(
                nix::unistd::Pid::from_raw(self.0),
                nix::sys::signal::Signal::SIGTERM,
            );
        }
    }
    let agent = Agent(pid);
    run_agent_child(&sock, d.to_str().expect("utf-8"));
    drop(agent);
    run_agent_child(&d.join("gone.sock"), "gone");
    let _ = fs::remove_dir_all(&d);
}

/// test_copy_id_argv and test_copy_id_checks_the_user_too with the installed tools.
#[test]
fn copy_id_argv_with_installed_tools() {
    if !have(
        "copy_id_argv_with_installed_tools",
        &["ssh-copy-id", "sshpass"],
    ) {
        return;
    }
    let d = live::scratch("copyid");
    let pub_key = d.join("k.pub");
    let p = pub_key.to_str().expect("utf-8");
    let conn = map(&[
        ("host", s("db")),
        ("user", s("pg")),
        ("port", Value::Int(2222)),
        ("proxy_jump", s("j@bastion")),
    ]);
    // Through a jump host the password goes to the askpass helper, not sshpass.
    let argv = copy_id_argv(&pub_key, &conn, true, &live::installed).expect("argv");
    assert_eq!(
        argv,
        [
            "ssh-copy-id",
            "-i",
            p,
            "-p",
            "2222",
            "-o",
            "ProxyJump=j@bastion",
            "pg@db"
        ]
    );
    let direct = map(&[("host", s("db")), ("user", s("pg"))]);
    let argv = copy_id_argv(&pub_key, &direct, true, &live::installed).expect("direct");
    assert_eq!(argv, ["sshpass", "-e", "ssh-copy-id", "-i", p, "pg@db"]);
    let plain = copy_id_argv(&pub_key, &map(&[("host", s("h"))]), false, &live::installed);
    assert_eq!(plain.expect("plain"), ["ssh-copy-id", "-i", p, "h"]);
    let bad_host = map(&[("host", s("-oProxyCommand=x"))]);
    assert!(copy_id_argv(&pub_key, &bad_host, false, &live::installed).is_err());
    let bad_user = map(&[("host", s("h")), ("user", s("-oProxyCommand=x"))]);
    let err = copy_id_argv(&pub_key, &bad_user, false, &live::installed).unwrap_err();
    assert!(err.contains("must not start with '-'"), "{err}");
    let ok = copy_id_argv(
        &pub_key,
        &map(&[("host", s("h")), ("user", s("ok"))]),
        false,
        &live::installed,
    );
    assert_eq!(ok.expect("ok").last().map(String::as_str), Some("ok@h"));
    let _ = fs::remove_dir_all(&d);
}

/// The connection has no password of its own; the credential's goes to `sshpass -e` through
/// `SSHPASS` (never argv), and sshpass answers the real password prompt with it.
#[test]
fn sshpass_answers_the_prompt_with_the_credential_password() {
    let _g = live::serial();
    if !have(
        "sshpass_answers_the_prompt_with_the_credential_password",
        &["sshpass"],
    ) {
        return;
    }
    let d = live::scratch("sshpass");
    let bin = d.join("bin");
    fs::create_dir(&bin).expect("bin");
    // A stand-in ssh that asks for the password on its controlling terminal (/dev/tty, the
    // pty sshpass gives it), like OpenSSH does; stdout is still the tab.
    live::script(
        &bin.join("ssh"),
        "#!/bin/sh\nfor last; do :; done\nprintf '%s'\"'\"'s password: ' \"$last\" >/dev/tty\n\
         read -r pw </dev/tty\necho \"GOT:$pw\"\nsleep 5\n",
    );
    let path = live::path_with(&bin);
    let creds = [map(&[
        ("name", s("ops")),
        ("user", s("root")),
        ("password", s("s3cret")),
    ])];
    let entry = map(&[
        ("host", s("h.example")),
        ("user", s("root")),
        ("credential", s("ops")),
    ]);
    let conn = apply_credential(&entry, &creds);
    let cmd = build_ssh_command_with(&conn, &launch(), &env_on(&path)).expect("ssh line");
    assert!(cmd.starts_with("sshpass -e "), "{cmd}");
    assert!(
        cmd.contains("root@h.example") && !cmd.contains("s3cret"),
        "{cmd}"
    );
    let env = ssh_password_env(&conn).expect("password env");
    let extra: Vec<(&str, &str)> = env
        .iter()
        .map(|(k, v)| (k.as_str(), std::str::from_utf8(v.expose()).expect("utf-8")))
        .collect();
    assert_eq!(extra, [("SSHPASS", "s3cret")]);
    let mut tab = Term::bash(&cmd, &[("PATH", &path)], &extra);
    tab.expect("GOT:s3cret", Duration::from_secs(10));
    tab.close();
    let _ = fs::remove_dir_all(&d);
}

/// A key with a passphrase and no agent is not loaded silently — OpenSSH asks for the
/// passphrase right in the (embedded) terminal. Real `sshd` as this user on 127.0.0.1.
#[test]
fn encrypted_key_without_agent_asks_for_the_passphrase_in_the_terminal() {
    let _g = live::serial();
    let name = "encrypted_key_without_agent_asks_for_the_passphrase_in_the_terminal";
    if !have(name, &["sshd", "ssh", "ssh-keygen"]) {
        return;
    }
    let d = live::scratch("encrypted");
    let keygen = |file: &str, pass: &str| {
        let st = Command::new("ssh-keygen")
            .args(["-q", "-t", "ed25519", "-N", pass, "-f"])
            .arg(d.join(file))
            .status()
            .expect("ssh-keygen");
        assert!(st.success(), "ssh-keygen {file}");
    };
    keygen("id", "s3cret-pass");
    keygen("host", "");
    fs::copy(d.join("id.pub"), d.join("authorized_keys")).expect("authorized_keys");
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .expect("free port")
        .local_addr()
        .expect("addr")
        .port();
    let cfg = d.join("sshd_config");
    fs::write(
        &cfg,
        format!(
            "Port {port}\nListenAddress 127.0.0.1\nHostKey {d}/host\nPidFile {d}/sshd.pid\n\
             AuthorizedKeysFile {d}/authorized_keys\nStrictModes no\nUsePAM no\n\
             PasswordAuthentication no\nKbdInteractiveAuthentication no\n",
            d = d.display()
        ),
    )
    .expect("sshd_config");
    let sshd = live::which("sshd").expect("sshd");
    let _server = live::Daemon::spawn(Command::new(sshd).arg("-D").arg("-f").arg(&cfg));
    assert!(
        wait_until(Duration::from_secs(10), || live::port_open(port)),
        "sshd did not listen"
    );
    // The real ssh with the options a test needs (no known_hosts, no user config).
    let bin = d.join("bin");
    fs::create_dir(&bin).expect("bin");
    let real = live::which("ssh").expect("ssh");
    fake_ssh(
        &bin,
        &format!(
            "exec {} -F /dev/null -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null \"$@\"",
            real.display()
        ),
    );
    let path = live::path_with(&bin);
    let user = std::env::var("USER").unwrap_or_else(|_| "root".to_owned());
    let entry = map(&[
        ("host", s("127.0.0.1")),
        ("user", s(&user)),
        ("port", Value::Int(i64::from(port))),
        ("identity_file", s(&d.join("id").to_string_lossy())),
    ]);
    let cmd = build_ssh_command_with(&entry, &launch(), &env_on(&path)).expect("ssh line");
    let no_agent = OsString::new();
    let mut tab = Term::bash(&cmd, &[("PATH", &path), ("SSH_AUTH_SOCK", &no_agent)], &[]);
    tab.expect("Enter passphrase for key", Duration::from_secs(15));
    tab.close();
    let _ = fs::remove_dir_all(&d);
}
