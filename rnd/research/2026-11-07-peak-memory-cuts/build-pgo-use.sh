#!/usr/bin/env bash
# peak-cuts: the profile-guided build alone, from pgo/merged.profdata (the second
# half of build-pgo.sh), from an empty musl target.
set -euo pipefail
source /Users/jb/agent-targets/peak-cuts/scripts/env.sh
S=$PC/scripts P=$PC/pgo
TC=$(rustc --print sysroot)
PROFDATA=$TC/lib/rustlib/x86_64-unknown-linux-gnu/bin/llvm-profdata
rm -rf "$CARGO_TARGET_DIR/$MUSL"
RUSTFLAGS="-Cprofile-use=$P/merged.profdata -Cllvm-args=-no-pgo-warn-mismatch" "$S/build-x.sh" pgo
"$PROFDATA" show --all-functions --counts "$P/merged.profdata" > "$P/functions.txt"
echo PGO-DONE
