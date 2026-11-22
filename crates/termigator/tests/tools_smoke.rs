//! Tunnels, snippets, bulk edit and the disconnect notice in the real binary, offscreen
//! (`qml/tools/ToolsSmoke.qml`). `ssh` is a stub first in `PATH`: it stays up like a working
//! tunnel, or exits with ssh's own words when the forward is `fail:1`. Afterwards: what ended up
//! in `config.yaml`, and that no stub process outlived the application.

mod common;

use std::fs;
use std::path::{Path, PathBuf};

// cxx-qt's generated C++ is linked into this test too; it needs the library's Rust half.
use termigator as _;
use termigator_core::config::doc;
use termigator_core::config::model::value_str;

const CONFIG: &str = "\
window:
  width: 900
  confirm_close: false
  confirm_close_tab: false
  notify_on_disconnect: true
connections:
- {label: bastion, host: 127.0.0.1, user: u}
- {label: web, host: 10.0.0.5, user: admin}
credentials:
- {name: ops, user: root, password: x}
tunnels:
- {name: auto, via: bastion, forwards: ['L 18081:127.0.0.1:9'], autostart: true}
- {name: bad, via: bastion, forwards: ['L 18083:fail:1']}
snippets:
- {name: uptime, command: uptime}
- {name: logs, command: 'journalctl -u {?Service} -f on {host}', enter: false}
";

fn stub_ssh(dir: &Path, pids: &Path) {
    fs::create_dir_all(dir).expect("stub dir");
    fs::create_dir_all(pids).expect("pid dir");
    let script = format!(
        "#!/bin/sh\n\
         echo $$ > '{pids}'/$$\n\
         case \"$*\" in *fail:1*) echo 'Could not request local forwarding.' >&2; exit 255;; esac\n\
         exec sleep 60\n",
        pids = pids.display()
    );
    let path = dir.join("ssh");
    common::write_executable(&path, &script);
}

#[test]
fn tools_run_offscreen() {
    let base = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("termigator-u8-stub");
    let _ = fs::remove_dir_all(&base);
    let (bin, pids) = (base.join("bin"), base.join("pids"));
    stub_ssh(&bin, &pids);
    let path = format!(
        "{}:{}",
        bin.display(),
        std::env::var("PATH").unwrap_or_default()
    );

    let mut run = common::launch_env(
        "u8-tools",
        CONFIG,
        &["--smoke-check=tools/ToolsSmoke.qml"],
        &[("PATH", path.as_str())],
    );
    let status = run.wait(90, |_| {});
    let log = run.log();
    assert!(status.success(), "exit {status}, stderr:\n{log}");
    assert!(
        log.contains("termigator tools smoke: done"),
        "stderr:\n{log}"
    );
    // A QML error (a null hook called, a bad binding) is only a warning. The style's TabBar logs a
    // TypeError whenever a pane's tabs change.
    let ours: String = log
        .lines()
        .filter(|l| !l.contains("org/kde/desktop/TabBar.qml"))
        .collect::<Vec<_>>()
        .join("\n");
    for bad in ["TypeError", "ReferenceError", "qml/tools/"] {
        assert!(!ours.contains(bad), "{bad} in stderr:\n{log}");
    }

    let saved = fs::read_to_string(run.config.join("config.yaml")).expect("config.yaml");
    let cfg = doc::load(&saved).expect("saved yaml");
    let tunnels = cfg.to_json()["tunnels"].clone();
    assert_eq!(
        tunnels,
        serde_json::json!([
            {"name": "auto", "via": "bastion", "forwards": ["L 18081:127.0.0.1:9"], "autostart": true},
            {"name": "bad", "via": "bastion", "forwards": ["L 18083:fail:1"], "autostart": true},
        ]),
        "{saved}"
    );
    let snippets = cfg.to_json()["snippets"].clone();
    assert_eq!(
        snippets,
        serde_json::json!([
            {"name": "logs", "command": "journalctl -u {?Service} -f on {host}", "enter": false},
            {"name": "disk", "command": "df -h {host}", "enter": true},
        ]),
        "{saved}"
    );
    // Muted: the saved config no longer asks for it (a default is not written out).
    let window = cfg.get("window").expect("window section");
    assert!(
        window
            .get("notify_on_disconnect")
            .is_none_or(|v| value_str(v) == "False"),
        "{saved}"
    );

    // Quitting stopped every tunnel: no stub is left running.
    let started: Vec<_> = fs::read_dir(&pids).expect("pids").flatten().collect();
    assert!(started.len() >= 4, "stubs started: {}", started.len());
    for pid in started {
        let proc = Path::new("/proc").join(pid.file_name());
        let alive = fs::read_to_string(proc.join("stat"))
            .map(|s| !s.contains(") Z "))
            .unwrap_or(false);
        assert!(
            !alive,
            "stub {:?} outlived the application",
            pid.file_name()
        );
    }
}
