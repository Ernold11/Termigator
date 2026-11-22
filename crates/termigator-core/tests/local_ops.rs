//! The local Files pane's New folder / Rename / Delete.

use std::fs;
use std::path::PathBuf;

use termigator_core::remote::local_ops::{delete, mkdir, rename, valid_name};

fn scratch(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("termigator-localops-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&d);
    fs::create_dir_all(&d).expect("mkdir");
    d
}

#[test]
fn names_must_be_plain() {
    assert_eq!(valid_name("  notes.txt "), Ok("notes.txt"));
    for bad in ["", "  ", ".", "..", "a/b", "a\\b", "a\0b"] {
        assert!(valid_name(bad).is_err(), "{bad:?}");
    }
}

#[test]
fn mkdir_creates_once() {
    let d = scratch("mkdir");
    let made = mkdir(&d, "new").expect("create");
    assert!(made.is_dir());
    assert!(
        mkdir(&d, "new")
            .expect_err("twice")
            .contains("already exists")
    );
    assert!(mkdir(&d, "../escape").is_err());
    let _ = fs::remove_dir_all(&d);
}

#[test]
fn rename_never_replaces() {
    let d = scratch("rename");
    fs::write(d.join("a"), "1").expect("a");
    fs::write(d.join("b"), "2").expect("b");
    assert!(
        rename(&d.join("a"), "b")
            .expect_err("clash")
            .contains("already exists")
    );
    assert_eq!(fs::read_to_string(d.join("b")).expect("b"), "2");
    let to = rename(&d.join("a"), "c").expect("rename");
    assert_eq!(to, d.join("c"));
    assert!(!d.join("a").exists() && d.join("c").is_file());
    // Same name: nothing to do.
    assert!(rename(&d.join("c"), "c").is_ok());
    let _ = fs::remove_dir_all(&d);
}

#[test]
fn delete_removes_trees_and_reports_failures() {
    let d = scratch("delete");
    fs::create_dir_all(d.join("dir/sub")).expect("tree");
    fs::write(d.join("dir/sub/f"), "x").expect("f");
    fs::write(d.join("file"), "y").expect("file");
    delete(&[d.join("dir"), d.join("file")]).expect("delete");
    assert!(!d.join("dir").exists() && !d.join("file").exists());
    // One missing path does not stop the others.
    fs::write(d.join("keep"), "z").expect("keep");
    let err = delete(&[d.join("missing"), d.join("keep")]).expect_err("missing");
    assert!(err.contains("missing"), "{err}");
    assert!(!d.join("keep").exists());
    let _ = fs::remove_dir_all(&d);
}

#[cfg(unix)]
#[test]
fn delete_does_not_follow_a_link() {
    let d = scratch("link");
    fs::create_dir_all(d.join("real")).expect("real");
    fs::write(d.join("real/f"), "x").expect("f");
    std::os::unix::fs::symlink(d.join("real"), d.join("ln")).expect("link");
    delete(&[d.join("ln")]).expect("delete link");
    assert!(!d.join("ln").exists());
    assert!(d.join("real/f").is_file(), "the target must survive");
    let _ = fs::remove_dir_all(&d);
}
