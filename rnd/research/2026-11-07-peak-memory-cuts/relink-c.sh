#!/usr/bin/env bash
# peak-cuts phase C links: the order file from the PGO profile, then re-links
# of the PGO build (link/pgo) and of the plain one (link/x).
set -euo pipefail
cd /Users/jb/agent-targets/peak-cuts
python3 -I scripts/orderfile.py pgo/functions.txt pgo/order.txt
ord=(--symbol-ordering-file=/Users/jb/agent-targets/peak-cuts/pgo/order.txt --no-warn-symbol-ordering)
t0=$(date +%s)
scripts/relink.sh link/pgo bin/fh-pgo-prefix -z keep-text-section-prefix
t1=$(date +%s); echo "one re-link: $((t1-t0)) s" | tee -a logs/build.times
scripts/relink.sh link/pgo bin/fh-pgo-order "${ord[@]}"
scripts/relink.sh link/pgo bin/fh-pgo-both -z keep-text-section-prefix "${ord[@]}"
scripts/relink.sh link/x bin/fh-x-order "${ord[@]}"
rm -rf link/pgo-pie
scripts/pie-args.sh link/pgo link/pgo-pie scripts/pic.list > /dev/null
scripts/relink.sh link/pgo-pie bin/fh-pgo-both-relr -z pack-relative-relocs -z keep-text-section-prefix "${ord[@]}"
for b in fh-pgo fh-pgo-prefix fh-pgo-order fh-pgo-both fh-x-order fh-pgo-both-relr; do
  echo "$b $(stat -c %s bin/$b) $(bin/$b --version </dev/null)"
done
