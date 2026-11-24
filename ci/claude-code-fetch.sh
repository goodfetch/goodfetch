#!/usr/bin/env bash
# Downloads the Claude Code release test-support/claude-code/pins.toml pins,
# for this machine's platform, into the tests' copy,
# $GOODFETCH_CLAUDE_CODE_DOWNLOADS or ~/.cache/goodfetch/claude-code, as
# <version>/claude (claude.exe on Windows), checked against its pinned
# SHA-256. Each Product's plugin test runs it offline to validate the
# plugin (story 11-239), so no test downloads it. The copy is Anthropic's
# proprietary software, downloaded for the tests alone and never shipped
# (the pins file's header). Nothing is downloaded when the copy is there
# with its digest, or on a platform Claude Code builds nothing for. It
# tests nothing, so it may retry (ADR 0064).
#
# The copy is about 340 MB, so a CI job (CI set) downloads it only when it
# runs the plugin tests and says so with GOODFETCH_FETCH_CLAUDE_CODE=yes:
# the Release suite's cells, and every job that runs the Pre-merge suite
# over goodfetch-collie or goodfetch-foxhound. A developer's machine and the
# gate host, where CI is unset, always download it.
#
# `win32-x64` as its argument fetches Windows' copy instead: ci/windows.sh
# fetches it for the plugin tests under Wine.
set -euo pipefail

root=$(cd "$(dirname "$0")/.." && pwd)
pins="$root/test-support/claude-code/pins.toml"

if [[ -n "${CI:-}" && "${GOODFETCH_FETCH_CLAUDE_CODE:-}" != yes ]]; then
  echo "This CI job runs no plugin test (GOODFETCH_FETCH_CLAUDE_CODE is not yes): Claude Code is not fetched"
  exit 0
fi

executable=claude
case "${1:-$(uname -s)-$(uname -m)}" in
  win32-x64) platform=win32-x64 executable=claude.exe ;;
  Linux-x86_64) platform=linux-x64 ;;
  Linux-aarch64 | Linux-arm64) platform=linux-arm64 ;;
  Darwin-arm64) platform=darwin-arm64 ;;
  Darwin-x86_64) platform=darwin-x64 ;;
  MINGW*-x86_64 | MSYS*-x86_64) platform=win32-x64 executable=claude.exe ;;
  MINGW*-aarch64 | MSYS*-aarch64) platform=win32-arm64 executable=claude.exe ;;
  *)
    echo "Claude Code builds nothing for $(uname -s) $(uname -m): nothing to fetch"
    exit 0
    ;;
esac
# Linux's builds link glibc or musl: the copy must run on this machine.
if [[ $platform == linux-* ]] && ldd --version 2>&1 | grep -qi musl; then
  platform="$platform-musl"
fi

# The pins file's own simple layout, read without Python, which Windows'
# Git Bash may lack: `key = "value"` lines, each platform's under its table.
pinned() { sed -n "s/^$1 = \"\(.*\)\"\$/\1/p" "$pins" | head -1; }
version=$(pinned version)
bucket=$(pinned bucket)
digest=$(awk -v table="[platforms.$platform]" '
  $0 == table { inside = 1; next }
  /^\[/ { inside = 0 }
  inside && $1 == "sha256" { gsub(/"/, "", $3); print $3 }
' "$pins")
if [[ -z "$version" || -z "$bucket" || -z "$digest" ]]; then
  echo "error: $pins pins no Claude Code release for $platform" >&2
  exit 1
fi

folder="${GOODFETCH_CLAUDE_CODE_DOWNLOADS:-$HOME/.cache/goodfetch/claude-code}/$version"
target="$folder/$executable"

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
  "$bucket/$version/$platform/$executable"
if [[ "$(sha256 "$partial")" != "$digest" ]]; then
  rm -f "$partial"
  echo "error: the Claude Code $version download for $platform does not have its pinned digest" >&2
  exit 1
fi
chmod 755 "$partial"
mv "$partial" "$target"
echo "Claude Code $version is at $target"
