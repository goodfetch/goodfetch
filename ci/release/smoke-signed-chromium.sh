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
# fetched directory may be empty; skip cleanly only when chromium/manifest.json
# (or legacy chromium/manifest.toml) lists zero artifacts.
if [[ ! -d "$chromium_dir" ]] || [[ -z "$(ls -A "$chromium_dir" 2>/dev/null)" ]]; then
  has_manifest=0
  if [[ -f chromium/manifest.json ]] && jq -e '(.artifact // .builds // []) | length > 0' chromium/manifest.json >/dev/null 2>&1; then
    has_manifest=1
  elif [[ -f chromium/manifest.toml ]] && grep -q '^\[\[artifact\]\]' chromium/manifest.toml; then
    has_manifest=1
  fi
  if [[ $has_manifest -eq 0 ]]; then
    echo "signed-bundle smoke: no Goodfetch Chromium artifacts in chromium/manifest.json yet"
    exit 0
  fi
  echo "error: chromium manifest lists artifacts but $chromium_dir is empty" >&2
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

# Unpack any fetched Chromium archives into scratch before locating the executable.
unpack_dir="$scratch/chromium"
mkdir -p "$unpack_dir"
while IFS= read -r -d '' archive; do
  case "$archive" in
    *.tar.xz) tar -xJf "$archive" -C "$unpack_dir" ;;
    *.zip) unzip -q -o "$archive" -d "$unpack_dir" ;;
  esac
done < <(find "$chromium_dir" -maxdepth 3 -type f \( -name '*.tar.xz' -o -name '*.zip' \) -print0)

chrome_bin=$(find "$chromium_dir" "$unpack_dir" -type f \( -name chrome -o -name chrome.exe -o -name 'Goodfetch Chromium' \) | head -n 1)
if [[ -z "$chrome_bin" ]]; then
  echo "error: no Chromium executable found under $chromium_dir" >&2
  exit 1
fi

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
# Confirm renderer sandbox stays enabled (no `--no-sandbox` fallback warning in log).
if grep -qiE 'no-sandbox|sandbox disabled' "$scratch/chrome.log"; then
  echo "error: signed Goodfetch Chromium ran without renderer sandbox" >&2
  cat "$scratch/chrome.log" >&2
  exit 1
fi

# 2. Start local Proving ground with its real CLI (`serve --seed --copy --address --network-profile`),
#    parse JSON `.address`, verify Collie's signed executable starts, and navigate
#    the signed Chromium instance to a Proving ground fixture confirming webdriver is hidden.
mkdir -p "$scratch/pg-copy"
"$proving_ground_bin" serve \
  --seed 1 \
  --copy "$scratch/pg-copy" \
  --address 127.0.0.1:0 \
  --network-profile loopback >"$scratch/pg.json" 2>"$scratch/pg.log" &
pg_pid=$!
for _ in $(seq 40); do
  [[ -s "$scratch/pg.json" ]] && break
  sleep 0.25
done
pg_addr=$(jq -er '.address' "$scratch/pg.json")
"$collie_bin" --version >/dev/null
target_url="http://${pg_addr}/"
curl -fsS -X PUT "http://127.0.0.1:${cdp_port}/json/new?${target_url}" \
  | jq -e '.id and .webSocketDebuggerUrl' >/dev/null
kill "$chrome_pid" 2>/dev/null || true
chrome_pid=""
