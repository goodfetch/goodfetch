#!/usr/bin/env bash
# peak-gap phase 9: the order file built from the exact touch map with mangled
# names (ord5), alone and in the combinations, cold with Spider, fully cached,
# and its wall time against fh-k.
set -uo pipefail
source /Users/jb/agent-targets/peak-gap/scripts/env.sh
S=$PC/scripts
W2="TOKIO_WORKER_THREADS=2,PEAKGAP_MAX_BLOCKING=4" W1="TOKIO_WORKER_THREADS=1,PEAKGAP_MAX_BLOCKING=4"
EVICT=1 "$S/measure-x.sh" f- 3 "100 1000 10000" k:fh-k: ord3:fh-k-ord3: ord5:fh-k-ord5: \
  a2:fh-k-ord3-ovr:$W2 a2o5:fh-k-ord5-ovr:$W2 a1o5:fh-k-ord5-ovr:$W1
EVICT=0 WARM=1 SPIDER=0 "$S/measure-x.sh" fw- 3 "100 1000" k:fh-k: ord5:fh-k-ord5: a2o5:fh-k-ord5-ovr:$W2
mkdir -p "$PC/runs/wall-ord5"
rm -f "$PC"/runs/wall/run-base-*
"$S/measure-w.sh" 12 6 ord5:fh-k-ord5: a2o5:fh-k-ord5-ovr:$W2
mv "$PC"/runs/wall/ord5-* "$PC"/runs/wall/a2o5-* "$PC"/runs/wall/run-* "$PC/runs/wall-ord5/"
echo PHASE9-DONE
