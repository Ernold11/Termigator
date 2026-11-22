# AGENTS.md

Guidance for AI coding assistants (and humans) working on this repository. Rules for contributions are
in `CONTRIBUTING.md`; the architecture and its decisions (ADR-n, cited in code comments) are in
`ARCHITECTURE.md`.

Termigator is an SSH / RDP / VNC / telnet / serial launcher with an embedded terminal for KDE: Rust
(edition 2024, 1.99+), Qt 6.10+ through cxx-qt 0.10 (pinned `=0.10.0`), QML, KF6. Linux and Windows 11.
CI (`.github/workflows/ci.yml`) runs the Linux checks and the core on Windows; run the checks below locally before pushing.

## Commands

```bash
export QT_QPA_PLATFORM=offscreen            # required for the GUI smoke tests
scripts/fetch-ghostty.sh                    # once: libghostty-vt release into ~/.cache/ghostty-vt-release (offline builds)
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo test -p termigator-core --test proto_ssh                 # one test file
cargo test -p termigator-core --test proto_ssh -- test_name    # one test
cargo test -p termigator --test panes_smoke                    # one GUI smoke test
scripts/qmllint.sh                          # ratchet against scripts/qmllint.baseline (after a build); --update after fixes
scripts/secret-audit.sh                     # heuristics + optional gitleaks (.gitleaks.toml)
cargo deny check                            # licences, advisories, sources
cargo check -p termigator-core --target x86_64-pc-windows-msvc  # core must build for Windows
./run-termigator.sh                         # cargo run --release -p termigator; log in /tmp/termigator.log
```

Packages: `scripts/make-deb.sh` (podman/docker; `--bundled` for Mint 22.x / Ubuntu 24.04),
`scripts/make-arch.sh`, `scripts/make-appimage.sh`, `scripts/make-windows-installer.ps1` (on Windows
with KDE Craft), `./install.sh` (last commit, for the current user), `scripts/release-assets.sh`
(packages of one commit + `SHA256SUMS` for a GitHub Release).

## Lints

Workspace lints deny `unwrap_used`, `todo`, `unimplemented`, `dbg_macro` and `unsafe_op_in_unsafe_fn`
(`unwrap`/`expect` are allowed in tests, see `clippy.toml`). `termigator-core` is
`#![forbid(unsafe_code)]`.

## Layout

Three crates, dependencies only downward:

- `crates/termigator-core` — all logic, no Qt, no `unsafe`: config YAML (comments preserved, output
  compatible with PyYAML, skip-if-unchanged writes), vault (scrypt + AES-GCM), argv builders for
  ssh/scp/rdp/vnc/…, import parsers, PTY and processes (`proc/`), file listing / transfer / sync
  (`remote/`), host tree, panes and session-state models, the platform layer (`platform/`: the only
  place with OS-specific code). Tests in `tests/` use stub tools (ssh, socat, sshpass); `live_*` tests
  use real OpenSSH with a local `sftp-server`.
- `crates/termigator-term-bridge` — C++/Qt through cxx: terminal view and session objects on the
  **ghostty-vt** engine, MultiExec, global shortcuts, built-in colour schemes. Built by `build.rs`
  (cc + moc + rcc), no CMake. Calls into Rust only through a narrow `extern "Rust"` (output tap, session end).
- `crates/termigator` — the binary: QML (`qml/<area>/`), thin cxx-qt QObjects (`src/*_qobj.rs`) over
  core controllers, data to QML as properties / signals / JSON (`qml_json.rs` logs bad JSON), and
  `src/*_native.{h,cpp}` wrappers for Qt APIs outside cxx-qt-lib, without business logic.

Terminal I/O: Rust creates the PTY. On Linux the C++ side reads the descriptor; on Windows (ConPTY) a
Rust reader thread feeds the bytes in ("feed mode", `crates/termigator/src/platform.rs`). A tap sends
the output to Rust too (session log, asciicast, OSC 7, distro detection). A session belongs to its tab
controller; QML views are disposable, so split / detach attach a new view to the existing session.

## Tests

- GUI smoke tests (`crates/termigator/tests/*_smoke.rs`, shared launcher `tests/common/mod.rs`) run
  the real binary with `TERMIGATOR_SMOKE_TEST`, a private config directory and
  `--smoke-check=<area>/<File>.qml`; that QML file drives the UI through the `Probe` bridge.
- Golden JSON files in `compat/golden/` are reference test data (`compat/golden/README.md`). Fixtures are synthetic — never real hosts or passwords.
- A bug fix comes with a test that fails without it.

## Configuration

User config: `~/.config/termigator/` (Windows: `%APPDATA%\termigator`; override with
`TERMIGATOR_CONFIG_DIR`) with `config.yaml` and `session.yaml`; templates in `config/*.example.yaml`.
Never commit a real configuration.

## Conventions

- Code, comments, commit messages and documentation in English.
- Secrets never go into argv, logs or error messages; external commands are argument lists, not shell strings.
- Logic and its tests go into core; QObjects and QML stay thin.
- One focused change per commit, with a message that says why.
