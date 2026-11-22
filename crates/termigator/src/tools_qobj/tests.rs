use std::thread::sleep;
use std::time::Instant;

use termigator_core::config::doc;

use super::*;

fn cfg(yaml: &str) -> Value {
    doc::load(yaml).expect("test yaml")
}

fn shell(script: &str) -> Vec<String> {
    if cfg!(windows) {
        vec!["cmd".to_owned(), "/c".to_owned(), script.to_owned()]
    } else {
        vec!["sh".to_owned(), "-c".to_owned(), script.to_owned()]
    }
}

/// A process that keeps running, like a live tunnel.
fn sleeper() -> Vec<String> {
    shell(if cfg!(windows) {
        "ping -n 30 127.0.0.1 >nul"
    } else {
        "sleep 30"
    })
}

/// A process that prints `msg` on stderr and exits 255, like ssh refusing a forward.
fn fails_with(msg: &str) -> Vec<String> {
    shell(&if cfg!(windows) {
        format!("1>&2 echo {msg}& exit 255")
    } else {
        format!("echo '{msg}' >&2; exit 255")
    })
}

fn wait_for(runs: &mut Runs, name: &str, status: &str) {
    let end = Instant::now() + Duration::from_secs(8);
    while runs.state(name).0 != status {
        runs.poll();
        assert!(Instant::now() < end, "{name} stayed {:?}", runs.state(name));
        sleep(Duration::from_millis(20));
    }
}

#[test]
fn form_checks_follow_the_dialog_rules() {
    let taken = vec!["db".to_owned()];
    let ok = json!({"name": " web ", "via": "bastion", "forwards": "L 8080:h:80\n\n"});
    assert_eq!(form_error(&ok, &taken), None);
    let t = form_tunnel(&ok);
    assert_eq!(
        (t.name.as_str(), t.forwards.len(), t.autostart),
        ("web", 1, false)
    );
    let dup = json!({"name": "db", "via": "b", "forwards": "D 1080"});
    assert_eq!(
        form_error(&dup, &taken).as_deref(),
        Some("Give the tunnel a name not used by another tunnel.")
    );
    let no_via = json!({"name": "x", "via": " ", "forwards": "D 1080"});
    assert_eq!(
        form_error(&no_via, &taken).as_deref(),
        Some("Pick the SSH entry to tunnel through (add one first).")
    );
    let bad = json!({"name": "x", "via": "b", "forwards": "Q 1"});
    assert!(form_error(&bad, &taken).is_some());
    let none = json!({"name": "x", "via": "b", "forwards": ""});
    assert_eq!(
        form_error(&none, &taken).as_deref(),
        Some("A tunnel needs at least one forward.")
    );
}

#[test]
fn saving_keeps_the_four_keys_and_an_empty_list_removes_the_key() {
    let mut c = cfg("tunnels:\n- {name: a, via: b, forwards: ['L 1:h:2']}\nother: 1\n");
    let mut all = saved_tunnels(&c);
    all[0].autostart = true;
    store_tunnels(&mut c, &all);
    let j = c.to_json();
    assert_eq!(
        j["tunnels"],
        json!([{"name": "a", "via": "b", "forwards": ["L 1:h:2"], "autostart": true}])
    );
    store_tunnels(&mut c, &[]);
    assert!(c.get("tunnels").is_none() && c.get("other").is_some());
}

