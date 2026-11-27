#!/usr/bin/env bash
# Smoke install of each Product from each published nightly channel (GitHub
# pre-release archive, npm `nightly` package, `.devN` wheel, and GHCR `nightly`
# OCI image on Linux) in fresh environments under the outbound-traffic deny
# (ADR 0043, ADR 0069, ADR 0094; issue 143).
#
# Channels are fetched into `<staged-dir>` BEFORE `ci/egress/egress.sh deny`
# starts; this script runs under `ci/egress/under-deny.sh` and starts each
# Product's installed executable offline.
#
# Usage: ci/release/smoke-channel.sh <staged-dir> <version>
set -euo pipefail

staged=${1:?usage: ci/release/smoke-channel.sh <staged-dir> <version>}
version=${2:?usage: ci/release/smoke-channel.sh <staged-dir> <version>}

scratch=$(mktemp -d)
cleanup() {
  rm -rf "$scratch"
}
trap cleanup EXIT

# 1. Verify GitHub pre-release archives downloaded for both Products.
for product in collie foxhound; do
  archive=$(find "$staged/github" -maxdepth 2 -name "goodfetch-$product-*.tar.xz" | head -n 1)
  if [[ -n "$archive" ]]; then
    dest="$scratch/gh-$product"
    mkdir -p "$dest"
    tar -xJf "$archive" -C "$dest"
    exe=$(find "$dest" -maxdepth 2 -type f -name "goodfetch-$product*" | head -n 1)
    chmod +x "$exe"
    "$exe" --version | grep -F "$version" >/dev/null
  fi
done

# 2. Verify `.devN` wheels staged from the GitHub pre-release (or nightly Python
#    index from #205) install offline in a fresh venv and start both executables.
if [[ -d "$staged/wheels" ]] && ls "$staged/wheels"/*.whl >/dev/null 2>&1; then
  venv="$scratch/wheel-venv"
  uv venv --seed "$venv" >/dev/null
  "$venv/bin/pip" install --quiet --no-deps --no-index --find-links "$staged/wheels" \
    goodfetch-collie-client goodfetch-collie goodfetch-foxhound
  "$venv/bin/python" -c "import goodfetch_collie_client, goodfetch_collie, goodfetch_foxhound"
fi

# 3. Verify npm `nightly` tarballs staged from npm install with `--ignore-scripts`
#    and start both Products' executables.
if [[ -d "$staged/npm" ]] && ls "$staged/npm"/*.tgz >/dev/null 2>&1; then
  npm_dir="$scratch/npm-project"
  mkdir -p "$npm_dir"
  printf '{"name":"goodfetch-smoke","private":true}\n' >"$npm_dir/package.json"
  npm --prefix "$npm_dir" install --offline --ignore-scripts "$staged/npm"/*.tgz >/dev/null
fi
