//! "Add to Hosts…" on a tab: secrets dropped, an existing entry found by label or by target,
//! and the update keeping what only the stored entry has.

use termigator_core::config::doc::{Value, load as yaml_load};
use termigator_core::hosttree::{matching_connection, merged_with_tab, without_secrets};

fn yaml(text: &str) -> Value {
    yaml_load(text).expect("yaml")
}

fn cfg() -> Value {
    yaml(
        r"
connections:
  - label: prod-db
    host: db.example.test
    user: admin
    port: 2222
    password: hunter2
    group: prod
  - label: Lab Switch
    protocol: telnet
    host: 10.0.0.9
  - label: console
    protocol: serial
    device: /dev/ttyUSB0
  - label: desk
    protocol: rdp
    host: desk.example.test
    user: alice
",
    )
}

#[test]
fn without_secrets_drops_passwords_at_any_depth() {
    let tab = yaml(
        r"
label: web
host: web.example.test
user: root
password: hunter2
passphrase: open sesame
password_env: WEB_PW
credential: shared
gateway:
  host: jump.example.test
  password: also-secret
tunnels:
  - name: t
    password: tunnel-secret
",
    );
    let out = without_secrets(&tab);
    assert_eq!(out.get("password"), None);
    assert_eq!(out.get("passphrase"), None);
    // A variable name and a credential reference are no secrets.
    assert_eq!(out.get("password_env"), Some(&Value::Str("WEB_PW".into())));
    assert_eq!(out.get("credential"), Some(&Value::Str("shared".into())));
    let gw = out.get("gateway").expect("gateway");
    assert_eq!(gw.get("password"), None);
    assert_eq!(
        gw.get("host"),
        Some(&Value::Str("jump.example.test".into()))
    );
    let Some(Value::Seq(tunnels)) = out.get("tunnels") else {
        panic!("tunnels")
    };
    assert_eq!(tunnels[0].get("password"), None);
    assert_eq!(tunnels[0].get("name"), Some(&Value::Str("t".into())));
}

#[test]
fn without_secrets_drops_sealed_tokens_under_any_key() {
    let token = format!("{}xyz", termigator_core::vault::SEALED_PREFIX);
    let tab = yaml(&format!("host: h\nuser: u\nother: \"{token}\"\n"));
    let out = without_secrets(&tab);
    assert_eq!(out.get("other"), None);
    assert_eq!(out.get("host"), Some(&Value::Str("h".into())));
}

#[test]
fn matches_by_label_case_folded() {
    let tab = yaml("label: LAB SWITCH\nhost: 192.0.2.1\nuser: x\n");
    assert_eq!(matching_connection(&cfg(), &tab), Some(1));
}

#[test]
fn matches_by_protocol_host_port_user() {
    // Quick connect: another label, the host in another case.
    let tab =
        yaml("label: admin@DB.example.test\nhost: DB.example.test\nuser: admin\nport: 2222\n");
    assert_eq!(matching_connection(&cfg(), &tab), Some(0));
}

#[test]
fn an_unset_port_is_the_protocol_default() {
    let tab = yaml("label: sw\nprotocol: telnet\nhost: 10.0.0.9\nport: 23\n");
    assert_eq!(matching_connection(&cfg(), &tab), Some(1));
    let rdp = yaml("label: x\nprotocol: rdp\nhost: desk.example.test\nuser: alice\nport: 3389\n");
    assert_eq!(matching_connection(&cfg(), &rdp), Some(3));
}

#[test]
fn a_different_port_user_or_protocol_is_no_match() {
    let c = cfg();
    let port = yaml("label: a\nhost: db.example.test\nuser: admin\n");
    assert_eq!(matching_connection(&c, &port), None);
    let user = yaml("label: b\nhost: db.example.test\nuser: root\nport: 2222\n");
    assert_eq!(matching_connection(&c, &user), None);
    let proto =
        yaml("label: c\nprotocol: telnet\nhost: db.example.test\nuser: admin\nport: 2222\n");
    assert_eq!(matching_connection(&c, &proto), None);
}

#[test]
fn serial_matches_by_device() {
    let tab = yaml("label: ttl\nprotocol: serial\ndevice: /dev/ttyUSB0\n");
    assert_eq!(matching_connection(&cfg(), &tab), Some(2));
}

#[test]
fn no_connections_no_match() {
    let tab = yaml("label: a\nhost: h\nuser: u\n");
    assert_eq!(matching_connection(&yaml("{}"), &tab), None);
}

#[test]
fn merged_keeps_the_stored_label_group_and_password() {
    let Some(Value::Seq(list)) = cfg().get("connections").cloned() else {
        panic!("list")
    };
    let tab = yaml(
        "label: admin@db.example.test\nhost: db.example.test\nuser: admin\nport: 2200\nterminal: external\npassword: from-tab\ngroup: other\n",
    );
    let out = merged_with_tab(&list[0], &tab);
    assert_eq!(out.get("label"), Some(&Value::Str("prod-db".into())));
    assert_eq!(out.get("group"), Some(&Value::Str("prod".into())));
    assert_eq!(out.get("password"), Some(&Value::Str("hunter2".into())));
    assert_eq!(out.get("port"), Some(&Value::Int(2200)));
    assert_eq!(out.get("terminal"), Some(&Value::Str("external".into())));
}
