#!/usr/bin/env bash
# Builds a .deb from the LAST COMMIT (git archive HEAD), inside an Ubuntu 26.04 container (podman or
# docker — whichever is on PATH, podman first). Then installs it in a clean container and runs the
# offscreen smoke check.
#
#   scripts/make-deb.sh            -> target/deb/termigator_<version>_ubuntu26.04_amd64.deb (+ smoke-ubuntu26.04.png)
#   scripts/make-deb.sh --bundled  -> target/deb/termigator_<version>_mint22.3_amd64.deb (+ smoke-mint22.3.png)
#   scripts/make-deb.sh --appimage -> the --bundled package + target/appimage/Termigator-x86_64.AppImage (same tree)
#   (--bundled / --appimage also write target/deb/termigator-<version>-linux-sources.tar: the sources of every
#   bundled package, to publish next to them; first run downloads ~1 GB into the cache)
#   scripts/make-deb.sh [--bundled | --appimage] --no-test   build only
#
# APT_MIRROR=http://pl.archive.ubuntu.com/ubuntu/ scripts/make-deb.sh   # when archive.ubuntu.com is slow
#
# The distribution label (file name, package description) is the smoke-test image name, e.g.
# ubuntu:26.04 -> ubuntu26.04, linuxmintd/mint22.3-amd64 -> mint22.3; DISTRO=<label> overrides it.
#
# Native (default): links against Ubuntu 26.04's Qt 6 / KF6, Depends on the distribution packages.
# Bundled: for distributions without Qt 6.10 / KF6 (Linux Mint 22.x, Ubuntu 24.04). The package carries
# Qt, KF6, the QML modules and glibc under /opt/termigator (the AppDir of scripts/make-appimage.sh) and
# needs only the host's GPU stack. TEST_IMAGE=<image> overrides the smoke-test container.
#
# Rust comes from rustup inside the build container (the project needs a newer rustc than Ubuntu's);
# it is a build-time tool only, the package needs nothing of it. Caches (rustup, cargo, target) live in
# ~/.cache/termigator/deb so rebuilds are incremental.
set -euo pipefail

log() { echo "==> $*" >&2; }

BUNDLED=0
APPIMAGE=0
TEST=1
for a in "$@"; do
  case $a in
    --bundled) BUNDLED=1 ;;
    --appimage) BUNDLED=1 APPIMAGE=1 ;;
    --no-test) TEST=0 ;;
    *) echo "usage: $0 [--bundled | --appimage] [--no-test]" >&2; exit 2 ;;
  esac
done

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
IMAGE=docker.io/library/ubuntu:26.04
OUT="$REPO/target/deb"
if ((BUNDLED)); then
  TEST_IMAGE="${TEST_IMAGE:-docker.io/linuxmintd/mint22.3-amd64}"
  TARGET_DESC="Linux Mint 22.x / Ubuntu 24.04 (bundled Qt 6, KF6 and glibc under /opt/termigator)"
else
  TEST_IMAGE="${TEST_IMAGE:-$IMAGE}"
  TARGET_DESC="Ubuntu 26.04 (system Qt 6 / KF6)"
