#!/usr/bin/env bash
# Runs the visual projects in the job's VM, inside the pinned Playwright image
# (ADR 0073), which `ci/vm/start.sh <mode> playwright` loaded:
#   ci/vm/visual.sh install  starts the image's container for the job and
#                            installs the packages, before ADR 0043's deny
#   ci/vm/visual.sh test     compares every Reference screenshot (the visual job)
#   ci/vm/visual.sh update   rewrites the changed ones (`update-screenshots`)
# The container runs ci/visual/run.sh on the workspace at /work; the cache
# folder is read-only to it. Under the deny, pnpm runs offline. The pinned
# image has no Rust, so `install` also writes the test quarantine list's
# exclusions (`xtask quarantine exclude`, in the CI image's container) to
# target/visual/quarantine-<runner>.txt, one argument a line, which `test`
# reads (ADR 0064).
set -euo pipefail

mode=${1:?usage: ci/vm/visual.sh install|test|update}
here=$(cd "$(dirname "$0")" && pwd)
# shellcheck source=ci/vm/common.sh
source "$here/common.sh"

if [[ "$mode" == install ]]; then
  "$here/exec.sh" bash -c \
    "set -euo pipefail && mkdir -p target/visual && exclusions=\$(cargo run --quiet --bin xtask -- quarantine exclude) && for runner in vitest playwright; do jq -r --arg runner \"\$runner\" '.[\$runner][]' <<<\"\$exclusions\" >\"target/visual/quarantine-\$runner.txt\"; done"
  vm_shell nerdctl run --detach --name "$VISUAL_CONTAINER" --platform linux/amd64 --ipc=host \
    --volume /work:/work --volume /cache:/cache:ro \
    --env CI=true --env "GOODFETCH_VISUAL_IMAGE=$PLAYWRIGHT_DIGEST" \
    "$PLAYWRIGHT_IMAGE" sleep infinity
fi

environment=(--env CI=true)
# The Release suite's job and the update-screenshots workflow set it to
# `release`, so the visual projects of Firefox and WebKit run as well (ADR 0073).
if [[ -n "${GOODFETCH_TEST_SUITE:-}" ]]; then
  environment+=(--env "GOODFETCH_TEST_SUITE=$GOODFETCH_TEST_SUITE")
fi
if [[ -f "$EGRESS_STATE" ]]; then
  environment+=(--env npm_config_offline=true)
fi
vm_shell nerdctl exec "${environment[@]}" "$VISUAL_CONTAINER" /work/ci/visual/run.sh "$mode"
