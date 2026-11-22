#!/usr/bin/env bash
# Collects what the Windows installer puts in <app>\share from this (Linux) machine, the same
# files the AppImage carries (scripts/make-appimage.sh): the Breeze colour schemes and the Noto
# Sans / Hack fonts, plus the dark Breeze icons. KDE Craft on Windows has none of them. Copy the folder to the Windows machine and
# pass it to scripts/make-windows-installer.ps1 -Share.
#
#   scripts/windows-share.sh <out-dir>
set -euo pipefail
out="${1:?usage: $0 <out-dir>}"
first() { for f in "$@"; do [[ -f $f ]] && { echo "$f"; return; }; done; echo "missing: $1" >&2; exit 1; }
mkdir -p "$out/color-schemes" "$out/fonts"
cp /usr/share/color-schemes/Breeze{Light,Dark}.colors "$out/color-schemes/"
for f in noto/NotoSans-Regular noto/NotoSans-Bold hack/Hack-Regular hack/Hack-Bold; do
    cp "$(first "/usr/share/fonts/${f%%/*}/${f#*/}.ttf" "/usr/share/fonts/TTF/${f#*/}.ttf" \
        "/usr/share/fonts/truetype/$f.ttf")" "$out/fonts/"
done
# The dark Breeze icons as one resource file (Craft has only the light set): applyFixedLook
# registers it as :/icons/breeze-dark. Links become copies; ~13 MB.
rcc=$(command -v rcc-qt6 || command -v /usr/lib/qt6/rcc || command -v /usr/lib/qt6/libexec/rcc || command -v rcc) \
    || { echo "rcc (Qt 6) not found" >&2; exit 1; }
src=/usr/share/icons/breeze-dark
[[ -f $src/index.theme ]] || { echo "missing: $src (breeze-icons)" >&2; exit 1; }
mkdir -p "$out/icons"
qrc=$(mktemp --suffix=.qrc); trap 'rm -f "$qrc"' EXIT
{
    echo '<RCC><qresource prefix="/icons/breeze-dark">'
    (cd "$src" && find . \( -type f -o -type l \) ! -name icon-theme.cache -printf '%P\n' | sort) |
        while read -r f; do echo "<file alias=\"$f\">$src/$f</file>"; done
    echo '</qresource></RCC>'
} >"$qrc"
"$rcc" --binary "$qrc" -o "$out/icons/breeze-dark.rcc"

# Licences of what the installer carries: Termigator's, the Rust crates and libghostty-vt for the
# Windows target, and the projects behind the Qt / KF6 / other DLLs from KDE Craft (taken from this
# machine's packages of the same projects; Arch package names). make-windows-installer.ps1 refuses
# a DLL it cannot map to one of these folders, so keep both lists in step.
repo="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
lic="$out/licenses"
rm -rf "$lic"
install -Dm644 -t "$lic/termigator" "$repo/LICENSE"
python3 -I "$repo/scripts/third-party-licenses.py" rust "$lic/termigator/THIRD-PARTY-RUST.txt" --target x86_64-pc-windows-msvc
python3 -I "$repo/scripts/third-party-licenses.py" system "$lic/packages" --packages \
    qt6-base qt6-declarative qt6-svg kirigami qqc2-desktop-style karchive kcolorscheme kconfig kcoreaddons \
    kguiaddons ki18n kiconthemes kwidgetsaddons sonnet breeze breeze-icons noto-fonts ttf-hack \
    icu freetype2 harfbuzz brotli bzip2 gettext libjpeg-turbo openssl xz libpng pcre2 zlib zstd libb2
# GNU libiconv (iconv.dll) has no package here: glibc has its own iconv.
install -Dm644 /usr/share/licenses/spdx/LGPL-2.1-or-later.txt "$lic/packages/libiconv/LGPL-2.1-or-later.txt"
printf 'libiconv\t(KDE Craft build)\tLGPL-2.1-or-later\thttps://www.gnu.org/software/libiconv/\n' >>"$lic/packages/PACKAGES.txt"
cat >>"$lic/packages/PACKAGES.txt" <<'EOF2'

On Windows these libraries are KDE Craft's builds of the same projects (https://community.kde.org/Craft);
the versions above are those of the Linux packages the licence texts were taken from. The versions
shipped, and their sources, are in SOURCES.txt (written by scripts/make-windows-installer.ps1). The
Microsoft Visual C++ runtime DLLs are redistributed under Microsoft's Visual Studio redistribution terms.
EOF2
# share\ holds data from this machine's packages (Breeze colour schemes and dark icons; the fonts
# are OFL / MIT): the upstream release archives of those versions, for the sources archive that
# make-windows-installer.ps1 builds. KDE's archives are the source of Arch's packages.
cache="${XDG_CACHE_HOME:-$HOME/.cache}/termigator/upstream"
mkdir -p "$cache"
: >"$out/upstream-sources.txt"
for pkg in breeze-icons breeze; do
    ver=$(pacman -Q "$pkg" | cut -d' ' -f2); ver=${ver#*:}; ver=${ver%%-*}
    case $pkg in
        breeze-icons) url="https://download.kde.org/stable/frameworks/${ver%.*}/breeze-icons-$ver.tar.xz" ;;
        breeze) url="https://download.kde.org/stable/plasma/$ver/breeze-$ver.tar.xz" ;;
    esac
    f="$cache/${url##*/}"
    if [[ ! -s $f ]]; then
        curl -fsSL --retry 3 -o "$f.part" "$url"
        mv "$f.part" "$f"
    fi
    printf '%s\t%s\t%s\t%s\n' "$pkg" "$ver" "$url" "$(sha256sum "$f" | cut -d' ' -f1)" >>"$out/upstream-sources.txt"
done
echo "share: $out"
