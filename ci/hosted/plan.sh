#!/usr/bin/env bash
# The hosted tier's planning job (ADR 0135's amendment of 2026-11-22), on
# its bare standard runner: `ci/hosted/plan.sh <base> <branch> <run> <pull
# request>`, on the checked-out head of a batch's, a spec's or a fork's pull
# request into `<base>`, `<run>` the job's own run. It prints the job's
# outputs, one line each, for the jobs that run the gate's hosted-owned
# steps:
#   form=      the gate's form the jobs plan with, as `just merge` picks it
#              (ci/merge-pr.sh): `all` on the pull request that finishes a
#              spec, from its `spec/<n>` branch with none of the spec's
#              sub-issues left open (ci/spec-sub-issues.sh, #3382), when a
#              test reads the change; `batch` on a batch's branch
#              (`batch/…`, issues 3378 and 3380), whose run also holds the
#              Pre-merge suite of every member it reaches; else `quick`;
#   lint=      a JSON list of the steps that run before the deny, each a
#              job of its own, when the planner runs them in full or in
#              part: lint, whose zizmor reads GitHub, and the proofs, whose
#              audit downloads the pinned advisory snapshot (ci/proofs/run.sh);
#   visual=    `["visual"]` when the planner runs the visual tests, which
#              run in the pinned Playwright image beside the job's container
#              (ci/hosted/visual.sh), else `[]`;
#   quick=     a JSON list of the form's other hosted-owned steps the planner
#              runs (the Quick suite, the Journey task lists, xtask's codegen
#              tests, and in the full form the feature tests, the Kennel's
#              check and the MSRV build), in the form's order, each a job
#              of its own;
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
# archive, one job each, never more partitions than crates. The build job
# ends before the shards start and the record job after they end, and the
# workflow runs at most two of the `quick` list's jobs at once: with the
# shards, the two jobs before the deny, the visual tests' job and the
# Chromium build gate, a run holds at most 10 jobs at once (ADR 0061).
set -euo pipefail

usage="usage: ci/hosted/plan.sh <base> <branch> <run> <pull request>"
base=${1:?$usage}
branch=${2:?$usage}
run=${3:?$usage}
number=${4:?$usage}
shard_count=4
cd "$(dirname "$0")/../.."
# shellcheck source=ci/output-value.sh
source ci/output-value.sh

selection=$(cargo run --quiet --locked --bin xtask -- affected --base "origin/$base" --head HEAD)
gate=$(output_value gate "$selection")
echo "xtask affected: gate=$gate" >&2

form=quick
[[ $branch != batch/* ]] || form="batch"
if [[ $gate == full && $branch =~ ^spec/[0-9]+$ ]]; then
  # An answer that cannot be read keeps the quick form: `just merge`, which
  # asks it again, then runs the full gate's other steps itself.
  spec=${branch#spec/}
  if ! left_open=$(ci/spec-sub-issues.sh "$spec" "$number") || [[ ! $left_open =~ ^(0|[1-9][0-9]*)$ ]]; then
    echo "warning: the sub-issues of spec #$spec that stay open after pull request #$number could not be read (\"${left_open:-}\"), so the run plans the quick form" >&2
  elif ((left_open == 0)); then
    form=all
    echo "The pull request finishes spec #$spec: the run plans the full form." >&2
  else
    echo "Spec #$spec is not finished: $left_open of its sub-issues stay open after this pull request." >&2
  fi
fi
printf 'form=%s\n' "$form"

if [[ $gate == docs ]]; then
  echo "The docs gate runs no hosted step: the change is documents alone." >&2
  printf 'lint=[]\nvisual=[]\nquick=[]\npre_merge=\npre-merge-build=[]\npre-merge-shard=[]\npre-merge-record=[]\n'
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
visual=()
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
    lint:run | lint:part | proofs:run | proofs:part) lint+=("$step") ;;
    visual:run) visual+=("$step") ;;
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
printf 'visual=%s\n' "$(list ${visual[@]+"${visual[@]}"})"
printf 'quick=%s\n' "$(list ${quick[@]+"${quick[@]}"})"
printf 'pre_merge=%s\n' "${members[*]-}"
printf 'pre-merge-build=%s\n' "$(list ${held[@]+"${held[@]}"})"
printf 'pre-merge-shard=%s\n' "$(list ${shards[@]+"${shards[@]}"})"
printf 'pre-merge-record=%s\n' "$(list ${held[@]+"${held[@]}"})"
