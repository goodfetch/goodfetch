#!/usr/bin/env bash
# Runs the visual projects of Vitest and Playwright inside the pinned
# Playwright image (ADR 0073), the only place they exist: ci/vm/visual.sh
# starts the image with GOODFETCH_VISUAL_IMAGE set to the pinned digest, and
# both runner configs define their `visual` project only when it is set.
#   run.sh install  installs pnpm and the locked packages, before ADR 0043's
#                   deny
#   run.sh test     compares every Reference screenshot; writes none
#   run.sh update   rewrites the Reference screenshots that changed or are
#                   missing (the `update-screenshots` workflow only)
# Each Reference screenshot a test reads is recorded under target/visual/read/
# for the orphan check (`xtask screenshots orphans`).
set -uo pipefail

mode=${1:?usage: ci/visual/run.sh install|test|update}
cd "$(dirname "$0")/../.." || exit 2

pinned=$(sed -n 's/^digest = "\(.*\)"$/\1/p' ci/visual/image.toml)
if [[ -z "${GOODFETCH_VISUAL_IMAGE:-}" || "$GOODFETCH_VISUAL_IMAGE" != "$pinned" ]]; then
  echo "error: the visual projects run only in the pinned Playwright image ($pinned), through ci/vm/visual.sh" >&2
  exit 2
fi

case "$mode" in
  install) ;;
  test)
    vitest_flags=()
    playwright_flags=()
    ;;
  update)
    vitest_flags=(--update)
    playwright_flags=(--update-snapshots=changed)
    ;;
  *)
    echo "error: the mode is install, test or update, not $mode" >&2
    exit 2
    ;;
esac

# The image has Node and the browsers, not pnpm: install the version the
# repository pins, then the locked packages, from the cache folder's store
# when the cache-warming job filled it. The container lives for the job, so
# the later modes use them.
export npm_config_store_dir=/var/tmp/pnpm-store
if [[ "$mode" == install ]]; then
  set -e
  mkdir -p "$npm_config_store_dir"
  if [[ -d /cache/pnpm-store ]]; then cp -a /cache/pnpm-store/. "$npm_config_store_dir/"; fi
  pnpm_version=$(node -p 'require("./package.json").packageManager.split("@")[1]')
  npm install --global --silent "pnpm@$pnpm_version"
  pnpm install --frozen-lockfile
  exit 0
fi

rm -rf target/visual/read
mkdir -p target/visual/read

status=0
pnpm exec vitest run --project visual "${vitest_flags[@]}" || status=1
pnpm exec playwright test --project visual "${playwright_flags[@]}" || status=1
exit "$status"
