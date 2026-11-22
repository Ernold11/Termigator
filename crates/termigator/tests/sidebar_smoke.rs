//! The host list in the real binary, offscreen (`qml/sidebar/SidebarSmoke.qml`): rows,
//! search, keys, context menus, null and stubbed hooks, editor validation, copy, duplicate,
//! move, drop, delete and the probe dialog; then what ends up in `config.yaml` / `session.yaml`.

mod common;

use std::fs;

// cxx-qt's generated C++ is linked into this test too; it needs the library's Rust half.
use termigator as _;

const CONFIG: &str = "\
# my hosts
window:
  width: 900
connections:
- {label: web, host: 10.0.0.5, user: admin, group: prod, distro: ubuntu, environment: production}
- {label: db, host: 10.0.0.6, user: root, group: prod/db}
- {label: desk, protocol: rdp, host: 10.0.0.7, user: u}
- {label: broken, protocol: vnc, host: 10.0.0.8, ssh_gateway: gone}
favorites:
- {label: top, command: htop, terminal: external}
";

#[test]
fn host_list_runs_offscreen() {
    let log = common::run_check("u2-sidebar", CONFIG, "sidebar/SidebarSmoke.qml");
    assert!(
        log.contains("termigator sidebar smoke: done"),
        "stderr:\n{log}"
    );
    // A QML error in the panel (a null hook called, a bad binding) is only a warning.
    for bad in ["TypeError", "ReferenceError", "qml/sidebar/"] {
        assert!(!log.contains(bad), "{bad} in stderr:\n{log}");
    }

    let dir = std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("termigator-u2-sidebar");
    assert!(
        fs::metadata(dir.join("shot.png"))
            .map(|m| m.len())
            .unwrap_or(0)
            > 0,
        "no grab of the list"
    );
    let saved = fs::read_to_string(dir.join("config/config.yaml")).expect("config.yaml");
    assert!(saved.contains("# my hosts"), "comment lost:\n{saved}");
    assert!(saved.contains("- empty"), "new folder not saved:\n{saved}");
    // The connections go to session.yaml.
    let hosts = fs::read_to_string(dir.join("config/session.yaml")).expect("session.yaml");
    assert!(
        !hosts.contains("label: web"),
        "deleted folder kept:\n{hosts}"
    );
    assert!(hosts.contains("host: 10.0.0.9"), "edit not saved:\n{hosts}");
    assert!(hosts.contains("group: lab/new"), "move not saved:\n{hosts}");
}
