//! The jump host's own key (`jump_identity_file`): `-F <private config>` before `-J`, the config
//! giving the jump hosts the key and including the user's and the system's ssh config; checked
//! against the real OpenSSH (`ssh -G`) when it is installed.

use std::path::{Path, PathBuf};

use termigator_core::config::doc::Value;
use termigator_core::proto::scp::build_scp_command_with;
use termigator_core::proto::ssh::{Env, build_ssh_command_with, jump_config_text, jump_flags};
use termigator_core::shell;

fn s(t: &str) -> Value {
    Value::Str(t.to_owned())
}

fn conn(pairs: &[(&str, Value)]) -> Value {
    Value::Map(pairs.iter().map(|(k, v)| (s(k), v.clone())).collect())
}

fn env() -> Env {
    Env {
        which: Box::new(|_| None),
        control_dir: None,
        home: None,
    }
}

/// A key file in a fresh directory (content not a recognised private key, so the path is used
/// as it is rather than a private copy).
fn key_file(name: &str) -> String {
    let dir = std::env::temp_dir().join(format!("termigator-jump-key-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("mkdir");
    let p = dir.join(name);
    std::fs::write(&p, "key\n").expect("write");
    p.to_string_lossy().into_owned()
}

/// The value after `flag` in `argv`.
fn after<'a>(argv: &'a [String], flag: &str) -> Option<&'a str> {
    argv.iter()
        .position(|a| a == flag)
        .and_then(|i| argv.get(i + 1))
        .map(String::as_str)
}

fn argv_of(c: &Value) -> Vec<String> {
    let line = build_ssh_command_with(c, &Value::Null, &env()).expect("command");
    shell::split(&line).expect("split")
}

#[test]
fn the_jump_key_goes_into_a_config_file_not_the_command() {
    let jump_key = key_file("jump_key");
    let own_key = key_file("own_key");
    let c = conn(&[
        ("host", s("target.internal")),
        ("user", s("me")),
        ("identity_file", s(&own_key)),
        ("proxy_jump", s("ops@bastion:2222,10.0.0.1")),
        ("jump_identity_file", s(&jump_key)),
    ]);
    let argv = argv_of(&c);
    let cfg = after(&argv, "-F").expect("-F");
    assert_eq!(after(&argv, "-J"), Some("ops@bastion:2222,10.0.0.1"));
    let at = |f: &str| argv.iter().position(|a| a == f).expect(f);
    assert!(at("-F") < at("-J"));
    // The target keeps its own key; the jump key is nowhere in the command.
    assert_eq!(after(&argv, "-i"), Some(own_key.as_str()));
    assert!(!argv.iter().any(|a| a.contains("jump_key")), "{argv:?}");
    let text = std::fs::read_to_string(cfg).expect("config file");
    assert!(
        text.starts_with("Host \"bastion\" \"10.0.0.1\"\n"),
        "{text}"
    );
    assert!(text.contains("IdentitiesOnly yes"), "{text}");
    assert!(text.contains("jump_key"), "{text}");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(cfg).expect("stat").permissions().mode();
        assert_eq!(mode & 0o077, 0, "config readable by others: {mode:o}");
    }
}

#[test]
fn without_a_jump_key_or_a_jump_host_nothing_changes() {
    let plain = conn(&[
        ("host", s("t")),
        ("user", s("me")),
        ("proxy_jump", s("bastion")),
    ]);
    assert_eq!(jump_flags(&plain, None).expect("flags"), ["-J", "bastion"]);
    let blank = conn(&[
        ("host", s("t")),
        ("user", s("me")),
        ("proxy_jump", s("bastion")),
        ("jump_identity_file", s("  ")),
    ]);
    assert_eq!(jump_flags(&blank, None).expect("flags"), ["-J", "bastion"]);
    // A key without a jump host is not used anywhere.
    let no_jump = conn(&[
        ("host", s("t")),
        ("user", s("me")),
        ("jump_identity_file", s(&key_file("unused"))),
    ]);
    assert!(jump_flags(&no_jump, None).expect("flags").is_empty());
    assert!(!argv_of(&no_jump).iter().any(|a| a == "-F"));
}

#[test]
fn a_bad_jump_key_is_refused() {
    let with = |key: &str, extra: Option<&str>| {
        let mut pairs = vec![
            ("host", s("t")),
            ("user", s("me")),
            ("proxy_jump", s("bastion")),
            ("jump_identity_file", s(key)),
        ];
        if let Some(e) = extra {
            pairs.push(("extra_ssh_args", s(e)));
        }
        build_ssh_command_with(&conn(&pairs), &Value::Null, &env())
    };
    let missing = format!("{}-missing", key_file("jk"));
    let err = with(&missing, None).expect_err("missing key");
    assert!(
        err.contains("Jump host key") && err.contains("not found"),
        "{err}"
    );
    // The user's own -F would replace ours (or ours theirs): refused instead of guessed.
    let err = with(&key_file("jk"), Some("-F /etc/other_config")).expect_err("-F");
    assert!(err.contains("-F"), "{err}");
}

