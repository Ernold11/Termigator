#!/usr/bin/env bash
# Starts the AppImage in podman containers of other distributions (offscreen) and keeps a screenshot
# of each in target/appimage/distro-shots/<name>.png, so the look can be compared by eye or with
# `compare -metric AE`. A container has the host libraries of a normal desktop (EGL, Wayland client,
# fontconfig, D-Bus) installed first, the rest comes from the image.
#
#   scripts/test-appimage-distros.sh [name ...]     names: ubuntu22 ubuntu24 fedora41 mint22 mint22.3
#
# Needs podman or docker and target/appimage/Termigator-x86_64.AppImage (scripts/make-appimage.sh).
# The images are pulled on the first run.
set -uo pipefail

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
IMG="$REPO/target/appimage/Termigator-x86_64.AppImage"
OUT="$REPO/target/appimage/distro-shots"
[[ -x $IMG ]] || { echo "build the AppImage first: scripts/make-appimage.sh" >&2; exit 1; }
CONTAINER=""
for try in podman docker; do
    command -v "$try" >/dev/null 2>&1 && { CONTAINER=$try; break; }
done
[[ -n $CONTAINER ]] || { echo "podman or docker required on PATH" >&2; exit 1; }
mkdir -p "$OUT"

APT='apt-get update -qq && DEBIAN_FRONTEND=noninteractive apt-get install -y -qq libegl1 libgl1 libwayland-client0 libwayland-cursor0 libfontconfig1 libdbus-1-3 fonts-dejavu-core >/dev/null'
declare -A IMAGE=(
    [ubuntu22]=docker.io/library/ubuntu:22.04
    [ubuntu24]=docker.io/library/ubuntu:24.04
    [fedora41]=docker.io/library/fedora:41
    [mint22]=docker.io/linuxmintd/mint22-amd64
    [mint22.3]=docker.io/linuxmintd/mint22.3-amd64
)
declare -A PREP=(
    [ubuntu22]=$APT
    [ubuntu24]=$APT
    [fedora41]='dnf install -y -q mesa-libEGL mesa-libGL libwayland-client libwayland-cursor fontconfig dbus-libs dejavu-sans-fonts >/dev/null'
    [mint22]=$APT
    [mint22.3]=$APT
)

names=("$@")
[[ ${#names[@]} -gt 0 ]] || names=(ubuntu22 ubuntu24 fedora41 mint22 mint22.3)
rc=0
for n in "${names[@]}"; do
    [[ -n ${IMAGE[$n]:-} ]] || { echo "unknown distro $n" >&2; rc=1; continue; }
    rm -f "$OUT/$n.png"
    printf '== %s (%s): ' "$n" "${IMAGE[$n]}"
    # Own dark kdeglobals, to prove that it is ignored (the leak that once gave white on white).
    if $CONTAINER run --rm --user root \
        -v "$IMG:/app/Termigator.AppImage:ro" -v "$OUT:/out" \
        -e APPIMAGE_EXTRACT_AND_RUN=1 -e QT_QPA_PLATFORM=offscreen -e QT_QUICK_BACKEND=software \
        -e TERMIGATOR_CONFIG_DIR=/tmp/c -e TERMIGATOR_CONTROL_SOCKET=/tmp/s.sock -e TERMIGATOR_SMOKE_TEST="/out/$n.png" \
        -e XDG_CURRENT_DESKTOP=X-Cinnamon -e HOME=/root "${IMAGE[$n]}" bash -c "
            set -e
            (${PREP[$n]}) 2>&1 | tail -2
            mkdir -p /root/.config && printf '[General]\nColorScheme=BreezeDark\n[Icons]\nTheme=Adwaita\n' >/root/.config/kdeglobals
            timeout 40 /app/Termigator.AppImage --smoke-check=shell/LookCheck.qml >/tmp/out 2>/tmp/err </dev/null || true
            grep -v 'TabBar.qml:33\|propagateSize\|does not support' /tmp/err | head -5
            test -s /out/$n.png" >"$OUT/$n.log" 2>&1; then
        echo "ok ($(stat -c %s "$OUT/$n.png") bytes)"
    else
        echo "FAILED, see $OUT/$n.log"
        tail -5 "$OUT/$n.log"
        rc=1
    fi
done
exit $rc
