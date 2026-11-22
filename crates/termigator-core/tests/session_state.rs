//! Session state machine the rules of startup commands, auto-reconnect,
//! attention and disconnect notice, and the "Session stopped" keys, as event sequences with an
//! injected clock.

use std::time::Duration;

use termigator_core::session_state::*;

fn ms(n: u64) -> Duration {
    Duration::from_millis(n)
}

fn secs(n: u64) -> Duration {
    Duration::from_secs(n)
}

fn spec() -> SessionSpec {
    SessionSpec {
        title: "box".into(),
        has_connection: true,
        ..Default::default()
    }
}

fn spawned(spec: SessionSpec) -> Session {
    let mut s = Session::new(spec);
    assert!(s.handle(Duration::ZERO, Event::Spawned).is_empty());
    s
}

fn with_startup(cmds: &str) -> Session {
    spawned(SessionSpec {
        startup_commands: Some(cmds.into()),
        ..spec()
    })
}

fn sent(effects: &[Effect]) -> Vec<String> {
    effects
        .iter()
        .filter_map(|e| match e {
            Effect::SendStartupCommands(t) => Some(t.clone()),
            _ => None,
        })
        .collect()
}

fn exit(s: &mut Session, now: Duration, notify: bool) -> Vec<Effect> {
    s.handle(
        now,
        Event::Exited {
            notify_on_disconnect: notify,
        },
    )
}

fn key(s: &mut Session, k: StopKey) -> Vec<Effect> {
    s.handle(
        secs(1),
        Event::Key {
            key: k,
            modifier: false,
        },
    )
}

fn titles(effects: &[Effect]) -> Vec<String> {
    effects
        .iter()
        .filter_map(|e| match e {
            Effect::UpdateTitle(t) => Some(t.clone()),
            _ => None,
        })
        .collect()
}

// ---- startup commands -------------------------------------------------------------------

#[test]
fn startup_commands_typed_after_login() {
    let mut s = with_startup("echo STARTED-$((40+2))");
    assert_eq!(s.next_deadline(), Some(secs(30)));
    s.handle(ms(100), Event::Output(b"Welcome\n$ "));
    assert_eq!(s.next_deadline(), Some(ms(900)));
    assert!(s.handle(ms(899), Event::Tick).is_empty());
    let out = s.handle(ms(900), Event::Tick);
    assert_eq!(sent(&out), ["echo STARTED-$((40+2))\n"]);
    // Typed once only.
    s.handle(ms(1000), Event::Output(b"$ "));
    assert!(sent(&s.handle(ms(5000), Event::Tick)).is_empty());
    assert_eq!(s.next_deadline(), None);
}

#[test]
fn startup_commands_wait_for_output_and_ignore_blank() {
    let mut s = with_startup("ls");
    // No output yet: nothing to time from, only the give-up deadline.
    assert!(s.handle(secs(5), Event::Tick).is_empty());
    let mut blank = spawned(SessionSpec {
        startup_commands: Some("  \n ".into()),
        ..spec()
    });
    blank.handle(ms(10), Event::Output(b"$ "));
    assert_eq!(blank.next_deadline(), None);
    assert!(blank.handle(secs(2), Event::Tick).is_empty());
}

#[test]
fn output_during_the_quiet_window_restarts_it() {
    let mut s = with_startup("uptime");
    s.handle(ms(0), Event::Output(b"Last login"));
    s.handle(ms(700), Event::Output(b"..."));
    assert!(s.handle(ms(800), Event::Tick).is_empty());
    assert_eq!(s.next_deadline(), Some(ms(1500)));
    s.handle(ms(1400), Event::Output(b"\n$ "));
    assert!(s.handle(ms(1500), Event::Tick).is_empty());
    assert_eq!(sent(&s.handle(ms(2200), Event::Tick)), ["uptime\n"]);
}

