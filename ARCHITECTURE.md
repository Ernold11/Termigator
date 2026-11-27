# Termigator architecture

Termigator is an SSH/RDP/VNC/telnet/serial launcher with an embedded terminal, built for KDE Plasma.
It is written in Rust (edition 2024) with Qt 6 through cxx-qt 0.10, QML (Kirigami, QQC2
`org.kde.desktop`) and KDE Frameworks 6. It runs on Linux and on Windows 11.

ADR numbers in this document (ADR-n) are cited from code comments; keep them stable.

## 1. Principles

- All business logic is in Rust. C++ is limited to the terminal bridge, a few KDE APIs, and thin
  wrappers (`*_native.{h,cpp}` in the binary crate) around Qt APIs that cxx-qt does not expose
  (for example `QMessageBox`, `QIcon::fromTheme`, `QQuickStyle`). A wrapper only shows or reads
  something; decisions are made in Rust or QML.
- External tools do the protocol work (ADR-4); Termigator builds their command lines.
- Native look on KDE Plasma 6: Breeze light/dark, theme icons, native context menus and dialogs.
- Everything blocking runs off the GUI thread.

## 2. Repository layout

```
Cargo.toml                    workspace (edition 2024), shared dependencies and lints
crates/
  termigator-core/            all logic; no Qt; #![forbid(unsafe_code)]
    src/
      paths.rs                config dir (XDG / %APPDATA% / TERMIGATOR_CONFIG_DIR), private runtime dir
      platform/               mod.rs, unix.rs, windows.rs: the only OS-specific code in core
      shell.rs                shlex-compatible quote/join/split
      config/                 doc.rs (YAML 1.1 -> Value), save.rs (strip defaults, skip-if-unchanged,
                              atomic write, comment preservation), model.rs (typed accessors, defaults),
                              groups.rs
      vault.rs, credentials.rs
      proto/                  ssh, scp, rdp, gateway, vnc, telnet, serial, tunnels, nettools, ssh_keys,
                              konsole (external terminal template)
      connectivity.rs         endpoint_for, first_hop, DNS/ping/TCP/banner probe
      text/                   session_log, asciicast, osc7, distro, snippets
      remote/                 listing.rs (argv + byte-level ls/find parsers), transfers.rs (meter, queue),
                              sync.rs (planner + guards), panel.rs (generations, tabs, edit watcher)
      import/                 moba, putty, remmina, ssh_config
      export.rs, bulk.rs
      konsole.rs              Konsole profile reader (konsolerc -> profile -> colour scheme/font), font fallback
      hosttree.rs             flattened host tree, filter, Recent, drop zones, mutations
      panes.rs                split/tab tree, `last_session` snapshot
      session_state.rs        session state machine: reconnect (backoff), attention, startup commands, close
      multiexec.rs            MultiExec targets, broadcast, per-target snippet expansion
      actions.rs              action table: menus, shortcuts, contexts, shortcuts dialog
      single_instance.rs      `toggle|show [TOKEN]` protocol, control socket server/client
      proc/                   pty.rs (spawn, env, secrets, resize, wait, kill), runner.rs (command with
                              timeout), transfer.rs (scp under a pty, sftp reget), tunnel.rs (ssh -N),
                              external.rs (external terminal, xdotool)
    tests/                    integration tests (stub ssh/socat/sshpass; `live_*` use real OpenSSH)
  termigator-term-bridge/     C++/Qt glue + cxx FFI; terminal engine in Rust on ghostty-vt
    src/engine/               Engine/Snapshot facade over ghostty-vt, key and mouse encoders
    src/session.rs            SessionEngine: snapshots, history, search, colour schemes, paste, URLs
    src/                      ghostty_session.{h,cpp} (session QObject), spike_item.{h,cpp} (QML view),
                              multiexec.{h,cpp}, probe.{h,cpp} (smoke tests), kde.{h,cpp} (KGlobalAccel,
                              activation token), register.cpp (QML type registration), lib.rs (cxx bridge)
    color-schemes/            built-in colour schemes (original palettes, MIT), resource :/termigator/color-schemes
    build.rs                  cc + moc + rcc (no CMake)
  termigator/                 the binary
    src/main.rs               CLI (--toggle/--show before QApplication), startup, org.kde.desktop style
    src/lib.rs, platform.rs   application wiring; terminal I/O attach/detach/write (section 4.1)
    src/*_qobj.rs             thin cxx-qt QObjects over core controllers (sidebar, panes, terminal, files,
                              transfers, shell, preferences, vault, import, tools, editors)
    src/*_native.{h,cpp}      Qt glue without business logic (shell, preferences, terminal, message, bell)
    src/rdp_windows.rs        RDP on Windows through mstsc.exe
    src/bin/                  auxiliary binaries (e.g. termigator-import-moba)
    qml/                      Main.qml and one directory per area (sidebar, panes, terminal, files,
                              dialogs, settings, components)
    tests/                    GUI smoke tests (*_smoke.rs, shared launcher in tests/common/mod.rs)
compat/golden/                golden JSON fixtures (plain test data)
scripts/, packaging/          packaging and maintenance scripts (section 9)
```

