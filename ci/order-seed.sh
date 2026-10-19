#!/usr/bin/env bash
# Prints the seed that orders the tests (TESTING.md, Before merge): the
# GOODFETCH_ORDER_SEED given, as the constrained test-health run draws a
# fresh one, else the first eight hexadecimal digits of the tree under test,
# HEAD's, as a decimal number, so an order repeats with its tree.
# `just test`, the Release cells and ci/junit-context.sh read it.
set -euo pipefail

if [[ -n "${GOODFETCH_ORDER_SEED:-}" ]]; then
  echo "$GOODFETCH_ORDER_SEED"
  exit 0
fi
tree=$(git rev-parse 'HEAD^{tree}')
echo $((16#${tree:0:8}))
