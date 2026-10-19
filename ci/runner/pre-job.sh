#!/bin/bash
# The owner's machine's pre-job script (ADRs 0108 and 0135), which the runner
# runs before each job's first step through ACTIONS_RUNNER_HOOK_JOB_STARTED.
# A non-zero exit fails the job before any of its steps runs.
#
# It admits a job only when its repository, workflow, job, event and ref are
# on the owner's installed copy of the allowlist (allowlist.tsv beside this
# script, written by `just runner-install`), and, for a pull request, when
# the pull request's head is a branch of this repository, never a fork. It
# then destroys any VM an earlier job left behind and takes the lock the
# machine's runner instances share, so one job runs at a time.
#
# It uses only what stock macOS has (bash 3.2, plutil, ps) and Lima.
set -euo pipefail
export PATH="/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin"

here=$(cd "$(dirname "$0")" && pwd)
list="$here/allowlist.tsv"
lock_parent=${GOODFETCH_LOCK_FOLDER:-/Users/Shared/goodfetch-runner-lock}

refuse() {
  echo "goodfetch pre-job: refused, $1" >&2
  exit 1
}

[[ -r "$list" ]] || refuse "no allowlist at $list"

repository=${GITHUB_REPOSITORY:-}
event=${GITHUB_EVENT_NAME:-}
ref=${GITHUB_REF:-}
job=${GITHUB_JOB:-}
workflow_ref=${GITHUB_WORKFLOW_REF:-}
[[ -n "$repository" && -n "$event" && -n "$ref" && -n "$job" && -n "$workflow_ref" ]] ||
  refuse "the job's repository, event, ref, job or workflow is unknown"

allowed_repository=$(awk -F'\t' '$1 == "repository" { print $2 }' "$list")
[[ "$repository" == "$allowed_repository" ]] || refuse "repository $repository is not $allowed_repository"

# A pull_request_target job runs with the base's secrets on a fork's code.
[[ "$event" != pull_request_target ]] || refuse "pull_request_target is never admitted"

# owner/name/.github/workflows/<file>@<ref> gives <file>.
workflow=${workflow_ref#"$repository/.github/workflows/"}
workflow=${workflow%%@*}

admitted=false
while IFS=$'\t' read -r kind entry_workflow entry_job entry_event entry_ref; do
  [[ "$kind" == job ]] || continue
  # shellcheck disable=SC2053 # the ref pattern is a glob on purpose
  if [[ "$entry_workflow" == "$workflow" && "$entry_job" == "$job" && "$entry_event" == "$event" && "$ref" == $entry_ref ]]; then
    admitted=true
    break
  fi
done <"$list"
[[ "$admitted" == true ]] || refuse "$workflow#$job on $event at $ref is not on the allowlist"

if [[ "$event" == pull_request ]]; then
  head_repository=$(plutil -extract pull_request.head.repo.full_name raw -o - "${GITHUB_EVENT_PATH:?}" 2>/dev/null || true)
  [[ "$head_repository" == "$repository" ]] || refuse "the pull request comes from ${head_repository:-an unknown repository}, not a branch of $repository"
fi

# A cancelled job may skip the post-job script: destroy what it left.
limactl delete --force goodfetch-job >/dev/null 2>&1 || true

# One job at a time on the machine, across its runner instances: the lock is
# a folder whose holder file names the runner worker holding it. A holder
# whose worker is gone left it behind, and it is taken over: renamed away,
# which only one waiter can do, then removed. The lock and its holder file
# are writable by every user, since each runner instance runs as its own
# user and either may take over the other's.
mkdir -p "$lock_parent" 2>/dev/null || true
holder_of() { cat "$lock_parent/held/holder" 2>/dev/null || true; }
while ! mkdir -m 0777 "$lock_parent/held" 2>/dev/null; do
  holder=$(holder_of)
  holder_pid=${holder##* }
  if [[ -n "$holder_pid" ]] && ! ps -p "$holder_pid" >/dev/null 2>&1; then
    stale="$lock_parent/stale.$$"
    if mv "$lock_parent/held" "$stale" 2>/dev/null; then
      echo "goodfetch pre-job: took over the lock ${holder} left behind" >&2
      rm -rf "$stale" 2>/dev/null || echo "goodfetch pre-job: could not remove $stale" >&2
    fi
    continue
  fi
  echo "goodfetch pre-job: waiting for the job of ${holder:-another runner} to end" >&2
  sleep 15
done
echo "$(id -un) $PPID" >"$lock_parent/held/holder"
chmod 0666 "$lock_parent/held/holder"
echo "goodfetch pre-job: admitted $workflow#$job on $event at $ref" >&2
