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
    // The jump host key the SSH editor shows: one with a passphrase (EditorsSmoke.qml finds it
    // next to its own directory, termigator-u5-editors).
    let keys =
        std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("termigator-u5-editors-keys");
    std::fs::create_dir_all(&keys).expect("keys dir");
    std::fs::write(
        keys.join("jump_key"),
        "-----BEGIN ENCRYPTED PRIVATE KEY-----\nMIIE\n-----END ENCRYPTED PRIVATE KEY-----\n",
    )
    .expect("jump key");
    // Owner only, like a real key (else the field warns that OpenSSH would refuse it).
    termigator_core::platform::set_private_file(&keys.join("jump_key")).expect("chmod");
    let log = common::run_check("u5-editors", config, "editors/EditorsSmoke.qml");
    assert!(
        log.contains("termigator editors smoke: ok"),
        "stderr:\n{log}"
    );
}
