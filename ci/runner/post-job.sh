#!/bin/bash
# The owner's machine's post-job script (ADR 0135), which the runner runs
# after each job through ACTIONS_RUNNER_HOOK_JOB_COMPLETED: it destroys the
# job's VM and releases the machine's lock if this runner holds it. The
# pre-job script covers a cancelled job that skipped it.
set -uo pipefail
export PATH="/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin"

lock_parent=${GOODFETCH_LOCK_FOLDER:-/Users/Shared/goodfetch-runner-lock}

limactl delete --force goodfetch-job >/dev/null 2>&1 || true

holder=$(cat "$lock_parent/held/holder" 2>/dev/null || true)
if [[ "${holder%% *}" == "$(id -un)" ]]; then
  rm -rf "$lock_parent/held"
fi
exit 0
