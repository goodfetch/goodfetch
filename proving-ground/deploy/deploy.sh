#!/usr/bin/env bash
# The online copy's deploy (#168; ADR 0032), which the `proving ground online`
# workflow runs from a release tag under the protected `proving-ground-online`
# environment once the owner approved it; README.md says what it needs.
#
#   deploy.sh <the built goodfetch-proving-ground binary>
#
# It ships the binary, online.toml, Anubis and cloudflared (pinned, checked
# by SHA-256), their systemd units and the secrets online.toml names, read
# from the environment's secrets under the same names, to the host over SSH,
# restarts the services and checks the copy answers. With a `[botid]`
# section, it also deploys botid/ to Vercel with the copy's origin and the
# edge's secret. Every secret goes to a file of mode 600 on the host, never
# to the log.
set -euo pipefail

CLOUDFLARED_VERSION=2026.10.7
CLOUDFLARED_SHA256=1f99fb518b83dec2ba8d870a7cc991d82a77e6196295a3d2754ba17ab9f91872
ANUBIS_VERSION=1.28.2
ANUBIS_SHA256=69fa671d5438e4f54a50128228cf8c901611fea0ef5b25ad33f8514d8d24fe13
VERCEL_VERSION=65.12.1

binary=${1:?usage: deploy.sh <the built goodfetch-proving-ground binary>}
here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
config=$here/online.toml

fail() {
  echo "deploy.sh: $*" >&2
  exit 1
}

# Whether online.toml has the section `name`.
has_section() {
  grep -qx "\[$1\]" "$config"
}

