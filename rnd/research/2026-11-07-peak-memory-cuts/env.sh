# peak-cuts: shared environment for every build and run (sourced).
export WT=/Users/jb/Dev/goodfetch/.claude/worktrees/confident-edison-3e91c2
export PC=/Users/jb/agent-targets/peak-cuts
export CARGO_TARGET_DIR=/mnt/HC_Volume_107255002/agent-targets/peak-cuts/target  # on the volume: / is near the gates' 30 GB floor
export CARGO_INCREMENTAL=0
export RUSTC_WRAPPER=sccache SCCACHE_CLIENT_SIDE=1
export TMPDIR=/g
export MUSL=x86_64-unknown-linux-musl
export ZIG=$HOME/.local/share/goodfetch/musl-toolchain/zig-0.17.1-cargo-zigbuild-0.24.1/zig/zig
for v in ${!GOODFETCH_*}; do unset "$v"; done
