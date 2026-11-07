#!/usr/bin/env bash
# peak-gap phase 7: the cuts together. Settings: 2 or 1 Tokio workers, a
# blocking-pool cap of 4; build: the extended order file (ord3) and the C
# malloc family to mimalloc (ovr). Peaks with Spider, then wall against fh-k.
set -uo pipefail
source /Users/jb/agent-targets/peak-gap/scripts/env.sh
S=$PC/scripts
W2="TOKIO_WORKER_THREADS=2,PEAKGAP_MAX_BLOCKING=4" W1="TOKIO_WORKER_THREADS=1,PEAKGAP_MAX_BLOCKING=4"
EVICT=1 "$S/measure-x.sh" c- 3 "100 1000 10000" k:fh-k: s2:fh-k:$W2 s1:fh-k:$W1 \
  a2:fh-k-ord3-ovr:$W2 a1:fh-k-ord3-ovr:$W1 o2:fh-k-ord3:$W2
mkdir -p "$PC/runs/wall-combo"
rm -f "$PC"/runs/wall/run-base-*
"$S/measure-w.sh" 12 6 s2:fh-k:$W2 a2:fh-k-ord3-ovr:$W2 a1:fh-k-ord3-ovr:$W1 o2:fh-k-ord3:$W2
mv "$PC"/runs/wall/s2-* "$PC"/runs/wall/a2-* "$PC"/runs/wall/a1-* "$PC"/runs/wall/o2-* "$PC"/runs/wall/run-* "$PC/runs/wall-combo/"
echo PHASE7-DONE
