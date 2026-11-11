#!/usr/bin/env bash
# peak-cuts phase D: scenario 1's wall time, the best PGO layout against the
# plain build, THP off in both, interleaved pairs, no sampler.
# usage: measure-d.sh <pairs-at-1000> <pairs-at-10000>
set -uo pipefail
source /Users/jb/agent-targets/peak-cuts/scripts/env.sh
S=$PC/scripts R=$PC/runs/wall
mkdir -p "$R"; cd "$R"
one() { # <label> <exe> <pages> <i>
  local out=$R/pgo-$1-$3-$4
  uptime > "$out.load"
  WRAP_MODE=plain WRAP_ENV="PEAKCUT_THP=3" WRAP_EXE=$2 CONC=16 \
    "$S/trial.sh" raw foxhound "$S/wrap.sh" "$3" timed > "$out.json" 2> "$out.err"
}
for spec in "1000 $1" "10000 $2"; do
  set -- $spec; n=$1 pairs=$2
  for i in $(seq 1 "$pairs"); do
    if (( i % 2 )); then one on "$PC/bin/fh-x" "$n" "$i"; one off "$PC/bin/fh-pgo-both" "$n" "$i"
    else one off "$PC/bin/fh-pgo-both" "$n" "$i"; one on "$PC/bin/fh-x" "$n" "$i"; fi
  done
done
echo PHASE-D-DONE