#[test]
fn startup_commands_wait_for_the_password_prompt() {
    let mut s = with_startup("echo STARTED");
    s.handle(ms(50), Event::Output(b"u@h's password: "));
    // Well past the quiet period: the prompt holds the commands back.
    assert!(s.handle(ms(850), Event::Tick).is_empty());
    assert_eq!(s.next_deadline(), Some(secs(30)));
    assert!(s.handle(ms(1200), Event::Tick).is_empty());
    // The user's answer changes the tail and re-arms the timer.
    s.handle(ms(2000), Event::Output(b"\r\nPW=[hunter2]\r\nbash-5.2$ "));
    assert_eq!(sent(&s.handle(ms(2800), Event::Tick)), ["echo STARTED\n"]);
}

#[test]
fn host_key_and_passphrase_prompts_block_too() {
    for prompt in [
        &b"Are you sure you want to continue connecting (yes/no/[fingerprint])? "[..],
        b"Enter passphrase for key '/home/u/.ssh/id_ed25519': ",
    ] {
        let mut s = with_startup("id");
        s.handle(ms(0), Event::Output(prompt));
        assert!(s.handle(ms(800), Event::Tick).is_empty());
    }
}

#[test]
fn prompt_guard_looks_only_at_the_last_512_chars() {
    let mut s = with_startup("id");
    // A prompt far back in the output does not matter once the tail moved on.
    s.handle(ms(0), Event::Output(b"password: "));
    let mut long = vec![b'x'; 600];
    long.extend_from_slice(b"\n$ ");
    s.handle(ms(100), Event::Output(&long));
    assert_eq!(sent(&s.handle(ms(900), Event::Tick)), ["id\n"]);
}

#[test]
fn startup_commands_give_up_after_30_seconds() {
    let mut s = with_startup("id");
    s.handle(ms(0), Event::Output(b"password: "));
    s.handle(ms(29_000), Event::Output(b"password: "));
    assert!(s.handle(secs(30), Event::Tick).is_empty());
    assert_eq!(s.next_deadline(), None);
    s.handle(secs(31), Event::Output(b"\n$ "));
    assert!(s.handle(secs(40), Event::Tick).is_empty());
}

#[test]
fn quiet_deadline_past_the_give_up_never_fires() {
    let mut s = with_startup("id");
    s.handle(ms(29_500), Event::Output(b"$ "));
    // Both due in a late tick: the give-up came first.
    assert!(s.handle(secs(31), Event::Tick).is_empty());
}

#[test]
fn startup_commands_dropped_when_the_session_ends() {
    let mut s = with_startup("id");
    s.handle(ms(0), Event::Output(b"$ "));
    exit(&mut s, ms(100), false);
    assert!(sent(&s.handle(ms(900), Event::Tick)).is_empty());
    assert_eq!(s.next_deadline(), None);
}

#[test]
fn startup_commands_keep_lines_and_crlf_becomes_lf() {
    let mut s = with_startup("cd /tmp\r\nls\n\n");
    s.handle(ms(0), Event::Output(b"$ "));
    assert_eq!(sent(&s.handle(ms(800), Event::Tick)), ["cd /tmp\nls\n"]);
}

// ---- session stopped overlay ----------------------------------------------------------

#[test]
fn session_end_shows_the_overlay_and_marks_the_tab() {
    let mut s = spawned(spec());
    let out = exit(&mut s, secs(5), false);
    assert_eq!(
        out,
        [
            Effect::SetIcon(TabIcon::Disconnected),
            Effect::SetTooltip(
                "Session stopped — R restarts it, Enter closes the tab, S saves the output".into()
            ),
            Effect::ShowStoppedOverlay {
                text: "Session stopped\n    –  Press R to restart session\n    –  Press <Enter> to exit tab\n    –  Press S to save terminal output to\u{a0}file".into()
            },
        ]
    );
    assert_eq!(s.icon(), TabIcon::Disconnected);
    assert_eq!(
        TabIcon::Disconnected.theme_name(),
        Some("network-disconnect")
    );
    // A second exit report (EIO + try_wait) changes nothing.
    assert!(exit(&mut s, secs(6), true).is_empty());
}

