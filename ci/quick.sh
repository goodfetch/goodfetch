#!/usr/bin/env bash
# `just quick [all]`: the pre-push tier (T1), which `.githooks/pre-push`
# runs (TESTING.md, Commands and Checks by role). The compiler cache's
# coverage comes first, before the first build (`just compiler-cache`;
# TESTING.md, Compiler cache). The checks of the change's commits come
# next, as the gate's forms start: their sign-offs (`just dco`), their
# subjects (`just commit-messages`), the floor guard and the scan for
# secrets; they take seconds, and the gate would otherwise first run them at
# merge. Then the checks the change's own gate runs at merge, from `xtask
# affected`'s `gate=` line, as `just merge` picks its form (ADR 0135): a
# change of documents alone (`gate=docs`) takes `just lint-documents`, the
# checks of `just lint` that read documents, and no code lint or suite, as
# the docs gate does; any other change takes the licence allowlist (`just
# notices`), `just lint` and the Quick suite over what changed, and, when
# the change reaches what only xtask's codegen tests check (`xtask
# affected`'s `codegen-tests=true`), those tests (`just codegen-tests`),
# which the Quick suite, running no `tests/` binary, misses. `all` takes
# the allowlist, every lint and the Quick suite over the whole workspace,
# whatever changed.
#
# What changed is the working tree against its merge base with the change's
# base: `GOODFETCH_QUICK_BASE` when the pre-push hook gives it (the commit
# the push's remote ref holds, when the push only adds the branch's own
# commits to it), else the branch's own base (ci/change-base.sh): its
# upstream off `main` when it is another branch (the integration branch a
# ticket goes into), else `origin/main`. The branch's own remote copy, its
# upstream after `git push -u`, is never the base: against it a pushed
# branch changed nothing, so no test would run and the run would still
# record the tree. The base is read once, first, and every step that reads
# the change's commits gets it: the commit checks, the floor guard, the scan
# for secrets, `xtask affected` and the Quick suite, `all`'s commit checks
# too, so on an integration branch they judge the ticket's own commits, not
# the earlier tickets'.
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
# `just quick` never runs twice on one tree (ADR 0135's amendment of
# 2026-11-22): a run on a clean checkout is recorded, green or red, in the
# gate's records every worktree shares (`just _quick-record`), by the layer
# that ran it, `pre-push` when the pre-push hook started it (its
# `GOODFETCH_GATE_LAYER`), else `quick`, with the result of the steps after
# the commit checks, since a red commit is mended by a rewrite that keeps
# the tree;
# and a run whose key a green run already holds (`just _quick-held`: the same
# tree, form and merge base with the base, on this host and in this
# environment, within a day, with lint's other reads unchanged) runs only the
# checks of the change's commits, which read more than the tree, and no lint
# or suite. A run on uncommitted changes is neither held nor recorded.
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

# HEAD when the working tree is HEAD's, untracked files included; nothing on
# uncommitted changes or outside a repository.
clean_head() {
  git rev-parse --is-inside-work-tree > /dev/null 2>&1 || return 0
  [[ -z $(git status --porcelain 2>/dev/null) ]] || return 0
  git rev-parse HEAD 2>/dev/null || true
}

# Records the run's result, `pass` or `fail`, for the head it started on; a
# record that fails costs a rerun later and fails no run.
record_run() {
  [[ -n $head ]] || return 0
  just _quick-record "$layer" "$head" "$base" "$form" "$1" \
    || echo "warning: \`just quick\`'s run was not recorded, so the next one on this tree runs again." >&2
}

# shellcheck source=ci/keep-going.sh
source "$(dirname "${BASH_SOURCE[0]}")/keep-going.sh"
# shellcheck source=ci/output-value.sh
source "$(dirname "${BASH_SOURCE[0]}")/output-value.sh"

# shellcheck source=ci/change-base.sh
source "$(dirname "${BASH_SOURCE[0]}")/change-base.sh"

base=${GOODFETCH_QUICK_BASE:-}
if [[ -z $base ]]; then
  if branch=$(own_remote_copy); then
    echo "The upstream of $branch is its own remote copy, which is no base for its change, so the change is read against origin/main. When the branch goes into an integration branch, give that branch as GOODFETCH_QUICK_BASE."
  fi
  base=$(branch_base)
fi
echo "The change: the working tree against its merge base with $base."
quoted_base=$(printf '%q' "$base")

# The run's key: the head of a clean checkout, the form and the layer.
head=$(clean_head)
form=${all:-changed}
case ${GOODFETCH_GATE_LAYER:-} in
  pre-push) layer=pre-push ;;
  *) layer=quick ;;
esac
if [[ -z $head ]]; then
  echo "The working tree differs from HEAD, so this run is neither held nor recorded for the next one."
fi

rm -f target/skipped.txt
trap 'just _skips-report' EXIT
steps_begin quick
step compiler-cache 'just compiler-cache'
held=""
if [[ -n $head ]] && just _quick-held "$head" "$base" "$form"; then
  held=yes
fi
step dco "just dco $quoted_base"
step commit-messages "just commit-messages $quoted_base"
step floor-guard "just floor-guard $quoted_base"
step commit-leaks "just commit-leaks $quoted_base"

if [[ -n $held ]]; then
  echo "No lint and no suite runs again: only the checks of the change's commits ran, which read more than the tree."
  steps_end
  exit 0
fi

# What the run records is the tree's result, the steps after this mark: a
# red commit check is mended by rewriting the commit, which keeps the tree.
tree_steps=$(steps_mark)
if [[ -z $all ]]; then
  change=(--local --since "$base")
  if ! selection=$(cargo run --quiet --locked --bin xtask -- affected "${change[@]}"); then
    echo "error: \`xtask affected\` failed, so no lint and no Quick suite ran: the tier cannot tell which checks the change takes" >&2
    steps_end || true
    record_run fail
    exit 1
  fi
  if ! gate=$(output_value gate "$selection"); then
    echo "error: \`xtask affected\` gave no gate, so no lint and no Quick suite ran: the tier cannot tell which checks the change takes" >&2
    steps_end || true
    record_run fail
    exit 1
  fi
  if ! codegen=$(output_value codegen-tests "$selection"); then
    echo "error: \`xtask affected\` gave no codegen-tests line, so no lint and no Quick suite ran: the tier cannot tell whether the change takes xtask's codegen tests" >&2
    steps_end || true
    record_run fail
    exit 1
  fi
  if [[ $gate == docs ]]; then
    echo "The change is documents alone (xtask affected's gate=docs), so the pre-push tier runs the docs gate's checks: \`just lint-documents\`; the code lints and the Quick suite do not run, as at the merge (ADR 0135)."
    step lint-documents 'just lint-documents'
  else
    step notices 'just notices'
    step lint 'just lint'
    suite=$(printf ' %q' "${change[@]}")
    step quick-suite "just test-affected quick$suite"
    # xtask's codegen tests are `tests/` tests, which the Quick suite never
    # runs: a change to what only they check runs them too, as the quick
    # gate does (issue 2892).
    if [[ $codegen == true ]]; then
      step codegen-tests "just codegen-tests$suite"
    fi
  fi
else
  step notices 'just notices'
  step lint 'just lint'
  step quick-suite 'just _suite quick all'
fi

if steps_red_since "$tree_steps"; then
  record_run fail
else
  record_run pass
fi
steps_end
