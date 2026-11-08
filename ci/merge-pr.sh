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
# passes `--past-cap`. It refuses to start a gate while the file system of
# the gate's `target/` has less free space than a gate's builds take (#1822);
# keeps a failing gate's log and test results where no cleanup reaches them
# (#2087); and, still holding the gate's lock, removes the build units no
# gate read since the day before this one started (`xtask sweep-target`).
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
# shellcheck source=ci/output-value.sh
source "$(dirname "$0")/output-value.sh"
gate_worktree_lock

# A failing gate's log and test results are kept in the repository's common
# folder, which no checkout's clean, no worktree removal and no sweep of the
# temporary folder reaches, for 30 days: a flaky-candidate issue links them.
gate_log=$(mktemp)
failures="$(git rev-parse --path-format=absolute --git-common-dir)/goodfetch-gate-failures"
keep_failed_gate() {
  local kept entry
  kept="$failures/$(date -u +%Y%m%dT%H%M%SZ)-pr$number-${head:0:12}"
  mkdir -p "$kept"
  cp "$gate_log" "$kept/gate.log"
  for entry in nextest junit skipped.txt stress-selection.tsv; do
    [[ ! -e $worktree/target/$entry ]] || cp -R "$worktree/target/$entry" "$kept/"
  done
  find "$failures" -mindepth 1 -maxdepth 1 -type d -mtime +30 -exec rm -rf {} +
  echo "The failing gate's log and test results are kept in $kept: link that folder in the flaky-candidate issue" >&2
}

# After the gate, pass or fail, and before the lock is released: the build
# units no gate read since the day before this gate started go, which
# `relatime` keeps telling apart (xtask/src/sweep_target.rs). The sweep takes
# the start itself, so the gate's own reads stay however long it ran. A head
# from before the command only warns.
started=""
on_exit() {
  if [[ -n $started ]]; then
    (cd "$worktree" && cargo run --quiet --locked --bin xtask -- sweep-target --read-since "$started") ||
      echo "warning: the gate's target/ was not swept" >&2
  fi
  rm -f "$gate_log"
  rm -rf "$lock"
}
trap on_exit EXIT

# A gate's builds, a dependency change's included, take up to this much on
# the file system of its `target/`, which can be another than the
# checkout's (a mount of its own).
min_free_gb=${GOODFETCH_GATE_MIN_FREE_GB:-30}
on=$worktree/target
[[ -e $on ]] || on=$root
read -r free_gb mount < <(df -Pk "$on" | awk 'NR == 2 { print int($4 / 1048576), $6 }')
if ((free_gb < min_free_gb)); then
  echo "error: the gate's target/ is on $mount, which has $free_gb GB free, under the $min_free_gb GB a gate's builds need (GOODFETCH_GATE_MIN_FREE_GB); free space there first, for example with \`cargo run --bin xtask -- sweep-target --target $worktree/target --unused-hours 25\`" >&2
  exit 1
fi
echo "The gate's target/ is on $mount, with $free_gb GB free (at least $min_free_gb GB needed)"

gate_worktree_checkout "$head"

# The head's own `xtask affected` decides; a selection without a `gate=`
# line or with a gate this script does not know fails before any step.
selection=$(cd "$worktree" && cargo run --quiet --locked --bin xtask -- affected --base "origin/$base" --head "$head")
gate=$(output_value gate "$selection")
case $gate in
  quick) ran="\`just pre-merge quick\` (no test, benchmark, model, container or Evaluation reads the change)" ;;
  docs) ran="\`just pre-merge docs\` (the change is documents alone)" ;;
  full)
    gate=all
    ran="\`just pre-merge all\`, which starts with \`just quick all\`'s lint and Quick suite,"
    ;;
  *)
    echo "error: \`xtask affected\` gave the gate $gate, which is quick, docs or full" >&2
    exit 1
    ;;
esac
echo "Running the local gate ($gate) on #$number at $head in $worktree"
started=$(date -u +%s)
status=0
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
) 2>&1 | tee "$gate_log" || status=$?
if ((status != 0)); then
  keep_failed_gate
  exit "$status"
fi
# The incremental gate's verdict combines this run with the green results it
# reused from earlier runs on ancestor heads (ADR 0135); the full gate's plan
# names the nearest such run.
if [[ $gate == all ]]; then
  plan=$(cat "$worktree/target/gate-runs/$head.plan")
  reused=$(output_value previous "$plan")
  if [[ $reused != none ]]; then
    ran+=" reusing the green results of the gate run on $reused for the steps the change since did not reach (ADR 0135),"
  fi
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
