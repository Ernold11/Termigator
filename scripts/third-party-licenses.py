#!/usr/bin/env python3
"""Licence texts of what a Termigator binary package carries besides its own (MIT) code.

    third-party-licenses.py rust OUT [--target TRIPLE]
        Every crates.io crate linked into the `termigator` binary (normal dependencies only, for
        TRIPLE, default: the host): name, version, licence, repository and the crate's own licence
        files, in one text file.
    third-party-licenses.py system OUTDIR (--files FILE... | --packages PKG...)
        The distribution packages that own FILEs (or the named packages): OUTDIR/PACKAGES.txt
        (name, version, licence, homepage) and OUTDIR/<package>/ with the package's licence files.
        Arch (pacman, /usr/share/licenses) and Debian/Ubuntu (dpkg, /usr/share/doc/*/copyright).

    third-party-licenses.py sources OUTDIR (--files FILE... | --packages PKG...)
        Debian/Ubuntu: the source packages (exact versions, `apt-get source --download-only`, needs
        deb-src) of the packages that own FILEs, into OUTDIR, plus OUTDIR/SOURCES.txt. For the trees
        that bundle LGPL / GPL libraries (AppImage, bundled .deb): shipped next to the binaries.
    third-party-licenses.py craft-sources STAGE OUTDIR [--craft DIR] [--share-sources FILE]
        Windows: the KDE Craft packages whose files are in STAGE (matched by path and SHA-256 with
        Craft's install.db), their source archives (`craft --fetch`) and blueprints with patches,
        plus the upstream archives listed in FILE (scripts/windows-share.sh), into OUTDIR, plus
        OUTDIR/SOURCES.txt.

Used by scripts/make-appimage.sh, make-deb.sh, make-arch.sh (PKGBUILD), windows-share.sh and
make-windows-installer.ps1.
Fails when a crate has no licence file or a file has no owning package, so nothing is shipped
without its licence.
"""

import argparse
import hashlib
import json
import os
import re
import shutil
import subprocess
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
LICENSE_FILE = re.compile(r"^(licen[cs]e|copying|notice|copyright|unlicense|authors)([-_.].*)?$", re.I)
# Crates whose published package has no licence file: crate -> text under LICENSES/third-party/.
OVERRIDES = {
    "cxx-qt": "cxx-qt-MIT.txt",
    "cxx-qt-lib": "cxx-qt-MIT.txt",
    "cxx-qt-lib-extras": "cxx-qt-MIT.txt",
    "yaml-edit": "Apache-2.0.txt",  # Jelmer Vernooĳ, https://github.com/jelmer/yaml-edit
}


def rust(out: Path, target: str | None) -> None:
    cmd = ["cargo", "metadata", "--format-version", "1", "--locked"]
    if target:
        cmd += ["--filter-platform", target]
    else:
        host = subprocess.run(["rustc", "-vV"], capture_output=True, text=True, check=True).stdout
        cmd += ["--filter-platform", re.search(r"^host: (\S+)", host, re.M).group(1)]
    meta = json.loads(subprocess.run(cmd, cwd=REPO, capture_output=True, text=True, check=True).stdout)
    pkgs = {p["id"]: p for p in meta["packages"]}
    nodes = {n["id"]: n for n in meta["resolve"]["nodes"]}
    root = next(i for i, p in pkgs.items() if p["name"] == "termigator" and p["source"] is None)
    seen, stack = set(), [root]
    while stack:
        i = stack.pop()
        if i in seen:
            continue
        seen.add(i)
        # Proc macros run in the compiler and are not linked, nor is anything they depend on.
        if any("proc-macro" in t["kind"] for t in pkgs[i]["targets"]):
            continue
        # kind None = normal dependency; build and dev dependencies are not in the binary.
        stack += [d["pkg"] for d in nodes[i]["deps"] if any(k["kind"] is None for k in d["dep_kinds"])]
    linked = [pkgs[i] for i in seen if not any("proc-macro" in t["kind"] for t in pkgs[i]["targets"])]
    crates = sorted((p for p in linked if p["source"]), key=lambda p: (p["name"], p["version"]))
    missing, parts = [], []
    for p in crates:
        d = Path(p["manifest_path"]).parent
        files = sorted(f for f in d.iterdir() if f.is_file() and LICENSE_FILE.match(f.name))
        if p.get("license_file"):
            files.append(d / p["license_file"])
        if p["name"] in OVERRIDES:
            files.append(REPO / "LICENSES" / "third-party" / OVERRIDES[p["name"]])
        files = list(dict.fromkeys(files))
        if not files:
            missing.append(f'{p["name"]} {p["version"]} ({p.get("license")})')
        head = f'{p["name"]} {p["version"]}\nLicense: {p.get("license") or "see below"}\n'
        if p.get("repository"):
            head += f'Source: {p["repository"]}\n'
        body = "".join(f"\n--- {f.name} ---\n{f.read_text(errors='replace').strip()}\n" for f in files)
        parts.append("=" * 78 + "\n" + head + body)
    if missing:
        sys.exit("crates without a licence file (add them to OVERRIDES):\n  " + "\n  ".join(missing))
    parts += ghostty()
    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_text(
        "Third-party Rust crates linked into Termigator "
        f"({target or 'host'}), {len(crates)} crates.\n"
        "Termigator itself is MIT licensed (LICENSE).\n\n" + "\n".join(parts)
    )
    print(f"{out}: {len(crates)} crates")