#[test]
fn overlay_keys() {
    let mut s = spawned(spec());
    exit(&mut s, secs(1), false);
    // S is repeatable and leaves the overlay up.
    assert_eq!(key(&mut s, StopKey::S), [Effect::SaveOutput]);
    assert_eq!(key(&mut s, StopKey::S), [Effect::SaveOutput]);
    // With Ctrl/Alt/Meta the keys are not the overlay's.
    let ctrl = Event::Key {
        key: StopKey::R,
        modifier: true,
    };
    assert!(s.handle(secs(1), ctrl).is_empty());
    assert_eq!(
        key(&mut s, StopKey::R),
        [Effect::HideStoppedOverlay, Effect::Restart { log: None }]
    );
    // Fires exactly once.
    assert!(key(&mut s, StopKey::R).is_empty());
    assert!(key(&mut s, StopKey::Enter).is_empty());

    let mut s = spawned(spec());
    exit(&mut s, secs(1), false);
    assert_eq!(
        key(&mut s, StopKey::Enter),
        [Effect::HideStoppedOverlay, Effect::CloseTab]
    );
    assert!(s.is_gone());
    assert!(key(&mut s, StopKey::Enter).is_empty());
}

#[test]
fn overlay_keys_do_nothing_on_a_live_session() {
    let mut s = spawned(spec());
    for k in [StopKey::R, StopKey::Enter, StopKey::S] {
        assert!(key(&mut s, k).is_empty());
    }
}

#[test]
fn session_log_indicator_and_carry_over_on_restart() {
    let mut s = spawned(spec());
    assert_eq!(
        s.handle(secs(1), Event::SessionLog(Some("/tmp/s.log"))),
        [
            Effect::SetLogMarker(true),
            Effect::SetTooltip("Logging to /tmp/s.log".into())
        ]
    );
    // R on "Session stopped" (or the tab's Reconnect) keeps appending to the same log.
    assert_eq!(
        s.handle(secs(2), Event::RestartRequested),
        [Effect::Restart {
            log: Some("/tmp/s.log".into())
        }]
    );
    let mut new = spawned(SessionSpec {
        log_path: Some("/tmp/s.log".into()),
        ..spec()
    });
    assert_eq!(
        new.handle(secs(3), Event::SessionLog(None)),
        [
            Effect::SetLogMarker(false),
            Effect::SetTooltip(String::new())
        ]
    );
}

#[test]
fn restart_keeps_one_auto_log() {
    // The replacement gets the running log, so the GUI starts no second automatic log.
    let mut s = spawned(SessionSpec {
        log_path: Some("/logs/box.log".into()),
        ..spec()
    });
    exit(&mut s, secs(1), false);
    assert_eq!(
        key(&mut s, StopKey::R)[1],
        Effect::Restart {
            log: Some("/logs/box.log".into())
        }
    );
}

// ---- disconnect notice ----------------------------------------------------------------

#[test]
fn disconnect_notice_rules() {
    struct Case {
        name: &'static str,
        connection: bool,
        setting: bool,
        auto_reconnect: bool,
        user_closed: bool,
        notified: bool,
    }
    let cases = [
        Case {
            name: "connection, setting on",
            connection: true,
            setting: true,
            auto_reconnect: false,
            user_closed: false,
            notified: true,
        },
        Case {
            name: "setting off (default)",
            connection: true,
            setting: false,
            auto_reconnect: false,
            user_closed: false,
            notified: false,
        },
        Case {
            name: "local shell",
            connection: false,
            setting: true,
            auto_reconnect: false,
            user_closed: false,
            notified: false,
        },
        Case {
            name: "user closed the tab",
            connection: true,
            setting: true,
            auto_reconnect: false,
            user_closed: true,
            notified: false,
        },
        Case {
            name: "auto-reconnect still retrying",
            connection: true,
            setting: true,
            auto_reconnect: true,
            user_closed: false,
            notified: false,
        },
    ];
    for c in cases {
        let mut s = spawned(SessionSpec {
            has_connection: c.connection,
            auto_reconnect: c.auto_reconnect,
            ..spec()
        });
        if c.user_closed {
            assert!(s.handle(secs(1), Event::UserClosed).is_empty());
        }
        let out = exit(&mut s, secs(42), c.setting);
        let notice = out
            .iter()
            .any(|e| matches!(e, Effect::NotifyDisconnect { .. }));
        assert_eq!(notice, c.notified, "{}", c.name);
    }
    let mut s = spawned(spec());
    assert_eq!(
        exit(&mut s, secs(42), true).last(),
        Some(&Effect::NotifyDisconnect {
            label: "box".into(),
            lived: secs(42)
        })
    );
    assert_eq!(DISCONNECT_TRAY_TITLE, "Termigator — session ended");
    assert_eq!(disconnect_tray_body("box"), "box disconnected.");
}

