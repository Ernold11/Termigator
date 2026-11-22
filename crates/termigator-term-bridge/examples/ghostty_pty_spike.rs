//! Spike: real pty round-trip through the engine.
//!
//! Spawns `cat` on a pty, feeds every byte it returns into the Ghostty engine and measures the
//! time from writing one byte to receiving it back (tty echo plus EOF-free `cat` output). This is
//! the input latency the plan budgets at < 50 ms; the production session path adds Qt event-loop
//! and paint time, which the interactive spike window shows separately.
//!
//! `cargo run --release --example ghostty_pty_spike [-- --rounds 100]`
//!
//! Unix only: it clones the master descriptor and writes through `Pty::write`, which has no
//! Windows counterpart (feed mode does the I/O through the streams instead).
#![cfg_attr(not(unix), allow(dead_code, unused_imports, unused_variables))]

use std::{
    collections::BTreeMap,
    ffi::OsString,
    fs::File,
    io::Read as _,
    sync::mpsc,
    time::{Duration, Instant},
};

use termigator_core::proc::pty::{Pty, Size, Spawn};
use termigator_term_bridge::engine::Engine;

fn arg(name: &str) -> Option<String> {
    let args: Vec<String> = std::env::args().collect();
    let i = args.iter().position(|arg| arg == name)?;
    args.get(i + 1).cloned()
}

#[cfg(unix)]
fn main() {
    run();
}

#[cfg(not(unix))]
fn main() {
    eprintln!("ghostty_pty_spike: needs a Unix pty (no master descriptor on Windows)");
}

#[cfg(unix)]
fn run() {
    let rounds: usize = arg("--rounds")
        .and_then(|value| value.parse().ok())
        .unwrap_or(100);
    let env: BTreeMap<OsString, OsString> = [
        ("PATH".into(), "/usr/bin:/bin".into()),
        ("LANG".into(), "C.UTF-8".into()),
        ("TERM".into(), "xterm-256color".into()),
    ]
    .into();
    let mut pty = Pty::spawn(&Spawn {
        argv: vec!["cat".into()],
        env,
        cwd: "/".into(),
        size: Size { cols: 80, rows: 24 },
    })
    .expect("spawn cat");
    let fd = pty
        .master_fd()
        .try_clone_to_owned()
        .expect("clone the pty master");
    let (tx, rx) = mpsc::channel::<Vec<u8>>();
    let reader = std::thread::spawn(move || {
        let mut file = File::from(fd);
        let mut buf = [0u8; 8192];
        loop {
            match file.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    if tx.send(buf[..n].to_vec()).is_err() {
                        break;
                    }
                }
            }
        }
    });

    let mut engine = Engine::new(80, 24, Some(1_000), (8, 16)).expect("engine");
    std::thread::sleep(Duration::from_millis(200));
    while let Ok(data) = rx.try_recv() {
        engine.feed(&data);
    }

    let mut latencies = Vec::with_capacity(rounds);
    for _ in 0..rounds {
        let start = Instant::now();
        if pty.write_all(b"x").is_err() {
            break;
        }
        let mut answered = false;
        while start.elapsed() < Duration::from_secs(1) {
            match rx.recv_timeout(Duration::from_millis(50)) {
                Ok(data) => {
                    engine.feed(&data);
                    if data.contains(&b'x') {
                        answered = true;
                        break;
                    }
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
        }
        if !answered {
            eprintln!("no echo after {rounds} rounds");
            break;
        }
        latencies.push(start.elapsed());
    }

    let _ = pty.close();
    let _ = reader.join();
    let snapshot = engine.snapshot().expect("snapshot");
    let text: String = snapshot.row_data[0]
        .cells
        .iter()
        .map(|cell| cell.text.as_str())
        .collect();

    latencies.sort_unstable();
    let percentile = |p: usize| -> f64 {
        if latencies.is_empty() {
            return f64::NAN;
        }
        let index = (latencies.len() - 1) * p / 100;
        latencies[index].as_secs_f64() * 1000.0
    };
    println!(
        "PTY_LATENCY rounds={} min_ms={:.2} median_ms={:.2} p95_ms={:.2} max_ms={:.2} \
         echo_seen={} first_row={:?}",
        latencies.len(),
        percentile(0),
        percentile(50),
        percentile(95),
        percentile(100),
        !latencies.is_empty(),
        text
    );
}