#[test]
fn the_config_text_quotes_and_escapes() {
    let hops = vec![
        "u@bastion".to_owned(),
        "[fe80::1]:2222".to_owned(),
        "jump:22".to_owned(),
        "a@b@c".to_owned(),
    ];
    let text = jump_config_text(
        &hops,
        r"/k/50%\key",
        &[
            "/home/u/.ssh/config".to_owned(),
            "/etc/ssh/ssh_config".to_owned(),
        ],
    )
    .expect("text");
    assert_eq!(
        text,
        "Host \"bastion\" \"fe80::1\" \"jump\" \"c\"\n  IdentityFile \"/k/50%%\\\\key\"\n  IdentitiesOnly yes\n\
         Match all\nInclude \"/home/u/.ssh/config\"\nInclude \"/etc/ssh/ssh_config\"\n"
    );
    assert!(jump_config_text(&hops, "/k/a\"b", &[]).is_err());
    assert!(jump_config_text(&["x\"y".to_owned()], "/k", &[]).is_err());
}

#[test]
fn scp_uses_the_jump_key_too() {
    let jump_key = key_file("scp_jump_key");
    let c = conn(&[
        ("host", s("t")),
        ("user", s("me")),
        ("proxy_jump", s("bastion")),
        ("jump_identity_file", s(&jump_key)),
    ]);
    let local = std::env::temp_dir().join(format!("termigator-jump-scp-{}", std::process::id()));
    let line =
        build_scp_command_with(&c, &local.to_string_lossy(), "/tmp/x", false, &env()).expect("scp");
    let argv = shell::split(&line).expect("split");
    assert!(after(&argv, "-F").is_some(), "{argv:?}");
    assert_eq!(after(&argv, "-J"), Some("bastion"));
}

fn real_ssh() -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|d| d.join(if cfg!(windows) { "ssh.exe" } else { "ssh" }))
        .find(|p| p.is_file())
}

/// `ssh -F cfg -G host`: the identity files OpenSSH would use for `host`.
fn identities_for(ssh: &Path, cfg: &str, host: &str) -> Vec<String> {
    let out = std::process::Command::new(ssh)
        .args(["-F", cfg, "-G", host])
        .output()
        .expect("ssh -G");
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|l| l.strip_prefix("identityfile "))
        .map(str::to_owned)
        .collect()
}

#[test]
fn openssh_gives_the_key_to_the_jump_hosts_only() {
    let Some(ssh) = real_ssh() else {
        eprintln!("skipped: no ssh in PATH");
        return;
    };
    let jump_key = key_file("live_jump_key");
    let c = conn(&[
        ("host", s("target.internal")),
        ("user", s("me")),
        ("proxy_jump", s("ops@bastion,hop2:2200")),
        ("jump_identity_file", s(&jump_key)),
    ]);
    let argv = argv_of(&c);
    let cfg = after(&argv, "-F").expect("-F").to_owned();
    // OpenSSH runs itself again for every hop with the same -F, so each hop reads the block.
    for hop in ["bastion", "hop2"] {
        let ids = identities_for(&ssh, &cfg, hop);
        assert_eq!(
            ids.first().map(String::as_str),
            Some(jump_key.as_str()),
            "{hop}: {ids:?}"
        );
    }
    assert!(!identities_for(&ssh, &cfg, "target.internal").contains(&jump_key));
}

#[test]
fn tunnels_and_gateways_keep_the_jump_key_of_their_ssh_entry() {
    use termigator_core::proto::gateway::gateway_tunnel_argv;
    use termigator_core::proto::tunnels::tunnel_argv;
    let jump_key = key_file("hop_jump_key");
    let entry = conn(&[
        ("label", s("bastioned")),
        ("host", s("t")),
        ("user", s("me")),
        ("proxy_jump", s("bastion")),
        ("jump_identity_file", s(&jump_key)),
    ]);
    let tunnel = conn(&[
        ("via", s("bastioned")),
        ("forwards", Value::Seq(vec![s("D 1080")])),
    ]);
    let argv = tunnel_argv(&tunnel, Some(&entry), &Value::Null, &env()).expect("tunnel");
    assert!(after(&argv, "-F").is_some(), "tunnel: {argv:?}");
    let argv =
        gateway_tunnel_argv("rdp.internal", 3389, &entry, 40000, &Value::Null, &env()).expect("gw");
    assert!(after(&argv, "-F").is_some(), "gateway: {argv:?}");
}
