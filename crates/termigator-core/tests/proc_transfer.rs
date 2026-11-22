//! Transfer workers with stub scp/sftp and, when available, the real binaries.
// Unix-only for now: uses modes, symlinks, sockets or sh stubs.
#![cfg(unix)]
mod live;

use std::{
    fs,
    path::{Path, PathBuf},
    sync::{Mutex, MutexGuard, PoisonError},
    time::{Duration, Instant},
};

use termigator_core::{
    config::doc::Value,
    proc::transfer::{Event, TransferHandle, start, start_resume},
    proto::{
        scp::{SftpResume, sftp_resume_argv_with},
        ssh::Env,
    },
    remote::transfers::TransferJob,
};

/// Writing an executable and forking in a sibling test can give ETXTBSY; run one test at a time.
static SERIAL: Mutex<()> = Mutex::new(());

fn serial() -> MutexGuard<'static, ()> {
    SERIAL.lock().unwrap_or_else(PoisonError::into_inner)
}

fn tmp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("termigator-r3-{}-{name}", std::process::id()));
    let _ = fs::remove_dir_all(&d);
    fs::create_dir_all(&d).expect("mkdir");
    d
}

fn script(dir: &Path, name: &str, body: &str) {
    let p = dir.join(name);
    live::script(&p, &format!("#!/bin/sh\n{body}"));
}

fn job(argv: &[&str], path_dir: &Path) -> TransferJob {
    TransferJob::new(
        false,
        PathBuf::from("/l"),
        b"/r".to_vec(),
        b"x".to_vec(),
        argv.iter().map(|a| a.as_bytes().to_vec()).collect(),
        vec![(
            "PATH".to_owned(),
            termigator_core::vault::Secret::new(
                format!("{}:/usr/bin:/bin", path_dir.display()).into_bytes(),
            ),
        )],
    )
}

fn collect(h: &TransferHandle) -> Vec<Event> {
    let mut out = Vec::new();
    while let Ok(ev) = h.events().recv_timeout(Duration::from_secs(30)) {
        out.push(ev);
    }
    out
}

fn percents(evs: &[Event]) -> Vec<u8> {
    evs.iter()
        .filter_map(|e| match e {
            Event::Progress { percent, .. } => Some(*percent),
            _ => None,
        })
        .collect()
}

const METER: &str = "printf 'big.bin    0%%    0     0.0KB/s   --:-- ETA\\r'\n\
    printf 'big.bin   45%%  171MB  12.3MB/s   00:05 ETA\\r'\n\
    printf 'big.bin  100%%  381MB -2097151.-9KB/s   00:00\\n'\n";

#[test]
fn stub_scp_progress_and_done() {
    let _g = serial();
    let d = tmp("ok");
    script(&d, "scp", &format!("{METER}exit 0\n"));
    let evs = collect(&start(job(&["scp", "a", "b"], &d)));
    assert_eq!(percents(&evs), [0, 45, 100]);
    assert_eq!(evs.last(), Some(&Event::Done));
    assert!(matches!(
        &evs[1],
        Event::Progress { size, speed, eta, .. } if size == "171MB" && speed == "12.3MB/s" && eta == "00:05"
    ));
    assert!(matches!(&evs[2], Event::Progress { speed, .. } if speed.is_empty()));
}

#[test]
fn stub_scp_failure_reports_last_text() {
    let _g = serial();
    let d = tmp("fail");
    script(
        &d,
        "scp",
        "echo 'scp: /x: No such file or directory'\nexit 1\n",
    );
    let evs = collect(&start(job(&["scp", "a", "b"], &d)));
    assert_eq!(
        evs.last(),
        Some(&Event::Failed {
            message: "scp: /x: No such file or directory".to_owned()
        })
    );
    script(&d, "scp", "exit 3\n");
    let evs = collect(&start(job(&["scp", "a", "b"], &d)));
    assert_eq!(
        evs.last(),
        Some(&Event::Failed {
            message: "exit 3".to_owned()
        })
    );
}

#[test]
fn missing_program_fails() {
    let _g = serial();
    let d = tmp("missing");
    let evs = collect(&start(job(&["no-such-scp-r3"], &d)));
    assert!(matches!(evs.as_slice(), [Event::Failed { .. }]));
}

fn alive(pid: &str) -> bool {
    match fs::read_to_string(format!("/proc/{pid}/stat")) {
        Ok(s) => !s.contains(") Z "),
        Err(_) => false,
    }
}

