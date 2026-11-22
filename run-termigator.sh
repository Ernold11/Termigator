#!/usr/bin/env bash
# Launch Termigator from this checkout (development mode, no install): `cargo run --release -p termigator`.
# For an installed, committed build use ./install.sh instead.
set -euo pipefail
cd "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# The terminal engine (ghostty-vt) builds from a pinned release; use the fetched copy when present
# so the build needs no network (`scripts/fetch-ghostty.sh`).
if [[ -z ${GHOSTTY_RELEASE_DIR:-} && -d $HOME/.cache/ghostty-vt-release ]]; then
    export GHOSTTY_RELEASE_DIR=$HOME/.cache/ghostty-vt-release
fi
# Output also goes to a log for bug reports (CONTRIBUTING.md); pipefail keeps the app's exit code.
"${CARGO:-cargo}" run --release -p termigator -- "$@" 2>&1 | tee "${TMPDIR:-/tmp}/termigator.log"
