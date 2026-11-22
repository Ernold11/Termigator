# Changelog

All notable changes to Termigator. The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/);
versions follow [Semantic Versioning](https://semver.org/).

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
