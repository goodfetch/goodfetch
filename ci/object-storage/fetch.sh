#!/usr/bin/env bash
# Pulls the Checkpoint object-storage tests' pinned emulator images
# (ci/object-storage/images.toml) with the container engine that answers on
# this host, so those tests run under ADR 0043's deny with no download. Part
# of `just fetch`. Nothing is pulled on a host that is not Linux, when an
# image is already present, or when no engine answers: the tests then fail
# with the command that pulls it, never skip. It tests nothing, so it may
# retry (ADR 0064).
set -euo pipefail

root=$(cd "$(dirname "$0")/../.." && pwd)
# shellcheck source=ci/container.sh
source "$root/ci/container.sh"

[[ "$(uname -s)" == Linux ]] || exit 0
engine=$(container_engine)
if [[ -z "$engine" ]]; then
  echo "No container engine answers: the images of ci/object-storage/images.toml are not pulled"
  exit 0
fi

pull_section() {
  local section="$1"
  local repo digest image
  repo=$(awk -v s="[$section]" '$0==s{f=1;next} /^\[/{f=0} f && /^repository = /{gsub(/"/,"",$3); print $3}' "$root/ci/object-storage/images.toml")
  digest=$(awk -v s="[$section]" '$0==s{f=1;next} /^\[/{f=0} f && /^digest = /{gsub(/"/,"",$3); print $3}' "$root/ci/object-storage/images.toml")
  image="${repo}@${digest}"
  "$engine" image inspect "$image" >/dev/null 2>&1 && return 0
  for attempt in 1 2 3; do
    "$engine" pull --quiet "$image" >/dev/null && return 0
    echo "pulling $image failed (attempt $attempt of 3)" >&2
    sleep $((attempt * 5))
  done
  return 1
}

pull_section rustfs
pull_section azurite
pull_section fake_gcs
