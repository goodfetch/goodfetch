#!/usr/bin/env bash
# Downloads the lychee release test-support/lychee/pins.toml pins, for this
# machine's platform, checked against its pinned SHA-256, and unpacks its
# executable into the tests' copy, $GOODFETCH_LYCHEE_DOWNLOADS or
# ~/.cache/goodfetch/lychee, before the outbound-traffic deny. The docs
# site's tests run it offline over the built site (TESTING.md, Docs site);
# no test downloads it. Nothing is downloaded when the copy is there. Part
# of `just fetch`. It tests nothing, so it may retry (ADR 0064).
set -euo pipefail

root=$(cd "$(dirname "$0")/.." && pwd)
pins="$root/test-support/lychee/pins.toml"

executable=lychee
case "$(uname -s)-$(uname -m)" in
  Linux-x86_64) platform=linux-x64 ;;
  Linux-aarch64 | Linux-arm64) platform=linux-arm64 ;;
  Darwin-arm64) platform=mac-arm64 ;;
  Darwin-x86_64) platform=mac-x64 ;;
  MINGW*-x86_64 | MSYS*-x86_64 | CYGWIN*-x86_64)
    platform=windows-x64
    executable=lychee.exe
    ;;
  *)
    echo "error: lychee has no pin for $(uname -s) $(uname -m)" >&2
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
  echo "error: $pins pins no lychee release for $platform" >&2
  exit 1
fi

folder="${GOODFETCH_LYCHEE_DOWNLOADS:-$HOME/.cache/goodfetch/lychee}/$version"
archive="$folder/$asset"
target="$folder/$executable"

sha256() {
  if command -v sha256sum >/dev/null; then
    sha256sum "$1" | cut -d' ' -f1
  else
    shasum -a 256 "$1" | cut -d' ' -f1
  fi
}

# The executable is unpacked from the checked archive, which stays beside it,
# so a copy is current when both are there and the archive has its pin.
if [[ -x "$target" && -f "$archive" && "$(sha256 "$archive")" == "$digest" ]]; then
  exit 0
fi
mkdir -p "$folder"
partial="$archive.partial"
curl --fail --location --silent --show-error --retry 3 \
  --output "$partial" \
  "https://github.com/lycheeverse/lychee/releases/download/$version/$asset"
if [[ "$(sha256 "$partial")" != "$digest" ]]; then
  rm -f "$partial"
  echo "error: the lychee $version download for $platform does not have its pinned SHA-256" >&2
  exit 1
fi
mv "$partial" "$archive"

unpacked=$(mktemp -d "$folder/unpack.XXXXXX")
trap 'rm -rf "$unpacked"' EXIT
case "$asset" in
  *.zip) unzip -q "$archive" -d "$unpacked" ;;
  *) tar -xzf "$archive" -C "$unpacked" ;;
esac
found=$(find "$unpacked" -type f -name "$executable" | head -1)
if [[ -z "$found" ]]; then
  echo "error: the lychee $version archive for $platform holds no $executable" >&2
  exit 1
fi
chmod 755 "$found"
mv "$found" "$target"
echo "lychee $version is at $target"
