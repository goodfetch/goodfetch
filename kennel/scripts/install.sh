#!/usr/bin/env bash
# `just kennel` (ADR 0141): builds the Kennel from the repository and starts
# it.
#
# On macOS it builds the release executable, assembles `Kennel.app` with
# `kennel/Info.plist` under the Kennel's own version (bundle identifier
# `dev.goodfetch.kennel`, kept stable so a rebuild keeps the desktop-notice
# permission) and signs it ad hoc (bundle.sh). Then the install script a
# release publishes (install-kennel.sh) installs it by rename into the
# user's Applications folder (never by overwriting a signed binary in
# place), quitting a running Kennel first, and opens it with `open`,
# through LaunchServices: run directly, the same app is refused desktop
# notices (#1915). On Linux and Windows it builds and runs the executable.
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

# Assembled in the Kennel's target folder, then installed by the script a
# release publishes.
bundles="$root/kennel/target/bundle"
rm -rf "$bundles"
"$root/kennel/scripts/bundle.sh" "$binary" "$bundles"
arguments=(--app "$bundles/Kennel.app")
$open_it || arguments+=(--no-open)
"$root/kennel/scripts/install-kennel.sh" "${arguments[@]}"
codesign --display --verbose=2 "$HOME/Applications/Kennel.app" 2>&1 | grep -E '^(Identifier|Signature)='
