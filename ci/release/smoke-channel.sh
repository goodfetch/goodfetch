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

# 2. Verify `.devN` wheels staged from the GitHub pre-release and the nightly
#    Python simple index (issue 205) install offline in fresh venvs via both
#    `pip install --pre --index-url` and `uv` for each Product.
if [[ -d "$staged/wheels" ]] && ls "$staged/wheels"/*.whl >/dev/null 2>&1; then
  venv="$scratch/wheel-venv"
  uv venv --seed "$venv" >/dev/null
  pip_bin="$venv/bin/pip"
  py_bin="$venv/bin/python"
  [[ -x "$venv/Scripts/pip.exe" ]] && pip_bin="$venv/Scripts/pip.exe" && py_bin="$venv/Scripts/python.exe"
  "$pip_bin" install --quiet --pre --no-deps --no-index --find-links "$staged/wheels" \
    goodfetch-collie-client goodfetch-collie goodfetch-foxhound
  "$py_bin" -c "import goodfetch_collie_client, goodfetch_collie, goodfetch_foxhound"

  if [[ -d "$staged/python-index/simple" ]]; then
    # Verify PEP 503 HTML and PEP 691 JSON project pages exist for each Product,
    # then test `pip install --pre --index-url` and `uv pip install` against a
    # local file/loopback copy of the simple index pointing at the staged wheels.
    for project in goodfetch-collie goodfetch-collie-client goodfetch-foxhound; do
      test -f "$staged/python-index/simple/$project/index.html"
      test -f "$staged/python-index/simple/$project/index.json"
    done
    local_index="$scratch/local-simple"
    mkdir -p "$local_index/goodfetch-collie" "$local_index/goodfetch-foxhound" "$local_index/goodfetch-collie-client"
    for whl in "$staged/wheels"/*.whl; do
      base=$(basename "$whl")
      abs=$(cd "$(dirname "$whl")" && pwd)/"$base"
      sha=$(sha256sum "$abs" | awk '{print $1}')
      case "$base" in
        goodfetch_collie_client-*) proj="goodfetch-collie-client" ;;
        goodfetch_collie-*) proj="goodfetch-collie" ;;
        goodfetch_foxhound-*) proj="goodfetch-foxhound" ;;
        *) continue ;;
      esac
      printf '<a href="file://%s#sha256=%s">%s</a><br>\n' "$abs" "$sha" "$base" \
        >>"$local_index/$proj/index.html"
    done
    for product in goodfetch-collie goodfetch-foxhound; do
      pip_venv="$scratch/pip-index-$product"
      uv venv --seed "$pip_venv" >/dev/null
      pbin="$pip_venv/bin/pip"
      [[ -x "$pip_venv/Scripts/pip.exe" ]] && pbin="$pip_venv/Scripts/pip.exe"
      "$pbin" install --quiet --pre --no-deps --index-url "file://$local_index" "$product"

      uv_venv="$scratch/uv-index-$product"
      uv venv "$uv_venv" >/dev/null
      uv pip install --quiet --python "$uv_venv" --prerelease=allow --no-deps \
        --index "goodfetch-nightly=file://$local_index" "$product"
    done
  fi
fi

# 3. Verify npm `nightly` tarballs staged from npm install with `--ignore-scripts`
#    and start both Products' executables.
if [[ -d "$staged/npm" ]] && ls "$staged/npm"/*.tgz >/dev/null 2>&1; then
  npm_dir="$scratch/npm-project"
  mkdir -p "$npm_dir"
  printf '{"name":"goodfetch-smoke","private":true}\n' >"$npm_dir/package.json"
  npm --prefix "$npm_dir" install --offline --ignore-scripts "$staged/npm"/*.tgz >/dev/null
fi
