#!/usr/bin/env bash
# Assembles the Kennel's `Kennel.app` (ADR 0141) from its executable, for
# `just kennel` (install.sh) and the release (release.sh build).
#
# The bundle holds the executable and kennel/Info.plist, stamped with the
# Kennel's own version, `[workspace.package]`'s in kennel/Cargo.toml, never
# the Products'. Its bundle identifier, `dev.goodfetch.kennel`, stays the
# same across builds, so a rebuild keeps the desktop-notice permission. It
# is signed ad hoc and verified; a release signs it again with Developer ID
# (release.sh sign).
#
#   kennel/scripts/bundle.sh <executable> <folder>   writes <folder>/Kennel.app
#
# It keeps to bash 3.2, which stock macOS runs.
set -euo pipefail

root=$(cd "$(dirname "$0")/../.." && pwd)
executable=${1:?usage: kennel/scripts/bundle.sh <executable> <folder>}
folder=${2:?usage: kennel/scripts/bundle.sh <executable> <folder>}

version=$(sed -n '/^\[workspace\.package\]/,/^\[/s/^version = "\(.*\)"$/\1/p' "$root/kennel/Cargo.toml")
if [[ -z "$version" ]]; then
  echo "error: kennel/Cargo.toml's [workspace.package] names no version" >&2
  exit 1
fi

app="$folder/Kennel.app"
if [[ -e "$app" ]]; then
  echo "error: $app already exists" >&2
  exit 1
fi
mkdir -p "$app/Contents/MacOS"
cp "$executable" "$app/Contents/MacOS/goodfetch-kennel"
cp "$root/kennel/Info.plist" "$app/Contents/Info.plist"
plutil -replace CFBundleShortVersionString -string "$version" "$app/Contents/Info.plist"
plutil -replace CFBundleVersion -string "$version" "$app/Contents/Info.plist"
codesign --sign - --force "$app"
codesign --verify --strict "$app"
