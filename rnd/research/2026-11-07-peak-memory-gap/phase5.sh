#!/usr/bin/env bash
# peak-gap phase 5: BOLT (non-PIE; the static-PIE output crashes) against its
# input, peaks and wall time; then the exact touch map: one 1,000-page trial of
# fh-k-rand (MADV_RANDOM: no read-around) with fault-around at one page
# (debugfs fault_around_bytes 4096, restored right after), so every resident
# page of the executable is one the process touched itself.
set -uo pipefail
source /Users/jb/agent-targets/peak-gap/scripts/env.sh
S=$PC/scripts
"$S/measure-x.sh" b- 3 "100 1000 10000" nonpie:fh-pgo-order: nbolt:fh-pgo-order-bolt: k:fh-k:
mkdir -p "$PC/runs/wall-bolt"
BASE=fh-pgo-order "$S/measure-w.sh" 12 6 nbolt:fh-pgo-order-bolt:
mv "$PC"/runs/wall/nbolt-* "$PC"/runs/wall/run-*nbolt* "$PC/runs/wall-bolt/" 2>/dev/null
F=/sys/kernel/debug/fault_around_bytes
old=$(sudo -n cat $F)
trap 'echo '"$old"' | sudo -n tee '"$F"' >/dev/null' EXIT
echo 4096 | sudo -n tee $F >/dev/null
for i in 1 2; do
  EVICT=1 WRAP_ENV="PEAKCUT_THP=3" CONC=16 "$S/counted.sh" "touch-krand-r$i" raw foxhound "$PC/bin/fh-k-rand" sample 1000
done
echo "$old" | sudo -n tee $F >/dev/null
echo "fault_around_bytes back to $(sudo -n cat $F)"
echo PHASE5-DONE
