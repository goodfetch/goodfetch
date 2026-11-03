#!/usr/bin/env bash
# `just kennel` (ADR 0141): builds the Kennel from the repository and starts
# it.
#
# On macOS it builds the release executable, assembles `Kennel.app` with
# `kennel/Info.plist` (bundle identifier `dev.goodfetch.kennel`, kept
# stable so a rebuild keeps the desktop-notice permission), signs it ad
# hoc, installs it by rename into the user's Applications folder (never by
# overwriting a signed binary in place), and opens it with `open`, through
# LaunchServices: run directly, the same app is refused desktop notices
# (#1915). A Kennel already running is quit first. On Linux and Windows it
# builds and runs the executable.
#
#   kennel/scripts/install.sh [--no-open]   --no-open stops after the install
set -euo pipefail

root=$(cd "$(dirname "$0")/../.." && pwd)
open_it=true
[[ "${1:-}" == --no-open ]] && open_it=false

# From its own folder, so rustup takes the Kennel's own toolchain pin.
cd "$root/kennel"
cargo build --locked --release -p goodfetch-kennel-app
binary="$root/kennel/target/release/goodfetch-kennel"
[[ -f "$binary.exe" ]] && binary="$binary.exe"
echo "Executable: $(wc -c <"$binary" | tr -d ' ') bytes (stripped, LTO)"

if [[ "$(uname -s)" != Darwin ]]; then
  $open_it || exit 0
  exec "$binary"
fi

applications="$HOME/Applications"
app="$applications/Kennel.app"
mkdir -p "$applications"
# Assembled beside its place, so the rename stays on one file system.
staged=$(mktemp -d "$applications/.Kennel.XXXXXX")
trap 'rm -rf "$staged"' EXIT
mkdir -p "$staged/Kennel.app/Contents/MacOS"
cp "$binary" "$staged/Kennel.app/Contents/MacOS/goodfetch-kennel"
cp "$root/kennel/Info.plist" "$staged/Kennel.app/Contents/Info.plist"
codesign --sign - --force "$staged/Kennel.app"
codesign --verify --strict "$staged/Kennel.app"

if pgrep -x goodfetch-kennel >/dev/null; then
  echo "Quitting the running Kennel"
  pkill -x goodfetch-kennel || true
  while pgrep -x goodfetch-kennel >/dev/null; do sleep 0.2; done
fi
if [[ -e "$app" ]]; then
  mv "$app" "$staged/Kennel.old.app"
fi
mv "$staged/Kennel.app" "$app"
echo "Installed $app"
codesign --display --verbose=2 "$app" 2>&1 | grep -E '^(Identifier|Signature)='

if $open_it; then
  open "$app"
  echo "Opened: look for the Kennel's icon in the menu bar"
fi
