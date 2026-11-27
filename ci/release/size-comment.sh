#!/usr/bin/env bash
# Runs `xtask size estimate` on Linux x64 for both Products and creates or
# updates one sticky comment on the pull request (ADRs 0057, 0087 and 0094;
# issue 204). Never fails the pull request.
#
# Usage: ci/release/size-comment.sh <target> <pull-request-number>
set -euo pipefail

target=${1:?usage: ci/release/size-comment.sh <target> <pull-request-number>}
pr=${2:?usage: ci/release/size-comment.sh <target> <pull-request-number>}
root=$(cd "$(dirname "$0")/../.." && pwd)
cd "$root"

marker="<!-- goodfetch-size-estimate -->"
mkdir -p target/release-suite

ci/release/size-manifest.sh "$target" >target/release-suite/size-files.toml

# Look up `main`'s saved size report for `origin/main^{tree}` when available.
main_report=""
if main_tree=$(git rev-parse --verify --quiet 'origin/main^{tree}' 2>/dev/null); then
  artifact_name="size-report-${main_tree}-${target}"
  candidate="target/release-suite/${artifact_name}.json"
  if [[ -f "$candidate" ]]; then
    main_report="$candidate"
  elif [[ -n "${GH_TOKEN:-}" && -n "${GITHUB_REPOSITORY:-}" ]]; then
    artifact_id=$(
      gh api "repos/${GITHUB_REPOSITORY}/actions/artifacts?name=${artifact_name}&per_page=1" \
        --jq '.artifacts[0].id // empty' 2>/dev/null || true
    )
    if [[ -n "$artifact_id" ]]; then
      tmp_zip="target/release-suite/${artifact_name}.zip"
      if gh api "repos/${GITHUB_REPOSITORY}/actions/artifacts/${artifact_id}/zip" >"$tmp_zip" 2>/dev/null; then
        unzip -oq "$tmp_zip" -d target/release-suite/ || true
        [[ -f "$candidate" ]] && main_report="$candidate"
      fi
      rm -f "$tmp_zip"
    fi
  fi
fi

args=(size estimate --files target/release-suite/size-files.toml --main origin/main)
if [[ -n "$main_report" ]]; then
  args+=(--main-report "$main_report")
fi

table=$(cargo run --quiet --locked --bin xtask -- "${args[@]}")
body=$(printf '%s\n%s\n' "$marker" "$table")

# Post or update the single sticky comment on the pull request.
if [[ -n "${GH_TOKEN:-}" && -n "${GITHUB_REPOSITORY:-}" ]]; then
  existing_id=$(
    gh api "repos/${GITHUB_REPOSITORY}/issues/${pr}/comments?per_page=100" \
      --jq ".[] | select(.body | contains(\"${marker}\")) | .id" | head -n 1 || true
  )
  if [[ -n "$existing_id" ]]; then
    gh api --method PATCH "repos/${GITHUB_REPOSITORY}/issues/comments/${existing_id}" \
      -f body="$body" >/dev/null
  else
    gh api --method POST "repos/${GITHUB_REPOSITORY}/issues/${pr}/comments" \
      -f body="$body" >/dev/null
  fi
else
  printf '%s\n' "$body"
fi
