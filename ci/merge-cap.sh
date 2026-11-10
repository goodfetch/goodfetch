#!/usr/bin/env bash
# The merge cap (ADR 0135; TESTING.md, Commands), which `just merge` runs
# before its gate: it refuses once CAP merges have landed on `origin/main`
# since the last green run of the incremental gate's backstop, so the
# merges no full run has checked stay at most CAP. The backstop runs daily
# (ci/gate-backstop.sh); its record is `<log>/results.txt`, `<log>` being
# GOODFETCH_GATE_BACKSTOP_LOG or ~/gate-backstop, and a green run is a line
# `<day> <commit> gate=pass misses=0 …`. The cap counts the first-parent
# commits of `origin/main` since the latest green run whose commit `main`
# holds. Until the backstop has had a green run it blocks nothing and says
# so; when green runs exist but `main` holds none of their commits, it
# refuses rather than guess. While the backstop is switched off on `main`
# (`ci/gate-backstop.state`), it counts nothing and says so. `--past-cap`, the owner's explicit override,
# merges anyway and says so. It reads refs only: the caller fetches. It
# keeps to bash 3.2, which stock macOS runs.
#   ci/merge-cap.sh [--past-cap]
set -euo pipefail

# Twice the most merges `main` took in a day since the local gate began
# (5, on 2026-11-03): one red or skipped daily backstop run leaves a day of
# merges, and the next merge after a second such day is refused.
CAP=10

past_cap=false
case "${1:-}" in
  "") ;;
  --past-cap) past_cap=true ;;
  *)
    echo "error: the only option is --past-cap, not $1" >&2
    exit 2
    ;;
esac

ref=origin/main
results=${GOODFETCH_GATE_BACKSTOP_LOG:-$HOME/gate-backstop}/results.txt
# shellcheck source=ci/backstop-green.sh
source "$(dirname "$0")/backstop-green.sh"
# The backstop's switch, as `main` holds it: while it is off no green run
# comes, so the cap has nothing to count from and blocks no merge.
state=$(backstop_state "$(git show "$ref:ci/gate-backstop.state" 2>/dev/null || true)")
if [[ $state == off ]]; then
  echo "The merge cap: the backstop is off on $ref (ci/gate-backstop.state), so the cap counts nothing"
  exit 0
fi
greens=$(backstop_greens "$results")
if [[ -z $greens ]]; then
  echo "The merge cap: the backstop has no green run yet in $results, so the cap counts nothing until its first one"
  exit 0
fi

tested=$(latest_green "$greens" "$ref")

refuse() {
  if $past_cap; then
    echo "The merge cap: $1; merging past the cap of $CAP (--past-cap)"
    exit 0
  fi
  echo "error: $1, the cap of $CAP (ADR 0135): fix the backstop until it runs green on $ref, or let the owner pass --past-cap" >&2
  exit 1
}

if [[ -z $tested ]]; then
  refuse "none of the backstop's green runs in $results is on $ref's history"
fi
merges=$(git rev-list --count --first-parent "$tested..$ref")
said="$merges merges have landed on $ref since the backstop's green run on $tested"
if ((merges >= CAP)); then
  refuse "$said"
fi
echo "The merge cap: $said, under the cap of $CAP"
