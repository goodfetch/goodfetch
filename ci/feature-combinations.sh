#!/usr/bin/env bash
# cargo-hack over every combination of the Cargo features of each Product's
# published Rust SDK crate (ADR 0087; STYLE.md, Cargo features): features
# exist only there, for heavy optional dependencies and `test-util`, and are
# additive, so each combination must build.
#
#   ci/feature-combinations.sh [<workspace Cargo.toml>]
#
# A crate that does not exist yet (area 3 adds them) is skipped and listed;
# the check is still proven on ci/proofs/feature-combinations/.
set -euo pipefail

repository=$(cd "$(dirname "$0")/.." && pwd)
manifest=${1:-$repository/Cargo.toml}

# Each Product's published Rust SDK crate (ADR 0090).
crates=(goodfetch-collie goodfetch-foxhound)

members=$(cargo metadata --format-version 1 --no-deps --manifest-path "$manifest" | jq -r '.packages[].name')
packages=()
for crate in "${crates[@]}"; do
  if grep -qx "$crate" <<<"$members"; then
    packages+=(--package "$crate")
  else
    mkdir -p "$repository/target"
    echo "cargo-hack on $crate: skipped, the crate does not exist yet" >>"$repository/target/skipped.txt"
    echo "skipped: $crate does not exist yet"
  fi
done
[[ ${#packages[@]} -gt 0 ]] || exit 0

cargo hack check --manifest-path "$manifest" --feature-powerset --no-dev-deps --locked "${packages[@]}"