# The `secret_env` of online.toml's section `name`, if it names one.
section_secret() {
  awk -v section="[$1]" '
    /^\[/ { inside = ($0 == section) }
    inside && $1 == "secret_env" { gsub(/"/, "", $3); print $3 }
  ' "$config"
}

# The value of the secret `name`, refused when empty or when it holds a
# character a systemd environment file would read otherwise.
secret() {
  local value=${!1:-}
  [[ -n $value ]] || fail "the secret $1 is not set in the environment"
  [[ $value =~ ^[A-Za-z0-9._~+/=-]+$ ]] || fail "the secret $1 holds a character outside [A-Za-z0-9._~+/=-]"
  printf '%s' "$value"
}

origin=$(awk -F' = ' '/^\[/ { exit } $1 == "origin" { gsub(/"/, "", $2); print $2 }' "$config")
[[ $origin == https://* ]] || fail "online.toml names no https origin"
host=${PROVING_GROUND_DEPLOY_HOST:?the deploy host, user@host, is not set}

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
root=$work/root
opt=$root/opt/goodfetch-proving-ground
etc=$root/etc/goodfetch-proving-ground
units=$root/etc/systemd/system
install -d "$opt" "$units"
install -d -m 700 "$etc"

# The copy, its configuration and its secrets.
install -m 755 "$binary" "$opt/goodfetch-proving-ground"
install -m 644 "$config" "$here/cloudflared.yml" "$opt/"
services=(goodfetch-proving-ground.service goodfetch-cloudflared.service)
(
  umask 077
  awk '$1 == "secret_env" { gsub(/"/, "", $3); print $3 }' "$config" | while read -r name; do
    value=$(secret "$name")
    printf '%s=%s\n' "$name" "$value"
  done >"$etc/online.env"
)

# The tunnel, its ID read from its credentials.
(
  umask 077
  printf '%s' "${PROVING_GROUND_TUNNEL_CREDENTIALS:?the tunnel credentials are not set}" >"$etc/tunnel.json"
  tunnel_id=$(jq -er '.TunnelID' "$etc/tunnel.json") || fail "the tunnel's credentials name no TunnelID"
  printf 'PROVING_GROUND_TUNNEL_ID=%s\n' "$tunnel_id" >"$etc/cloudflared.env"
)
curl -fsSL -o "$work/cloudflared" \
  "https://github.com/cloudflare/cloudflared/releases/download/$CLOUDFLARED_VERSION/cloudflared-linux-amd64"
echo "$CLOUDFLARED_SHA256  $work/cloudflared" | sha256sum --check --quiet
install -m 755 "$work/cloudflared" "$opt/cloudflared"

# Anubis, signing with the key the copy verifies its cookie with.
if has_section anubis; then
  anubis_key=$(secret "$(section_secret anubis)")
  (
    umask 077
    printf 'HS512_SECRET=%s\n' "$anubis_key" >"$etc/anubis.env"
  )
  curl -fsSL -o "$work/anubis.tar.gz" \
    "https://github.com/TecharoHQ/anubis/releases/download/v$ANUBIS_VERSION/anubis-$ANUBIS_VERSION-linux-amd64.tar.gz"
  echo "$ANUBIS_SHA256  $work/anubis.tar.gz" | sha256sum --check --quiet
  tar -xzf "$work/anubis.tar.gz" -C "$work"
  install -m 755 "$work/anubis-$ANUBIS_VERSION-linux-amd64/bin/anubis" "$opt/anubis"
  services+=(goodfetch-anubis.service)
fi
for service in "${services[@]}"; do
  install -m 644 "$here/$service" "$units/"
done

# The host: what the archive holds replaces what was there, the services
# restart, and a service no longer configured stops.
known_hosts=$work/known_hosts
key=$work/key
printf '%s\n' "${PROVING_GROUND_DEPLOY_KNOWN_HOSTS:?the known_hosts line of the deploy host is not set}" >"$known_hosts"
(
  umask 077
  printf '%s\n' "${PROVING_GROUND_DEPLOY_SSH_KEY:?the deploy key is not set}" >"$key"
)
ssh_options=(-i "$key" -o IdentitiesOnly=yes -o StrictHostKeyChecking=yes -o "UserKnownHostsFile=$known_hosts")
tar -C "$root" --owner=0 --group=0 -czf - \
  opt/goodfetch-proving-ground etc/goodfetch-proving-ground "${services[@]/#/etc/systemd/system/}" |
  ssh "${ssh_options[@]}" "$host" 'umask 077 && cat >/var/tmp/goodfetch-proving-ground.tar.gz'
vendor=$(awk '/^\[/ { gsub(/[][]/, ""); if ($0 != "edge") { print; exit } }' "$config")
ssh "${ssh_options[@]}" "$host" bash -s -- /var/tmp/goodfetch-proving-ground.tar.gz "$vendor" "${services[@]}" <<'REMOTE'
set -euo pipefail
archive=$1
vendor=$2
shift 2
rm -rf /etc/goodfetch-proving-ground
tar -xzf "$archive" -C / --no-overwrite-dir
rm "$archive"
if [[ ! " $* " =~ " goodfetch-anubis.service " && -e /etc/systemd/system/goodfetch-anubis.service ]]; then
  systemctl disable --now goodfetch-anubis.service
  rm /etc/systemd/system/goodfetch-anubis.service
fi
systemctl daemon-reload
systemctl enable "$@"
systemctl restart "$@"
for _ in $(seq 30); do
  if curl -fsS -o /dev/null -H 'Host: challenges.test' "http://127.0.0.1:8920/$vendor"; then
    exit 0
  fi
  sleep 1
done
systemctl status --no-pager "$@" >&2
echo "the copy does not answer /$vendor" >&2
exit 1
REMOTE

# BotID's host on Vercel.
if has_section botid; then
  : "${VERCEL_TOKEN:?} ${VERCEL_ORG_ID:?} ${VERCEL_PROJECT_ID:?}"
  edge_secret=$(secret "$(section_secret edge)")
  (cd "$here/botid" && npx --yes "vercel@$VERCEL_VERSION" deploy --prod --yes --token "$VERCEL_TOKEN" \
    --env "PROVING_GROUND_ORIGIN=$origin" --env "PROVING_GROUND_EDGE_SECRET=$edge_secret")
fi
