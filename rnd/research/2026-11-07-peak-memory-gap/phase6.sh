#!/usr/bin/env bash
# peak-gap phase 6: the order file extended with every touched function and
# named data symbol (ord2) and with the touched anonymous Rust constants (ord3),
# against fh-k: peaks cold (protocol) and with the whole file cached, then wall.
set -uo pipefail
source /Users/jb/agent-targets/peak-gap/scripts/env.sh
S=$PC/scripts
EVICT=1 "$S/measure-x.sh" o- 3 "100 1000 10000" k:fh-k: ord2:fh-k-ord2: ord3:fh-k-ord3:
EVICT=0 WARM=1 SPIDER=0 "$S/measure-x.sh" ow- 3 "100 1000" k:fh-k: ord3:fh-k-ord3:
mkdir -p "$PC/runs/wall-ord"
rm -f "$PC"/runs/wall/run-base-*
"$S/measure-w.sh" 12 6 ord3:fh-k-ord3:
mv "$PC"/runs/wall/ord3-* "$PC"/runs/wall/run-* "$PC/runs/wall-ord/"
echo PHASE6-DONE
