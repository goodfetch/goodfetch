#!/bin/zsh
# PROTOTYPE, throwaway. Builds everything, runs the scenarios, prints a summary after each.
#   ./run.sh                      build + all scenarios (feel s1 s2a s2b s2c s3 s4 s5)
#   ./run.sh --no-build s1 s4     selected scenarios only
#   ./run.sh --build-feel         clean build timings and sizes (slow: three clean builds)
# Env: BLOCKS (paired blocks, default 5), BLOCKS_QUAL (s2b/s2c, default 3).
# Raw rows: results/raw/<scenario>.jsonl; summaries: results/summary_<scenario>.md.
set -u
cd "${0:A:h}"
build=1
scens=()
for a in "$@"; do
  case $a in
    --no-build) build=0 ;;
    --build-feel) scens+=(build-feel) ;;
    *) scens+=($a) ;;
  esac
done
(( ${#scens} )) || scens=(feel s1 s2a s2b s2c s3 s4 s5)
if (( build )); then ./build.sh || exit 1; fi
mkdir -p results/raw
for s in $scens; do
  if [[ $s == build-feel ]]; then
    ./feel.sh | tee results/summary_build_feel.md
    continue
  fi
  case $s in
    s3|feel) b=1 ;;
    s2b|s2c) b=${BLOCKS_QUAL:-3} ;;
    *) b=${BLOCKS:-5} ;;
  esac
  python3 scenarios/orchestrate.py $s --blocks $b
  echo
  python3 scenarios/analyze.py $s | tee results/summary_$s.md
  echo
done
