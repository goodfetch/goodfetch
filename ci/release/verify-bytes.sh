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

hello_payload='{"jsonrpc":"2.0","id":1,"method":"hello","params":{"protocol_version":"0.1","product":"%s","surface":"cli"}}'

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
  bin_path="$extract_dir/$exe"
  chmod +x "$bin_path"

  "$bin_path" --version | grep -F "$version" >/dev/null
  # Answer `hello` over stdio JSON-RPC (ADR 0038, ADR 0094).
  reply=$(printf "$hello_payload\n" "$product" | "$bin_path" engine --stdio 2>/dev/null | head -n 1)
  echo "$reply" | jq -e '.id == 1 and (.result != null)' >/dev/null
done

# Fresh Python virtual environment installing both Products' wheels from their
# own packaged bytes (no network, no install scripts).
venv="$scratch/venv"
uv venv --seed "$venv" >/dev/null
"$venv/bin/pip" install --quiet --no-deps --no-index --find-links "$packaged/wheels" \
  goodfetch-collie-client goodfetch-collie goodfetch-foxhound
"$venv/bin/python" -c "
import subprocess, pathlib
for pkg, exe in [('goodfetch_collie', 'goodfetch-collie${suffix}'), ('goodfetch_foxhound', 'goodfetch-foxhound${suffix}')]:
    mod = __import__(pkg)
    bin_path = pathlib.Path(mod.__file__).parent / 'bin' / exe
    out = subprocess.check_output([str(bin_path), '--version'], text=True)
    assert '${version}' in out, (pkg, out)
"

# Fresh npm project verifying both Products' packaged platform binaries start
# without any install script.
for product in collie foxhound; do
  npm_suffix=""
  case "$target" in
    x86_64-unknown-linux-*) npm_suffix="linux-x64" ;;
    aarch64-unknown-linux-*) npm_suffix="linux-arm64" ;;
    aarch64-apple-darwin) npm_suffix="darwin-arm64" ;;
    x86_64-apple-darwin) npm_suffix="darwin-x64" ;;
    x86_64-pc-windows-msvc) npm_suffix="win32-x64" ;;
    aarch64-pc-windows-msvc) npm_suffix="win32-arm64" ;;
  esac
  npm_bin="$packaged/npm/$product-$npm_suffix/bin/goodfetch-$product$suffix"
  if [[ -f "$npm_bin" ]]; then
    chmod +x "$npm_bin"
    "$npm_bin" --version | grep -F "$version" >/dev/null
  fi
done
