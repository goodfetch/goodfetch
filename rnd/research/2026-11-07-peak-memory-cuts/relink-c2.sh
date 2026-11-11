#!/usr/bin/env bash
# peak-cuts phase C, second pass: the order-file re-links again, now that
# orderfile.py reads the profile's block counts (the first pass listed nothing).
set -euo pipefail
cd /Users/jb/agent-targets/peak-cuts
python3 -I scripts/orderfile.py pgo/functions.txt pgo/order.txt
ord=(--symbol-ordering-file=/Users/jb/agent-targets/peak-cuts/pgo/order.txt --no-warn-symbol-ordering)
scripts/relink.sh link/pgo bin/fh-pgo-order "${ord[@]}"
scripts/relink.sh link/pgo bin/fh-pgo-both -z keep-text-section-prefix "${ord[@]}"
scripts/relink.sh link/x bin/fh-x-order "${ord[@]}"
scripts/relink.sh link/pgo-pie bin/fh-pgo-order-relr -z pack-relative-relocs "${ord[@]}"
for b in fh-pgo fh-pgo-order fh-pgo-both fh-x fh-x-order fh-pgo-order-relr; do
  echo "$b $(stat -c %s bin/$b) $(sha256sum < bin/$b | cut -c1-12) $(bin/$b --version </dev/null)"
done
