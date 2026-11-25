#!/usr/bin/env bash
# The nightly tier's first job (ADR 0061): runs the Release suite again on
# `main`'s head before the nightly-only jobs. It dispatches release-suite.yml
# on `main` with every cell and the commit COMMIT, finds the run the dispatch
# created, and waits until it ends. The run stays a release-suite.yml run of
# its own, so its plan, its ceiling, its verdict (`xtask release-suite
# verdict`) and its ingest are a manual dispatch's. A run that ends red ends
# this job red; the nightly-only jobs run after it either way.
#
# Environment: GH_TOKEN (the job's token, with `actions: write`),
# GITHUB_REPOSITORY and COMMIT.
set -euo pipefail

commit=${COMMIT:?the job sets COMMIT}
repository=${GITHUB_REPOSITORY:?the runner sets GITHUB_REPOSITORY}
started=$(date -u +%Y-%m-%dT%H:%M:%SZ)

gh workflow run release-suite.yml --repo "$repository" --ref main \
  -f cells=all -f commit="$commit"

# The dispatch answers before the run is listed: look for it for 5 minutes.
run=""
for _ in $(seq 30); do
  run=$(gh run list --repo "$repository" --workflow release-suite.yml \
    --event workflow_dispatch --branch main --user 'github-actions[bot]' \
    --limit 10 --json databaseId,createdAt |
    jq -r --arg started "$started" \
      '[.[] | select(.createdAt >= $started)] | sort_by(.createdAt) | first | .databaseId // empty')
  [[ -n "$run" ]] && break
  sleep 10
done
if [[ -z "$run" ]]; then
  echo "::error::No Release run this job dispatched at $started appeared within 5 minutes." >&2
  exit 1
fi

echo "Release suite run $run on $commit: https://github.com/$repository/actions/runs/$run"
gh run watch "$run" --repo "$repository" --interval 60 --exit-status
