//! Terminal session state machine per tab: startup commands, "Session stopped" overlay keys,
//! auto-reconnect with backoff, attention marks, disconnect notice.
//!
//! Pure and clock-free: the GUI feeds [`Event`]s with the current time as a [`Duration`] since any
//! fixed base (ARCHITECTURE.md §5), performs the returned [`Effect`]s and, after every call, arms one
//! single-shot timer for [`Session::next_deadline`] that delivers [`Event::Tick`]. Nothing sleeps.
//!
//! One `Session` lives as long as its tab. [`Effect::Restart`] / [`Effect::Reconnect`] ask the GUI to
//! put a new session in the same pane at the same index; the GUI then builds a new `Session`
//! (passing [`Effect::Reconnect::attempt`] as [`SessionSpec::reconnect_attempt`]). After a
//! replacement, [`Effect::CloseTab`] or [`Event::UserClosed`] the session ignores every event, so a
//! late timer or the exit of the killed process never acts on a tab that is gone.

use std::time::Duration;

use crate::text::snippets::{text_to_send, waiting_for_user_input};

/// Quiet period after the last output before startup commands are typed.
pub const STARTUP_QUIET: Duration = Duration::from_millis(800);
/// Startup commands are dropped when they could not be typed this long after the spawn.
pub const STARTUP_GIVE_UP: Duration = Duration::from_secs(30);
/// How much trailing output the password / host-key prompt guard looks at.
pub const STARTUP_TAIL_CHARS: usize = 512;
/// Auto-reconnect attempts before giving up.
pub const MAX_RECONNECT_ATTEMPTS: u32 = 5;
/// A session that lived longer than this restarts the reconnect backoff.
pub const HEALTHY_SESSION: Duration = Duration::from_secs(60);

/// Tab tooltip of a dead session.
pub const STOPPED_TOOLTIP: &str =
    "Session stopped — R restarts it, Enter closes the tab, S saves the output";
/// Tab tooltip once auto-reconnect is exhausted.
pub const GAVE_UP_TOOLTIP: &str = "Session ended — auto-reconnect gave up after 5 attempts";
/// Overlay headline once auto-reconnect is exhausted.
pub const GAVE_UP_HEADLINE: &str = "Session stopped — auto-reconnect gave up after 5 attempts";
/// Default overlay headline.
pub const STOPPED_HEADLINE: &str = "Session stopped";
/// Tray message title for a disconnect while the window is not active.
pub const DISCONNECT_TRAY_TITLE: &str = "Termigator — session ended";
/// Tooltip of a tab marked by a bell.
pub const BELL_TOOLTIP: &str = "Bell in this session";
/// Tooltip of a tab marked by new output.
pub const ACTIVITY_TOOLTIP: &str = "New output in this session";

/// Full text of the "Session stopped" overlay (shown as plain text: `<Enter>` is literal).
pub fn stopped_overlay_text(headline: &str) -> String {
    format!(
        "{headline}\n    –  Press R to restart session\n    –  Press <Enter> to exit tab\n    –  Press S to save terminal output to\u{a0}file"
    )
}

/// Tray message body for a disconnect notice.
pub fn disconnect_tray_body(label: &str) -> String {
    format!("{label} disconnected.")
}

/// Tab title while waiting for the next auto-reconnect.
pub fn retry_title(title: &str, seconds: u64) -> String {
    format!("{title} · retry in {seconds}s")
}

/// Backoff before auto-reconnect attempt `attempts + 1`: 3, 6, 12, 24, then 30 seconds.
pub fn reconnect_delay(attempts: u32) -> Duration {
    let secs = 3u64.saturating_mul(1u64.checked_shl(attempts).unwrap_or(u64::MAX));
    Duration::from_secs(secs.min(30))
}

/// Icons the session puts on its tab.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TabIcon {
    /// No icon.
    None,
    /// The process exited.
    Disconnected,
    /// A bell rang in the background.
    Bell,
    /// New output arrived in the background.
    Activity,
}

impl TabIcon {
    /// Freedesktop theme icon name (`None` has none). Activity is not `media-record`.
    pub fn theme_name(self) -> Option<&'static str> {
        match self {
            TabIcon::None => None,
            TabIcon::Disconnected => Some("network-disconnect"),
            TabIcon::Bell => Some("notifications"),
            TabIcon::Activity => Some("dialog-information"),
        }
    }
}

/// Keys of the "Session stopped" overlay.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StopKey {
    /// R: restart the session in place.
    R,
    /// Return or Enter: close the tab.
    Enter,
    /// S: save the terminal output (repeatable).
    S,
}

