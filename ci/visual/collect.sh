#!/usr/bin/env bash
# Collects what the `update-screenshots` workflow uploads (ADR 0073), after
# `ci/hosted/visual.sh update` rewrote the changed Reference screenshots:
#   ci/visual/collect.sh <folder>
# writes <folder>/screenshots/<repository path> for every new or changed
# Reference screenshot (a PNG file in a `__screenshots__/` or `-snapshots/`
# folder), and <folder>/provenance.json with the pinned image's digest, the
# commit the run rendered and the run's ID, which `xtask screenshots fetch`
# checks. It runs on the runner's host, outside the job's container.
set -euo pipefail

out=${1:?usage: ci/visual/collect.sh <folder>}
cd "$(dirname "$0")/../.."

digest=$(sed -n 's/^digest = "\(.*\)"$/\1/p' ci/visual/image.toml)
commit=$(git rev-parse HEAD)
if [[ -n "${GITHUB_SHA:-}" && "$commit" != "$GITHUB_SHA" ]]; then
  echo "error: the checkout is at $commit, not the dispatched commit $GITHUB_SHA" >&2
  exit 1
fi

rm -rf "$out"
mkdir -p "$out/screenshots"

count=0
while IFS= read -r -d '' entry; do
  path=${entry:3}
  case "$path" in
    *.png) ;;
    *) continue ;;
  esac
  case "/$path" in
    */__screenshots__/* | *-snapshots/*) ;;
    *) continue ;;
  esac
  [[ -f "$path" ]] || continue
  mkdir -p "$out/screenshots/$(dirname "$path")"
  cp "$path" "$out/screenshots/$path"
  echo "collected $path"
  count=$((count + 1))
done < <(git status --porcelain=v1 -z --untracked-files=all)

printf '{ "digest": "%s", "commit": "%s", "run": %s }\n' \
  "$digest" "$commit" "${GITHUB_RUN_ID:-0}" >"$out/provenance.json"
echo "collected $count Reference screenshots rendered at $commit in $digest"
