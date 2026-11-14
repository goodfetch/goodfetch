#!/usr/bin/env bash
# `just test-health <constrained|mutants|post-merge> [ref]`: the test-health
# runs the gate host starts while ADR 0135 keeps GitHub Actions to the
# Release suite (TESTING.md, After merge), on `main`'s head by default: its
# cron the constrained and the mutation run (the entries in
# ci/test-health.crontab), and each `just merge` gate, as it ends, the
# post-merge run (ci/merge-pr.sh). They take the lock of the backstop's gate
# worktree (ci/gate-worktree.sh's `gate-backstop`), so they never run beside
# the backstop or each other. The constrained and the mutation run build in
# that worktree and reuse its build; the post-merge run builds in `just
# merge`'s (`merge-gate`), where the gate just built `main`'s tree. None
# starts while `just merge`'s gate runs, and a gate started during a run
# waits for it, but for a post-merge run, which it stops.
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
#                when the first fails, within 90 minutes. A gate that held
#                its lock starts it as it ends, merged or not: it does nothing
#                while `main` has not moved, and records a head whose merge
#                passed the full gate as checked without running anything.
# Each run appends one line to `<log>/results.txt`, `<UTC day> <commit>
# <run>=<pass|fail>`, or `<run>=skipped (…)` when `just merge`'s gate runs,
# the disk lacks room or the worktree is busy; a run that ends on its setup,
# before its tests, reads `fail`. A post-merge run's line ends with `since
# <checked>`, and once its suites have started it then moves
# `post-merge-checked` to the commit, pass or fail: a red range is reported,
# not run again. One that ends on its setup, that cannot run now or that a
# gate stopped moves nothing, so the run the next gate starts covers the
# range; one that cannot run now or was stopped writes `post-merge=skipped
# (…)`, once for a commit and a reason. Its log is `<log>/<day>-<run>.log`,
# with `<log>` GOODFETCH_TEST_HEALTH_LOG or ~/test-health. It fails when the
# run fails or, but for a post-merge run, is skipped. A run is the scripts of
# the commit it tests, whichever script started it: nothing fast-forwards the
# checkout it starts in. It keeps to bash 3.2, which stock macOS runs.
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
# The post-merge run's range starts here once it is known, and is checked
# once its suites have started: a run that ends before, on its setup, says
# `fail` and leaves the range to the next run (#2619: it moved the commit
# too, so a range no suite had run was listed red once and never run).
checked=""
suites=""
result() {
  if [[ -n $checked ]]; then
    echo "$day $commit $run=$1 since $checked" >>"$logs/results.txt"
    [[ -z $suites ]] || echo "$commit" >"$logs/post-merge-checked"
  else
    echo "$day $commit $run=$1" >>"$logs/results.txt"
  fi
  said=yes
}
# A run that cannot run now lists itself as skipped, for the reason `$2`, and
# fails. A post-merge run exits green instead and moves nothing, leaving its
# range for the next one, and writes its line once for a commit and a reason
# (#2619: it said so in its log alone, where no watch reads), whatever the
# day and whatever came between: the run after each gate would otherwise
# repeat it.
skip() {
  local line
  echo "== skipped: $1"
  if [[ $run == post-merge ]]; then
    line="$commit $run=skipped ($2)"
    if ! grep -qF " $line" "$logs/results.txt" 2>/dev/null; then
      echo "$day $line" >>"$logs/results.txt"
    fi
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
# The post-merge run holds the backstop's lock, as the other runs do, and
# builds in `just merge`'s worktree: the gate that merged `main`'s head built
# its tree there, so the run compiles what its suites need beyond that and no
# second build folder exists (#2619). No gate is in that worktree meanwhile:
# the run reads the gate's lock once it holds and has marked its own, below,
# and a gate that starts later stops the run and waits for its lock to go
# before its checkout (ci/merge-pr.sh).
if [[ $run == post-merge ]]; then
  worktree="$root/.claude/worktrees/merge-gate"
fi
# The backstop's floor (ci/gate-backstop.sh): the host keeps 60 GB free, and
# the worktree's first build takes about 25 GB, a later one 5 GB
# (GOODFETCH_TEST_HEALTH_MIN_FREE_GB sets the floor itself). A post-merge
# run needs 40 GB once the gate's target/ exists, 10 more than `just merge`'s
# gate refuses under, so it leaves no gate short. The space is read on the
# file system the build goes to (#2619: a run read the checkout's disk and
# built on another): that of the worktree's target/, which can be a mount of
# its own, and before it exists that of the folder it is created in.
if [[ -d $worktree/target ]]; then
  on=$worktree/target
  needed=65
  [[ $run != post-merge ]] || needed=40
elif [[ -d $worktree ]]; then
  on=$worktree
  needed=85
else
  on=$root
  needed=85
fi
needed=${GOODFETCH_TEST_HEALTH_MIN_FREE_GB:-$needed}
read -r free mount < <(df -Pk "$on/" | awk 'NR == 2 { print int($4 / 1048576), $6 }')
if ((free < needed)); then
  short="free disk $free GB < $needed GB"
  # A post-merge run's line is written once for a reason: no figure that moves.
  [[ $run != post-merge ]] || short="free disk under $needed GB on $mount"
  skip "$free GB free on $mount, the run needs $needed GB" "$short"
fi
holder=$(gate_worktree_holder "$lock")
if [[ -n $holder ]]; then
  skip "process $holder holds $lock" "the backstop's worktree is busy"
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
# The gate checks its head out in the worktree the run builds in once the
# run's lock is gone, so the lock outlives every process of the suites, not
# their leader alone: a test still ending would write beside that checkout.
# The suites run in a session of their own, which `timeout` leads, so their
# processes are that session's: `timeout`'s process group, which its stop
# reaches, and the groups of their own nextest gives each test and a test
# gives a process it starts, which nextest stops as it ends. One still there
# once `timeout`'s group has ended, a Daemon a test left, is told to stop
# then. A process that outlasts 30 seconds is killed, and the lock goes 5
# seconds later whatever is still listed. Where `ps` does not give a
# session's number (macOS), `timeout`'s group alone is waited for.
child=""
asked=""
# The processes of the suites' session, one a line.
left() {
  ps -A -o pid= -o sess= 2>/dev/null | awk -v session="$child" '$2 == session { print $1 }'
}
# Sends each of them the signal `$1`.
tell() {
  local process
  for process in $(left); do kill "-$1" "$process" 2>/dev/null || true; done
}
stopped() {
  local waited=0 told=""
  if [[ -n $child ]]; then
    kill -TERM -- "-$child" 2>/dev/null || kill -TERM "$child" 2>/dev/null || true
    wait "$child" 2>/dev/null || true
    while ((waited < 350)); do
      if ! kill -0 -- "-$child" 2>/dev/null; then
        [[ -n $(left) ]] || break
        if [[ -z $told ]]; then
          tell TERM
          told=yes
        fi
      fi
      waited=$((waited + 1))
      if ((waited == 300)); then
        kill -KILL -- "-$child" 2>/dev/null || true
        tell KILL
      fi
      sleep 0.1
    done
  fi
  skip "a merge gate stopped the run" "a merge gate stopped the run"
}
# The trap comes before the mark a gate reads, so no gate's TERM finds the
# run without it. While the suites start, between `suites` being set and
# `child` naming them, the trap only notes the stop, which the start then
# acts on: a stop that ran there would find no process to end and let the
# lock go under suites that had just started.
if [[ $run == post-merge ]]; then
  trap 'if [[ -n $child || -z $suites ]]; then stopped; else asked=yes; fi' TERM
  : >"$lock/post-merge"
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
    # In the background, so the TERM trap runs at once; `timeout` leads a
    # session of its own, whose processes the trap ends.
    suites=started
    perl -MPOSIX -e '
      defined POSIX::setsid() or die "setsid: $!\n";
      exec { $ARGV[0] } @ARGV or die "exec $ARGV[0]: $!\n";
    ' timeout "${minutes}m" bash -c "$both" post-merge "$checked" "$commit" &
    child=$!
    [[ -z $asked ]] || stopped
    wait "$child" || status=$?
    ;;
esac
if ((status == 124)); then echo "== stopped after $minutes minutes"; fi
if ((status == 0)); then result pass; else result fail; fi
exit "$status"
