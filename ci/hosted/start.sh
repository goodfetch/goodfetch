#!/usr/bin/env bash
# Starts a hosted tier job's container on its standard `ubuntu-24.04` runner
# (ADR 0135's amendment of 2026-11-22): builds the tree's pinned Linux x86-64
# CI image (ci/image/Containerfile), starts the job's container from it with
# the workspace at /work, then makes every download the job needs
# (ci/vm/in-container.sh prepare), all before ADR 0043's deny. No cache is
# saved or restored here: only `main`'s cache-warming job will save one.
#
# CAP_SYS_ADMIN lets the container's default seccomp profile allow `unshare`
# and `mount`, so `DnsNamespace` makes its user, network and mount
# namespaces there as the container's root (ADR 0067), as in the owner's
# machine's VM (ci/vm/start.sh).
set -euo pipefail

here=$(cd "$(dirname "$0")" && pwd)
# shellcheck source=ci/hosted/common.sh
source "$here/common.sh"
workspace=${GITHUB_WORKSPACE:-$(pwd)}

rm -f "$EGRESS_STATE"
docker build --quiet --platform linux/amd64 --tag "$CI_IMAGE" \
  --file "$hosted_root/ci/image/Containerfile" "$hosted_root/ci/image" >/dev/null
docker rm --force "$CONTAINER" >/dev/null 2>&1 || true
docker run --detach --name "$CONTAINER" --platform linux/amd64 \
  --cap-add NET_ADMIN --cap-add SYS_ADMIN \
  --volume "$workspace:/work" \
  --env CI=true --env GOODFETCH_TEST_DNS_NAMESPACE=user_namespace \
  "$CI_IMAGE" sleep infinity >/dev/null
docker exec "$CONTAINER" /work/ci/vm/in-container.sh prepare
echo "The job's container runs $CI_IMAGE, built from the tree's ci/image/Containerfile."
