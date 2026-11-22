//! Linux distribution detection from login output (the Qt badge painting is UI and lives elsewhere).
//!
//! An `ID=` line (as printed by `/etc/os-release`) wins; otherwise the ordered list of 19 name
//! patterns (most specific first) is tried against the text.
//!
//! Golden: `compat/golden/text_distro.json`, tests in `tests/text_distro.rs`.

use std::sync::LazyLock;

use regex::Regex;

/// Ordered name patterns matched against the session output.
const NAME_PATTERNS: [(&str, &str); 19] = [
    (r"(?i)raspberry pi os|raspbian", "raspbian"),
    (r"(?i)linux mint", "mint"),
    (r"(?i)pop!_os|pop os", "pop"),
    (r"(?i)amazon linux", "amazon"),
    (r"(?i)oracle linux", "oracle"),
    (r"(?i)\bubuntu\b", "ubuntu"),
    (r"(?i)\bdebian\b", "debian"),
    (r"(?i)rocky linux", "rocky"),
    (r"(?i)alma\s*linux", "almalinux"),
    (r"(?i)red hat enterprise|\brhel\b", "rhel"),
    (r"(?i)\bcentos\b", "centos"),
    (r"(?i)\bfedora\b", "fedora"),
    (r"(?i)\bmanjaro\b", "manjaro"),
    (r"(?i)arch\s*linux", "arch"),
    (r"(?i)\bkali\b", "kali"),
    (r"(?i)opensuse|suse linux", "opensuse"),
    (r"(?i)\bgentoo\b", "gentoo"),
    (r"(?i)void linux", "void"),
    (r"(?i)\balpine\b", "alpine"),
];

/// os-release `ID=` value -> canonical id.
const ID_MAP: [(&str, &str); 22] = [
    ("ubuntu", "ubuntu"),
    ("debian", "debian"),
    ("raspbian", "raspbian"),
    ("linuxmint", "mint"),
    ("pop", "pop"),
    ("rocky", "rocky"),
    ("almalinux", "almalinux"),
    ("rhel", "rhel"),
    ("centos", "centos"),
    ("fedora", "fedora"),
    ("manjaro", "manjaro"),
    ("arch", "arch"),
    ("kali", "kali"),
    ("opensuse", "opensuse"),
    ("opensuse-leap", "opensuse"),
    ("opensuse-tumbleweed", "opensuse"),
    ("sles", "opensuse"),
    ("gentoo", "gentoo"),
    ("void", "void"),
    ("alpine", "alpine"),
    ("amzn", "amazon"),
    ("ol", "oracle"),
];

// `[\s\x1c-\x1f]` is Unicode `\s` (it also counts the four information separators).
static ID_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?mi)^[\s\x1c-\x1f]*ID=["']?([a-z0-9._-]+)"#)
        .unwrap_or_else(|e| unreachable!("static regex: {e}"))
});

static NAME_RES: LazyLock<Vec<(Regex, &'static str)>> = LazyLock::new(|| {
    NAME_PATTERNS
        .iter()
        .map(|(p, id)| {
            (
                Regex::new(p).unwrap_or_else(|e| unreachable!("static regex: {e}")),
                *id,
            )
        })
        .collect()
});

/// A canonical distro id found in `text`, or `None`. The os-release `ID=` wins over names.
pub fn detect_distro(text: &str) -> Option<&'static str> {
    if text.is_empty() {
        return None;
    }
    for m in ID_RE.captures_iter(text) {
        let key = m[1].to_lowercase();
        if let Some((_, id)) = ID_MAP.iter().find(|(k, _)| *k == key) {
            return Some(id);
        }
    }
    NAME_RES
        .iter()
        .find(|(rx, _)| rx.is_match(text))
        .map(|(_, id)| *id)
}
