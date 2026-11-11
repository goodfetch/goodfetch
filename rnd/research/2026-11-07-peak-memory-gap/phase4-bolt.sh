#!/usr/bin/env bash
# peak-gap phase 4: BOLT on #2165's build (fh-2165-q: the same link with
# --emit-relocs and its symbol table). perf profile without LBR (cycles:u) of
# the PGO training workload, perf2bolt -nl, then llvm-bolt with Rust's own
# BOLT flags (src/tools/opt-dist/src/bolt.rs) minus --update-debug-sections;
# output stripped like the shipped executable.
set -euo pipefail
source /Users/jb/agent-targets/peak-gap/scripts/env.sh
S=$PC/scripts B=$PC/bin P=$PC/bolt
L=/usr/lib/llvm-21/bin
rm -rf "$P"; mkdir -p "$P"
for b in fh-2165-q fh-pgo-order-q; do
  for n in 1000 1000 1000 10000; do
    i=$((${i:-0} + 1))
    WRAP_MODE=perf WRAP_OUT=$P/$b-$i WRAP_ENV="PEAKCUT_THP=3" WRAP_EXE=$B/$b CONC=16 \
      "$S/trial.sh" raw foxhound "$S/wrap.sh" "$n" timed > "$P/$b-$i.json" 2> "$P/$b-$i.err" || echo "perf $b $n failed"
  done
  for f in "$P/$b"-*.perf.data; do "$L/perf2bolt" -nl -p "$f" -o "${f%.perf.data}.fdata" "$B/$b" > "${f%.perf.data}.p2b.log" 2>&1; done
  "$L/merge-fdata" "$P/$b"-*.fdata > "$P/$b.fdata"
  t0=$(date +%s)
  "$L/llvm-bolt" "$B/$b" -o "$P/$b.bolt" -data "$P/$b.fdata" \
    -reorder-blocks=ext-tsp -reorder-functions=cdsort -split-functions -split-strategy=cdsplit \
    -split-all-cold -jump-tables=move -icf=all -dyno-stats > "$P/$b.bolt.log" 2>&1 || { echo "bolt $b failed"; tail -5 "$P/$b.bolt.log"; continue; }
  echo "llvm-bolt $b: $(( $(date +%s) - t0 )) s" | tee -a "$PC/logs/build.times"
  strip -s -o "$B/${b%-q}-bolt" "$P/$b.bolt"
  rm -f "$P/$b.bolt" "$P/$b"-*.perf.data
done
ls -la "$B"/*bolt*
for b in "$B"/*-bolt; do echo "$(basename "$b") $("$b" --version </dev/null)"; done
echo PHASE4-DONE
