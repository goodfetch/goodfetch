#!/usr/bin/env bash
# Pulls the pinned n8n and Activepieces Integration harness images
# (ci/integrations/images.toml) with the container engine that answers on
# this host, so Integration tests run them under ADR 0043's outbound-traffic
# deny with no download (ADR 0070; issue 426). Part of `just fetch`. Nothing
# is pulled off Linux or when no container engine answers; tests then fail
# naming this script. It tests nothing, so it may retry (ADR 0064).
set -euo pipefail

root=$(cd "$(dirname "$0")/../.." && pwd)
# shellcheck source=ci/container.sh
source "$root/ci/container.sh"

[[ "$(uname -s)" == Linux ]] || exit 0
engine=$(container_engine)
if [[ -z "$engine" ]]; then
  echo "No container engine answers: the images of ci/integrations/images.toml are not pulled"
  exit 0
fi

pull_section() {
  local section="$1"
  local repository digest image
  repository=$(awk -v s="[$section]" '
    $0 == s { in_s = 1; next }
    /^\[/ { in_s = 0 }
    in_s && /^repository = / { gsub(/^repository = "|"$/, ""); print; exit }
  ' "$root/ci/integrations/images.toml")
  digest=$(awk -v s="[$section]" '
    $0 == s { in_s = 1; next }
    /^\[/ { in_s = 0 }
    in_s && /^digest = / { gsub(/^digest = "|"$/, ""); print; exit }
  ' "$root/ci/integrations/images.toml")
  if [[ -z "$repository" || -z "$digest" ]]; then
    echo "error: missing repository or digest for [$section] in ci/integrations/images.toml" >&2
    exit 1
  fi
  image="${repository}@${digest}"
  "$engine" image inspect "$image" >/dev/null 2>&1 && return 0
  for attempt in 1 2 3; do
    "$engine" pull --quiet "$image" >/dev/null && return 0
    echo "pulling $image failed (attempt $attempt of 3)" >&2
    sleep $((attempt * 5))
  done
  exit 1
}

pull_section n8n
pull_section activepieces
