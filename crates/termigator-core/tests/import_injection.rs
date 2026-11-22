//! Hostile user / host / port / jump host values that come in through an import must not
//! turn into options of the launched client. Every importer gets the same payloads; each
//! imported entry goes through the builder of its protocol (and scp for SSH), and the builder
//! either refuses it or quotes the value so that no shell word starts with the payload.

#![cfg(unix)]

use std::collections::BTreeSet;
use std::fs;
use std::path::PathBuf;

use termigator_core::config::doc::Value;
use termigator_core::config::model::connection_protocol;
use termigator_core::import::moba::parse_moba_export;
use termigator_core::import::putty::{parse_putty_session_file, parse_superputty_xml};
use termigator_core::import::remmina::parse_remmina_profiles;
use termigator_core::import::ssh_config::parse_ssh_config;
use termigator_core::proto::rdp::build_rdp_command;
use termigator_core::proto::scp::build_scp_command_with;
use termigator_core::proto::ssh::{Env, build_ssh_command_with};
use termigator_core::proto::telnet::build_telnet_command;
use termigator_core::proto::vnc::build_vnc_command;
use termigator_core::shell;

/// Single shell words: an option for ssh / mosh / scp / telnet / vncviewer and one for
/// xfreerdp. The builders never emit these themselves.
const PAYLOADS: [&str; 3] = [
    "-oProxyCommand=PWNED",
    "/shell:PWNED",
    "-ProxyCommand=PWNED",
];

/// The same payloads behind a space, for fields a parser keeps whole.
fn values() -> Vec<String> {
    PAYLOADS
        .iter()
        .flat_map(|p| [(*p).to_owned(), format!("x {p}")])
        .collect()
}

fn env() -> Env {
    Env {
        which: Box::new(|n| Some(format!("/usr/bin/{n}"))),
        control_dir: None,
        home: Some("/home/n12".to_owned()),
    }
}

fn map(pairs: &[(&str, Value)]) -> Value {
    Value::Map(
        pairs
            .iter()
            .map(|(k, v)| (Value::Str((*k).to_owned()), v.clone()))
            .collect(),
    )
}

fn tmp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("termigator-n12-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&d);
    fs::create_dir_all(&d).expect("tmp dir");
    d
}

/// The shell lines the app would run for `entry`, or the builders' refusals.
fn lines(entry: &Value) -> Vec<Result<String, String>> {
    let launch = map(&[("ssh_keepalive", Value::Bool(false))]);
    let env = env();
    let all = |_: &str| true;
    match connection_protocol(entry) {
        "rdp" => vec![build_rdp_command(entry, &launch, &env)],
        "vnc" => vec![build_vnc_command(entry, &launch, &all)],
        "telnet" => vec![build_telnet_command(entry, &all)],
        "serial" => vec![],
        _ => {
            let mosh = match entry {
                Value::Map(m) => {
                    let mut m = m.clone();
                    m.push((Value::Str("use_mosh".to_owned()), Value::Bool(true)));
                    Value::Map(m)
                }
                other => other.clone(),
            };
            vec![
                build_ssh_command_with(entry, &launch, &env),
                build_ssh_command_with(&mosh, &launch, &env),
                build_scp_command_with(entry, "/tmp/local", "/remote/file", false, &env),
                build_scp_command_with(entry, "/tmp/local", "/remote/file", true, &env),
            ]
        }
    }
}

/// The protocols among `entries`, so a test sees that every kind it wrote was imported.
fn protocols(entries: &[Value]) -> BTreeSet<&'static str> {
    entries.iter().map(connection_protocol).collect()
}

/// Every built line keeps the payloads out of option position: no word that carries one starts
/// with `-` (any client; for mosh also the words of the `--ssh=` command), and for
/// xfreerdp none starts with `/shell` either (its own words are `/v:…`, `/u:…`).
fn assert_safe(source: &str, entries: &[Value]) {
    assert!(!entries.is_empty(), "{source}: nothing imported");
    for entry in entries {
        let rdp = connection_protocol(entry) == "rdp";
        for line in lines(entry).into_iter().flatten() {
            let mut words = shell::split(&line).expect("the builder's line splits");
            // mosh's `--ssh=` command is split into words again; assume the worst (whitespace).
            let inner: Vec<String> = words
                .iter()
                .filter_map(|w| w.strip_prefix("--ssh="))
                .flat_map(|c| c.split_whitespace().map(str::to_owned).collect::<Vec<_>>())
                .collect();
            words.retain(|w| !w.starts_with("--ssh="));
            words.extend(inner);
            for w in words.iter().filter(|w| w.contains("PWNED")) {
                let option = w.starts_with('-') || (rdp && w.starts_with("/shell"));
                assert!(!option, "{source}: {w:?} in `{line}` from {entry:?}");
            }
        }
    }
}

