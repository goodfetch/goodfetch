#!/usr/bin/env bash
# peak-cuts phase A: peak RSS of each THP mode and of packed relocations on
# the musl executable, beside Spider in the same run; 16 in flight; 3 trials
# per size, the configurations interleaved within each trial.
set -uo pipefail
source /Users/jb/agent-targets/peak-cuts/scripts/env.sh
S=$PC/scripts B=$PC/bin R=$PC/${RUNS:-runs}
export EVICT=${EVICT:-0}
mkdir -p "$R"; cd "$R"
{ uname -r; cat /sys/kernel/mm/transparent_hugepage/enabled /sys/kernel/mm/transparent_hugepage/defrag; } > host.txt
declare -A EXE=( [base]=$B/fh-base [baseenv]=$B/fh-base [x0]=$B/fh-x [x1]=$B/fh-x [x2]=$B/fh-x [x3]=$B/fh-x [pie3]=$B/fh-x-pie [relr3]=$B/fh-x-relr )
declare -A ENV=( [base]="" [baseenv]="MIMALLOC_ALLOW_THP=0" [x0]="" [x1]="PEAKCUT_THP=1" [x2]="PEAKCUT_THP=2" [x3]="PEAKCUT_THP=3" [pie3]="PEAKCUT_THP=3" [relr3]="PEAKCUT_THP=3" )
order=(base baseenv x0 x1 x2 x3 pie3 relr3)
for k in "${order[@]}"; do
  "$S/idle.sh" "idle-$k" "${EXE[$k]}" ${ENV[$k]}
done
for r in 1 2 3; do
  for n in 100 1000 10000; do
    CONC=16 "$S/counted.sh" "sp-r$r" raw spider "$B/spider" sample "$n"
    for k in "${order[@]}"; do
      WRAP_ENV="${ENV[$k]}" CONC=16 "$S/counted.sh" "$k-r$r" raw foxhound "${EXE[$k]}" sample "$n"
    done
  done
done
echo PHASE-A-DONE
