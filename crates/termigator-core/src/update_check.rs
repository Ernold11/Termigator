//! "A newer version is out": the latest GitHub release against the running version, at most once a
//! day (`window.check_updates`). The request itself is made by the UI (Qt's network stack); this
//! module decides when to ask, reads the answer and remembers the last check and a skipped version
//! in `<config dir>/update-check.json`.

use std::path::Path;

use serde_json::{Value as J, json};

use crate::config::save::write_atomic;

/// Seconds between two checks.
pub const INTERVAL_SECS: u64 = 24 * 60 * 60;
/// The state file next to `config.yaml`.
pub const STATE_FILE: &str = "update-check.json";

/// GitHub's "latest release" API for the repository in `Cargo.toml` (drafts and pre-releases are
/// never "latest"); `None` when the repository is not on github.com.
pub fn latest_release_url() -> Option<String> {
    let repo = env!("CARGO_PKG_REPOSITORY").trim_end_matches('/');
    let path = repo.strip_prefix("https://github.com/")?;
    Some(format!(
        "https://api.github.com/repos/{path}/releases/latest"
    ))
}

/// A release newer than the running version.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Release {
    pub version: String,
    pub url: String,
}

/// `1.2.3` from `v1.2.3` / `1.2.3`; a pre-release or build suffix (`-rc1`, `+git…`) makes the
/// version not comparable. `None` for anything else.
fn numbers(version: &str) -> Option<Vec<u64>> {
    let v = version.trim().trim_start_matches(['v', 'V']);
    if v.is_empty() {
        return None;
    }
    v.split('.').map(|p| p.parse::<u64>().ok()).collect()
}

/// True when `candidate` is a higher plain version than `current` (`0.1.10` > `0.1.9`,
/// `0.2` > `0.1.9`); false when either one is not a plain dotted number.
pub fn is_newer(candidate: &str, current: &str) -> bool {
    let current = current.split(['+', '-']).next().unwrap_or("");
    match (numbers(candidate), numbers(current)) {
        (Some(mut a), Some(mut b)) => {
            let n = a.len().max(b.len());
            a.resize(n, 0);
            b.resize(n, 0);
            a > b
        }
        _ => false,
    }
}

/// The release in a `releases/latest` answer when it is newer than `current` and not `skipped`.
/// Bad JSON, an error object (404 while the repository is private) or a draft give `None`.
pub fn newer_release(body: &str, current: &str, skipped: &str) -> Option<Release> {
    let v: J = serde_json::from_str(body).ok()?;
    if v.get("draft").and_then(J::as_bool) == Some(true)
        || v.get("prerelease").and_then(J::as_bool) == Some(true)
    {
        return None;
    }
    let tag = v.get("tag_name")?.as_str()?;
    let version = tag.trim().trim_start_matches(['v', 'V']).to_owned();
    if !is_newer(&version, current) || version == skipped.trim_start_matches(['v', 'V']) {
        return None;
    }
    let url = v
        .get("html_url")
        .and_then(J::as_str)
        .filter(|u| u.starts_with("https://github.com/"))?
        .to_owned();
    Some(Release { version, url })
}

/// What `update-check.json` holds.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct State {
    /// Unix time of the last check.
    pub checked_at: u64,
    /// A version the user dismissed: not shown again.
    pub skipped: String,
}

impl State {
    /// The state in `dir`; a missing or damaged file is the default (check now).
    pub fn load(dir: &Path) -> Self {
        let Ok(text) = std::fs::read_to_string(dir.join(STATE_FILE)) else {
            return Self::default();
        };
        let v: J = serde_json::from_str(&text).unwrap_or(J::Null);
        Self {
            checked_at: v.get("checked_at").and_then(J::as_u64).unwrap_or(0),
            skipped: v
                .get("skipped")
                .and_then(J::as_str)
                .unwrap_or("")
                .to_owned(),
        }
    }

    pub fn save(&self, dir: &Path) -> std::io::Result<()> {
        let text = json!({"checked_at": self.checked_at, "skipped": self.skipped}).to_string();
        write_atomic(&dir.join(STATE_FILE), &format!("{text}\n"))
    }

    /// A day since the last check, or the clock went back.
    pub fn due(&self, now: u64) -> bool {
        now < self.checked_at || now - self.checked_at >= INTERVAL_SECS
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_compare_by_number() {
        assert!(is_newer("0.1.4", "0.1.3"));
        assert!(is_newer("v0.1.10", "0.1.9"));
        assert!(is_newer("0.2", "0.1.9"));
        assert!(is_newer("1.0.0", "0.9.99"));
        assert!(!is_newer("0.1.3", "0.1.3"));
        assert!(!is_newer("0.1.3", "0.1.4"));
        assert!(!is_newer("0.1.3.0", "0.1.3"));
        // A build suffix on the running version does not count.
        assert!(!is_newer("0.1.3", "0.1.3+git7511464"));
        assert!(is_newer("0.1.4", "0.1.3+git7511464"));
        // Not comparable: never "newer".
        assert!(!is_newer("0.1.4-rc1", "0.1.3"));
        assert!(!is_newer("nightly", "0.1.3"));
        assert!(!is_newer("", "0.1.3"));
    }

    #[test]
    fn reads_a_latest_release() {
        let body = r#"{"tag_name": "v0.1.4", "html_url": "https://github.com/Ernold11/Termigator/releases/tag/v0.1.4", "draft": false, "prerelease": false}"#;
        assert_eq!(
            newer_release(body, "0.1.3", ""),
            Some(Release {
                version: "0.1.4".into(),
                url: "https://github.com/Ernold11/Termigator/releases/tag/v0.1.4".into()
            })
        );
        assert_eq!(newer_release(body, "0.1.4", ""), None);
        assert_eq!(newer_release(body, "0.1.3", "0.1.4"), None);
        assert_eq!(newer_release(body, "0.1.3", "v0.1.4"), None);
    }

    #[test]
    fn ignores_errors_drafts_and_foreign_links() {
        assert_eq!(
            newer_release(r#"{"message": "Not Found"}"#, "0.1.3", ""),
            None
        );
        assert_eq!(newer_release("<html>", "0.1.3", ""), None);
        assert_eq!(newer_release("", "0.1.3", ""), None);
        let draft =
            r#"{"tag_name": "v9.0.0", "html_url": "https://github.com/x/y", "draft": true}"#;
        assert_eq!(newer_release(draft, "0.1.3", ""), None);
        let foreign = r#"{"tag_name": "v9.0.0", "html_url": "https://evil.example/download"}"#;
        assert_eq!(newer_release(foreign, "0.1.3", ""), None);
    }

    #[test]
    fn url_comes_from_the_manifest() {
        assert_eq!(
            latest_release_url().as_deref(),
            Some("https://api.github.com/repos/Ernold11/Termigator/releases/latest")
        );
    }

    #[test]
    fn state_round_trip_and_due() {
        let dir = std::env::temp_dir().join(format!("tg-update-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("dir");
        assert_eq!(State::load(&dir), State::default());
        let s = State {
            checked_at: 1_000_000,
            skipped: "0.1.4".into(),
        };
        s.save(&dir).expect("save");
        assert_eq!(State::load(&dir), s);
        assert!(!s.due(1_000_000 + INTERVAL_SECS - 1));
        assert!(s.due(1_000_000 + INTERVAL_SECS));
        assert!(s.due(999_999)); // clock went back
        std::fs::write(dir.join(STATE_FILE), "garbage").expect("write");
        assert_eq!(State::load(&dir), State::default());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
