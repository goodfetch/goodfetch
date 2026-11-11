#!/usr/bin/env bash
# peak-gap phase 1: host tools, PGO of the musl executable, and #2165's build:
# static-PIE + DT_RELR + PGO + order file (THP off at run time: PEAKCUT_THP=3).
set -euo pipefail
source /Users/jb/agent-targets/peak-gap/scripts/env.sh
S=$PC/scripts
"$S/build-base.sh"
"$S/build-pgo.sh"
cd "$PC"
python3 -I scripts/orderfile.py pgo/functions.txt pgo/order.txt
"$S/pie-args.sh" link/pgo link/pgo-pie "$S/pic.list"
ord=(--symbol-ordering-file=$PC/pgo/order.txt --no-warn-symbol-ordering)
"$S/relink.sh" link/pgo bin/fh-pgo-order "${ord[@]}"
"$S/relink.sh" link/pgo-pie bin/fh-2165 -z pack-relative-relocs "${ord[@]}"
for b in fh-pgo fh-pgo-order fh-2165; do
  echo "$b $(stat -c %s bin/$b) $(sha256sum < bin/$b | cut -c1-12) $(bin/$b --version </dev/null)"
done
echo PHASE1-DONE
