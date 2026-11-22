#!/usr/bin/env bash
# Builds Termigator-x86_64.AppImage from the current tree (release build) on a host with Qt 6 + KF6
# (Arch or Debian/Ubuntu layout). The image carries Qt, KF6, the QML modules, the Breeze style plugin AND glibc, so it
# runs on distributions with older libraries (Linux Mint, Ubuntu LTS). Not bundled: the GPU stack
# (EGL / GL / GBM / DRM / Vulkan; libwayland IS bundled: KF6 needs a newer one than Ubuntu 22.04 has), which must come from the host.
#
#   scripts/make-appimage.sh            -> target/appimage/Termigator-x86_64.AppImage
#   SKIP_BUILD=1 scripts/make-appimage.sh   reuse target/release/termigator (or $BIN)
#   APPDIR_ONLY=1 scripts/make-appimage.sh  stop after the AppDir (scripts/make-deb.sh --bundled packs it)
#
# Needs: appimagetool (and squashfs-tools), python3 (licences), patchelf is not needed. The app starts through the
# bundled dynamic loader (`ld.so --library-path`), so its children (ssh, shells) keep the host's
# libraries; Qt finds its plugins and QML modules through usr/lib/qt.conf.
set -euo pipefail

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
OUT="$REPO/target/appimage"
APPDIR="$OUT/Termigator.AppDir"
LIB="$APPDIR/usr/lib"
QT_PLUGINS=$(qtpaths6 --query QT_INSTALL_PLUGINS 2>/dev/null || echo /usr/lib/qt6/plugins)
QT_QML=$(qtpaths6 --query QT_INSTALL_QML 2>/dev/null || echo /usr/lib/qt6/qml)
die() { printf 'make-appimage.sh: %s\n' "$*" >&2; exit 1; }
say() { printf '\033[1m==>\033[0m %s\n' "$*"; }
# First existing file of the candidates.
first() { for f in "$@"; do [[ -e $f ]] && { echo "$f"; return; }; done; die "none of: $*"; }

[[ -n ${APPDIR_ONLY:-} ]] || command -v appimagetool >/dev/null || die "appimagetool not found"
[[ -d $QT_PLUGINS && -d $QT_QML ]] || die "Qt 6 not found ($QT_PLUGINS, $QT_QML)"

if [[ -z ${SKIP_BUILD:-} ]]; then
    # The terminal engine needs Zig 0.17 and the pinned libghostty-vt release (fetched once into
    # GHOSTTY_RELEASE_DIR, so the build itself needs no network).
    if ! command -v zig >/dev/null; then
        die "Zig 0.17.x is not on PATH (the libghostty-vt build needs it); see scripts/fetch-ghostty.sh"
    fi
    "$REPO/scripts/fetch-ghostty.sh" >/dev/null
    export GHOSTTY_RELEASE_DIR="${GHOSTTY_RELEASE_DIR:-$HOME/.cache/ghostty-vt-release}"
    say "Release build"
    (cd "$REPO" && cargo build --release --locked -p termigator)
fi
BIN="${BIN:-$REPO/target/release/termigator}"
[[ -x $BIN ]] || die "$BIN is missing"

rm -rf "$APPDIR"
mkdir -p "$LIB/qt6/plugins" "$LIB/qt6/qml" "$APPDIR/usr/bin"
# Where every bundled file came from: the owning packages' licences go into the image (below).
SRC_LIST="$OUT/bundled-from.txt"
: >"$SRC_LIST"
install -m 0755 "$BIN" "$APPDIR/usr/bin/termigator"

# Plugins and QML modules the app uses (everything else of Qt stays out).
for p in platforms wayland-decoration-client wayland-graphics-integration-client \
    wayland-shell-integration xcbglintegrations iconengines styles/breeze6.so \
    platforminputcontexts tls networkinformation generic kf6/kirigami platformthemes/libqxdgdesktopportal.so; do
    [[ -e $QT_PLUGINS/$p ]] || { echo "  (no plugin $p)"; continue; }
    mkdir -p "$LIB/qt6/plugins/$(dirname "$p")"
    cp -a "$QT_PLUGINS/$p" "$LIB/qt6/plugins/$(dirname "$p")/"
    find "$QT_PLUGINS/$p" -type f >>"$SRC_LIST"
