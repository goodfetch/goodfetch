#!/usr/bin/env bash
# The `cargo nextest` that the Quick suite's `cargo insta test` runs, through
# INSTA_CARGO_NEXTEST_BIN: `cargo nextest` with the arguments cargo-insta
# gives, and with a filterset that keeps the tests of the workspace members
# GOODFETCH_QUICK_PACKAGES names, space-separated, when it names any. `just
# test quick` builds every member's in-process tests, so its build is the
# same whatever changed, and selects through this filterset, which
# cargo-insta has no option for: it hands its trailing arguments to the test
# binaries. With no member named, every test runs.
# It keeps to bash 3.2, which stock macOS runs.
set -euo pipefail

cargo=${CARGO:-cargo}
command=$1
shift
filter=
for name in ${GOODFETCH_QUICK_PACKAGES:-}; do
  filter="${filter:+$filter | }package(=$name)"
done
if [[ -z $filter ]]; then
  exec "$cargo" nextest "$command" "$@"
fi
exec "$cargo" nextest "$command" -E "$filter" "$@"
