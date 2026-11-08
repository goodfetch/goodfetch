#!/usr/bin/env bash
# What each Product ships (ADR 0087): cargo-deny's `check bans` once per
# Product, rooted at its executable crate with `--all-features` and
# `--exclude-dev`, against ci/deny/<product>.toml. Dev-dependencies are left
# out because tests take the test-only crates; deny.toml's workspace-wide
# rules cover them.
#
#   ci/deny/products.sh [<workspace Cargo.toml>]
#
# The workspace defaults to the repository's. `DENY_FORMAT=json` passes
# cargo-deny's JSON output through (the proofs read it). Before each
# Product's check it prints `product: <name>`. A Product whose executable
# crate is no package of the workspace fails the check, so a moved or
# renamed crate (xtask/src/products.rs holds each) cannot skip it.
set -euo pipefail

repository=$(cd "$(dirname "$0")/../.." && pwd)
manifest=${1:-$repository/Cargo.toml}

# Each Product and its executable crate (ADRs 0087 and 0090).
products=(collie:goodfetch-collie foxhound:goodfetch-foxhound)

members=$(cargo metadata --format-version 1 --no-deps --manifest-path "$manifest" |
  jq -r '.packages[] | "\(.name)\t\(.manifest_path)"')

failed=0
for entry in "${products[@]}"; do
  product=${entry%%:*}
  crate=${entry#*:}
  echo "product: $product"
  crate_manifest=$(awk -F'\t' -v name="$crate" '$1 == name { print $2 }' <<<"$members")
  if [[ -z "$crate_manifest" ]]; then
    echo "error: $product's executable crate $crate is no package of $manifest; name the crate here as xtask/src/products.rs does" >&2
    failed=1
    continue
  fi
  # -A unused-wrapper: a wrapper for a crate the Product does not link is
  # the rule's shape, not a finding.
  if ! cargo deny --format "${DENY_FORMAT:-human}" --manifest-path "$crate_manifest" \
    --config "$repository/ci/deny/$product.toml" --all-features --exclude-dev \
    check -A unused-wrapper bans; then
    failed=1
  fi
done
exit "$failed"
