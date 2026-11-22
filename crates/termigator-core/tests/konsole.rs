//! `termigator_core::konsole` (default profile, scheme and font choice) and `proto::konsole`
//! (external terminal template, xdotool): golden
//! corpus compat/golden/tools_konsole.json, tools_konsole_argv.json, tools_xdotool.json.
// Konsole is Linux-only (out of scope on Windows); XDG_DATA_DIRS is `:`-separated.
#![cfg(unix)]
#![allow(clippy::unwrap_used)]

mod common;

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};

use serde_json::Value as J;
use termigator_core::config::doc::Value;
use termigator_core::konsole::{
    XdgDirs, choose_color_scheme, default_appearance, effective_font_size, parse_ini,
    pick_terminal_font, scheme_dirs,
};
use termigator_core::proto::konsole::{
    external_terminal_argv, konsole_argv, parse_window_geometry, parse_window_id,
    xdotool_geometry_argv, xdotool_search_name_argv, xdotool_search_pid_argv,
};

fn from_golden(v: &J) -> Value {
    if let Some((tag, p)) = common::tagged(v) {
        let s = || p.as_str().unwrap_or_default().to_owned();
        return match tag {
            "$int" => Value::Int(s().parse().expect("i64")),
            "$float" => Value::Float(s().parse().expect("f64")),
            "$map" => Value::Map(
                p.as_array()
                    .into_iter()
                    .flatten()
                    .map(|kv| (from_golden(&kv[0]), from_golden(&kv[1])))
                    .collect(),
            ),
            other => panic!("unknown tag {other}"),
        };
    }
    match v {
        J::Null => Value::Null,
        J::Bool(b) => Value::Bool(*b),
        J::String(s) => Value::Str(s.clone()),
        J::Array(a) => Value::Seq(a.iter().map(from_golden).collect()),
        other => panic!("not a golden value: {other}"),
    }
}

/// A throwaway directory tree built from a golden `files` map (`@ROOT@` is its path).
struct Tree(PathBuf);

