#!/usr/bin/env bash
# Builds an Arch Linux package from the LAST COMMIT (git archive HEAD) with packaging/arch/PKGBUILD,
# inside an archlinux container (podman or docker), then installs it in a clean container and runs
# the offscreen smoke check.
#
#   scripts/make-arch.sh            -> target/arch/termigator-<version>-1-x86_64.pkg.tar.zst (+ smoke-arch.png)
#   scripts/make-arch.sh --no-test  build only
#
# Rust and Zig come from the Arch repositories. Caches (cargo, target, ghostty, pacman packages) live in
# ~/.cache/termigator/arch.
set -euo pipefail

log() { echo "==> $*" >&2; }

TEST=1
for a in "$@"; do
  case $a in
    --no-test) TEST=0 ;;
    *) echo "usage: $0 [--no-test]" >&2; exit 2 ;;
  esac
done

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
IMAGE=docker.io/library/archlinux:latest
OUT="$REPO/target/arch"
CACHE="${XDG_CACHE_HOME:-$HOME/.cache}/termigator/arch"
CONTAINER=""
for try in podman docker; do
  command -v "$try" >/dev/null 2>&1 && { CONTAINER=$try; break; }
done
[[ -n $CONTAINER ]] || { echo "podman or docker required on PATH" >&2; exit 1; }
# makepkg refuses root: build as the host's uid so the caches and the package stay the user's files.
USERNS=()
[[ $CONTAINER == podman ]] && USERNS=(--userns=keep-id)

VERSION="$(git -C "$REPO" show HEAD:Cargo.toml | sed -n 's/^version = "\(.*\)"/\1/p' | head -1)+git$(git -C "$REPO" rev-parse --short HEAD)"
SRC="$(mktemp -d)"
trap 'rm -rf "$SRC"' EXIT
log "Exporting git HEAD as termigator-$VERSION.tar.gz"
git -C "$REPO" archive --prefix="termigator-$VERSION/" HEAD | gzip >"$SRC/termigator-$VERSION.tar.gz"
# Packaging tooling, not application code: the worktree copy wins.
SHA=$(sha256sum "$SRC/termigator-$VERSION.tar.gz" | cut -d' ' -f1)
sed -e "s/@VERSION@/$VERSION/" -e "s/@SHA256@/$SHA/" "$REPO/packaging/arch/PKGBUILD" >"$SRC/PKGBUILD"
mkdir -p "$OUT" "$CACHE/pacman"
rm -f "$OUT"/termigator-*.pkg.tar.zst

log "Build container ($CONTAINER, $IMAGE), version $VERSION"
$CONTAINER run -i --rm --user root "${USERNS[@]}" -v "$SRC:/src" -v "$OUT:/out" -v "$CACHE:/cache" \
    -v "$CACHE/pacman:/var/cache/pacman/pkg" -e HOST_UID="$(id -u)" "$IMAGE" bash -s <<'INNER'
set -euo pipefail
log() { echo "==> [build] $*" >&2; }
log "pacman -Syu + build dependencies"
pacman -Syu --noconfirm --needed base-devel >/dev/null
. /src/PKGBUILD
pacman -S --noconfirm --needed "${depends[@]}" "${makedepends[@]}" >/dev/null
getent passwd "$HOST_UID" >/dev/null || useradd -M -u "$HOST_UID" builder
U=$(getent passwd "$HOST_UID" | cut -d: -f1)
mkdir -p /tmp/home /cache/src && chown "$HOST_UID" /tmp/home /cache/src
log "makepkg as $U"
cd /src
runuser -u "$U" -- env HOME=/tmp/home CARGO_HOME=/cache/cargo CARGO_TARGET_DIR=/cache/target \
    SRCDEST=/cache/src PKGDEST=/out BUILDDIR=/tmp/build makepkg -f --noconfirm
INNER
PKG="$(ls "$OUT"/termigator-*.pkg.tar.zst)"
ls -lh "$PKG"

if ((!TEST)); then
  log "Skipping install/smoke test (--no-test)"
  exit 0
fi

log "Smoke test: clean archlinux container, pacman -U, LookCheck offscreen"
SHOT=smoke-arch.png
rm -f "$OUT/$SHOT"
$CONTAINER run --rm --user root -v "$OUT:/out" -v "$CACHE/pacman:/var/cache/pacman/pkg" \
    -e PKG="${PKG##*/}" -e QT_QPA_PLATFORM=offscreen -e QT_QUICK_BACKEND=software -e HOME=/root \
    -e XDG_CURRENT_DESKTOP=X-Cinnamon -e TERMIGATOR_CONFIG_DIR=/tmp/c -e TERMIGATOR_CONTROL_SOCKET=/tmp/s.sock \
    -e TERMIGATOR_SMOKE_TEST="/out/$SHOT" "$IMAGE" bash -c '
        set -e
        pacman -Syu --noconfirm >/dev/null
        pacman -U --noconfirm "/out/$PKG" >/dev/null
        timeout 60 termigator --smoke-check=shell/LookCheck.qml </dev/null 2>&1 | grep -v "does not support" | head -8 || true
        test -s "/out/$SHOT"'
log "Smoke test passed"
ls -lh "$OUT/$SHOT"
