#!/usr/bin/env bash
# Merges a pull request on the local gate (AGENTS.md, "Merging"). GitHub
# Actions runs only the Release suite, so no required check can block a
# merge: this command is the lock instead. In a clean checkout of the pull
# request's exact head it runs `just pre-merge all`, which starts with `just
# quick all`'s lint and Quick suite, or `just pre-merge quick` when `xtask
# affected` finds that no test, benchmark, model, container or Evaluation
# reads the change (`gate=quick`, ADR 0135), or `just pre-merge docs` when
# that change is documents alone (`gate=docs`), and merges only when it
# passes. First the merge cap (ci/merge-cap.sh) refuses once 10 merges have
# landed on `main` since the backstop's last green run, unless the owner
# passes `--past-cap`.
set -euo pipefail

usage="usage: ci/merge-pr.sh <pull request number> [--past-cap]"
number=${1:?$usage}
cap=()
case "${2:-}" in
  "") ;;
  --past-cap) cap=(--past-cap) ;;
  *)
    echo "error: $usage" >&2
    exit 2
    ;;
esac

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
git fetch --quiet origin main
"$(dirname "$0")/merge-cap.sh" ${cap[@]+"${cap[@]}"}
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

# This checkout's compiler cache check, which the gate runs in its own.
compiler_cache=$(cd "$(dirname "$0")" && pwd)/compiler-cache.sh
# One worktree for every gate, locked and held by one gate at a time, reset
# to the exact head (ci/gate-worktree.sh).
# shellcheck source=ci/gate-worktree.sh
source "$(dirname "$0")/gate-worktree.sh"
gate_worktree_lock
trap 'rm -rf "$lock"' EXIT
gate_worktree_checkout "$head"

# The head's own `xtask affected` decides, so a head from before `gate=`
# existed prints no such line and gets the full gate.
gate=$(cd "$worktree" && cargo run --quiet --locked --bin xtask -- affected --base "origin/$base" --head "$head" |
  sed -n 's/^gate=//p')
if [[ $gate == quick ]]; then
  ran="\`just pre-merge quick\` (no test, benchmark, model, container or Evaluation reads the change)"
elif [[ $gate == docs ]]; then
  ran="\`just pre-merge docs\` (the change is documents alone)"
else
  gate=all
  ran="\`just pre-merge all\`, which starts with \`just quick all\`'s lint and Quick suite,"
fi
echo "Running the local gate ($gate) on #$number at $head in $worktree"
(
  cd "$worktree"
  # Before any build: whether the gate's checkout reuses the C and C++
  # objects other checkouts compiled (TESTING.md, Compiler cache); a gap
  # fails the gate.
  COMPILER_CACHE_REQUIRED=1 "$compiler_cache"
  just fetch
  if [[ $gate == quick || $gate == docs ]]; then
    just pre-merge "$gate"
  else
    # What the pull request's "Performance change" section and its owner's
    # acceptance comment accept, read at this head for `just perf` (ADR 0134).
    # Until M1's optimization pass it only marks figures accepted in the
    # table: no figure past its threshold fails (ADR 0134, as amended on
    # 2026-10-22).
    GITHUB_TOKEN="${GITHUB_TOKEN:-$(gh auth token)}" cargo run --quiet --bin xtask -- \
      perf pull-request --number "$number" --save target/perf-pull-request.json
    GOODFETCH_PERF_PULL_REQUEST="$worktree/target/perf-pull-request.json" just pre-merge all
  fi
)
# The incremental gate's verdict combines this run with the green results it
# reused from earlier runs on ancestor heads (ADR 0135); the plan names the
# nearest such run.
reused=$(awk -F'\t' '$1 == "previous" { print $2 }' "$worktree/target/gate-runs/$head.plan" 2>/dev/null || true)
if [[ -n "$reused" && "$reused" != none ]]; then
  ran+=" reusing the green results of the gate run on $reused for the steps the change since did not reach (ADR 0135),"
fi

past=""
[[ ${#cap[@]} -eq 0 ]] || past=" Merged past the merge cap: the owner passed --past-cap."
gh pr merge "$number" --merge --match-head-commit "$head" \
  --subject "$title (#$number)" \
  --body "Merged on the local gate: $ran passed at $head.$past

Signed-off-by: $(git config user.name) <$(git config user.email)>"
echo "Merged #$number at $head"

# The next gate's stress run reads its base's listings from here, the base
# being the tip of the base branch it must contain: this merge, whose tree is
# the head's when the head contained the base's tip, as checked above. Saved
# only then; otherwise the next stress run lists its base from a checkout of
# it, as it does without them. The head's test binaries are built, so listing
# them takes seconds.
base_tip=$(git rev-parse "origin/$base")
git fetch --quiet origin "$base" || true
merged=$(git rev-parse "origin/$base")
listings="$worktree/target/stress-listings"
if [[ $gate == quick || $gate == docs ]]; then
  # No test reads a quick or docs gate's change, so the base's listings,
  # when saved, are the merge's too, and no test binary was built to list.
  if [[ -d "$listings/$base_tip" ]] && mv "$listings/$base_tip" "$listings/$merged"; then
    echo "Kept the stress listings of $base_tip for $merged"
  else
    rm -rf "$listings"
    echo "warning: no stress listings of $merged were saved; the next stress run lists its base from a checkout" >&2
  fi
elif [[ $(git rev-parse "$merged^{tree}") == $(git rev-parse "$head^{tree}") ]] &&
  (cd "$worktree" && rm -rf "$listings" &&
    cargo run --quiet --locked --bin xtask -- stress listings --cargo-profile test --out "$listings" &&
    mv "$listings/$head" "$listings/$merged"); then
  echo "Saved the stress listings of $merged for the next gate"
else
  rm -rf "$listings"
  echo "warning: the stress listings of $merged were not saved; the next stress run lists its base from a checkout" >&2
fi
