#!/usr/bin/env bash
# The arguments nextest takes to skip what every suite skips, one per line:
# the quarantined tests (ADR 0064), from the exclusions `xtask quarantine
# exclude` printed, its one argument, then, on a local host without an OS
# service the tests use (an extra loopback address, the Secret Service),
# the tests that need it, which ci/os-services.sh lists in
# target/skipped.txt; in CI a missing service fails instead. `just test`,
# ci/mutants.sh and ci/asan.sh read them, each having listed the
# exclusions once:
#   exclusions=$(cargo run --quiet --bin xtask -- quarantine exclude)
#   skips=$(ci/nextest-skips.sh "$exclusions")
#   while IFS= read -r argument; do [[ -z "$argument" ]] || args+=("$argument"); done <<<"$skips"
set -euo pipefail

exclusions=${1?usage: ci/nextest-skips.sh <the JSON xtask quarantine exclude printed>}

root=$(cd "$(dirname "$0")/.." && pwd)
cd "$root"

quarantine=$(jq -r '.nextest[]' <<<"$exclusions")
[[ -z "$quarantine" ]] || printf '%s\n' "$quarantine"

# The quarantine's arguments start with `--`, after which nextest passes
# `--skip` to each test binary; with none, the service skips add it.
services=$(ci/os-services.sh skipped target/skipped.txt)
if [[ -n "$services" ]]; then
  [[ -n "$quarantine" ]] || printf '%s\n' --
  while IFS= read -r pattern; do
    printf '%s\n%s\n' --skip "$pattern"
  done <<<"$services"
fi
