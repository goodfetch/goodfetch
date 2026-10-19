#!/usr/bin/env bash
# Makes the job's later Cargo builds for TARGET link with mold, which the
# host-tools action installed: Linux test jobs only (spec #325). Its
# `-fuse-ld=mold` also replaces rust-lld, Rust's own default on x86-64 Linux.
# Local builds keep the default linker.
set -euo pipefail

target=${TARGET:?the step sets TARGET}
variable="CARGO_TARGET_$(echo "$target" | tr 'a-z-' 'A-Z_')_RUSTFLAGS"
echo "$variable=-Clink-arg=-fuse-ld=mold" >>"${GITHUB_ENV:?the runner sets GITHUB_ENV}"
echo "$target links with $(mold --version)."
