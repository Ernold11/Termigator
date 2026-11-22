#!/usr/bin/env bash
# Install (or update) the Rust Termigator from the LAST COMMIT of this checkout — run it by hand
# after committing. The installed app is a release build of a snapshot (`git archive HEAD`), so
# the everyday app never picks up half-finished, uncommitted work from the tree.
#
#   ./install.sh              build HEAD (release) and install / update
#   ./install.sh --uninstall  remove the installed app, icons and menu entry
#
# Layout (override the base with TERMIGATOR_HOME, the data root with XDG_DATA_HOME):
#   ~/.local/share/termigator/app/termigator     release binary (replaced atomically)
#   ~/.local/share/termigator/app/licenses/   Termigator's licence texts + THIRD-PARTY-RUST.txt (needs python3)
#   ~/.local/share/termigator/termigator         launcher used by the menu entry
#   ~/.local/share/applications/termigator.desktop         menu entry + "Show / hide" action (--toggle)
#   ~/.local/share/icons/hicolor/<N>x<N>/apps/termigator.png
#
# Config is NOT copied or touched: the binary finds its own config (ADR-13, paths.rs) exactly as
# it does when started from the build tree. Uninstalling keeps it.
#
# Environment:
#   TERMIGATOR_TARGET_DIR  cargo target dir cache (default ~/.cache/termigator/target) — keeps rebuilds incremental
#   TERMIGATOR_BINARY      install this prebuilt binary instead of building (CI / packaging / tests)
#   CARGO               cargo to use (default: cargo)
#   --force             overwrite an old, incompatible install (it has a venv) in the same base dir
set -euo pipefail

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
DATA="${XDG_DATA_HOME:-$HOME/.local/share}"
BASE="${TERMIGATOR_HOME:-$HOME/.local/share/termigator}"
APP="$BASE/app"
LAUNCHER="$BASE/termigator"
APPS_DIR="$DATA/applications"
DESKTOP="$APPS_DIR/termigator.desktop"
ICON_DIR="$DATA/icons/hicolor"
CARGO="${CARGO:-cargo}"
TARGET="${TERMIGATOR_TARGET_DIR:-${XDG_CACHE_HOME:-$HOME/.cache}/termigator/target}"
SIZES=(16 24 32 48 64 128 256 512)

say() { printf '\033[1m==>\033[0m %s\n' "$*"; }
die() { printf 'install.sh: %s\n' "$*" >&2; exit 1; }

refresh_menu() {
    if command -v update-desktop-database >/dev/null 2>&1; then update-desktop-database -q "$APPS_DIR" || true; fi
    if command -v gtk-update-icon-cache >/dev/null 2>&1; then gtk-update-icon-cache -q -t "$ICON_DIR" 2>/dev/null || true; fi
}

remove_icons() {
    local s
    for s in "${SIZES[@]}"; do rm -f "$ICON_DIR/${s}x${s}/apps/termigator.png"; done
}

MODE=install
FORCE=0
for arg in "$@"; do
    case "$arg" in
        --uninstall) MODE=uninstall ;;
        --force) FORCE=1 ;;
        *) die "unknown option: $arg (use --uninstall, --force or no arguments)" ;;
    esac
done

if [[ "$MODE" == uninstall ]]; then
    say "Removing $BASE, $DESKTOP and the hicolor icons (config is kept)"
    rm -rf "$BASE"
    rm -f "$DESKTOP"
    remove_icons
    refresh_menu
    exit 0
fi

# An old, incompatible install (it has a venv) in the same default place: don't clobber it silently.
if [[ -d "$BASE/venv" && "$FORCE" -eq 0 ]]; then
    die "$BASE holds an old Termigator install (with a venv). Use TERMIGATOR_HOME=<other dir> to keep both, or --force to replace it"
fi

git -C "$REPO" rev-parse --verify -q HEAD >/dev/null || die "$REPO is not a git checkout with commits"
COMMIT="$(git -C "$REPO" rev-parse --short HEAD)"
SUBJECT="$(git -C "$REPO" log -1 --format=%s)"

# Uncommitted code is deliberately left out — say so, so nobody wonders where a fix went.
DIRTY="$(git -C "$REPO" status --porcelain --untracked-files=no | wc -l)"
if [[ "$DIRTY" -gt 0 ]]; then
    say "Note: $DIRTY uncommitted file(s) are NOT part of this install (snapshot of HEAD only)"
