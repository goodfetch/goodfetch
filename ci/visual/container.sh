#!/usr/bin/env bash
# The visual projects of Vitest and Playwright in the pinned Playwright image
# (ADR 0073) through Docker or Podman on this host, for the local gate (ADR
# 0135's note of 2026-10-20):
#   ci/visual/container.sh test    `just visual`: compares every Reference
#                                  screenshot, as CI's `visual` job does in
#                                  the job's VM (ci/vm/visual.sh install, then
#                                  test)
#                                  (`GOODFETCH_TEST_SUITE=release` adds the
#                                  Release suite's Firefox and WebKit ones)
#   ci/visual/container.sh update  `just screenshots-update`: rewrites the
#                                  changed or missing ones of every browser
#                                  in the image, as the Release suite reads them
#                                  (ci/visual/run.sh update) and copies them
#                                  into the working tree, while GitHub Actions
#                                  is off and the `update-screenshots`
#                                  workflow cannot run (ADR 0073's note of
#                                  2026-10-20)
#
# The image gets a copy of the working tree's files (tracked, and untracked
# unless ignored), never the tree itself: its `pnpm install` writes Linux
# packages, which would replace the host's in node_modules/. Its target/ is
# the host's target/visual-image/, so the JUnit files, the Reference
# screenshots read (target/visual-image/visual/read/), a failure's
# attachments and traces, and the rewritten screenshots stay on the host.
# pnpm's store lives in a volume of its own between runs. A host with neither
# Docker nor Podman fails with an error naming the fix (ADR 0073).
set -euo pipefail

root=$(cd "$(dirname "$0")/../.." && pwd)
cd "$root"

mode=${1:?usage: ci/visual/container.sh test|update}

# In the image, on the copy: install the packages, then compare, or rewrite
# and hand every Reference screenshot back through target/references/.
# Playwright writes a failure's traces beside its config, outside target/.
if [[ "$mode" == in-image ]]; then
  ci/visual/run.sh install
  status=0
  case "${2:?}" in
    test)
      ci/visual/run.sh test || status=$?
      if [[ -d test-results ]]; then cp -a test-results target/; fi
      ;;
    update)
      ci/visual/run.sh update || status=$?
      find . \( -path ./node_modules -o -path ./target -o -path ./rnd \) -prune -o -type f -name '*.png' \
        \( -path '*/__screenshots__/*' -o -path '*-snapshots/*' \) -print |
        while IFS= read -r file; do
          mkdir -p "target/references/$(dirname "$file")"
          cp "$file" "target/references/$file"
        done
      ;;
  esac
  exit "$status"
fi

case "$mode" in
  test | update) ;;
  *)
    echo "error: the mode is test or update, not $mode" >&2
    exit 2
    ;;
esac

# shellcheck source=ci/container.sh
source "$root/ci/container.sh"

engine=$(container_engine)
if [[ -z "$engine" ]]; then
  echo "error: the visual projects run only in the pinned Playwright image, and no container engine is running on this host; install and start Docker or Podman (ADR 0073)" >&2
  exit 1
fi
# shellcheck source=ci/vm/common.sh
source "$root/ci/vm/common.sh"

output="$root/target/visual-image"
rm -rf "$output"
mkdir -p "$output/visual"
# The image has no Rust, so the test quarantine list's exclusions are written
# here, where ci/visual/run.sh test reads them (ADR 0064).
cargo run --quiet --locked --bin xtask -- quarantine exclude >"$output/visual/quarantine.tsv"

environment=(--env CI=true --env "GOODFETCH_VISUAL_IMAGE=$PLAYWRIGHT_DIGEST")
if [[ "$mode" == update ]]; then
  # Every browser's References, as the `update-screenshots` workflow writes
  # them: the Release suite's visual job compares Firefox's and WebKit's too
  # (ADR 0073).
  environment+=(--env GOODFETCH_TEST_SUITE=release)
elif [[ -n "${GOODFETCH_TEST_SUITE:-}" ]]; then
  # `GOODFETCH_TEST_SUITE=release just visual` compares as the Release
  # suite's visual job does (ci/vm/visual.sh).
  environment+=(--env "GOODFETCH_TEST_SUITE=$GOODFETCH_TEST_SUITE")
fi

git ls-files -z --cached --others --exclude-standard |
  while IFS= read -r -d '' file; do
    if [[ -e "$file" || -L "$file" ]]; then printf '%s\0' "$file"; fi
  done |
  COPYFILE_DISABLE=1 tar -c --no-xattrs --null -T - -f - |
  "$engine" run --rm --interactive --init --platform linux/amd64 --ipc=host \
    --volume "$output:/work/target" --volume goodfetch-visual-pnpm:/var/tmp/pnpm-store \
    "${environment[@]}" \
    "$PLAYWRIGHT_IMAGE" bash -c "mkdir -p /work && tar -x -C /work && exec /work/ci/visual/container.sh in-image $mode"

if [[ "$mode" == update ]]; then
  if [[ -d "$output/references" ]]; then cp -R "$output/references/." "$root/"; fi
  echo "Reference screenshots rewritten in $PLAYWRIGHT_IMAGE; commit the changed ones with your own sign-off:"
  git status --short --untracked-files=all -- '*.png'
fi
