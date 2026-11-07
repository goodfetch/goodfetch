#!/usr/bin/env bash
# peak-cuts: xtask, the Proving ground and Spider (host builds, as #2150), and
# Foxhound's shipped static musl executable (ci/musl/cargo, ADR 0024's note).
set -euo pipefail
source /Users/jb/agent-targets/peak-cuts/scripts/env.sh
cd "$WT"
t0=$(date +%s)
cargo build --locked --bin xtask
"$CARGO_TARGET_DIR/debug/xtask" console build
cargo build --locked --profile release --bin goodfetch-proving-ground --features executable
cargo build --locked --manifest-path xtask/compare-spider/Cargo.toml --profile release --target-dir /mnt/HC_Volume_107255002/agent-targets/peak-cuts/spider
cp "$CARGO_TARGET_DIR/debug/xtask" "$PC/bin/xtask"
cp "$CARGO_TARGET_DIR/release/goodfetch-proving-ground" "$PC/bin/proving-ground"
cp /mnt/HC_Volume_107255002/agent-targets/peak-cuts/spider/release/goodfetch-compare-spider "$PC/bin/spider"
t1=$(date +%s)
ci/musl/cargo build --locked --profile release --target "$MUSL" --bin goodfetch-foxhound --features executable
t2=$(date +%s)
cp "$CARGO_TARGET_DIR/$MUSL/release/goodfetch-foxhound" "$PC/bin/fh-base"
echo "host tools $((t1-t0)) s; musl foxhound $((t2-t1)) s" | tee "$PC/logs/build-base.times"
ls -la "$PC/bin"
