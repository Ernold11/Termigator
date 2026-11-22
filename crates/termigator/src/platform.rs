//! How the GUI gets its terminal I/O (ADR-3).
//!
//! Two modes, chosen by [`feeds`]:
//!
//! - **descriptor** (Unix, the default): the bridge duplicates the pty master and reads/writes it
//!   with a `QSocketNotifier`; the app only hands the fd over here ([`attach`]).
//! - **feed** (always on Windows, `TERMIGATOR_PTY_FEED=1` on Unix so the mode can be tested
//!   there): ConPTY has no descriptor to poll, so a thread here reads the pty and feeds the
//!   session ([`termigator_term_bridge::feed_ghostty_pty`]), the bridge hands input back through
//!   `PtyEvents::write` ([`write`] queues it, never blocks the GUI thread) and resizes from the
//!   GUI thread.
//!
//! The writer of a session is kept in a registry by `sessionId`, so `on_write` can find it. That
//! writer is what keeps the reader alive on Windows: ConPTY does not close the output pipe when
//! the child exits, so [`detach`] (tab closed or child reaped) drops the sender, the writer thread
//! closes its half and only then does the reader see EOF.

use std::{collections::HashMap, io, sync::Mutex, sync::OnceLock, sync::mpsc, thread};

use termigator_core::proc::pty::Pty;

type Writers = Mutex<HashMap<u64, mpsc::Sender<Vec<u8>>>>;

fn writers() -> &'static Writers {
    static WRITERS: OnceLock<Writers> = OnceLock::new();
    WRITERS.get_or_init(|| Mutex::new(HashMap::new()))
}

/// A poisoned registry is still a usable map (a panic in a thread must not lose the writer of
/// another session), so the guard is recovered instead of unwrapped.
fn lock(map: &Writers) -> std::sync::MutexGuard<'_, HashMap<u64, mpsc::Sender<Vec<u8>>>> {
    map.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Feed mode: the GUI is fed by threads here, not by a notifier on the master descriptor.
pub(crate) fn feeds() -> bool {
    termigator_core::platform::use_feed()
}

/// Hands the pty of `id` to the bridge (descriptor mode) or starts the feed threads.
pub(crate) fn attach(id: u64, pty: &mut Pty) -> Result<(), String> {
    if !feeds() {
        #[cfg(unix)]
        return termigator_term_bridge::attach_ghostty_pty(id, pty.master_fd())
            .map_err(|e| e.what().to_owned());
        #[cfg(not(unix))]
        return Err("a pty has no descriptor on this platform".to_owned());
    }
    attach_feed(id, pty)
}

fn attach_feed(id: u64, pty: &mut Pty) -> Result<(), String> {
    let (mut reader, mut writer) = pty.take_streams().map_err(|e| e.to_string())?;
    termigator_term_bridge::attach_ghostty_pty_feed(id).map_err(|e| e.what().to_owned())?;
    thread::Builder::new()
        .name(format!("pty-read-{id}"))
        .spawn(move || {
            use io::Read;
            let mut buf = vec![0u8; 64 * 1024];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => termigator_term_bridge::feed_ghostty_pty(id, &buf[..n]),
                    Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                    // EIO on Unix, a broken ConPTY pipe: the child side is gone.
                    Err(_) => break,
                }
            }
            termigator_term_bridge::eof_ghostty_pty(id);
        })
        .map_err(|e| format!("cannot start the pty reader thread: {e}"))?;
    let (tx, rx) = mpsc::channel::<Vec<u8>>();
    thread::Builder::new()
        .name(format!("pty-write-{id}"))
        .spawn(move || {
            use io::Write;
            for data in rx {
                if writer.write_all(&data).is_err() {
                    break;
                }
                let _ = writer.flush();
            }
        })
        .map_err(|e| format!("cannot start the pty writer thread: {e}"))?;
    // Replaces the sender of a previous pty of the same session, which ends its writer thread.
    lock(writers()).insert(id, tx);
    Ok(())
}

/// Ends the session's writer thread (its sender is dropped, so it finishes and closes its half);
/// that is also what makes the reader see EOF on Windows, where ConPTY keeps the pipe open after
/// the child exits. Called when the tab closes and when the child is reaped.
pub(crate) fn detach(id: u64) {
    lock(writers()).remove(&id);
}

/// Input of a feed-mode session, from the bridge (GUI thread). Never blocks: the writer thread
/// does the writing, and a closed session simply drops the bytes.
pub(crate) fn write(id: u64, data: &[u8]) {
    // Both the lookup and the send happen under the lock: `detach` may drop the sender between
    // them, and a send on a dropped channel just returns Err.
    let _ = lock(writers()).get(&id).map(|tx| tx.send(data.to_vec()));
}