## 3. Layers and dependencies

```
            +----------------------------- termigator (bin) ------------------------------+
            |  qml/ (Kirigami, QQC2 org.kde.desktop, Qt.labs.platform, QtQuick.Dialogs)    |
            |        ^ properties/signals, QAbstractListModel, JSON                        |
            |  *_qobj.rs (cxx-qt, thin) -------------+                                     |
            +--------------+-------------------------+-------------------------------------+
                           | Rust calls              | cxx FFI
            +--------------v-------------+   +-------v--------------------------------+
            | termigator-core (no Qt)    |<--+ termigator-term-bridge (C++/Qt)        |
            | logic, models, processes,  |   | session object, terminal view,         |
            | PTY (portable-pty)         |   | ghostty-vt engine, MultiExec,          |
            +----------------------------+   | KGlobalAccel, activation token         |
                                             +----------------------------------------+
```

Dependencies point only downward.

- `termigator-core` depends on neither Qt nor the bridge and is fully tested with `cargo test`
  without a display.
- `termigator-term-bridge` calls Rust only through a narrow `extern "Rust"` interface (terminal
  output tap, session end) and provides QML types (`Termigator.Terminal`).
- `termigator` owns `QApplication`, registers the QML modules and holds the core controllers.

## 4. Key flows

### 4.1 Terminal

The emulation runs in Rust (Ghostty's VT engine, `ghostty-vt` crate wrapping libghostty-vt, pinned
exactly because its C ABI is unstable). The bridge session draws from engine snapshots.

```
 Rust: proc::pty::spawn(argv, env) --> Child + PTY        (bash -lc <cmd> | <shell> -l)
                                           |
  Linux (descriptor mode):  C++ GhosttySession duplicates the master fd, reads it with a
                            QSocketNotifier on the GUI thread
  Windows (feed mode):      a Rust reader thread reads ConPTY and feeds the session
                            (platform.rs: attach / detach / write)
                                           |  bytes
                                           +--> extern "Rust" on_output(id, &[u8])   (tap, see below)
                                           +--> engine -> snapshot -> terminal view (QML item)
 Replies from the program (DSR/DA, OSC 52 answers) come back from the engine and are written to the pty.
```

- **Tap.** Every chunk goes to Rust first: session log, asciicast, distribution sniffing (first
  64 KiB), OSC 7 (cwd), the startup-command silence timer and the password/host-key prompt guard.
- **Input.** Keys and mouse are encoded by the engine and written to the pty. MultiExec: a C++ hub
  replays key events on the target sessions (reentrancy-protected; targets are chosen by
  `core::multiexec`).
