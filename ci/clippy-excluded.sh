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
# On Linux the Kennel's graphical crate, goodfetch-kennel-app, needs the
# development files of GTK 3 and libdbus, found by pkg-config, as
# `just kennel-check` does (kennel/scripts/check.sh): without them it is left
# out and listed as skipped on a local host, and the step fails in CI (ADR
# 0061).
set -euo pipefail

root=$(pwd -P)

lint() {
  local folder=$1 name lock narrowed=() missing=() library
  name=$(basename "$folder")
  if [[ -f $folder/Cargo.lock ]]; then
    lock=(--locked)
  else
    local copy=$root/target/clippy-excluded/$name/Cargo.lock
    mkdir -p "$(dirname "$copy")"
    cp Cargo.lock "$copy"
    lock=(--offline --config "resolver.lockfile-path=\"$copy\"")
  fi
  if [[ $folder == kennel && "$(uname -s)" == Linux ]]; then
    for library in gtk+-3.0 dbus-1; do
      pkg-config --exists "$library" || missing+=("$library")
    done
    if [[ ${#missing[@]} -gt 0 ]]; then
      local reason="the Kennel's graphical crate goodfetch-kennel-app, since pkg-config finds no ${missing[*]}: GTK 3 (gtk+-3.0) is what its Tray builds on, libdbus (dbus-1) what its desktop notices send through (Debian and Ubuntu: apt install libgtk-3-dev libdbus-1-dev)"
      if [[ -n "${CI:-}" ]]; then
        echo "error: Clippy cannot lint $reason, and CI runs every part" >&2
        return 1
      fi
      mkdir -p "$root/target"
      echo "ci/clippy-excluded.sh $folder: skipped, $reason" >>"$root/target/skipped.txt"
      echo "skipped: Clippy on $reason" >&2
      narrowed=(--exclude goodfetch-kennel-app)
    fi
  fi
  cd "$folder"
  cargo clippy "${lock[@]}" ${narrowed[@]+"${narrowed[@]}"} --target-dir "$root/target" \
    --workspace --all-targets --all-features -- -D warnings
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
