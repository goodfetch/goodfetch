#!/usr/bin/env bash
# `just quick [all]`: the pre-push tier (T1), which `.githooks/pre-push`
# runs (TESTING.md, Commands). The floor guard and the scan of the change's
# commits for secrets come first: they take seconds, and the gate would
# otherwise first run them at merge. Then the checks the change's own gate
# runs at merge, from `xtask affected`'s `gate=` line, as `just merge` picks
# its form (ADR 0135): a change of documents alone (`gate=docs`) takes
# `just lint-documents`, the checks of `just lint` that read documents, and
# no code lint or suite, as the docs gate does; any other change takes
# `just lint` and the Quick suite over what changed. `all` takes every lint
# and the Quick suite over the whole workspace, whatever changed.
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

rm -f target/skipped.txt
trap 'just _skips-report' EXIT
just floor-guard
just commit-leaks

if [[ -z $all ]]; then
  # The working tree's change since the merge base with `origin/main`, as
  # the Quick suite's own selection reads it.
  gate=$(cargo run --quiet --locked --bin xtask -- affected --local | sed -n 's/^gate=//p')
  if [[ $gate == docs ]]; then
    echo "The change is documents alone (xtask affected's gate=docs), so the pre-push tier runs the docs gate's checks: \`just lint-documents\`; the code lints and the Quick suite do not run, as at the merge (ADR 0135)."
    just lint-documents
    exit 0
  fi
fi

just lint
just _suite quick "$all"
