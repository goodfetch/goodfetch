#!/usr/bin/env bash
# Runs the visual projects in the job's VM, inside the pinned Playwright image
# (ADR 0073), which `ci/vm/start.sh <mode> playwright` loaded:
#   ci/vm/visual.sh test     compares every Reference screenshot (the visual job)
#   ci/vm/visual.sh update   rewrites the changed ones (`update-screenshots`)
# The image runs ci/visual/run.sh on the workspace at /work; the cache folder
# is read-only to it.
set -euo pipefail

mode=${1:?usage: ci/vm/visual.sh test|update}
here=$(cd "$(dirname "$0")" && pwd)
# shellcheck source=ci/vm/common.sh
source "$here/common.sh"

vm_shell nerdctl run --rm --platform linux/amd64 --ipc=host \
  --volume /work:/work --volume /cache:/cache:ro \
  --env CI=true --env "GOODFETCH_VISUAL_IMAGE=$PLAYWRIGHT_DIGEST" \
  "$PLAYWRIGHT_IMAGE" /work/ci/visual/run.sh "$mode"
