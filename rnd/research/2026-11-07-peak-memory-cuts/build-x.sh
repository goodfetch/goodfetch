#!/usr/bin/env bash
# peak-cuts: the measurement-only THP-modes variant (thp-modes.py) of the musl
# executable, linked through link-save.sh so relink.sh can re-link it.
# usage: build-x.sh <name>   (RUSTFLAGS from the caller apply, e.g. PGO)
set -euo pipefail
source /Users/jb/agent-targets/peak-cuts/scripts/env.sh
name=$1
cd "$WT"
source ci/musl/toolchain.sh
export CARGO_ZIGBUILD_ZIG_PATH="$musl_toolchain/zig/zig" CARGO=cargo
alloc=crates/goodfetch-collie/src/executable/allocator.rs
trap 'git -C "$WT" checkout -- "$alloc"' EXIT
python3 -I "$PC/scripts/thp-modes.py" "$WT"
export LINK_SAVE=$PC/link/$name
rm -rf "$LINK_SAVE"
t0=$(date +%s)
"$musl_toolchain/bin/cargo-zigbuild" rustc --locked --profile release --target "$MUSL" \
  -p goodfetch-foxhound --bin goodfetch-foxhound --features executable \
  -- -C linker="$PC/scripts/link-save.sh"
t1=$(date +%s)
cp "$CARGO_TARGET_DIR/$MUSL/release/goodfetch-foxhound" "$PC/bin/fh-$name"
echo "$name: $((t1-t0)) s (RUSTFLAGS=${RUSTFLAGS:-})" | tee -a "$PC/logs/build.times"
git -C "$WT" checkout -- "$alloc"
trap - EXIT
git -C "$WT" status --short
