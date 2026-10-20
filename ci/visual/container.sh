#!/usr/bin/env bash
# `just visual`: the visual projects of Vitest and Playwright in the pinned
# Playwright image (ADR 0073) through Docker or Podman on this host, for the
# local gate (ADR 0135's note of 2026-10-20), as CI's `visual` job runs them
# in the job's VM (ci/vm/visual.sh install, then test).
#
# The image gets a copy of the working tree's files (tracked, and untracked
# unless ignored), never the tree itself: its `pnpm install` writes Linux
# packages, which would replace the host's in node_modules/. Its target/ is
# the host's target/visual-image/, so the JUnit files, the Reference
# screenshots read (target/visual-image/visual/read/) and a failure's
# attachments and traces stay on the host. pnpm's store lives in a volume of
# its own between runs. Without a container engine it lists itself as
# skipped with the reason (ADR 0061), as ci/perf.sh does.
set -euo pipefail

root=$(cd "$(dirname "$0")/../.." && pwd)
cd "$root"

# In the image, on the copy: install the packages, then compare. Playwright
# writes a failure's traces beside its config, outside target/.
if [[ "${1:-}" == in-image ]]; then
  ci/visual/run.sh install
  status=0
  ci/visual/run.sh test || status=$?
  if [[ -d test-results ]]; then cp -a test-results target/; fi
  exit "$status"
fi

# shellcheck source=ci/container.sh
source "$root/ci/container.sh"

engine=$(container_engine)
[[ -n "$engine" ]] || skip "just visual" "the visual projects run only in the pinned Playwright image, and no container engine (Docker or Podman) is running on this host"
# shellcheck source=ci/vm/common.sh
source "$root/ci/vm/common.sh"

output="$root/target/visual-image"
rm -rf "$output"
mkdir -p "$output/visual"
# The image has no Rust, so the test quarantine list's exclusions are written
# here, where ci/visual/run.sh test reads them (ADR 0064).
cargo run --quiet --locked -p xtask -- quarantine exclude >"$output/visual/quarantine.tsv"

git ls-files -z --cached --others --exclude-standard |
  while IFS= read -r -d '' file; do
    if [[ -e "$file" || -L "$file" ]]; then printf '%s\0' "$file"; fi
  done |
  COPYFILE_DISABLE=1 tar -c --no-xattrs --null -T - -f - |
  "$engine" run --rm --interactive --init --platform linux/amd64 --ipc=host \
    --volume "$output:/work/target" --volume goodfetch-visual-pnpm:/var/tmp/pnpm-store \
    --env CI=true --env "GOODFETCH_VISUAL_IMAGE=$PLAYWRIGHT_DIGEST" \
    "$PLAYWRIGHT_IMAGE" bash -c 'mkdir -p /work && tar -x -C /work && exec /work/ci/visual/container.sh in-image'
