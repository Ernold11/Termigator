# Contributing to Termigator

Thanks for helping. Bug reports, fixes and small features are welcome; for anything larger, open an
issue first so we can agree on the approach before you spend time on it.

## Reporting a bug

Include your distribution and desktop, how you installed Termigator (package, AppImage, source),
`termigator --version` or the commit, the steps to reproduce, and what you expected. When you
started it from a terminal, the log is useful (`./run-termigator.sh` also writes it to `/tmp/termigator.log`, or `$TMPDIR/termigator.log`).
**Remove host names, user names and passwords** from anything you paste.

Security problems: please do not open a public issue — use GitHub's private vulnerability
reporting on this repository instead.

## Building and checking

Requirements and the build itself are in the README ("Build and develop"). Before sending a pull
request, run the same checks as CI:

```bash
export QT_QPA_PLATFORM=offscreen
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
scripts/qmllint.sh          # after a build; fails when QML warnings go above scripts/qmllint.baseline
cargo test --workspace
cargo deny check            # licences and advisories of dependencies
```

GUI tests (`crates/termigator/tests/*_smoke.rs`) start the real binary offscreen with a private
config directory, so they never touch your own `~/.config/termigator`.

## Code conventions

- Logic goes into `crates/termigator-core` (no Qt, no `unsafe`) with tests next to it; the
  `crates/termigator` QObjects and QML stay thin. `ARCHITECTURE.md` explains the layers and the
  decisions behind them; `AGENTS.md` is a compact overview (commands, layout, tests) for AI assistants.
- No `unwrap()`, `todo!()`, `unimplemented!()` or `dbg!()` outside tests (workspace lints deny them).
- Secrets never go into argv, logs or error messages; external commands are built as argument
  lists, not shell strings.
- Keep commits focused, one change per commit, with a message that says why.

By contributing you agree that your work is licensed under the MIT license (`LICENSE`).
