//! Every button in the app's QML has a name a screen reader can say: a `text:` (shown, or kept off
//! screen with `display: IconOnly`) or an `Accessible.name:`. A static scan, no Qt needed.

use std::path::Path;

fn qml_files(dir: &Path, out: &mut Vec<std::path::PathBuf>) {
    for e in std::fs::read_dir(dir).expect("qml dir").flatten() {
        let p = e.path();
        if p.is_dir() {
            qml_files(&p, out);
        } else if p.extension().is_some_and(|x| x == "qml") {
            out.push(p);
        }
    }
}

/// The body of the block that starts at `open` (just after `{`), nested blocks removed.
fn top_level(s: &str, open: usize) -> String {
    let (mut depth, mut out) = (1, String::new());
    for c in s[open..].chars() {
        match c {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    break;
                }
            }
            _ if depth == 1 => out.push(c),
            _ => {}
        }
    }
    out
}

#[test]
fn buttons_have_an_accessible_name() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("qml");
    let mut files = Vec::new();
    qml_files(&root, &mut files);
    let mut missing = Vec::new();
    for f in files {
        let name = f.file_name().and_then(|n| n.to_str()).unwrap_or_default();
        // Test scenes drive the UI; they are not UI.
        if name.contains("Smoke") || name.contains("Check") {
            continue;
        }
        let s = std::fs::read_to_string(&f).expect("read");
        for kind in ["ToolButton {", "RoundButton {", "Button {"] {
            for (at, _) in s.match_indices(kind) {
                // Whole type names only: `ActionButton {` (a component that names itself) and
                // `ToolButton {` (scanned on its own) also contain `Button {`.
                if s[..at]
                    .chars()
                    .last()
                    .is_some_and(|c| c.is_alphanumeric() || c == '_')
                {
                    continue;
                }
                let body = top_level(&s, at + kind.len());
                let named = body.lines().any(|l| {
                    let l = l.trim_start();
                    l.starts_with("text:") || l.starts_with("Accessible.name:")
                });
                if !named {
                    let line = s[..at].matches('\n').count() + 1;
                    missing.push(format!("{}:{line}", f.display()));
                }
            }
        }
    }
    assert!(
        missing.is_empty(),
        "buttons without text / Accessible.name:\n{}",
        missing.join("\n")
    );
}