// ---- auto-reconnect --------------------------------------------------------------

fn auto(attempt: u32) -> Session {
    spawned(SessionSpec {
        auto_reconnect: true,
        reconnect_attempt: attempt,
        ..spec()
    })
}

#[test]
fn reconnect_schedule_3_6_12_24_30_then_stop() {
    let mut attempt = 0;
    for want in [3, 6, 12, 24, 30] {
        let mut s = auto(attempt);
        let out = exit(&mut s, secs(10), true);
        assert_eq!(titles(&out), [format!("box · retry in {want}s")]);
        assert!(out.contains(&Effect::ScheduleReconnect {
            after: secs(want),
            attempt: attempt + 1
        }));
        assert!(!out.iter().any(|e| matches!(
            e,
            Effect::ShowStoppedOverlay { .. } | Effect::NotifyDisconnect { .. }
        )));
        let fired = s.handle(secs(10 + want), Event::Tick);
        assert_eq!(
            fired.last(),
            Some(&Effect::Reconnect {
                attempt: attempt + 1,
                log: None
            })
        );
        attempt += 1;
    }
    let mut s = auto(attempt);
    let out = exit(&mut s, secs(10), true);
    assert_eq!(
        out[2..],
        [
            Effect::SetTooltip("Session ended — auto-reconnect gave up after 5 attempts".into()),
            Effect::ShowStoppedOverlay {
                text: stopped_overlay_text(
                    "Session stopped — auto-reconnect gave up after 5 attempts"
                )
            },
            Effect::NotifyDisconnect {
                label: "box".into(),
                lived: secs(10)
            },
        ]
    );
    assert_eq!(s.next_deadline(), None);
    // The gave-up overlay takes the keys like the plain one.
    assert_eq!(key(&mut s, StopKey::Enter)[1], Effect::CloseTab);
}

#[test]
fn reconnect_counter_resets_after_a_healthy_session() {
    let mut s = auto(4);
    let out = exit(&mut s, secs(61), false);
    assert_eq!(titles(&out), ["box · retry in 3s"]);
    assert!(out.contains(&Effect::ScheduleReconnect {
        after: secs(3),
        attempt: 1
    }));
    // Exactly 60 s is not "longer than 60 s".
    let mut s = auto(5);
    let out = exit(&mut s, secs(60), false);
    assert!(
        out.iter()
            .any(|e| matches!(e, Effect::ShowStoppedOverlay { .. }))
    );
}

#[test]
fn auto_reconnect_counts_down_on_the_tab() {
    let mut s = auto(1);
    let out = exit(&mut s, secs(5), false);
    assert_eq!(titles(&out), ["box · retry in 6s"]);
    assert_eq!(s.next_deadline(), Some(secs(6)));
    let mut seen = Vec::new();
    let mut fired = None;
    for t in 6..=11 {
        assert_eq!(s.next_deadline(), Some(secs(t)));
        let out = s.handle(secs(t), Event::Tick);
        seen.extend(titles(&out));
        if let Some(e) = out.iter().find(|e| matches!(e, Effect::Reconnect { .. })) {
            fired = Some((t, e.clone()));
        }
    }
    assert_eq!(
        seen,
        [5, 4, 3, 2, 1].map(|n| format!("box · retry in {n}s"))
    );
    assert_eq!(
        fired,
        Some((
            11,
            Effect::Reconnect {
                attempt: 2,
                log: None
            }
        ))
    );
    assert!(s.is_gone());
}

