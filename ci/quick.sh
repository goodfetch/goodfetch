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
# the push's remote ref holds), else the branch's upstream off `main` (the
# integration branch a ticket goes into), else `origin/main`.
#
# A green run on a clean checkout records its tree in the repository's
# common folder, shared by its worktrees, and the pre-push hook skips a push
# of a recorded tree: `just quick` never runs twice on one tree.
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
  echo "\`just quick\` passed on tree $tree: the pre-push hook skips a push of it."
}

rm -f target/skipped.txt
trap 'just _skips-report' EXIT
just compiler-cache
just floor-guard
just commit-leaks

if [[ -z $all ]]; then
  change=(--local)
  base=${GOODFETCH_QUICK_BASE:-}
  if [[ -z $base ]]; then
    branch=$(git symbolic-ref --quiet --short HEAD 2>/dev/null || true)
    if [[ -n $branch && $branch != main ]]; then
      base=$(git rev-parse --abbrev-ref --symbolic-full-name '@{upstream}' 2>/dev/null || true)
    fi
  fi
  [[ -z $base ]] || change+=(--since "$base")
  echo "The change: the working tree against its merge base with ${base:-origin/main}."
  gate=$(cargo run --quiet --locked --bin xtask -- affected "${change[@]}" | sed -n 's/^gate=//p')
  if [[ $gate == docs ]]; then
    echo "The change is documents alone (xtask affected's gate=docs), so the pre-push tier runs the docs gate's checks: \`just lint-documents\`; the code lints and the Quick suite do not run, as at the merge (ADR 0135)."
    just lint-documents
    record_green
    exit 0
  fi
fi

just lint
if [[ -n $all ]]; then
  just _suite quick all
else
  just test-affected quick "${change[@]}"
fi
record_green
