#!/usr/bin/env bash
# Builds the Proving ground binary for one release target with the release
# profile, using ci/musl/cargo for static musl targets (ADRs 0024, 0033, 0039
# and 0094; issue 141). Built first so its Platform signature and digest are
# fixed before the Products' executables build.
#
# Usage: ci/release/build-proving-ground.sh <target> <out-dir>
set -euo pipefail

target=${1:?usage: ci/release/build-proving-ground.sh <target> <out-dir>}
out_dir=${2:?usage: ci/release/build-proving-ground.sh <target> <out-dir>}
root=$(cd "$(dirname "$0")/../.." && pwd)
cd "$root"

cargo_bin=(cargo)
if [[ "$target" == *-linux-musl ]]; then
  cargo_bin=("$root/ci/musl/cargo")
fi

suffix=""
[[ "$target" == *-windows-* ]] && suffix=.exe

"${cargo_bin[@]}" build --locked --profile release --target "$target" \
  -p goodfetch-proving-ground --bin goodfetch-proving-ground

dest="$out_dir/$target"
mkdir -p "$dest"
cp "target/$target/release/goodfetch-proving-ground$suffix" \
  "$dest/goodfetch-proving-ground$suffix"
