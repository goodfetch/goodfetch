#!/usr/bin/env bash
# Pulls the PostgreSQL/Supabase Sink conformance tests' pinned Postgres image
# (ci/postgres/image.toml). Part of `just fetch`.
set -euo pipefail

root=$(cd "$(dirname "$0")/../.." && pwd)
# shellcheck source=ci/container.sh
source "$root/ci/container.sh"

[[ "$(uname -s)" == Linux ]] || exit 0
engine=$(container_engine)
if [[ -z "$engine" ]]; then
  echo "No container engine answers: the Postgres image of ci/postgres/image.toml is not pulled"
  exit 0
fi

key() { sed -n "s/^$1 = \"\\(.*\\)\"\$/\\1/p" "$root/ci/postgres/image.toml"; }
image="$(key repository)@$(key digest)"
"$engine" image inspect "$image" >/dev/null 2>&1 && exit 0
for attempt in 1 2 3; do
  "$engine" pull --quiet "$image" >/dev/null && exit 0
  echo "pulling $image failed (attempt $attempt of 3)" >&2
  sleep $((attempt * 5))
done
exit 1