#[test]
fn mobaxterm_bookmarks() {
    let mut ini = String::from("[Bookmarks]\nSubRep=\n");
    for (i, v) in values().iter().enumerate() {
        ini.push_str(&format!("user{i}=#109#0%h.example%22%{v}%\n"));
        ini.push_str(&format!("host{i}=#109#0%{v}%22%u%\n"));
        ini.push_str(&format!("port{i}=#109#0%h.example%{v}%u%\n"));
        ini.push_str(&format!("rdp{i}=#91#4%{v}%3389%{v}%\n"));
        ini.push_str(&format!("vnc{i}=#128#5%{v}%5900%\n"));
        ini.push_str(&format!("telnet{i}=#98#1%{v}%23%{v}%\n"));
    }
    let (rows, _) = parse_moba_export(&ini, "me");
    assert_eq!(
        protocols(&rows),
        BTreeSet::from(["rdp", "ssh", "telnet", "vnc"])
    );
    assert_safe("MobaXterm", &rows);
}

#[test]
fn putty_sessions() {
    let mut got = Vec::new();
    for (i, v) in values().iter().enumerate() {
        // PuTTY imports SSH sessions only.
        for proto in ["ssh"] {
            for (field, rest) in [
                ("HostName", "UserName=u\n"),
                ("UserName", "HostName=h\n"),
                ("PortNumber", "HostName=h\nUserName=u\n"),
            ] {
                let text = format!("{field}={v}\n{rest}Protocol={proto}\n");
                got.extend(parse_putty_session_file(&format!("s{i}"), &text, "me"));
            }
        }
    }
    assert_eq!(protocols(&got), BTreeSet::from(["ssh"]));
    assert_safe("PuTTY", &got);
}

#[test]
fn superputty_sessions() {
    let mut xml = String::from("<ArrayOfSessionData>\n");
    for (i, v) in values().iter().enumerate() {
        // SuperPuTTY: SSH sessions only, the rest is skipped.
        for proto in ["SSH", "RDP", "VNC"] {
            xml.push_str(&format!(
                "<SessionData SessionId=\"a/{proto}{i}\" SessionName=\"n\" Host=\"{v}\" Port=\"22\" Proto=\"{proto}\" Username=\"{v}\" />\n"
            ));
        }
    }
    xml.push_str("</ArrayOfSessionData>\n");
    let rows = parse_superputty_xml(&xml);
    assert_eq!(protocols(&rows), BTreeSet::from(["ssh"]));
    assert_safe("SuperPuTTY", &rows);
}

#[test]
fn ssh_config_hosts() {
    let d = tmp("ssh-config");
    let mut text = String::new();
    for (i, v) in values().iter().enumerate() {
        text.push_str(&format!(
            "Host h{i}\n  HostName \"{v}\"\n  User u\nHost u{i}\n  HostName h\n  User \"{v}\"\nHost j{i}\n  HostName h\n  User u\n  ProxyJump \"{v}\"\nHost p{i}\n  HostName h\n  User u\n  Port \"{v}\"\n"
        ));
    }
    let path = d.join("config");
    fs::write(&path, text).expect("write config");
    let res = parse_ssh_config(&path, &d, "me");
    assert_safe("ssh_config", &res.connections);
    let _ = fs::remove_dir_all(&d);
}

#[test]
fn remmina_profiles() {
    let d = tmp("remmina");
    let mut paths = Vec::new();
    for (i, v) in values().iter().enumerate() {
        for proto in ["SSH", "RDP", "VNC"] {
            for (j, (server, user)) in [(v.as_str(), "u"), ("h", v.as_str())].iter().enumerate() {
                let p = d.join(format!("{proto}-{i}-{j}.remmina"));
                fs::write(
                    &p,
                    format!(
                        "[remmina]\nname=n{i}\nprotocol={proto}\nserver={server}\nusername={user}\n"
                    ),
                )
                .expect("write profile");
                paths.push(p);
            }
        }
    }
    let res = parse_remmina_profiles(&paths, &[], &d);
    assert_eq!(
        protocols(&res.connections),
        BTreeSet::from(["rdp", "ssh", "vnc"])
    );
    assert_safe("Remmina", &res.connections);
    let _ = fs::remove_dir_all(&d);
}
