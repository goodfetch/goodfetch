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
# skipped with the reason on a local host, and fails in CI
# (kennel/scripts/prerequisites.sh).
set -euo pipefail

root=$(cd "$(dirname "$0")/../.." && pwd)
cd "$root"
profile=${1:?usage: kennel/scripts/check.sh <cargo profile>}
# shellcheck source=kennel/scripts/prerequisites.sh
source "$root/kennel/scripts/prerequisites.sh"

skip() {
  kennel_skip "$root" kennel/scripts/check.sh "$1"
}

cargo run --quiet --locked --bin xtask -- modules --workspace kennel
# The two executables and their manifest, as every test's: Collie's needs
# its `executable` feature (ADR 0094).
cargo run --quiet --locked --bin xtask -- test-tools build --cargo-profile "$profile" \
  --tool scripted_engine --tool collie --out "$root/target/kennel-test-tools.json"
export GOODFETCH_TEST_TOOLS="$root/target/kennel-test-tools.json"

# From its own folder, so rustup takes the Kennel's own toolchain pin.
cd kennel
if [[ "$(uname -s)" == Darwin ]]; then
  cargo clippy --locked --workspace --all-targets --all-features -- -D warnings
  cargo test --locked --workspace
else
  cargo clippy --locked -p goodfetch-kennel --all-targets --all-features -- -D warnings
  cargo test --locked -p goodfetch-kennel
  missing=$(kennel_cross_missing)
  if [[ -z $missing ]]; then
    cargo clippy --locked -p goodfetch-kennel-app --target "$kennel_cross" \
      --all-targets --all-features -- -D warnings
  else
    skip "the graphical crate's type-check for macOS, since $missing"
  fi
  if [[ "$(uname -s)" == Linux ]]; then
    missing=$(kennel_linux_libraries_missing)
    if [[ -z $missing ]]; then
      cargo clippy --locked -p goodfetch-kennel-app --all-targets --all-features -- -D warnings
    else
      skip "the graphical crate's type-check for Linux, since $missing"
    fi
  fi
fi