#[test]
fn countdown_tolerates_late_and_early_ticks() {
    let mut s = auto(0);
    exit(&mut s, secs(0), false);
    // A wake-up before the next second changes nothing.
    assert!(s.handle(ms(500), Event::Tick).is_empty());
    // A late tick shows the rounded-up rest and re-aligns to whole seconds.
    assert_eq!(
        titles(&s.handle(ms(1700), Event::Tick)),
        ["box · retry in 2s"]
    );
    assert_eq!(s.next_deadline(), Some(secs(2)));
    // Way late: straight to the reconnect.
    assert!(matches!(
        s.handle(secs(9), Event::Tick).last(),
        Some(Effect::Reconnect { attempt: 1, .. })
    ));
}

#[test]
fn reconnect_carries_the_session_log() {
    let mut s = spawned(SessionSpec {
        auto_reconnect: true,
        log_path: Some("/l/box.log".into()),
        ..spec()
    });
    exit(&mut s, secs(0), false);
    assert_eq!(
        s.handle(secs(3), Event::Tick).last(),
        Some(&Effect::Reconnect {
            attempt: 1,
            log: Some("/l/box.log".into())
        })
    );
}

#[test]
fn closing_tab_cancels_pending_auto_reconnect() {
    let mut s = auto(0);
    exit(&mut s, secs(0), false);
    assert!(s.handle(secs(1), Event::UserClosed).is_empty());
    assert_eq!(s.next_deadline(), None);
    assert!(s.handle(secs(4), Event::Tick).is_empty());
}

#[test]
fn late_auto_reconnect_of_closed_tab_is_ignored() {
    let mut s = spawned(spec());
    s.handle(secs(1), Event::UserClosed);
    // The reconnect timer firing after the close, the notice's "Reconnect" after the close,
    // and the exit of the hung-up process.
    assert!(s.handle(secs(9), Event::Tick).is_empty());
    assert!(s.handle(secs(9), Event::RestartRequested).is_empty());
    assert!(exit(&mut s, secs(9), true).is_empty());
}

#[test]
fn overlay_keys_ignored_while_reconnecting() {
    let mut s = auto(0);
    exit(&mut s, secs(0), false);
    assert!(key(&mut s, StopKey::R).is_empty());
    // The tab menu's Reconnect still restarts right away.
    assert_eq!(
        s.handle(secs(1), Event::RestartRequested),
        [Effect::Restart { log: None }]
    );
    assert!(s.handle(secs(3), Event::Tick).is_empty());
}

#[test]
fn replaced_session_exit_never_notifies() {
    let mut s = spawned(spec());
    s.handle(secs(1), Event::RestartRequested);
    assert!(exit(&mut s, secs(1), true).is_empty());
}

// ---- attention ----------------------------------------------------------------

const BACK: Event<'static> = Event::Activity {
    current_tab: false,
    window_active: true,
};
const BELL: Event<'static> = Event::Bell {
    current_tab: false,
    window_active: true,
};

#[test]
fn background_activity_then_bell_marks_tab_until_viewed() {
    let mut s = spawned(spec());
    assert_eq!(
        s.handle(secs(1), BACK),
        [
            Effect::SetIcon(TabIcon::Activity),
            Effect::SetTooltip("New output in this session".into())
        ]
    );
    assert_eq!(
        s.handle(secs(2), BELL),
        [
            Effect::SetIcon(TabIcon::Bell),
            Effect::SetTooltip("Bell in this session".into())
        ]
    );
    // Bell outranks activity.
    assert!(s.handle(secs(3), BACK).is_empty());
    assert_eq!(s.icon(), TabIcon::Bell);
    assert_eq!(
        s.handle(secs(4), Event::TabSeen),
        [
            Effect::SetIcon(TabIcon::None),
            Effect::SetTooltip(String::new()),
            Effect::RearmActivity
        ]
    );
    assert_eq!(s.icon(), TabIcon::None);
    // Seen again without a mark: nothing (no re-arm either).
    assert!(s.handle(secs(5), Event::TabSeen).is_empty());
}

