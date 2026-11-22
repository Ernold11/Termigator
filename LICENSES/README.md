# Licenses

| Part | License | Text |
|---|---|---|
| Termigator's own code (`crates/termigator`, `crates/termigator-core`, `crates/termigator-term-bridge`) | MIT | `MIT.txt` (also `../LICENSE`) |
| `ghostty-vt` / `ghostty-vt-sys` crates (Rust wrapper, `crates/termigator-term-bridge`) | Apache-2.0 | in the crate (`LICENSE`); copied into `THIRD-PARTY-RUST.txt` of every package |
| libghostty-vt 1.0.1 (the terminal engine inside those crates) and the code it vendors (simdutf, highway, …) | MIT © Mitchell Hashimoto, Ghostty contributors; vendored parts MIT / Apache-2.0 / BSD-3-Clause | `LICENSE-GHOSTTY` in the crate and the release's licence files, in `THIRD-PARTY-RUST.txt` |
| Distribution logos (`assets/distros/*.svg`, the badge of a detected Linux distribution) | CC0-1.0 | from [Simple Icons](https://simpleicons.org) (unmodified); see the trademark note below |
| Termigator logo and icons (`assets/termigator-*`, `assets/termigator.ico`, `assets/icons/`) | MIT | made for this project with ChatGPT (OpenAI) image generation; released with the code under `MIT.txt` to the extent they are protected (AI-generated images may not be copyrightable in some jurisdictions) |
| Microsoft Visual C++ runtime (`msvcp*`, `vcruntime*`, `vccorlib*`, `concrt*`, `vcomp*` DLLs; Windows installer only) | Microsoft software licence terms | unmodified files from Visual Studio's `VC\Redist` folder, redistributed as Distributable Code; they remain under Microsoft's terms, not MIT |
| Built-in colour schemes (`crates/termigator-term-bridge/color-schemes`) | MIT | our own definitions, `MIT.txt` |
| `option-ext` (crates.io, via `ghostty-vt` → `dirs`), linked unmodified | MPL-2.0 | source on crates.io; the only non-permissive dependency, allowed by name in `../deny.toml` |
| `third-party/` | MIT, Apache-2.0 | texts for crates whose published package has none (cxx-qt, cxx-qt-lib, cxx-qt-lib-extras, yaml-edit) |
| AppImage runtime ([type2-runtime](https://github.com/AppImage/type2-runtime), pinned release in `scripts/make-deb.sh`; only in the AppImage) | MIT; statically links libfuse (LGPL-2.1), squashfuse (BSD-2-Clause), zstd, zlib, musl | `third-party/appimage-type2-runtime.txt`; copied into the image with a pointer to the release's source (`usr/share/licenses/appimage-runtime/`) and listed in `SOURCES.txt` |

The logos in `assets/distros/` are trademarks of their owners (Canonical, Debian, Red Hat, SUSE, the Raspberry Pi
Foundation and the other projects named there). Termigator shows them only to identify the system a session runs
on; this implies no endorsement or affiliation. Simple Icons releases the SVG files under CC0-1.0, which does not
cover the trademarks.

Termigator's own code is MIT (ARCHITECTURE.md, ADR-14); terminal emulation runs on Ghostty's engine
through the `ghostty-vt` crate. The binary also contains third-party code under other permissive licences
(Apache-2.0, BSD, ISC, Zlib, Unicode-3.0, …) and one MPL-2.0 crate; none of them is copyleft for
Termigator's code, and all of them are listed with their texts in `THIRD-PARTY-RUST.txt`. `install.sh` installs the texts to `~/.local/share/termigator/licenses/`.
Licenses of crates.io dependencies are policed by `deny.toml` (cargo-deny in CI).

## In the binary packages

`scripts/third-party-licenses.py` collects what a package carries besides Termigator's own code:

- `THIRD-PARTY-RUST.txt` — every crates.io crate linked into the binary (name, version, licence, source and
  the crate's licence files) plus libghostty-vt and the code it vendors, from the pinned release. In every
  package: `.deb` (`/usr/share/doc/termigator/`), Arch (`/usr/share/licenses/termigator/`), AppImage and the
  bundled `.deb` (`usr/share/licenses/termigator/`), Windows (`share\licenses\termigator\`).
- `packages/` — for the trees that bundle libraries (AppImage, bundled `.deb`, Windows installer): one folder of
  licence texts per distribution package a bundled file comes from (Qt, KF6, glibc, ICU, fonts, …) and
  `PACKAGES.txt` with name, version, licence and homepage. The native `.deb` and Arch packages use the system's
  libraries, so they need none. On Windows the texts come from the Linux packages of the same projects
  (`scripts/windows-share.sh`); `make-windows-installer.ps1` stops when a DLL has no licence folder.

LGPL libraries (Qt, KF6, glibc, …) are linked dynamically and can be replaced in the bundled trees.

Their sources, in the exact versions shipped, are published next to the binaries of each release (the
same download page), and `SOURCES.txt` beside `PACKAGES.txt` lists them:

- `termigator-<version>-linux-sources.tar` — for the AppImage and the bundled `.deb` (built from the same
  tree): the Ubuntu source packages (`.dsc`, upstream archive, Debian patches) of every bundled package,
  from `scripts/third-party-licenses.py sources` in `scripts/make-deb.sh --bundled` / `--appimage`.
- `termigator-<version>-windows-sources.tar` — for the Windows installer: for each KDE Craft package a
  staged file comes from (matched by path and SHA-256 with Craft's install database), the source archive
  Craft builds from and its blueprint with the patches; plus KDE's release archives of the Breeze colour
  schemes and dark icons in `share\` (`scripts/make-windows-installer.ps1`, `scripts/windows-share.sh`).

The native `.deb` and the Arch package bundle nothing: the distribution provides those sources.
