#!/usr/bin/env bash
# mem-research: mimalloc's runtime options against the shipped binary, at
# 1,000 pages and 16 in flight, and idle; 2 trials each.
set -uo pipefail
S=/Users/jb/agent-targets/mem-research/scripts
B=/Users/jb/agent-targets/mem-research/bin
R=/Users/jb/agent-targets/mem-research/runs
cd "$R"
for v in ${!GOODFETCH_*}; do unset "$v"; done
declare -A V=(
  [thp0]="MIMALLOC_ALLOW_THP=0"
  [purge0]="MIMALLOC_PURGE_DELAY=0"
  [thp0purge0]="MIMALLOC_ALLOW_THP=0 MIMALLOC_PURGE_DELAY=0"
  [eager0]="MIMALLOC_ARENA_EAGER_COMMIT=0"
  [tw4]="TOKIO_WORKER_THREADS=4"
  [tw4thp0]="TOKIO_WORKER_THREADS=4 MIMALLOC_ALLOW_THP=0"
)
for k in thp0 purge0 thp0purge0 eager0 tw4 tw4thp0; do
  for r in 1 2; do
    WRAP_ENV="${V[$k]}" CONC=16 "$S/counted.sh" "mi-$k-r$r" raw foxhound "$B/fh-mimalloc" sample 1000
  done
  for n in 100 10000; do
    WRAP_ENV="${V[$k]}" CONC=16 "$S/counted.sh" "mi-$k" raw foxhound "$B/fh-mimalloc" sample "$n"
  done
  tag=idle-mi-$k
  python3 -I -c 'import time; time.sleep(3)' | env ${V[$k]} "$B/fh-mimalloc" attached-engine > /dev/null 2> "$R/$tag.err" &
  p=$!
  python3 -I "$S/sampler.py" "$p" "$R/$tag" fh-mimalloc
  wait "$p"; echo "$tag rc=$?"
done
# The System-allocator variant at 100 pages and with 1 and 64 in flight.
CONC=16 "$S/counted.sh" fs raw foxhound "$B/fh-system" sample 100
for c in 1 64; do CONC=$c "$S/counted.sh" fs raw foxhound "$B/fh-system" sample 1000; done
WRAP_ENV="TOKIO_WORKER_THREADS=4" CONC=16 "$S/counted.sh" fs-tw4 raw foxhound "$B/fh-system" sample 1000
echo ALL-DONE