- **Resize.** The view reports its size; the pty is resized (TIOCSWINSZ / ConPTY resize) and an `r`
  event is written to asciicast.
- **End.** `try_wait` every second plus EIO/EOF feed `session_state` (overlay "Session stopped",
  reconnect with backoff).
- **Feed mode details.** `platform::feeds()` is true on Windows, and on Unix when
  `TERMIGATOR_PTY_FEED=1` (so the mode can be tested on Linux). Input from the bridge is queued to a
  writer thread, never written on the GUI thread. ConPTY does not close the output pipe when the
  child exits, so `detach` (tab closed or child reaped) drops the writer, which closes its half and
  lets the reader see EOF.
- **Ownership.** The session (bridge session object plus Rust session) belongs to the tab
  controller. QML terminal views are created and destroyed by QML; split, detach and attach bind a
  new view to the existing session (`session` is a writable property), so scrollback and the
  process survive the move.

### 4.2 Configuration

```
load:  config.yaml + session.yaml -> event parser -> YAML 1.1 resolver -> Value (ordered maps)
       -> legacy migration -> merge_defaults -> typed accessors
save:  merge_defaults -> strip_defaults_for_save -> (vault configured?) drop plain-text passwords
       -> compare with the file on disk (skip-if-unchanged)
       -> session.yaml: emitter byte-compatible with PyYAML safe_dump (header, no anchors, allow_unicode)
       -> config.yaml: yaml-edit (maps synced key by key, lists/scalars replaced) | fallback: byte ranges
       -> atomic write: temp file in the same directory, fsync, keep mode/symlink, rename, fsync directory
```

### 4.3 Files panel

`core::remote::panel` keeps a target generation. Every command result, drag-and-drop payload and
post-upload refresh carries `(generation, cwd)` and is dropped when it no longer matches. Remote
names are `Vec<u8>` from listing through transfers and sync; QML sees only a row id and
`display_name`.

## 5. Concurrency model

| Work | Where | Mechanism |
|---|---|---|
| Terminal I/O, tap to Rust | GUI thread (Linux) / reader thread feeding the session (Windows) | QSocketNotifier + synchronous Rust call / feed threads |
| Session liveness, reconnect, startup commands | GUI thread | QTimer driven by `session_state` (clock injected in tests) |
| ssh listing, mkdir/rename/delete/chmod | worker thread | `proc::runner` (killable, timeout) -> `CxxQtThread::queue` |
| scp/sftp transfers | one thread per transfer (parallelism limit 2) | `proc::transfer` under a pty -> progress events |
| `ssh -N` tunnels | one supervisor thread per tunnel | `proc::tunnel` (stop does not block the GUI) |
| Connection probe, network tools, ssh-keygen/ssh-add, xdotool | worker thread | result through the Qt queue |
| `--toggle/--show` socket | listener thread | listener -> Qt queue |
| Edit watcher | GUI thread | QFileSystemWatcher on the QML/QObject side, 700 ms debounce |

There is no async runtime (ADR-6). Every result from a worker thread reaches QObjects only through
the Qt thread queue. Nothing blocking runs on the GUI thread.

## 6. Errors and diagnostics

- `termigator-core`: per-module error types (`thiserror`), functions return `Result`. User-facing
  messages are stable text (tests assert substrings such as `must not start with '-'`,
  `between 1 and 65535`, `Cannot reach`, `already exists`).
- No `unwrap()`/`expect()` outside tests and invariants documented in a comment (workspace lints
  deny `unwrap_used`, `todo`, `unimplemented`, `dbg_macro`, `unsafe_op_in_unsafe_fn`).
- Diagnostics: `tracing` to stderr, filter from `TERMIGATOR_LOG` (default `warn`). Secrets are never
  logged: `SSHPASS`, `TERMIGATOR_RDP_PW`, passwords, `enc:` tokens and the master password go through
  a `Secret` type whose `Debug`/`Display` print `****`.

## 7. Security guards

