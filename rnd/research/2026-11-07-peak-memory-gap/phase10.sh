#!/usr/bin/env bash
# peak-gap phase 10: the combination (2 workers, blocking cap 4, ord5, C malloc
# to mimalloc) as static-PIE (#2165) against the same as static non-PIE.
set -uo pipefail
source /Users/jb/agent-targets/peak-gap/scripts/env.sh
W2="TOKIO_WORKER_THREADS=2,PEAKGAP_MAX_BLOCKING=4"
EVICT=1 "$PC/scripts/measure-x.sh" n- 3 "100 1000 10000" a2o5:fh-k-ord5-ovr:$W2 a2o5np:fh-k-ord5-ovr-nopie:$W2
echo PHASE10-DONE
