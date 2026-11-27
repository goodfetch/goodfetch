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
http_pid=""
cleanup() {
  [[ -n "$http_pid" ]] && kill "$http_pid" 2>/dev/null || true
  rm -rf "$scratch"
}
trap cleanup EXIT

exe_suffix=""
case "$(uname -s)" in
  MINGW*|MSYS*|CYGWIN*) exe_suffix=".exe" ;;
esac

verify_installed_exe() {
  local bin_path=$1
  local product=$2
  chmod +x "$bin_path"
  "$bin_path" --version | grep -F "$version" >/dev/null
  local hello_json
  hello_json=$(jq -cn \
    --arg needs "$product" \
    --arg product_version "$version" \
    '{jsonrpc:"2.0",id:1,method:"hello",params:{protocol_version:"0.1",surface:"cli",needs:$needs,product_version:$product_version}}')
  local reply
  reply=$(printf '%s\n' "$hello_json" | "$bin_path" attached-engine 2>/dev/null | head -n 1)
  printf '%s\n' "$reply" | jq -e '.id == 1 and (.result != null)' >/dev/null
}

# 1. Verify GitHub pre-release archives downloaded for both Products.
for product in collie foxhound; do
  archive=$(find "$staged/github" -maxdepth 2 -name "goodfetch-$product-*.tar.xz" | head -n 1)
  if [[ -n "$archive" ]]; then
    dest="$scratch/gh-$product"
    mkdir -p "$dest"
    tar -xJf "$archive" -C "$dest"
    exe=$(find "$dest" -maxdepth 2 -type f -name "goodfetch-$product*" | head -n 1)
    verify_installed_exe "$exe" "$product"
  fi
done

