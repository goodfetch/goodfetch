#!/usr/bin/env bash
# Runs the local release rehearsal against the stand-ins of #140 and ADR
# 0069's installed-package harnesses for both Products (issue 141):
# - npm packages (with platform packages and no install scripts) installed
#   from a local Verdaccio through each package manager's release-age gate;
# - `py3-none` platform wheels installed into fresh environments on the host
#   (glibc on Linux) and Alpine (musl container on Linux);
# - `cargo package` output built in a temporary consumer project.
#
# Usage: ci/release/rehearse.sh [--packaged <dir>] [--binaries <dir>] [--version <semver>]
set -euo pipefail

root=$(cd "$(dirname "$0")/../.." && pwd)
cd "$root"

packaged=""
binaries=""
version=""
while [[ $# -gt 0 ]]; do
  case "$1" in
    --packaged) packaged=$2; shift 2 ;;
    --binaries) binaries=$2; shift 2 ;;
    --version) version=$2; shift 2 ;;
    *)
      echo "usage: ci/release/rehearse.sh [--packaged <dir>] [--binaries <dir>] [--version <semver>]" >&2
      exit 2
      ;;
  esac
done
if [[ -z "$version" ]]; then
  version=$(awk -F'"' '/^version = / {print $2; exit}' Cargo.toml)
fi

export GOODFETCH_REHEARSE_PACKAGED="$packaged"
export GOODFETCH_REHEARSE_VERSION="$version"
export GOODFETCH_TEST_SUITE="release"

# 0. Published Operation catalog, debug symbols, Ceilings, Helm chart OCI push,
#    and packaged wheel/npm offline install rehearsal (issues 141, 1662, 202, 203).
if [[ -n "$packaged" ]]; then
  key_dir="$packaged/rehearsal-keys"
  for asset in "$packaged/assets/goodfetch-catalog.json" "$packaged"/assets/*-debug.tar.xz; do
    [[ -f "$asset" ]] || continue
    cargo run --quiet --bin xtask -- release sign-rehearsal \
      --file "$asset" \
      --key-dir "$key_dir"
    cargo run --quiet --bin xtask -- release verify-rehearsal \
      --file "$asset" \
      --signature "${asset}.sig" \
      --attestation "${asset}.sigstore.jsonl" \
      --public-key "$key_dir/rehearsal.pub"
  done

  # Enforce both Products' release-profile Size budget ceilings before any publish
  # step: a broken ceiling of either Product stops the rehearsal for both (ADR 0087, 0094).
  manifest="$packaged/release-size-files.toml"
  size_args=(--packaged "$packaged" --out "$manifest")
  if [[ -n "$binaries" ]]; then
    size_args+=(--binaries "$binaries")
  fi
  cargo run --quiet --bin xtask -- release size-manifest "${size_args[@]}"
  cargo run --quiet --bin xtask -- size ceilings --files "$manifest" >"$packaged/release-size-table.md"

  # Rehearse Helm chart OCI push and install against local `registry:2` when configured (issue 202).
  if [[ -n "${GOODFETCH_REHEARSE_OCI_REGISTRY:-}" && -d "$packaged/oci/charts" ]]; then
    chart_scratch=$(mktemp -d)
    for product in collie foxhound; do
      chart_dir="$packaged/oci/charts/$product"
      [[ -d "$chart_dir" ]] || continue
      cargo run --quiet --bin xtask -- release oci-push \
        --layout "$chart_dir" \
        --registry "$GOODFETCH_REHEARSE_OCI_REGISTRY" \
        --repository "goodfetch/charts/$product" \
        --tag "$version"
      if command -v helm >/dev/null 2>&1; then
        reg_host="${GOODFETCH_REHEARSE_OCI_REGISTRY#http://}"
        reg_host="${reg_host#https://}"
        helm pull "oci://${reg_host}/goodfetch/charts/${product}" \
          --version "$version" --plain-http --destination "$chart_scratch"
        helm template "goodfetch-${product}" "$chart_scratch/goodfetch-${product}-${version}.tgz" >/dev/null
      fi
    done
    rm -rf "$chart_scratch"
  fi

  # Rehearse actual packaged wheels and npm `.tgz` tarballs when host-matching files exist.
  if ls "$packaged"/npm/*.tgz >/dev/null 2>&1 && ls "$packaged"/wheels/*.whl >/dev/null 2>&1; then
    host_target=$(rustc -vV | awk '/^host:/ {print $2}')
    if [[ -f "$packaged/assets/goodfetch-collie-${host_target}.tar.xz" ]]; then
      ci/release/verify-bytes.sh "$packaged" "$host_target" "$version"
    fi
  fi
fi

# 1. TypeScript installed-package rehearsal against local Verdaccio (ADR 0069).
pnpm exec vitest run --project install test-support/typescript/tests/install.install.test.ts

# 2. Python wheel rehearsal on host (and Alpine on Linux) with uv and pip.
uv run --offline pytest test-support/python/tests/integration/test_install.py

# 3. Rust `cargo package` temporary consumer rehearsal.
cargo nextest run -p goodfetch-test-support --test it -E 'test(/cargo_consumer::/)'
