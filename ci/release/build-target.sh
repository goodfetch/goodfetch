#!/usr/bin/env bash
# Builds both Products' release executables and shared release components for
# one target triple in one target folder, Collie first (ADR 0094; issue 141):
# - Linux musl targets build as static-PIE with DT_RELR through ci/musl/cargo
#   (zig ld.lld, llvm-tools, committed PGO profile and order file, ADR 0024
#   and spec #2165 / 2026-11-07 amendment), verified by `xtask static-pie`.
# - Both Products' executables build with `--profile release` and split debug
#   symbols beside each executable (`.dwp` on Linux, `.dSYM` on macOS, `.pdb`
#   on Windows).
# - Shared components (`goodfetch-sandbox`, `goodfetch-filter-broker`, and
#   each `goodfetch-plugin-*` crate) build once per target, never once per
#   Product.
#
# Environment:
#   GOODFETCH_PROVING_GROUND_DIGESTS  path to the fixed Proving ground digest
#                                     manifest written before this build runs.
#
# Usage: ci/release/build-target.sh <target> <out-dir>
set -euo pipefail

target=${1:?usage: ci/release/build-target.sh <target> <out-dir>}
out_dir=${2:?usage: ci/release/build-target.sh <target> <out-dir>}
root=$(cd "$(dirname "$0")/../.." && pwd)
cd "$root"

digests=${GOODFETCH_PROVING_GROUND_DIGESTS:?GOODFETCH_PROVING_GROUND_DIGESTS must name the fixed Proving ground digest table}
if [[ ! -f "$digests" ]]; then
  echo "error: Proving ground digest table missing at $digests" >&2
  exit 2
fi

cargo_bin=(cargo)
if [[ "$target" == *-linux-musl ]]; then
  cargo_bin=("$root/ci/musl/cargo")
fi

suffix=""
[[ "$target" == *-windows-* ]] && suffix=.exe

# Build Collie first, then Foxhound, in the same target folder so shared
# crates compile once (ADR 0094).
"${cargo_bin[@]}" build --locked --profile release --target "$target" \
  -p goodfetch-collie --bin goodfetch-collie --features executable
"${cargo_bin[@]}" build --locked --profile release --target "$target" \
  -p goodfetch-foxhound --bin goodfetch-foxhound --features executable

release_dir="target/$target/release"
dest="$out_dir/$target"
mkdir -p "$dest"

copy_debug() {
  local exe=$1
  if [[ "$target" == *-windows-* ]]; then
    local pdb="${exe//-/_}.pdb"
    cp "$release_dir/$pdb" "$dest/$pdb"
  elif [[ "$target" == *-apple-* ]]; then
    cp -R "$release_dir/$exe.dSYM" "$dest/$exe.dSYM"
  else
    cp "$release_dir/$exe.dwp" "$dest/$exe.dwp"
  fi
}

for exe in goodfetch-collie goodfetch-foxhound; do
  cp "$release_dir/$exe$suffix" "$dest/$exe$suffix"
  copy_debug "$exe"
done

# Verify static-PIE + DT_RELR on Linux musl executables (ADR 0024).
if [[ "$target" == *-linux-musl ]]; then
  manifest="$dest/static-pie-files.toml"
  {
    echo 'profile = "release"'
    for product in collie foxhound; do
      printf '\n[[file]]\nproduct = "%s"\nkind = "executable"\ntarget = "%s"\nname = "goodfetch-%s"\npath = "%s"\n' \
        "$product" "$target" "$product" "$dest/goodfetch-$product"
    done
  } >"$manifest"
  cargo run --quiet --locked --bin xtask -- static-pie --target "$target" --files "$manifest"
  rm -f "$manifest"
fi

# Shared helpers built once per target, not once per Product (ADR 0094).
if [[ "$target" == *-linux-* && -f crates/goodfetch-sandbox/Cargo.toml ]]; then
  "${cargo_bin[@]}" build --locked --profile release --target "$target" \
    -p goodfetch-sandbox --bin goodfetch-sandbox
  cp "$release_dir/goodfetch-sandbox" "$dest/goodfetch-sandbox"
fi

if [[ "$target" == *-windows-* && -f crates/goodfetch-filter-broker/Cargo.toml ]]; then
  "${cargo_bin[@]}" build --locked --profile release --target "$target" \
    -p goodfetch-filter-broker --bin goodfetch-filter-broker
  cp "$release_dir/goodfetch-filter-broker.exe" "$dest/goodfetch-filter-broker.exe"
fi

# First-party Plugins built once per target (ADR 0033 and ADR 0094).
for dir in crates/goodfetch-plugin-* crates/foxhound/goodfetch-plugin-* crates/foxhound/sinks/goodfetch-plugin-*; do
  [[ -f "$dir/Cargo.toml" ]] || continue
  plugin=$(basename "$dir")
  "${cargo_bin[@]}" build --locked --profile release --target "$target" \
    -p "$plugin" --bin "$plugin"
  cp "$release_dir/$plugin$suffix" "$dest/$plugin$suffix"
done