/// What happened to the tab. Every event comes with the current time (see [`Session::handle`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event<'a> {
    /// The process was started.
    Spawned,
    /// Raw PTY output.
    Output(&'a [u8]),
    /// A timer armed for [`Session::next_deadline`] fired (or any other wake-up).
    Tick,
    /// The process exited on its own or was hung up. `notify_on_disconnect` is the current
    /// `window.notify_on_disconnect` setting.
    Exited {
        /// `window.notify_on_disconnect` at the time of the exit.
        notify_on_disconnect: bool,
    },
    /// The user closed the tab (the GUI closes it itself).
    UserClosed,
    /// A key pressed in the focused terminal view (`Keys.priority: BeforeItem`). `modifier` is
    /// true when Ctrl, Alt or Meta is held.
    Key {
        /// Which overlay key.
        key: StopKey,
        /// Ctrl, Alt or Meta held.
        modifier: bool,
    },
    /// Restart requested outside the overlay (tab menu "Reconnect", disconnect notice).
    RestartRequested,
    /// The tab became the current tab of an active window.
    TabSeen,
    /// The terminal rang the bell.
    Bell {
        /// The tab is its pane's current tab.
        current_tab: bool,
        /// The window holding the tab (main or detached) is active.
        window_active: bool,
    },
    /// The terminal reported activity (Konsole: once per arming).
    Activity {
        /// The tab is its pane's current tab.
        current_tab: bool,
        /// The window holding the tab (main or detached) is active.
        window_active: bool,
    },
    /// The session log started (`Some(path)`) or stopped (`None`).
    SessionLog(Option<&'a str>),
}

/// What the GUI has to do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Effect {
    /// Type this text into the terminal.
    SendStartupCommands(String),
    /// Auto-reconnect armed: attempt `attempt` runs `after` from now ([`Effect::Reconnect`]).
    ScheduleReconnect {
        /// Delay before the attempt.
        after: Duration,
        /// Attempt number the replacement session gets.
        attempt: u32,
    },
    /// New tab text.
    UpdateTitle(String),
    /// New tab tooltip.
    SetTooltip(String),
    /// New tab icon.
    SetIcon(TabIcon),
    /// Red tab title while a session log is written.
    SetLogMarker(bool),
    /// Show the "Session stopped" overlay with this text; take focus only if it was in this tab
    /// or nowhere.
    ShowStoppedOverlay {
        /// Full overlay text ([`stopped_overlay_text`]).
        text: String,
    },
    /// Hide the overlay.
    HideStoppedOverlay,
    /// Replace the tab by a fresh session in the same pane and index (user restart).
    Restart {
        /// Running session log to keep appending to (no new automatic log then).
        log: Option<String>,
    },
    /// Replace the tab by a fresh session in the same pane and index (auto-reconnect).
    Reconnect {
        /// The replacement's [`SessionSpec::reconnect_attempt`].
        attempt: u32,
        /// Running session log to keep appending to (no new automatic log then).
        log: Option<String>,
    },
    /// Close the tab.
    CloseTab,
    /// Ask where to save the terminal output and write it.
    SaveOutput,
    /// Taskbar / window-manager urgency hint for the window holding the tab.
    AlertWindow,
    /// Re-arm Konsole activity monitoring.
    RearmActivity,
    /// Show the disconnect notice (and, when the window is not active, the tray message
    /// [`DISCONNECT_TRAY_TITLE`] / [`disconnect_tray_body`]).
    NotifyDisconnect {
        /// Tab title.
        label: String,
        /// How long the session lived.
        lived: Duration,
    },
}

/// What the tab was opened with.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SessionSpec {
    /// Tab title (`termigator_title`).
    pub title: String,
    /// The entry's `startup_commands`; blank means none.
    pub startup_commands: Option<String>,
    /// The entry has `auto_reconnect: true`.
    pub auto_reconnect: bool,
    /// The tab carries a connection (not a local shell): only those get a disconnect notice.
    pub has_connection: bool,
    /// Attempts already made by auto-reconnect (0 for a fresh tab).
    pub reconnect_attempt: u32,
    /// Session log carried over from the replaced tab.
    pub log_path: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Startup {
    commands: String,
    tail: String,
    quiet_at: Option<Duration>,
    give_up_at: Duration,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    Running,
    Stopped {
        overlay: bool,
    },
    Reconnecting {
        started: Duration,
        next_tick: Duration,
        attempt: u32,
        shown: u64,
    },
    Gone,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Attention {
    Bell,
    Activity,
}

/// State of one terminal tab.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Session {
    spec: SessionSpec,
    phase: Phase,
    spawned_at: Option<Duration>,
    startup: Option<Startup>,
    icon: TabIcon,
    attention: Option<(Attention, TabIcon)>,
    /// The tooltip of the dead session ([`STOPPED_TOOLTIP`] / [`GAVE_UP_TOOLTIP`]), which
    /// clearing a mark brings back instead of the log tooltip.
    stopped_tip: Option<&'static str>,
    delay: Duration,
}

