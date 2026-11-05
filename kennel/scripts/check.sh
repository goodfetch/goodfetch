#!/usr/bin/env bash
# The Kennel's checks (ADR 0141), `just kennel-check`, the gate's `kennel`
# step: the module and size check of its crates (ADR 0126, through the
# root's `xtask modules`, which reads their source and builds nothing),
# its workspace's Clippy, with `-D warnings`, and its tests. The
# Client's tests run goodfetch-scripted-engine as a process (ADR 0069), and
# the Run controls' test runs a real Daemon of goodfetch-collie; the root
# workspace builds both here with the tests' Cargo profile, and the tests
# find them through a test tool manifest of their own.
#
#   kennel/scripts/check.sh <cargo profile>
#
# On macOS it checks and tests every crate. Elsewhere the graphical crate,
# goodfetch-kennel-app, needs a display to run, which the display-less
# Linux gate does not have, so it checks and tests the non-graphical crate,
# goodfetch-kennel, and type-checks the graphical one:
#   - for macOS (aarch64-apple-darwin), as #1915 did;
#   - on Linux, for this host, which reaches the Linux Tray's GTK thread
#     (#2077); that type-check needs the development files of GTK 3, which
#     the Tray builds on, and of libdbus, which desktop notices send
#     through, found by pkg-config (Debian and Ubuntu:
#     `apt install libgtk-3-dev libdbus-1-dev`), and no display.
#     AppIndicator's library is loaded at run time, so it needs none.
# Without the Rust target or those files, that type-check is listed as
# skipped with the reason on a local host, and fails in CI.
set -euo pipefail

root=$(cd "$(dirname "$0")/../.." && pwd)
cd "$root"
profile=${1:?usage: kennel/scripts/check.sh <cargo profile>}
cross=aarch64-apple-darwin

skip() {
  if [[ -n "${CI:-}" ]]; then
    echo "error: $1, and CI runs every part" >&2
    exit 1
  fi
  mkdir -p "$root/target"
  echo "kennel/scripts/check.sh: skipped, $1" >>"$root/target/skipped.txt"
  echo "skipped: kennel/scripts/check.sh ($1)" >&2
}

# Collie's executable embeds its Console build, made first (ADR 0057), and
# needs its `executable` feature (ADR 0094), as `xtask test-tools` builds it.
cargo run --quiet --locked --bin xtask -- console build
cargo run --quiet --locked --bin xtask -- modules --workspace kennel
built=$(cargo build --quiet --locked --profile "$profile" -p goodfetch-test-support \
  --bin goodfetch-scripted-engine -p goodfetch-collie --bin goodfetch-collie \
  --features executable --message-format json)
executable() {
  jq -r --arg name "$1" 'select(.reason == "compiler-artifact" and .target.name == $name) | .executable' <<<"$built"
}
mkdir -p target
jq -n --arg engine "$(executable goodfetch-scripted-engine)" --arg collie "$(executable goodfetch-collie)" \
  '{scripted_engine: $engine, collie: $collie}' >target/kennel-test-tools.json
export GOODFETCH_TEST_TOOLS="$root/target/kennel-test-tools.json"

# From its own folder, so rustup takes the Kennel's own toolchain pin.
cd kennel
if [[ "$(uname -s)" == Darwin ]]; then
  cargo clippy --locked --workspace --all-targets --all-features -- -D warnings
  cargo test --locked --workspace
else
  cargo clippy --locked -p goodfetch-kennel --all-targets --all-features -- -D warnings
  cargo test --locked -p goodfetch-kennel
  if rustup target list --installed | grep -qx "$cross"; then
    cargo clippy --locked -p goodfetch-kennel-app --target "$cross" \
      --all-targets --all-features -- -D warnings
  else
    skip "the graphical crate's type-check for macOS, since the Rust target $cross is not installed (rustup target add $cross)"
  fi
  if [[ "$(uname -s)" == Linux ]]; then
    missing=()
    for library in gtk+-3.0 dbus-1; do
      pkg-config --exists "$library" || missing+=("$library")
    done
    if [[ ${#missing[@]} -eq 0 ]]; then
      cargo clippy --locked -p goodfetch-kennel-app --all-targets --all-features -- -D warnings
    else
      skip "the graphical crate's type-check for Linux, since pkg-config finds no ${missing[*]}: GTK 3 (gtk+-3.0) is what its Tray builds on, libdbus (dbus-1) what its desktop notices send through (Debian and Ubuntu: apt install libgtk-3-dev libdbus-1-dev)"
    fi
  fi
fi
