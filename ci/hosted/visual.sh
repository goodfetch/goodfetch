#!/usr/bin/env bash
# Runs the visual projects on a standard hosted `ubuntu-24.04` runner, inside
# the pinned Playwright image (ADR 0073), beside the job's container that
# ci/hosted/start.sh started (#3383):
#   ci/hosted/visual.sh install  starts the image's container for the job and
#                                installs the packages, before ADR 0043's deny
#   ci/hosted/visual.sh test     compares every Reference screenshot (the
#                                Release suite's visual job)
#   ci/hosted/visual.sh update   rewrites the changed ones
# The image's container joins the job container's network namespace, so the
# deny ci/egress/egress.sh sets there holds it too. It runs
# ci/visual/run.sh on the workspace at /work; under the deny, pnpm runs
# offline. The pinned image has no Rust, so `install` also writes the test
# quarantine list's exclusions (`xtask quarantine exclude`, in the job's
# container) to target/visual/quarantine-<runner>.txt, one argument a line,
# which `test` reads (ADR 0064).
set -euo pipefail

mode=${1:?usage: ci/hosted/visual.sh install|test|update}
here=$(cd "$(dirname "$0")" && pwd)
# shellcheck source=ci/hosted/common.sh
source "$here/common.sh"
workspace=${GITHUB_WORKSPACE:-$(pwd)}

visual_container=goodfetch-visual
image_key() {
  sed -n "s/^$1 = \"\\(.*\\)\"\$/\\1/p" "$hosted_root/ci/visual/image.toml"
}
digest=$(image_key digest)

if [[ "$mode" == install ]]; then
  "$here/exec.sh" bash -c \
    "set -euo pipefail && mkdir -p target/visual && exclusions=\$(cargo run --quiet --bin xtask -- quarantine exclude) && for runner in vitest playwright; do jq -r --arg runner \"\$runner\" '.[\$runner][]' <<<\"\$exclusions\" >\"target/visual/quarantine-\$runner.txt\"; done"
  docker rm --force "$visual_container" >/dev/null 2>&1 || true
  docker run --detach --name "$visual_container" --platform linux/amd64 --ipc=host \
    --network "container:$CONTAINER" --volume "$workspace:/work" \
    --env CI=true --env "GOODFETCH_VISUAL_IMAGE=$digest" \
    "$(image_key repository)@$digest" sleep infinity >/dev/null
fi

environment=(--env CI=true)
# The Release suite's job sets it to `release`, so the visual projects of
# Firefox and WebKit run as well (ADR 0073).
if [[ -n "${GOODFETCH_TEST_SUITE:-}" ]]; then
  environment+=(--env "GOODFETCH_TEST_SUITE=$GOODFETCH_TEST_SUITE")
fi
if [[ -f "$EGRESS_STATE" ]]; then
  environment+=(--env npm_config_offline=true)
fi
docker exec "${environment[@]}" "$visual_container" /work/ci/visual/run.sh "$mode"
