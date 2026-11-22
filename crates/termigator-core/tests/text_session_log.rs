//! Golden and unit tests for `termigator_core::text::session_log`.

mod common;

use termigator_core::text::session_log::{LocalTime, LogFormatter, render_log_path_with_home};

const NOW: LocalTime = LocalTime {
    year: 2026,
    month: 10,
    day: 21,
    hour: 9,
    minute: 5,
    second: 7,
};

fn time(v: &serde_json::Value) -> LocalTime {
    let n = |i: usize| {
        v[i].as_u64()
            .and_then(|x| u32::try_from(x).ok())
            .unwrap_or(0)
    };
    LocalTime {
        year: n(0),
        month: n(1),
        day: n(2),
        hour: n(3),
        minute: n(4),
        second: n(5),
    }
}

/// Stamps `2026-10-21 09:05:(07+n)` for the n-th stamp, like the golden generator.
fn ticking() -> LogFormatterClock {
    let mut n = 0;
    Box::new(move || {
        let t = LocalTime {
            second: 7 + n,
            ..NOW
        };
        n += 1;
        t
    })
}

type LogFormatterClock = termigator_core::text::session_log::Clock;

#[test]
fn matches_golden() {
    let cases = common::golden("text_session_log");
    assert!(cases.len() > 150);
    for c in &cases {
        let n = common::name(c);
        match c["op"].as_str() {
            Some("path") => {
                let s = |k: &str| c[k].as_str().unwrap_or_default().to_owned();
                let got = render_log_path_with_home(
                    &s("template"),
                    &s("label"),
                    &s("host"),
                    &s("user"),
                    time(&c["now"]),
                    c["home"].as_str(),
                );
                assert_eq!(Some(got.as_str()), c["expect"].as_str(), "{n}");
            }
            Some("format") => {
                let mut f = LogFormatter::new(
                    c["strip_ansi"].as_bool().unwrap_or(false),
                    c["timestamps"].as_bool().unwrap_or(false),
                    ticking(),
                );
                let chunks = c["chunks"].as_array().cloned().unwrap_or_default();
                let expect = c["expect"].as_array().cloned().unwrap_or_default();
                assert_eq!(chunks.len(), expect.len(), "{n}");
                for (i, (ch, ex)) in chunks.iter().zip(&expect).enumerate() {
                    assert_eq!(
                        f.feed(&common::bytes(ch)),
                        common::bytes(ex),
                        "{n} chunk {i}"
                    );
                }
            }
            other => panic!("{n}: unknown op {other:?}"),
        }
    }
}

fn fmt(strip: bool, ts: bool) -> LogFormatter {
    LogFormatter::new(strip, ts, Box::new(|| NOW))
}

#[test]
fn render_path_is_filename_safe() {
    let p = render_log_path_with_home(
        "~/logs/{label}/{host}-{user}-{date}_{time}.log",
        "prod db / main",
        "10.0.0.5",
        "root",
        NOW,
        Some("/home/me"),
    );
    assert_eq!(
        p,
        "/home/me/logs/prod_db_main/10.0.0.5-root-2026-10-21_090507.log"
    );
    assert_eq!(
        render_log_path_with_home("/x/{nope}/{label}.log", "a", "", "", NOW, None),
        "/x/{nope}/a.log"
    );
    assert_eq!(
        render_log_path_with_home("/x/{label}.log", "../../etc", "", "", NOW, None),
        "/x/etc.log"
    );
}

#[test]
fn passthrough_when_nothing_enabled() {
    let raw = b"\x1b[31mred\x1b[0m\r\n";
    assert_eq!(fmt(false, false).feed(raw), raw);
}

#[test]
fn strip_colours_cursor_osc_and_cr() {
    let raw = b"\x1b]0;user@host: ~\x07\x1b[01;32muser@host\x1b[00m:\x1b[01;34m~\x1b[00m$ ls\r\n\x1b[?2004l\x1b(Bfile.txt\r\n";
    assert_eq!(fmt(true, false).feed(raw), b"user@host:~$ ls\nfile.txt\n");
}

#[test]
fn escape_split_across_chunks() {
    let mut f = fmt(true, false);
    let mut out = f.feed(b"a\x1b[3");
    out.extend(f.feed(b"1mb\x1b"));
    out.extend(f.feed(b"]0;title"));
    out.extend(f.feed(b"\x1b\\c\n"));
    assert_eq!(out, b"abc\n");
}

#[test]
fn timestamps_at_line_starts_only() {
    let mut f = fmt(false, true);
    let mut out = f.feed(b"one\npart");
    out.extend(f.feed(b"ial\n\nthree"));
    let stamp = b"[2026-10-21 09:05:07] ";
    let want = [&stamp[..], b"one\n", stamp, b"partial\n\n", stamp, b"three"].concat();
    assert_eq!(out, want);
}

#[test]
fn utf8_survives_stripping() {
    assert_eq!(
        fmt(true, false).feed("\x1b[1mzażółć €\x1b[0m\r\n".as_bytes()),
        "zażółć €\n".as_bytes()
    );
}

#[test]
fn k1_c1_csi_eats_polish_s_acute() {
    // Known defect kept: C5 9B ("ś") loses its second byte to the C1 CSI rule, which then swallows
    // the following bytes up to a CSI final byte (here the "o" of "ok").
    assert_eq!(fmt(true, false).feed("ś ok m!\n".as_bytes()), b"\xc5k m!\n");
}
