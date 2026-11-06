#!/usr/bin/env bash
# Measures every running Kennel process on macOS, for the README's
# Performance targets (ADR 0141), as #1915's throwaway build did: its memory
# as macOS counts it (`phys_footprint`), then its CPU sampled every 5 s,
# the first sample of `top` discarded. Start the Kennel with `just kennel`
# first. No gate runs it.
#
#   kennel/scripts/measure.sh [seconds]   default 60
set -euo pipefail

root=$(cd "$(dirname "$0")/../.." && pwd)
seconds=${1:-60}
pids=$(pgrep -x goodfetch-kennel | tr '\n' ' ' || true)
if [[ -z "$pids" ]]; then
  echo "no goodfetch-kennel process is running: start it with \`just kennel\` first" >&2
  exit 1
fi
mkdir -p "$root/kennel/target"
args=()
for pid in $pids; do
  echo "== pid $pid: $(ps -o command= -p "$pid")"
  table="$root/kennel/target/footprint-$pid.txt"
  # The full table shows what the footprint is made of (graphics memory, heap).
  footprint "$pid" >"$table" 2>&1 || true
  grep -iE 'phys_footprint|^ *Footprint' "$table" || echo "   footprint unavailable"
  echo "   full table: $table"
  args+=(-pid "$pid")
done
samples=$((seconds / 5 + 1))
echo "== CPU over ${seconds} s (the first sample of top is discarded)"
/usr/bin/top -l "$samples" -s 5 -stats pid,cpu,mem "${args[@]}" |
  awk -v pids="$pids" '
    BEGIN { n = split(pids, list, /[ \n]+/); for (i = 1; i <= n; i++) watched[list[i]] = 1 }
    ($1 in watched) { seen[$1]++; if (seen[$1] > 1) { sum[$1] += $2; count[$1]++; mem[$1] = $3 } }
    END { for (pid in sum) printf "pid %s: mean CPU %.2f%% over %d samples, last MEM %s\n", pid, sum[pid] / count[pid], count[pid], mem[pid] }'
