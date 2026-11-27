#!/usr/bin/env bash
# Runs every packaged file of both Products from its own bytes on the system it
# targets before publishing (ADR 0094; issue 143):
# 1. Each Product's GitHub Release `.tar.xz` archive: extracts the executable,
#    checks `--version`, and sends a JSON-RPC 2.0 `hello` over stdio verifying
#    it answers `hello`.
# 2. Each Product's `py3-none` wheel: installs into a fresh virtual environment
#    offline and starts its executable.
# 3. Each Product's npm package + platform package: installs into a fresh
#    temporary directory with `--ignore-scripts` and starts its executable.
#
# Usage: ci/release/verify-bytes.sh <packaged-dir> <target> <version>
set -euo pipefail

packaged=${1:?usage: ci/release/verify-bytes.sh <packaged-dir> <target> <version>}
target=${2:?usage: ci/release/verify-bytes.sh <packaged-dir> <target> <version>}
version=${3:?usage: ci/release/verify-bytes.sh <packaged-dir> <target> <version>}

scratch=$(mktemp -d)
cleanup() {
  rm -rf "$scratch"
}
trap cleanup EXIT

suffix=""
[[ "$target" == *-windows-* ]] && suffix=.exe

verify_executable() {
  local bin_path=$1
  local product=$2
  chmod +x "$bin_path"
  "$bin_path" --version | grep -F "$version" >/dev/null
  # Answer `hello` over stdio JSON-RPC (ADR 0038, ADR 0094; HelloParams schema).
  local hello_json
  hello_json=$(jq -cn \
    --arg needs "$product" \
    --arg product_version "$version" \
    '{jsonrpc:"2.0",id:1,method:"hello",params:{protocol_version:"0.1",surface:"cli",needs:$needs,product_version:$product_version}}')
  local reply
  reply=$(printf '%s\n' "$hello_json" | "$bin_path" attached-engine 2>/dev/null | head -n 1)
  printf '%s\n' "$reply" | jq -e '.id == 1 and (.result != null)' >/dev/null
}

for product in collie foxhound; do
  exe="goodfetch-$product$suffix"
  archive="$packaged/assets/goodfetch-$product-$target.tar.xz"
  if [[ ! -f "$archive" ]]; then
    echo "error: missing archive $archive" >&2
    exit 1
  fi
  extract_dir="$scratch/archive-$product"
  mkdir -p "$extract_dir"
  tar -xJf "$archive" -C "$extract_dir"
  verify_executable "$extract_dir/$exe" "$product"
done

# Fresh Python virtual environment installing both Products' wheels from their
# own packaged bytes (no network, no install scripts) and verifying each
# installed executable answers `--version` and `hello`.
venv="$scratch/venv"
uv venv --seed "$venv" >/dev/null
pip_bin="$venv/bin/pip"
py_bin="$venv/bin/python"
if [[ -x "$venv/Scripts/pip.exe" ]]; then
  pip_bin="$venv/Scripts/pip.exe"
  py_bin="$venv/Scripts/python.exe"
fi
"$pip_bin" install --quiet --no-deps --no-index --find-links "$packaged/wheels" \
  goodfetch-collie-client goodfetch-collie goodfetch-foxhound
for product in collie foxhound; do
  wheel_bin=$("$py_bin" -c "import pathlib, goodfetch_${product}; print(pathlib.Path(goodfetch_${product}.__file__).parent / 'bin' / 'goodfetch-${product}${suffix}')")
  if [[ ! -f "$wheel_bin" ]]; then
    echo "error: wheel did not install expected binary $wheel_bin" >&2
    exit 1
  fi
  verify_executable "$wheel_bin" "$product"
done

# Fresh npm project installing each Product's packaged root + platform tarballs
# with `--ignore-scripts` and starting its installed executable (ADR 0090, 0094).
npm_suffix=""
case "$target" in
  x86_64-unknown-linux-*) npm_suffix="linux-x64" ;;
  aarch64-unknown-linux-*) npm_suffix="linux-arm64" ;;
  aarch64-apple-darwin) npm_suffix="darwin-arm64" ;;
  x86_64-apple-darwin) npm_suffix="darwin-x64" ;;
  x86_64-pc-windows-msvc) npm_suffix="win32-x64" ;;
  aarch64-pc-windows-msvc) npm_suffix="win32-arm64" ;;
  *)
    echo "error: unsupported npm target $target" >&2
    exit 1
    ;;
esac

npm_proj="$scratch/npm-project"
mkdir -p "$npm_proj"
printf '{"name":"goodfetch-verify-bytes","private":true}\n' >"$npm_proj/package.json"
npm_tarballs=(
  "$packaged/npm/goodfetch-collie-client-${version}.tgz"
  "$packaged/npm/goodfetch-collie-${npm_suffix}-${version}.tgz"
  "$packaged/npm/goodfetch-collie-${version}.tgz"
  "$packaged/npm/goodfetch-foxhound-${npm_suffix}-${version}.tgz"
  "$packaged/npm/goodfetch-foxhound-${version}.tgz"
)
for tgz in "${npm_tarballs[@]}"; do
  if [[ ! -f "$tgz" ]]; then
    echo "error: missing packaged npm tarball $tgz" >&2
    exit 1
  fi
done
npm --prefix "$npm_proj" install --offline --ignore-scripts "${npm_tarballs[@]}" >/dev/null
for product in collie foxhound; do
  npm_bin="$npm_proj/node_modules/@goodfetch/$product-$npm_suffix/bin/goodfetch-$product$suffix"
  if [[ ! -f "$npm_bin" ]]; then
    echo "error: npm install did not unpack platform binary $npm_bin" >&2
    exit 1
  fi
  verify_executable "$npm_bin" "$product"
done
