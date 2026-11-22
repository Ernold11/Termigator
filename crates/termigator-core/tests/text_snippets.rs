//! Golden and unit tests for `termigator_core::text::snippets`.

mod common;

use std::collections::HashMap;

use serde_json::{Value as J, json};
use termigator_core::text::snippets::{
    Snippet, SnippetConn, expand, normalize_snippets, questions, text_to_send,
    waiting_for_user_input,
};

fn s(v: &J) -> String {
    match v {
        J::String(x) => x.clone(),
        J::Null => String::new(),
        // A missing or falsy value -> "", else its text.
        J::Number(n) if n.as_f64() == Some(0.0) => String::new(),
        other => other.to_string(),
    }
}

fn conn(v: &J) -> SnippetConn {
    SnippetConn {
        host: s(&v["host"]),
        user: s(&v["user"]),
        label: s(&v["label"]),
        port: s(&v["port"]),
    }
}

fn answers(v: &J) -> HashMap<String, String> {
    v.as_object()
        .map(|o| o.iter().map(|(k, x)| (k.clone(), s(x))).collect())
        .unwrap_or_default()
}

fn strings(v: &J) -> Vec<String> {
    v.as_array()
        .map(|a| a.iter().map(s).collect())
        .unwrap_or_default()
}

#[test]
fn matches_golden() {
    let cases = common::golden("text_snippets");
    assert!(cases.len() > 400);
    for c in &cases {
        let n = common::name(c);
        match c["op"].as_str() {
            Some("questions") => {
                assert_eq!(
                    questions(c["command"].as_str().unwrap_or_default()),
                    strings(&c["expect"]),
                    "{n}"
                );
            }
            Some("expand") => {
                let got = expand(
                    c["command"].as_str().unwrap_or_default(),
                    &conn(&c["conn"]),
                    &answers(&c["answers"]),
                );
                assert_eq!(Some(got.as_str()), c["expect"].as_str(), "{n}");
            }
            Some("to_send") => {
                let got = text_to_send(
                    c["command"].as_str().unwrap_or_default(),
                    c["enter"].as_bool().unwrap_or(false),
                );
                assert_eq!(Some(got.as_str()), c["expect"].as_str(), "{n}");
            }
            Some("normalize") => {
                let want: Vec<Snippet> = c["expect"]
                    .as_array()
                    .cloned()
                    .unwrap_or_default()
                    .iter()
                    .map(|e| Snippet {
                        name: s(&e["name"]),
                        command: s(&e["command"]),
                        enter: e["enter"].as_bool().unwrap_or(false),
                    })
                    .collect();
                assert_eq!(normalize_snippets(&c["raw"]), want, "{n}");
            }
            Some("waiting") => {
                let got = waiting_for_user_input(c["input"].as_str().unwrap_or_default());
                assert_eq!(Some(got), c["expect"].as_bool(), "{n}");
            }
            other => panic!("{n}: unknown op {other:?}"),
        }
    }
}

#[test]
fn expand_per_host_fields_and_questions() {
    let cmd = "ssh-keyscan -p {port} {host} # {label} as {user}; journalctl -u {?Service} -n {?Lines} -u {?Service}";
    let c = SnippetConn {
        host: "10.0.0.5".into(),
        user: "root".into(),
        label: "db".into(),
        port: String::new(),
    };
    assert_eq!(questions(cmd), ["Service", "Lines"]);
    let a = HashMap::from([
        ("Service".to_owned(), "nginx".to_owned()),
        ("Lines".to_owned(), "50".to_owned()),
    ]);
    assert_eq!(
        expand(cmd, &c, &a),
        "ssh-keyscan -p 22 10.0.0.5 # db as root; journalctl -u nginx -n 50 -u nginx"
    );
}

#[test]
fn literal_braces_and_unknown_fields_untouched() {
    let c = SnippetConn {
        host: "h".into(),
        ..SnippetConn::default()
    };
    assert_eq!(
        expand("awk '{{print $1}}' {host} {other}", &c, &HashMap::new()),
        "awk '{print $1}' h {other}"
    );
}

#[test]
fn missing_values_become_empty() {
    assert_eq!(
        expand(
            "echo [{user}] [{?Q}]",
            &SnippetConn::default(),
            &HashMap::new()
        ),
        "echo [] []"
    );
}

#[test]
fn text_to_send_cases() {
    assert_eq!(text_to_send("uptime", true), "uptime\n");
    assert_eq!(text_to_send("cd /srv\nls\n", true), "cd /srv\nls\n");
    assert_eq!(
        text_to_send("sudo systemctl restart ", false),
        "sudo systemctl restart "
    );
}

#[test]
fn normalize_drops_broken_entries() {
    let raw = json!([{"name": "Up", "command": "uptime"}, {"command": "  "}, "junk", {"command": "df -h\nfree"}]);
    assert_eq!(
        normalize_snippets(&raw),
        [
            Snippet {
                name: "Up".into(),
                command: "uptime".into(),
                enter: true
            },
            Snippet {
                name: "df -h".into(),
                command: "df -h\nfree".into(),
                enter: true
            },
        ]
    );
    assert!(normalize_snippets(&J::Null).is_empty());
}

#[test]
fn waiting_for_user_input_cases() {
    let w = waiting_for_user_input;
    assert!(w("root@10.0.0.5's password: "));
    assert!(w("Enter passphrase for key '/home/me/.ssh/id_ed25519': "));
    assert!(w(
        "Are you sure you want to continue connecting (yes/no/[fingerprint])? "
    ));
    assert!(w("\x1b[1mVerification code:\x1b[0m "));
    assert!(!w("Welcome to Ubuntu\r\n\x1b[01;32mroot@db\x1b[00m:~# "));
    assert!(!w("Last login: Mon Oct 20 from 10.0.0.1\r\n$ "));
    assert!(!w("password changed\r\nme@host:~$ "));
    assert!(!w("user@pinboard:~$ ") && !w("[token@vault ~]# "));
    assert!(w("Enter PIN for 'YubiKey': "));
}
