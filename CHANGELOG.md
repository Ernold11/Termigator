# Changelog

All notable changes to Termigator. The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/);
versions follow [Semantic Versioning](https://semver.org/).

## [0.4.1] - 2026-11-28

Fixes from an audit of the embedded terminal.

### Embedded terminal

- **Fixed a crash** of the whole application when searching without *Match case* in output that has a
  character whose lower case is longer or shorter (Turkish `İ`, the Kelvin sign) before a match.
- **Shift+Tab** reaches the program (reverse completion in shells, mc, dialog forms, vim); before, it sent
  nothing. `Ctrl+@` sends NUL, as in xterm.
- **Shift+PgUp / Shift+PgDn** page through the scrollback, and typing or pasting returns to the bottom, as in
  xterm and Konsole. In full-screen programs (the alternate screen) the program still gets these keys.
- htop's values, totals and thread counts are readable with the built-in Breeze scheme (its bright black was
  almost the background colour), and bold text in the eight ANSI colours is drawn in their bright twins.
- Faint text (`SGR 2`: zsh autosuggestions, git, systemd) is drawn dimmed.
- The selection shows over coloured cells (mc's panels, coloured lines) and over the empty cells a prompt
  skips; a single click clears it, and a **triple click** selects the whole line.
- Search highlights and Ctrl+click on links hit the right text after wide characters (CJK, emoji) and
  combining marks.
- The cursor is no longer drawn on the top-left cell while the view shows the scrollback.
- Text with Windows line ends (CRLF) is pasted as one Enter per line, not two.
- **Programs can copy to the clipboard** (OSC 52: vim, nvim, tmux, also over ssh). Settings → Clipboard →
  *Let programs copy to the clipboard* (on by default); programs can never read the clipboard.

## [0.4.0] - 2026-11-27

### Embedded terminal

- **See-through background**, as in Konsole: Settings → Appearance → *Background opacity* (30–100 %) and
  *Blur what shows through* (KDE). Only the terminal's default background lets the desktop show; text,
  coloured cells, tabs, toolbars and the side panel stay solid. Without a setting, the Konsole colour
  scheme's `Opacity` / `Blur` apply. One setting for every tab and detached window.
- Full-screen programs stay on the cell grid: a coloured blank cell (mc's panels) no longer shifts the
  rest of its row left, and box drawing and block elements are drawn as shapes, so frames meet their
  corners (k9s) whatever the font has.

### Window

- **One instance:** starting Termigator again (menu, desktop shortcut) brings the running window
  forward — from the tray, minimized or behind other windows — instead of a second copy; sessions,
  transfers and tunnels keep running. Several starts at once and a crashed instance are handled.
  Settings → Application → *Allow only one application instance* (on by default) allows several.
- The menu bar is hidden by default and its menus sit behind a **☰** button at the end of the toolbar,
  laid out as in KDE applications: frequent actions first (duplicate, reconnect, log, full screen, zoom),
  then Session, Tools, Settings and Help, and **More** with File, Edit and View. **View → Show menubar**
  (`Ctrl+Shift+M`) brings the classic menu bar back, as in Konsole; the terminal's context menu offers it
  too. Hiding the toolbar shows the menu bar, so the menus stay reachable.
- **MultiExec** moved to `Ctrl+Alt+M` (`Ctrl+Shift+M` now shows the menu bar).
- Windows: a lone `Alt` also shows the menu bar with its first menu open, as in Windows applications.
- The status bar is slimmer and hides with the side panel it describes. It shows the summary of the side
  tab in front: the host list, or the Files panel's status line (which no longer takes a line of the
  panel). Its right corner shows the version: *up to date*, or *Termigator X is available* with a link to
  the release page (from the last update check).
- The tray icon's menu no longer tries to open at start (a warning on Wayland).
- The main window loads again with Qt 6.10 (a type cycle in the settings dialog stopped it).

## [0.3.0] - 2026-11-26

### Hosts

- Folders are managed fully from the tree: rename only renames, **Move folder to…** picks a target, and
  **Delete folder…** asks where the entries and subfolders go (the parent, another folder, or deleted
  with it). An emptied folder stays.
- Keyboard in the tree: `F2` renames in place, `Delete` asks first, `Ctrl+C` / `Ctrl+X` / `Ctrl+V` copy
  and move hosts and folders (an in-memory clipboard, passwords never reach the system clipboard),
  `Ctrl+N` makes a folder, `Space` / `Left` / `Right` fold and move, `Esc` cancels. The menus show the keys.
- **Add to Hosts…** in a tab's context menu saves an open connection (without its secrets), or updates the
  entry it matches.

### Embedded terminal

- The multiline paste question fits its text instead of the window; `Enter` answers **Yes**, `Tab` moves
  between Yes and No.
- Dragging a selection past the top or bottom edge scrolls the terminal.
- The mouse wheel reaches programs on the alternate screen (vim, less, htop, k9s), as mouse reports or as
  arrow keys; `Shift` keeps it in the scrollback.
- Programs that track mouse buttons (vim `mouse=a`, tmux, mc) get drags; motions are reported once per
  cell; programs are told the real cell size in pixels (image tools such as chafa draw at the right size).
- The zoom stays with the tab through split, drag, move and detach.
- The text no longer touches the left edge or hides under the environment frame.
- A tab that ended after a bell or activity mark keeps its disconnect icon and tooltip.

### Connections

- The quick-connect field keeps a history (opens on click, `Down` or the dropdown button; `Shift+Delete`
  forgets an entry; length in Settings → Application, 0 = off). `user:password@host` is refused, so a
  password never reaches ssh's arguments or the history.
- A tunnel can have its own private key, and the jump host of an SSH entry its own key (**Jump host key**
  on the Forwarding tab); keys with a passphrase get **Add to agent…**. The SSH dialog tabs scroll.

### Files

- Both panes work from the keyboard: `Enter`, `Backspace` / `Alt+Up`, `Delete`, `F2`, `Ctrl+N`, `F5`,
  `Shift+F10`, and in the local pane the arrows, `Home` / `End`, `Page Up` / `Down` and `Ctrl+A`.

### Desktop

- With the tray icon on, closing the window asks: **Hide in Tray**, **Exit** or Cancel, with "Remember my
  choice" (Settings → Application → Close behavior). File → Quit and the tray's Exit always exit; a logout
  closes without asking. The tray menu has Show / Hide / Exit, and a double click restores the window.

### Fixes

- Windows: the tray shows Termigator's icon instead of a generic terminal prompt.
- With Qt 6.12.1, dragging the terminal's scroll handle no longer jumps towards the top.
- An incremental build could compile the QML against an older build's QObjects and crash; every build now
  compiles the QML like a clean one.

## [0.2.0] - 2026-11-22

First public release.

### Connections

- SSH (optionally over mosh), RDP (FreeRDP on Linux, `mstsc.exe` on Windows), VNC (TigerVNC), telnet and
  serial, plus favourite commands. Connection kinds without a client on the machine are greyed out.
- Saved connections in nested folders (`prod/web/eu`) with drag & drop, search (`Ctrl+Shift+H`),
  duplicate, quick connect (`Ctrl+Shift+O` or the toolbar field) and "copy the exact `ssh` command"
  (password masked).
- Labels for hosts and folders (inherited inside a folder), shown on the tab, with an optional coloured
  frame around the panel; one or two lines per host in the list.
- RDP starts with the screen's size by default; a host can override it.
- Test connection without logging in, network tools (ping, traceroute, DNS, port check, port scan).

### Embedded terminal

- Ghostty's VT engine (libghostty-vt) with tabs, nested split panes, detachable tabs, find, zoom,
  snippets, session logging, asciinema recording, MultiExec and broadcast to all live sessions.
- Follows your Konsole profile (colour scheme and font) unless you override it; every Ctrl+key
  combination reaches the program; middle-click pastes the mouse selection; bell sound.
- A stopped session keeps its tab and offers restart / close / save output; optional auto-reconnect with
  back-off, SSH keepalive and a disconnect notification.
- An external terminal (Konsole or any `launch.template`) per entry on Linux.

### Files

- SFTP-style sidebar that follows the active SSH tab and reuses its connection (OpenSSH ControlMaster, no
  second login), two-pane local / remote view, drag & drop transfers, a right-click menu in both panes,
  folder synchronisation with a plan you review first. Nothing is ever overwritten silently.

### SSH

- Jump hosts, local / remote / dynamic forwards, agent and X11 forwarding.
- Key manager: generate, copy, add to the agent, install on a host. The SSH editor warns about a public
  key or a PuTTY `.ppk` in the key field.
- A key that other users can read (a too open mode on Linux, an inherited ACL on Windows) still logs in:
  Termigator passes ssh a private copy, says how to fix the file and deletes the copy on exit.

### Secrets

- Saved passwords stay in plain text (files readable only by you) unless you set a master password
  (scrypt N=2^17 + AES-256-GCM); it is asked once per run and only when needed. Each encrypted password
  is bound to its connection, so it cannot be copied to another entry.
- The SSH and RDP editors say when a typed password will be saved without encryption.

### Import / export

- MobaXterm, PuTTY, SuperPuTTY (Windows), Remmina (Linux) and `~/.ssh/config`; backup export and an
  OpenSSH client config export.

### Desktop

- KDE look and feel (the same look on other desktops), global show / hide with `termigator --toggle`,
  `termigator --show`, `--version` and `--help`.
- Windows 11: ConPTY terminal, local shells (cmd, PowerShell, WSL), SSH password logins without
  `sshpass`, serial through `plink`, sounds; installer per user (no administrator rights) or for all users.
- Update check: once a day at startup a banner shows when a newer release is on GitHub (**Skip this
  version** hides it for good). Nothing is downloaded. Chosen in the welcome window on the first start,
  later in Settings → Configure Termigator… → Application.
- A configuration that cannot be read (a YAML error) shows the file, the line and an **Open folder**
  button instead of quitting; nothing is changed. Without a display it ends with an error message.

### Security

- Importing an `ssh_config` other than your own `~/.ssh/config` leaves out `ProxyCommand`,
  `LocalCommand` and the other options that run programs on connect, as well as `ForwardAgent` and
  `ForwardX11`, and lists the hosts it changed.
- Through a jump host the saved password is given only to the target host, never to the jump host's
  prompt (needs OpenSSH 8.4 or newer). Jump hosts with spaces and host names starting with `-` are refused;
  imported entries pass the same argument checks as typed ones.
- Configuration, session logs, recordings and runtime files are readable by your account only (Linux and
  Windows); SSH tunnels and transfers end together with Termigator, also after a crash.

### Packages

- Ubuntu 26.04 `.deb` (system Qt 6 / KF6), Linux Mint 22.x / Ubuntu 24.04 `.deb` (bundled Qt / KF6 in
  `/opt/termigator`), AppImage, Arch Linux, Windows 11 installer.
- Every package carries the licence texts of what it bundles; the sources of the bundled LGPL / GPL
  libraries are published with each release (`termigator-<version>-linux-sources.tar`,
  `termigator-<version>-windows-sources.tar`). Package builds check the checksums of every tool and source
  they download; the AppImage runtime is a pinned, checksummed release.
