#!/usr/bin/env bash
# `xtask perf` in a Linux gnu cell of the Release suite (ADR 0071: the merge
# queue adds Linux arm64), on the runner's own system, after
# .github/actions/host-tools and ci/host/perf-tools.sh. The base is the
# tested commit's first parent: `main` before the commit, as the group's
# base is in the merge queue.
#   build         before the deny: the base's and the commit's bench targets
#                 that `xtask affected` selects;
#   pull-request  before the deny: the "Performance change" section of the
#                 pull request that merged the commit, if one did;
#   run           under the deny (ci/egress/under-deny.sh): measures and
#                 compares, writing the table to the job's summary page and
#                 the figures beside the JUnit files, which the results store
#                 keeps for the drift check.
set -euo pipefail

part=${1:?usage: ci/release/perf.sh build|pull-request|run}
root=$(cd "$(dirname "$0")/../.." && pwd)
cd "$root"
# shellcheck source=ci/output-value.sh
source "$root/ci/output-value.sh"

xtask() {
  cargo run --quiet --locked --bin xtask -- "$@"
}

case "$part" in
  build)
    selection=$(xtask affected --base HEAD^1 --head HEAD)
    targets=$(output_value bench-targets "$selection")
    xtask perf build --base HEAD^1 --bench-targets "$targets"
    ;;
  pull-request)
    xtask perf pull-request --commit "$(git rev-parse HEAD)" \
      --repository "${GITHUB_REPOSITORY:?the job sets GITHUB_REPOSITORY}" \
      --save target/perf-pull-request.json
    ;;
  run)
    xtask perf run --pull-request target/perf-pull-request.json \
      --summary "${GITHUB_STEP_SUMMARY:?the job sets GITHUB_STEP_SUMMARY}"
    ;;
  *)
    echo "error: the part is build, pull-request or run, not $part" >&2
    exit 2
    ;;
esac
