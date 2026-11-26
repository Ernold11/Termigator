//! Unix implementation of the platform layer.

use std::path::{Path, PathBuf};

use rustix::fs::{Access, access};

/// `access(X_OK)`: the current user may execute `p`.
pub fn can_exec(p: &Path) -> bool {
    access(p, Access::EXEC_OK).is_ok()
}

/// `access(R_OK)`.
pub fn can_read(p: &Path) -> bool {
    access(p, Access::READ_OK).is_ok()
}

/// `access(R_OK | W_OK)`.
pub fn can_read_write(p: &Path) -> bool {
    access(p, Access::READ_OK | Access::WRITE_OK).is_ok()
}

/// `p` is a regular file the current user may execute.
pub fn is_executable(p: &Path) -> bool {
    p.is_file() && can_exec(p)
}

/// First `dir/name` on `PATH` that [`is_executable`].
pub fn find_in_path(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|dir| dir.join(name))
        .find(|p| is_executable(p))
}

fn current_user() -> Option<nix::unistd::User> {
    nix::unistd::User::from_uid(nix::unistd::getuid())
        .ok()
        .flatten()
}

/// Login name of the current user (passwd database).
pub fn current_user_name() -> Option<String> {
    current_user().map(|u| u.name)
}

/// Home directory of the current user (passwd database).
pub fn current_user_home() -> Option<PathBuf> {
    current_user().map(|u| u.dir)
}

/// Home directory of a named user (passwd database).
pub fn user_home_by_name(name: &str) -> Option<PathBuf> {
    nix::unistd::User::from_name(name)
        .ok()
        .flatten()
        .map(|u| u.dir)
}

/// `strerror`-style text for an OS error code (e.g. `Connection refused`).
pub fn errno_text(code: i32) -> String {
    nix::errno::Errno::from_raw(code).desc().to_owned()
}

// --- private files and directories (0600 / 0700) ---

use std::borrow::Cow;
use std::ffi::{OsStr, OsString};
use std::fs::{self, DirBuilder, File, Metadata, OpenOptions};
use std::io;
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt};

/// Makes files opened with `opts` and created by it readable by the owner only (0600).
pub fn private_file(opts: &mut OpenOptions) -> &mut OpenOptions {
    opts.mode(0o600)
}

/// Creates `dir` (and, if `recursive`, its parents) as owner-only (0700); an existing `dir` is an error.
pub fn private_dir_create(dir: &Path, recursive: bool) -> io::Result<()> {
    DirBuilder::new()
        .recursive(recursive)
        .mode(0o700)
        .create(dir)
}

/// Existing file becomes owner-only (0600).
pub fn set_private_file(p: &Path) -> io::Result<()> {
    fs::set_permissions(p, fs::Permissions::from_mode(0o600))
}

/// OpenSSH refuses this private key as "UNPROTECTED PRIVATE KEY FILE": our own file with group or
/// other bits set (`sshkey_perm_ok`).
/// The system-wide OpenSSH client configuration.
pub fn ssh_system_config() -> Option<PathBuf> {
    Some(PathBuf::from("/etc/ssh/ssh_config"))
}

pub fn key_too_open(p: &Path) -> bool {
    fs::metadata(p).is_ok_and(|m| m.uid() == current_uid() && m.mode() & 0o077 != 0)
}

/// Existing directory becomes owner-only (0700).
pub fn set_private_dir(p: &Path) -> io::Result<()> {
    fs::set_permissions(p, fs::Permissions::from_mode(0o700))
}

/// `file` gets the permission bits of `from` (an existing file it replaces).
pub fn copy_permissions(file: &File, from: &Metadata) -> io::Result<()> {
    file.set_permissions(fs::Permissions::from_mode(
        from.permissions().mode() & 0o777,
    ))
}

/// Numeric id of the current user.
pub fn current_uid() -> u32 {
    rustix::process::getuid().as_raw()
}

/// `md` belongs to `uid`.
pub fn owned_by(md: &Metadata, uid: u32) -> bool {
    md.uid() == uid
}

/// Group or others have any access to `md`.
pub fn open_to_others(md: &Metadata) -> bool {
    md.mode() & 0o077 != 0
}

/// `(seconds, nanoseconds)` of the modification time, as `st_mtime` / `st_mtime_nsec`.
pub fn mtime_parts(md: &Metadata) -> (i64, i64) {
    (md.mtime(), md.mtime_nsec())
}

// --- names as bytes ---

pub fn os_str_bytes(s: &OsStr) -> Cow<'_, [u8]> {
    Cow::Borrowed(s.as_bytes())
}

pub fn os_str_from_bytes(b: &[u8]) -> OsString {
    OsStr::from_bytes(b).to_owned()
}

pub fn os_string_from_vec(v: Vec<u8>) -> OsString {
    OsString::from_vec(v)
}

pub fn os_string_into_vec(s: OsString) -> Vec<u8> {
    s.into_vec()
}

// --- single instance: local IPC endpoint and lock ---

pub use std::os::unix::net::{UnixListener as IpcListener, UnixStream as IpcStream};

/// `md` describes an IPC endpoint file (a Unix socket) rather than something else.
pub fn is_ipc_endpoint(md: &Metadata) -> bool {
    use std::os::unix::fs::FileTypeExt;
    md.file_type().is_socket()
}

/// Non-blocking exclusive lock on `f`, held until it is closed. `Ok(false)`: someone else holds it.
pub fn try_lock_exclusive(f: &File) -> io::Result<bool> {
    match rustix::fs::flock(f, rustix::fs::FlockOperation::NonBlockingLockExclusive) {
        Ok(()) => Ok(true),
        Err(rustix::io::Errno::WOULDBLOCK) => Ok(false),
        Err(e) => Err(e.into()),
    }
}

