//! Golden tests for `termigator_core::text::asciicast`.

mod common;

use termigator_core::text::asciicast::CastRecorder;

fn int(v: &serde_json::Value) -> i64 {
    v.as_i64().unwrap_or(0)
}

fn num(v: &serde_json::Value) -> f64 {
    v.as_f64().unwrap_or(0.0)
}

#[test]
fn matches_golden() {
    let cases = common::golden("text_asciicast");
    assert!(cases.len() > 100);
    for c in &cases {
        let n = common::name(c);
        let env: Vec<(String, String)> = c["env"]
            .as_array()
            .cloned()
            .unwrap_or_default()
            .iter()
            .map(|p| {
                (
                    p[0].as_str().unwrap_or_default().to_owned(),
                    p[1].as_str().unwrap_or_default().to_owned(),
                )
            })
            .collect();
        let mut rec = CastRecorder::new(
            Vec::new(),
            int(&c["width"]),
            int(&c["height"]),
            c["title"].as_str().unwrap_or_default(),
            &env,
            num(&c["now"]),
            num(&c["start"]),
        )
        .unwrap_or_else(|e| panic!("{n}: {e}"));
        let mut out = None;
        for s in c["steps"].as_array().cloned().unwrap_or_default() {
            let t = num(&s["t"]);
            match s["op"].as_str() {
                Some("output") => rec
                    .output(&common::bytes(&s["bytes"]), t)
                    .unwrap_or_else(|e| panic!("{n}: {e}")),
                Some("resize") => rec
                    .resize(int(&s["w"]), int(&s["h"]), t)
                    .unwrap_or_else(|e| panic!("{n}: {e}")),
                _ => {
                    out = Some(rec.close(t).unwrap_or_else(|e| panic!("{n}: {e}")));
                    break;
                }
            }
        }
        let got = out.unwrap_or_else(|| panic!("{n}: no close step"));
        assert_eq!(got, common::bytes(&c["expect"]), "{n}");
    }
}

#[test]
fn v2_header_events_and_split_utf8() {
    let env = vec![
        ("TERM".to_owned(), "xterm-256color".to_owned()),
        ("SECRET".to_owned(), "x".to_owned()),
    ];
    let mut rec = CastRecorder::new(Vec::new(), 80, 24, "demo", &env, 1_700_000_000.0, 100.0)
        .expect("header");
    let zz = "żółw".as_bytes();
    rec.output(&zz[..1], 100.5).expect("write"); // half of "ż": nothing yet
    rec.output(&[&zz[1..], b"\x1b[0m"].concat(), 100.5)
        .expect("write");
    rec.resize(120, 30, 101.25).expect("write");
    rec.resize(120, 30, 101.5).expect("write"); // unchanged: no event
    let out = String::from_utf8(rec.close(103.0).expect("close")).expect("utf8");
    assert_eq!(
        out,
        "{\"version\": 2, \"width\": 80, \"height\": 24, \"timestamp\": 1700000000, \"title\": \"demo\", \"env\": {\"TERM\": \"xterm-256color\"}}\n\
         [0.5, \"o\", \"żółw\\u001b[0m\"]\n[1.25, \"r\", \"120x30\"]\n"
    );
}
