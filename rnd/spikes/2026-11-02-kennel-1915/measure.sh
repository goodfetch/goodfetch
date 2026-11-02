#!/bin/sh
# THROWAWAY SPIKE for goodfetch/goodfetch#1915.
# Measures every running kennel-spike process (the tray, and the window if open):
# memory as macOS counts it (phys_footprint), then CPU sampled every 5 s.
#   ./measure.sh [seconds]   default 60
set -eu
seconds=${1:-60}
pids=$(pgrep -x kennel-spike | tr '\n' ' ' || true)
if [ -z "$pids" ]; then
    echo "no kennel-spike process is running: start ./run.sh first" >&2
    exit 1
fi
args=""
for pid in $pids; do
    echo "== pid $pid: $(ps -o command= -p "$pid")"
    # The full table shows what the footprint is made of (graphics memory, heap).
    footprint "$pid" >"target/footprint-$pid.txt" 2>&1 || true
    grep -iE 'phys_footprint|^ *Footprint' "target/footprint-$pid.txt" || echo "   footprint unavailable"
    echo "   full table: target/footprint-$pid.txt"
    args="$args -pid $pid"
done
samples=$((seconds / 5 + 1))
echo "== CPU over ${seconds} s (the first sample of top is discarded)"
# shellcheck disable=SC2086
/usr/bin/top -l "$samples" -s 5 -stats pid,cpu,mem $args \
    | awk -v pids="$pids" '
        BEGIN { n = split(pids, list, /[ \n]+/); for (i = 1; i <= n; i++) watched[list[i]] = 1 }
        ($1 in watched) { seen[$1]++; if (seen[$1] > 1) { sum[$1] += $2; count[$1]++; mem[$1] = $3 } }
        END { for (pid in sum) printf "pid %s: mean CPU %.2f%% over %d samples, last MEM %s\n", pid, sum[pid] / count[pid], count[pid], mem[pid] }'