done
# Only the image formats the app needs (the KDE kimg_* ones pull in tens of MB of codecs).
mkdir -p "$LIB/qt6/plugins/imageformats"
for f in libqsvg libqjpeg libqgif libqico libqwebp; do
    cp -a "$QT_PLUGINS/imageformats/$f.so" "$LIB/qt6/plugins/imageformats/"
    echo "$QT_PLUGINS/imageformats/$f.so" >>"$SRC_LIST"
done
for m in QtQuick QtQml QtCore Qt Qt5Compat/GraphicalEffects org/kde/kirigami org/kde/desktop \
    org/kde/qqc2desktopstyle org/kde/iconthemes org/kde/config org/kde/coreaddons org/kde/guiaddons \
    org/kde/i18n org/kde/kquickcontrolsaddons org/kde/ksvg org/kde/windowsystem org/kde/kirigamiaddons \
    org/kde/breeze org/kde/sonnet; do
    [[ -d $QT_QML/$m ]] || { echo "  (no QML module $m)"; continue; }
    mkdir -p "$LIB/qt6/qml/$(dirname "$m")"
    cp -a "$QT_QML/$m" "$LIB/qt6/qml/$(dirname "$m")/"
    find "$QT_QML/$m" -type f \( -name qmldir -o -name '*.so' \) >>"$SRC_LIST"
done
find "$LIB/qt6" -name '*.qmltypes' -delete
find "$LIB/qt6" -name '*.debug' -delete

cat >"$LIB/qt.conf" <<'EOF'
[Paths]
Prefix = ..
Plugins = lib/qt6/plugins
Imports = lib/qt6/qml
Qml2Imports = lib/qt6/qml
EOF

# Shared libraries of the binary and of everything above, transitively, minus the host's GPU stack.
skip='^(libGL\.|libEGL|libGLX|libGLdispatch|libgbm|libdrm|libvulkan|libnvidia|libcuda)'
say "Collecting libraries"
find "$APPDIR/usr" -type f \( -name '*.so' -o -name '*.so.*' -o -path '*/bin/termigator' \) -print0 |
    { xargs -0 ldd 2>/dev/null || true; } | awk '/=> \//{print $3}' | sort -u |
    while read -r l; do
        b=$(basename "$l")
        [[ $b =~ $skip ]] && continue
        [[ -e $LIB/$b ]] || { cp -L "$l" "$LIB/$b"; echo "$l" >>"$SRC_LIST"; }
    done
# glibc: the loader and the name-service modules the resolver dlopens.
for f in ld-linux-x86-64.so.2 libnss_files.so.2 libnss_dns.so.2 libresolv.so.2 libdl.so.2 libpthread.so.0 librt.so.1 libutil.so.1; do
    for d in /usr/lib /usr/lib/x86_64-linux-gnu; do
        [[ -e $d/$f && ! -e $LIB/$f ]] && cp -L "$d/$f" "$LIB/$f" && echo "$d/$f" >>"$SRC_LIST"
    done
done
[[ -e $LIB/ld-linux-x86-64.so.2 ]] || die "dynamic loader not found"
# Second round: the libraries copied above bring their own dependencies.
find "$LIB" -maxdepth 1 -type f -print0 | { xargs -0 ldd 2>/dev/null || true; } | awk '/=> \//{print $3}' | sort -u |
    while read -r l; do
        b=$(basename "$l")
        [[ $b =~ $skip ]] && continue
        [[ -e $LIB/$b ]] || { cp -L "$l" "$LIB/$b"; echo "$l" >>"$SRC_LIST"; }
    done
# Nothing of the host's GPU stack may be in the image.
if ls "$LIB" | grep -E "$skip" | grep -q .; then die "GPU/Wayland libraries got into the image: $(ls "$LIB" | grep -E "$skip" | tr '\n' ' ')"; fi
strip --strip-unneeded "$LIB"/*.so* "$APPDIR/usr/bin/termigator" 2>/dev/null || true

# The same look everywhere (applyFixedLook): Breeze colour schemes and the fonts. The Breeze
# icons are inside libKF6BreezeIcons.
mkdir -p "$APPDIR/usr/share/color-schemes" "$APPDIR/usr/share/fonts"
cp /usr/share/color-schemes/Breeze{Light,Dark}.colors "$APPDIR/usr/share/color-schemes/"
echo /usr/share/color-schemes/BreezeLight.colors >>"$SRC_LIST"
for f in noto/NotoSans-Regular noto/NotoSans-Bold hack/Hack-Regular hack/Hack-Bold; do
    font=$(first "/usr/share/fonts/${f%%/*}/${f#*/}.ttf" "/usr/share/fonts/TTF/${f#*/}.ttf" \
        "/usr/share/fonts/truetype/$f.ttf")
    cp "$font" "$APPDIR/usr/share/fonts/"
    echo "$font" >>"$SRC_LIST"