#[test]
fn resolving_needs_an_ssh_entry_and_asks_for_a_locked_vault() {
    let tunnel = tunnel_value(&SavedTunnel {
        name: "t".to_owned(),
        via: "bastion".to_owned(),
        forwards: vec!["L 1:h:2".to_owned()],
        autostart: false,
    });
    let c = cfg("connections: []\n");
    match resolve_tunnel(&c, &tunnel, None) {
        Err(Resolve::Failed(e)) => {
            assert!(e.starts_with("SSH entry 'bastion' not found"), "{e}")
        }
        _ => panic!("resolved without an entry"),
    }
    let c = cfg("connections:\n- {label: bastion, host: jump.example, user: j}\n");
    let (argv, env) = resolve_tunnel(&c, &tunnel, None).unwrap_or_default();
    assert!(argv.iter().any(|a| a == "-N") && argv.iter().any(|a| a == "BatchMode=yes"));
    assert!(env.is_empty());
    let sealed = "enc:v1:AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";
    let c = cfg(&format!(
        "vault: {{salt: abc, check: def}}\nconnections:\n- {{label: bastion, host: h, user: u, password: '{sealed}'}}\n"
    ));
    match resolve_tunnel(&c, &tunnel, None) {
        Err(Resolve::Locked) => {}
        Err(Resolve::Failed(e)) => panic!("failed: {e}"),
        Ok(_) => panic!("resolved with a locked vault"),
    }
}

#[test]
fn a_tunnel_runs_fails_and_stops() {
    let mut runs = Runs::default();
    let settle = Duration::from_millis(100);
    runs.start("ok", &sleeper(), &[], settle);
    runs.start(
        "bad",
        &fails_with("Could not request local forwarding."),
        &[],
        settle,
    );
    assert_eq!(runs.state("ok").0, "starting");
    wait_for(&mut runs, "ok", "running");
    wait_for(&mut runs, "bad", "failed");
    assert_eq!(runs.state("bad").1, "Could not request local forwarding.");
    assert_eq!(runs.running_count(), 1);
    assert!(!runs.stop("ok"));
    wait_for(&mut runs, "ok", "stopped");
    assert_eq!(runs.running_count(), 0);
    assert!(runs.stop("bad"), "a failed tunnel becomes stopped at once");
}

#[test]
fn stop_all_waits_for_the_processes() {
    let mut runs = Runs::default();
    runs.start("a", &sleeper(), &[], Duration::from_millis(50));
    runs.start("b", &sleeper(), &[], Duration::from_millis(50));
    let mut names = runs.stop_all();
    names.sort();
    assert_eq!(names, ["a", "b"]);
    assert_eq!(runs.running_count(), 0);
    runs.start("c", &["/nonexistent/ssh".to_owned()], &[], SETTLE);
    wait_for(&mut runs, "c", "failed");
    assert_eq!(runs.state("c").1, "cannot start '/nonexistent/ssh'");
}

#[test]
fn view_lists_status_and_summary() {
    let c =
        cfg("tunnels:\n- {name: a, via: b, forwards: ['L 1:h:2', 'D 1080'], autostart: true}\n");
    let runs = Runs::default();
    let rows = tunnels_view(&saved_tunnels(&c), &runs);
    assert_eq!(rows[0]["summary"], "L 1:h:2, D 127.0.0.1:1080");
    assert_eq!(rows[0]["statusText"], "Stopped");
    assert_eq!(rows[0]["autostart"], true);
}

#[test]
fn snippets_expand_per_session_like_golden() {
    let list = snippet_list(&json!({"snippets": [
            {"name": "", "command": "uptime\nwho"}, {"name": "x", "command": "  "},
            {"name": "f", "command": "ls {host}", "enter": false}]}));
    assert_eq!(list[0]["name"], "uptime");
    assert_eq!(list.as_array().map(Vec::len), Some(2));
    let conn = snippet_conn(&json!({"host": "h", "user": "u"}), "tab title");
    assert_eq!(
        (conn.host.as_str(), conn.label.as_str(), conn.port.as_str()),
        ("h", "tab title", "")
    );
    let with_label = snippet_conn(&json!({"label": "web", "port": 2222}), "tab");
    assert_eq!(
        (with_label.label.as_str(), with_label.port.as_str()),
        ("web", "2222")
    );
    let text = text_to_send(
        &expand("ssh {user}@{host}:{port} {?Q}", &conn, &HashMap::new()),
        true,
    );
    assert_eq!(text, "ssh u@h:22 \n");
}