fi

WORK="$(mktemp -d "${TMPDIR:-/tmp}/termigator-install.XXXXXX")"
trap 'rm -rf "$WORK"' EXIT

# 1. Snapshot HEAD.
say "Snapshot $COMMIT — $SUBJECT"
SRC="$WORK/src"
mkdir -p "$SRC"
git -C "$REPO" archive --format=tar HEAD | tar -x -C "$SRC"
[[ -f "$SRC/Cargo.toml" && -d "$SRC/crates/termigator" ]] \
    || die "commit $COMMIT has no Rust workspace — this installer is for the Rust app"

# 2. Release build (or a prebuilt binary).
if [[ -n "${TERMIGATOR_BINARY:-}" ]]; then
    [[ -x "$TERMIGATOR_BINARY" ]] || die "TERMIGATOR_BINARY=$TERMIGATOR_BINARY is not an executable file"
    BIN="$TERMIGATOR_BINARY"
    say "Using prebuilt binary $BIN"
else
    command -v "$CARGO" >/dev/null 2>&1 || die "cargo not found (install a Rust toolchain, or set CARGO / TERMIGATOR_BINARY)"
    say "Building release binary (target cache: $TARGET)"
    mkdir -p "$TARGET"
    CARGO_TARGET_DIR="$TARGET" "$CARGO" build --release --locked -p termigator --manifest-path "$SRC/Cargo.toml"
    BIN="$TARGET/release/termigator"
    [[ -x "$BIN" ]] || die "build finished but $BIN is missing"
fi

# 3. Stage the app next to the current install, then swap — a running Termigator keeps its inode,
#    the next start gets the new version.
mkdir -p "$BASE"
NEW="$BASE/app.new.$$"
rm -rf "$NEW"
mkdir -p "$NEW/licenses"
install -m 0755 "$BIN" "$NEW/termigator"
cp "$SRC"/LICENSES/*.txt "$NEW/licenses/"
# Licences of the linked crates and libghostty-vt, as in the packages; best effort (a local install is
# not a redistribution), so a missing python3 or ghostty release only warns.
if ! (cd "$SRC" && python3 -I scripts/third-party-licenses.py rust "$NEW/licenses/THIRD-PARTY-RUST.txt") 2>/dev/null; then
    say "Note: THIRD-PARTY-RUST.txt not generated (needs python3 and scripts/fetch-ghostty.sh)"
fi
printf '%s %s\n%s\n' "$COMMIT" "$(date -Iseconds)" "$SUBJECT" >"$NEW/INSTALLED_FROM"
if [[ -d "$APP" ]]; then
    mv "$APP" "$BASE/app.old.$$"
fi
mv "$NEW" "$APP"
rm -rf "$BASE"/app.old.*

# 4. Launcher.
cat >"$LAUNCHER" <<EOF
#!/usr/bin/env bash
# Generated by install.sh (Termigator $COMMIT). Config location is decided by the binary itself.
exec "$APP/termigator" "\$@"
EOF
chmod +x "$LAUNCHER"

# 5. Icons (hicolor theme, name "termigator" as in the .desktop file).
for s in "${SIZES[@]}"; do
    src_icon="$SRC/assets/icons/termigator-$s.png"
    [[ -f "$src_icon" ]] || continue
    mkdir -p "$ICON_DIR/${s}x${s}/apps"
    install -m 0644 "$src_icon" "$ICON_DIR/${s}x${s}/apps/termigator.png"
done

# 6. Menu entry from the generic termigator.desktop. Only the program is replaced — arguments stay
#    (the toggle action's --toggle); Icon=termigator stays a theme name.
mkdir -p "$APPS_DIR"
sed -e "s|^Exec=[^ ]*|Exec=$LAUNCHER|" \
    -e "s|^TryExec=.*|TryExec=$LAUNCHER|" \
    "$SRC/termigator.desktop" >"$DESKTOP"
refresh_menu

say "Installed Termigator $COMMIT → $LAUNCHER"
say "Menu entry: $DESKTOP  ·  config: untouched (found by the binary, see crates/termigator-core/src/paths.rs)"
