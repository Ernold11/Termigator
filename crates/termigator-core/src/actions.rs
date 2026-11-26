//! Action table: menus, shortcuts and their contexts (ADR-11).
//!
//! One static table ([`ACTIONS`]) is the single source of truth for the menu bar, the window's
//! shortcuts, Help → Keyboard shortcuts and the uniqueness test. It covers the menus, host-list
//! shortcuts, detached-window shortcuts and terminal keys.
//!
//! Key sequences use Qt's portable text (`Ctrl+Shift+T`, `Shift+Ins`). Entries of the same menu keep
//! table order; a submenu is an entry with [`Action::submenu`] whose children have
//! `menu == "<parent>/<submenu>"` (plain names, no `&`). Actions with `menu == ""` are not in a menu.
//!
//! Key routing: Ctrl+letter and the C0 keys always go to the
//! terminal ([`terminal_claims`]); every other combination fires application shortcuts first.

/// Where a shortcut is live.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Context {
    /// Anywhere in the main window.
    Window,
    /// Only while the host list has focus.
    HostList,
    /// Only while a terminal has focus (handled by the terminal itself, not in a menu).
    Terminal,
    /// Only in a detached tab window (the main window's shortcuts are not active there).
    DetachedWindow,
    /// Shown in a menu for discoverability; the terminal binds the keys itself.
    DisplayOnly,
}

/// One menu entry or shortcut.
#[derive(Clone, Copy, Debug)]
pub struct Action {
    /// Stable identifier, `<area>.<name>`.
    pub id: &'static str,
    /// Plain menu path, e.g. `"File"` or `"File/New connection"`; `""` = not in a menu.
    pub menu: &'static str,
    /// Display text with `&` mnemonics, (empty for bare shortcuts).
    pub text: &'static str,
    /// freedesktop icon name; `a|b` = first theme icon that exists.
    pub icon: &'static str,
    /// Primary key sequence first, alternatives after.
    pub keys: &'static [&'static str],
    pub context: Context,
    /// A separator precedes this entry in its menu.
    pub sep_before: bool,
    pub checkable: bool,
    /// This entry opens the submenu `<menu>/<plain text>`.
    pub submenu: bool,
    /// The terminal may consume these keys before the application sees them.
    pub terminal_consumes: bool,
    /// Tooltip / status tip.
    pub tip: &'static str,
}

const fn act(
    id: &'static str,
    menu: &'static str,
    text: &'static str,
    icon: &'static str,
    keys: &'static [&'static str],
) -> Action {
    Action {
        id,
        menu,
        text,
        icon,
        keys,
        context: Context::Window,
        sep_before: false,
        checkable: false,
        submenu: false,
        terminal_consumes: false,
        tip: "",
    }
}

impl Action {
    const fn sep(mut self) -> Self {
        self.sep_before = true;
        self
    }
    const fn check(mut self) -> Self {
        self.checkable = true;
        self
    }
    const fn sub(mut self) -> Self {
        self.submenu = true;
        self
    }
    const fn tip(mut self, tip: &'static str) -> Self {
        self.tip = tip;
        self
    }
    const fn ctx(mut self, context: Context) -> Self {
        self.context = context;
        self.terminal_consumes = matches!(context, Context::DisplayOnly | Context::Terminal);
        self
    }
    /// The plain name of this entry's submenu (text without mnemonics).
    fn submenu_path(&self) -> String {
        format!("{}/{}", self.menu, strip_mnemonics(self.text))
    }
}

/// Top-level menus in menu-bar order (with mnemonics).
pub const MENUS: &[&str] = &[
    "&File",
    "&Edit",
    "&View",
    "&Session",
    "&Tools",
    "Se&ttings",
    "&Help",
];

use Context::{DetachedWindow, DisplayOnly, HostList, Terminal};

