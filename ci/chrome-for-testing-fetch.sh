#!/usr/bin/env bash
# Downloads the archive of the Chrome for Testing build Collie pins for this
# machine's platform (crates/goodfetch-collie/src/pins.json) into the tests'
# copy, $GOODFETCH_CHROME_FOR_TESTING_DOWNLOADS or
# ~/.cache/goodfetch/chrome-for-testing, laid out as Chrome for Testing's
# storage lays it out (<build>/<platform>/chrome-<platform>.zip) and checked
# against its pinned digest. The provisioning tests (#3100) hand that folder
# to the Proving ground's browser-builds.test, from which an engine's first
# use downloads the build, so no test reaches Google's storage. Nothing is
# downloaded when the copy is there with its digest, or on a platform Chrome
# for Testing builds nothing for. It tests nothing, so it may retry
# (ADR 0064).
set -euo pipefail

root=$(cd "$(dirname "$0")/.." && pwd)
pins="$root/crates/goodfetch-collie/src/pins.json"
storage="https://storage.googleapis.com/chrome-for-testing-public"

case "$(uname -s)-$(uname -m)" in
  Linux-x86_64) platform=linux64 ;;
  Darwin-arm64) platform=mac-arm64 ;;
  Darwin-x86_64) platform=mac-x64 ;;
  *)
    echo "Chrome for Testing builds nothing for $(uname -s) $(uname -m): nothing to fetch"
    exit 0
    ;;
esac

read -r build digest < <(python3 -c '
import json, sys
pins = json.load(open(sys.argv[1]))["chrome_for_testing"]
print(pins["build"], pins["platforms"][sys.argv[2]]["archive_digest"].removeprefix("sha256:"))
' "$pins" "$platform")

archive="chrome-$platform.zip"
folder="${GOODFETCH_CHROME_FOR_TESTING_DOWNLOADS:-$HOME/.cache/goodfetch/chrome-for-testing}/$build/$platform"
target="$folder/$archive"

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
  "$storage/$build/$platform/$archive"
if [[ "$(sha256 "$partial")" != "$digest" ]]; then
  rm -f "$partial"
  echo "error: the Chrome for Testing $build download for $platform does not have its pinned digest" >&2
  exit 1
fi
mv "$partial" "$target"
echo "Chrome for Testing $build is at $target"
