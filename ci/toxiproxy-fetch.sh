#!/usr/bin/env bash
# Downloads the Toxiproxy server release test-support/toxiproxy/pins.toml
# pins, for this machine's platform, into the tests' copy,
# $GOODFETCH_TOXIPROXY_DOWNLOADS or ~/.cache/goodfetch/toxiproxy, checked
# against its pinned SHA-256, before the outbound-traffic deny. Toxiproxy is
# free and runs on loopback (ADR 0060); no test downloads it. Nothing is
# downloaded when the copy is there. Part of `just fetch`. It tests nothing,
# so it may retry (ADR 0064).
#
# A platform of pins.toml as its argument fetches that platform's copy
# instead: ci/windows.sh fetches `windows-x64`'s for the tests under Wine.
set -euo pipefail

root=$(cd "$(dirname "$0")/.." && pwd)
pins="$root/test-support/toxiproxy/pins.toml"

case "${1:-$(uname -s)-$(uname -m)}" in
  linux-x64 | linux-arm64 | mac-arm64 | mac-x64 | windows-x64) platform=$1 ;;
  Linux-x86_64) platform=linux-x64 ;;
  Linux-aarch64 | Linux-arm64) platform=linux-arm64 ;;
  Darwin-arm64) platform=mac-arm64 ;;
  Darwin-x86_64) platform=mac-x64 ;;
  MINGW*-x86_64 | MSYS*-x86_64 | CYGWIN*-x86_64) platform=windows-x64 ;;
  *)
    echo "error: Toxiproxy has no pin for $(uname -s) $(uname -m)" >&2
    exit 1
    ;;
esac

# The pins file's own simple layout, read without Python, which Windows'
# Git Bash may lack: `key = "value"` lines, each platform's under its table.
version=$(sed -n 's/^version = "\(.*\)"$/\1/p' "$pins" | head -1)
pinned() {
  awk -v table="[platforms.$platform]" -v key="$1" '
    $0 == table { inside = 1; next }
    /^\[/ { inside = 0 }
    inside && $1 == key { gsub(/"/, "", $3); print $3 }
  ' "$pins"
}
asset=$(pinned asset)
digest=$(pinned sha256)
if [[ -z "$version" || -z "$asset" || -z "$digest" ]]; then
  echo "error: $pins pins no Toxiproxy release for $platform" >&2
  exit 1
fi

folder="${GOODFETCH_TOXIPROXY_DOWNLOADS:-$HOME/.cache/goodfetch/toxiproxy}/$version"
target="$folder/$asset"

sha256() {
  if command -v sha256sum >/dev/null; then
    sha256sum "$1" | cut -d' ' -f1
  else
    shasum -a 256 "$1" | cut -d' ' -f1
  fi
}

if [[ -f "$target" && "$(sha256 "$target")" == "$digest" ]]; then
  exit 0
fi
mkdir -p "$folder"
partial="$target.partial"
curl --fail --location --silent --show-error --retry 3 \
  --output "$partial" \
  "https://github.com/Shopify/toxiproxy/releases/download/$version/$asset"
if [[ "$(sha256 "$partial")" != "$digest" ]]; then
  rm -f "$partial"
  echo "error: the Toxiproxy $version download for $platform does not have its pinned SHA-256" >&2
  exit 1
fi
chmod 755 "$partial"
mv "$partial" "$target"
echo "Toxiproxy $version is at $target"