1. Argument guard: empty values or values starting with `-` are rejected (user, host, `-J` hops,
   `-L/-R/-D` forwards, tunnel forwards, ssh-copy-id host).
2. Target guard: user/host contain no whitespace or control characters; host contains no `@`
   (ssh, scp, ssh-copy-id). `user@host` from imports and config goes through `ssh_target_guard`.
3. Allowlists: `remote_locale` (`^[A-Za-z][A-Za-z0-9_.@-]{0,63}$`), StrictHostKeyChecking
   (yes/no/ask/accept-new/off), port 1-65535, remote command must parse as `shlex.split`.
4. scp: paths must not start with `-`; download sources are glob-escaped; IPv6 in brackets.
5. RDP/VNC: first character is not `/`, `-` or `+`; RDP sub-values contain no `,`; shared folders are
   absolute; resolution is validated by regex.
6. Secrets never in argv: `sshpass -e` + `SSHPASS`, `printf | /from-stdin` + `TERMIGATOR_RDP_PW`;
   `enc:` tokens are refused at spawn; clipboard copies are masked as `SSHPASS='****'`. On Windows
   the RDP password goes into a short-lived `.rdp` file encrypted with DPAPI, in the private runtime
   directory.
7. Telnet/serial: guards for a leading `-`, whitespace and commas; the device path is absolute.
8. Network tools: host matches `^[A-Za-z0-9_.:%\-]+$` and has no leading `-`.
9. `private_runtime_dir`: mode 0700, owned by the current uid, not a symlink (ControlPath, control
   socket).
10. Sync: `safe_rel` (no empty, `.`, `..`, leading `/`) and `local_target` (inside the root).
11. `rm -rf` rejects `""`, `/`, `.`, `..`.
12. Every remote path goes through `shell::quote`; `sftp_quote` for sftp batch files.
13. With a vault configured, plain-text passwords are stripped before writing to disk.
14. Key generation never overwrites an existing file; the name does not start with `-`.
15. The terminal opens only http/https/ftp/mailto/file links.

Other attack surface: the cxx FFI (byte types and ids only, no raw pointers in the Rust API), the
C++ code (review security-sensitive changes), and `unsafe`, which is forbidden in `termigator-core`.

## 8. Data compatibility

| Item | Compatibility |
|---|---|
| `config.yaml`, `session.yaml` | same keys and defaults as files written by earlier versions; legacy aliases (`pyte_history`, `jumphost`, `jump_host`) and a legacy root `config.yaml` are still read; YAML 1.1 semantics |
| Comments in `config.yaml` | preserved (yaml-edit); `session.yaml` has a fixed header |
| Vault | `enc:v2:` (AAD binds the entry target); `enc:v1:` is still read and migrated; v1 has known-answer vectors in the golden fixtures |
| `last_session` | tree of `split`/`sizes`/`children`, `tabs`/`current`/`main`, `detached` |
| Control socket | `$TERMIGATOR_CONTROL_SOCKET` or `control.sock` in the private runtime directory; line protocol `toggle|show [TOKEN]` |
| Asciicast v2, session logs, edit cache | standard formats and stable paths |
| `window.dialogs.<Class>` | legacy key, read but never written; window sizes live in `window.dialog_sizes` |

Config location: `$XDG_CONFIG_HOME/termigator` (default `~/.config/termigator`) on Linux,
`%APPDATA%\termigator` on Windows; `TERMIGATOR_CONFIG_DIR` overrides both. Examples:
`config/*.example.yaml`.

## 9. Platforms

Core never uses `cfg` in domain code. `crates/termigator-core/src/platform/{mod,unix,windows}.rs`
exposes free functions (executable/readable checks, `PATH` lookup, current user and home, private
file/dir creation and permissions, uid/ownership checks, process groups and termination,
OS-string/bytes conversion, IPC endpoint detection, file locking, ...). `unix.rs` implements them
with POSIX calls; `windows.rs` implements them with Windows equivalents or reports "unsupported"
where there is none. The core must compile for Windows (`cargo check -p termigator-core --target
x86_64-pc-windows-msvc`).

