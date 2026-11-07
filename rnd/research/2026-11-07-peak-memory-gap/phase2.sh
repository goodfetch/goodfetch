#!/usr/bin/env bash
# peak-gap phase 2: the peaks (measure-g.sh), then one attribution trial per
# configuration of interest at 1,000 pages (sampler2.py: threads' stacks).
set -uo pipefail
source /Users/jb/agent-targets/peak-gap/scripts/env.sh
S=$PC/scripts B=$PC/bin
"$S/measure-g.sh"
export EVICT=1
for c in "k:fh-k:" "w4:fh-k:TOKIO_WORKER_THREADS=4" "ovr:fh-k-ovr:" "mb4:fh-k:PEAKGAP_MAX_BLOCKING=4"; do
  tag=${c%%:*} rest=${c#*:}; exe=${rest%%:*} e=${rest#*:}
  WRAP_ENV="PEAKCUT_THP=3${e:+ ${e//,/ }}" CONC=16 "$S/counted.sh" "attr-$tag-r1" raw foxhound "$B/$exe" attr 1000 10000
done
CONC=16 "$S/counted.sh" "attr-sp-r1" raw spider "$B/spider" attr 1000
echo PHASE2-DONE
