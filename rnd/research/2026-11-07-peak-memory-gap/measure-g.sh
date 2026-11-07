#!/usr/bin/env bash
# peak-gap phase 2: peaks of the runtime cuts on #2165's build, Spider in the
# same rounds; every executable evicted from the page cache before each trial;
# 3 rounds, configurations interleaved within each round. THP off in all
# (PEAKCUT_THP=3, the .init_array.00100 constructor).
# cfg = tag:exe:VAR=value,VAR=value
set -uo pipefail
source /Users/jb/agent-targets/peak-gap/scripts/env.sh
export EVICT=1
S=$PC/scripts B=$PC/bin
cfgs=(
  base:fh-2165:
  k:fh-k:
  w1:fh-k:TOKIO_WORKER_THREADS=1
  w2:fh-k:TOKIO_WORKER_THREADS=2
  w4:fh-k:TOKIO_WORKER_THREADS=4
  w8:fh-k:TOKIO_WORKER_THREADS=8
  mb4:fh-k:PEAKGAP_MAX_BLOCKING=4
  ka1s:fh-k:PEAKGAP_KEEP_ALIVE_MS=1000
  ovr:fh-k-ovr:
  fr0:fh-k:MIMALLOC_PAGE_FULL_RETAIN=0
  rec1:fh-k:MIMALLOC_PAGE_RECLAIM_ON_FREE=1
  gc1k:fh-k:MIMALLOC_GENERIC_COLLECT=1000
  ar32:fh-k:MIMALLOC_ARENA_RESERVE=32768
  cand16:fh-k:MIMALLOC_PAGE_MAX_CANDIDATES=16
  nonpie:fh-pgo-order:
)
env_of() { local e=${1#*:}; e=${e#*:}; echo "PEAKCUT_THP=3${e:+ ${e//,/ }}"; }
for r in 1 2 3; do
  for n in 100 1000 10000; do
    CONC=16 "$S/counted.sh" "sp-r$r" raw spider "$B/spider" sample "$n"
    for c in "${cfgs[@]}"; do
      tag=${c%%:*} rest=${c#*:}; exe=${rest%%:*}
      WRAP_ENV="$(env_of "$c")" CONC=16 "$S/counted.sh" "$tag-r$r" raw foxhound "$B/$exe" sample "$n"
    done
  done
done
echo PHASE-G-DONE
