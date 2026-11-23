#!/usr/bin/env bash
# The hosted tier's planning job (ADR 0135's amendment of 2026-11-22), on
# its bare standard runner: `ci/hosted/plan.sh <base> <branch> <run>`, on the
# checked-out head of a batch's, a spec's or a fork's pull request into
# `<base>`, `<run>` the job's own run. It prints the job's outputs, one line
# each, for the jobs that run the gate's hosted-owned steps:
#   form=      the gate's form the jobs plan with: `batch` on a batch's
#              branch (`batch/…`, issues 3378 and 3380), whose run also
#              holds the Pre-merge suite of every member it reaches, else
#              `quick`;
#   lint=      a JSON list, `["lint"]` when the planner runs lint in full or
#              in part, which runs before the deny;
#   quick=     a JSON list of the form's other hosted-owned steps the planner
#              runs (the Quick suite, the Journey task lists, xtask's codegen
#              tests), in the form's order, each a job of its own;
#   pre_merge= the members whose Pre-merge suite the run holds, on one line,
#              space-separated, empty when it holds none;
#   pre-merge-build=, pre-merge-record=
#              `["pre-merge"]` when it holds some, else `[]`;
#   pre-merge-shard=
#              a JSON list of the Pre-merge shards' numbers, as strings,
#              `"1"` to `"n"`, `[]` when it holds no crate's: the build
#              job runs the other members' Vitest, Playwright and pytest
#              tests itself (ci/hosted/pre-merge.sh).
# Every other line goes to stderr, for the job's log. A job's JSON list is
# named after it, so "all green" passes a job its list leaves `[]` to
# (`xtask all-green`).
#
# A documents-only change takes the docs gate, which runs none of those
# steps. Otherwise it downloads the records of the runs on the head's
# ancestors (ci/hosted-records.sh ancestors), with GH_TOKEN, and plans from
# them without it, as `just merge` plans on the gate host, which holds no
# token: so a lint a run on an ancestor holds is reused, and a plan made with
# the token would rerun it (ADR 0135's note on lint's online audits).
#
# The Pre-merge suite runs in up to `shard_count` hash partitions of one
# archive, one job each, never more partitions than crates: with the build
# job, the record job, lint, the three other steps' jobs and the Chromium
# build gate, a run holds at most 10 jobs at once (ADR 0061).
set -euo pipefail

usage="usage: ci/hosted/plan.sh <base> <branch> <run>"
base=${1:?$usage}
branch=${2:?$usage}
run=${3:?$usage}
shard_count=4
cd "$(dirname "$0")/../.."
# shellcheck source=ci/output-value.sh
source ci/output-value.sh

form=quick
[[ $branch != batch/* ]] || form="batch"
printf 'form=%s\n' "$form"

selection=$(cargo run --quiet --locked --bin xtask -- affected --base "origin/$base" --head HEAD)
gate=$(output_value gate "$selection")
echo "xtask affected: gate=$gate" >&2
if [[ $gate == docs ]]; then
  echo "The docs gate runs no hosted step: the change is documents alone." >&2
  printf 'lint=[]\nquick=[]\npre_merge=\npre-merge-build=[]\npre-merge-shard=[]\npre-merge-record=[]\n'
  exit 0
fi

records=${GOODFETCH_GATE_HOSTED_RECORDS:?the job sets GOODFETCH_GATE_HOSTED_RECORDS}
repository=${GOODFETCH_GATE_REPOSITORY:?the job sets GOODFETCH_GATE_REPOSITORY}
ci/hosted-records.sh ancestors "$repository" HEAD "origin/$base" "$branch" "$records" "$run" >&2

listing=$(cargo run --quiet --locked --bin xtask -- gate steps --form "$form")
plan=$(env -u GH_TOKEN -u GITHUB_TOKEN ci/gate-plan.sh "$form" hosted)
echo "$plan" >&2
order=$(output_value steps "$listing")
read -ra steps <<<"$order"
lint=()
quick=()
pre_merge=""
for step in "${steps[@]}"; do
  owner=$(output_value "$step.owner" "$listing")
  [[ $owner == hosted ]] || continue
  action=$(output_value "$step.action" "$plan")
  case $step:$action in
    pre-merge:run) pre_merge=$(output_value members "$plan") ;;
    pre-merge:part) pre_merge=$(output_value pre-merge.members "$plan") ;;
    pre-merge:*) ;;
    lint:run | lint:part) lint+=("$step") ;;
    *:run | *:part) quick+=("$step") ;;
    *) ;;
  esac
done
read -ra members <<<"$pre_merge"
held=()
[[ ${#members[@]} -eq 0 ]] || held=(pre-merge)
crates=0
if [[ ${#members[@]} -gt 0 ]]; then
  names=$(cargo metadata --no-deps --format-version 1 --locked | jq -r '.packages[].name')
  for member in "${members[@]}"; do
    if grep -qxF -- "$member" <<<"$names"; then crates=$((crates + 1)); fi
  done
fi
shards=()
for ((shard = 1; shard <= shard_count && shard <= crates; shard++)); do
  shards+=("$shard")
done
list() { jq --compact-output --null-input '$ARGS.positional' --args "$@"; }
printf 'lint=%s\n' "$(list ${lint[@]+"${lint[@]}"})"
printf 'quick=%s\n' "$(list ${quick[@]+"${quick[@]}"})"
printf 'pre_merge=%s\n' "${members[*]-}"
printf 'pre-merge-build=%s\n' "$(list ${held[@]+"${held[@]}"})"
printf 'pre-merge-shard=%s\n' "$(list ${shards[@]+"${shards[@]}"})"
printf 'pre-merge-record=%s\n' "$(list ${held[@]+"${held[@]}"})"
