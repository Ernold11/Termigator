//! The Files local pane from the keyboard (`qml/files/FilesKeysSmoke.qml`), next to a local
//! shell and without a server: arrows, Enter / Backspace, F2, Ctrl+N, Delete, Shift+F10 and no
//! reaction while the path field has the focus; then what is on the disk.
// The check compares `/`-separated folders; the Windows build is checked on the VM by hand.
#![cfg(unix)]

mod common;

use std::fs;

// cxx-qt's generated C++ is linked into this test too; it needs the library's Rust half.
use termigator as _;

#[test]
fn local_files_are_managed_from_the_keyboard() {
    let mut run = common::launch(
        "u2-files-keys",
        "window: {width: 1100}\n",
        &["--smoke-check=files/FilesKeysSmoke.qml"],
    );
    // The launcher made the directory afresh; the check waits until it lists these.
    let keys = run.shot.with_file_name("keys");
    fs::create_dir_all(keys.join("sub")).expect("sub");
    fs::write(keys.join("sub/inner.txt"), "inner").expect("inner");
    fs::write(keys.join("a.txt"), "a").expect("a");
    fs::write(keys.join("b.txt"), "b").expect("b");
    let status = run.wait(60, |_| {});
    let log = run.log();
    assert!(status.success(), "exit {status}, stderr:\n{log}");
    assert!(
        log.contains("termigator files keys smoke: done"),
        "stderr:\n{log}"
    );
    for bad in ["TypeError", "ReferenceError", "qml/files/"] {
        assert!(!log.contains(bad), "{bad} in stderr:\n{log}");
    }
    assert!(keys.join("c.txt").is_file(), "F2 rename not on disk");
    assert!(!keys.join("a.txt").exists(), "old name still there");
    assert!(keys.join("newdir").is_dir(), "Ctrl+N folder not on disk");
    assert!(!keys.join("b.txt").exists(), "Delete did not delete");
    assert!(
        keys.join("sub/inner.txt").is_file(),
        "the folder lost its file"
    );
}