done

# Licences: Termigator's, the Rust crates and libghostty-vt linked into the binary, and every
# distribution package a bundled file came from (Qt, KF6, glibc, fonts, ...).
say "Licences"
LIC="$APPDIR/usr/share/licenses"
install -Dm644 -t "$LIC/termigator" "$REPO/LICENSE"
python3 -I "$REPO/scripts/third-party-licenses.py" rust "$LIC/termigator/THIRD-PARTY-RUST.txt"
mapfile -t bundled < <(sort -u "$SRC_LIST")
python3 -I "$REPO/scripts/third-party-licenses.py" system "$LIC/packages" --files "${bundled[@]}"
# A release tree also needs the sources of those packages (LGPL / GPL), in the versions bundled:
# SOURCES_DIR gets them (shipped next to the image), SOURCES.txt in the image says where.
# scripts/make-deb.sh --bundled / --appimage sets it.
if [[ -n ${SOURCES_DIR:-} ]]; then
    say "Sources of the bundled packages -> $SOURCES_DIR"
    python3 -I "$REPO/scripts/third-party-licenses.py" sources "$SOURCES_DIR" --files "${bundled[@]}"
    cp "$SOURCES_DIR/SOURCES.txt" "$LIC/packages/"
fi
cp "$REPO/assets/icons/termigator-256.png" "$APPDIR/termigator.png"
cp "$APPDIR/termigator.png" "$APPDIR/.DirIcon"
sed -e '/^#!/d' -e 's/^TryExec=.*//' "$REPO/termigator.desktop" >"$APPDIR/termigator.desktop"

cat >"$APPDIR/AppRun" <<'EOF'
#!/bin/sh
HERE="$(dirname "$(readlink -f "$0")")"
exec "$HERE/usr/lib/ld-linux-x86-64.so.2" --library-path "$HERE/usr/lib" --argv0 termigator "$HERE/usr/bin/termigator" "$@"
EOF
chmod +x "$APPDIR/AppRun"
[[ -z ${APPDIR_ONLY:-} ]] || { say "AppDir ready: $APPDIR"; exit 0; }

# The runtime at the front of the image links libfuse (LGPL-2.1), squashfuse, zstd, zlib and musl
# statically: its licence and where its source is go into the image. APPIMAGE_RUNTIME pins the file
# (scripts/make-deb.sh); without it appimagetool downloads type2-runtime's moving "continuous" build.
runtime=()
if [[ -n ${APPIMAGE_RUNTIME:-} ]]; then
    runtime=(--runtime-file "$APPIMAGE_RUNTIME")
    rt_src="AppImage runtime (type2-runtime ${APPIMAGE_RUNTIME_VERSION:-?}): https://github.com/AppImage/type2-runtime/releases/tag/${APPIMAGE_RUNTIME_VERSION:-continuous} (source and build scripts of that release)"
else
    rt_src="AppImage runtime (type2-runtime, continuous build): https://github.com/AppImage/type2-runtime"
fi
install -Dm644 "$REPO/LICENSES/third-party/appimage-type2-runtime.txt" "$LIC/appimage-runtime/LICENSE"
echo "$rt_src" >"$LIC/appimage-runtime/SOURCE.txt"
for f in "$LIC/packages/SOURCES.txt" ${SOURCES_DIR:+"$SOURCES_DIR/SOURCES.txt"}; do
    [[ -f $f ]] && printf '\n%s\n' "$rt_src" >>"$f"
done

say "Packing"
rm -f "$OUT/Termigator-x86_64.AppImage"
# --appimage-extract-and-run belongs to appimagetool's own runtime: it works only as the first argument.
ARCH=x86_64 appimagetool --appimage-extract-and-run "${runtime[@]}" "$APPDIR" "$OUT/Termigator-x86_64.AppImage" >/dev/null 2>&1 ||
    ARCH=x86_64 appimagetool "${runtime[@]}" "$APPDIR" "$OUT/Termigator-x86_64.AppImage"
ls -lh "$OUT/Termigator-x86_64.AppImage"