def ghostty() -> list[str]:
    """Licence files of libghostty-vt (statically linked by ghostty-vt-sys) and its vendored code,
    from the pinned release that scripts/fetch-ghostty.sh puts in GHOSTTY_RELEASE_DIR."""
    import tarfile

    d = Path(os.environ.get("GHOSTTY_RELEASE_DIR") or Path.home() / ".cache/ghostty-vt-release")
    tars = sorted(d.glob("libghostty-vt-*.tar.gz"))
    if not tars:
        sys.exit(f"no libghostty-vt release in {d} (run scripts/fetch-ghostty.sh)")
    parts = []
    for t in tars:
        with tarfile.open(t) as tf:
            for m in tf.getmembers():
                # afl++ is the fuzzer of Ghostty's test setup, not part of the library.
                if m.isfile() and LICENSE_FILE.match(Path(m.name).name) and "/afl++/" not in m.name:
                    text = tf.extractfile(m).read().decode(errors="replace").strip()
                    parts.append("=" * 78 + f"\nlibghostty-vt release: {m.name}\n\n{text}\n")
    return parts


# Pre-SPDX licence names some Arch packages still declare, with the SPDX text Arch ships.
LEGACY_ARCH = {
    "GPL": "GPL-2.0-or-later",
    "GPL2": "GPL-2.0-only",
    "GPL3": "GPL-3.0-only",
    "LGPL": "LGPL-2.1-or-later",
    "LGPL2.1": "LGPL-2.1-only",
    "LGPL3": "LGPL-3.0-only",
    "APACHE": "Apache-2.0",
    "MPL2": "MPL-2.0",
}


def run(*cmd: str) -> str:
    return subprocess.run(cmd, capture_output=True, text=True, check=True).stdout


def owner(path: str, pacman: bool) -> str:
    real = os.path.realpath(path)
    for p in dict.fromkeys([path, real]):
        try:
            if pacman:
                return run("pacman", "-Qqo", p).split()[0]
            # dpkg knows the path as installed; /lib → /usr/lib merges hide some of them.
            for q in dict.fromkeys([p, re.sub(r"^/usr(/lib)", r"\1", p)]):
                r = subprocess.run(["dpkg-query", "-S", q], capture_output=True, text=True)
                if r.returncode == 0:
                    return r.stdout.split(":")[0].split(",")[0].strip()
        except subprocess.CalledProcessError:
            continue
    raise LookupError(path)


