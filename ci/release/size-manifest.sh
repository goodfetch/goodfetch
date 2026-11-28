#!/usr/bin/env bash
# Prints the manifest `xtask size` reads for one target of a Release cell or
# pull-request size estimate: each Product's `ci` executable, compressed
# release archive, published crate tarballs, and Linux OCI image tags (ADRs
# 0057, 0087 and 0094; issues 201 and 204).
#   ci/release/size-manifest.sh <target>
set -euo pipefail

target=${1:?usage: ci/release/size-manifest.sh <target>}
suffix=""
[[ "$target" == *-windows-* ]] && suffix=.exe

echo 'profile = "ci"'
for product in collie foxhound; do
  path="target/$target/ci/goodfetch-$product$suffix"
  if [[ -f "$path" ]]; then
    printf '\n[[file]]\nproduct = "%s"\nkind = "executable"\ntarget = "%s"\nname = "goodfetch-%s%s"\npath = "%s"\n' \
      "$product" "$target" "$product" "$suffix" "$path"
  fi

  archive="target/release-suite/archives/goodfetch-$product-$target.tar.xz"
  if [[ -f "$archive" ]]; then
    printf '\n[[file]]\nproduct = "%s"\nkind = "release-archive"\ntarget = "%s"\nname = "goodfetch-%s-%s.tar.xz"\npath = "%s"\n' \
      "$product" "$target" "$product" "$target" "$archive"
  fi

  for tag in default slim full node python; do
    oci="target/release-suite/oci/$product-$tag.manifest.json"
    if [[ -f "$oci" ]]; then
      printf '\n[[file]]\nproduct = "%s"\nkind = "oci-image"\ntarget = "%s"\nname = "%s-%s"\npath = "%s"\n' \
        "$product" "$target" "$product" "$tag" "$oci"
    fi
  done
done

shopt -s nullglob
for crate in target/release-suite/crates/*.crate; do
  name=$(basename "$crate")
  product="collie"
  if [[ "$name" == goodfetch-foxhound-* ]]; then
    product="foxhound"
  fi
  printf '\n[[file]]\nproduct = "%s"\nkind = "crate"\nname = "%s"\npath = "%s"\n' \
    "$product" "$name" "$crate"
done
