#!/usr/bin/env bash
# `just test-health <constrained|mutants|post-merge> [ref]`: the test-health
# runs the gate host's cron starts while ADR 0135 keeps GitHub Actions to the
# Release suite (TESTING.md, After merge; the entries in
# ci/test-health.crontab), on `main`'s head by default. They run in the
# backstop's gate worktree (ci/gate-worktree.sh's `gate-backstop`), taking
# its lock, so they never run beside the backstop or each other and reuse
# its build; none holds `just merge`'s worktree, none starts while
# `just merge`'s gate runs, and a gate started during a run waits for it,
# but for a post-merge run, which it stops.
#   constrained  the constrained test-health run (ADR 0064): the Pre-merge
#                suite on half the host's CPU and memory, with a fresh order
#                seed and Vitest's `detectAsyncLeaks` (ci/test-health/
#                constrained.sh), within its 120 minutes. Its egress deny is
#                the hosted job's alone: the gate host keeps no deny.
#   mutants      ADR 0072's mutation testing, advisory: `just mutants` over
#                the Rust that `main` changed in the last 24 hours, within
#                150 minutes; a surviving mutant fails nothing, and the
#                report is kept beside the log.
#   post-merge   what quick merges skip (the 2026-11-13 retro: regressions
#                surfaced hours later, at the next full gate): `just counts`
#                and the Pre-merge suite over what `main` gained since the
#                commit it last checked (`<log>/post-merge-checked`; on its
#                first run, `main`'s commit before its head), both run even
#                when the first fails, within 90 minutes. Cron starts it every
#                20 minutes: it does nothing while `main` has not moved, and
#                records a head whose merge passed the full gate as checked
#                without running anything.
# Each run appends one line to `<log>/results.txt`, `<UTC day> <commit>
# <run>=<pass|fail>`, or `<run>=skipped (…)` when `just merge`'s gate runs,
# the disk lacks room or the worktree is busy; a run that ends on its setup,
# before its tests, reads `fail`. A post-merge run's line ends with `since
# <checked>`, and it then moves `post-merge-checked` to the commit, pass or
# fail: a red range is reported, not run again. A post-merge run that cannot
# run now, or that a gate stopped, writes no line and moves nothing, so the
# next cron run covers the range. Its log is `<log>/<day>-<run>.log`, with `<log>`
# GOODFETCH_TEST_HEALTH_LOG or ~/test-health. It fails when the run fails or,
# but for a post-merge run, is skipped. A run is the scripts of the commit it
# tests, whichever script started it: nothing fast-forwards the checkout cron
# starts in. It keeps to bash 3.2, which stock macOS runs.
set -euo pipefail

run=${1:?usage: just test-health <constrained|mutants|post-merge> [ref]}
ref=${2:-origin/main}
case $run in
  constrained) minutes=120 ;;
  mutants) minutes=150 ;;
  post-merge) minutes=90 ;;
  *)
    echo "error: the run is constrained, mutants or post-merge, not $run" >&2
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
scripts=${GOODFETCH_GATE_SCRIPTS:-}
# The post-merge run's range starts here once it is known.
checked=""
result() {
  if [[ -n $checked ]]; then
    echo "$day $commit $run=$1 since $checked" >>"$logs/results.txt"
    echo "$commit" >"$logs/post-merge-checked"
  else
    echo "$day $commit $run=$1" >>"$logs/results.txt"
  fi
  said=yes
}
# A run that cannot run now: a post-merge run says so in its log alone and
# exits green, leaving its range for the next cron run; any other lists
# itself as skipped and fails.
skip() {
  echo "== skipped: $1"
  if [[ $run == post-merge ]]; then
    said=yes
    exit 0
  fi
  result "skipped ($2)"
  exit 1
}
# `set -e` ends the run on a setup that fails: its line still says so.
trap '[[ -n $said ]] || result fail; [[ -z $locked ]] || rm -rf "$lock"; rm -rf "$scripts"' EXIT

git fetch --quiet origin
commit=$(git rev-parse --verify "$ref^{commit}")
if [[ ${GOODFETCH_GATE_SCRIPTS_OF:-} != "$commit" ]]; then
  scripts=$(mktemp -d)
  git archive "$commit" ci | tar -x -C "$scripts"
  GOODFETCH_GATE_SCRIPTS_OF=$commit GOODFETCH_GATE_SCRIPTS=$scripts \
    exec bash "$scripts/ci/test-health/scheduled.sh" "$run" "$commit" </dev/null
fi
unset GOODFETCH_GATE_SCRIPTS GOODFETCH_GATE_SCRIPTS_OF

