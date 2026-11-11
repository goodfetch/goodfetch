#!/usr/bin/env bash
# peak-cuts: does the executable's resident size depend on the page cache's
# state? Each binary idle: as found, then evicted (cold), then again (warm).
set -uo pipefail
source /Users/jb/agent-targets/peak-cuts/scripts/env.sh
S=$PC/scripts B=$PC/bin
for b in fh-base fh-x; do
  "$S/idle.sh" "cache-$b-asfound" "$B/$b"
  python3 -I "$S/evict.py" "$B/$b"
  "$S/idle.sh" "cache-$b-cold" "$B/$b"
  "$S/idle.sh" "cache-$b-warm" "$B/$b"
done
cp "$B/fh-base" "$B/fh-base-copy"
"$S/idle.sh" "cache-fh-base-copy-written" "$B/fh-base-copy"
