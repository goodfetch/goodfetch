#!/usr/bin/env bash
# The Kennel's checks (ADR 0141), `just kennel-check`, the gate's `kennel`
# step: the module and size check of its crates (ADR 0126, through the
# root's `xtask modules`, which reads their source and builds nothing) and
# its tests. The Client's tests run goodfetch-scripted-engine as a process
# (ADR 0069), and the Run controls' test runs a real Daemon of
# goodfetch-collie; the root workspace builds both here with the tests'
# Cargo profile, and the tests find them through a test tool manifest of
# their own.
#
#   kennel/scripts/check.sh <cargo profile>
#
# Its Clippy is `just lint`'s, which every gate runs: `ci/clippy-excluded.sh
# kennel` lints the whole workspace, for this host and, from another host,
# for macOS, into the root's target folder, so no gate lints it twice
# (#3217).
#
# On macOS it tests every crate. Elsewhere the graphical crate,
# goodfetch-kennel-app, needs a display to run, which the display-less
# Linux gate does not have, so it tests the non-graphical crate,
# goodfetch-kennel.
set -euo pipefail

root=$(cd "$(dirname "$0")/../.." && pwd)
cd "$root"
profile=${1:?usage: kennel/scripts/check.sh <cargo profile>}

cargo run --quiet --locked --bin xtask -- modules --workspace kennel
# The two executables and their manifest, as every test's: Collie's needs
# its `executable` feature (ADR 0094).
cargo run --quiet --locked --bin xtask -- test-tools build --cargo-profile "$profile" \
  --tool scripted_engine --tool collie --out "$root/target/kennel-test-tools.json"
export GOODFETCH_TEST_TOOLS="$root/target/kennel-test-tools.json"

# From its own folder, so rustup takes the Kennel's own toolchain pin.
cd kennel
if [[ "$(uname -s)" == Darwin ]]; then
  cargo test --locked --workspace
else
  cargo test --locked -p goodfetch-kennel
fi
