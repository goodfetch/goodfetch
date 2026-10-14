#!/usr/bin/env sh
# Type-checks the SDK API sketch in all three languages. Needs Node 22+, uv and a Rust toolchain.
set -eu
here=$(cd "$(dirname "$0")" && pwd)

echo "TypeScript (tsc 7.0.2)"
(cd "$here/typescript" && npx --yes -p typescript@7.0.2 tsc -p .)

echo "Python (pyright 1.1.414, mypy 2.4.0)"
(cd "$here/python" && uvx pyright@1.1.414 && uvx mypy@2.4.0)

echo "Rust (cargo check)"
(cd "$here/rust" && RUSTFLAGS="-D warnings" cargo check --all-targets --quiet)

echo "All three type-check."
