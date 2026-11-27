#!/usr/bin/env bash
# Downloads the pinned local model servers (llama.cpp server and Ollama) and
# small CPU GGUF model from test-support/local-models/pins.toml before ADR
# 0043's outbound-traffic deny, into $GOODFETCH_LOCAL_MODELS_DOWNLOADS or
# ~/.cache/goodfetch/local-models, verifying every SHA-256 digest before
# moving any artifact into place (ADRs 0065 and 0076; issue 427).
#
# In CI, downloads only when GOODFETCH_FETCH_LOCAL_MODELS=yes (the Release
# suite's Linux cell); on a developer machine where CI is unset, fetches only
# when GOODFETCH_FETCH_LOCAL_MODELS=yes so routine quick/pre-merge setups do
# not download hundreds of megabytes unnecessarily.
set -euo pipefail

root=$(cd "$(dirname "$0")/.." && pwd)
pins="$root/test-support/local-models/pins.toml"

if [[ "${GOODFETCH_FETCH_LOCAL_MODELS:-}" != yes ]]; then
  echo "GOODFETCH_FETCH_LOCAL_MODELS is not yes: skipping local model server fetch"
  exit 0
fi

case "$(uname -s)-$(uname -m)" in
  Linux-x86_64) platform=linux-x64 ;;
  Linux-aarch64 | Linux-arm64) platform=linux-arm64 ;;
  *)
    echo "Local model Release cell runs on Linux only; skipping on $(uname -s) $(uname -m)"
    exit 0
    ;;
esac

table_field() {
  local table=$1
  local key=$2
  awk -v table="[$table]" -v key="$key" '
    $0 == table { inside = 1; next }
    /^\[/ { inside = 0 }
    inside && $1 == key { gsub(/"/, "", $3); print $3; exit }
  ' "$pins"
}

sha256() {
  if command -v sha256sum >/dev/null; then
    sha256sum "$1" | cut -d' ' -f1
  else
    shasum -a 256 "$1" | cut -d' ' -f1
  fi
}

fetch_verified() {
  local url=$1
  local expected=$2
  local target=$3
  if [[ -f "$target" && "$(sha256 "$target")" == "$expected" ]]; then
    return 0
  fi
  mkdir -p "$(dirname "$target")"
  local partial="${target}.partial"
  curl --fail --location --silent --show-error --retry 3 --output "$partial" "$url"
  local actual
  actual=$(sha256 "$partial")
  if [[ "$actual" != "$expected" ]]; then
    rm -f "$partial"
    echo "error: digest mismatch for $url (expected $expected, got $actual)" >&2
    exit 1
  fi
  mv "$partial" "$target"
}

cache="${GOODFETCH_LOCAL_MODELS_DOWNLOADS:-$HOME/.cache/goodfetch/local-models}"

# 1. Pinned small CPU GGUF model.
model_file=$(table_field "model" "file")
model_url=$(table_field "model" "url")
model_sha=$(table_field "model" "sha256")
fetch_verified "$model_url" "$model_sha" "$cache/models/$model_file"

# 2. Pinned llama.cpp server.
llama_ver=$(table_field "llama_cpp" "version")
llama_url=$(table_field "llama_cpp.platforms.$platform" "url")
llama_sha=$(table_field "llama_cpp.platforms.$platform" "sha256")
llama_archive="$cache/archives/llama-cpp-${llama_ver}-${platform}.tar.gz"
llama_dir="$cache/llama-cpp/$llama_ver"
fetch_verified "$llama_url" "$llama_sha" "$llama_archive"
if [[ ! -x "$llama_dir/llama-server" ]]; then
  mkdir -p "$llama_dir"
  tar -xzf "$llama_archive" -C "$llama_dir" --strip-components=1
fi

# 3. Pinned Ollama server.
ollama_ver=$(table_field "ollama" "version")
ollama_url=$(table_field "ollama.platforms.$platform" "url")
ollama_sha=$(table_field "ollama.platforms.$platform" "sha256")
ollama_archive="$cache/archives/ollama-${ollama_ver}-${platform}.tar.zst"
ollama_dir="$cache/ollama/$ollama_ver"
fetch_verified "$ollama_url" "$ollama_sha" "$ollama_archive"
if [[ ! -x "$ollama_dir/bin/ollama" ]]; then
  mkdir -p "$ollama_dir"
  tar --zstd -xf "$ollama_archive" -C "$ollama_dir"
fi

echo "Local models ready in $cache (llama.cpp $llama_ver, ollama $ollama_ver, $model_file)"
