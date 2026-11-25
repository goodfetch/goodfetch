#!/usr/bin/env bash
# `ci/hosted-records.sh ancestors <repository> <head> <base> <branch> <folder> <run>`,
# or `ci/hosted-records.sh tree <repository> <commit> <folder>`:
# downloads into `folder` the gate records the hosted tier's runs uploaded
# (ADR 0135's amendment of 2026-11-22), one folder a run, each job's
# `gate-record-<step>` artifact in it, for `xtask gate plan
# --hosted-records` (GOODFETCH_GATE_HOSTED_RECORDS). It waits for no run and
# cancels none; `just merge` no longer calls it (ADR 0135's note of
# 2026-11-25).
#
# It reads a run only when GitHub's API says all of these: it is a run of
# the repository's own `.github/workflows/pull-request.yml`, for a
# `pull_request` event, whose head repository is `repository`, and whose head
# commit is `head` or one of its first-parent ancestors that `base` lacks. So
# no other workflow, fork or commit stands in for a result; the plan then
# counts a record only under the CI image the gated tree pins.
#
# `ancestors`, for the hosted tier's planning job: the runs that ended on
# `head`'s first-parent ancestors, and on `head` itself, but `run`, the job's
# own.
#
# `tree`, for the Release suite's Linux x64 cell (ci/release/reuse.sh,
# #3384): the runs that ended on `commit` or on one of its parents whose
# tree is `commit`'s, such as the pull request's head that `just merge`
# gated and merged as that commit. Its rules are those above, `commit` and
# those parents standing for `head` and its ancestors.
#
# A run's records that cannot be downloaded only warn: the units they hold
# run again. Every line it prints names the run, its head and its page.
# It keeps to bash 3.2, which stock macOS runs.
set -euo pipefail

usage="usage: ci/hosted-records.sh ancestors <repository> <head> <base> <branch> <folder> <run>, or tree <repository> <commit> <folder>"
mode=${1:?$usage}
repository=${2:?$usage}
head=${3:?$usage}
base="" branch="" own=""
case $mode in
  ancestors)
    base=${4:?$usage}
    branch=${5:?$usage}
    folder=${6:?$usage}
    own=${7:?$usage}
    ;;
  tree) folder=${4:?$usage} ;;
  *)
    echo "error: $usage" >&2
    exit 2
    ;;
esac
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
# `<id>\t<status>\t<conclusion>\t<head>\t<page>`,
# the conclusion `none` until the run ends, since `read` would merge an
# empty field with the tabs around it.
# The repository is an `owner/name` GitHub gave, so it is safe in the filter.
# Each listing of `tree` keeps its own commit's runs alone, so none is read
# twice; a commit is hexadecimal, so it is safe in the filter too.
filter() {
  echo ".workflow_runs[]
    | select(.path == \"$workflow\" and .event == \"pull_request\" and .head_repository.full_name == \"$repository\"$1)
    | [.id, .status, (.conclusion // \"none\"), .head_sha, .html_url]
    | @tsv"
}
runs() {
  local listed id status conclusion sha page candidate
  if [[ $mode == tree ]]; then
    listed=""
    for candidate in $candidates; do
      listed+=$(gh api "$endpoint/runs?event=pull_request&head_sha=$candidate&per_page=100" \
        --jq "$(filter " and .head_sha == \"$candidate\"")")$'\n'
    done
  else
    listed=$(gh api "$endpoint/runs?event=pull_request&branch=$branch&per_page=100" --jq "$(filter "")")
  fi
  while IFS=$'\t' read -r id status conclusion sha page; do
    [[ -n $id ]] || continue
    grep -qx "$sha" <<<"$candidates" || continue
    printf '%s\t%s\t%s\t%s\t%s\n' "$id" "$status" "$conclusion" "$sha" "$page"
  done <<<"$listed"
}

# The runs, listed. A listing GitHub cannot give ends this with nothing
# downloaded: the plan runs every unit no local record holds.
if ! listed=$(runs); then
  echo "warning: GitHub did not list the hosted runs of ${branch:-$head}, so the gate runs every unit no local record holds" >&2
  exit 0
fi

# Every run listed that has ended, the job's own aside, its records
# downloaded into a folder of its own.
rm -rf "$folder"
mkdir -p "$folder"
while IFS=$'\t' read -r id status conclusion sha page; do
  [[ -n $id && $id != "$own" && $status == completed ]] || continue
  if gh run download "$id" --pattern 'gate-record-*' --dir "$folder/$id" >/dev/null 2>&1; then
    echo "The hosted run $id on $sha ($page), $conclusion: its gate records are read"
  else
    rm -rf "${folder:?}/$id"
    echo "warning: the gate records of the hosted run $id on $sha ($page) could not be downloaded; the units they hold run again" >&2
  fi
done <<<"$listed"