/// Every action, in creation order (menu order within each menu).
pub const ACTIONS: &[Action] = &[
    // ---- File
    act(
        "file.new_tab",
        "File",
        "New &tab",
        "tab-new",
        &["Ctrl+Shift+T"],
    )
    .tip("Local shell in a new tab of the active pane"),
    act(
        "file.quick_connect",
        "File",
        "&Quick connect…",
        "network-connect",
        &["Ctrl+Shift+O"],
    )
    .tip("SSH to user@host[:port] without saving an entry"),
    act(
        "file.new_connection",
        "File",
        "&New connection",
        "list-add",
        &[],
    )
    .sub(),
    act(
        "file.new.ssh",
        "File/New connection",
        "&SSH…",
        "utilities-terminal",
        &["Ctrl+Shift+N"],
    ),
    act("file.new.rdp", "File/New connection", "&RDP…", "krdc", &[]),
    act("file.new.vnc", "File/New connection", "&VNC…", "krdc", &[]),
    act(
        "file.new.telnet",
        "File/New connection",
        "&Telnet…",
        "network-wired",
        &[],
    ),
    act(
        "file.new.serial",
        "File/New connection",
        "S&erial…",
        "network-modem",
        &[],
    ),
    act(
        "file.new.favorite",
        "File/New connection",
        "&Favorite command…",
        "bookmark-new",
        &[],
    )
    .sep(),
    act(
        "file.new.folder",
        "File/New connection",
        "F&older…",
        "folder-new",
        &[],
    ),
    act("file.import", "File", "&Import", "document-import", &[])
        .sub()
        .sep(),
    act(
        "file.import.mobaxterm",
        "File/Import",
        "&MobaXterm sessions…",
        "document-import",
        &[],
    )
    .tip("MobaXterm's own sessions (Windows) or a .mxtsessions / .ini export"),
    act(
        "file.import.putty",
        "File/Import",
        "&PuTTY sessions…",
        "document-import",
        &[],
    )
    .tip(
        "PuTTY's saved sessions (Windows: the registry; Linux: ~/.putty/sessions) or a .reg export",
    ),
    act(
        "file.import.superputty",
        "File/Import",
        "S&uperPuTTY sessions…",
        "document-import",
        &[],
    )
    .tip("SuperPuTTY's Sessions.xml (Documents\\SuperPuTTY) or another sessions .xml"),
    act(
        "file.import.remmina",
        "File/Import",
        "&Remmina profiles…",
        "document-import",
        &[],
    )
    .tip("SSH, SFTP, RDP and VNC profiles from ~/.local/share/remmina (no passwords)"),
    act(
        "file.import.ssh_config",
        "File/Import",
        "~/.&ssh/config hosts…",
        "document-import",
        &[],
    )
    .tip("Every Host alias of your OpenSSH client config"),
    act("file.export", "File", "&Export", "document-export", &[]).sub(),
    act(
        "file.export.ssh_config",
        "File/Export",
        "SSH hosts as ssh_&config…",
        "document-export",
        &[],
    )
    .tip("A file you can Include from ~/.ssh/config (no passwords)"),
    act(
        "file.export.backup",
        "File/Export",
        "&Back up settings (zip)…",
        "archive-insert",
        &[],
    )
    .tip("config.yaml + session.yaml in one zip, to move Termigator to another machine"),
    act(
        "file.open_config_folder",
        "File",
        "Open config &folder",
        "folder-open",
        &[],
    ),
    act(
        "file.quit",
        "File",
        "&Quit",
        "application-exit",
        &["Ctrl+Shift+Q"],
    )
    .sep()
    .tip("Close every session and tunnel and exit"),
    // ---- Edit (terminal-handled keys, shown for discoverability)
    act("edit.copy", "Edit", "&Copy", "edit-copy", &["Ctrl+Shift+C"]).ctx(DisplayOnly),
    act(
        "edit.paste",
        "Edit",
        "&Paste",
        "edit-paste",
        &["Ctrl+Shift+V", "Shift+Ins"],
    )
    .ctx(DisplayOnly),
    act(
        "edit.find",
        "Edit",
        "&Find…",
        "edit-find",
        &["Ctrl+Shift+F"],
    )
    .sep()
    .ctx(DisplayOnly)
    .tip("Search in terminal output"),
    act("edit.snippets", "Edit", "&Snippets", "insert-text", &[])
        .sub()
        .sep(),
    act(
        "edit.search_hosts",
        "Edit",
        "Search &hosts",
        "search|edit-find",
        &["Ctrl+Shift+H"],
    )
    .tip("Jump to the host list search (Ctrl+L there too, Ctrl+F on the list)"),
    act(
        "edit.copy_conn_command",
        "Edit",
        "Copy connection &command",
        "edit-copy",
        &[],
    )
    .tip("The ssh / xfreerdp line of the host selected in the list"),
    // ---- View
    act(
        "view.toolbar",
        "View",
        "Show &toolbar",
        "configure-toolbars",
        &[],
    )
    .check(),
    act(
        "view.sidebar",
        "View",
        "Show &sidebar",
        "view-sidetree",
        &["Ctrl+Shift+B"],
    )
    .check(),
    act(
        "view.fullscreen",
        "View",
        "F&ull screen",
        "view-fullscreen",
        &["Ctrl+Shift+F11", "F11"],
    )
    .check(),
    act("view.zoom_in", "View", "Zoom &in", "zoom-in", &["Ctrl++"])
        .sep()
        .ctx(DisplayOnly),
    act(
        "view.zoom_out",
        "View",
        "Zoom &out",
        "zoom-out",
        &["Ctrl+-"],
    )
    .ctx(DisplayOnly),
    act(
        "view.zoom_reset",
        "View",
        "&Reset zoom",
        "zoom-original",
        &["Ctrl+0"],
    )
    .ctx(DisplayOnly),
    act(
        "view.split_right",
        "View",
        "Split &right",
        "view-split-left-right",
        &["Ctrl+(", "Ctrl+Shift+E"],
    )
    .sep()
    .tip("Split the active pane into left and right halves"),
    act(
        "view.split_down",
        "View",
        "Split &down",
        "view-split-top-bottom",
        &["Ctrl+)", "Ctrl+Shift+D"],
    )
    .tip("Split the active pane into top and bottom halves"),
    act(
        "view.close_pane",
        "View",
        "Close &pane",
        "view-close",
        &["Ctrl+Shift+X"],
    )
    .tip("Close every tab in the active pane and remove the pane"),
    act(
        "view.next_tab",
        "View",
        "&Next tab",
        "go-next",
        &["Ctrl+Tab", "Ctrl+PgDown"],
    )
    .sep(),
    act(
        "view.prev_tab",
        "View",
        "Pre&vious tab",
        "go-previous",
        &["Ctrl+Shift+Tab", "Ctrl+PgUp"],
    ),
    // ---- Window-level tab switching (no menu entry)
    act("tabs.goto_1", "", "Tab 1", "", &["Alt+1"]),
    act("tabs.goto_2", "", "Tab 2", "", &["Alt+2"]),
    act("tabs.goto_3", "", "Tab 3", "", &["Alt+3"]),
    act("tabs.goto_4", "", "Tab 4", "", &["Alt+4"]),
    act("tabs.goto_5", "", "Tab 5", "", &["Alt+5"]),
    act("tabs.goto_6", "", "Tab 6", "", &["Alt+6"]),
    act("tabs.goto_7", "", "Tab 7", "", &["Alt+7"]),
    act("tabs.goto_8", "", "Tab 8", "", &["Alt+8"]),
    act("tabs.goto_last", "", "Last tab", "", &["Alt+9"]),
    // ---- Session (the active tab)
    act(
        "session.rename",
        "Session",
        "&Rename tab…",
        "edit-rename",
        &["Ctrl+Alt+S"],
    ),
    act(
        "session.duplicate",
        "Session",
        "D&uplicate tab",
        "tab-duplicate",
        &[],
    ),
    act(
        "session.reconnect",
        "Session",
        "Rec&onnect",
        "view-refresh",
        &[],
    )
    .tip("Close the session and start it again"),
    act(
        "session.detach",
        "Session",
        "Detac&h tab",
        "window-new",
        &["Ctrl+Shift+L"],
    )
    .tip("Move this tab into a window of its own (tab menu → Attach brings it back)"),
    act(
        "session.files",
        "Session",
        "Open &files (SFTP)",
        "folder-remote",
        &[],
    )
    .tip("This host's files in a tab of their own"),
    act(
        "session.log",
        "Session",
        "&Log output to file",
        "document-save",
        &[],
    )
    .sep()
    .check()
    .tip("Append this tab's output to a log file"),
    act(
        "session.record",
        "Session",
        "Re&cord (asciinema)",
        "media-record",
        &[],
    )
    .check()
    .tip("Record this tab as an asciinema .cast — play it back with “asciinema play”"),
    act(
        "session.multiexec",
        "Session",
        "&MultiExec — type into all remote sessions",
        "system-run",
        &["Ctrl+Shift+M"],
    )
    .sep()
    .check()
    .tip("Every key typed in one remote session goes to all of them"),
    act(
        "session.broadcast",
        "Session",
        "&Broadcast line…",
        "mail-send",
        &[],
    )
    .tip("Send one line + Enter to every live remote session"),
    act(
        "session.close_tab",
        "Session",
        "&Close tab",
        "tab-close",
        &["Ctrl+Shift+W", "Ctrl+F4"],
    )
    .sep(),
    // ---- Tools
    act("tools.tunnels", "Tools", "&Tunnels…", "network-vpn", &[])
        .tip("SSH tunnels running in the background, without a terminal"),
    act(
        "tools.test_connection",
        "Tools",
        "Test &connection…",
        "network-connect",
        &[],
    )
    .tip("Does the selected host answer? No login is attempted"),
    act(
        "tools.network",
        "Tools",
        "&Network tools",
        "network-wired",
        &[],
    )
    .sub(),
    act(
        "tools.net.ping",
        "Tools/Network tools",
        "Ping",
        "network-wired",
        &[],
    )
    .tip("For the host selected in the list"),
    act(
        "tools.net.trace",
        "Tools/Network tools",
        "Trace route",
        "network-workgroup|network-wired",
        &[],
    )
    .tip("For the host selected in the list"),
    act(
        "tools.net.dns",
        "Tools/Network tools",
        "DNS lookup",
        "edit-find",
        &[],
    )
    .tip("For the host selected in the list"),
    act(
        "tools.net.port",
        "Tools/Network tools",
        "Check port",
        "network-connect",
        &[],
    )
    .tip("For the host selected in the list"),
    act(
        "tools.net.scan",
        "Tools/Network tools",
        "Scan common ports",
        "security-high|network-wired",
        &[],
    )
    .tip("For the host selected in the list"),
    act(
        "tools.ssh_keys",
        "Tools",
        "SSH &keys…",
        "document-encrypt|dialog-password",
        &[],
    )
    .sep()
    .tip("Your SSH key pairs: generate, add to the agent, install on a host"),
    act(
        "tools.credentials",
        "Tools",
        "Cr&edentials…",
        "dialog-password",
        &[],
    )
    .tip("Named logins shared by many SSH / RDP entries"),
    act(
        "tools.master_password",
        "Tools",
        "&Master password…",
        "document-encrypt",
        &[],
    )
    .tip("Encrypt saved passwords with a master password"),
    // ---- Settings
    act(
        "settings.configure",
        "Settings",
        "&Configure Termigator…",
        "configure",
        &["Ctrl+Shift+,", "Ctrl+,"],
    ),
    // ---- Help
    act(
        "help.shortcuts",
        "Help",
        "&Keyboard shortcuts",
        "input-keyboard",
        &[],
    ),
    act(
        "help.host_list_tips",
        "Help",
        "&Host list tips",
        "help-contents",
        &[],
    ),
    act("help.about", "Help", "&About Termigator", "help-about", &[]).sep(),
    act(
        "help.about_qt",
        "Help",
        "About &Qt",
        "qtcreator|help-about",
        &[],
    ),
    // ---- Host list (QShortcut with widget scope)
    act("hostlist.connect", "", "", "", &["Return", "Enter"]).ctx(HostList),
    act("hostlist.move_up", "", "", "", &["Alt+Up"]).ctx(HostList),
    act("hostlist.move_down", "", "", "", &["Alt+Down"]).ctx(HostList),
    act("hostlist.delete", "", "", "", &["Delete"]).ctx(HostList),
    act("hostlist.rename", "", "", "", &["F2"]).ctx(HostList),
    act("hostlist.focus_search", "", "", "", &["Ctrl+F", "Ctrl+L"]).ctx(HostList),
    act("hostlist.context_menu", "", "", "", &["Shift+F10", "Menu"]).ctx(HostList),
    // In the search field: clear it; in the list: drop a pending cut, else the selection.
    act("hostlist.cancel", "", "", "", &["Esc"]).ctx(HostList),
    act("hostlist.toggle", "", "", "", &["Space"]).ctx(HostList),
    act("hostlist.select_all", "", "", "", &["Ctrl+A"]).ctx(HostList),
    act("hostlist.copy", "", "", "", &["Ctrl+C"]).ctx(HostList),
    act("hostlist.cut", "", "", "", &["Ctrl+X"]).ctx(HostList),
    act("hostlist.paste", "", "", "", &["Ctrl+V"]).ctx(HostList),
    act("hostlist.new_folder", "", "", "", &["Ctrl+N"]).ctx(HostList),
    // ---- Terminal keys the terminal binds itself and that have no menu entry
    act("terminal.zoom_in_equals", "", "", "", &["Ctrl+="]).ctx(Terminal),
    // ---- Detached tab window
    act("detached.close_tab", "", "", "", &["Ctrl+Shift+W"]).ctx(DetachedWindow),
    act("detached.attach", "", "", "", &["Ctrl+Shift+A"]).ctx(DetachedWindow),
    act("detached.new_tab", "", "", "", &["Ctrl+Shift+T"]).ctx(DetachedWindow),
    act("detached.multiexec", "", "", "", &["Ctrl+Shift+M"]).ctx(DetachedWindow),
    act("detached.fullscreen", "", "", "", &["F11"]).ctx(DetachedWindow),
];

