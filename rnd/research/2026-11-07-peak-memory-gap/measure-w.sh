#!/usr/bin/env bash
# peak-gap phase 3: scenario 1 wall and CPU time of each cut against #2165's
# build (fh-k, no knob), no sampler. Per round i, the base and every variant
# run once, in an order rotated each round; each variant pairs with that
# round's base (wall.py: "on" = base, "off" = the cut).
# usage: measure-w.sh <rounds-at-1000> <rounds-at-10000> <cfg...>  (cfg = tag:exe:VAR=v,VAR=v)
set -uo pipefail
source /Users/jb/agent-targets/peak-gap/scripts/env.sh
S=$PC/scripts R=$PC/runs/wall
mkdir -p "$R"
r1=$1 r2=$2; shift 2
cfgs=("base:${BASE:-fh-k}:" "$@")
one() { # <cfg> <pages> <i>
  local c=$1 n=$2 i=$3 tag=${1%%:*} rest=${1#*:}; local exe=${rest%%:*} e=${rest#*:}
  local out=$R/run-$tag-$n-$i
  uptime > "$out.load"
  WRAP_MODE=plain WRAP_ENV="PEAKCUT_THP=3${e:+ ${e//,/ }}" WRAP_EXE=$PC/bin/$exe CONC=16 \
    "$S/trial.sh" raw foxhound "$S/wrap.sh" "$n" timed > "$out.json" 2> "$out.err"
}
for spec in "1000 $r1" "10000 $r2"; do
  set -- $spec; n=$1 rounds=$2
  for i in $(seq 1 "$rounds"); do
    k=${#cfgs[@]}
    for j in $(seq 0 $((k - 1))); do one "${cfgs[$(( (i + j) % k ))]}" "$n" "$i"; done
    for c in "${cfgs[@]:1}"; do
      t=${c%%:*}
      cp "$R/run-base-$n-$i.json" "$R/$t-on-$n-$i.json"
      cp "$R/run-$t-$n-$i.json" "$R/$t-off-$n-$i.json"
    done
  done
done
echo PHASE-W-DONE
