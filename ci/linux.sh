#!/usr/bin/env bash
# `just linux`: the Pre-merge suite's tests that only Linux decides, for a
# host that is not Linux (ADR 0135's note of 2026-10-20). CI's Pre-merge suite
# runs on Linux; the local gate's host steps run on macOS, which compiles,
# skips or asserts these tests differently:
#   - `tc_netem::` and `os_services::keychain::`, compiled only on Linux;
#   - `fetch_edge::the_ipv6_path_drops_connections_while_ipv4_answers`, whose
#     IPv6 Happy Eyeballs check asserts the dropped SYN only on Linux;
#   - `extra_loopback::`, skipped on a macOS host without the `lo0` aliases,
#     while Linux answers on all of 127.0.0.0/8.
# A test that only Linux decides joins `filter` below.
#
#   - Elsewhere than Linux it runs this script in the pinned Linux x86-64 CI
#     image (ci/image/Containerfile) through Docker or Podman, with
#     CAP_NET_ADMIN for `tc netem`, as ci/vm/start.sh gives the job's
#     container; `ci/os-services.sh setup` starts every Linux OS service
#     there but the user's service manager, and a missing one fails, as in CI.
#   - On Linux the Pre-merge suite runs these tests on the host itself,
#     after ci/pre-merge.sh's `ci/os-services.sh setup`, where a missing
#     service fails instead of skipping them.
#   - Without a container engine it lists itself as skipped with the reason
#     (ADR 0061), as ci/perf.sh does.
set -euo pipefail

root=$(cd "$(dirname "$0")/.." && pwd)
cd "$root"

# shellcheck source=ci/container.sh
source "$root/ci/container.sh"

# The tests, as a nextest filterset over the packages that hold them.
packages=(-p goodfetch-proving-ground -p xtask)
filter='test(/^tc_netem::/) | test(/^extra_loopback::/) | test(/^os_services::keychain::/)'
filter+=' | test(=fetch_edge::the_ipv6_path_drops_connections_while_ipv4_answers)'

if [[ "${1:-}" == in-container ]]; then
  # Without the user's service manager: this container does not boot
  # systemd, and none of the tests above install a service.
  ci/os-services.sh setup --without-service-manager
  set -a
  # shellcheck source=/dev/null
  source "${RUNNER_TEMP:-/tmp}/goodfetch-os-services/env"
  set +a
  # Without IPv6 loopback the Happy Eyeballs check passes without
  # connecting, so its absence fails here instead.
  if ! ip -6 address show dev lo | grep -q 'inet6 ::1/128'; then
    echo "error: the container has no IPv6 loopback, which the Happy Eyeballs check needs" >&2
    exit 1
  fi
  export GOODFETCH_TEST_SUITE=pre-merge
  # The tests on the test quarantine list are skipped, as in every suite
  # (ADR 0064).
  skips=()
  exclusions=$(cargo run --quiet --locked --bin xtask -- quarantine exclude)
  quarantined=$(jq -r '.nextest[]' <<<"$exclusions")
  while IFS= read -r argument; do
    [[ -z "$argument" ]] || skips+=("$argument")
  done <<<"$quarantined"
  exec cargo nextest run "${packages[@]}" --locked --profile ci --cargo-profile test \
    -E "$filter" ${skips[@]+"${skips[@]}"}
fi

if [[ "$(uname -s)" == Linux ]]; then
  echo "just linux: the Pre-merge suite runs these tests on this Linux host itself"
  exit 0
fi

engine=$(container_engine)
[[ -n "$engine" ]] || skip "just linux" "its tests need Linux, and no container engine (Docker or Podman) is running on this host"
# shellcheck source=ci/vm/common.sh
source "$root/ci/vm/common.sh"
if ! "$engine" image inspect "$CI_IMAGE" >/dev/null 2>&1; then
  echo "Building the CI image $CI_IMAGE once (ci/image/Containerfile)…" >&2
  "$engine" build --platform linux/amd64 --tag "$CI_IMAGE" --file ci/image/Containerfile ci/image
fi
# As ci/perf.sh mounts it, with build outputs and Cargo's registry in
# volumes of their own: perf's run beside it, so sharing them would make
# each wait for the other's Cargo lock.
mounts=(--volume "$root:$root")
common=$(git rev-parse --path-format=absolute --git-common-dir)
[[ "$common" == "$root"/* ]] || mounts+=(--volume "$common:$common")
environment=(--env GIT_CONFIG_COUNT=1 --env GIT_CONFIG_KEY_0=safe.directory --env GIT_CONFIG_VALUE_0='*')
[[ -z "${CARGO_BUILD_JOBS:-}" ]] || environment+=(--env "CARGO_BUILD_JOBS=$CARGO_BUILD_JOBS")
exec "$engine" run --rm --init --platform linux/amd64 --cap-add NET_ADMIN "${mounts[@]}" --workdir "$root" \
  --volume goodfetch-linux-target:/var/tmp/target --env CARGO_TARGET_DIR=/var/tmp/target \
  --volume goodfetch-linux-cargo:/usr/local/cargo/registry \
  "${environment[@]}" "$CI_IMAGE" ci/linux.sh in-container
