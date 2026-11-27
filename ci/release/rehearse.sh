#!/usr/bin/env bash
# Runs the local release rehearsal against the stand-ins of #140 and ADR
# 0069's installed-package harnesses for both Products (issue 141):
# - npm packages (with platform packages and no install scripts) installed
#   from a local Verdaccio through each package manager's release-age gate;
# - `py3-none` platform wheels installed into fresh environments on the host
#   (glibc on Linux) and Alpine (musl container on Linux);
# - `cargo package` output built in a temporary consumer project.
#
# Usage: ci/release/rehearse.sh [--packaged <dir>] [--version <semver>]
set -euo pipefail

root=$(cd "$(dirname "$0")/../.." && pwd)
cd "$root"

packaged=""
version="0.0.0"
while [[ $# -gt 0 ]]; do
  case "$1" in
    --packaged) packaged=$2; shift 2 ;;
    --version) version=$2; shift 2 ;;
    *)
      echo "usage: ci/release/rehearse.sh [--packaged <dir>] [--version <semver>]" >&2
      exit 2
      ;;
  esac
done

export GOODFETCH_REHEARSE_PACKAGED="$packaged"
export GOODFETCH_REHEARSE_VERSION="$version"

# 0. Published Operation catalog signature and attestation rehearsal (issue 1662).
if [[ -n "$packaged" && -f "$packaged/assets/goodfetch-catalog.json" ]]; then
  key_dir="$packaged/rehearsal-keys"
  cargo run --quiet --bin xtask -- release sign-rehearsal \
    --file "$packaged/assets/goodfetch-catalog.json" \
    --key-dir "$key_dir"
  cargo run --quiet --bin xtask -- release verify-rehearsal \
    --file "$packaged/assets/goodfetch-catalog.json" \
    --signature "$packaged/assets/goodfetch-catalog.json.sig" \
    --attestation "$packaged/assets/goodfetch-catalog.json.sigstore.jsonl" \
    --public-key "$key_dir/rehearsal.pub"
fi

# 1. TypeScript installed-package rehearsal against local Verdaccio (ADR 0069).
pnpm exec vitest run --project install test-support/typescript/tests/install.install.test.ts

# 2. Python wheel rehearsal on host (and Alpine on Linux) with uv and pip.
uv run --offline pytest test-support/python/tests/integration/test_install.py

# 3. Rust `cargo package` temporary consumer rehearsal.
cargo nextest run -p goodfetch-test-support --test it -E 'test(/cargo_consumer::/)'
