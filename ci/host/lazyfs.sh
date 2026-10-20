#!/usr/bin/env bash
# Builds LazyFS on a hosted Linux runner (ADR 0060), before the deny: FUSE 3
# (its library, headers and fusermount3) and the build tools from the
# runner's packages, then `just lazyfs-build`, which downloads LazyFS's
# pinned source archives, checks each SHA-256 and builds from them alone.
# Its tests then run under the deny through `ci/release/cell.sh lazyfs`.
set -euo pipefail

root=$(cd "$(dirname "$0")/../.." && pwd)

sudo apt-get update --quiet
sudo apt-get install --yes --no-install-recommends fuse3 libfuse3-dev cmake g++ pkg-config
[[ -c /dev/fuse ]] || {
  echo "error: /dev/fuse is missing, so no LazyFS mount can be made" >&2
  exit 1
}
cd "$root"
just lazyfs-build
