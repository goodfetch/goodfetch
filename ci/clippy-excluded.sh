#!/usr/bin/env bash
# Clippy, with `-D warnings`, on each first-party crate the root workspace's
# `exclude` list holds, which `just lint`'s `clippy` step (`--workspace`)
# never reaches (#2980): the Kennel's workspace (ADR 0141) and the library
# Foxhound's tests use. The forks under `vendor/` are third-party code, not
# linted. `just lint`'s `clippy-excluded` step.
#
#   ci/clippy-excluded.sh           every such crate, one keep-going step each,
#                                   named `clippy-<folder>`, so a red one names it
#   ci/clippy-excluded.sh <folder>  that one crate, as its step runs it
#
# Each crate is linted from its own folder, so rustup takes its own toolchain
# pin, into the root's target folder, which keeps it warm and in reach of
# `xtask sweep-target`. One with a lockfile of its own is linted with it,
# `--locked`. One without, a path dependency of a member that the root
# lockfile resolves, reads a fresh copy of the root lockfile under target/,
# offline, so it takes the versions the root locks and writes no lockfile
# into the source tree.
#
# This is the Kennel's only Clippy: `just kennel-check` tests it and lints
# none of it (#3217). On a host other than macOS it is also linted for macOS
# (aarch64-apple-darwin), so its macOS-only code is, which needs the Rust
# target and no macOS SDK. On Linux its graphical crate, goodfetch-kennel-app,
# needs system libraries that kennel/scripts/prerequisites.sh probes for.
# Without the target or those libraries, that lint leaves out what it cannot
# build and lists it as skipped on a local host, and the step fails in CI
# (ADR 0061).
set -euo pipefail

root=$(pwd -P)
# shellcheck source=kennel/scripts/prerequisites.sh
source "$(dirname "${BASH_SOURCE[0]}")/../kennel/scripts/prerequisites.sh"

lint() {
  local folder=$1 lock narrowed=() missing
  if [[ -f $folder/Cargo.lock ]]; then
    lock=(--locked)
  else
    local copy=$root/target/clippy-excluded/$folder/Cargo.lock
    mkdir -p "$(dirname "$copy")"
    cp Cargo.lock "$copy"
    lock=(--offline --config "resolver.lockfile-path=\"$copy\"")
  fi
  if [[ $folder == kennel && "$(uname -s)" == Linux ]]; then
    missing=$(kennel_linux_libraries_missing)
    if [[ -n $missing ]]; then
      kennel_skip "$root" "ci/clippy-excluded.sh $folder" \
        "Clippy on the Kennel's graphical crate goodfetch-kennel-app for this host, since $missing"
      narrowed=(--exclude goodfetch-kennel-app)
    fi
  fi
  cd "$folder"
  cargo clippy "${lock[@]}" ${narrowed[@]+"${narrowed[@]}"} --target-dir "$root/target" \
    --workspace --all-targets --all-features -- -D warnings
  if [[ $folder == kennel && "$(uname -s)" != Darwin ]]; then
    missing=$(kennel_cross_missing)
    if [[ -n $missing ]]; then
      kennel_skip "$root" "ci/clippy-excluded.sh $folder" "Clippy on the Kennel for macOS, since $missing"
    else
      cargo clippy "${lock[@]}" --target "$kennel_cross" --target-dir "$root/target" \
        --workspace --all-targets --all-features -- -D warnings
    fi
  fi
}

if [[ $# -gt 0 ]]; then
  lint "$1"
  exit
fi

excluded=$(uv run --frozen --quiet python -c "import tomllib; print('\n'.join(tomllib.load(open('Cargo.toml', 'rb'))['workspace'].get('exclude', [])))")
# shellcheck source=ci/keep-going.sh
source "$(dirname "${BASH_SOURCE[0]}")/keep-going.sh"
steps_begin clippy-excluded
while IFS= read -r folder; do
  case $folder in
    '' | vendor/*) continue ;;
  esac
  step "clippy-$(basename "$folder")" "bash $(printf '%q' "$0") $(printf '%q' "$folder")"
done <<<"$excluded"
steps_end
