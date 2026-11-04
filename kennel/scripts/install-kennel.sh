#!/usr/bin/env bash
# The Kennel's install script (ADR 0141). A release publishes it beside the
# Kennel's archives, stamped with their names and SHA-256 digests
# (kennel/scripts/release.sh package); `just kennel` runs it on the bundle
# it has just built (kennel/scripts/install.sh).
#
# It installs `Kennel.app` into ~/Applications by rename, never by
# overwriting a signed binary in place: it assembles the new bundle beside
# its place, quits a running Kennel, moves the installed one aside and
# renames the new one into place. Then it opens it with `open`, through
# LaunchServices: run directly, the app is refused desktop notices. It
# installs no login agent: start at login is a setting of the Kennel's own,
# off by default.
#
#   bash install-kennel.sh [--no-open]                 the release's Kennel, for this Mac
#   install-kennel.sh --app <Kennel.app> [--no-open]   this bundle, as it is
#
# --no-open stops after the install. It keeps to bash 3.2, which stock
# macOS runs.
set -euo pipefail

# >>> release: written by kennel/scripts/release.sh package
version=''
base_url=''
# One line per target: <Rust target> <archive> <SHA-256>.
archives=''
# <<< release

fail() {
  echo "error: $*" >&2
  exit 1
}

source_app=''
open_it=true
while [[ $# -gt 0 ]]; do
  case "$1" in
    --app)
      source_app=${2:?--app needs the path of a Kennel.app}
      shift 2
      ;;
    --no-open)
      open_it=false
      shift
      ;;
    *) fail "unknown argument $1 (usage: install-kennel.sh [--app <Kennel.app>] [--no-open])" ;;
  esac
done

[[ "$(uname -s)" == Darwin ]] ||
  fail "the Kennel's app is for macOS; on Linux and Windows build and run it from the repository with \`just kennel\`"

applications="$HOME/Applications"
app="$applications/Kennel.app"
mkdir -p "$applications"
# Assembled beside its place, so the rename stays on one file system.
staged=$(mktemp -d "$applications/.Kennel.XXXXXX")
trap 'rm -rf "$staged"' EXIT

if [[ -n "$source_app" ]]; then
  ditto "$source_app" "$staged/Kennel.app"
else
  [[ -n "$archives" ]] ||
    fail "this copy of the install script names no release; use the one a release publishes, or --app"
  case "$(uname -m)" in
    arm64) target=aarch64-apple-darwin ;;
    x86_64) target=x86_64-apple-darwin ;;
    *) fail "no Kennel is built for $(uname -m)" ;;
  esac
  line=$(printf '%s\n' "$archives" | awk -v target="$target" '$1 == target')
  [[ -n "$line" ]] || fail "the Kennel $version has no archive for $target"
  archive=$(printf '%s\n' "$line" | awk '{ print $2 }')
  digest=$(printf '%s\n' "$line" | awk '{ print $3 }')
  echo "Downloading the Kennel $version for $target"
  curl --proto '=https,file' --tlsv1.2 --fail --silent --show-error --location \
    --output "$staged/$archive" "$base_url/$archive"
  actual=$(shasum -a 256 "$staged/$archive" | awk '{ print $1 }')
  [[ "$actual" == "$digest" ]] ||
    fail "$archive's SHA-256 is $actual, not the release's $digest; nothing was installed"
  ditto -x -k "$staged/$archive" "$staged"
  [[ -d "$staged/Kennel.app" ]] || fail "$archive holds no Kennel.app"
fi
codesign --verify --strict "$staged/Kennel.app" ||
  fail "the new Kennel.app's signature does not verify; nothing was installed"

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

if $open_it; then
  open "$app"
  echo "Opened: look for the Kennel's icon in the menu bar"
fi