// --- processes ---

use std::process::{Child, Command, ExitStatus};

/// `cmd` becomes the leader of a new process group, so the whole tree can be signalled.
pub fn new_process_group(cmd: &mut Command) -> &mut Command {
    use std::os::unix::process::CommandExt;
    cmd.process_group(0)
}

/// [`new_process_group`]; Windows also keeps a console program's window hidden there.
pub fn background_process(cmd: &mut Command) -> &mut Command {
    new_process_group(cmd)
}

fn killpg(pid: u32, sig: nix::sys::signal::Signal) {
    let pid = nix::unistd::Pid::from_raw(i32::try_from(pid).unwrap_or(i32::MAX));
    let _ = nix::sys::signal::killpg(pid, sig);
}

/// Asks the group led by `pid` to stop (`SIGTERM`).
pub fn group_terminate(pid: u32) {
    killpg(pid, nix::sys::signal::Signal::SIGTERM);
}

/// Kills the group led by `pid` (`SIGKILL`).
pub fn group_kill(pid: u32) {
    killpg(pid, nix::sys::signal::Signal::SIGKILL);
}

/// Has the leader exited? Peeks with `WNOWAIT`, leaving the zombie so its pid stays ours.
pub fn has_exited(child: &mut Child) -> bool {
    use nix::sys::wait::{Id, WaitPidFlag, WaitStatus, waitid};
    let pid = nix::unistd::Pid::from_raw(i32::try_from(child.id()).unwrap_or(i32::MAX));
    let flags = WaitPidFlag::WEXITED | WaitPidFlag::WNOHANG | WaitPidFlag::WNOWAIT;
    !matches!(waitid(Id::Pid(pid), flags), Ok(WaitStatus::StillAlive))
}

/// Signal that ended the process, if it was one.
pub fn exit_signal(s: &ExitStatus) -> Option<i32> {
    use std::os::unix::process::ExitStatusExt;
    s.signal()
}

/// A successful exit status, for when waiting itself failed.
pub fn exit_status_success() -> ExitStatus {
    use std::os::unix::process::ExitStatusExt;
    ExitStatus::from_raw(0)
}

/// `EIO` on Linux and the BSDs: the pty reports a closed child side that way.
const EIO: i32 = 5;

/// Our duplicate of the PTY master for reads and writes (portable-pty's writer sends EOF on drop).
pub struct PtyIo(filedescriptor::FileDescriptor);

impl PtyIo {
    pub fn open(master: &dyn portable_pty::MasterPty) -> Result<Self, String> {
        let fd = master
            .as_raw_fd()
            .ok_or("portable-pty master has no file descriptor")?;
        filedescriptor::FileDescriptor::dup(&fd)
            .map(Self)
            .map_err(|e| e.to_string())
    }

    /// `Ok(0)` once the child side is closed.
    pub fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        use io::Read;
        match self.0.read(buf) {
            Err(e) if e.raw_os_error() == Some(EIO) => Ok(0),
            other => other,
        }
    }

    pub fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        use io::Write;
        self.0.write(data)
    }

    pub fn write_all(&mut self, data: &[u8]) -> io::Result<()> {
        use io::Write;
        self.0.write_all(data)
    }

    /// For the GUI's notifier; valid while `self` lives.
    pub fn as_fd(&self) -> std::os::fd::BorrowedFd<'_> {
        std::os::fd::AsFd::as_fd(&self.0)
    }
}

/// The process on a pty. portable-pty's own `ChildKiller` is not used: only this `Child` reaps
/// the process, so its pid cannot be reused while we signal it.
pub struct PtyChild(Child);

impl PtyChild {
    pub fn from_portable(
        child: Box<dyn portable_pty::Child + Send + Sync>,
    ) -> Result<Self, String> {
        child
            .into_any()
            .downcast::<Child>()
            .map(|c| Self(*c))
            .map_err(|_| "portable-pty returned a non-process child".to_owned())
    }

    pub fn id(&self) -> u32 {
        self.0.id()
    }

    pub fn try_wait(&mut self) -> io::Result<Option<ExitStatus>> {
        self.0.try_wait()
    }

    pub fn kill(&mut self) -> io::Result<()> {
        self.0.kill()
    }

    pub fn wait(&mut self) -> io::Result<ExitStatus> {
        self.0.wait()
    }
}

/// Machine name (`gethostname`).
pub fn hostname() -> Option<String> {
    let h = fs::read_to_string("/proc/sys/kernel/hostname").ok()?;
    Some(h.trim().to_owned()).filter(|h| !h.is_empty())
}

/// fsync of a directory, so a rename inside it survives a crash.
pub fn sync_dir(dir: &Path) -> io::Result<()> {
    File::open(dir)?.sync_all()
}

/// Feed mode (the GUI is fed by a reader thread, not by a notifier on the master descriptor):
/// always on Windows; on Unix only with `TERMIGATOR_PTY_FEED` set, so the mode can be tested there.
pub fn use_feed() -> bool {
    std::env::var_os("TERMIGATOR_PTY_FEED").is_some_and(|v| !v.is_empty())
}

/// Installed WSL distributions: none outside Windows.
pub fn wsl_distributions() -> Vec<String> {
    Vec::new()
}

/// No registry outside Windows (see the Windows `registry_keys`).
pub fn registry_keys(_hkcu_path: &str) -> Vec<super::RegKeyValues> {
    Vec::new()
}
