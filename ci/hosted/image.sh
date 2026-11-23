#!/usr/bin/env bash
# `ci/hosted/image.sh ready|save`: the hosted tier's CI image on a standard
# hosted runner (#3480).
#
# - `ready` (ci/hosted/start.sh): loads the image the job restored from the
#   Actions cache, whose key holds the Containerfile's hash, then builds it
#   from the tree's ci/image/Containerfile when no archive was restored or
#   the archive holds no image under the tree's tag ($CI_IMAGE), saying why.
#   So a job runs only the image of its own Containerfile, never an older
#   one.
# - `save` (the Release suite's `ci image` job, on `main` only): builds the
#   image and writes it, compressed, where the cache saves it from, so that
#   pull-request jobs restore it (ADR 0061: caches are saved only on `main`).
set -euo pipefail

here=$(cd "$(dirname "$0")" && pwd)
# shellcheck source=ci/hosted/common.sh
source "$here/common.sh"

build() {
  docker build --quiet --platform linux/amd64 --tag "$CI_IMAGE" \
    --file "$hosted_root/ci/image/Containerfile" "$hosted_root/ci/image" >/dev/null
}

case "${1:-}" in
ready)
  if [[ -f "$CI_IMAGE_ARCHIVE" ]]; then
    # The archive goes once loaded: the runner's disk then holds the image
    # once, not twice.
    zstd --decompress --stdout --quiet "$CI_IMAGE_ARCHIVE" | docker load --quiet >/dev/null
    rm -f "$CI_IMAGE_ARCHIVE"
    if docker image inspect "$CI_IMAGE" >/dev/null 2>&1; then
      echo "Loaded $CI_IMAGE from the Actions cache that main saved."
      exit 0
    fi
    echo "The cached archive holds no $CI_IMAGE; building it from the tree's ci/image/Containerfile."
  else
    echo "The Actions cache holds no $CI_IMAGE, which the Release suite saves on main; building it from the tree's ci/image/Containerfile."
  fi
  build
  ;;
save)
  build
  mkdir -p "$(dirname "$CI_IMAGE_ARCHIVE")"
  docker save "$CI_IMAGE" | zstd --quiet -T0 -3 -o "$CI_IMAGE_ARCHIVE" --force
  echo "Saved $CI_IMAGE: $(stat --format %s "$CI_IMAGE_ARCHIVE") bytes compressed."
  ;;
*)
  echo "usage: ci/hosted/image.sh ready|save" >&2
  exit 2
  ;;
esac
