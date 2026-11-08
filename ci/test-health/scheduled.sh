#!/usr/bin/env bash
# `just test-health <constrained|mutants> [ref]`: the test-health runs the
# gate host's cron starts while ADR 0135 keeps GitHub Actions to the
# Release suite (TESTING.md, After merge; the entries in
# ci/test-health.crontab), on `main`'s head by default. Both run in the
# backstop's gate worktree (ci/gate-worktree.sh's `gate-backstop`), taking
# its lock, so they never run beside the backstop or each other and reuse
# its build; neither holds `just merge`'s worktree, and neither starts while
# `just merge`'s gate runs.
#   constrained  the constrained test-health run (ADR 0064): the Pre-merge
#                suite on half the host's CPU and memory, with a fresh order
#                seed and Vitest's `detectAsyncLeaks` (ci/test-health/
#                constrained.sh), within its 120 minutes. Its egress deny is
#                the hosted job's alone: the gate host keeps no deny.
#   mutants      ADR 0072's mutation testing, advisory: `just mutants` over
#                the Rust that `main` changed in the last 24 hours, within
#                150 minutes; a surviving mutant fails nothing, and the
#                report is kept beside the log.
# Each run appends one line to `<log>/results.txt`, `<UTC day> <commit>
# <run>=<pass|fail>`, or `<run>=skipped (…)` when `just merge`'s gate runs,
# the disk lacks room or the worktree is busy; a run that ends on its setup,
# before its tests, reads `fail`. Its log is `<log>/<day>-<run>.log`, with
# `<log>` GOODFETCH_TEST_HEALTH_LOG or ~/test-health. It fails when the run
# fails or is skipped. It keeps to bash 3.2, which stock macOS runs.
set -euo pipefail

run=${1:?usage: just test-health <constrained|mutants> [ref]}
ref=${2:-origin/main}
case $run in
  constrained) minutes=120 ;;
  mutants) minutes=150 ;;
  *)
    echo "error: the run is constrained or mutants, not $run" >&2
    exit 2
    ;;
esac
logs=${GOODFETCH_TEST_HEALTH_LOG:-$HOME/test-health}
mkdir -p "$logs"
day=$(date -u +%F)
exec >>"$logs/$day-$run.log" 2>&1
echo "== $(date -u +%FT%TZ) the $run test-health run on $ref"

commit=unknown
said=""
locked=""
result() {
  echo "$day $commit $run=$1" >>"$logs/results.txt"
  said=yes
}
# `set -e` ends the run on a setup that fails: its line still says so.
trap '[[ -n $said ]] || result fail; [[ -z $locked ]] || rm -rf "$lock"' EXIT

git fetch --quiet origin
commit=$(git rev-parse --verify "$ref^{commit}")

gate_worktree_name="gate-backstop"
# shellcheck source=ci/gate-worktree.sh
source "$(dirname "$0")/../gate-worktree.sh"
# A run beside `just merge`'s gate takes its cores: on 2026-11-08 the daily
# mutation run's load failed a gate on a leak timeout. A lock whose holder
# ended holds nothing, and is the next gate's to clear.
holder=$(gate_worktree_holder "$root/.claude/worktrees/merge-gate.lock")
if [[ -n $holder ]]; then
  echo "== skipped: the merge gate, process $holder, is running"
  result "skipped (the merge gate is running)"
  exit 1
fi
# The backstop's floor (ci/gate-backstop.sh): the host keeps 60 GB free, and
# the worktree's first build takes about 25 GB, a later one 5 GB
# (GOODFETCH_TEST_HEALTH_MIN_FREE_GB sets the floor itself).
if [[ -d "$worktree/target" ]]; then needed=65; else needed=85; fi
needed=${GOODFETCH_TEST_HEALTH_MIN_FREE_GB:-$needed}
free=$(df -Pk "$root" | awk 'NR == 2 { print int($4 / 1048576) }')
if ((free < needed)); then
  echo "== skipped: $free GB free, the run needs $needed GB"
  result "skipped (free disk $free GB < $needed GB)"
  exit 1
fi
holder=$(gate_worktree_holder "$lock")
if [[ -n $holder ]]; then
  echo "== skipped: process $holder holds $worktree"
  result "skipped (the backstop's worktree is busy)"
  exit 1
fi
gate_worktree_lock
locked=yes
gate_worktree_checkout "$commit"
cd "$worktree"
just fetch

# Cron gives a run no user session: both runs start the OS services their tests
# use and find them through the setup's environment file, as `just pre-merge`
# does, so a missing one fails instead of skipping its tests.
if [[ "$(uname -s)" == Linux ]]; then
  just _os-services
  set -a
  # shellcheck source=/dev/null
  source "${RUNNER_TEMP:-/tmp}/goodfetch-os-services/env"
  set +a
  export GOODFETCH_OS_SERVICES=required
  export GOODFETCH_TEST_DNS_NAMESPACE=${GOODFETCH_TEST_DNS_NAMESPACE:-sudo}
fi

status=0
case $run in
  constrained)
    # A fresh seed, as the hosted job draws it, printed to replay the order.
    GOODFETCH_ORDER_SEED=$(shuf --input-range=1-2147483647 --head-count=1)
    export GOODFETCH_ORDER_SEED
    echo "== order seed $GOODFETCH_ORDER_SEED"
    timeout "${minutes}m" ci/test-health/constrained.sh || status=$?
    ;;
  mutants)
    base=$(git rev-list -1 --first-parent --before="24 hours ago" "$commit")
    [[ -n $base ]] || base=$commit
    echo "== mutants of the Rust changed since $base"
    timeout "${minutes}m" just mutants "$base" || status=$?
    if [[ -d target/mutants ]]; then
      rm -rf "$logs/$day-mutants"
      cp -R target/mutants "$logs/$day-mutants"
    fi
    ;;
esac
if ((status == 124)); then echo "== stopped after $minutes minutes"; fi
if ((status == 0)); then result pass; else result fail; fi
exit "$status"
