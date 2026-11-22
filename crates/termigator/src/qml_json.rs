//! JSON payloads that QML hands to invokables.

use serde_json::Value as J;

/// A payload from QML parsed as any JSON value; see [`logged`].
pub(crate) fn value(invokable: &str, text: &str) -> Option<J> {
    logged(invokable, text, serde_json::from_str(text))
}

/// `parsed` (the result of `serde_json::from_str(text)`) as an `Option`: callers keep their own
/// fallback for a bad payload, which is logged once. Only the error kind and position are
/// logged, because a data error's message quotes the offending value and payloads can hold
/// passwords. An empty text means "nothing" and is not logged.
pub(crate) fn logged<T>(invokable: &str, text: &str, parsed: serde_json::Result<T>) -> Option<T> {
    parsed
        .map_err(|e| {
            if let Some(why) = describe(&e, text) {
                eprintln!("termigator: {invokable}: bad JSON from QML: {why}");
            }
        })
        .ok()
}

fn describe(e: &serde_json::Error, text: &str) -> Option<String> {
    if text.trim().is_empty() {
        return None;
    }
    Some(if e.is_data() {
        format!(
            "unexpected shape at line {} column {}",
            e.line(),
            e.column()
        )
    } else {
        e.to_string()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_good_payloads_and_describes_bad_ones_without_their_contents() {
        let ids = |s: &str| logged::<Vec<u64>>("t", s, serde_json::from_str(s));
        assert_eq!(ids("[1, 2]"), Some(vec![1, 2]));
        assert_eq!(ids(r#"["hunter2"]"#), None);
        assert_eq!(value("t", "{"), None);
        assert_eq!(value("t", "{}"), Some(J::Object(Default::default())));

        let data = serde_json::from_str::<Vec<u64>>(r#"["hunter2"]"#).unwrap_err();
        let why = describe(&data, r#"["hunter2"]"#).unwrap();
        assert!(!why.contains("hunter2"), "{why}");
        let syntax = serde_json::from_str::<J>("{").unwrap_err();
        assert!(
            describe(&syntax, "{").unwrap().contains("EOF"),
            "syntax errors keep serde's text"
        );
        let empty = serde_json::from_str::<J>(" ").unwrap_err();
        assert_eq!(describe(&empty, " "), None);
    }
}