fi
if [[ -z ${DISTRO:-} ]]; then
  DISTRO=${TEST_IMAGE##*/}
  DISTRO=${DISTRO%-amd64}
  DISTRO=${DISTRO/:/}
fi
CACHE="${XDG_CACHE_HOME:-$HOME/.cache}/termigator/deb"
CONTAINER=""
for try in podman docker; do
  command -v "$try" >/dev/null 2>&1 && { CONTAINER=$try; break; }
done
[[ -n $CONTAINER ]] || { echo "podman or docker required on PATH" >&2; exit 1; }

log "make-deb: repository $REPO ($( ((BUNDLED)) && echo bundled || echo native))"
log "Container runtime: $CONTAINER (image $IMAGE)"

SRC="$(mktemp -d)"
trap 'rm -rf "$SRC"' EXIT
log "Exporting sources from git HEAD into $SRC"
git -C "$REPO" archive HEAD | tar -x -C "$SRC"
# Packaging tooling, not application code: the worktree copy wins.
cp "$REPO/scripts/make-appimage.sh" "$SRC/scripts/"
cp "$REPO/scripts/fetch-ghostty.sh" "$SRC/scripts/"
cp "$REPO/scripts/third-party-licenses.py" "$SRC/scripts/"
# Package version must match the sources we compile (HEAD archive), not an uncommitted Cargo.toml bump.
VERSION="$(sed -n 's/^version = "\(.*\)"/\1/p' "$SRC/Cargo.toml" | head -1)+git$(git -C "$REPO" rev-parse --short HEAD)"
WT_VER="$(sed -n 's/^version = "\(.*\)"/\1/p' "$REPO/Cargo.toml" | head -1)"
ARCH_VER="${VERSION%%+git*}"
if [[ $WT_VER != "$ARCH_VER" ]]; then
  echo "warning: worktree Cargo.toml is $WT_VER but HEAD (what we build) is $ARCH_VER — commit the bump before make-deb" >&2
fi
DEB="termigator_${VERSION}_${DISTRO}_amd64.deb"
APPOUT="$REPO/target/appimage"
mkdir -p "$OUT" "$CACHE" "$APPOUT" "$CACHE/apt-lists"
rm -f "$OUT"/termigator_*_"${DISTRO}"_amd64.deb
log "Package version: $VERSION for $DISTRO -> $DEB (archive Cargo.toml $ARCH_VER, worktree $WT_VER)"
log "Debian output: $OUT"
log "Shared build cache: $CACHE"

log "Starting build container for termigator $VERSION"
$CONTAINER run -i --rm --user root -v "$SRC:/src" -v "$OUT:/out" -v "$APPOUT:/appout" -v "$CACHE:/cache" \
    -v "$CACHE/apt-lists:/var/lib/apt/lists" \
    -e VERSION="$VERSION" -e BUNDLED="$BUNDLED" -e APPIMAGE="$APPIMAGE" -e DEB="$DEB" -e TARGET_DESC="$TARGET_DESC" -e CARGO_HOME=/cache/cargo -e RUSTUP_HOME=/cache/rustup \
    -e CARGO_TARGET_DIR=/cache/target -e APT_MIRROR="${APT_MIRROR:-}" "$IMAGE" bash -s <<'INNER'
set -euo pipefail
log() { echo "==> [build] $*" >&2; }
# Download $1 to $3 and check its SHA-256 ($2, from the upstream release) before anything uses it.
fetch() {
  curl -fsSL --retry 3 -o "$3.part" "$1"
  echo "$2  $3.part" | sha256sum -c --quiet - || { rm -f "$3.part"; echo "$1: checksum mismatch" >&2; exit 1; }
  mv "$3.part" "$3"
}
# Pinned tools (checksums: rustup-init.sha256, ziglang.org/download/index.json, the GitHub release digest).
RUSTUP_VERSION=1.29.3 RUSTUP_SHA=b1c8cb14e30889e7e2999e6b69e62d9e9b9ffc0f4df8180564d306d9680e6c55
RUST_TOOLCHAIN=1.100.0
ZIG_VERSION=0.17.1 ZIG_SHA=3581863e3cbcfd0fce0efc682ce7e9e93da96668a977791d356a26d2a7e9b17a
APPIMAGETOOL_VERSION=1.9.1 APPIMAGETOOL_SHA=ed4ce84f0d9caff66f50bcca6ff6f35aae54ce8135408b3fa33abfc3cb384eb0
# The AppImage runtime appimagetool would otherwise download from type2-runtime's moving "continuous" release.
APPIMAGE_RUNTIME_VERSION=20251108 APPIMAGE_RUNTIME_SHA=2fca8b443c92510f1483a883f60061ad09b46b978b2631c807cd873a47ec260d
# Files created here belong to the container's root; the host cannot delete them from /src.
trap 'rm -rf /src/target/appimage' EXIT
export DEBIAN_FRONTEND=noninteractive
mkdir -p /etc/apt/apt.conf.d
echo 'Acquire::Retries "5";' >/etc/apt/apt.conf.d/99-termigator-build
if [[ -n ${APT_MIRROR:-} ]]; then
  log "Using APT mirror: $APT_MIRROR"
  sed -i "s|http://archive.ubuntu.com/ubuntu/|${APT_MIRROR%/}/|" \
    /etc/apt/sources.list.d/ubuntu.sources 2>/dev/null || true
fi
# QML modules are loaded at run time: native Depends on them, bundled copies them into the package.
QML=""
for m in qtcore qtqml qtquick qtquick-controls qtquick-layouts qtquick-window qtquick-dialogs qtquick-templates \
    qt-labs-platform qt-labs-folderlistmodel qt5compat-graphicaleffects org-kde-kirigami org-kde-desktop \
    org-kde-breeze org-kde-iconthemes org-kde-config org-kde-coreaddons org-kde-guiaddons org-kde-ksvg \
    org-kde-kquickcontrolsaddons; do QML="$QML qml6-module-$m"; done
EXTRA=""
if [ "$BUNDLED" = 1 ]; then
  EXTRA="$QML kde-style-breeze kde-style-breeze-data kf6-breeze-icon-theme qt6-svg-plugins \
    qt6-image-formats-plugins qt6-wayland fonts-noto-core fonts-hack file"
fi
if [ "$BUNDLED" = 1 ]; then
  # The bundled tree ships the sources of what it bundles (apt-get source, third-party-licenses.py).
  sed -i 's/^Types: deb$/Types: deb deb-src/' /etc/apt/sources.list.d/ubuntu.sources
fi
log "apt-get update (archive indices ~30 MB; first run often 1–3 min, then cached under ~/.cache/termigator/deb)"
apt-get update
log "apt-get update done"
log "Installing Qt 6 / KF6 and toolchain packages (may take several minutes)"
apt-get install -y --no-install-recommends build-essential pkg-config curl ca-certificates git cmake clang dpkg-dev python3 \
    qt6-base-dev qt6-base-private-dev qt6-declarative-dev qt6-declarative-private-dev libgl-dev libxkbcommon-dev \
    libkf6notifications-dev libkf6globalaccel-dev libkf6windowsystem-dev libkf6colorscheme-dev libkf6iconthemes-dev \
    libkf6breezeicons-dev libkf6config-dev libkf6guiaddons-dev libkf6coreaddons-dev libkf6widgetsaddons-dev \
    libqtermwidget6-2-dev $EXTRA
log "Build dependencies installed"
if [ -x "$CARGO_HOME/bin/cargo" ]; then
  log "Reusing Rust toolchain in \$CARGO_HOME"
else
  log "Installing rustup (minimal profile)"
  fetch "https://static.rust-lang.org/rustup/archive/$RUSTUP_VERSION/x86_64-unknown-linux-gnu/rustup-init" \
    "$RUSTUP_SHA" /tmp/rustup-init
  chmod +x /tmp/rustup-init
  /tmp/rustup-init -y --profile minimal --default-toolchain "$RUST_TOOLCHAIN" >/dev/null 2>&1
  log "rustup installed"
fi
. "$CARGO_HOME/env"
# A cached rustup from an older run may sit on another toolchain.
rustup toolchain install "$RUST_TOOLCHAIN" --profile minimal >/dev/null 2>&1
rustup default "$RUST_TOOLCHAIN" >/dev/null
cd /src
# The terminal engine builds from a pinned libghostty-vt release with Zig; fetch both once (the cache
# lives in /cache, so later builds are offline).
if [ "$(/cache/zig/zig version 2>/dev/null)" != "$ZIG_VERSION" ]; then
  log "Installing Zig $ZIG_VERSION (for libghostty-vt)"
  rm -rf /cache/zig && mkdir -p /cache/zig
  fetch "https://ziglang.org/download/$ZIG_VERSION/zig-x86_64-linux-$ZIG_VERSION.tar.xz" "$ZIG_SHA" /tmp/zig.tar.xz
  tar -xJf /tmp/zig.tar.xz -C /cache/zig --strip-components=1
fi
export PATH=/cache/zig:$PATH
export GHOSTTY_RELEASE_DIR=/cache/ghostty-vt-release
scripts/fetch-ghostty.sh
log "cargo build --release -p termigator (target dir $CARGO_TARGET_DIR)"
# The shared target cache has served stale QML type metadata (plugin.qmltypes) next to a newly
# compiled binary. qmlcachegen then AOT-compiles calls with old method indexes and the app
# segfaults on the first timer callback. Drop this package's release artifacts; dependencies
# and the Qt/rust toolchains stay cached.
cargo clean --release -p termigator
cargo build --release --locked -p termigator 2>&1 | tail -3
log "cargo build done"

log "Staging the .deb tree"
P=/tmp/pkg
rm -rf $P; mkdir -p $P/DEBIAN $P/usr/bin $P/usr/share/applications $P/usr/share/doc/termigator
sed -e '/^#!/d' termigator.desktop >$P/usr/share/applications/termigator.desktop
for f in assets/icons/termigator-*.png; do
    s=${f##*-}; s=${s%.png}
    install -Dm644 "$f" "$P/usr/share/icons/hicolor/${s}x${s}/apps/termigator.png"
done
cp LICENSE LICENSES/*.txt $P/usr/share/doc/termigator/
# Debian Policy 12.5: the copyright file (machine-readable format), pointing at the texts beside it.
cat >$P/usr/share/doc/termigator/copyright <<'COPYRIGHT'
Format: https://www.debian.org/doc/packaging-manuals/copyright-format/1.0/
Upstream-Name: Termigator
Source: https://github.com/Ernold11/Termigator
Comment: The Rust crates and libghostty-vt linked into the binary are listed with their licence
 texts in /usr/share/doc/termigator/THIRD-PARTY-RUST.txt; the bundled package lists the libraries
 it carries in /opt/termigator/usr/share/licenses.

Files: *
Copyright: 2026 Termigator contributors
License: MIT
 See /usr/share/doc/termigator/LICENSE.
COPYRIGHT
# The Rust crates and libghostty-vt linked into the binary (the bundled tree adds its packages' licences).
python3 -I scripts/third-party-licenses.py rust $P/usr/share/doc/termigator/THIRD-PARTY-RUST.txt

if [ "$BUNDLED" = 1 ]; then
  # Sources of the bundled packages (LGPL / GPL), shipped next to the package and the AppImage:
  # one archive for the tree both are made from. Downloads are cached in /cache/srcpkgs.
  SOURCES_TAR="termigator-${VERSION%%+git*}-linux-sources.tar"
  export SOURCES_DIR=/tmp/sources SOURCES_CACHE=/cache/srcpkgs SOURCES_ARCHIVE="$SOURCES_TAR"
  rm -rf "$SOURCES_DIR"
  if [ "$APPIMAGE" = 1 ]; then
    if ! echo "$APPIMAGETOOL_SHA  /cache/tools/appimagetool" | sha256sum -c --quiet - >/dev/null 2>&1; then
      log "Downloading appimagetool $APPIMAGETOOL_VERSION"
      mkdir -p /cache/tools
      fetch "https://github.com/AppImage/appimagetool/releases/download/$APPIMAGETOOL_VERSION/appimagetool-x86_64.AppImage" \
        "$APPIMAGETOOL_SHA" /cache/tools/appimagetool
      chmod +x /cache/tools/appimagetool
    fi
    if ! echo "$APPIMAGE_RUNTIME_SHA  /cache/tools/runtime-x86_64" | sha256sum -c --quiet - >/dev/null 2>&1; then
      log "Downloading the AppImage runtime $APPIMAGE_RUNTIME_VERSION"
      fetch "https://github.com/AppImage/type2-runtime/releases/download/$APPIMAGE_RUNTIME_VERSION/runtime-x86_64" \
        "$APPIMAGE_RUNTIME_SHA" /cache/tools/runtime-x86_64
    fi
    log "Assembling the AppDir and packing the AppImage (scripts/make-appimage.sh)"
    PATH=/cache/tools:$PATH SKIP_BUILD=1 BIN=$CARGO_TARGET_DIR/release/termigator \
      APPIMAGE_RUNTIME=/cache/tools/runtime-x86_64 APPIMAGE_RUNTIME_VERSION=$APPIMAGE_RUNTIME_VERSION \
      scripts/make-appimage.sh
    cp target/appimage/Termigator-x86_64.AppImage /appout/
  else
    log "Assembling the bundled AppDir (scripts/make-appimage.sh, APPDIR_ONLY)"
    SKIP_BUILD=1 APPDIR_ONLY=1 BIN=$CARGO_TARGET_DIR/release/termigator scripts/make-appimage.sh
  fi
  log "Sources archive -> /out/$SOURCES_TAR ($(du -sh /tmp/sources | cut -f1))"
  tar -cf "/out/$SOURCES_TAR" -C /tmp sources
  mkdir -p $P/opt
  cp -a target/appimage/Termigator.AppDir $P/opt/termigator
  rm -rf target/appimage $P/opt/termigator/.DirIcon $P/opt/termigator/termigator.png $P/opt/termigator/termigator.desktop
  ln -s ../../opt/termigator/AppRun $P/usr/bin/termigator
  # Only the GPU stack (kept out of the bundle) comes from the host; the names are the same on Ubuntu 24.04.
  DEPENDS="libegl1, libgl1, libgbm1, libdrm2, libfontconfig1, libdbus-1-3, fonts-dejavu-core"
  # konsole on a non-KDE distribution would pull in a second, older Qt/KF6 stack.
  RELATIONS="Recommends: openssh-client
Suggests: konsole"
else
  RELATIONS="Recommends: openssh-client, konsole"
  install -m755 $CARGO_TARGET_DIR/release/termigator $P/usr/bin/termigator
  strip --strip-unneeded $P/usr/bin/termigator
  log "Computing Depends: shared libraries (dpkg-shlibdeps) and QML modules"
  mkdir -p /tmp/dh/debian && printf 'Source: termigator\n\nPackage: termigator\nArchitecture: any\n' >/tmp/dh/debian/control
  SHLIBS=$(cd /tmp/dh && dpkg-shlibdeps -O -e$P/usr/bin/termigator 2>/dev/null | sed 's/^shlibs:Depends=//')
  DEPENDS="$SHLIBS$(printf ', %s' $QML), kf6-breeze-icon-theme, libqt6svg6, qt6-svg-plugins"
fi
cat >$P/DEBIAN/control <<CTL
Package: termigator
Version: $VERSION
Architecture: amd64
Section: net
Priority: optional
Homepage: https://github.com/Ernold11/Termigator
Maintainer: Termigator packagers <noreply@localhost>
Installed-Size: $(du -sk $P | cut -f1)
Depends: $DEPENDS
$RELATIONS
Description: SSH launcher with an embedded terminal
 SSH bookmarks and favourite commands, an embedded terminal, file transfer and tunnels.
 .
 Built for: $TARGET_DESC.
CTL
log "dpkg-deb --build -> /out/$DEB"
dpkg-deb --root-owner-group --build $P "/out/$DEB" >/dev/null
log "dpkg-deb build done"
log "Built package metadata:"
dpkg-deb -I "/out/$DEB" | sed -n '/^ Package:\|^ Version:\|^ Installed-Size:\|^ Depends:\|Built for:/p'
INNER
log "Build container finished"
((BUNDLED)) && ls -lh "$OUT/termigator-${VERSION%%+git*}-linux-sources.tar"
ls -lh "$OUT/$DEB"

if ((!TEST)); then
  log "Skipping install/smoke test (--no-test)"
  log "Done."
  exit 0
fi

log "Smoke test: clean $TEST_IMAGE container, install .deb, run LookCheck offscreen"
SHOT="smoke-$DISTRO.png"
rm -f "$OUT/$SHOT"
mkdir -p "$CACHE/apt-lists-smoke-$DISTRO"
$CONTAINER run --rm --user root -v "$OUT:/out" -v "$CACHE/apt-lists-smoke-$DISTRO:/var/lib/apt/lists" \
    -e DEB="$DEB" -e SHOT="$SHOT" -e APT_MIRROR="${APT_MIRROR:-}" \
    -e QT_QPA_PLATFORM=offscreen -e QT_QUICK_BACKEND=software -e HOME=/root -e XDG_CURRENT_DESKTOP=X-Cinnamon \
    -e TERMIGATOR_CONFIG_DIR=/tmp/c -e TERMIGATOR_CONTROL_SOCKET=/tmp/s.sock -e TERMIGATOR_SMOKE_TEST="/out/$SHOT" \
    "$TEST_IMAGE" bash -c '
        set -e
        log() { echo "==> [smoke] $*" >&2; }
        export DEBIAN_FRONTEND=noninteractive
        mkdir -p /etc/apt/apt.conf.d
        echo "Acquire::Retries \"5\";" >/etc/apt/apt.conf.d/99-termigator-build
        if [[ -n ${APT_MIRROR:-} ]]; then
          sed -i "s|http://archive.ubuntu.com/ubuntu/|${APT_MIRROR%/}/|" \
            /etc/apt/sources.list.d/ubuntu.sources 2>/dev/null || true
        fi
        log "apt-get update (smoke container; may take 1–3 min on first run)"
        apt-get update
        log "apt-get update done"
        log "Installing .deb and runtime Depends (may take several minutes)"
        apt-get install -y --no-install-recommends "/out/$DEB"
        log "Package install done"
        log "termigator --smoke-check=shell/LookCheck.qml (timeout 60s)"
        timeout 60 termigator --smoke-check=shell/LookCheck.qml </dev/null 2>&1 | grep -v "does not support" | head -8 || true
        log "Smoke check finished"
        log "Checking TERMIGATOR_SMOKE_TEST screenshot"
        test -s "/out/$SHOT"'
log "Smoke test passed"
ls -lh "$OUT/$SHOT"
log "Done."
