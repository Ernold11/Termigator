#!/usr/bin/env bash
# QML lint (Qt 6 qmllint) over crates/termigator/qml. A ratchet, not a clean slate: the tree already has
# warnings (unqualified access, missing ComponentBehavior pragmas, ...), so the step fails on any
# error (syntax, ...) and when the warning count goes ABOVE scripts/qmllint.baseline. After fixing
# warnings, lower the baseline: scripts/qmllint.sh --update
# The app's own QML module (Termigator, Termigator.Terminal, ...) comes from the cxx-qt build, so
# build first (any cargo build/clippy/test of the workspace): without it qmllint cannot resolve the
# app's types and the count would be noise that depends on the machine.
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"
Q="${QMLLINT:-}"
for c in /usr/lib/qt6/bin/qmllint /usr/lib64/qt6/bin/qmllint qmllint-qt6 qmllint6; do
    [[ -n "$Q" ]] && break
    if [[ -x "$c" ]] || command -v "$c" >/dev/null 2>&1; then Q="$c"; fi
done
[[ -n "$Q" ]] || { echo "qmllint.sh: no Qt 6 qmllint (set QMLLINT)" >&2; exit 2; }
QML_IMPORTS="${QML_IMPORT_PATH:-/usr/lib/qt6/qml}"
APP_MODULES="${CARGO_TARGET_DIR:-$ROOT/target}/cxxqt/qml_modules"
[[ -f "$APP_MODULES/Termigator/qmldir" ]] || {
    echo "qmllint.sh: $APP_MODULES/Termigator not found — build the workspace first (cargo build -p termigator)" >&2
    exit 2
}
mapfile -t FILES < <(git ls-files 'crates/termigator/qml/*.qml')
OUT="$(mktemp)"; trap 'rm -f "$OUT"' EXIT
"$Q" -I "$QML_IMPORTS" -I "$APP_MODULES" "${FILES[@]}" >"$OUT" 2>&1 || true
ERRORS="$(grep -c -E '^Error:' "$OUT" || true)"
WARNINGS="$(grep -c -E '^Warning:' "$OUT" || true)"
BASE_FILE="$ROOT/scripts/qmllint.baseline"
if [[ "${1:-}" == "--update" ]]; then echo "$WARNINGS" >"$BASE_FILE"; echo "baseline = $WARNINGS"; exit 0; fi
BASE="$(cat "$BASE_FILE")"
echo "qmllint: ${#FILES[@]} files, $ERRORS errors, $WARNINGS warnings (baseline $BASE)"
if [[ "$ERRORS" -gt 0 ]]; then grep -E -A3 '^Error:' "$OUT" | head -40; exit 1; fi
if [[ "$WARNINGS" -gt "$BASE" ]]; then
    echo "qmllint: more warnings than the baseline — fix the new ones (full output below, tail)" >&2
    grep -E '^Warning:' "$OUT" | tail -20 >&2; exit 1
fi
if [[ "$WARNINGS" -lt "$BASE" ]]; then echo "qmllint: fewer warnings than the baseline — run scripts/qmllint.sh --update"; fi