# What `main` gained since the post-merge run last checked it. The words
# ci/merge-pr.sh opens a full gate's merge message with: a head merged on it
# passed `just counts` and the Pre-merge suite on its own tree.
if [[ $run == post-merge ]]; then
  since=$(cat "$logs/post-merge-checked" 2>/dev/null || true)
  if [[ -z $since ]] || ! git merge-base --is-ancestor "$since" "$commit" 2>/dev/null; then
    since=$(git rev-parse --verify --quiet "$commit^1" || echo "$commit")
  fi
  if [[ $since == "$commit" ]]; then
    echo "== main has not moved since $commit"
    said=yes
    exit 0
  fi
  full_gate=$(git rev-list -1 --first-parent --merges --fixed-strings \
    --grep="Merged on the local gate: \`just pre-merge all\`" "$since..$commit")
  if [[ $full_gate == "$commit" ]]; then
    echo "== $commit merged on the full gate: recorded as checked"
    echo "$commit" >"$logs/post-merge-checked"
    said=yes
    exit 0
  fi
  checked=$since
fi

gate_worktree_name="gate-backstop"
# shellcheck source=ci/gate-worktree.sh
source "$(dirname "$0")/../gate-worktree.sh"
# The backstop's floor (ci/gate-backstop.sh): the host keeps 60 GB free, and
# the worktree's first build takes about 25 GB, a later one 5 GB
# (GOODFETCH_TEST_HEALTH_MIN_FREE_GB sets the floor itself). A post-merge
# run, every 20 minutes, needs 40 GB on the file system of the worktree's
# target/ once it exists, 10 more than `just merge`'s gate refuses under.
on=$root
if [[ -d "$worktree/target" ]]; then
  needed=65
  if [[ $run == post-merge ]]; then
    needed=40
    on=$worktree/target
  fi
else
  needed=85
fi
needed=${GOODFETCH_TEST_HEALTH_MIN_FREE_GB:-$needed}
free=$(df -Pk "$on/" | awk 'NR == 2 { print int($4 / 1048576) }')
if ((free < needed)); then
  skip "$free GB free, the run needs $needed GB" "free disk $free GB < $needed GB"
fi
holder=$(gate_worktree_holder "$lock")
if [[ -n $holder ]]; then
  skip "process $holder holds $worktree" "the backstop's worktree is busy"
fi
gate_worktree_lock
locked=yes
# A run beside `just merge`'s gate takes its cores: on 2026-11-08 the daily
# mutation run's load failed a gate on a leak timeout. So the lock says it is a
# test-health run's before the gate's lock is read, and a gate reads the two the
# other way round: of a run and a gate starting together, at least one sees the
# other. The gate waits for a run it sees (ci/merge-pr.sh); a run that sees a gate
# skips. A lock whose holder ended holds nothing, and is the next gate's to clear.
# A gate never waits for a post-merge run: it sends it TERM, on which the run
# stops its suites and ends as one that cannot run now. Its mark comes first, so
# a gate that sees the lock as a test-health run's sees it as a post-merge one's.
child=""
if [[ $run == post-merge ]]; then
  : >"$lock/post-merge"
  trap 'if [[ -n $child ]]; then
    kill -TERM -- "-$child" 2>/dev/null || kill -TERM "$child" 2>/dev/null || true
    wait "$child" || true
  fi
  skip "a merge gate stopped the run"' TERM
fi
: >"$lock/test-health"
holder=$(gate_worktree_holder "$root/.claude/worktrees/merge-gate.lock")
if [[ -n $holder ]]; then
  skip "the merge gate, process $holder, is running" "the merge gate is running"
fi
gate_worktree_checkout "$commit"
cd "$worktree"
just fetch

# Cron gives a run no user session: the runs start the OS services their tests
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
  post-merge)
    echo "== just counts and the Pre-merge suite over $checked..$commit"
    # Both run even when the first fails, and the inner shell expands $1 and $2.
    read -r -d '' both <<'RUN' || true
status=0
just counts || status=1
just test-affected pre-merge --base "$1" --head "$2" || status=1
exit "$status"
RUN
    # In the background, so the TERM trap runs at once; `timeout` leads its own
    # process group, which the trap ends whole.
    timeout "${minutes}m" bash -c "$both" post-merge "$checked" "$commit" &
    child=$!
    wait "$child" || status=$?
    ;;
esac
if ((status == 124)); then echo "== stopped after $minutes minutes"; fi
if ((status == 0)); then result pass; else result fail; fi
exit "$status"