# 2. Verify `.devN` wheels staged from the GitHub pre-release and the generated
#    PEP 503/691 nightly Python simple index (issue 205) install offline in
#    fresh venvs via both `pip install --pre --index-url` and `uv`, and start
#    each Product's installed executable.
if [[ -d "$staged/wheels" ]] && ls "$staged/wheels"/*.whl >/dev/null 2>&1; then
  venv="$scratch/wheel-venv"
  uv venv --seed "$venv" >/dev/null
  pip_bin="$venv/bin/pip"
  py_bin="$venv/bin/python"
  [[ -x "$venv/Scripts/pip.exe" ]] && pip_bin="$venv/Scripts/pip.exe" && py_bin="$venv/Scripts/python.exe"
  "$pip_bin" install --quiet --pre --no-deps --no-index --find-links "$staged/wheels" \
    goodfetch-collie-client goodfetch-collie goodfetch-foxhound
  for product in collie foxhound; do
    wbin=$("$py_bin" -c "import pathlib, goodfetch_${product}; print(pathlib.Path(goodfetch_${product}.__file__).parent / 'bin' / 'goodfetch-${product}${exe_suffix}')")
    verify_installed_exe "$wbin" "$product"
  done

  if [[ -d "$staged/python-index/simple" ]]; then
    for project in goodfetch-collie goodfetch-collie-client goodfetch-foxhound; do
      test -f "$staged/python-index/simple/$project/index.html"
      test -f "$staged/python-index/simple/$project/index.json"
    done
    # Serve the generated PEP 503/691 index and staged wheel files over loopback
    # HTTP so `pip` and `uv` consume the real generated index on every OS
    # (including Windows Git Bash) under the outbound deny.
    serve_root="$scratch/http-index"
    cp -R "$staged/python-index" "$serve_root"
    mkdir -p "$serve_root/wheels"
    cp "$staged/wheels"/*.whl "$serve_root/wheels/"
    port=$("$py_bin" -c "import socket; s=socket.socket(); s.bind(('127.0.0.1',0)); print(s.getsockname()[1]); s.close()")
    # Rewrite external GitHub download URLs inside the staged copy to our loopback wheel directory.
    "$py_bin" - "$serve_root/simple" "http://127.0.0.1:${port}/wheels" <<'PY'
import pathlib, re, sys
root = pathlib.Path(sys.argv[1])
base = sys.argv[2]
for page in list(root.rglob("index.html")) + list(root.rglob("index.json")):
    text = page.read_text(encoding="utf-8")
    updated = re.sub(r'https://[^"\s#]+/([^/"\s#]+\.whl)', rf'{base}/\1', text)
    page.write_text(updated, encoding="utf-8")
PY
    "$py_bin" -m http.server "$port" --bind 127.0.0.1 --directory "$serve_root" >"$scratch/http.log" 2>&1 &
    http_pid=$!
    for _ in $(seq 20); do
      curl -fsS "http://127.0.0.1:${port}/simple/" >/dev/null 2>&1 && break
      sleep 0.1
    done
    index_url="http://127.0.0.1:${port}/simple/"
    for product in collie foxhound; do
      pkg="goodfetch-$product"
      pip_venv="$scratch/pip-index-$product"
      uv venv --seed "$pip_venv" >/dev/null
      pbin="$pip_venv/bin/pip"
      ppy="$pip_venv/bin/python"
      [[ -x "$pip_venv/Scripts/pip.exe" ]] && pbin="$pip_venv/Scripts/pip.exe" && ppy="$pip_venv/Scripts/python.exe"
      "$pbin" install --quiet --pre --no-deps --index-url "$index_url" "$pkg"
      wbin=$("$ppy" -c "import pathlib, goodfetch_${product}; print(pathlib.Path(goodfetch_${product}.__file__).parent / 'bin' / 'goodfetch-${product}${exe_suffix}')")
      verify_installed_exe "$wbin" "$product"

      uv_venv="$scratch/uv-index-$product"
      uv venv "$uv_venv" >/dev/null
      upy="$uv_venv/bin/python"
      [[ -x "$uv_venv/Scripts/python.exe" ]] && upy="$uv_venv/Scripts/python.exe"
      uv pip install --quiet --python "$uv_venv" --prerelease=allow --no-deps \
        --index "goodfetch-nightly=$index_url" "$pkg"
      wbin=$("$upy" -c "import pathlib, goodfetch_${product}; print(pathlib.Path(goodfetch_${product}.__file__).parent / 'bin' / 'goodfetch-${product}${exe_suffix}')")
      verify_installed_exe "$wbin" "$product"
    done
    kill "$http_pid" 2>/dev/null || true
    http_pid=""
  fi
fi

# 3. Verify npm `nightly` tarballs staged from npm install with `--ignore-scripts`
#    and start both Products' executables.
if [[ -d "$staged/npm" ]] && ls "$staged/npm"/*.tgz >/dev/null 2>&1; then
  npm_dir="$scratch/npm-project"
  mkdir -p "$npm_dir"
  printf '{"name":"goodfetch-smoke","private":true}\n' >"$npm_dir/package.json"
  npm --prefix "$npm_dir" install --offline --ignore-scripts "$staged/npm"/*.tgz >/dev/null
  for product in collie foxhound; do
    npm_bin=$(find "$npm_dir/node_modules/@goodfetch" -path "*/bin/goodfetch-${product}${exe_suffix}" -type f | head -n 1)
    if [[ -z "$npm_bin" ]]; then
      echo "error: npm smoke install did not unpack platform binary for $product" >&2
      exit 1
    fi
    verify_installed_exe "$npm_bin" "$product"
  done
fi

# 4. Verify staged GHCR `nightly` OCI images on Linux start each Product's executable.
if [[ -d "$staged/oci" ]] && command -v docker >/dev/null 2>&1; then
  for product in collie foxhound; do
    img_tar="$staged/oci/goodfetch-$product-nightly.tar"
    if [[ -f "$img_tar" ]]; then
      loaded=$(docker load -i "$img_tar" | awk '/Loaded image:/ {print $3}' | tail -n 1)
      docker run --rm --network none "$loaded" --version | grep -F "$version" >/dev/null
    fi
  done
fi