impl Tree {
    fn new(files: &J) -> Tree {
        static N: AtomicU32 = AtomicU32::new(0);
        let root = std::env::temp_dir().join(format!(
            "termigator-b5k-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&root).unwrap();
        let t = Tree(common::canon(&root));
        for (rel, content) in files.as_object().into_iter().flatten() {
            let p = t.0.join(rel.trim_end_matches('/'));
            if content.is_null() {
                std::fs::create_dir_all(&p).unwrap();
                continue;
            }
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            let raw = common::bytes(content);
            let text = String::from_utf8_lossy(&raw).replace("@ROOT@", &t.0.to_string_lossy());
            // Invalid UTF-8 must reach the file untouched.
            let out = if std::str::from_utf8(&raw).is_ok() {
                text.into_bytes()
            } else {
                raw
            };
            std::fs::write(&p, out).unwrap();
        }
        t
    }

    fn sub(&self, s: &str) -> String {
        s.replace("@ROOT@", &self.0.to_string_lossy())
    }

    fn unsub(&self, s: &str) -> String {
        let s = s.replace(&*self.0.to_string_lossy(), "@ROOT@");
        // Windows joins with `\`; the golden data uses `/`.
        if cfg!(windows) {
            s.replace('\\', "/")
        } else {
            s
        }
    }
}

impl Drop for Tree {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn strings(v: &J) -> Vec<String> {
    v.as_array()
        .expect("list")
        .iter()
        .map(|s| s.as_str().expect("str").to_owned())
        .collect()
}

fn opt_str(v: &J) -> Option<String> {
    v.as_str().map(str::to_owned)
}

fn message(c: &J) -> String {
    c["message"].as_str().expect("message").to_owned()
}

#[test]
fn golden_profile_and_choices() {
    let cases = common::golden("tools_konsole");
    assert!(cases.len() > 450);
    for c in &cases {
        let n = common::name(c);
        match c["fn"].as_str().expect("fn") {
            "appearance" => {
                let t = Tree::new(&c["files"]);
                let env = |k: &str| c["env"].get(k).and_then(J::as_str).map(|v| t.sub(v));
                let xdg = XdgDirs {
                    home: PathBuf::from(env("HOME").unwrap()),
                    config_home: env("XDG_CONFIG_HOME"),
                    data_home: env("XDG_DATA_HOME"),
                    data_dirs: env("XDG_DATA_DIRS"),
                };
                let got = default_appearance(&xdg);
                let e = &c["expect"];
                assert_eq!(got.color_scheme, opt_str(&e["color_scheme"]), "{n}: scheme");
                assert_eq!(got.font_family, opt_str(&e["font_family"]), "{n}: family");
                assert_eq!(
                    got.font_size.map(|s| s.to_string()),
                    e["font_size"]["$int"].as_str().map(str::to_owned),
                    "{n}: size"
                );
                let dirs: Vec<String> = scheme_dirs(&xdg)
                    .iter()
                    .map(|d| t.unsub(&d.to_string_lossy()))
                    .collect();
                assert_eq!(dirs, strings(&c["dirs"]), "{n}: dirs");
            }
            "pick_font" => {
                let installed: HashSet<String> = strings(&c["installed"]).into_iter().collect();
                let got = pick_terminal_font(c["family"].as_str(), &installed);
                assert_eq!(got, c["expect"].as_str().unwrap(), "{n}");
            }
            "choose_scheme" => {
                let got = choose_color_scheme(
                    &strings(&c["available"]),
                    c["preferred"].as_str(),
                    c["dark"].as_bool().unwrap(),
                );
                assert_eq!(got, opt_str(&c["expect"]), "{n}");
            }
            other => panic!("{n}: unknown fn {other}"),
        }
    }
}

fn which_of(c: &J) -> impl Fn(&str) -> Option<String> {
    let tools: Vec<String> = c["which"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|s| s.as_str().map(str::to_owned))
        .collect();
    move |n| {
        tools
            .iter()
            .any(|t| t == n)
            .then(|| format!("/usr/bin/{n}"))
    }
}

#[test]
fn golden_template() {
    let cases = common::golden("tools_konsole_argv");
    assert!(cases.len() > 1000);
    for c in &cases {
        let n = common::name(c);
        let launch = from_golden(&c["launch"]);
        let inner = c["inner"].as_str().unwrap();
        let got = match c["fn"].as_str().expect("fn") {
            "konsole_argv" => konsole_argv(&launch, inner),
            "external_terminal_argv" => external_terminal_argv(&launch, inner, &which_of(c)),
            other => panic!("{n}: unknown fn {other}"),
        };
        if c.get("error").is_some() {
            assert_eq!(got, Err(message(c)), "{n}");
        } else {
            assert_eq!(got, Ok(strings(&c["expect"])), "{n}");
        }
    }
}

#[test]
fn golden_xdotool() {
    let cases = common::golden("tools_xdotool");
    assert!(cases.len() > 50);
    for c in &cases {
        let n = common::name(c);
        match c["fn"].as_str().expect("fn") {
            "window_id" => {
                let out = c["outputs"][0].as_str().unwrap();
                let pid = c["pid"].as_u64().map(|p| p as u32);
                let argvs: Vec<Vec<String>> =
                    c["argv"].as_array().unwrap().iter().map(strings).collect();
                let first = match pid {
                    Some(p) => xdotool_search_pid_argv(p),
                    None => xdotool_search_name_argv(),
                };
                assert_eq!(argvs.first(), Some(&first), "{n}: argv");
                assert_eq!(parse_window_id(out), opt_str(&c["expect"]), "{n}");
            }
            "geometry" => {
                assert_eq!(
                    c["argv"],
                    serde_json::json!([xdotool_geometry_argv("123")]),
                    "{n}: argv"
                );
                let want = c["expect"].as_array().map(|a| {
                    a.iter()
                        .map(|x| x["$int"].as_str().unwrap().parse::<i64>().unwrap())
                        .collect::<Vec<_>>()
                });
                let got = parse_window_geometry(c["output"].as_str().unwrap())
                    .map(|(x, y, w, h)| vec![x, y, w, h]);
                assert_eq!(got, want, "{n}");
            }
            other => panic!("{n}: unknown fn {other}"),
        }
    }
}

fn s(t: &str) -> Value {
    Value::Str(t.to_owned())
}

/// The template rules the golden corpus pins, spelled out.
#[test]
fn template_rules() {
    let tpl = |t: &str| Value::Map(vec![(s("template"), s(t))]);
    let none = Value::Map(vec![]);
    assert_eq!(
        konsole_argv(&none, "ssh u@h").unwrap(),
        ["konsole", "-e", "bash", "-lc", "ssh u@h"]
    );
    assert_eq!(konsole_argv(&tpl("  "), "x").unwrap()[0], "konsole");
    assert_eq!(
        konsole_argv(&tpl("term {{a}} {foo} {cmd_quoted}"), "x").unwrap(),
        ["term", "{{a}}", "{foo}", "x"]
    );
    // An out-of-range index is an error value.
    assert_eq!(
        konsole_argv(&tpl("term {0}"), "x"),
        Err("Replacement index 0 out of range for positional args tuple".to_owned())
    );
    assert!(konsole_argv(&tpl("term {"), "x").is_err());
    // Not supported (padding / indexing): an error instead of a wrong command.
    assert!(konsole_argv(&tpl("term {cmd_quoted:>9}"), "x").is_err());
    assert!(konsole_argv(&tpl("term {cmd_quoted[0]}"), "x").is_err());
    let which = |n: &str| (n == "xterm").then(|| "/usr/bin/xterm".to_owned());
    assert_eq!(
        external_terminal_argv(&tpl("xterm -e {cmd_quoted}"), "a b", &which).unwrap(),
        ["/usr/bin/xterm", "-e", "a b"]
    );
    let missing = external_terminal_argv(&none, "x", &which).unwrap_err();
    assert!(
        missing.starts_with("`konsole` not found in PATH."),
        "{missing}"
    );
}

#[test]
fn font_size_and_defaults() {
    assert_eq!(effective_font_size(Some(14), Some(10)), 14);
    assert_eq!(effective_font_size(None, Some(10)), 10);
    assert_eq!(effective_font_size(Some(0), Some(12)), 12);
    assert_eq!(effective_font_size(None, None), 11);
    assert_eq!(effective_font_size(Some(-3), None), 11);
    let installed: HashSet<String> = HashSet::new();
    assert_eq!(
        pick_terminal_font(None, &installed),
        "JetBrainsMono Nerd Font Mono"
    );
    let only_x: HashSet<String> = ["Other".to_owned()].into();
    assert_eq!(pick_terminal_font(Some("Mine"), &only_x), "Mine");
}

#[test]
fn ini_reader_follows_configparser() {
    let ini = parse_ini("[A]\nk = v\nj: w\n  more\n[DEFAULT]\nd=1\n").unwrap();
    assert_eq!(ini.get("A", "k"), Some("v"));
    assert_eq!(ini.get("A", "j"), Some("w\nmore"));
    assert_eq!(ini.get("A", "d"), Some("1"));
    assert_eq!(ini.get("B", "d"), None);
    assert!(parse_ini("k=v\n").is_none());
    assert!(parse_ini("[A]\nnot an option\n").is_none());
}
