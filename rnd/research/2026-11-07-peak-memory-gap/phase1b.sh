#!/usr/bin/env bash
# peak-gap phase 1b: the Tokio-knobs build (knobs.py; no knob set = today's
# runtime), profile-guided from phase 1's profile, re-linked as #2165's build;
# plus the malloc-shim re-links (C malloc to mimalloc) and BOLT inputs.
set -euo pipefail
source /Users/jb/agent-targets/peak-gap/scripts/env.sh
S=$PC/scripts
cd "$PC"
rm -rf "$CARGO_TARGET_DIR/$MUSL"
KNOBS=1 RUSTFLAGS="-Cprofile-use=$PC/pgo/merged.profdata -Cllvm-args=-no-pgo-warn-mismatch" "$S/build-x2.sh" knobs
"$S/pie-args.sh" link/knobs link/knobs-pie "$S/pic.list"
ord=(--symbol-ordering-file=$PC/pgo/order.txt --no-warn-symbol-ordering)
"$S/relink.sh" link/knobs-pie bin/fh-k -z pack-relative-relocs "${ord[@]}"
"$S/add-obj.sh" link/pgo-pie link/pgo-pie-ovr "$PC/ovr/malloc-shim.o"
"$S/relink.sh" link/pgo-pie-ovr bin/fh-2165-ovr -z pack-relative-relocs "${ord[@]}"
"$S/add-obj.sh" link/knobs-pie link/knobs-pie-ovr "$PC/ovr/malloc-shim.o"
"$S/relink.sh" link/knobs-pie-ovr bin/fh-k-ovr -z pack-relative-relocs "${ord[@]}"
"$S/relink.sh" link/pgo bin/fh-pgo-order-q -q "${ord[@]}"
"$S/relink.sh" link/pgo-pie bin/fh-2165-q -q -z pack-relative-relocs "${ord[@]}"
for b in fh-2165 fh-k fh-2165-ovr fh-k-ovr fh-pgo-order-q fh-2165-q; do
  echo "$b $(stat -c %s bin/$b) $(sha256sum < bin/$b | cut -c1-12) $(bin/$b --version </dev/null)"
done
nm bin/fh-2165-ovr | grep -E ' (T|W) (malloc|free)$'
echo PHASE1B-DONE
