//! Tunnels, snippets, bulk edit and the disconnect notice in the real binary, offscreen
//! (`qml/tools/ToolsSmoke.qml`). `ssh` is a stub first in `PATH`: it stays up like a working
//! tunnel, or exits with ssh's own words when the forward is `fail:1`. Afterwards: what ended up
//! in `config.yaml`, which keys the stub was given, and that no stub process outlived the
//! application.

mod common;

use std::fs;
use std::path::{Path, PathBuf};

// cxx-qt's generated C++ is linked into this test too; it needs the library's Rust half.
use termigator as _;
use termigator_core::config::doc;
use termigator_core::config::model::value_str;

/// `{entry_key}` and `{tunnel_key}`: the key of the `keyhost` entry and the tunnel `keyed`'s own.
const CONFIG: &str = "\
window:
  width: 900
  confirm_close: false
  confirm_close_tab: false
  notify_on_disconnect: true
connections:
- {label: bastion, host: 127.0.0.1, user: u}
- {label: web, host: 10.0.0.5, user: admin}
- {label: keyhost, host: 127.0.0.1, user: k, identity_file: '{entry_key}'}
credentials:
- {name: ops, user: root, password: x}
tunnels:
- {name: auto, via: bastion, forwards: ['L 18081:127.0.0.1:9'], autostart: true}
- {name: bad, via: bastion, forwards: ['L 18083:fail:1']}
- {name: keyed, via: keyhost, forwards: ['D 18086'], identity_file: '{tunnel_key}'}
snippets:
- {name: uptime, command: uptime}
- {name: logs, command: 'journalctl -u {?Service} -f on {host}', enter: false}
";

fn stub_ssh(dir: &Path, pids: &Path, args: &Path) {
    fs::create_dir_all(dir).expect("stub dir");
    fs::create_dir_all(pids).expect("pid dir");
    fs::create_dir_all(args).expect("args dir");
    let script = format!(
        "#!/bin/sh\n\
         echo $$ > '{pids}'/$$\n\
         printf '%s\\n' \"$@\" > '{args}'/$$\n\
         case \"$*\" in *fail:1*) echo 'Could not request local forwarding.' >&2; exit 255;; esac\n\
         exec sleep 60\n",
        pids = pids.display(),
        args = args.display()
    );
    let path = dir.join("ssh");
    common::write_executable(&path, &script);
}

/// A private key file (owner only, so ssh is given the file itself) whose header says it has a
/// passphrase; the stub ssh never reads it.
fn locked_key(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    fs::write(
        path,
        "-----BEGIN ENCRYPTED PRIVATE KEY-----\nMIIE\n-----END ENCRYPTED PRIVATE KEY-----\n",
    )
    .expect("key");
    fs::set_permissions(path, fs::Permissions::from_mode(0o600)).expect("chmod");
}

/// The `-i` arguments of every stub ssh started with `forward`.
fn keys_given(args: &Path, forward: &str) -> Vec<Vec<String>> {
    fs::read_dir(args)
        .expect("args")
        .flatten()
        .filter_map(|e| fs::read_to_string(e.path()).ok())
        .filter(|a| a.lines().any(|l| l == forward))
        .map(|a| {
            let lines: Vec<&str> = a.lines().collect();
            lines
                .windows(2)
                .filter(|w| w[0] == "-i")
                .map(|w| w[1].to_owned())
                .collect()
        })
        .collect()
}

#[test]
fn tools_run_offscreen() {
    let base = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("termigator-u8-stub");
    let _ = fs::remove_dir_all(&base);
    let (bin, pids, args) = (base.join("bin"), base.join("pids"), base.join("args"));
    stub_ssh(&bin, &pids, &args);
    let (entry_key, tunnel_key) = (base.join("entry_key"), base.join("tunnel_key"));
    locked_key(&entry_key);
    locked_key(&tunnel_key);
    let config = CONFIG
        .replace("{entry_key}", &entry_key.to_string_lossy())
        .replace("{tunnel_key}", &tunnel_key.to_string_lossy());
    let path = format!(
        "{}:{}",
        bin.display(),
        std::env::var("PATH").unwrap_or_default()
    );

    let mut run = common::launch_env(
        "u8-tools",
        &config,
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
            {"name": "keyed", "via": "keyhost", "forwards": ["D 18086"], "autostart": false,
             "identity_file": tunnel_key.to_string_lossy()},
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

    // The tunnel's own key, not its entry's; a tunnel without one has none of its own either.
    let tunnel_key = tunnel_key.to_string_lossy().into_owned();
    assert_eq!(keys_given(&args, "127.0.0.1:18086"), [vec![tunnel_key]]);
    let plain = keys_given(&args, "18081:127.0.0.1:9");
    assert!(
        !plain.is_empty() && plain.iter().all(Vec::is_empty),
        "{plain:?}"
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
