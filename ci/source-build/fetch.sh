#!/usr/bin/env bash
# Builds the clean container of ci/source-build/Containerfile with the
# container engine that answers on this host, tagged by the Containerfile's
# digest as goodfetch-collie's `source_build` test reads it, so the Release
# suite's build of the crate from source runs it under ADR 0043's deny with
# no download. Part of `just fetch`. Nothing is built on a host that is not
# Linux x86-64, where that test does not run, when the image is already
# there, or when no engine answers: the test then fails with this command,
# never skips. It tests nothing, so it may retry (ADR 0064).
set -euo pipefail

root=$(cd "$(dirname "$0")/../.." && pwd)
# shellcheck source=ci/container.sh
source "$root/ci/container.sh"

[[ "$(uname -s)" == Linux && "$(uname -m)" == x86_64 ]] || exit 0
engine=$(container_engine)
if [[ -z "$engine" ]]; then
  echo "No container engine answers: the image of ci/source-build/Containerfile is not built"
  exit 0
fi

file="$root/ci/source-build/Containerfile"
image="goodfetch-source-build:$(sha256sum "$file" | cut -c1-16)"
"$engine" image inspect "$image" >/dev/null 2>&1 && exit 0
for attempt in 1 2 3; do
  "$engine" build --quiet --tag "$image" --file "$file" "$root/ci/source-build" >/dev/null && exit 0
  echo "building $image failed (attempt $attempt of 3)" >&2
  sleep $((attempt * 5))
done
exit 1