impl Session {
    /// A tab that is about to spawn its process.
    pub fn new(spec: SessionSpec) -> Self {
        Session {
            spec,
            phase: Phase::Running,
            spawned_at: None,
            startup: None,
            icon: TabIcon::None,
            attention: None,
            stopped_tip: None,
            delay: Duration::ZERO,
        }
    }

    /// True once the tab is closed or replaced: every further event is ignored.
    pub fn is_gone(&self) -> bool {
        self.phase == Phase::Gone
    }

    /// Icon the tab should show now.
    pub fn icon(&self) -> TabIcon {
        self.icon
    }

    /// When the GUI should deliver the next [`Event::Tick`], if at all.
    pub fn next_deadline(&self) -> Option<Duration> {
        let startup = self
            .startup
            .as_ref()
            .map(|s| s.quiet_at.map_or(s.give_up_at, |q| q.min(s.give_up_at)));
        let reconnect = match self.phase {
            Phase::Reconnecting { next_tick, .. } => Some(next_tick),
            _ => None,
        };
        match (startup, reconnect) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        }
    }

    /// Apply `event` at time `now` and return what the GUI must do, in order.
    pub fn handle(&mut self, now: Duration, event: Event<'_>) -> Vec<Effect> {
        if self.phase == Phase::Gone {
            return Vec::new();
        }
        match event {
            Event::Spawned => self.spawned(now),
            Event::Output(data) => self.output(now, data),
            Event::Tick => self.tick(now),
            Event::Exited {
                notify_on_disconnect,
            } => self.exited(now, notify_on_disconnect),
            Event::UserClosed => {
                self.gone();
                Vec::new()
            }
            Event::Key { key, modifier } => self.key(key, modifier),
            Event::RestartRequested => {
                self.gone();
                vec![Effect::Restart {
                    log: self.spec.log_path.clone(),
                }]
            }
            Event::TabSeen => self.seen(),
            Event::Bell {
                current_tab,
                window_active,
            } => self.mark(Attention::Bell, current_tab, window_active),
            Event::Activity {
                current_tab,
                window_active,
            } => self.mark(Attention::Activity, current_tab, window_active),
            Event::SessionLog(path) => {
                self.spec.log_path = path.map(str::to_owned);
                vec![
                    Effect::SetLogMarker(path.is_some()),
                    Effect::SetTooltip(self.log_tooltip()),
                ]
            }
        }
    }

    fn gone(&mut self) {
        self.phase = Phase::Gone;
        self.startup = None;
    }

    /// The tooltip of the tab when no mark is on it.
    fn base_tooltip(&self) -> String {
        self.stopped_tip
            .map_or_else(|| self.log_tooltip(), str::to_owned)
    }

    fn log_tooltip(&self) -> String {
        self.spec
            .log_path
            .as_deref()
            .map_or_else(String::new, |p| format!("Logging to {p}"))
    }

    fn spawned(&mut self, now: Duration) -> Vec<Effect> {
        self.spawned_at = Some(now);
        self.startup = self
            .spec
            .startup_commands
            .as_ref()
            .filter(|c| !c.trim().is_empty())
            .map(|c| Startup {
                commands: c.clone(),
                tail: String::new(),
                quiet_at: None,
                give_up_at: now + STARTUP_GIVE_UP,
            });
        Vec::new()
    }

    fn output(&mut self, now: Duration, data: &[u8]) -> Vec<Effect> {
        if let Some(s) = self.startup.as_mut() {
            // The output is kept Latin-1-decoded: one char per byte.
            s.tail.extend(data.iter().map(|&b| char::from(b)));
            let extra = s.tail.chars().count().saturating_sub(STARTUP_TAIL_CHARS);
            if extra > 0 {
                let cut = s.tail.char_indices().nth(extra).map_or(0, |(i, _)| i);
                s.tail.drain(..cut);
            }
            s.quiet_at = Some(now + STARTUP_QUIET);
        }
        Vec::new()
    }

    fn tick(&mut self, now: Duration) -> Vec<Effect> {
        let mut out = Vec::new();
        if let Some(s) = self.startup.as_mut()
            && s.quiet_at.is_some_and(|q| q <= now && q < s.give_up_at)
        {
            s.quiet_at = None;
            // The user answers the prompt first; their answer's output re-arms the timer.
            if !waiting_for_user_input(&s.tail) {
                out.push(Effect::SendStartupCommands(text_to_send(&s.commands, true)));
                self.startup = None;
            }
        }
        if self.startup.as_ref().is_some_and(|s| s.give_up_at <= now) {
            self.startup = None;
        }
        if let Phase::Reconnecting {
            started,
            next_tick,
            attempt,
            shown,
        } = self.phase
            && next_tick <= now
        {
            let deadline = started + self.delay;
            if now >= deadline {
                self.gone();
                out.push(Effect::Reconnect {
                    attempt,
                    log: self.spec.log_path.clone(),
                });
                return out;
            }
            let rest = deadline - now;
            let left = rest.as_secs() + u64::from(rest.subsec_nanos() > 0);
            let elapsed = (now - started).as_secs();
            self.phase = Phase::Reconnecting {
                started,
                next_tick: started + Duration::from_secs(elapsed + 1),
                attempt,
                shown: left,
            };
            if left != shown {
                out.push(Effect::UpdateTitle(retry_title(&self.spec.title, left)));
            }
        }
        out
    }

    fn exited(&mut self, now: Duration, notify: bool) -> Vec<Effect> {
        if self.phase != Phase::Running {
            return Vec::new();
        }
        self.startup = None;
        self.icon = TabIcon::Disconnected;
        // The disconnect icon replaces a bell / activity mark: clearing that mark later must not
        // bring back the icon and tooltip of the session that was alive when it was set.
        self.attention = None;
        self.stopped_tip = Some(STOPPED_TOOLTIP);
        let mut out = vec![
            Effect::SetIcon(TabIcon::Disconnected),
            Effect::SetTooltip(STOPPED_TOOLTIP.to_owned()),
        ];
        let lived = self
            .spawned_at
            .map_or(Duration::ZERO, |t| now.saturating_sub(t));
        let headline = if self.spec.auto_reconnect {
            let attempts = if lived > HEALTHY_SESSION {
                0
            } else {
                self.spec.reconnect_attempt
            };
            if attempts < MAX_RECONNECT_ATTEMPTS {
                self.delay = reconnect_delay(attempts);
                let secs = self.delay.as_secs();
                self.phase = Phase::Reconnecting {
                    started: now,
                    next_tick: now + Duration::from_secs(1),
                    attempt: attempts + 1,
                    shown: secs,
                };
                out.push(Effect::UpdateTitle(retry_title(&self.spec.title, secs)));
                out.push(Effect::ScheduleReconnect {
                    after: self.delay,
                    attempt: attempts + 1,
                });
                return out;
            }
            self.stopped_tip = Some(GAVE_UP_TOOLTIP);
            out.push(Effect::SetTooltip(GAVE_UP_TOOLTIP.to_owned()));
            GAVE_UP_HEADLINE
        } else {
            STOPPED_HEADLINE
        };
        self.phase = Phase::Stopped { overlay: true };
        out.push(Effect::ShowStoppedOverlay {
            text: stopped_overlay_text(headline),
        });
        if notify && self.spec.has_connection {
            out.push(Effect::NotifyDisconnect {
                label: self.spec.title.clone(),
                lived,
            });
        }
        out
    }

    fn key(&mut self, key: StopKey, modifier: bool) -> Vec<Effect> {
        if self.phase != (Phase::Stopped { overlay: true }) || modifier {
            return Vec::new();
        }
        match key {
            StopKey::R => {
                self.gone();
                vec![
                    Effect::HideStoppedOverlay,
                    Effect::Restart {
                        log: self.spec.log_path.clone(),
                    },
                ]
            }
            StopKey::Enter => {
                self.gone();
                vec![Effect::HideStoppedOverlay, Effect::CloseTab]
            }
            StopKey::S => vec![Effect::SaveOutput],
        }
    }

    fn mark(&mut self, kind: Attention, current_tab: bool, window_active: bool) -> Vec<Effect> {
        let mut out = Vec::new();
        if kind == Attention::Bell && !window_active {
            out.push(Effect::AlertWindow);
        }
        if current_tab && window_active {
            return out; // the user is looking at it
        }
        let before = match self.attention {
            Some((Attention::Bell, _)) if kind == Attention::Activity => return out,
            Some((_, before)) => before,
            None => self.icon,
        };
        self.attention = Some((kind, before));
        let (icon, tip) = match kind {
            Attention::Bell => (TabIcon::Bell, BELL_TOOLTIP),
            Attention::Activity => (TabIcon::Activity, ACTIVITY_TOOLTIP),
        };
        self.icon = icon;
        out.push(Effect::SetIcon(icon));
        out.push(Effect::SetTooltip(tip.to_owned()));
        out
    }

    fn seen(&mut self) -> Vec<Effect> {
        let Some((_, before)) = self.attention.take() else {
            return Vec::new();
        };
        self.icon = before;
        vec![
            Effect::SetIcon(before),
            Effect::SetTooltip(self.base_tooltip()),
            Effect::RearmActivity,
        ]
    }
}
