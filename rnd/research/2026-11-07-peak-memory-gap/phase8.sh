#!/usr/bin/env bash
# peak-gap phase 8: the second touch map, on the ord3 layout (fh-k-ord3-rand),
# fault-around at one page for these two trials only (restored by the trap).
set -uo pipefail
source /Users/jb/agent-targets/peak-gap/scripts/env.sh
S=$PC/scripts
F=/sys/kernel/debug/fault_around_bytes
old=$(sudo -n cat $F)
trap 'echo '"$old"' | sudo -n tee '"$F"' >/dev/null' EXIT
echo 4096 | sudo -n tee $F >/dev/null
for i in 1 2; do
  EVICT=1 WRAP_ENV="PEAKCUT_THP=3" CONC=16 "$S/counted.sh" "touch3-r$i" raw foxhound "$PC/bin/fh-k-ord3-rand" sample 1000
done
echo "$old" | sudo -n tee $F >/dev/null
echo PHASE8-DONE
