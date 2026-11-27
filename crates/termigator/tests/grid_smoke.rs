//! `qml/terminal/GridCheck.qml` offscreen at several scale factors: glyphs land on their cells
//! (mc's coloured blank cells keep their columns, k9s's box corners meet their lines).

mod common;

// cxx-qt's generated C++ is linked into this test too; it needs the library's Rust half.
use termigator as _;

const CONFIG: &str =
    "window:\n  confirm_close: false\n  confirm_close_tab: false\n  width: 1000\n  height: 700\n";
const CHECK: &str = "--smoke-check=terminal/GridCheck.qml";

fn check(name: &str, scale: &str) {
    let mut run = common::launch_env(name, CONFIG, &[CHECK], &[("QT_SCALE_FACTOR", scale)]);
    let status = run.wait(60, |_| {});
    let log = run.log();
    assert!(status.success(), "exit {status}, stderr:\n{log}");
    assert!(
        log.contains("GRID_DONE"),
        "the check did not finish, stderr:\n{log}"
    );
}

#[test]
fn cell_grid_at_scale_1() {
    check("grid-1", "1");
}

#[test]
fn cell_grid_at_scale_1_25() {
    check("grid-1.25", "1.25");
}

#[test]
fn cell_grid_at_scale_1_5() {
    check("grid-1.5", "1.5");
}
