#!/usr/bin/env bash
# Prints, one per line, the arguments that order a runner's tests by the
# order seed (TESTING.md, Before merge): `ci/order-arguments.sh <runner>`,
# for vitest, playwright or pytest, with the seed ci/order-seed.sh prints.
# The justfile's recipes pass them, so one seed gives one order in each
# runner; nextest runs each test in its own process and takes none.
# xtask's stress tests run each runner with them twice on one seed and once
# on another.
set -euo pipefail

runner=${1:?usage: ci/order-arguments.sh vitest|playwright|pytest}
seed=$("$(dirname "$0")/order-seed.sh")
case "$runner" in
  vitest) printf '%s\n' --sequence.shuffle.tests --sequence.shuffle.files "--sequence.seed=$seed" ;;
  playwright) printf '%s\n' "--shuffle=$seed" ;;
  pytest) printf '%s\n' -p randomly "--randomly-seed=$seed" ;;
  *)
    echo "error: no runner $runner; give vitest, playwright or pytest" >&2
    exit 2
    ;;
esac
