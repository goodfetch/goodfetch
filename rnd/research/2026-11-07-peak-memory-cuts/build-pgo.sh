#!/usr/bin/env bash
# peak-cuts: PGO of the musl executable (the THP-modes variant), trained on
# scenario 1's crawl: instrumented build, training, merge, profile-guided build.
# Rust code only: the C and C++ that Zig compiles (DuckDB, SQLite, mimalloc,
# BoringSSL, zstd, brotli) get no profile.
set -euo pipefail
source /Users/jb/agent-targets/peak-cuts/scripts/env.sh
S=$PC/scripts B=$PC/bin
P=$PC/pgo
TC=$(rustc --print sysroot)
PROFDATA=$TC/lib/rustlib/x86_64-unknown-linux-gnu/bin/llvm-profdata
rm -rf "$P"; mkdir -p "$P/raw"
# Every musl crate recompiles with the new RUSTFLAGS; the earlier objects live on in
# the saved links (link/x), so free their space first.
[[ ${KEEP_TARGET:-0} == 1 ]] || rm -rf "$CARGO_TARGET_DIR/$MUSL"
RUSTFLAGS="-Cprofile-generate=$P/raw" "$S/build-x.sh" pgogen
t0=$(date +%s)
export LLVM_PROFILE_FILE="$P/raw/fh-%p-%m.profraw"
"$S/idle.sh" train-idle "$B/fh-pgogen" "LLVM_PROFILE_FILE=$LLVM_PROFILE_FILE"
for n in 100 1000 1000 1000 10000; do
  WRAP_MODE=plain WRAP_ENV="LLVM_PROFILE_FILE=$LLVM_PROFILE_FILE" WRAP_EXE=$B/fh-pgogen CONC=16 \
    "$S/trial.sh" raw foxhound "$S/wrap.sh" "$n" timed > "$P/train-$n.json" 2> "$P/train-$n.err" || echo "train $n failed"
done
ls -la "$P/raw"
"$PROFDATA" merge -o "$P/merged.profdata" "$P"/raw/*.profraw
rm -rf "$PC/link/pgogen" "$CARGO_TARGET_DIR/$MUSL"  # nothing re-links or reuses the instrumented build
t1=$(date +%s)
echo "pgo training+merge: $((t1-t0)) s" | tee -a "$PC/logs/build.times"
RUSTFLAGS="-Cprofile-use=$P/merged.profdata -Cllvm-args=-no-pgo-warn-mismatch" "$S/build-x.sh" pgo
"$PROFDATA" show --all-functions --counts "$P/merged.profdata" > "$P/functions.txt" || true
echo PGO-DONE
