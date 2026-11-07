#!/usr/bin/env bash
# peak-cuts phase C, second pass: the order-file variants with fh-pgo and fh-x
# as controls and Spider beside them; as measure-c.sh.
set -uo pipefail
source /Users/jb/agent-targets/peak-cuts/scripts/env.sh
export EVICT=1
S=$PC/scripts B=$PC/bin
cfgs=(d-x3:fh-x d-pgo:fh-pgo d-order:fh-pgo-order d-both:fh-pgo-both d-xorder:fh-x-order d-orelr:fh-pgo-order-relr)
for c in "${cfgs[@]}"; do "$S/idle.sh" "idle-${c%%:*}" "$B/${c#*:}" PEAKCUT_THP=3; done
for r in 1 2 3; do
  for n in 100 1000 10000; do
    CONC=16 "$S/counted.sh" "d-sp-r$r" raw spider "$B/spider" sample "$n"
    for c in "${cfgs[@]}"; do
      WRAP_ENV="PEAKCUT_THP=3" CONC=16 "$S/counted.sh" "${c%%:*}-r$r" raw foxhound "$B/${c#*:}" sample "$n"
    done
  done
done
echo PHASE-C2-DONE
