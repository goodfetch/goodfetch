#!/usr/bin/env sh
# Type-checks the SDK API sketch in all three languages. Needs Node 22+, uv and a Rust toolchain.
set -eu
here=$(cd "$(dirname "$0")" && pwd)

echo "TypeScript (tsc 7.0.2, zod 4.6.7)"
(cd "$here/typescript" && npm install --no-audit --no-fund --silent && npx --yes -p typescript@7.0.2 tsc -p .)

echo "Python (pyright 1.1.414, mypy 2.4.0, pydantic 2.14.1)"
(cd "$here/python" && uv run --no-project --with pydantic==2.14.1 --with pyright==1.1.414 pyright \
  && uv run --no-project --with pydantic==2.14.1 --with mypy==2.4.0 mypy)

echo "Rust (cargo check)"
(cd "$here/rust" && RUSTFLAGS="-D warnings" cargo check --all-targets --quiet)

echo "All three type-check."
