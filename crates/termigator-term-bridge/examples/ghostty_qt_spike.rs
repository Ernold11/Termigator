//! Opens the spike window on a real pty owned by Rust.
//!
//! `cargo run --release --example ghostty_qt_spike` spawns `$SHELL -l`, attaches the pty to the
//! QML `GhosttySession { sessionId: 1 }` and renders it with the spike view. `--source <file>` draws
//! a capture instead; `--run <command>` sends one command; `--out <png>` grabs and quits.
//!
//! `QT_QPA_PLATFORM=offscreen QT_QUICK_BACKEND=software \
//!  cargo run --release --example ghostty_qt_spike -- --run "htop -d 10" --out /tmp/shot.png`

use std::{cell::RefCell, rc::Rc};

use cxx_qt_lib::{QGuiApplication, QQmlApplicationEngine, QString, QUrl};
use termigator_core::proc::pty::{Pty, Size};
use termigator_term_bridge::{PtyEvents, set_pty_events};

#[path = "support/mod.rs"]
mod support;

struct Reaper {
    pty: Rc<RefCell<Option<Pty>>>,
    writer: support::Writer,
}

impl PtyEvents for Reaper {
    fn output(&mut self, _id: u64, _data: &[u8]) {}

    fn write(&mut self, _id: u64, data: &[u8]) {
        self.writer.send(data);
    }

    fn eof(&mut self, _id: u64) {
        if let Some(mut pty) = self.pty.borrow_mut().take() {
            let _ = pty.try_wait();
        }
    }
}

pub fn main() {
    let args: Vec<String> = std::env::args().collect();
    let shell_mode = !args.iter().any(|arg| arg == "--source");
    termigator_term_bridge::register_spike_types();
    let mut app = QGuiApplication::new();
    let mut engine = QQmlApplicationEngine::new();
    let qml = QString::from(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/examples/ghostty_spike.qml"
    ));
    let (Some(app), Some(mut engine)) = (app.as_mut(), engine.as_mut()) else {
        eprintln!("cannot create the Qt application");
        std::process::exit(1);
    };
    engine
        .as_mut()
        .on_object_creation_failed(|_, _| std::process::exit(1))
        .release();
    engine.load(&QUrl::from_local_file(&qml));

    if shell_mode {
        // The QML session exists after load; attach the Rust-owned pty to it.
        let writer = support::Writer::default();
        let pty = Rc::new(RefCell::new(Some(support::shell(Size {
            cols: 118,
            rows: 38,
        }))));
        set_pty_events(Box::new(Reaper {
            pty: Rc::clone(&pty),
            writer: writer.clone(),
        }));
        {
            let mut borrowed = pty.borrow_mut();
            let Some(attached) = borrowed.as_mut() else {
                std::process::exit(1);
            };
            if let Err(error) = support::attach(1, attached, &writer) {
                eprintln!("cannot attach the pty: {error}");
                std::process::exit(1);
            }
        }
    }

    let code = app.exec();
    if code != 0 {
        eprintln!("ghostty spike: Qt exited with {code}");
    }
    std::process::exit(code);
}