| | Linux | Windows 11 |
|---|---|---|
| Qt/KF6 | system packages | KDE Craft |
| PTY | Unix pty; bridge reads the master fd | ConPTY; Rust reader thread feeds the bridge ("feed mode") |
| RDP | FreeRDP | `mstsc.exe` with a temporary `.rdp` file (`rdp_windows.rs`) |
| Global shortcut | KGlobalAccel | not available |
| Packaging | deb, bundled deb, AppImage, Arch package | Inno Setup installer |

Packaging scripts live in `scripts/` and `packaging/`:

- `scripts/make-deb.sh` builds a `.deb` in a container (Ubuntu 26.04); `--bundled` ships Qt, KF6
  and glibc under `/opt/termigator` for older distributions (Mint 22.x, Ubuntu 24.04).
- `scripts/make-appimage.sh` builds an AppImage; `scripts/test-appimage-distros.sh` checks it in
  containers of several distributions.
- `scripts/make-arch.sh` builds with `packaging/arch/PKGBUILD` in an Arch container.
- `scripts/make-windows-installer.ps1` stages the app and runs Inno Setup on
  `packaging/windows/termigator.iss`.
- `scripts/fetch-ghostty.sh` fetches libghostty-vt into a local cache so builds work offline.

## 10. Testing

- `crates/*/tests/`: integration tests with stub tools (ssh, socat, sshpass); `live_*` tests use a
  real OpenSSH with a local `sftp-server`.
- GUI smoke tests (`crates/termigator/tests/*_smoke.rs`, launcher in `tests/common/mod.rs`) run the
  real binary with `TERMIGATOR_SMOKE_TEST`, a private config directory and
  `--smoke-check=<area>/<File>.qml`; the QML file drives the UI through `Probe` (a bridge object).
  Run with `QT_QPA_PLATFORM=offscreen`.
- `compat/golden/*.json` are golden fixtures (reference inputs and expected results).
- `scripts/qmllint.sh` checks QML against a baseline (`scripts/qmllint.baseline`);
  `scripts/secret-audit.sh` runs secret-detection heuristics.

## 11. Architecture decisions

### ADR-1: GUI toolkit is Qt 6 + cxx-qt + QML (Kirigami)

**Status:** Accepted.

**Context.** The target is KDE Plasma 6 / Wayland with a native look (Breeze, icons, menus,
dialogs). Logic should be Rust; C++ only for the terminal bridge and thin wrappers around Qt/KDE
APIs that cxx-qt does not expose (message boxes use `QMessageBox` because plasma-integration 6.6
provides no native `MessageDialog` for QtQuick.Dialogs).

**Decision.** Qt 6 + cxx-qt 0.10 + QML; Kirigami components and the QQC2 `org.kde.desktop` style;
context menus and tray through `Qt.labs.platform` (QMenu/QSystemTrayIcon under QApplication, SNI
tray); file and message dialogs through `QtQuick.Dialogs`; forms in `Kirigami.FormLayout`;
preferences in `kirigami-addons` ConfigurationView; mnemonics from qqc2-desktop-style.

**Alternatives.** QWidget + QQuickWidget hybrid (more C++, mixes two UI paradigms); GTK4 + VTE and
iced/egui (look foreign on Plasma, different terminal engine).

**Consequences.** Logic is testable without a GUI and there is one UI paradigm. Costs: QML has no
widget-scoped shortcuts (ADR-11), a QML MenuBar does not reach the Plasma global menu, and
`Qt.labs.platform` is "labs" (fallback: a QMenu helper in C++).

### ADR-2: Terminal engine (Superseded)