#[test]
fn watched_tab_is_never_marked_but_bell_alerts_an_inactive_window() {
    let mut s = spawned(spec());
    let watched = Event::Activity {
        current_tab: true,
        window_active: true,
    };
    assert!(s.handle(secs(1), watched).is_empty());
    let bell_watched = Event::Bell {
        current_tab: true,
        window_active: true,
    };
    assert!(s.handle(secs(1), bell_watched).is_empty());
    // Current tab of an inactive window: alert and mark.
    let out = s.handle(
        secs(2),
        Event::Bell {
            current_tab: true,
            window_active: false,
        },
    );
    assert_eq!(out[0], Effect::AlertWindow);
    assert_eq!(out[1], Effect::SetIcon(TabIcon::Bell));
    // Background tab in the active window: marked without an alert.
    let mut s = spawned(spec());
    assert!(!s.handle(secs(1), BELL).contains(&Effect::AlertWindow));
}

#[test]
fn attention_follows_the_detached_window() {
    // The window holding the tab decides, the GUI reports it per tab.
    let mut watched_in_detached = spawned(spec());
    let ev = Event::Activity {
        current_tab: true,
        window_active: true,
    };
    assert!(watched_in_detached.handle(secs(1), ev).is_empty());
    let mut other = spawned(spec());
    let ev = Event::Activity {
        current_tab: true,
        window_active: false,
    };
    assert_eq!(
        other.handle(secs(1), ev)[0],
        Effect::SetIcon(TabIcon::Activity)
    );
    // Not the recording dot.
    assert_eq!(TabIcon::Activity.theme_name(), Some("dialog-information"));
    assert_eq!(TabIcon::Bell.theme_name(), Some("notifications"));
}

#[test]
fn clearing_attention_restores_the_dead_session_icon() {
    let mut s = spawned(spec());
    exit(&mut s, secs(1), false);
    assert_eq!(s.handle(secs(2), BELL)[0], Effect::SetIcon(TabIcon::Bell));
    assert_eq!(
        s.handle(secs(3), Event::TabSeen)[0],
        Effect::SetIcon(TabIcon::Disconnected)
    );
    // The overlay keys still work after the mark was cleared.
    assert_eq!(key(&mut s, StopKey::S), [Effect::SaveOutput]);
}

#[test]
fn clearing_attention_restores_the_log_tooltip() {
    let mut s = spawned(spec());
    s.handle(secs(1), Event::SessionLog(Some("/l/a.log")));
    s.handle(secs(2), BACK);
    assert_eq!(
        s.handle(secs(3), Event::TabSeen)[1],
        Effect::SetTooltip("Logging to /l/a.log".into())
    );
}

#[test]
fn activity_is_rearmed_after_viewing() {
    let mut s = spawned(spec());
    s.handle(secs(1), BACK);
    assert!(
        s.handle(secs(2), Event::TabSeen)
            .contains(&Effect::RearmActivity)
    );
    assert_eq!(
        s.handle(secs(3), BACK)[0],
        Effect::SetIcon(TabIcon::Activity)
    );
}

#[test]
fn closed_tab_gets_no_attention() {
    let mut s = spawned(spec());
    s.handle(secs(1), Event::UserClosed);
    assert!(s.handle(secs(2), BELL).is_empty());
    assert!(s.handle(secs(2), Event::TabSeen).is_empty());
}

#[test]
fn delays() {
    let got: Vec<u64> = (0..8).map(|n| reconnect_delay(n).as_secs()).collect();
    assert_eq!(got, [3, 6, 12, 24, 30, 30, 30, 30]);
    assert_eq!(reconnect_delay(200), secs(30));
    assert_eq!(retry_title("x", 7), "x · retry in 7s");
}
