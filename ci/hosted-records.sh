#!/usr/bin/env bash
# `ci/hosted-records.sh await|ancestors <repository> <head> <base> <branch> <folder> [<run>]`,
# or `ci/hosted-records.sh tree <repository> <commit> <folder>`:
# downloads into `folder` the gate records the hosted tier's runs uploaded
# (ADR 0135's amendment of 2026-11-22), one folder a run, each job's
# `gate-record-<step>` artifact in it, for `xtask gate plan
# --hosted-records` (GOODFETCH_GATE_HOSTED_RECORDS).
#
# It reads a run only when GitHub's API says all of these: it is a run of
# the repository's own `.github/workflows/pull-request.yml`, for a
# `pull_request` event, whose head repository is `repository`, and whose head
# commit is `head` or one of its first-parent ancestors that `base` lacks. So
# no other workflow, fork or commit stands in for a result; the plan then
# counts a record only under the CI image the gated tree pins.
#
# `await`, for `just merge` (ci/merge-pr.sh): it waits for the run on `head`,
# which a push of the pull request's branch starts, up to
# GOODFETCH_HOSTED_APPEAR seconds (120) to appear and then until it ends,
# polling every GOODFETCH_HOSTED_POLL seconds (10). Once a run still going is
# GOODFETCH_HOSTED_CAP seconds (1800, ADR 0061's job cap) past its start, it
# cancels it and the gate runs what no record holds itself; a run it may not
# cancel it waits for to its end, so no unit runs on two layers at once. A
# run on `head` that ended red ends this red, naming each failed job and its
# page (ADR 0064): the gate runs nothing of it again. A run that appears
# after the wait is cancelled before the gate starts its steps. When the
# workflow is not active, or GitHub cannot say, no hosted run serves the
# gate: it says so and downloads nothing.
#
# `ancestors`, for the hosted tier's planning job: the runs that ended on
# `head`'s first-parent ancestors, and on `head` itself, but `run`, the job's
# own, waiting for none, so the run on the merge with `main`'s tip that the
# gate pushes plans from the batch head's records.
#
# `tree`, for the Release suite's Linux x64 cell (ci/release/reuse.sh,
# #3384): the runs that ended on `commit` or on one of its parents whose
# tree is `commit`'s, such as the pull request's head that `just merge`
# gated and merged as that commit, waiting for none. Its rules are those
# above, `commit` and those parents standing for `head` and its ancestors.
#
# A run's records that cannot be downloaded only warn: the units they hold
# run again. Every line it prints names the run, its head and its page.
# It keeps to bash 3.2, which stock macOS runs.
set -euo pipefail

usage="usage: ci/hosted-records.sh await|ancestors <repository> <head> <base> <branch> <folder> [<run>], or tree <repository> <commit> <folder>"
mode=${1:?$usage}
repository=${2:?$usage}
head=${3:?$usage}
base="" branch="" own=""
case $mode in
  await | ancestors)
    base=${4:?$usage}
    branch=${5:?$usage}
    folder=${6:?$usage}
    own=${7:-}
    ;;
  tree) folder=${4:?$usage} ;;
  *)
    echo "error: $usage" >&2
    exit 2
    ;;
esac
appear=${GOODFETCH_HOSTED_APPEAR:-120}
poll=${GOODFETCH_HOSTED_POLL:-10}
cap=${GOODFETCH_HOSTED_CAP:-1800}
workflow=.github/workflows/pull-request.yml
endpoint="repos/{owner}/{repo}/actions/workflows/pull-request.yml"

# The commits whose runs count, one a line: `head` and its first-parent
# ancestors that `base` lacks, or, for `tree`, `head` and its parents whose
# tree is its own.
if [[ $mode == tree ]]; then
  head=$(git rev-parse --verify "$head^{commit}")
  tree=$(git rev-parse --verify "$head^{tree}")
  candidates=$head
  for parent in $(git rev-list --parents --max-count=1 "$head" | cut -d ' ' -f 2-); do
    [[ $(git rev-parse --verify "$parent^{tree}") != "$tree" ]] || candidates+=$'\n'$parent
  done
else
  candidates=$(git rev-list --first-parent "$head" --not "$base")
fi

