#!/usr/bin/env bash
# mem-research: every measurement, in one load-guard slot.
set -uo pipefail
B=/Users/jb/agent-targets/mem-research/bin
S=/Users/jb/agent-targets/mem-research/scripts
R=/Users/jb/agent-targets/mem-research/runs
mkdir -p "$R"
cd "$R"
for v in ${!GOODFETCH_*}; do unset "$v"; done
FH=$B/fh-mimalloc FS=$B/fh-system SP=$B/spider

# Per process: the engine started and idle 3 s, before any request.
for exe in "$FH" "$FS"; do
  tag=idle-$(basename "$exe")
  python3 -I -c 'import time; time.sleep(3)' | "$exe" attached-engine > /dev/null 2> "$R/$tag.err" &
  p=$!
  python3 -I "$S/sampler.py" "$p" "$R/$tag" "$(basename "$exe")"
  wait "$p"; echo "$tag rc=$?"
done

# Site size, concurrency 16, shipped allocator.
CONC=16 "$S/counted.sh" fh raw foxhound "$FH" sample 10 100 1000 10000
# Repeats at 1,000.
for r in 2 3; do CONC=16 "$S/counted.sh" fh-r$r raw foxhound "$FH" sample 1000; done
# Pages in flight: concurrency 1, 4, 64 at 1,000 pages.
for c in 1 4 64; do CONC=$c "$S/counted.sh" fh raw foxhound "$FH" sample 1000; done
# mimalloc's own statistics at 1,000.
CONC=16 "$S/counted.sh" fh raw foxhound "$FH" mistats 1000
# The System-allocator variant: RSS, then heaptrack.
CONC=16 "$S/counted.sh" fs raw foxhound "$FS" sample 10 1000 10000
CONC=16 "$S/counted.sh" fs raw foxhound "$FS" heaptrack 10 1000 10000
CONC=64 "$S/counted.sh" fs raw foxhound "$FS" heaptrack 1000
CONC=1 "$S/counted.sh" fs raw foxhound "$FS" heaptrack 1000
# Spider: RSS and heaptrack.
CONC=16 "$S/counted.sh" sp raw spider "$SP" sample 100 1000 10000
CONC=16 "$S/counted.sh" sp raw spider "$SP" heaptrack 1000 10000
echo ALL-DONE
