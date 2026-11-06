#!/usr/bin/env bash
# mem-research: builds Foxhound (mimalloc, as shipped, and a measurement-only
# System-allocator variant), the Proving ground, xtask and Spider's harness.
set -euo pipefail
wt=/Users/jb/Dev/goodfetch/.claude/worktrees/agent-a7b3f8a331ad3dd47
T=/Users/jb/agent-targets/mem-research
export CARGO_TARGET_DIR=$T/target
mkdir -p "$T/bin"
cd "$wt"
cargo build --locked --bin xtask
"$CARGO_TARGET_DIR/debug/xtask" console build
bins=(--bin goodfetch-foxhound --bin goodfetch-proving-ground --features executable)
cargo build --locked --profile release "${bins[@]}"
cp "$CARGO_TARGET_DIR/release/goodfetch-foxhound" "$T/bin/fh-mimalloc"
cp "$CARGO_TARGET_DIR/release/goodfetch-proving-ground" "$T/bin/proving-ground"
cp "$CARGO_TARGET_DIR/debug/xtask" "$T/bin/xtask"
# Measurement-only variant: the same binary with the System allocator under the
# same Counting wrapper, so heaptrack sees every allocation. Reverted after.
main=crates/foxhound/goodfetch-foxhound/src/main.rs
trap 'git -C "$wt" checkout -- "$main"' EXIT
sed -i 's/Counting<mimalloc::MiMalloc> = Counting::new(mimalloc::MiMalloc)/Counting<std::alloc::System> = Counting::new(std::alloc::System)/' "$main"
grep -q 'Counting::new(std::alloc::System)' "$main"
cargo build --locked --profile release "${bins[@]}"
cp "$CARGO_TARGET_DIR/release/goodfetch-foxhound" "$T/bin/fh-system"
git -C "$wt" checkout -- "$main"
trap - EXIT
cargo build --locked --manifest-path xtask/compare-spider/Cargo.toml --profile release --target-dir "$T/spider"
cp "$T/spider/release/goodfetch-compare-spider" "$T/bin/spider"
ls -la "$T/bin"