/// The host-list rows of Help → Keyboard shortcuts (the `hostlist.*` keys, and File → New
/// connection → SSH, which goes into the selected folder while the list has focus).
const HOST_LIST_ROWS: [(&str, &str, &str); 13] = [
    (
        "Host list",
        "Connect / expand or collapse a folder",
        "Enter",
    ),
    ("Host list", "Expand or collapse a folder", "Space"),
    ("Host list", "Collapse / go to the parent folder", "Left"),
    ("Host list", "Expand / go to the first entry", "Right"),
    ("Host list", "Rename", "F2"),
    ("Host list", "Delete", "Delete"),
    ("Host list", "Copy / cut / paste", "Ctrl+C, Ctrl+X, Ctrl+V"),
    ("Host list", "New folder in the selected folder", "Ctrl+N"),
    (
        "Host list",
        "New SSH connection in the selected folder",
        "Ctrl+Shift+N",
    ),
    ("Host list", "Context menu", "Shift+F10, Menu"),
    ("Host list", "Cancel the cut / clear the selection", "Esc"),
    ("Host list", "Focus the search", "Ctrl+L, Ctrl+F"),
    ("Host list", "Move entry up / down", "Alt+Up, Alt+Down"),
];

/// The action with this id.
pub fn by_id(id: &str) -> Option<&'static Action> {
    ACTIONS.iter().find(|a| a.id == id)
}