# The runs GitHub lists for the branch, or for `tree` on each candidate,
# that pass the rules above, one a line:
# `<id>\t<status>\t<conclusion>\t<head>\t<started, in Unix seconds>\t<page>`,
# the conclusion `none` until the run ends, since `read` would merge an
# empty field with the tabs around it.
# The repository is an `owner/name` GitHub gave, so it is safe in the filter.
# Each listing of `tree` keeps its own commit's runs alone, so none is read
# twice; a commit is hexadecimal, so it is safe in the filter too.
filter() {
  echo ".workflow_runs[]
    | select(.path == \"$workflow\" and .event == \"pull_request\" and .head_repository.full_name == \"$repository\"$1)
    | [.id, .status, (.conclusion // \"none\"), .head_sha, ((.run_started_at // .created_at) | fromdateiso8601), .html_url]
    | @tsv"
}
runs() {
  local listed id status conclusion sha started page candidate
  if [[ $mode == tree ]]; then
    listed=""
    for candidate in $candidates; do
      listed+=$(gh api "$endpoint/runs?event=pull_request&head_sha=$candidate&per_page=100" \
        --jq "$(filter " and .head_sha == \"$candidate\"")")$'\n'
    done
  else
    listed=$(gh api "$endpoint/runs?event=pull_request&branch=$branch&per_page=100" --jq "$(filter "")")
  fi
  while IFS=$'\t' read -r id status conclusion sha started page; do
    [[ -n $id ]] || continue
    grep -qx "$sha" <<<"$candidates" || continue
    printf '%s\t%s\t%s\t%s\t%s\t%s\n' "$id" "$status" "$conclusion" "$sha" "$started" "$page"
  done <<<"$listed"
}

# Lists the runs into `listed`, and their newest on `head` into `run`. A
# listing GitHub cannot give ends this with nothing downloaded: the gate runs
# every unit no local record holds, as without the hosted tier.
list() {
  if ! listed=$(runs); then
    echo "warning: GitHub did not list the hosted runs of ${branch:-$head}, so the gate runs every unit no local record holds" >&2
    exit 0
  fi
  run=$(awk -F '\t' -v head="$head" '$4 == head { print; exit }' <<<"$listed")
}

if [[ $mode == await ]]; then
  if ! state=$(gh api "$endpoint" --jq .state) || [[ $state != active ]]; then
    echo "No hosted run serves this gate: the pull-request workflow is ${state:-unreadable on GitHub}, so the gate runs every unit no local record holds"
    exit 0
  fi
fi

listed=""
run=""
list
if [[ $mode == await ]]; then
  waited=$SECONDS
  while [[ -z $run ]] && ((SECONDS - waited < appear)); do
    sleep "$poll"
    list
  done
  if [[ -z $run ]]; then
    echo "No hosted run on $head appeared within $appear s: the gate runs every unit no record holds"
  fi
  said=""
  cancelled=""
  while [[ -n $run ]]; do
    IFS=$'\t' read -r id status conclusion _ started page <<<"$run"
    [[ $status != completed ]] || break
    # A cancelled run that has not ended a while later holds the gate no
    # more: the check below cancels it again before any step.
    [[ $cancelled != yes ]] || ((SECONDS - cancelled_at < appear)) || break
    if [[ -z $cancelled ]] && (($(date -u +%s) - started >= cap)); then
      if gh run cancel "$id" >/dev/null; then
        echo "The hosted run $id on $head ($page) is past its $cap s cap: the gate cancelled it, and runs what it left itself"
        cancelled=yes
        cancelled_at=$SECONDS
      else
        echo "The hosted run $id on $head ($page) is past its $cap s cap, and the gate may not cancel it: the gate waits for its end, so no unit runs beside it"
        cancelled=refused
      fi
    elif [[ -z $said ]]; then
      echo "Waiting for the hosted run $id on $head ($page), $status, so the gate runs none of its units beside it"
      said=yes
    fi
    sleep "$poll"
    list
  done
  if [[ -n $run ]]; then
    IFS=$'\t' read -r id status conclusion _ _ page <<<"$run"
    case $conclusion in
      failure | timed_out | startup_failure)
        jobs=$(gh api "repos/{owner}/{repo}/actions/runs/$id/jobs?per_page=100" \
          --jq '.jobs[] | select(.conclusion == "failure" or .conclusion == "timed_out") | "  \(.name): \(.html_url)"') || jobs=""
        {
          echo "error: the hosted run $id on $head ended $conclusion ($page), so the gate is red and runs none of it again (ADR 0064):"
          [[ -z $jobs ]] || printf '%s\n' "$jobs"
        }
        exit 1
        ;;
    esac
  fi
fi

# Every run listed that has ended, the job's own aside, its records
# downloaded into a folder of its own.
rm -rf "$folder"
mkdir -p "$folder"
while IFS=$'\t' read -r id status conclusion sha _ page; do
  [[ -n $id && $id != "$own" && $status == completed ]] || continue
  if gh run download "$id" --pattern 'gate-record-*' --dir "$folder/$id" >/dev/null 2>&1; then
    echo "The hosted run $id on $sha ($page), $conclusion: its gate records are read"
  else
    rm -rf "${folder:?}/$id"
    echo "warning: the gate records of the hosted run $id on $sha ($page) could not be downloaded; the units they hold run again" >&2
  fi
done <<<"$listed"

# A run on `head` that is still going now, one that appeared after the
# wait, is cancelled, so no unit runs on two layers.
if [[ $mode == await ]]; then
  list
  if [[ -n $run ]]; then
    IFS=$'\t' read -r id status _ _ _ page <<<"$run"
    if [[ $status != completed ]]; then
      if gh run cancel "$id" >/dev/null; then
        echo "The hosted run $id on $head ($page) is still going: the gate cancelled it, and runs what it left itself"
      else
        echo "error: the hosted run $id on $head ($page) is still going, and the gate may not cancel it: run \`just merge\` again once it ends" >&2
        exit 1
      fi
    fi
  fi
fi
