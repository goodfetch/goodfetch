#!/usr/bin/env bash
# Pulls the Azure Blob Sink conformance tests' pinned Azurite image
# (ci/azurite/image.toml). Part of `just fetch`.
set -euo pipefail

root=$(cd "$(dirname "$0")/../.." && pwd)
# shellcheck source=ci/container.sh
source "$root/ci/container.sh"

[[ "$(uname -s)" == Linux ]] || exit 0
engine=$(container_engine)
if [[ -z "$engine" ]]; then
  echo "No container engine answers: the Azurite image of ci/azurite/image.toml is not pulled"
  exit 0
fi

key() { sed -n "s/^$1 = \"\\(.*\\)\"\$/\\1/p" "$root/ci/azurite/image.toml"; }
image="$(key repository)@$(key digest)"
"$engine" image inspect "$image" >/dev/null 2>&1 && exit 0
for attempt in 1 2 3; do
  "$engine" pull --quiet "$image" >/dev/null && exit 0
  echo "pulling $image failed (attempt $attempt of 3)" >&2
  sleep $((attempt * 5))
done
exit 1