def system(outdir: Path, files: list[str], packages: list[str]) -> None:
    pacman = shutil.which("pacman") is not None
    if not pacman and not shutil.which("dpkg-query"):
        sys.exit("neither pacman nor dpkg-query found")
    names, unowned = set(packages), []
    for f in files:
        try:
            names.add(owner(f, pacman))
        except LookupError:
            unowned.append(f)
    if unowned:
        sys.exit("files without an owning package:\n  " + "\n  ".join(unowned))
    outdir.mkdir(parents=True, exist_ok=True)
    rows = []
    for name in sorted(names):
        dest = outdir / name
        dest.mkdir(exist_ok=True)
        if pacman:
            info = dict(re.findall(r"^(\w[\w ]*?)\s*: (.*)$", run("pacman", "-Qi", name), re.M))
            version, homepage, lic = info["Version"], info.get("URL", ""), info.get("Licenses", "")
            # A provided name (zlib → zlib-ng-compat) keeps its texts under the real package.
            own = Path("/usr/share/licenses") / info["Name"]
            texts = sorted(own.iterdir()) if own.is_dir() else []
            # Packages under a common licence ship no files of their own: take the SPDX texts.
            if not texts:
                ids = [LEGACY_ARCH.get(i, i) for i in lic.split()]
                texts = [t for t in (Path("/usr/share/licenses/spdx") / f"{i}.txt" for i in ids) if t.is_file()]
        else:
            version, homepage = run("dpkg-query", "-W", "-f", "${Version}\t${Homepage}", name).split("\t")
            texts = [Path("/usr/share/doc") / name / "copyright"]
            lic = "see copyright"
        texts = [t for t in texts if t.is_file()]
        if not texts:
            sys.exit(f"{name}: no licence text found")
        for t in texts:
            shutil.copy(t, dest / t.name)
        rows.append(f"{name}\t{version}\t{lic}\t{homepage}")
    (outdir / "PACKAGES.txt").write_text(
        "Distribution packages whose files are bundled with Termigator\n"
        "(name, version, licence, homepage; licence texts in the folder of the same name).\n"
        "LGPL libraries are linked dynamically and can be replaced; their sources, in the versions\n"
        "shipped, are in the sources archive next to this package (see SOURCES.txt here).\n\n"
        + "\n".join(rows) + "\n"
    )
    print(f"{outdir}: {len(rows)} packages")


SOURCES_NOTE = (
    "Sources of the bundled libraries, in the exact versions shipped (LGPL / GPL: you may rebuild\n"
    "and replace them), are in this archive, next to this package on the same download page\n"
    "(the project's GitHub Releases):\n\n    {archive}\n\n"
    "Listed below: bundled package, its version, the source archive(s).\n\n"
)


# Source packages whose licence asks for no source: font files under SIL OFL 1.1 / MIT (fonts-noto
# alone is ~860 MB of sources for the two NotoSans files the trees carry).
DEB_NO_SOURCE = {
    "fonts-noto": "SIL OFL 1.1, no source requirement",
    "fonts-hack": "MIT / Bitstream Vera licence, no source requirement",
}


def sources(outdir: Path, files: list[str], packages: list[str]) -> None:
    if not shutil.which("dpkg-query"):
        sys.exit("sources: only Debian/Ubuntu (dpkg + apt deb-src); build the release trees with "
                 "scripts/make-deb.sh --appimage")
    names, unowned = set(packages), []
    for f in files:
        try:
            names.add(owner(f, False))
        except LookupError:
            unowned.append(f)
    if unowned:
        sys.exit("files without an owning package:\n  " + "\n  ".join(unowned))
    outdir.mkdir(parents=True, exist_ok=True)
    rows, srcs = [], {}
    for name in sorted(names):
        version, src, srcver = run(
            "dpkg-query", "-W", "-f", "${Version}\t${source:Package}\t${source:Version}", name
        ).split("\t")
        if src in DEB_NO_SOURCE:
            rows.append((name, version, f"({DEB_NO_SOURCE[src]})"))
            continue
        srcs[src] = srcver
        rows.append((name, version, f"{src}_{srcver}"))
    # Downloads go to SOURCES_CACHE (kept between builds; apt skips files it has), then exactly the
    # files each .dsc names are copied to OUTDIR.
    cache = Path(os.environ.get("SOURCES_CACHE") or outdir)
    cache.mkdir(parents=True, exist_ok=True)
    for src, ver in sorted(srcs.items()):
        # --only-source: the name is the source package's even where a binary has the same name.
        r = subprocess.run(["apt-get", "source", "--download-only", "--only-source", f"{src}={ver}"],
                           cwd=cache, capture_output=True, text=True)
        if r.returncode != 0:
            sys.exit(f"apt-get source {src}={ver} failed (deb-src enabled?):\n{r.stderr.strip()}")
        dsc = cache / f"{src}_{ver.split(':', 1)[-1]}.dsc"
        text = dsc.read_text(errors="replace")
        part = text.split("Checksums-Sha256:", 1)[1].split("\n\n", 1)[0]
        wanted = [dsc.name] + [ln.split()[2] for ln in part.splitlines()[1:]
                               if ln.startswith(" ") and len(ln.split()) == 3]
        if cache != outdir:
            for w in wanted:
                shutil.copy2(cache / w, outdir / w)
    archive = os.environ.get("SOURCES_ARCHIVE", "termigator-<version>-sources.tar")
    (outdir / "SOURCES.txt").write_text(
        SOURCES_NOTE.format(archive=archive) + "".join(f"{n}\t{v}\t{a}\n" for n, v, a in rows)
    )
    print(f"{outdir}: {len(srcs)} source packages for {len(rows)} packages")


