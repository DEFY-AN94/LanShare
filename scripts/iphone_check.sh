#!/bin/bash
# Serves the 15-minute iPhone check page (docs/iphone-check.md) from a debug
# build of LanShare on this Mac.
#
#   scripts/iphone_check.sh              # the desktop app, debug build: your normal data,
#                                        # certificate and paired phones (quit LanShare first)
#   scripts/iphone_check.sh --headless   # headless runner with throwaway data in
#                                        # .dev-data/iphone-check (its own certificate, so
#                                        # Safari warns: only for working on the page itself)
#
# The page is copied into dist/ only while this script runs, and removed when
# it exits. Debug builds read dist/ from disk on every request; release builds
# (`npm run tauri build`) run `npm run build`, which empties dist/, before
# embedding it, so they never contain the page.
set -euo pipefail
cd "$(dirname "$0")/.."

[ "$(uname)" = Darwin ] || { echo "This check runs on the Mac only." >&2; exit 1; }

MODE=app
case "${1:-}" in
  "") ;;
  --headless) MODE=headless ;;
  *) echo "usage: $0 [--headless]" >&2; exit 2 ;;
esac

if [ "$MODE" = app ]; then
  # Same directory as the installed app (Tauri's app_data_dir for com.lanshare.desktop).
  DATA="$HOME/Library/Application Support/com.lanshare.desktop"
  PORT=$(sed -n 's/.*"port": *\([0-9][0-9]*\).*/\1/p' "$DATA/config.json" 2>/dev/null | head -1)
  PORT=${PORT:-39210}
else
  PORT=39290 # away from 39210, so the installed app can keep running
fi

# The server moves to the next free port and saves it, which would change the
# address your iPhone uses. Refuse instead.
if lsof -nP -iTCP:"$PORT" -sTCP:LISTEN >/dev/null 2>&1; then
  echo "Port $PORT is in use: is LanShare (or an earlier check) still running? Quit it (menu bar icon › 退出 LanShare) and run this again." >&2
  exit 1
fi

[ -d node_modules ] || { echo "node_modules is missing: run npm ci first." >&2; exit 1; }
npm run build

# The Bonjour name the server certificate covers, in lower case as Safari shows it.
HOST=$(scutil --get LocalHostName 2>/dev/null | tr "[:upper:]" "[:lower:]" || true)
IFACE=$(route -n get default 2>/dev/null | awk '/interface:/ { print $2 }')
IP=$(ipconfig getifaddr "${IFACE:-en0}" 2>/dev/null || ipconfig getifaddr en0 2>/dev/null || ipconfig getifaddr en1 2>/dev/null || true)
LOCAL_ORIGIN=${HOST:+https://$HOST.local:$PORT}
IP_ORIGIN=${IP:+https://$IP:$PORT}

PAGE=(dist/iphone-check.html dist/iphone-check.js)
cleanup() { rm -f "${PAGE[@]}"; }
trap cleanup EXIT
sed -e "s#__LOCAL_ORIGIN__#$LOCAL_ORIGIN#" -e "s#__IP_ORIGIN__#$IP_ORIGIN#" \
  scripts/iphone-check/iphone-check.html > dist/iphone-check.html
cp scripts/iphone-check/iphone-check.js dist/iphone-check.js

echo
echo "iPhone check page (open the IP address first):"
[ -n "$IP_ORIGIN" ] && echo "  IP:     $IP_ORIGIN/iphone-check.html"
[ -n "$LOCAL_ORIGIN" ] && echo "  .local: $LOCAL_ORIGIN/iphone-check.html"
[ -z "$IP_ORIGIN$LOCAL_ORIGIN" ] && echo "  (no address found: use the IP address shown in LanShare › Add device)"
if [ -n "$IP_ORIGIN" ]; then
  printf '%s' "$IP_ORIGIN/iphone-check.html" | pbcopy
  echo "  The IP address is on the Mac clipboard (paste it on the iPhone with Universal Clipboard)."
fi
echo "Stop with Ctrl+C (or 退出 LanShare from the menu bar); the page is removed from dist/ on exit."
echo

if [ "$MODE" = app ]; then
  cargo run --manifest-path src-tauri/Cargo.toml
else
  cargo run --manifest-path src-tauri/Cargo.toml --example headless -- \
    --data .dev-data/iphone-check/app --shared .dev-data/iphone-check/shared --port "$PORT"
fi