Superseded by ADR-15. The first design vendored QMLTermWidget (Konsole's emulator) with a patch
series. It was replaced by emulation in Rust on `ghostty-vt`; the vendored code and the patches are
gone.

### ADR-3: PTY and processes in Rust, terminal I/O reaches the engine on one thread

**Status:** Accepted.

**Decision.** Rust (portable-pty) spawns the process and owns the `Child` and the PTY. On Linux the
C++ session reads a duplicate of the master fd with a `QSocketNotifier` on the GUI thread, forwards
each chunk to Rust synchronously (`on_output`) and feeds the engine; this gives natural
back-pressure. On Windows ConPTY has no descriptor to poll, so Rust threads read and write the pty
and feed the session (feed mode, section 4.1). Session end: `try_wait` every second plus EIO/EOF.

**Consequences.** Logs, asciicast, distro detection, OSC 7 and startup commands work on raw bytes
(no charset round trip). Signals are sent only to Termigator's own `Child`, never to a recycled PID.

### ADR-4: External tools instead of protocol libraries

**Status:** Accepted. OpenSSH, sshpass, scp/sftp, mosh, FreeRDP, vncviewer, telnet,
picocom/tio/screen and network tools are run as processes. This keeps ControlMaster, ProxyJump,
`~/.ssh/config`, 2FA prompts in the terminal and predictable command lines (compared byte for byte
against golden fixtures). Rejected: russh/ssh2 (would change the semantics of the user's ssh
configuration).

### ADR-5: YAML 1.1 compatible with PyYAML, comments preserved

**Status:** Accepted.

**Context.** PyYAML `safe_load` applies YAML 1.1 (`yes/no/on/off` are booleans, `0777` is octal,
`1:30` is sexagesimal, dates). serde-based YAML crates apply 1.2. `config.yaml` must keep user
comments; `session.yaml` must not produce noisy diffs.

**Decision.** Event parser (libyaml-safer) + own YAML 1.1 resolver into an ordered `Value`; typed
accessors with loose coercion (`true == 1` in strip defaults); own emitter byte-compatible with
`safe_dump` for `session.yaml`; yaml-edit to update `config.yaml` (recursive over maps, lists and
scalars replaced whole, new keys appended); fallback: replace values in byte ranges. Writes are
always skip-if-unchanged and atomic.

**Alternatives.** serde_yaml_ng with typed structs (YAML 1.2, loses comments); yaml-edit for read and
write (no 1.1 resolver).

**Consequences.** More own code (resolver, emitter), checked by golden fixtures and cross-read tests.

### ADR-6: Concurrency without an async runtime

**Status:** Accepted. Plain `std` threads and channels for the command runner, transfers, tunnels
and probes; results reach QObjects through `CxxQtThread::queue`. No tokio; all blocking operations
stay off the GUI thread.

### ADR-7: Errors and diagnostics

**Status:** Accepted. `thiserror` + `Result`, stable user-facing messages, `tracing` to stderr with
the `TERMIGATOR_LOG` filter (default `warn`), a `Secret` type with masking `Debug`/`Display`.

### ADR-8: Build with cargo only

**Status:** Accepted. `cxx-qt-build` for QObjects and the QML module (QML in qrc); the bridge's
`build.rs` uses `cc` + moc + rcc and registers QML types statically. `cargo build/test/clippy/fmt`
are the standard workflow; Qt 6 and KF6 come from the system (pkg-config) on Linux and from KDE
Craft on Windows.

### ADR-9: KDE integration

**Status:** Accepted. KGlobalAccel provides a "Show/hide Termigator" action with no default
shortcut, configured in System Settings. `--toggle`/`--show` and the socket protocol are handled
before `QApplication` is created. The Wayland activation token is set before `requestActivate()`.
Tray through `Qt.labs.platform` SystemTrayIcon (SNI); attention alert through `QWindow.alert()`.
Rejected: ashpd (GlobalShortcuts portal needs an async runtime) and KDBusService (would change the
multi-instance semantics).
Single instance (`window.single_instance`, default on): before the app state and `QApplication`,
`single_instance::claim` either takes the control socket's `flock` (this process is the one; the
server is handed to `AppController`) or sends `show` to the holder and exits, retrying up to 10 s
while the holder is still starting; a holder that never answers gets a message box, not a second
instance. The kernel drops the lock of a crashed process and its stale socket is replaced. The
setting is read by each new start, so a change needs no restart. KDBusService stays rejected: it
would tie the check to a session bus (none on Windows, in containers or over ssh).

