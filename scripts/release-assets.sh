#!/usr/bin/env bash
# Collects the packages of one commit into target/release/<version>/ with SHA256SUMS, ready to
# attach to a GitHub Release. Build them first, all from the same commit:
#   scripts/make-deb.sh, scripts/make-deb.sh --bundled, scripts/make-arch.sh,
#   scripts/make-deb.sh --appimage (or make-appimage.sh), and the Windows installer copied to
#   target/windows/ (scripts/make-windows-installer.ps1 on Windows), with the sources archives they
#   write (target/deb/termigator-<v>-linux-sources.tar, target/windows/termigator-<v>-windows-sources.tar:
#   the bundled LGPL / GPL libraries' sources must be published next to the binaries).
# HEAD must be the release tag v<version>: the .deb and Arch builds of that tag carry the clean version
# (others are named +git<commit>). The AppImage, the Windows installer and the sources archives carry no
# version source in their names, so the script lists their dates for you to check.
set -euo pipefail
REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO"
die() { printf 'release-assets.sh: %s\n' "$*" >&2; exit 1; }

ver=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
rev=$(git rev-parse --short HEAD)
[[ -z $(git status --porcelain --untracked-files=no) ]] || die "uncommitted changes: commit, rebuild, then collect"
git describe --exact-match --tags --match "v$ver" HEAD >/dev/null 2>&1 || die "HEAD ($rev) is not the tag v$ver: tag it, rebuild, then collect"
out="target/release/$ver"
rm -rf "$out"
mkdir -p "$out"

take() { [[ -f $1 ]] || die "missing $1 (build it from $rev first)"; cp "$1" "$out/$2"; }
take "target/deb/termigator_${ver}_ubuntu26.04_amd64.deb" "termigator_${ver}_ubuntu26.04_amd64.deb"
take "target/deb/termigator_${ver}_mint22.3_amd64.deb" "termigator_${ver}_mint22.3_amd64.deb"
take "target/arch/termigator-${ver}-1-x86_64.pkg.tar.zst" "termigator-${ver}-1-x86_64.pkg.tar.zst"
take target/appimage/Termigator-x86_64.AppImage "Termigator-${ver}-x86_64.AppImage"
take "target/windows/termigator-${ver}-setup.exe" "termigator-${ver}-setup.exe"
take "target/deb/termigator-${ver}-linux-sources.tar" "termigator-${ver}-linux-sources.tar"
take "target/windows/termigator-${ver}-windows-sources.tar" "termigator-${ver}-windows-sources.tar"

echo "Commit $rev ($(git log -1 --format=%ci)). Check these were built after it:"
ls -l --time-style=+%F\ %T target/appimage/Termigator-x86_64.AppImage "target/windows/termigator-${ver}-setup.exe" \
    "target/deb/termigator-${ver}-linux-sources.tar" "target/windows/termigator-${ver}-windows-sources.tar" |
    awk '{print "  " $6, $7, $8}'
(cd "$out" && sha256sum -- * >SHA256SUMS)
cat "$out/SHA256SUMS"
echo "-> $out (tag: v$ver)"
