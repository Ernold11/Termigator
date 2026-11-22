#!/usr/bin/env bash
# README screenshots on a real KDE Plasma session, with a demo profile only: fake hosts, a fresh HOME
# with demo files and a neutral prompt. Your own configuration, sessions and files are never read;
# your theme and Konsole profile (XDG directories) are kept, so the shots look like your desktop.
#
#   scripts/readme-screenshots.sh [out-dir]     -> assets/screenshots/{hosts,files}.png
#
# Needs a build (target/release or target/debug) and spectacle. The demo lives in /tmp/termigator-demo
# (a fixed path, because it shows up in the screenshots) and is deleted at the end.
set -euo pipefail
REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
OUT="${1:-$REPO/assets/screenshots}"
BIN=$(ls -t "$REPO"/target/{release,debug}/termigator 2>/dev/null | head -1) # the newer build
[[ -n $BIN ]] || { echo "build first: cargo build --release" >&2; exit 1; }
command -v spectacle >/dev/null || { echo "spectacle not found" >&2; exit 1; }

DEMO=/tmp/termigator-demo
rm -rf "$DEMO"; mkdir -p "$DEMO/cfg" "$DEMO/home/projects" "$OUT"
trap 'kill "${APP:-0}" 2>/dev/null || true; rm -rf "$DEMO"' EXIT

cat >"$DEMO/cfg/config.yaml" <<'YAML'
window:
  width: 1400
  height: 860
  list_width: 360
launch:
  shell_executable: /bin/bash
favorites:
  - {label: local shell, command: bash, terminal: embedded}
YAML
cat >"$DEMO/cfg/session.yaml" <<'YAML'
- {label: web-01, host: web-01.example.com, user: deploy, group: Production/Web, environment: production}
- {label: web-02, host: web-02.example.com, user: deploy, group: Production/Web, environment: production}
- {label: db-primary, host: db-1.example.com, user: postgres, group: Production/Database, environment: production}
- {label: staging, host: staging.example.com, user: deploy, group: Staging, environment: staging}
- {label: build server, host: ci.example.net, user: builder, group: Development, environment: development}
- {label: raspberry pi, host: 192.0.2.40, user: pi, group: Home lab}
- {label: router, host: 192.0.2.1, user: admin, protocol: telnet, group: Home lab}
- {label: windows desktop, host: 192.0.2.20, protocol: rdp, group: Home lab}
YAML

H="$DEMO/home"
printf 'PS1='"'"'\\[\\e[1;32m\\]demo@termigator\\[\\e[0m\\]:\\[\\e[1;34m\\]\\w\\[\\e[0m\\]\\$ '"'"'\nHISTFILE=/dev/null\n' >"$H/.bash_profile"

printf 'Deploy checklist\n  1. staging green\n  2. web-01, web-02 one at a time\n  3. db migrations last\n' >"$H/notes.txt"
mkdir -p "$H/projects"/{web-api,infra,dotfiles,scripts} "$H/Downloads" "$H/Documents"
touch "$H/projects/README.md" "$H/projects/backup.sh" "$H/Documents/runbook.pdf" "$H/Downloads/termigator.deb"
chmod +x "$H/projects/backup.sh"
g() { git -C "$H/projects/web-api" -c user.name=Demo -c user.email=demo@example.com "$@" >/dev/null; }
git -C "$H/projects/web-api" init -q -b main
for m in "Initial API skeleton" "Add health endpoint" "Rate limiting for /login" "Cache user profiles" "Release 1.4.0"; do
    g commit -q --allow-empty -m "$m"
done

log="$DEMO/app.log"
env HOME="$H" XDG_CONFIG_HOME="${XDG_CONFIG_HOME:-$HOME/.config}" XDG_DATA_HOME="${XDG_DATA_HOME:-$HOME/.local/share}" \
    XDG_CACHE_HOME="${XDG_CACHE_HOME:-$HOME/.cache}" TERMIGATOR_CONFIG_DIR="$DEMO/cfg" \
    TERMIGATOR_CONTROL_SOCKET="$DEMO/control.sock" TERMIGATOR_SMOKE_TEST="$DEMO/unused.png" QT_FORCE_STDERR_LOGGING=1 \
    "$BIN" --smoke-check=shell/ShowcaseCheck.qml >"$log" 2>&1 </dev/null &
APP=$!

shot() { # waits for "showcase: $1", lets the frame settle, captures the active window with its frame
    for _ in $(seq 120); do grep -q "showcase: $1" "$log" && break; sleep 0.5; done
    grep -q "showcase: $1" "$log" || { echo "scene $1 not reached (log: $OUT/app.log)" >&2; cp "$log" "$OUT/app.log"; exit 1; }
    sleep 2
    spectacle -b -n -a -o "$OUT/$1.png"
    echo "$OUT/$1.png"
}
shot hosts
shot files
