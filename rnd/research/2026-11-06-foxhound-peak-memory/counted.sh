#!/usr/bin/env bash
# mem-research (after /g/gap-http/scripts/counted.sh): one tool, scenario, mode, sizes.
# usage: counted.sh <tag> <scenario> <cell> <exe> <mode> <pages...>; CONC sets concurrency.
set -uo pipefail
tag=$1 scenario=$2 cell=$3 exe=$4 mode=$5
shift 5
R=/Users/jb/agent-targets/mem-research/runs
mkdir -p "$R"
for n in "$@"; do
  out=$R/$tag-$scenario-$n-c${CONC:-16}
  uptime > "$out.load-$mode"
  WRAP_MODE=$mode WRAP_EXE=$exe WRAP_OUT=$out \
    /Users/jb/agent-targets/mem-research/scripts/trial.sh "$scenario" "$cell" /Users/jb/agent-targets/mem-research/scripts/wrap.sh "$n" timed \
    > "$out.json-$mode" 2> "$out.err-$mode"
  echo "$tag $scenario n=$n c=${CONC:-16} $mode rc=$?"
done
