#!/usr/bin/env bash
# The hosted tier's planning job (ADR 0135's amendment of 2026-11-22), on
# its bare standard runner: `ci/hosted/plan.sh <base> <branch> <run>`, on the
# checked-out head of a batch's, a spec's or a fork's pull request into
# `<base>`, `<run>` the job's own run. It prints two lines for the job's
# outputs, each named after the job that runs its steps, a JSON list: the
# quick form's hosted-owned steps (lint, the Quick suite, the Journey task
# lists, xtask's codegen tests) that the gate's planner runs in full or in
# part, in the form's order, `lint=` lint, which runs before the deny, and
# `quick=` the others. Every other line goes to stderr, for the job's log.
#
# A documents-only change takes the docs gate, which runs none of those
# steps. Otherwise it downloads the records of the runs on the head's
# ancestors (ci/hosted-records.sh ancestors), with GH_TOKEN, and plans from
# them without it, as `just merge` plans on the gate host, which holds no
# token: so a lint a run on an ancestor holds is reused, and a plan made with
# the token would rerun it (ADR 0135's note on lint's online audits).
set -euo pipefail

usage="usage: ci/hosted/plan.sh <base> <branch> <run>"
base=${1:?$usage}
branch=${2:?$usage}
run=${3:?$usage}
cd "$(dirname "$0")/../.."
# shellcheck source=ci/output-value.sh
source ci/output-value.sh

selection=$(cargo run --quiet --locked --bin xtask -- affected --base "origin/$base" --head HEAD)
gate=$(output_value gate "$selection")
echo "xtask affected: gate=$gate" >&2
if [[ $gate == docs ]]; then
  echo "The docs gate runs no hosted step: the change is documents alone." >&2
  printf 'lint=[]\nquick=[]\n'
  exit 0
fi

records=${GOODFETCH_GATE_HOSTED_RECORDS:?the job sets GOODFETCH_GATE_HOSTED_RECORDS}
repository=${GOODFETCH_GATE_REPOSITORY:?the job sets GOODFETCH_GATE_REPOSITORY}
ci/hosted-records.sh ancestors "$repository" HEAD "origin/$base" "$branch" "$records" "$run" >&2

listing=$(cargo run --quiet --locked --bin xtask -- gate steps --form quick)
plan=$(env -u GH_TOKEN -u GITHUB_TOKEN ci/gate-plan.sh quick hosted)
echo "$plan" >&2
order=$(output_value steps "$listing")
read -ra steps <<<"$order"
lint=()
quick=()
for step in "${steps[@]}"; do
  owner=$(output_value "$step.owner" "$listing")
  [[ $owner == hosted ]] || continue
  action=$(output_value "$step.action" "$plan")
  case $action in
    run | part) if [[ $step == lint ]]; then lint+=("$step"); else quick+=("$step"); fi ;;
    *) ;;
  esac
done
list() { jq --compact-output --null-input '$ARGS.positional' --args "$@"; }
printf 'lint=%s\n' "$(list ${lint[@]+"${lint[@]}"})"
printf 'quick=%s\n' "$(list ${quick[@]+"${quick[@]}"})"
