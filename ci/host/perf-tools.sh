#!/usr/bin/env bash
# Installs `xtask perf`'s tools on a hosted Linux runner (ADR 0071), before
# the deny: Valgrind from the runner's packages, and gungraun-runner at the
# version of the `gungraun` crate in Cargo.lock, which it must match. The CI
# image carries the same tools (ci/image/Containerfile).
set -euo pipefail

root=$(cd "$(dirname "$0")/../.." && pwd)
version=$(sed -n '/^name = "gungraun"$/{n;s/^version = "\(.*\)"$/\1/p;}' "$root/Cargo.lock")
[[ -n "$version" ]] || {
  echo "error: Cargo.lock has no gungraun package" >&2
  exit 2
}

sudo apt-get update --quiet
sudo apt-get install --yes --no-install-recommends valgrind
cargo install --locked "gungraun-runner@$version"
