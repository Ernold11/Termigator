#!/usr/bin/env bash
# Scan the tracked tree for accidental secrets. Golden/fixture paths are excluded (see .gitleaks.toml).
# Prefers gitleaks when installed; always runs the built-in IP / key heuristics on non-allowlisted files.
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

FAIL=0

if command -v gitleaks >/dev/null 2>&1; then
    echo "secret-audit: gitleaks (config .gitleaks.toml, tracked files only)"
    # A copy of the tracked files: build output (target/) and ignored local files are not the repo.
    TREE=$(mktemp -d)
    trap 'rm -rf "$TREE"' EXIT
    git ls-files -z --cached | tar --null -T - -cf - 2>/dev/null | tar -xf - -C "$TREE"
    if ! (cd "$TREE" && gitleaks dir . --config "$ROOT/.gitleaks.toml" --no-banner --redact); then
        FAIL=1
    fi
else
    echo "secret-audit: gitleaks not in PATH — skipping (install: https://github.com/gitleaks/gitleaks)" >&2
fi

echo "secret-audit: built-in heuristics (non-allowlisted git files)"
if ! python3 - "$ROOT" <<'PY'
import ipaddress, re, subprocess, sys
from pathlib import Path

root = Path(sys.argv[1])

ALLOWED_PREFIXES = (
    "compat/golden/",
)
ALLOWED_REGEX = [
    re.compile(r"^crates/[^/]+/tests/"),
    re.compile(r"^crates/[^/]+/src/(.+/)?tests\.rs$"),  # unit tests in <module>/tests.rs
    re.compile(r"^config/session\.example\.yaml$"),
    re.compile(r"^config/config\.example\.yaml$"),
    re.compile(r"^crates/termigator/qml/.+Smoke\.qml$"),
]

def allowed(rel: str) -> bool:
    if any(rel.startswith(p) for p in ALLOWED_PREFIXES):
        return True
    return any(r.match(rel) for r in ALLOWED_REGEX)

def is_doc_ip(ip: str) -> bool:
    try:
        a = ipaddress.ip_address(ip)
    except ValueError:
        return True
    if a.is_private or a.is_loopback or a.is_link_local or a.is_multicast or a.is_reserved:
        return True
    s = str(a)
    if s.startswith(("192.0.2.", "198.51.100.", "203.0.113.", "100.64.")):
        return True
    return False

IP = re.compile(r"\b(?:\d{1,3}\.){3}\d{1,3}\b")
PATTERNS = [
    (re.compile(r"BEGIN (?:RSA |OPENSSH |EC )?PRIVATE KEY"), "private key PEM"),
    (re.compile(r"\bsk-[A-Za-z0-9]{20,}\b"), "API key (sk-)"),
    (re.compile(r"\bghp_[A-Za-z0-9]{20,}\b"), "GitHub token"),
    (re.compile(r"\bAKIA[0-9A-Z]{16}\b"), "AWS access key id"),
]

files = subprocess.check_output(["git", "ls-files"], cwd=root, text=True).splitlines()
hits: list[str] = []

for rel in files:
    if allowed(rel):
        continue
    # Skip binary-ish assets
    if rel.endswith((".svg", ".png", ".ico", ".rcc", ".qm")):
        continue
    path = root / rel
    if not path.is_file():
        continue
    try:
        text = path.read_text(errors="ignore")
    except OSError:
        continue
    for i, line in enumerate(text.splitlines(), 1):
        for pat, label in PATTERNS:
            if pat.search(line):
                hits.append(f"{rel}:{i}: {label}")
        for m in IP.finditer(line):
            ip = m.group()
            parts = [int(x) for x in ip.split(".")]
            if any(p > 255 for p in parts):
                continue
            if not is_doc_ip(ip):
                hits.append(f"{rel}:{i}: public IPv4 {ip}")

if hits:
    print("FAIL: suspicious literals outside allowlisted test paths:", file=sys.stderr)
    for h in hits[:50]:
        print(f"  {h}", file=sys.stderr)
    if len(hits) > 50:
        print(f"  ... and {len(hits) - 50} more", file=sys.stderr)
    sys.exit(1)
print(f"ok ({len(files)} tracked files, heuristics clean)")
PY
then
    FAIL=1
fi

exit "$FAIL"