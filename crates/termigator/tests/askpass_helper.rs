//! The binary as ssh's `SSH_ASKPASS` helper: no Qt, answers over the channel.
#![cfg(unix)]

use std::process::Command;

use termigator_core::proto::askpass::{self, Server};
use termigator_core::vault::Secret;

fn run_helper(path: &std::path::Path, token: &str, prompt: &str) -> (bool, Vec<u8>) {
    let out = Command::new(env!("CARGO_BIN_EXE_termigator"))
        .arg(prompt)
        .env(askpass::CHANNEL_ENV, path)
        .env(askpass::TOKEN_ENV, token)
        .output()
        .expect("run helper");
    (out.status.success(), out.stdout)
}

#[test]
fn helper_prints_the_password_and_refuses_other_prompts() {
    let dir = std::env::temp_dir().join(format!("tg-askpass-bin-{}", std::process::id()));
    let path = dir.join("ask.sock");
    let server = Server::start(&path).expect("server");
    server.add("tok", Secret::new(b"s3cret".to_vec()), None);

    assert_eq!(
        run_helper(&path, "tok", "u@h's password: "),
        (true, b"s3cret\n".to_vec())
    );
    assert_eq!(
        run_helper(&path, "tok", "Enter passphrase for key 'k': "),
        (false, Vec::new())
    );
    assert_eq!(
        run_helper(&path, "wrong", "u@h's password: "),
        (false, Vec::new())
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn helper_fails_when_nobody_listens() {
    let path = std::env::temp_dir().join(format!("tg-askpass-none-{}.sock", std::process::id()));
    assert_eq!(run_helper(&path, "tok", "Password: "), (false, Vec::new()));
}