#[test]
fn cancel_kills_the_whole_group() {
    let _g = serial();
    let d = tmp("cancel");
    let pidfile = d.join("bg.pid");
    script(
        &d,
        "scp",
        &format!(
            "printf 'big.bin    1%%  1MB  1MB/s   00:50 ETA\\r'\nsleep 30 &\necho $! > {}\nwait\n",
            pidfile.display()
        ),
    );
    let h = start(job(&["scp", "a", "b"], &d));
    // first progress event: the stub is running, the pid file follows immediately
    assert!(matches!(
        h.events().recv_timeout(Duration::from_secs(10)),
        Ok(Event::Progress { .. })
    ));
    let t = Instant::now();
    while !pidfile.exists() || fs::read_to_string(&pidfile).unwrap_or_default().is_empty() {
        assert!(t.elapsed() < Duration::from_secs(10), "no pid file");
        std::thread::sleep(Duration::from_millis(20));
    }
    let bg = fs::read_to_string(&pidfile).expect("pid").trim().to_owned();
    assert!(alive(&bg));
    h.cancel();
    let evs = collect(&h);
    assert_eq!(evs.last(), Some(&Event::Cancelled));
    let t = Instant::now();
    while alive(&bg) {
        assert!(
            t.elapsed() < Duration::from_secs(5),
            "background child survived"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn cancel_escalates_to_kill_when_term_is_ignored() {
    let _g = serial();
    let d = tmp("kill");
    script(
        &d,
        "scp",
        "trap '' TERM\nprintf 'a    1%%  1MB  1MB/s   00:50 ETA\\r'\nwhile :; do sleep 1; done\n",
    );
    let h = start(job(&["scp", "a", "b"], &d));
    assert!(matches!(
        h.events().recv_timeout(Duration::from_secs(10)),
        Ok(Event::Progress { .. })
    ));
    std::thread::sleep(Duration::from_millis(200)); // let the trap be installed
    h.cancel();
    assert_eq!(collect(&h).last(), Some(&Event::Cancelled));
}

#[test]
fn cancel_before_start_still_cancels() {
    let _g = serial();
    let d = tmp("early");
    script(&d, "scp", "sleep 30\n");
    let h = start(job(&["scp", "a", "b"], &d));
    h.cancel();
    assert_eq!(collect(&h).last(), Some(&Event::Cancelled));
}

fn env_none() -> Env {
    Env {
        which: Box::new(|_| None),
        control_dir: None,
        home: Some("/nonexistent-home".to_owned()),
    }
}

fn launch_no_control() -> Value {
    Value::Map(vec![(
        Value::Str("ssh_control_master".to_owned()),
        Value::Bool(false),
    )])
}

fn conn() -> Value {
    Value::Map(vec![
        (Value::Str("host".to_owned()), Value::Str("h".to_owned())),
        (Value::Str("user".to_owned()), Value::Str("u".to_owned())),
    ])
}

/// Builds the sftp plan for `local` <- `remote`, inserting `-S <fake_ssh>` after `sftp`.
fn plan(
    local: &Path,
    remote: &Path,
    batch: &str,
    fake_ssh: Option<&Path>,
) -> Result<(Vec<String>, String), String> {
    let (mut argv, text) = sftp_resume_argv_with(
        &conn(),
        &launch_no_control(),
        &SftpResume {
            local_path: &local.to_string_lossy(),
            remote_path: &remote.to_string_lossy(),
            batch_path: batch,
            recursive: false,
        },
        &env_none(),
    )?;
    if let Some(s) = fake_ssh {
        let i = argv.iter().position(|a| a == "sftp").ok_or("no sftp")?;
        argv.splice(
            i + 1..i + 1,
            ["-S".to_owned(), s.to_string_lossy().into_owned()],
        );
    }
    Ok((argv, text))
}

#[test]
fn resume_writes_private_batch_and_removes_it() {
    let _g = serial();
    let d = tmp("resume");
    let rec = d.join("rec");
    script(
        &d,
        "sftp",
        &format!(
            "for a; do prev=$last; last=$a; if [ \"$prev\" = -b ]; then b=$a; fi; done\n\
             cp \"$b\" {rec}.batch\nstat -c %a \"$b\" > {rec}.mode\necho \"$b\" > {rec}.path\nexit 0\n",
            rec = rec.display()
        ),
    );
    let h = start_resume(job(&[], &d), {
        let (local, remote) = (d.join("local"), d.join("remote"));
        move |batch| plan(&local, &remote, batch, None)
    });
    assert_eq!(collect(&h).last(), Some(&Event::Done));
    let text = fs::read_to_string(d.join("rec.batch")).expect("batch");
    assert!(text.starts_with("progress\nreget -p \""), "{text}");
    assert_eq!(
        fs::read_to_string(d.join("rec.mode")).expect("mode").trim(),
        "600"
    );
    let path = fs::read_to_string(d.join("rec.path")).expect("path");
    assert!(!Path::new(path.trim()).exists(), "batch file left behind");
}

#[test]
fn resume_build_error_fails_without_spawning() {
    let _g = serial();
    let d = tmp("resume-err");
    let h = start_resume(job(&[], &d), |_| {
        Err("Could not build sftp command.".to_owned())
    });
    assert_eq!(
        collect(&h),
        [Event::Failed {
            message: "Could not build sftp command.".to_owned()
        }]
    );
}

#[test]
fn resume_of_a_hostile_name_never_starts_sftp_k15() {
    // The server-sent name "x\n!touch pwned" would be a second line (a local command) in the batch.
    let _g = serial();
    let d = tmp("resume-k15");
    script(&d, "sftp", &format!("touch {}/sftp-ran\n", d.display()));
    let h = start_resume(job(&[], &d), {
        let (local, remote) = (d.join("local"), d.join("x\n!touch pwned"));
        move |batch| plan(&local, &remote, batch, None)
    });
    assert_eq!(
        collect(&h),
        [Event::Failed {
            message: termigator_core::proto::scp::LINE_BREAK_MSG.to_owned()
        }]
    );
    assert!(!d.join("sftp-ran").exists(), "sftp was started");
    assert!(!d.join("pwned").exists());
}

// ---- real binaries ----

fn which(bin: &str) -> Option<PathBuf> {
    std::env::var_os("PATH").and_then(|p| {
        std::env::split_paths(&p)
            .map(|d| d.join(bin))
            .find(|p| p.is_file())
    })
}

fn sftp_server() -> Option<PathBuf> {
    std::env::var_os("TERMIGATOR_TEST_SFTP_SERVER")
        .map(PathBuf::from)
        .into_iter()
        .chain(
            [
                "/usr/lib/ssh/sftp-server",
                "/usr/lib/openssh/sftp-server",
                "/usr/libexec/openssh/sftp-server",
                "/usr/libexec/sftp-server",
            ]
            .map(PathBuf::from),
        )
        .find(|p| p.is_file())
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

#[test]
fn real_scp_draws_its_meter_under_the_pty() {
    let _g = serial();
    let Some(scp) = which("scp") else {
        live::skip("real_scp_draws_its_meter_under_the_pty", "needs scp");
        return;
    };
    let d = tmp("real-scp");
    script(&d, "ssh", "for last; do :; done\nexec sh -c \"$last\"\n");
    let src = d.join("src.bin");
    fs::write(&src, vec![0u8; 5_000_000]).expect("src");
    let dst = d.join("dst.bin");
    let j = job(
        &[
            scp.to_str().expect("utf8"),
            "-O",
            "-S",
            d.join("ssh").to_str().expect("utf8"),
            src.to_str().expect("utf8"),
            &format!("u@h:{}", dst.display()),
        ],
        &d,
    );
    let evs = collect(&start(j));
    assert_eq!(evs.last(), Some(&Event::Done), "{evs:?}");
    assert_eq!(percents(&evs).last(), Some(&100));
    assert_eq!(fs::metadata(&dst).expect("dst").len(), 5_000_000);
}

#[test]
fn real_sftp_resume_continues_a_partial_download() {
    let _g = serial();
    let (Some(srv), Some(sftp)) = (sftp_server(), which("sftp")) else {
        live::skip(
            "real_sftp_resume_continues_a_partial_download",
            "needs sftp and an sftp-server binary",
        );
        return;
    };
    let d = tmp("real-sftp");
    let remote_dir = d.join("remote dir [1]");
    fs::create_dir_all(&remote_dir).expect("mkdir");
    let src = remote_dir.join("big [x].bin");
    let data = pseudo_random(6_000_000);
    fs::write(&src, &data).expect("src");
    let local = d.join("big [x].bin");
    fs::write(&local, &data[..2_500_000]).expect("partial"); // an interrupted download
    script(&d, "ssh", &format!("exec {}\n", srv.display()));
    let h = start_resume(job(&[], &d), {
        let (local, src, ssh, sftp) = (local.clone(), src, d.join("ssh"), sftp);
        move |batch| {
            let (mut argv, text) = plan(&local, &src, batch, Some(&ssh))?;
            argv[0] = sftp.to_string_lossy().into_owned();
            Ok((argv, text))
        }
    });
    let evs = collect(&h);
    assert_eq!(evs.last(), Some(&Event::Done), "{evs:?}");
    assert_eq!(fs::read(&local).expect("local"), data);
    let first = percents(&evs).first().copied();
    assert!(
        first.is_some_and(|p| p >= 40),
        "restarted, not resumed: {first:?}"
    );
}
