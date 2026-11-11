#!/usr/bin/env bash
# peak-cuts phase B: scenario 1 (raw, 16 in flight) wall time with mimalloc's
# huge pages on (as shipped) and off (PEAKCUT_THP=3, which matches
# MIMALLOC_ALLOW_THP=0), no sampler, in interleaved pairs whose order
# alternates. usage: measure-b.sh <exe> <tag> <pairs-at-1000> <pairs-at-10000>
set -uo pipefail
source /Users/jb/agent-targets/peak-cuts/scripts/env.sh
S=$PC/scripts R=$PC/runs/wall
exe=$1 tag=$2 p1=$3 p2=$4
mkdir -p "$R"; cd "$R"
one() { # <label> <env> <pages> <i>
  local out=$R/$tag-$1-$3-$4
  uptime > "$out.load"
  WRAP_MODE=plain WRAP_ENV="$2" WRAP_EXE=$exe CONC=16 \
    "$S/trial.sh" raw foxhound "$S/wrap.sh" "$3" timed > "$out.json" 2> "$out.err"
}
specs=("1000 $p1" "10000 $p2")
[[ -n ${ONLY:-} ]] && specs=("$ONLY")
for spec in "${specs[@]}"; do
  set -- $spec; n=$1 pairs=$2
  for i in $(seq "${START:-1}" "$pairs"); do
    if (( i % 2 )); then one on "" "$n" "$i"; one off "PEAKCUT_THP=3" "$n" "$i"
    else one off "PEAKCUT_THP=3" "$n" "$i"; one on "" "$n" "$i"; fi
  done
done
echo PHASE-B-DONE
