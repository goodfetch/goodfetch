#!/usr/bin/env bash
# `just batch-check [base]`: run in a batch branch's worktree before `just
# merge` (AGENTS.md, Merging). Each pull request's `just quick` read its own
# change against its own base; the batch's combined tree is new, and what it
# can break the gate would otherwise first see. So this runs, against `base`
# (`origin/main` by default):
#
# - the checks of the whole change's commits, which the gate's forms start
#   with: sign-offs, subjects, the floor guard and the scan for secrets;
# - the licence allowlist (`just notices`), the checks that read documents
#   (`just lint-documents`) and the drift checks (`just lint-drift`), each of
#   which a combination can fail though every change passed it alone;
# - the Quick suite over the members that two or more of the batch's
#   branches reach, the only members whose tests a combination can break
#   that each branch's own Quick suite did not run (batch h, 2026-11-18: two
#   pull requests green alone broke a corpus test of a crate both reached).
#   A branch is each branch a merge on the batch's first-parent line brought,
#   and the commits made on the batch branch itself, together, as one more. A
#   merge of the base itself, as the gate's merge of `main`'s tip, brings no
#   branch. `xtask affected` gives each branch's members; when it fails, no
#   suite runs and the run fails.
#
# Every step runs even after one failed (ci/keep-going.sh); the run then lists
# each failed step with its log under target/steps/batch-check/ and fails.
# Every step runs through the `just` on PATH, where the recipe puts its own.
# It keeps to bash 3.2, which stock macOS runs.
set -euo pipefail

base=${1:-origin/main}

# A hook may start this with git's GIT_DIR, and a build script's `git init`
# under a step (btls-sys's) would then reinitialize the repository as bare.
for variable in $(git rev-parse --local-env-vars); do unset "$variable"; done

# shellcheck source=ci/keep-going.sh
source "$(dirname "${BASH_SOURCE[0]}")/keep-going.sh"
# shellcheck source=ci/output-value.sh
source "$(dirname "${BASH_SOURCE[0]}")/output-value.sh"

quoted_base=$(printf '%q' "$base")
echo "The batch: HEAD against its merge base with $base."
steps_begin batch-check
step dco "just dco $quoted_base"
step commit-messages "just commit-messages $quoted_base"
step floor-guard "just floor-guard $quoted_base"
step commit-leaks "just commit-leaks $quoted_base"
step notices 'just notices'
step lint-documents 'just lint-documents'
step lint-drift 'just lint-drift'

members_dir=$(mktemp -d)
trap 'rm -rf "$members_dir"' EXIT
mkdir "$members_dir/branches"

# Appends to `$members_dir/branches/$1` the members `xtask affected` gives
# for the change from `$2` to `$3`; fails when it gives none of its lines.
add_members() {
  local selection key value member
  selection=$(cargo run --quiet --locked --bin xtask -- affected --base "$2" --head "$3") || return 1
  for key in crates vitest-packages pytest-packages; do
    value=$(output_value "$key" "$selection") || return 1
    for member in $value; do echo "$member" >> "$members_dir/branches/$1"; done
  done
}

branches=0
own=""
failed=""
for commit in $(git rev-list --first-parent "$base..HEAD"); do
  # The commit, then its parents.
  read -r -a parents <<< "$(git rev-list --parents -n 1 "$commit")"
  if [[ ${#parents[@]} -le 2 ]]; then
    own="$own $commit"
    continue
  fi
  for parent in "${parents[@]:2}"; do
    git merge-base --is-ancestor "$parent" "$base" && continue
    branches=$((branches + 1))
    touch "$members_dir/branches/$branches"
    add_members "$branches" "$base" "$parent" || failed="$failed $parent"
  done
done
if [[ -n $own ]]; then
  branches=$((branches + 1))
  touch "$members_dir/branches/$branches"
  for commit in $own; do
    add_members "$branches" "$commit^" "$commit" || failed="$failed $commit"
  done
fi

if [[ -n $failed ]]; then
  echo "error: \`xtask affected\` failed for${failed}, so no Quick suite ran: the run cannot tell which members two branches reach" >&2
  steps_end || true
  exit 1
fi

shared=""
if ((branches > 1)); then
  for file in "$members_dir"/branches/*; do sort -u "$file"; done > "$members_dir/all"
  shared=$(sort "$members_dir/all" | uniq -d | tr '\n' ' ')
  shared=${shared% }
fi
if [[ -n $shared ]]; then
  echo "The members two or more of the batch's $branches branches reach: $shared."
  step quick-suite "just test quick $shared"
else
  echo "No member is reached by two of the batch's $branches branches, so no suite runs: each branch's own \`just quick\` ran its tests."
fi

steps_end
