#!/usr/bin/env bash
# `cargo fuzz check` of every fuzz target (ADR 0067; TESTING.md, Fuzzing), in
# the Pre-merge suite on Linux x64: each target still builds, with the
# sanitizer flags fuzzing uses, on the pinned nightly of
# ci/sanitizer-toolchain.toml. No fuzzing runs here, nor in any pull
# request; the targets' seeds and crashers replay in the Quick suite on every
# OS (`fuzz_replay!`).
#
#   ci/fuzz-check.sh fetch   fetches each fuzz crate's locked dependencies,
#                            before the egress deny (`just fetch`);
#   ci/fuzz-check.sh check   checks every target of every fuzz crate.
#
# A fuzz crate is a `fuzz/Cargo.toml` the repository tracks whose package
# carries `cargo-fuzz = true` (each sits outside the workspace, with its own
# Cargo.lock). With none, both pass and say so. Each crate's Cargo.lock must
# be current: a stale one fails rather than being rewritten.
set -euo pipefail

root=$(cd "$(dirname "$0")/.." && pwd)
cd "$root"

pinned() { sed -n "s/^$1 = \"\(.*\)\"\$/\1/p" ci/sanitizer-toolchain.toml; }

crates=()
while IFS= read -r manifest; do
  if grep -q '^cargo-fuzz = true' "$manifest"; then
    crates+=("$(dirname "$manifest")")
  fi
done < <(git ls-files -- 'fuzz/Cargo.toml' '*/fuzz/Cargo.toml')

if [[ ${#crates[@]} -eq 0 ]]; then
  echo "No fuzz target yet: nothing to check."
  exit 0
fi

case "${1:-}" in
  fetch)
    for crate in "${crates[@]}"; do
      cargo fetch --locked --manifest-path "$crate/Cargo.toml"
    done
    ;;
  check)
    toolchain=$(pinned toolchain)
    # As ci/tool.sh does for a tool: a local host without the pinned
    # toolchain lists the check as skipped; in CI it fails.
    if ! rustup toolchain list | grep -q "^$toolchain"; then
      if [[ -n "${CI:-}" ]]; then
        echo "error: the pinned toolchain $toolchain is not installed, and CI runs every part" >&2
        exit 1
      fi
      mkdir -p target
      echo "ci/fuzz-check.sh check: skipped, the pinned toolchain $toolchain is not installed on this host" >>target/skipped.txt
      echo "skipped: ci/fuzz-check.sh check ($toolchain is not installed on this host)" >&2
      exit 0
    fi
    for crate in "${crates[@]}"; do
      echo "cargo fuzz check: $crate"
      cargo "+$toolchain" metadata --locked --offline --format-version 1 \
        --manifest-path "$crate/Cargo.toml" >/dev/null
      (cd "$(dirname "$crate")" && cargo "+$toolchain" fuzz check --fuzz-dir "$(basename "$crate")")
    done
    ;;
  *)
    echo "usage: ci/fuzz-check.sh fetch|check" >&2
    exit 2
    ;;
esac
