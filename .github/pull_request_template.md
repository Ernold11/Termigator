<!-- What does this change and why? Link the issue if there is one. -->

## Checks

- [ ] `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`
- [ ] `cargo test --workspace` (with `QT_QPA_PLATFORM=offscreen`) and `scripts/qmllint.sh`
- [ ] New logic has a test; user-visible changes are in `CHANGELOG.md`
- [ ] No secrets, real host names or passwords in code, tests or screenshots
