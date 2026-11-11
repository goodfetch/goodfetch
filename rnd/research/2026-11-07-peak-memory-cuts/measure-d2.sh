#!/usr/bin/env bash
# peak-cuts phase D2: scenario 1 wall time, fh-pgo-order ("off") against fh-pgo ("on"),
# THP off in both, interleaved pairs, no sampler: the order file's own effect.
# usage: measure-d.sh <pairs-at-1000> <pairs-at-10000>
set -uo pipefail
source /Users/jb/agent-targets/peak-cuts/scripts/env.sh
S=$PC/scripts R=$PC/runs/wall
mkdir -p "$R"; cd "$R"
one() { # <label> <exe> <pages> <i>
  local out=$R/ord-$1-$3-$4
  uptime > "$out.load"
  WRAP_MODE=plain WRAP_ENV="PEAKCUT_THP=3" WRAP_EXE=$2 CONC=16 \
    "$S/trial.sh" raw foxhound "$S/wrap.sh" "$3" timed > "$out.json" 2> "$out.err"
}
for spec in "1000 $1" "10000 $2"; do
  set -- $spec; n=$1 pairs=$2
  for i in $(seq 1 "$pairs"); do
    if (( i % 2 )); then one on "$PC/bin/fh-pgo" "$n" "$i"; one off "$PC/bin/fh-pgo-order" "$n" "$i"
    else one off "$PC/bin/fh-pgo-order" "$n" "$i"; one on "$PC/bin/fh-pgo" "$n" "$i"; fi
  done
done
echo PHASE-D2-DONE
