#!/usr/bin/env bash
# The MSRV build (ADR 0030; STYLE.md, Lints, formatting and toolchain): the
# workspace, with every Cargo feature, built by the toolchain its
# `rust-version` names, the latest stable minus two. Run on every pull
# request; the cache-warming job builds it too.
#
#   ci/msrv.sh [<workspace Cargo.toml>]
#
# The CI image installs that toolchain (ci/image/Containerfile); raise both
# together, only in a minor (ADR 0030).
set -euo pipefail

repository=$(cd "$(dirname "$0")/.." && pwd)
manifest=${1:-$repository/Cargo.toml}

versions=$(cargo metadata --format-version 1 --no-deps --manifest-path "$manifest" |
  jq -r '[.packages[].rust_version] | unique | .[]')
if [[ $(wc -w <<<"$versions") -ne 1 ]]; then
  echo "error: every crate takes the workspace's one rust-version, not: ${versions:-none}" >&2
  exit 2
fi

echo "MSRV build with Rust $versions"
CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$repository/target}/msrv" RUSTUP_AUTO_INSTALL=0 \
  cargo "+$versions" build --manifest-path "$manifest" --workspace --all-features --locked