def sha256(p: Path) -> str:
    h = hashlib.sha256()
    with open(p, "rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


# Craft packages whose files may be staged without needing sources from us.
CRAFT_NO_SOURCE = {
    "libs/runtime": "Microsoft Visual C++ runtime, redistributable binaries",
}


def craft_sources(stage: Path, outdir: Path, craft: Path, share_sources: Path | None) -> None:
    import sqlite3
    import urllib.request

    db = sqlite3.connect(craft / "etc" / "blueprints" / "install.db")
    known: dict[str, tuple[str, str]] = {}  # craft-relative path (lower, "/") -> package, version
    by_hash: dict[str, set[tuple[str, str, str]]] = {}  # hash -> package, version, file name
    for pkg, ver, fn, h in db.execute(
        "select p.packagePath, p.version, f.filename, f.fileHash "
        "from fileList f join packageList p using(packageId)"
    ):
        rel = fn.replace("\\", "/").lower()
        h = (h or "").removeprefix("[SHA256]").lower()
        known[rel + "|" + h] = (pkg, ver)
        by_hash.setdefault(h, set()).add((pkg, ver, rel.rsplit("/", 1)[-1]))
    revs = dict(db.execute("select packagePath, revision from packageList"))
    revision = (f"craft-blueprints-kde {revs.get('craft/craft-blueprints-kde', '?')}, "
                f"craft-core {revs.get('craft/craft-core', '?')}")
    used: dict[tuple[str, str], int] = {}
    unmatched = []
    for f in sorted(p for p in stage.rglob("*") if p.is_file()):
        rel = f.relative_to(stage).as_posix().lower()
        h = sha256(f)
        # Staged as bin/<craft bin file>, bin/qml/... (craft qml/...), bin/<plugin dir>/... (plugins/...).
        inner = rel.removeprefix("bin/")
        hit = next((known[c + "|" + h] for c in (rel, inner, "plugins/" + inner) if c + "|" + h in known),
                   None)
        if hit is None:  # same content elsewhere in one package (e.g. windeployqt's own layout)
            cands = {(p, v) for p, v, base in by_hash.get(h, ()) if base == rel.rsplit("/", 1)[-1]}
            hit = next(iter(cands)) if len(cands) == 1 else None
        if hit is None:
            unmatched.append(rel)
        else:
            used[hit] = used.get(hit, 0) + 1
    # Ours, Microsoft's runtime (copied from the MSVC tools) and share/ (from Linux, below).
    stray = [u for u in unmatched if u.endswith(".dll") and not re.match(
        r"bin/(msvcp|vcruntime|vccorlib|concrt|vcomp)", u)]
    if stray:
        sys.exit("staged DLLs from no Craft package:\n  " + "\n  ".join(stray))
    pkgs = sorted(p for p in used if p[0] not in CRAFT_NO_SOURCE)
    outdir.mkdir(parents=True, exist_ok=True)
    fetch = [p for p, _ in pkgs]
    r = subprocess.run(
        ["powershell", "-NoProfile", "-ExecutionPolicy", "Bypass", "-Command",
         f". '{craft / 'craft' / 'craftenv.ps1'}' | Out-Null; craft --fetch " + " ".join(fetch)],
        capture_output=True, text=True)
    if r.returncode != 0 or "fetch succeeded" not in r.stdout:
        sys.exit(f"craft --fetch failed:\n{r.stdout[-3000:]}\n{r.stderr[-2000:]}")
    # KDE's blueprint repository, and the core one (zlib, openssl, icu, ...) that comes with Craft.
    blueprints = [craft / "etc" / "blueprints" / "locations" / "craft-blueprints-kde",
                  craft / "craft" / "blueprints"]
    rows = []
    for pkg, ver in pkgs:
        dest = outdir / pkg.replace("/", "_")
        arch = craft / "download" / "archives" / pkg
        archives = sorted(a for a in arch.iterdir() if a.is_file() and not a.name.endswith(".sha256")) \
            if arch.is_dir() else []
        if not archives:
            sys.exit(f"{pkg} {ver}: craft --fetch left no source archive in {arch}")
        for a in archives:
            (dest / "archives").mkdir(parents=True, exist_ok=True)
            shutil.copy2(a, dest / "archives" / a.name)
        # The blueprint: how Craft built it, with the patches it applied.
        # Platform-specific blueprints sit in <category>/_windows/<name> (libs/_windows/iconv).
        head, _, name = pkg.rpartition("/")
        bp = next((d for b in blueprints for d in (b / pkg, b / head / "_windows" / name) if d.is_dir()),
                  None)
        if bp is None:
            sys.exit(f"{pkg}: no blueprint in {' or '.join(map(str, blueprints))}")
        shutil.copytree(bp, dest / "blueprint", ignore=shutil.ignore_patterns("__pycache__"),
                        dirs_exist_ok=True)
        rows.append(f"{pkg}\t{ver}\t" + ", ".join(a.name for a in archives))
    for line in (share_sources.read_text().splitlines() if share_sources else []):
        name, ver, url, want = line.split("\t")
        dest = outdir / "share" / url.rsplit("/", 1)[-1]
        dest.parent.mkdir(parents=True, exist_ok=True)
        if not dest.is_file() or sha256(dest) != want:
            urllib.request.urlretrieve(url, dest)
        if sha256(dest) != want:
            sys.exit(f"{url}: checksum mismatch")
        rows.append(f"{name}\t{ver}\t{dest.name} ({url})")
    archive = os.environ.get("SOURCES_ARCHIVE", "termigator-<version>-windows-sources.tar")
    (outdir / "SOURCES.txt").write_text(
        SOURCES_NOTE.format(archive=archive)
        + f"KDE Craft builds ({revision}); each folder holds the source archive\n"
        "and the blueprint with the patches Craft applied.\n\n"
        + "\n".join(rows) + "\n\nNo sources needed: "
        + "; ".join(f"{k} ({v})" for k, v in CRAFT_NO_SOURCE.items())
        + "; the fonts (SIL OFL / MIT).\n"
    )
    print(f"{outdir}: {len(pkgs)} Craft packages, {len(unmatched)} files of our own / not from Craft")


def main() -> None:
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    sub = ap.add_subparsers(dest="cmd", required=True)
    r = sub.add_parser("rust")
    r.add_argument("out", type=Path)
    r.add_argument("--target")
    s = sub.add_parser("system")
    s.add_argument("outdir", type=Path)
    g = s.add_mutually_exclusive_group(required=True)
    g.add_argument("--files", nargs="+")
    g.add_argument("--packages", nargs="+")
    so = sub.add_parser("sources")
    so.add_argument("outdir", type=Path)
    g = so.add_mutually_exclusive_group(required=True)
    g.add_argument("--files", nargs="+")
    g.add_argument("--packages", nargs="+")
    c = sub.add_parser("craft-sources")
    c.add_argument("stage", type=Path)
    c.add_argument("outdir", type=Path)
    c.add_argument("--craft", type=Path, default=Path("C:/CraftRoot"))
    c.add_argument("--share-sources", type=Path)
    a = ap.parse_args()
    if a.cmd == "rust":
        rust(a.out, a.target)
    elif a.cmd == "system":
        system(a.outdir, a.files or [], a.packages or [])
    elif a.cmd == "sources":
        sources(a.outdir, a.files or [], a.packages or [])
    else:
        craft_sources(a.stage, a.outdir, a.craft, a.share_sources)


if __name__ == "__main__":
    main()
