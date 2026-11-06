#!/usr/bin/env bash
# mem-research: summary and peak owners of one heaptrack file.
# usage: heap.sh <run prefix>   (e.g. fs-raw-1000-c16)
set -euo pipefail
cd /Users/jb/agent-targets/mem-research/runs
f=$1
heaptrack_print -f "$f.heaptrack.zst" --print-peaks 0 --print-allocators 0 --print-temporary 0 --print-leaks 0 \
  --print-flamegraph "$f.peak.folded" --flamegraph-cost-type peak 2>/dev/null \
  | grep -E 'total runtime|calls to allocation|peak heap|peak RSS|leaked' > "$f.heap-summary.txt"
python3 -I /Users/jb/agent-targets/mem-research/scripts/owners.py "$f.peak.folded" 30 >> "$f.heap-summary.txt"
cat "$f.heap-summary.txt"
