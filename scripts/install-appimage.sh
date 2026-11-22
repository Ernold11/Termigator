#!/bin/sh
# Installs an Termigator AppImage for the current user: ~/.local/bin/termigator, menu entry, icons.
# Usage: scripts/install-appimage.sh [path/to/Termigator-x86_64.AppImage]   (uninstall: add --uninstall)
set -eu
REPO=$(cd "$(dirname "$0")/.." && pwd)
BIN=${XDG_BIN_HOME:-$HOME/.local/bin}
DATA=${XDG_DATA_HOME:-$HOME/.local/share}
APPS=$DATA/applications ICONS=$DATA/icons/hicolor

if [ "${1:-}" = "--uninstall" ]; then
    rm -f "$BIN/termigator" "$BIN/Termigator.AppImage" "$APPS/termigator.desktop" "$ICONS"/*/apps/termigator.png
    echo "removed"; exit 0
fi

SRC=${1:-$REPO/target/appimage/Termigator-x86_64.AppImage}
[ -f "$SRC" ] || { echo "no AppImage: $SRC" >&2; exit 1; }

mkdir -p "$BIN" "$APPS"
install -m755 "$SRC" "$BIN/Termigator.AppImage"
ln -sf Termigator.AppImage "$BIN/termigator"

for f in "$REPO"/assets/icons/termigator-*.png; do
    s=${f##*-}; s=${s%.png}
    mkdir -p "$ICONS/${s}x${s}/apps"
    install -m644 "$f" "$ICONS/${s}x${s}/apps/termigator.png"
done

# Absolute Exec (the menu may not have ~/.local/bin in PATH); drop shebang line and TryExec.
sed -e '/^#!/d' -e '/^TryExec=/d' \
    -e "s|^Exec=termigator|Exec=$BIN/Termigator.AppImage|" "$REPO/termigator.desktop" >"$APPS/termigator.desktop"

command -v update-desktop-database >/dev/null && update-desktop-database "$APPS" || true
command -v gtk-update-icon-cache >/dev/null && gtk-update-icon-cache -q "$ICONS" || true
command -v kbuildsycoca6 >/dev/null && kbuildsycoca6 >/dev/null 2>&1 || true
echo "installed: $BIN/Termigator.AppImage, menu entry 'Termigator'"
