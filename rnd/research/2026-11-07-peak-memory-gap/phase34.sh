#!/usr/bin/env bash
# peak-gap phases 3 and 4: wall and CPU time of the runtime cuts, then BOLT
# (perf needs kernel.perf_event_paranoid <= 2 for a user-space profile; this
# server has 4: lowered to 2 for the profile only, then restored).
set -uo pipefail
source /Users/jb/agent-targets/peak-gap/scripts/env.sh
"$PC/scripts/measure-w.sh" 12 6 w1:fh-k:TOKIO_WORKER_THREADS=1 w2:fh-k:TOKIO_WORKER_THREADS=2 \
  w4:fh-k:TOKIO_WORKER_THREADS=4 w8:fh-k:TOKIO_WORKER_THREADS=8 ovr:fh-k-ovr: mb4:fh-k:PEAKGAP_MAX_BLOCKING=4
old=$(cat /proc/sys/kernel/perf_event_paranoid)
sudo -n sysctl -q kernel.perf_event_paranoid=2
trap 'sudo -n sysctl -q kernel.perf_event_paranoid='"$old" EXIT
"$PC/scripts/phase4-bolt.sh"
echo PHASE34-DONE
