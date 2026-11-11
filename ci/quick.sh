#!/usr/bin/env bash
# `just quick [all]`: the pre-push tier (T1), which `.githooks/pre-push`
# runs (TESTING.md, Commands and Checks by role). The compiler cache's
# coverage comes first, before the first build (`just compiler-cache`;
# TESTING.md, Compiler cache). The floor guard and the scan of the change's
# commits for secrets come next: they take seconds, and the gate would
# otherwise first run them at merge. Then the checks the
# change's own gate runs at merge, from `xtask affected`'s `gate=` line, as
# `just merge` picks its form (ADR 0135): a change of documents alone
# (`gate=docs`) takes `just lint-documents`, the checks of `just lint` that
# read documents, and no code lint or suite, as the docs gate does; any
# other change takes `just lint` and the Quick suite over what changed.
# `all` takes every lint and the Quick suite over the whole workspace,
# whatever changed.
#
# What changed is the working tree against its merge base with the change's
# base: `GOODFETCH_QUICK_BASE` when the pre-push hook gives it (the commit
# the push's remote ref holds), else the branch's upstream off `main` when
# it is another branch (the integration branch a ticket goes into), else
# `origin/main`. The branch's own remote copy, its upstream after `git push
# -u`, is never the base: against it a pushed branch changed nothing, so no
# test would run and the run would still record the tree. The base is
# read once, first, and every step that reads the change's commits gets it:
# the floor guard, the scan for secrets, `xtask affected` and the Quick
# suite, `all`'s floor guard and scan too, so on an integration branch they
# judge the ticket's own commits, not the earlier tickets'.
#
# Every step runs even after one failed (ci/keep-going.sh), the lints' own
# steps included, so one run reports every failure at once; the run then
# lists each failed step with its log under target/steps/quick/ and fails.
# The Quick suite runs after a red lint too: a lint failure (formatting,
# Clippy, a drift check, a document check) leaves the tests' build and
# results as meaningful as before, and a test failure then shows in the same
# round instead of the next one. A compile error fails both, which costs the
# suite's build once more and reports nothing new, a smaller cost than one
# more round for every ticket whose tests fail beside a red lint. Only
# `xtask affected` ends the run early, since without its `gate=` line the
# tier cannot tell which checks the change takes.
#
# A green run on a clean checkout records its tree in the repository's
# common folder, shared by its worktrees, and the pre-push hook does not run
# it on a push of a recorded tree: `just quick` never runs twice on one tree.
# A tree is recorded only when every step passed.
#
# Every step runs through the `just` on PATH, where the recipe puts its own.
# It keeps to bash 3.2, which stock macOS runs.
set -euo pipefail

all=${1:-}
case "$all" in
  all | "") ;;
  *)
    echo "error: the only argument is \`all\`, not $all" >&2
    exit 2
    ;;
esac

# A hook starts this with git's GIT_DIR (a linked worktree's on a push), and a
# build script's `git init` under a step (btls-sys's) would then reinitialize
# the repository as bare: every step finds the repository from its folder.
for variable in $(git rev-parse --local-env-vars); do unset "$variable"; done

# Records HEAD's tree as passed when the working tree is HEAD's, untracked
# files included; outside a repository it records nothing.
record_green() {
  local common tree stamps
  common=$(git rev-parse --path-format=absolute --git-common-dir 2>/dev/null) || return 0
  if [[ -n $(git status --porcelain 2>/dev/null) ]]; then
    echo "The working tree differs from HEAD, so no tree is recorded for the pre-push hook."
    return 0
  fi
  tree=$(git rev-parse 'HEAD^{tree}')
  stamps=$common/goodfetch-quick-green
  { tail -n 199 "$stamps" 2>/dev/null || true; echo "$tree"; } > "$stamps.$$"
  mv "$stamps.$$" "$stamps"
  echo "\`just quick\` passed on tree $tree: the pre-push hook does not run it again on a push of it."
}

# shellcheck source=ci/keep-going.sh
source "$(dirname "${BASH_SOURCE[0]}")/keep-going.sh"
# shellcheck source=ci/output-value.sh
source "$(dirname "${BASH_SOURCE[0]}")/output-value.sh"

base=${GOODFETCH_QUICK_BASE:-}
if [[ -z $base ]]; then
  branch=$(git symbolic-ref --quiet --short HEAD 2>/dev/null || true)
  if [[ -n $branch && $branch != main ]]; then
    # `git push -u` makes the branch's upstream its own remote copy: the
    # branch of its name on the remote, which holds the change's own commits.
    if [[ $(git config --get "branch.$branch.merge" 2>/dev/null || true) == "refs/heads/$branch" ]]; then
      echo "The upstream of $branch is its own remote copy, which is no base for its change, so the change is read against origin/main. When the branch goes into an integration branch, give that branch as GOODFETCH_QUICK_BASE."
    else
      base=$(git rev-parse --abbrev-ref --symbolic-full-name '@{upstream}' 2>/dev/null || true)
    fi
  fi
fi
base=${base:-origin/main}
echo "The change: the working tree against its merge base with $base."
quoted_base=$(printf '%q' "$base")

rm -f target/skipped.txt
trap 'just _skips-report' EXIT
steps_begin quick
step compiler-cache 'just compiler-cache'
step floor-guard "just floor-guard $quoted_base"
step commit-leaks "just commit-leaks $quoted_base"

if [[ -z $all ]]; then
  change=(--local --since "$base")
  if ! selection=$(cargo run --quiet --locked --bin xtask -- affected "${change[@]}"); then
    echo "error: \`xtask affected\` failed, so no lint and no Quick suite ran: the tier cannot tell which checks the change takes" >&2
    steps_end || true
    exit 1
  fi
  if ! gate=$(output_value gate "$selection"); then
    echo "error: \`xtask affected\` gave no gate, so no lint and no Quick suite ran: the tier cannot tell which checks the change takes" >&2
    steps_end || true
    exit 1
  fi
  if [[ $gate == docs ]]; then
    echo "The change is documents alone (xtask affected's gate=docs), so the pre-push tier runs the docs gate's checks: \`just lint-documents\`; the code lints and the Quick suite do not run, as at the merge (ADR 0135)."
    step lint-documents 'just lint-documents'
  else
    step lint 'just lint'
    suite=$(printf ' %q' "${change[@]}")
    step quick-suite "just test-affected quick$suite"
  fi
else
  step lint 'just lint'
  step quick-suite 'just _suite quick all'
fi

if ! steps_end; then
  exit 1
fi
record_green
