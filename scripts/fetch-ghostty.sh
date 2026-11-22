#!/usr/bin/env bash
# Fetch (once) the libghostty-vt release files the terminal engine builds from, so `cargo build`
# needs no network. The build script (`ghostty-vt-sys`) reads them from GHOSTTY_RELEASE_DIR and
# verifies their sha256 itself; the values here are the ones in its GHOSTTY.lock (vt-1.0.1).
#
#   scripts/fetch-ghostty.sh                 # Linux/macOS sources + deps (~10 MB)
#   scripts/fetch-ghostty.sh --windows       # also the Windows GNU prebuilt zip
#   GHOSTTY_RELEASE_DIR=/some/cache scripts/fetch-ghostty.sh
#
# Zig 0.17.x is still needed for the Linux source build (`zig` on PATH or ZIG=<binary>); the
# script checks it and says what is missing instead of failing later inside cargo.
set -euo pipefail

RELEASE=1.0.1
BASE="https://release.files.ghostty.org/libghostty-vt/$RELEASE"
DIR="${GHOSTTY_RELEASE_DIR:-$HOME/.cache/ghostty-vt-release}"
WINDOWS=0
for a in "$@"; do
    case $a in
        --windows) WINDOWS=1 ;;
        *) echo "usage: $0 [--windows]" >&2; exit 2 ;;
    esac
done

log() { echo "==> $*" >&2; }
mkdir -p "$DIR"

# name sha256
files=(
    "libghostty-vt-$RELEASE.tar.gz 333dfce97641ef9b536bb9e08907ad86abaaada50e8f7417b70d9284adef37aa"
    "libghostty-vt-$RELEASE-deps.tar.gz 91a4e10960a52bf5bf4b15aa2a2cddaff8a89d6cd903d3055bda4fd17de3356d"
)
((WINDOWS)) && files+=(
    "ghostty-vt-$RELEASE-x86_64-windows-gnu.zip 8e2eb9e1822e85d786b456c8beb86f49d09c318c91f0c2913f91e7ffa12cda29"
)

for entry in "${files[@]}"; do
    set -- $entry
    name=$1 sha=$2
    if [[ -f $DIR/$name ]] && [[ $(sha256sum "$DIR/$name" | cut -d' ' -f1) == "$sha" ]]; then
        log "$name already in $DIR"
        continue
    fi
    log "downloading $name"
    curl -fsSL --retry 3 -o "$DIR/$name.part" "$BASE/$name"
    got=$(sha256sum "$DIR/$name.part" | cut -d' ' -f1)
    [[ $got == "$sha" ]] || { rm -f "$DIR/$name.part"; echo "$name: sha256 $got != $sha" >&2; exit 1; }
    mv "$DIR/$name.part" "$DIR/$name"
done

log "release files in $DIR"
ZIG_BIN="${ZIG:-$(command -v zig || true)}"
if [[ -z $ZIG_BIN ]]; then
    echo "note: Zig not found — set ZIG=/path/to/zig-0.17 or put it on PATH (the Linux build needs 0.17.x)" >&2
else
    log "zig: $("$ZIG_BIN" version) ($ZIG_BIN)"
fi
echo "export GHOSTTY_RELEASE_DIR=$DIR"
