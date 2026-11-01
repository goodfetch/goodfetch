#!/usr/bin/env bash
# Prints the seed that orders the tests (TESTING.md, Before merge): the
# GOODFETCH_ORDER_SEED given, as the constrained test-health run draws a
# fresh one, else the first eight bytes of the tree under test, HEAD's, read
# as an unsigned integer, so an order repeats with its tree. `xtask
# order-seed` prints the same value; this script serves where no Cargo runs, such
# as a job's host. `just test`, `just repeat`, the Release cells and
# ci/junit-context.sh read it.
set -euo pipefail

if [[ -n "${GOODFETCH_ORDER_SEED:-}" ]]; then
  echo "$GOODFETCH_ORDER_SEED"
  exit 0
fi
tree=$(git rev-parse 'HEAD^{tree}')
# Bash's arithmetic is signed 64-bit; `%u` prints the same bits unsigned.
printf '%u\n' "$((16#${tree:0:16}))"
