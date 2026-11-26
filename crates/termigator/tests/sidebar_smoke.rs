//! The host list in the real binary, offscreen (`qml/sidebar/SidebarSmoke.qml`): rows,
//! search, keys, context menus, null and stubbed hooks, editor validation, copy, duplicate,
//! move, drop, delete, folder rename / move / delete and the probe dialog; then what ends up in `config.yaml` / `session.yaml`.
//! A second run drives the tree from the keyboard only (`qml/sidebar/HostsKeysSmoke.qml`).

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
    // A new empty folder is kept; deleted, renamed and moved-away ones are not.
    assert!(
        saved.contains("- infra/kept"),
        "new folder not saved:\n{saved}"
    );
    for gone in ["- empty", "- lab", "- prod"] {
        assert!(
            !saved.contains(gone),
            "deleted folder {gone} saved:\n{saved}"
        );
    }
    // The connections go to session.yaml.
    let hosts = fs::read_to_string(dir.join("config/session.yaml")).expect("session.yaml");
    assert!(
        !hosts.contains("label: web"),
        "deleted folder kept:\n{hosts}"
    );
    assert!(hosts.contains("host: 10.0.0.9"), "edit not saved:\n{hosts}");
    // Moved into a folder, which was renamed, then deleted into its parent; db left its folder.
    assert!(hosts.contains("group: infra\n"), "move not saved:\n{hosts}");
    assert!(!hosts.contains("group: lab"), "old folder kept:\n{hosts}");
    assert!(hosts.contains("label: db"), "moved entry lost:\n{hosts}");
    // "Add to Hosts…" from a tab: saved, without the tab's password; the update in place.
    assert!(hosts.contains("label: jump"), "tab not added:\n{hosts}");
    assert!(
        hosts.contains("host: 10.0.0.70"),
        "tab update not saved:\n{hosts}"
    );
    assert!(
        !hosts.contains("tab-secret"),
        "tab password saved:\n{hosts}"
    );
}

const KEYS_CONFIG: &str = "\
connections:
- {label: web, host: 10.0.0.5, user: admin, group: prod}
- {label: db, host: 10.0.0.6, user: root, group: prod/db}
- {label: desk, protocol: rdp, host: 10.0.0.7, user: u, ssh_gateway: web}
- {label: solo, host: 10.0.0.8, user: u}
groups: [lab]
";

#[test]
fn host_tree_is_managed_from_the_keyboard() {
    let log = common::run_check("u2-hosts-keys", KEYS_CONFIG, "sidebar/HostsKeysSmoke.qml");
    assert!(
        log.contains("termigator hosts keys smoke: done"),
        "stderr:\n{log}"
    );
    for bad in ["TypeError", "ReferenceError", "qml/sidebar/"] {
        assert!(!log.contains(bad), "{bad} in stderr:\n{log}");
    }
    let dir =
        std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("termigator-u2-hosts-keys");
    let hosts = fs::read_to_string(dir.join("config/session.yaml")).expect("session.yaml");
    // F2 on the gateway renamed it and the RDP entry that uses it.
    assert!(
        hosts.contains("label: bastion"),
        "rename not saved:\n{hosts}"
    );
    assert!(
        hosts.contains("ssh_gateway: bastion"),
        "gateway reference lost:\n{hosts}"
    );
    assert!(!hosts.contains("label: solo"), "old name kept:\n{hosts}");
    assert!(
        !hosts.contains("alone (copy)"),
        "deleted entry saved:\n{hosts}"
    );
}
