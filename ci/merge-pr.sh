#!/usr/bin/env bash
# Merges a pull request on the local gate (AGENTS.md, "Merging"). GitHub
# Actions is off, so no required check can block a merge: this command is the
# lock instead. It runs `just quick all` and `just pre-merge all` in a fresh
# worktree of the pull request's exact head, and merges only when both pass.
set -euo pipefail

number=${1:?usage: ci/merge-pr.sh <pull request number>}

IFS=$'\t' read -r state draft head base title < <(
  gh pr view "$number" --json state,isDraft,headRefOid,baseRefName,title \
    --jq '[.state, (.isDraft | tostring), .headRefOid, .baseRefName, .title] | @tsv'
)
if [[ $state != OPEN ]]; then
  echo "error: pull request #$number is $state, not open" >&2
  exit 1
fi
if [[ $draft != false ]]; then
  echo "error: pull request #$number is a draft; mark it ready first" >&2
  exit 1
fi

git fetch --quiet origin "$base"
git fetch --quiet origin "pull/$number/head"
if [[ $(git rev-parse FETCH_HEAD) != "$head" ]]; then
  echo "error: pull request #$number moved while it was read; run the command again" >&2
  exit 1
fi
# A head that contains its base's tip merges into the same tree it was tested
# on, so the gate's verdict holds for what lands on the base.
if ! git merge-base --is-ancestor "origin/$base" "$head"; then
  echo "error: pull request #$number does not contain the tip of $base; merge $base into it first" >&2
  exit 1
fi

worktree=$(mktemp -d "${TMPDIR:-/tmp}/goodfetch-merge-$number.XXXXXX")
remove_worktree() { git worktree remove --force "$worktree" >/dev/null 2>&1 || true; }
trap remove_worktree EXIT
git worktree add --quiet --detach "$worktree" "$head"

echo "Running the local gate on #$number at $head"
(
  cd "$worktree"
  just fetch
  just quick all
  just pre-merge all
)

gh pr merge "$number" --merge --match-head-commit "$head" \
  --subject "$title (#$number)" \
  --body "Merged on the local gate: \`just quick all\` and \`just pre-merge all\` passed at $head.

Signed-off-by: $(git config user.name) <$(git config user.email)>"
echo "Merged #$number at $head"
