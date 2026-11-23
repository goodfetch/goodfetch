#!/usr/bin/env bash
# Downloads the Lightpanda build Foxhound pins for this machine's platform
# (crates/foxhound/goodfetch-fetch-tier/src/light_browser/pins.json) into
# the tests' copy, $GOODFETCH_LIGHTPANDA_DOWNLOADS or
# ~/.cache/goodfetch/lightpanda, checked against its pinned digest. The
# light-browser tier's tests lay it out in their own Browser cache from
# there, as they lay out Playwright's Chrome for Testing, so no test
# downloads it (ADR 0020). Nothing is downloaded when the copy is there,
# or on a platform Lightpanda builds nothing for. It tests nothing, so it
# may retry (ADR 0064).
set -euo pipefail

root=$(cd "$(dirname "$0")/.." && pwd)
pins="$root/crates/foxhound/goodfetch-fetch-tier/src/light_browser/pins.json"

case "$(uname -s)-$(uname -m)" in
  Linux-x86_64) platform=linux-x64 ;;
  Linux-aarch64 | Linux-arm64) platform=linux-arm64 ;;
  Darwin-arm64) platform=mac-arm64 ;;
  Darwin-x86_64) platform=mac-x64 ;;
  *)
    echo "Lightpanda builds nothing for $(uname -s) $(uname -m): nothing to fetch"
    exit 0
    ;;
esac

# jq, not Python: the hosted tier's CI image has no system Python before
# `uv sync`'s environment, and every host that runs `just fetch` has jq.
read -r build asset digest < <(jq -er --arg platform "$platform" '
  .lightpanda as $pins
  | ($pins.platforms[$platform] // error("no Lightpanda pin for \($platform)")) as $pinned
  | [$pins.build, $pinned.asset, ($pinned.archive_digest | ltrimstr("sha256:"))]
  | @tsv
' "$pins")

folder="${GOODFETCH_LIGHTPANDA_DOWNLOADS:-$HOME/.cache/goodfetch/lightpanda}/$build"
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
  "https://github.com/lightpanda-io/browser/releases/download/$build/$asset"
if [[ "$(sha256 "$partial")" != "$digest" ]]; then
  rm -f "$partial"
  echo "error: the Lightpanda $build download for $platform does not have its pinned digest" >&2
  exit 1
fi
chmod 755 "$partial"
mv "$partial" "$target"
echo "Lightpanda $build is at $target"
