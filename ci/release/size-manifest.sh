#!/usr/bin/env bash
# Prints the manifest `xtask size` reads for one target of a Release cell:
# each Product's executable that the `ci` profile built (ADRs 0057, 0087 and
# 0094), named after its executable crate, `goodfetch-collie` and
# `goodfetch-foxhound`. Until those crates exist the manifest lists no file
# and the report is empty. Compressed release files and OCI images join the
# manifest with the release jobs that build them.
#   ci/release/size-manifest.sh <target>
set -euo pipefail

target=${1:?usage: ci/release/size-manifest.sh <target>}
suffix=""
[[ "$target" == *-windows-* ]] && suffix=.exe

echo 'profile = "ci"'
for product in collie foxhound; do
  path="target/$target/ci/goodfetch-$product$suffix"
  [[ -f "$path" ]] || continue
  printf '\n[[file]]\nproduct = "%s"\nkind = "executable"\ntarget = "%s"\nname = "goodfetch-%s%s"\npath = "%s"\n' \
    "$product" "$target" "$product" "$suffix" "$path"
done
