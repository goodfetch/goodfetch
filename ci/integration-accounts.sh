#!/usr/bin/env bash
# Runs ADRs 0070 and 0076's real-account Integration Live checks behind a
# Cloudflare Quick Tunnel (`cloudflared tunnel --url`, issue 428).
#
#   ci/integration-accounts.sh install   # before ADR 0043's deny
#   ci/integration-accounts.sh run       # under ADR 0043's deny
#
# Uses the same pinned cloudflared binary as proving-ground/deploy/deploy.sh
# (checked by ci/tool-versions.sh). No Cloudflare account, domain or credential
# is used; each run starts an Exposed Daemon with TLS and a token, opens a
# fresh trycloudflare.com URL exported in GOODFETCH_INTEGRATION_TUNNEL_URL,
# and exports the Daemon's token in GOODFETCH_INTEGRATION_DAEMON_TOKEN.
set -euo pipefail

CLOUDFLARED_VERSION=2026.10.7
CLOUDFLARED_SHA256=1f99fb518b83dec2ba8d870a7cc991d82a77e6196295a3d2754ba17ab9f91872
CLOUDFLARED_BIN="${CLOUDFLARED_BIN:-target/cloudflared/cloudflared}"
DAEMON_BIN="${DAEMON_BIN:-target/debug/goodfetch-collie}"
XTASK_BIN="${XTASK_BIN:-target/debug/xtask}"
JOB_KEY="live.yml#integration-accounts"

# Top-level state for EXIT trap (kept global so set -u stays safe after return).
WORK_DIR=""
DAEMON_PID=""
TUNNEL_PID=""

cleanup() {
  local pid
  for pid in "$TUNNEL_PID" "$DAEMON_PID"; do
    if [[ -n "$pid" ]]; then
      kill "$pid" 2>/dev/null || true
      wait "$pid" 2>/dev/null || true
    fi
  done
  if [[ -n "$WORK_DIR" ]]; then
    rm -rf "$WORK_DIR"
  fi
}

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

run_live_checks() {
  "$XTASK_BIN" results live-run --job "$JOB_KEY" --junit target/junit/live.xml
}

run_checks() {
  if ! has_live_checks; then
    run_live_checks
    return 0
  fi

  trap cleanup EXIT
  WORK_DIR=$(mktemp -d)
  local home="$WORK_DIR/home"
  local cert="$WORK_DIR/cert.pem"
  local key="$WORK_DIR/key.pem"
  local daemon_stdout="$WORK_DIR/daemon.stdout"
  local daemon_stderr="$WORK_DIR/daemon.stderr"
  local tunnel_log="$WORK_DIR/cloudflared.log"

  mkdir -p "$home"
  openssl req -x509 -newkey rsa:2048 -nodes \
    -keyout "$key" -out "$cert" -days 1 \
    -subj "/CN=localhost" \
    -addext "subjectAltName=DNS:localhost,IP:127.0.0.1" >/dev/null 2>&1

  local major
  major=$(GOODFETCH_HOME="$home" "$DAEMON_BIN" daemon status --json \
    | sed -n 's/.*"major"[[:space:]]*:[[:space:]]*\([0-9][0-9]*\).*/\1/p' | head -n 1)
  if [[ -z "$major" ]]; then
    echo "integration-accounts.sh: could not read protocol major from $DAEMON_BIN daemon status --json" >&2
    exit 1
  fi

  mkdir -p "$home/$major"
  cat >"$home/$major/configuration.toml" <<EOF
version = "0.1"
lowest_minor = 0

[settings.daemon.tls]
cert_file = "$cert"
key_file = "$key"
EOF

  GOODFETCH_HOME="$home" GOODFETCH_DAEMON_ADDRESS="127.0.0.1:0" \
    "$DAEMON_BIN" daemon run --json >"$daemon_stdout" 2>"$daemon_stderr" &
  DAEMON_PID=$!

  local daemon_address="" deadline=$((SECONDS + 30))
  while (( SECONDS < deadline )); do
    daemon_address=$(sed -n 's/.*"address"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' "$daemon_stdout" | head -n 1 || true)
    if [[ -n "$daemon_address" ]]; then
      break
    fi
    if ! kill -0 "$DAEMON_PID" 2>/dev/null; then
      cat "$daemon_stderr" >&2
      echo "integration-accounts.sh: Daemon exited before listening" >&2
      exit 1
    fi
    sleep 0.2
  done
  if [[ -z "$daemon_address" ]]; then
    cat "$daemon_stderr" >&2
    echo "integration-accounts.sh: timed out waiting for Daemon address" >&2
    exit 1
  fi

  local token_file="$home/$major/daemon.token"
  if [[ ! -f "$token_file" ]]; then
    echo "integration-accounts.sh: Daemon token file missing at $token_file" >&2
    exit 1
  fi
  local daemon_token
  daemon_token=$(tr -d '\r\n' <"$token_file")

  "$CLOUDFLARED_BIN" tunnel --no-autoupdate --protocol http2 \
    --origin-ca-pool "$cert" \
    --url "https://$daemon_address" >"$tunnel_log" 2>&1 &
  TUNNEL_PID=$!

  local tunnel_url=""
  deadline=$((SECONDS + 60))
  while (( SECONDS < deadline )); do
    tunnel_url=$(grep -Eo 'https://[a-z0-9-]+\.trycloudflare\.com' "$tunnel_log" | head -n 1 || true)
    if [[ -n "$tunnel_url" ]]; then
      break
    fi
    if ! kill -0 "$TUNNEL_PID" 2>/dev/null; then
      cat "$tunnel_log" >&2
      echo "integration-accounts.sh: cloudflared exited before publishing a Quick Tunnel URL" >&2
      exit 1
    fi
    sleep 0.2
  done
  if [[ -z "$tunnel_url" ]]; then
    cat "$tunnel_log" >&2
    echo "integration-accounts.sh: timed out waiting for Quick Tunnel URL" >&2
    exit 1
  fi

  export GOODFETCH_INTEGRATION_TUNNEL_URL="$tunnel_url"
  export GOODFETCH_INTEGRATION_DAEMON_TOKEN="$daemon_token"
  run_live_checks
}

case "${1:-}" in
  install) install_cloudflared ;;
  run) run_checks ;;
  *)
    echo "usage: $0 <install|run>" >&2
    exit 2
    ;;
esac