/// The entries of the menu at `path` (plain names, e.g. `"File/Import"`), in menu order.
pub fn menu_actions(path: &str) -> impl Iterator<Item = &'static Action> {
    let path = path.to_owned();
    ACTIONS.iter().filter(move |a| a.menu == path)
}

/// The menu text without mnemonics (`"&File"` → `"File"`), as used in menu paths.
pub fn strip_mnemonics(text: &str) -> String {
    let mut out = String::new();
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '&' {
            if chars.peek() == Some(&'&') {
                out.push('&');
                chars.next();
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// `(menu path, action, keys)` for every menu action and window-level tab action with a shortcut:
/// what Help → Keyboard shortcuts lists.
pub fn shortcut_rows() -> Vec<(String, String, String)> {
    fn walk(path: &str, display: &str, rows: &mut Vec<(String, String, String)>) {
        for a in menu_actions(path) {
            if a.submenu {
                let sub = a.submenu_path();
                let shown = format!("{display} › {}", strip_mnemonics(a.text));
                walk(&sub, &shown, rows);
            } else if !a.keys.is_empty() {
                let plain = strip_mnemonics(a.text);
                rows.push((
                    display.to_owned(),
                    plain.trim_end_matches('…').to_owned(),
                    a.keys.join(", "),
                ));
            }
        }
    }
    let mut rows = Vec::new();
    for top in MENUS {
        let plain = strip_mnemonics(top);
        walk(&plain, &plain, &mut rows);
    }
    for a in ACTIONS {
        if a.menu.is_empty() && a.context == Context::Window && !a.keys.is_empty() {
            rows.push(("Tabs".into(), a.text.into(), a.keys.join(", ")));
        }
    }
    rows.extend(HOST_LIST_ROWS.map(|(m, a, k)| (m.into(), a.into(), k.into())));
    rows
}

const CTRL: u8 = 1;
const SHIFT: u8 = 2;
const ALT: u8 = 4;
const META: u8 = 8;

/// Split a single-chord sequence into modifier bits and the upper-cased key name.
fn parse_chord(seq: &str) -> (u8, String) {
    let mut mods = 0;
    let mut rest = seq;
    loop {
        let bit = [
            ("Ctrl+", CTRL),
            ("Shift+", SHIFT),
            ("Alt+", ALT),
            ("Meta+", META),
        ]
        .into_iter()
        .find(|(p, _)| rest.len() > p.len() && rest.starts_with(p));
        match bit {
            Some((p, b)) => {
                mods |= b;
                rest = &rest[p.len()..];
            }
            None => return (mods, rest.to_uppercase()),
        }
    }
}

/// Would the terminal consume this key sequence before application shortcuts see it?
///
/// True for Ctrl+letter, Ctrl+Space and Ctrl+`@ [ \ ] ^ _` (Shift allowed only with the symbols,
/// which need it on most layouts), and for the bare C0 keys Tab, Return, Enter, Backspace, Esc.
/// Everything else (Ctrl+Shift+letter, Ctrl+Tab, Alt+digit, F-keys, …) belongs to the application.
pub fn terminal_claims(seq: &str) -> bool {
    let (mods, key) = parse_chord(seq);
    let mut chars = key.chars();
    let single = match (chars.next(), chars.next()) {
        (Some(c), None) => Some(c),
        _ => None,
    };
    if mods == 0 {
        return matches!(
            key.as_str(),
            "TAB" | "RETURN" | "ENTER" | "BACKSPACE" | "ESC"
        );
    }
    if mods & (ALT | META) != 0 || mods & CTRL == 0 {
        return false;
    }
    match single {
        Some(c) if c.is_ascii_alphabetic() => mods == CTRL,
        Some(' ') => mods == CTRL,
        _ if key == "SPACE" => mods == CTRL,
        Some('@' | '[' | '\\' | ']' | '^' | '_') => true,
        _ => false,
    }
}

/// Two contexts can be active for the same key press.
fn overlap(a: Context, b: Context) -> bool {
    use Context::{
        DetachedWindow as D, DisplayOnly as O, HostList as H, Terminal as T, Window as W,
    };
    matches!(
        (a, b),
        (W, W)
            | (W, H)
            | (H, W)
            | (H, H)
            | (W, T)
            | (T, W)
            | (T, T)
            | (T, O)
            | (O, T)
            | (O, O)
            | (D, D)
    )
}

/// A key bound twice in contexts that can be active together.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Conflict {
    pub key: String,
    pub first: &'static str,
    pub second: &'static str,
}

/// Duplicate keys within overlapping contexts (a key repeated inside one action counts too).
pub fn conflicts() -> Vec<Conflict> {
    conflicts_in(ACTIONS)
}

/// [`conflicts`] for any table.
pub fn conflicts_in(table: &[Action]) -> Vec<Conflict> {
    let bound: Vec<(&Action, (u8, String))> = table
        .iter()
        .flat_map(|a| a.keys.iter().map(move |k| (a, parse_chord(k))))
        .collect();
    let mut out = Vec::new();
    for (i, (a, ka)) in bound.iter().enumerate() {
        for (b, kb) in &bound[i + 1..] {
            if ka == kb && overlap(a.context, b.context) {
                out.push(Conflict {
                    key: format!("{}{}", mods_text(ka.0), ka.1),
                    first: a.id,
                    second: b.id,
                });
            }
        }
    }
    out
}

fn mods_text(mods: u8) -> String {
    [
        (CTRL, "Ctrl+"),
        (SHIFT, "Shift+"),
        (ALT, "Alt+"),
        (META, "Meta+"),
    ]
    .into_iter()
    .filter(|(b, _)| mods & b != 0)
    .map(|(_, t)| t)
    .collect()
}