### ADR-10: Data models for QML

**Status:** Accepted. Only `QAbstractListModel` from cxx-qt. The host tree is flattened (roles
depth, kind, index, expanded) and reset on every change; drop zone (above/on/below) is computed in
plain Rust from the y position in the row. The pane tree is a Rust structure serialised to JSON and
QML builds `SplitView` recursively. Connection forms exchange JSON, so unknown keys survive an edit.
The local file pane uses `Qt.labs.folderlistmodel`. Rejected: `QAbstractItemModel` from Rust and
`QFileSystemModel` (needless complexity).

### ADR-11: Keyboard and shortcuts

**Status:** Accepted. One action table in `core::actions` feeds menus, shortcuts, the "Keyboard
shortcuts" dialog and the uniqueness test. Widget-scoped shortcuts are `Keys.onPressed` in the
relevant FocusScope; application shortcuts while the terminal has focus go through an override in
the bridge; zoom and R/Enter/S use `Keys.priority: Keys.BeforeItem`. Key routing is covered by tests
(Ctrl+L, Ctrl+_, terminal-focus shortcuts).

### ADR-12: Remote names are bytes

**Status:** Accepted. `Vec<u8>` through listing, transfers and sync, so names that are not valid
UTF-8 survive; QML receives only a row id and `display_name` (lossy decoding for display only).

### ADR-13: Separate config directory (Superseded)

Superseded. A separate `termigator-rust` config directory and control socket were used during
development; the app uses the normal `termigator` directory and socket (section 8).

### ADR-14: Licences

**Status:** Accepted. All code (Rust, QML, C++ glue) is MIT and so is the binary as a whole; the
terminal engine (ghostty-vt / libghostty-vt) is permissively licensed. Built-in colour schemes in
`color-schemes/` are original palette definitions (MIT). `LICENSES/README.md` tabulates licences and
`cargo-deny` (`deny.toml`) checks dependency licences with no GPL exception.

### ADR-15: Terminal engine on ghostty-vt

**Status:** Accepted. Emulation is done in Rust by Ghostty's VT engine through the `ghostty-vt`
crate (libghostty-vt, version pinned exactly). The bridge provides the session QObject, the QML view
that draws engine snapshots, key/mouse encoding, search, paste handling and colour schemes. This
replaced the vendored QMLTermWidget (ADR-2).

## 12. Deliberate behaviour choices

Behaviour chosen on purpose (also noted in module comments); do not "restore" an older behaviour.

- `sftp_resume_argv_with` rejects `\n`, `\r` and `\0`; the `ssh_config` export replaces newlines in
  values with spaces.
- Backup zips are created with mode 0600 from the start (no umask window).
- Imports skip files larger than 16 MiB; XML is read as a stream without entity expansion.
- Serial with `screen` 5.x: `8N1` becomes `115200,cs8,-ixon,-ixoff` (screen 5 accepts no
  `-parenb,-cstopb` in any form).
- Closing a live tab asks for confirmation (`window.confirm_close`); a detached window is not
  transient, so application shortcuts do not escape to the main window.
- `window.modern_style` is dead: the style is chosen in Preferences; `follow_konsole` and
  `follow_poll_ms` are wired to the UI.
- `user@host` from imports and config passes `ssh_target_guard` (no leading `-`, whitespace, control
  characters or `@` in the host).
- The old repo-checkout config mode (`<repo>/config`) does not exist.
