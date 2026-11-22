//! `termigator_core::proto::ssh_keys` golden corpus
//! (compat/golden/tools_ssh_keys.json) and the logic tests (live ssh-keygen and agent are separate).
#![allow(clippy::unwrap_used)]

mod common;

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

use serde_json::Value as J;
use termigator_core::config::doc::Value;
use termigator_core::proto::ssh_keys::{
    KEY_TYPES, add_to_agent_argv, copy_id_argv, keygen_argv, list_keys_with, parse_agent_list,
    parse_keygen_line,
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
            "termigator-b5-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&root).unwrap();
        let root = common::canon(&root);
        let t = Tree(root);
        for (rel, content) in files.as_object().into_iter().flatten() {
            let p = t.0.join(rel.trim_end_matches('/'));
            if content.is_null() {
                std::fs::create_dir_all(&p).unwrap();
                continue;
            }
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(&p, t.fill(&common::bytes(content))).unwrap();
        }
        t
    }

    fn fill(&self, bytes: &[u8]) -> Vec<u8> {
        String::from_utf8_lossy(bytes)
            .replace("@ROOT@", &self.0.to_string_lossy())
            .into_bytes()
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

fn tools_of(case: &J) -> Vec<String> {
    case["which"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|s| s.as_str().map(str::to_owned))
        .collect()
}

fn strings(v: &J) -> Vec<String> {
    v.as_array()
        .expect("list")
        .iter()
        .map(|s| s.as_str().expect("str").to_owned())
        .collect()
}

fn message(c: &J) -> String {
    c["message"].as_str().expect("message").to_owned()
}

#[test]
fn golden_corpus() {
    let cases = common::golden("tools_ssh_keys");
    assert!(cases.len() > 500);
    for c in &cases {
        let n = common::name(c);
        match c["fn"].as_str().expect("fn") {
            "parse_line" => {
                let got = parse_keygen_line(c["input"].as_str().expect("input"));
                match c["expect"].as_array() {
                    None => assert_eq!(got, None, "{n}"),
                    Some(e) => {
                        let k = got.unwrap_or_else(|| panic!("{n}: no match"));
                        assert_eq!(k.bits.to_string(), e[0]["$int"].as_str().unwrap(), "{n}");
                        assert_eq!(
                            [k.fingerprint, k.comment, k.key_type],
                            [
                                e[1].as_str().unwrap(),
                                e[2].as_str().unwrap(),
                                e[3].as_str().unwrap()
                            ],
                            "{n}"
                        );
                    }
                }
            }
            "agent" => {
                let got = parse_agent_list(
                    i32::try_from(c["code"].as_i64().unwrap()).unwrap(),
                    c["stdout"].as_str().unwrap(),
                );
                let want = c["expect"].as_array().map(|a| {
                    a.iter()
                        .map(|s| s.as_str().unwrap().to_owned())
                        .collect::<Vec<_>>()
                });
                assert_eq!(got.map(|s| s.into_iter().collect::<Vec<_>>()), want, "{n}");
            }
            "copy_id" => {
                let tools = tools_of(c);
                let got = copy_id_argv(
                    Path::new(c["pub"].as_str().unwrap()),
                    &from_golden(&c["conn"]),
                    c["with_password"].as_bool().unwrap(),
                    &|name| tools.iter().any(|t| t == name),
                );
                if cfg!(windows) && c["with_password"].as_bool().unwrap() {
                    // Windows has no sshpass: a saved password goes through askpass.
                    assert!(got.as_ref().map_or(true, |a| a[0] != "sshpass"), "{n}");
                } else if c.get("error").is_some() {
                    assert_eq!(got, Err(message(c)), "{n}");
                } else {
                    assert_eq!(got, Ok(strings(&c["expect"])), "{n}");
                }
            }
            "keygen" => {
                let t = Tree::new(&c["files"]);
                let home = t.0.join("home");
                let got = keygen_argv(
                    c["type"].as_str().unwrap(),
                    &t.sub(c["path"].as_str().unwrap()),
                    c["comment"].as_str().unwrap(),
                    home.to_str(),
                );
                if c.get("error").is_some() {
                    assert_eq!(got.map_err(|e| t.unsub(&e)), Err(message(c)), "{n}");
                } else {
                    let want = strings(&c["expect"]);
                    assert_eq!(
                        got.map(|a| a.iter().map(|s| t.unsub(s)).collect::<Vec<_>>()),
                        Ok(want),
                        "{n}"
                    );
                }
            }
            "add_agent" => assert_eq!(
                add_to_agent_argv(Path::new(c["key"].as_str().unwrap())),
                strings(&c["expect"]),
                "{n}"
            ),
            "key_types" => {
                let want: Vec<(String, Vec<String>)> = c["expect"]["$map"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|kv| (kv[0].as_str().unwrap().to_owned(), strings(&kv[1])))
                    .collect();
                let got: Vec<(String, Vec<String>)> = KEY_TYPES
                    .iter()
                    .map(|(k, a)| ((*k).to_owned(), a.iter().map(|s| (*s).to_owned()).collect()))
                    .collect();
                assert_eq!(got, want, "{n}");
            }
            "list_keys" => {
                let t = Tree::new(&c["files"]);
                let fake = |pub_path: &Path| {
                    let text = std::fs::read_to_string(pub_path).ok()?;
                    if text == "FAIL" {
                        None
                    } else {
                        parse_keygen_line(&text)
                    }
                };
                let dir = PathBuf::from(t.sub(c["dir"].as_str().unwrap()));
                let got: Vec<Vec<String>> = list_keys_with(&dir, &fake)
                    .into_iter()
                    .map(|k| {
                        vec![
                            t.unsub(&k.path.to_string_lossy()),
                            t.unsub(&k.pub_path.to_string_lossy()),
                            k.bits.to_string(),
                            k.fingerprint,
                            k.comment,
                            k.key_type,
                        ]
                    })
                    .collect();
                let want: Vec<Vec<String>> = c["expect"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|k| {
                        let f = |i: usize| k[i].as_str().unwrap().to_owned();
                        vec![
                            f(0),
                            f(1),
                            k[2]["$int"].as_str().unwrap().to_owned(),
                            f(3),
                            f(4),
                            f(5),
                        ]
                    })
                    .collect();
                assert_eq!(got, want, "{n}");
            }
            other => panic!("{n}: unknown fn {other}"),
        }
    }
}

fn tmp_dir(name: &str) -> Tree {
    Tree::new(&serde_json::json!({ format!("{name}/"): null }))
}

/// test_keygen_argv: the file name rules, never overwriting, the folder must exist.
#[test]
fn keygen_never_overwrites() {
    let t = tmp_dir("ssh");
    let dir = t.0.join("ssh");
    let key = dir.join("id_new");
    let home = t.0.to_str();
    let argv = keygen_argv("ed25519", key.to_str().unwrap(), " me@host ", home).unwrap();
    assert_eq!(argv[..3], ["ssh-keygen", "-t", "ed25519"]);
    assert_eq!(argv[3..], ["-f", key.to_str().unwrap(), "-C", "me@host"]);
    let rsa = keygen_argv("rsa", key.to_str().unwrap(), "c", home).unwrap();
    assert_eq!(rsa[3..5], ["-b", "4096"]);
    assert!(keygen_argv("dsa", key.to_str().unwrap(), "c", home).is_err());
    assert!(keygen_argv("ed25519", "", "c", home).is_err());
    assert!(keygen_argv("ed25519", dir.join("-x").to_str().unwrap(), "c", home).is_err());
    std::fs::write(&key, "").unwrap();
    let err = keygen_argv("ed25519", key.to_str().unwrap(), "c", home).unwrap_err();
    assert!(err.contains("already exists"), "{err}");
    std::fs::remove_file(&key).unwrap();
    std::fs::write(dir.join("id_new.pub"), "").unwrap(); // only the .pub exists: still refused
    assert!(keygen_argv("ed25519", key.to_str().unwrap(), "c", home).is_err());
    let missing = dir.join("nodir/key");
    let err = keygen_argv("ed25519", missing.to_str().unwrap(), "c", home).unwrap_err();
    assert!(
        err.starts_with("Folder ") && err.ends_with("does not exist."),
        "{err}"
    );
}

/// test_copy_id_argv: the target guard and the sshpass fallback.
#[test]
fn copy_id_guards_and_password() {
    let s = |t: &str| Value::Str(t.to_owned());
    let conn = |p: &[(&str, Value)]| Value::Map(p.iter().map(|(k, v)| (s(k), v.clone())).collect());
    let all = |_: &str| true;
    let pub_key = Path::new("/k.pub");
    let a = copy_id_argv(
        pub_key,
        &conn(&[
            ("host", s("h")),
            ("user", s("u")),
            ("port", Value::Int(2200)),
        ]),
        true,
        &all,
    )
    .unwrap();
    // Windows has no sshpass: a saved password goes through askpass instead.
    let tail = ["ssh-copy-id", "-i", "/k.pub", "-p", "2200", "u@h"];
    let want: Vec<&str> = if cfg!(windows) {
        tail.to_vec()
    } else {
        ["sshpass", "-e"].iter().chain(&tail).copied().collect()
    };
    assert_eq!(a, want);
    let bad = conn(&[("host", s("-oProxyCommand=x")), ("user", s("u"))]);
    assert!(copy_id_argv(pub_key, &bad, false, &all).is_err());
    let none = copy_id_argv(pub_key, &conn(&[("host", s("h"))]), false, &|_| false).unwrap_err();
    assert_eq!(
        none,
        "ssh-copy-id is not installed (package openssh-client)."
    );
    // A list proxy_jump shows as the text of the list.
    let list = Value::Seq(vec![s("a"), s("b")]);
    let a = copy_id_argv(
        pub_key,
        &conn(&[("host", s("h")), ("proxy_jump", list)]),
        false,
        &all,
    )
    .unwrap();
    assert_eq!(a[a.len() - 2], "ProxyJump=['a', 'b']");
}

/// The `ssh-keygen -l` line and the `ssh-add -l` exit codes.
#[test]
fn parsers() {
    let k = parse_keygen_line("256 SHA256:abc user@host (ED25519)\n").unwrap();
    assert_eq!(
        (k.bits, k.key_type.as_str(), k.comment.as_str()),
        (256, "ED25519", "user@host")
    );
    assert!(parse_keygen_line("not a key").is_none());
    assert_eq!(parse_agent_list(2, ""), None);
    assert_eq!(
        parse_agent_list(1, "The agent has no identities.").map(|s| s.len()),
        Some(0)
    );
    let ids = parse_agent_list(0, "256 SHA256:aaa a (ED25519)\n4096 MD5:bb b (RSA)\n").unwrap();
    assert_eq!(
        ids.into_iter().collect::<Vec<_>>(),
        ["MD5:bb", "SHA256:aaa"]
    );
}
