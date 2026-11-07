# peak-cuts: shared environment for every build and run (sourced).
export WT=/Users/jb/agent-targets/peak-gap/src
export PC=/Users/jb/agent-targets/peak-gap
export CARGO_TARGET_DIR=/Users/jb/agent-targets/peak-gap/target  # on the volume: / is near the gates' 30 GB floor
export CARGO_INCREMENTAL=0
export RUSTC_WRAPPER=sccache SCCACHE_CLIENT_SIDE=1
export TMPDIR=/g
export MUSL=x86_64-unknown-linux-musl
export ZIG=$HOME/.local/share/goodfetch/musl-toolchain/zig-0.17.1-cargo-zigbuild-0.24.1/zig/zig
for v in ${!GOODFETCH_*}; do unset "$v"; done
