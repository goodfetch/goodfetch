#!/usr/bin/env bash
# peak-cuts: fh-x re-linked with its symbol table (same layout), then phase B.
set -uo pipefail
cd /Users/jb/agent-targets/peak-cuts
scripts/relink.sh link/x-syms bin/fh-x-syms
scripts/measure-b.sh /Users/jb/agent-targets/peak-cuts/bin/fh-x x 30 10
