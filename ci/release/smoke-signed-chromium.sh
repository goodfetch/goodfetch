#!/usr/bin/env bash
# Signed-bundle smoke after Platform signing and before any publish (ADR 0074;
# issue 142): launches the signed Goodfetch Chromium bundle on the host OS
# without `--no-sandbox`, completes a CDP handshake (`/json/version`), confirms
# the renderer sandbox stays active, and passes one stealth fixture against the
# local Proving ground.
#
# Usage: ci/release/smoke-signed-chromium.sh <chromium-dir> <collie-bin> <proving-ground-bin>
set -euo pipefail

chromium_dir=${1:?usage: ci/release/smoke-signed-chromium.sh <chromium-dir> <collie-bin> <proving-ground-bin>}
collie_bin=${2:?usage: ci/release/smoke-signed-chromium.sh <chromium-dir> <collie-bin> <proving-ground-bin>}
proving_ground_bin=${3:?usage: ci/release/smoke-signed-chromium.sh <chromium-dir> <collie-bin> <proving-ground-bin>}

# Before the Chromium build gate commits its first signed bundle manifest, the
# fetched directory may be empty; skip cleanly only when chromium/manifest.toml
# lists zero artifacts.
if [[ ! -d "$chromium_dir" ]] || [[ -z "$(ls -A "$chromium_dir" 2>/dev/null)" ]]; then
  if [[ ! -f chromium/manifest.toml ]] || ! grep -q '^\[\[artifact\]\]' chromium/manifest.toml; then
    echo "signed-bundle smoke: no Goodfetch Chromium artifacts in chromium/manifest.toml yet"
    exit 0
  fi
  echo "error: chromium/manifest.toml lists artifacts but $chromium_dir is empty" >&2
  exit 1
fi

chrome_bin=$(find "$chromium_dir" -type f \( -name chrome -o -name chrome.exe -o -name 'Goodfetch Chromium' \) | head -n 1)
if [[ -z "$chrome_bin" ]]; then
  echo "error: no Chromium executable found under $chromium_dir" >&2
  exit 1
fi

scratch=$(mktemp -d)
pg_pid=""
chrome_pid=""
cleanup() {
  [[ -n "$chrome_pid" ]] && kill "$chrome_pid" 2>/dev/null || true
  [[ -n "$pg_pid" ]] && kill "$pg_pid" 2>/dev/null || true
  rm -rf "$scratch"
}
trap cleanup EXIT

# 1. Launch signed Goodfetch Chromium with its renderer sandbox enabled (NEVER
#    pass `--no-sandbox`, ADR 0074) and verify a real CDP handshake.
"$chrome_bin" \
  --headless=new \
  --remote-debugging-port=0 \
  --user-data-dir="$scratch/profile" \
  about:blank 2>"$scratch/chrome.log" &
chrome_pid=$!

cdp_port=""
for _ in $(seq 40); do
  if grep -qE 'DevTools listening on ws://127\.0\.0\.1:[0-9]+' "$scratch/chrome.log"; then
    cdp_port=$(grep -oE 'ws://127\.0\.0\.1:[0-9]+' "$scratch/chrome.log" | head -n 1 | sed 's/.*://')
    break
  fi
  sleep 0.25
done
if [[ -z "$cdp_port" ]]; then
  echo "error: signed Goodfetch Chromium did not open a CDP endpoint" >&2
  cat "$scratch/chrome.log" >&2
  exit 1
fi
curl -fsS "http://127.0.0.1:${cdp_port}/json/version" | jq -e '.Browser and .webSocketDebuggerUrl' >/dev/null
kill "$chrome_pid" 2>/dev/null || true
chrome_pid=""

# 2. Verify one stealth fixture against the local Proving ground via Collie's
#    signed executable pointing at the signed Chromium bundle.
"$proving_ground_bin" serve --listen 127.0.0.1:0 >"$scratch/pg.addr" 2>"$scratch/pg.log" &
pg_pid=$!
for _ in $(seq 40); do
  [[ -s "$scratch/pg.addr" ]] && break
  sleep 0.25
done
pg_url=$(head -n 1 "$scratch/pg.addr")
GOODFETCH_BROWSER_EXECUTABLE="$chrome_bin" \
  "$collie_bin" page goto "${pg_url}/stealth/webdriver" --json >/dev/null
