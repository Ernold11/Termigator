//! Windows: background processes (tunnels, transfers, probes) end with Termigator, also when it
//! crashes or is killed. They go into a Job Object with KILL_ON_JOB_CLOSE; the job's
//! only handle is this process's, so Windows kills them when the process is gone. The app itself
//! stays outside the job: programs it opens through the shell (a file from the Files panel) live on.

#[cfg(windows)]
pub fn install() {
    use std::sync::OnceLock;
    use windows_sys::Win32::Foundation::HANDLE;
    use windows_sys::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
        JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
        SetInformationJobObject,
    };

    // The handle as an integer: HANDLE is a raw pointer, which a static cannot hold.
    static JOB: OnceLock<usize> = OnceLock::new();

    fn assign(child: &std::process::Child) {
        use std::os::windows::io::AsRawHandle;
        if let Some(&job) = JOB.get() {
            // SAFETY: `job` is a live job handle (never closed) and the child's handle is valid
            // while `child` is borrowed. A failure leaves the process outside the job, as before.
            unsafe { AssignProcessToJobObject(job as HANDLE, child.as_raw_handle() as HANDLE) };
        }
    }

    // SAFETY: no name, default security; the result is checked before use.
    let job = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
    if job.is_null() {
        return;
    }
    // SAFETY: a plain C struct for which all-zero is the documented "no limits" value.
    let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { std::mem::zeroed() };
    info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
    // SAFETY: `info` is the structure this information class expects, with its exact size.
    let ok = unsafe {
        SetInformationJobObject(
            job,
            JobObjectExtendedLimitInformation,
            (&raw const info).cast(),
            std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
        )
    };
    if ok == 0 {
        return;
    }
    let _ = JOB.set(job as usize);
    termigator_core::proc::runner::set_spawn_hook(assign);
}

#[cfg(not(windows))]
pub fn install() {}

#[cfg(all(test, windows))]
mod tests {
    use std::process::Command as Std;

    const MARK: &str = "7731"; // ping count that only these tests use
    const PTY_MARK: &str = "7733";

    fn ping_running() -> bool {
        ping_with_mark_running(MARK)
    }

    fn ping_with_mark_running(mark: &str) -> bool {
        let out = Std::new("powershell")
            .args([
                "-NoProfile",
                "-Command",
                &format!(
                    "@(Get-CimInstance Win32_Process -Filter \"Name='PING.EXE'\" | \
                     Where-Object {{ $_.CommandLine -match '-n {mark}' }}).Count"
                ),
            ])
            .output()
            .expect("powershell");
        String::from_utf8_lossy(&out.stdout).trim() != "0"
    }

    #[test]
    fn background_processes_end_with_the_app() {
        if std::env::var_os("TG_JOB_CHILD").is_some() {
            // The "app": install the job, start a long ping, and vanish without cleaning up.
            super::install();
            let cmd =
                termigator_core::proc::runner::Command::new(["ping", "-n", MARK, "127.0.0.1"]);
            let running = termigator_core::proc::runner::spawn(cmd).expect("spawn");
            std::thread::sleep(std::time::Duration::from_millis(1500));
            println!("STARTED={}", ping_running());
            std::mem::forget(running);
            std::process::exit(0);
        }
        let exe = std::env::current_exe().expect("exe");
        let out = Std::new(exe)
            .args([
                "--exact",
                "job_windows::tests::background_processes_end_with_the_app",
                "--nocapture",
            ])
            .env("TG_JOB_CHILD", "1")
            .output()
            .expect("child");
        let text = String::from_utf8_lossy(&out.stdout);
        assert!(text.contains("STARTED=true"), "ping never started:\n{text}");
        std::thread::sleep(std::time::Duration::from_millis(1500));
        let left = ping_running();
        if left {
            let _ = Std::new("taskkill")
                .args(["/F", "/IM", "PING.EXE"])
                .output();
        }
        assert!(!left, "the ping outlived its parent");
    }

    /// A terminal session (ConPTY) of an app that vanishes without cleaning up.
    #[test]
    fn terminal_sessions_end_with_the_app() {
        if std::env::var_os("TG_JOB_PTY_CHILD").is_some() {
            super::install();
            let spec = termigator_core::proc::pty::Spawn {
                argv: ["ping", "-n", PTY_MARK, "127.0.0.1"]
                    .iter()
                    .map(std::ffi::OsString::from)
                    .collect(),
                env: std::env::vars_os().collect(),
                cwd: std::env::temp_dir(),
                size: termigator_core::proc::pty::Size { cols: 80, rows: 24 },
            };
            let pty = termigator_core::proc::pty::Pty::spawn(&spec).expect("pty");
            std::thread::sleep(std::time::Duration::from_millis(1500));
            println!("STARTED={}", ping_with_mark_running(PTY_MARK));
            std::mem::forget(pty);
            std::process::exit(0);
        }
        let exe = std::env::current_exe().expect("exe");
        let out = Std::new(exe)
            .args([
                "--exact",
                "job_windows::tests::terminal_sessions_end_with_the_app",
                "--nocapture",
            ])
            .env("TG_JOB_PTY_CHILD", "1")
            .output()
            .expect("child");
        let text = String::from_utf8_lossy(&out.stdout);
        assert!(text.contains("STARTED=true"), "ping never started:\n{text}");
        std::thread::sleep(std::time::Duration::from_millis(2500));
        let left = ping_with_mark_running(PTY_MARK);
        if left {
            let _ = Std::new("taskkill")
                .args(["/F", "/IM", "PING.EXE"])
                .output();
        }
        assert!(!left, "the terminal session outlived its parent");
    }
}
