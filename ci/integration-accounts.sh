#!/usr/bin/env bash
# Runs ADRs 0070 and 0076's real-account Integration Live checks behind a
# Cloudflare Quick Tunnel (`cloudflared tunnel --url`, issue 428).
#
#   ci/integration-accounts.sh install   # before ADR 0043's deny
#   ci/integration-accounts.sh run       # under ADR 0043's deny
#
# Uses the same pinned cloudflared binary as proving-ground/deploy/deploy.sh.
# No Cloudflare account, domain or credential is used; each run gets a fresh
# trycloudflare.com URL exported in GOODFETCH_INTEGRATION_TUNNEL_URL.
set -euo pipefail

CLOUDFLARED_VERSION=2026.10.7
CLOUDFLARED_SHA256=1f99fb518b83dec2ba8d870a7cc991d82a77e6196295a3d2754ba17ab9f91872
CLOUDFLARED_BIN="target/cloudflared/cloudflared"
JOB_KEY="live.yml#integration-accounts"

install_cloudflared() {
  mkdir -p "$(dirname "$CLOUDFLARED_BIN")"
  if [[ -x "$CLOUDFLARED_BIN" ]] && echo "$CLOUDFLARED_SHA256  $CLOUDFLARED_BIN" | sha256sum --check --quiet 2>/dev/null; then
    return 0
  fi
  local tmp
  tmp=$(mktemp)
  trap 'rm -f "$tmp"' RETURN
  curl -fsSL -o "$tmp" \
    "https://github.com/cloudflare/cloudflared/releases/download/$CLOUDFLARED_VERSION/cloudflared-linux-amd64"
  echo "$CLOUDFLARED_SHA256  $tmp" | sha256sum --check --quiet
  install -m 755 "$tmp" "$CLOUDFLARED_BIN"
}

has_live_checks() {
  [[ -f ci/live-checks.toml ]] && grep -qF "job = \"$JOB_KEY\"" ci/live-checks.toml
}

run_checks() {
  if ! has_live_checks; then
    exec cargo run --quiet --locked --bin xtask -- \
      results live-run --job "$JOB_KEY" --junit target/junit/live.xml
  fi

  local daemon_url="${GOODFETCH_INTEGRATION_DAEMON_URL:-}"
  if [[ -z "$daemon_url" ]]; then
    echo "integration-accounts.sh: GOODFETCH_INTEGRATION_DAEMON_URL is not set (requires an Exposed Daemon with TLS and a token)" >&2
    exit 1
  fi

  local log tunnel_pid=""
  log=$(mktemp)
  cleanup() {
    if [[ -n "$tunnel_pid" ]]; then
      kill "$tunnel_pid" 2>/dev/null || true
      wait "$tunnel_pid" 2>/dev/null || true
    fi
    rm -f "$log"
  }
  trap cleanup EXIT

  "$CLOUDFLARED_BIN" tunnel --no-autoupdate --protocol http2 --url "$daemon_url" >"$log" 2>&1 &
  tunnel_pid=$!

  local tunnel_url="" deadline=$((SECONDS + 60))
  while (( SECONDS < deadline )); do
    tunnel_url=$(grep -Eo 'https://[a-z0-9-]+\.trycloudflare\.com' "$log" | head -n 1 || true)
    if [[ -n "$tunnel_url" ]]; then
      break
    fi
    if ! kill -0 "$tunnel_pid" 2>/dev/null; then
      cat "$log" >&2
      echo "integration-accounts.sh: cloudflared exited before publishing a Quick Tunnel URL" >&2
      exit 1
    fi
    sleep 1
  done

  if [[ -z "$tunnel_url" ]]; then
    cat "$log" >&2
    echo "integration-accounts.sh: timed out waiting for Quick Tunnel URL" >&2
    exit 1
  fi

  export GOODFETCH_INTEGRATION_TUNNEL_URL="$tunnel_url"
  cargo run --quiet --locked --bin xtask -- \
    results live-run --job "$JOB_KEY" --junit target/junit/live.xml
}

case "${1:-}" in
  install) install_cloudflared ;;
  run) run_checks ;;
  *)
    echo "usage: $0 <install|run>" >&2
    exit 2
    ;;
esac
