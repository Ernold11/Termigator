//! The connection editors, offscreen. `qml/editors/EditorsSmoke.qml` opens every editor
//! through `shell.openEditor` and quits by itself (0 = every check held).

mod common;

// cxx-qt's generated C++ is linked into this test too; it needs the library's Rust half.
use termigator as _;

#[test]
fn connection_editors_work_offscreen() {
    // probe_timeout 1: the probe that answers after its dialog is closed is quick.
    let config = "window:\n  width: 800\nlaunch:\n  probe_timeout: 1\ncredentials:\n- {name: ops, user: root, password: x}\n\
                  connections:\n- {label: bastion, host: b, user: u}\n";
    let log = common::run_check("u5-editors", config, "editors/EditorsSmoke.qml");
    assert!(
        log.contains("termigator editors smoke: ok"),
        "stderr:\n{log}"
    );
}
