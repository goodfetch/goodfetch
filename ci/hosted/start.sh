#!/usr/bin/env bash
# Starts a hosted tier job's container on its standard `ubuntu-24.04` runner
# (ADR 0135's amendment of 2026-11-22): builds the tree's pinned Linux x86-64
# CI image (ci/image/Containerfile), starts the job's container from it with
# the workspace at /work, then makes every download the job needs
# (ci/vm/in-container.sh prepare), all before ADR 0043's deny. No cache is
# saved or restored here: only `main`'s cache-warming job will save one.
#
# The container boots systemd as its init, so ci/os-services.sh starts the
# user's manager there, as on the gate host: `daemon install`'s tests and
# Collie's governor test need it (#3380). systemd needs a writable cgroup
# tree of its own, which a privileged container in a private cgroup
# namespace gets, as kind runs its nodes; that also lets `DnsNamespace` make
# its user, network and mount namespaces as the container's root (ADR 0067)
# and gives `tc` CAP_NET_ADMIN. /run is a tmpfs, as systemd expects.
#
# The boot is awaited from the runner, in the container's mount and PID
# namespaces only, never through `docker exec`: a command `docker exec`
# starts joins the container's root cgroup while systemd has enabled no
# controller there, and a process in it keeps systemd from enabling any, so
# no scope of the user's manager could then limit its memory. Once systemd
# has enabled them, `docker exec`'s commands join its init.scope instead,
# from which the user's manager can move them into scopes of their own.
set -euo pipefail

here=$(cd "$(dirname "$0")" && pwd)
# shellcheck source=ci/hosted/common.sh
source "$here/common.sh"
workspace=${GITHUB_WORKSPACE:-$(pwd)}

# The tests' copy of the pinned Chrome for Testing build, at the runner's
# path the job restored it to from the cache the Release suite saves on
# `main` (#3380); `just fetch` downloads it there when the cache missed.
chrome_for_testing="$HOME/.cache/goodfetch/chrome-for-testing"
mkdir -p "$chrome_for_testing"

rm -f "$EGRESS_STATE"
docker build --quiet --platform linux/amd64 --tag "$CI_IMAGE" \
  --file "$hosted_root/ci/image/Containerfile" "$hosted_root/ci/image" >/dev/null
docker rm --force "$CONTAINER" >/dev/null 2>&1 || true
docker run --detach --name "$CONTAINER" --platform linux/amd64 \
  --privileged --cgroupns private --tmpfs /run --tmpfs /run/lock \
  --volume "$workspace:/work" \
  --volume "$chrome_for_testing:/var/cache/chrome-for-testing" \
  --env GOODFETCH_CHROME_FOR_TESTING_DOWNLOADS=/var/cache/chrome-for-testing \
  --env CI=true --env GOODFETCH_TEST_DNS_NAMESPACE=user_namespace \
  "$CI_IMAGE" /sbin/init >/dev/null
# systemd's own socket comes first; then the boot ends, `degraded` when a
# unit a container cannot run (loading kernel modules) failed.
pid=$(docker inspect --format '{{.State.Pid}}' "$CONTAINER")
state=
if timeout 120 bash -c "until sudo test -S /proc/$pid/root/run/systemd/private; do sleep 0.2; done"; then
  state=$(timeout 120 sudo nsenter --target "$pid" --mount --pid -- systemctl is-system-running --wait) || true
fi
if [[ "$state" != running && "$state" != degraded ]]; then
  echo "error: systemd did not boot in the job's container (${state:-no answer})" >&2
  docker logs "$CONTAINER" >&2 || true
  docker exec "$CONTAINER" systemctl --failed --no-pager >&2 || true
  exit 1
fi
docker exec "$CONTAINER" /work/ci/vm/in-container.sh prepare
echo "The job's container runs $CI_IMAGE, built from the tree's ci/image/Containerfile."
