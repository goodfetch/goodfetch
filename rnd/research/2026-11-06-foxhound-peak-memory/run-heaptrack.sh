#!/usr/bin/env bash
# mem-research: the heaptrack runs (System-allocator Foxhound, and Spider).
set -uo pipefail
S=/Users/jb/agent-targets/mem-research/scripts
B=/Users/jb/agent-targets/mem-research/bin
cd /Users/jb/agent-targets/mem-research/runs
for v in ${!GOODFETCH_*}; do unset "$v"; done
rm -f fs-*heaptrack* sp-*heaptrack*
CONC=16 "$S/counted.sh" fs raw foxhound "$B/fh-system" heaptrack 10 100 1000 10000
CONC=64 "$S/counted.sh" fs raw foxhound "$B/fh-system" heaptrack 1000
CONC=1 "$S/counted.sh" fs raw foxhound "$B/fh-system" heaptrack 1000
CONC=16 "$S/counted.sh" sp raw spider "$B/spider" heaptrack 1000 10000
echo ALL-DONE
