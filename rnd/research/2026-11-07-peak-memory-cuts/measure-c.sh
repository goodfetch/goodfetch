#!/usr/bin/env bash
# peak-cuts phase C: peaks of the code-layout variants, THP off (PEAKCUT_THP=3)
# in all, fh-x as the control and Spider beside them; every executable evicted
# from the page cache before every trial; 3 trials per size, interleaved.
set -uo pipefail
source /Users/jb/agent-targets/peak-cuts/scripts/env.sh
export EVICT=1
S=$PC/scripts B=$PC/bin
cfgs=(c-x3:fh-x c-pgo:fh-pgo c-prefix:fh-pgo-prefix c-order:fh-pgo-order c-both:fh-pgo-both c-xorder:fh-x-order c-bothrelr:fh-pgo-both-relr)
for c in "${cfgs[@]}"; do "$S/idle.sh" "idle-${c%%:*}" "$B/${c#*:}" PEAKCUT_THP=3; done
for r in 1 2 3; do
  for n in 100 1000 10000; do
    CONC=16 "$S/counted.sh" "c-sp-r$r" raw spider "$B/spider" sample "$n"
    for c in "${cfgs[@]}"; do
      WRAP_ENV="PEAKCUT_THP=3" CONC=16 "$S/counted.sh" "${c%%:*}-r$r" raw foxhound "$B/${c#*:}" sample "$n"
    done
  done
done
echo PHASE-C-DONE
