//! Robustness of the parsers that read untrusted data (import files, server output, YAML): mutated
//! golden inputs and random bytes must never panic. A dependency-free stand-in for cargo-fuzz that
//! runs with the normal tests; deterministic, so a failure reproduces with its seed.
//! `TERMIGATOR_FUZZ_ITERS` raises the number of inputs per parser for a longer local run.

mod common;

use std::path::Path;

use serde_json::Value as J;
use termigator_core::config::doc;
use termigator_core::import::moba::{decode_moba_bytes, parse_moba_export};
use termigator_core::import::putty::{parse_putty_reg, parse_putty_session_file};
use termigator_core::import::remmina::parse_remmina_profiles;
use termigator_core::import::ssh_config::parse_ssh_config;
use termigator_core::remote::listing::parse_ls_output;
use termigator_core::remote::sync::parse_remote_tree;

/// xorshift64*: tiny, deterministic, good enough to pick mutations.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }
}

/// Bytes that matter to these formats: separators, quotes, escapes, brackets, NUL, UTF-8 parts.
const SPECIAL: &[u8] = b"\0\n\r\t %#=\"'\\[]{}:;,|@/.-*?~$\x7f\xc3\xa4\xff";

fn mutate(seed: &[u8], rng: &mut Rng) -> Vec<u8> {
    let mut v = seed.to_vec();
    for _ in 0..1 + rng.below(8) {
        let at = rng.below(v.len() + 1);
        match rng.below(6) {
            0 if at < v.len() => v[at] ^= 1 << rng.below(8),
            1 => v.insert(at, SPECIAL[rng.below(SPECIAL.len())]),
            2 => v.insert(at, rng.next() as u8),
            3 if at < v.len() => {
                let end = (at + 1 + rng.below(16)).min(v.len());
                v.drain(at..end);
            }
            4 if at < v.len() => {
                let end = (at + 1 + rng.below(32)).min(v.len());
                let chunk = v[at..end].to_vec();
                let to = rng.below(v.len() + 1);
                v.splice(to..to, chunk);
            }
            _ => v.truncate(at),
        }
    }
    v
}

fn iters() -> usize {
    std::env::var("TERMIGATOR_FUZZ_ITERS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(1500)
}

/// Mutations of every seed plus pure random inputs, `n` in all.
fn inputs(seeds: &[Vec<u8>], n: usize, salt: u64) -> Vec<Vec<u8>> {
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15 ^ salt);
    (0..n)
        .map(|i| {
            if seeds.is_empty() || i % 10 == 9 {
                (0..rng.below(512)).map(|_| rng.next() as u8).collect()
            } else {
                mutate(&seeds[i % seeds.len()], &mut rng)
            }
        })
        .collect()
}

fn text_seeds(golden: &str) -> Vec<Vec<u8>> {
    common::golden(golden)
        .iter()
        .filter_map(|c| c.get("input").map(common::bytes))
        .collect()
}

/// yaml_load cases name fixture files (relative to the repo root) or carry the text inline.
fn yaml_seeds() -> Vec<Vec<u8>> {
    common::golden("yaml_load")
        .iter()
        .map(|c| common::input_text(c).into_bytes())
        .collect()
}

fn file_seeds(golden: &str) -> Vec<Vec<u8>> {
    common::golden(golden)
        .iter()
        .filter_map(|c| c.get("files").and_then(J::as_object))
        .flat_map(|m| {
            m.values()
                .filter_map(J::as_str)
                .map(|s| s.as_bytes().to_vec())
        })
        .collect()
}

fn lossy(b: &[u8]) -> String {
    String::from_utf8_lossy(b).into_owned()
}

#[test]
fn moba_never_panics() {
    for b in inputs(&text_seeds("moba_parse"), iters(), 1) {
        let _ = decode_moba_bytes(&b);
        let _ = parse_moba_export(&lossy(&b), "me");
    }
}

#[test]
fn putty_never_panics() {
    let mut seeds = text_seeds("putty_reg");
    seeds.extend(text_seeds("putty_session"));
    for b in inputs(&seeds, iters(), 2) {
        let t = lossy(&b);
        let _ = parse_putty_reg(&t, "me");
        let _ = parse_putty_session_file("s", &t, "me");
    }
}

#[test]
fn ls_and_sync_output_never_panic() {
    let mut seeds = text_seeds("listing_parse");
    seeds.extend(text_seeds("sync_parse"));
    for b in inputs(&seeds, iters(), 3) {
        let _ = parse_ls_output(&b);
        let _ = parse_remote_tree(&b);
    }
}

#[test]
fn yaml_never_panics() {
    let target = doc::load("window: {width: 800}\nlaunch: {template: x}\n").expect("yaml");
    for b in inputs(&yaml_seeds(), iters(), 4) {
        let t = lossy(&b);
        let _ = doc::load(&t);
        let _ = doc::update_preserving_comments(&t, &target);
    }
}

/// The file-based parsers: fewer rounds, each one writes a file.
#[test]
fn ssh_config_and_remmina_never_panic() {
    let dir = std::env::temp_dir().join(format!("tg-fuzz-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("scratch");
    let config = dir.join("config");
    let profile = dir.join("x.remmina");
    let n = iters() / 3;
    for b in inputs(&file_seeds("ssh_config"), n, 5) {
        std::fs::write(&config, &b).expect("write");
        let _ = parse_ssh_config(&config, &dir, "me");
    }
    for b in inputs(&file_seeds("remmina"), n, 6) {
        std::fs::write(&profile, &b).expect("write");
        let _ = parse_remmina_profiles(&[&profile], &[], Path::new("/home/me"));
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// Found by this file: libyaml-safer 0.3.0 unwraps `None` on a backslash at the end of a
/// double-quoted scalar. The config must come back as an error (shown to the user), not a panic.
#[test]
fn yaml_backslash_at_the_end_is_an_error() {
    assert!(doc::load("a: \"\\").is_err());
}
