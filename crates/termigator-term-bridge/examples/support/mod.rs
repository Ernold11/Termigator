//! Shared by the Qt examples: a shell on a pty and the pty attached to a session the way the app
//! attaches it — by descriptor on Unix, in feed mode where there is none (ConPTY;
//! `TERMIGATOR_PTY_FEED=1` forces feed mode on Unix too).

use std::{
    collections::BTreeMap,
    ffi::OsString,
    io::{Read, Write},
    path::PathBuf,
    sync::{Arc, Mutex, mpsc},
    thread,
};

use termigator_core::proc::pty::{Pty, Size, Spawn};

/// A shell with this process's environment: `$SHELL -l` on Unix, `%COMSPEC%` on Windows.
pub fn shell(size: Size) -> Pty {
    let mut env: BTreeMap<OsString, OsString> = std::env::vars_os().collect();
    env.insert("TERM".into(), "xterm-256color".into());
    env.insert("COLORTERM".into(), "truecolor".into());
    env.insert("TERMIGATOR_PTY".into(), "1".into());
    env.entry("LANG".into()).or_insert_with(|| "C.UTF-8".into());
    #[cfg(windows)]
    let argv: Vec<OsString> = vec![
        env.get(&OsString::from("COMSPEC"))
            .cloned()
            .unwrap_or_else(|| "cmd.exe".into()),
    ];
    #[cfg(not(windows))]
    let argv: Vec<OsString> = {
        let shell = std::env::var_os("SHELL").unwrap_or_else(|| "/bin/bash".into());
        vec![shell, "-l".into()]
    };
    let home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map_or_else(|| PathBuf::from("/"), PathBuf::from);
    Pty::spawn(&Spawn {
        argv,
        env,
        cwd: home,
        size,
    })
    .expect("spawn the shell")
}

/// Where the bridge's `PtyEvents::write` sends input in feed mode; empty in descriptor mode.
#[derive(Clone, Default)]
pub struct Writer(Arc<Mutex<Option<mpsc::Sender<Vec<u8>>>>>);

impl Writer {
    /// Input typed in a feed-mode session; dropped once the pty is gone.
    pub fn send(&self, data: &[u8]) {
        let tx = self.0.lock().ok().and_then(|guard| guard.clone());
        if let Some(tx) = tx {
            let _ = tx.send(data.to_vec());
        }
    }
}

/// Attaches `pty` to the live session `id`: the same split as `crates/termigator/src/platform.rs`
/// (the app has one more layer of bookkeeping, not needed here).
pub fn attach(id: u64, pty: &mut Pty, writer: &Writer) -> Result<(), String> {
    if !termigator_core::platform::use_feed() {
        #[cfg(unix)]
        return termigator_term_bridge::attach_ghostty_pty(id, pty.master_fd())
            .map_err(|e| e.what().to_owned());
        #[cfg(not(unix))]
        return Err("this platform has no pty descriptor".to_owned());
    }
    let (mut reader, mut out) = pty.take_streams().map_err(|e| e.to_string())?;
    termigator_term_bridge::attach_ghostty_pty_feed(id).map_err(|e| e.what().to_owned())?;
    thread::spawn(move || {
        let mut buf = vec![0u8; 64 * 1024];
        loop {
            match reader.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => termigator_term_bridge::feed_ghostty_pty(id, &buf[..n]),
            }
        }
        termigator_term_bridge::eof_ghostty_pty(id);
    });
    let (tx, rx) = mpsc::channel::<Vec<u8>>();
    thread::spawn(move || {
        for data in rx {
            if out.write_all(&data).is_err() {
                break;
            }
            let _ = out.flush();
        }
    });
    if let Ok(mut guard) = writer.0.lock() {
        *guard = Some(tx);
    }
    Ok(())
}
