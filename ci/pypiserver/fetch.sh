#!/usr/bin/env bash
# Pulls the release plan tests' pinned PyPI stand-in
# (ci/pypiserver/image.toml) with the container engine that answers on this
# host, so those tests run it under ADR 0043's deny with no download (xtask's
# `tests/it/container.rs`). Part of `just fetch`. Nothing is pulled on a host
# that is not Linux, where those tests do not run, when the image is already
# there, or when no engine answers: the tests then fail with the command that
# pulls it, never skip. It tests nothing, so it may retry (ADR 0064).
set -euo pipefail

root=$(cd "$(dirname "$0")/../.." && pwd)
# shellcheck source=ci/container.sh
source "$root/ci/container.sh"

[[ "$(uname -s)" == Linux ]] || exit 0
engine=$(container_engine)
if [[ -z "$engine" ]]; then
  echo "No container engine answers: the pypiserver image of ci/pypiserver/image.toml is not pulled"
  exit 0
fi

key() { sed -n "s/^$1 = \"\\(.*\\)\"\$/\\1/p" "$root/ci/pypiserver/image.toml"; }
image="$(key repository)@$(key digest)"
"$engine" image inspect "$image" >/dev/null 2>&1 && exit 0
for attempt in 1 2 3; do
  "$engine" pull --quiet "$image" >/dev/null && exit 0
  echo "pulling $image failed (attempt $attempt of 3)" >&2
  sleep $((attempt * 5))
done
exit 1
