//! Golden tests for `termigator_core::shell` (compat/golden/shell.json).

mod common;

use serde_json::Value as J;
use termigator_core::shell::{join, quote, split};

fn strings(v: &J) -> Vec<String> {
    v.as_array()
        .unwrap_or_else(|| panic!("not a list: {v}"))
        .iter()
        .map(|s| s.as_str().unwrap_or_default().to_owned())
        .collect()
}

#[test]
fn matches_posix_shell_splitting() {
    let cases = common::golden("shell");
    assert!(cases.len() > 100);
    for case in &cases {
        let n = common::name(case);
        let input = &case["input"];
        let op = case["op"].as_str().unwrap_or_default();
        if op == "split" {
            let got = split(input.as_str().unwrap_or_default());
            if case.get("error").is_some() {
                let err = got.expect_err(n);
                assert_eq!(
                    Some(err.to_string().as_str()),
                    case["message"].as_str(),
                    "{n}"
                );
            } else {
                assert_eq!(got.expect(n), strings(&case["expect"]), "{n}");
            }
            continue;
        }
        let got = match op {
            "quote" => quote(input.as_str().unwrap_or_default()),
            "join" => join(&strings(input)),
            other => panic!("{n}: unknown op {other}"),
        };
        assert_eq!(Some(got.as_str()), case["expect"].as_str(), "{n}");
    }
}
