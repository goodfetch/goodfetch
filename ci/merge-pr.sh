#!/usr/bin/env bash
# Merges a pull request on the local gate (AGENTS.md, "Merging"). GitHub
# Actions runs only the Release suite, so no required check can block a
# merge: this command is the lock instead. In a clean checkout of the pull
# request's exact head it runs one gate and merges only when it passes.
# Which gate (ADR 0135's amendments of 2026-11-10, the owner's decisions for
# the refactor: quick per merge, the full gate once per finished spec and on
# the tenth change a test reads since the last full gate):
#   - `just pre-merge docs` when `xtask affected` finds the change is
#     documents alone (`gate=docs`);
#   - `just pre-merge quick` when it finds that no test, benchmark, model,
#     container or Evaluation reads the change (`gate=quick`);
#   - for a change a test reads (`gate=full`), `just pre-merge all`, which
#     starts with `just quick all`'s lint and Quick suite, on a spec's pull
#     request, whose head branch is `spec/<n>` (docs/agents/issue-tracker.md),
#     and on the tenth such change since the last full gate, the nine before
#     it having merged on the quick gate, as the messages of the base's
#     merges say it (#2457); `just pre-merge quick` on every other pull
#     request;
#   - `just pre-merge all` on any pull request when `--full` asks for it.
# The gate's log and the merge's message name the gate that ran and why, and
# a quick merge of a change a test reads says what it did not run. First the
# merge cap (ci/merge-cap.sh) refuses once 10 merges have
# landed on `main` since the backstop's last green run, unless the owner
# passes `--past-cap`. It refuses to start a gate while the file system of
# the gate's `target/` has less free space than a gate's builds take (#1822)
# or, on Linux, a full gate whose stress run could not run a LazyFS power cut;
# keeps a failing gate's log and test results where no cleanup reaches them
# (#2087); and, still holding the gate's lock, removes the build units no
# gate read since the day before this one started (`xtask sweep-target`).
#
# The gate outlives its caller. The command starts itself again in a session
# of its own, with its output in a log the common folder keeps for 30 days
# (`goodfetch-gate-logs/`), and only follows that log: it prints what the gate
# writes and ends with the gate's exit status. Whatever stops the caller, or
# closes its output, leaves the gate running to its merge (a session restart
# killed one 1 h 47 in, on 2026-11-08). SIGTERM to the gate itself, the
# process the caller names and the lock holds, stops its steps and releases
# the lock.
set -euo pipefail

usage="usage: ci/merge-pr.sh <pull request number> [--full] [--past-cap]"
# Only the command itself passes this, to start the gate: what names the
# gate's files, `<run>.log` and `<run>.status`.
run=""
if [[ ${1:-} == --detached ]]; then
  run=$2
  shift 2
fi
number=${1:?$usage}
shift
# The options as given, which the gate started below takes too: `--past-cap`,
# the owner's way past the merge cap, and `--full`, which asks for the full
# gate on a pull request that would take another.
options=("$@")
cap=()
full_asked=""
for option in ${options[@]+"${options[@]}"}; do
  case $option in
    --past-cap) cap=(--past-cap) ;;
    --full) full_asked=1 ;;
    *)
      echo "error: $usage" >&2
      exit 2
      ;;
  esac
done
common=$(git rev-parse --path-format=absolute --git-common-dir)

if [[ -z $run ]]; then
  logs="$common/goodfetch-gate-logs"
  mkdir -p "$logs"
  find "$logs" -mindepth 1 -maxdepth 1 -type f -mtime +30 -exec rm -f {} +
  run="$logs/$(date -u +%Y%m%dT%H%M%SZ)-pr$number-$$"
  : >"$run.log"
  # The command substitution's shell ends at once, so nothing the caller
  # started is the gate's parent, and it runs no job control, so the gate is
  # no process group's leader and can start a session. A background command
  # starts with SIGINT and SIGQUIT ignored, and the caller's own may ignore
  # more; the gate's steps get what a foreground command gets, since a test
  # may depend on a default, and the gate's SIGTERM trap on an unignored one.
  gate=$(
    perl -MPOSIX -e '
      $SIG{$_} = "DEFAULT" for qw(HUP INT QUIT TERM PIPE);
      defined POSIX::setsid() or die "setsid: $!\n";
      exec { $ARGV[0] } @ARGV or die "exec $ARGV[0]: $!\n";
    ' "$0" --detached "$run" "$number" ${options[@]+"${options[@]}"} </dev/null >>"$run.log" 2>&1 &
    echo "$!"
  )
  echo "The gate is process $gate, in a session of its own: it goes on if this command stops, and \`kill $gate\` stops it. Its output is in $run.log"
  exec 3<"$run.log"
  while [[ ! -s "$run.status" ]] && kill -0 "$gate" 2>/dev/null; do
    cat <&3 || true
    sleep 1
  done
  cat <&3 || true
  if [[ ! -s "$run.status" ]]; then
    echo "error: the gate (process $gate) ended without giving an exit status; its output is in $run.log" >&2
    exit 1
  fi
  exit "$(cat "$run.status")"
fi

# The gate's end, however it comes: after a gate that ran, pass or fail, and
# before the lock is released, the build units no gate read since the day
# before this gate started go, which `relatime` keeps telling apart
# (xtask/src/sweep_target.rs). The sweep takes the start itself, so the gate's
# own reads stay however long it ran. A head from before the command only
# warns. The exit status comes last, as the caller ends on it.
started=""
locked=""
on_exit() {
  local status=$?
  trap '' TERM HUP INT
  if [[ -n $started ]]; then
    (cd "$worktree" && cargo run --quiet --locked --bin xtask -- sweep-target --read-since "$started") ||
      echo "warning: the gate's target/ was not swept" >&2
  fi
  [[ -z $locked ]] || rm -rf "$lock"
  echo "$status" >"$run.status"
}
trap on_exit EXIT

# Stops the gate's steps, waits for them, so the lock outlives them, and skips
# the sweep: the gate was asked to stop. `just pre-merge` stops the steps it
# runs beside, in process groups of their own, as it ends.
steps=""
stop() {
  trap '' TERM HUP INT
  started=""
  echo "The gate was told to stop: stopping its steps"
  if [[ -n $steps ]]; then kill -TERM -- "-$steps" 2>/dev/null || true; fi
  wait || true
  exit 143
}
trap stop TERM HUP INT

# Reads the pull request, its state, head, base, head branch and title, and
# refuses one the gate cannot merge: closed, a draft, past the merge cap,
# moved while it was read, or without its base's tip.
read_pull_request() {
  IFS=$'\t' read -r state draft head base branch title < <(
    gh pr view "$number" --json state,isDraft,headRefOid,baseRefName,headRefName,title \
      --jq '[.state, (.isDraft | tostring), .headRefOid, .baseRefName, .headRefName, .title] | @tsv'
  )
  if [[ $state != OPEN ]]; then
    echo "error: pull request #$number is $state, not open" >&2
    exit 1
  fi
  if [[ $draft != false ]]; then
    echo "error: pull request #$number is a draft; mark it ready first" >&2
    exit 1
  fi

  git fetch --quiet origin "$base"
  git fetch --quiet origin main
  "$(dirname "$0")/merge-cap.sh" ${cap[@]+"${cap[@]}"}
  git fetch --quiet origin "pull/$number/head"
  if [[ $(git rev-parse FETCH_HEAD) != "$head" ]]; then
    echo "error: pull request #$number moved while it was read; run the command again" >&2
    exit 1
  fi
  # A head that contains its base's tip merges into the same tree it was tested
  # on, so the gate's verdict holds for what lands on the base.
  if ! git merge-base --is-ancestor "origin/$base" "$head"; then
    echo "error: pull request #$number does not contain the tip of $base; merge $base into it first" >&2
    exit 1
  fi
}
# Before the lock and any wait, so a pull request the gate refuses is refused
# at once and holds nothing.
read_pull_request

# One worktree for every gate, locked and held by one gate at a time, reset
# to the exact head (ci/gate-worktree.sh).
# shellcheck source=ci/gate-worktree.sh
source "$(dirname "$0")/gate-worktree.sh"
# shellcheck source=ci/output-value.sh
source "$(dirname "$0")/output-value.sh"
gate_worktree_lock
locked=1

# The nightly fuzzing gives way to a gate (ci/fuzz-nightly.sh): it takes its
# lock before it reads this one, as the gate has taken this one before it reads
# the fuzzing's, so of the two starting together at least one sees the other.
# The gate stops a night it sees and waits for that stop alone: the job ends
# what it started and writes its line before its lock goes.
fuzzing=$(fuzz_nightly_holder)
if [[ -n $fuzzing ]]; then
  echo "Stopping the nightly fuzzing, process $fuzzing, before the gate starts"
  kill -TERM "$fuzzing" 2>/dev/null || true
  while [[ -n $(fuzz_nightly_holder) ]]; do sleep 1; done
fi

# A run in the backstop's worktree, a test-health run or the backstop's own
# full run, shares the host's cores with whatever runs beside it, and on
# 2026-11-08 a test-health run's load failed a gate on a leak timeout. The gate
# waits, holding its lock so no later run starts, until that run ends: within
# its minutes, or at once if it died. A test-health run sees this lock once it
# has marked its own, and skips. The backstop takes its lock before it reads
# this one, as the gate has taken this one before it reads the backstop's, so
# of a backstop and a gate starting together at least one sees the other: a
# backstop that sees this gate lets it end first and says so in its lock, and
# the gate then goes on (ci/gate-worktree.sh). Behind a backstop the wait can
# last hours, so a gate that waited reads its pull request again: it judges
# the head, the base and the merge cap as they are once it starts.
waiting_for=""
while ahead=$(run_in_the_backstops_worktree "$$") && [[ -n $ahead ]]; do
  if [[ $ahead != "$waiting_for" ]]; then
    echo "Waiting for $ahead, to end before the gate starts"
    waiting_for=$ahead
  fi
  sleep 1
done
[[ -z $waiting_for ]] || read_pull_request

# This checkout's compiler cache check, which the gate runs in its own.
compiler_cache=$(cd "$(dirname "$0")" && pwd)/compiler-cache.sh

# A failing gate's log and test results are kept in the repository's common
# folder, which no checkout's clean, no worktree removal and no sweep of the
# temporary folder reaches, for 30 days: a flaky-candidate issue links them.
# `red-steps` holds what each red step's runners had written when it ended
# (ci/pre-merge.sh), since the gate goes on and later steps clear those files.
failures="$common/goodfetch-gate-failures"
keep_failed_gate() {
  local kept entry
  kept="$failures/$(date -u +%Y%m%dT%H%M%SZ)-pr$number-${head:0:12}"
  mkdir -p "$kept"
  cp "$run.log" "$kept/gate.log"
  for entry in nextest junit skipped.txt stress-selection.tsv red-steps; do
    [[ ! -e $worktree/target/$entry ]] || cp -R "$worktree/target/$entry" "$kept/"
  done
  find "$failures" -mindepth 1 -maxdepth 1 -type d -mtime +30 -exec rm -rf {} +
  echo "The failing gate's log and test results are kept in $kept: link that folder in the flaky-candidate issue" >&2
}

# A gate's builds, a dependency change's included, take up to this much on
# the file system of its `target/`, which can be another than the
# checkout's (a mount of its own).
min_free_gb=${GOODFETCH_GATE_MIN_FREE_GB:-30}
on=$worktree/target
[[ -e $on ]] || on=$root
read -r free_gb mount < <(df -Pk "$on" | awk 'NR == 2 { print int($4 / 1048576), $6 }')
if ((free_gb < min_free_gb)); then
  echo "error: the gate's target/ is on $mount, which has $free_gb GB free, under the $min_free_gb GB a gate's builds need (GOODFETCH_GATE_MIN_FREE_GB); free space there first, for example with \`cargo run --bin xtask -- sweep-target --target $worktree/target --unused-hours 25\`" >&2
  exit 1
fi
echo "The gate's target/ is on $mount, with $free_gb GB free (at least $min_free_gb GB needed)"

gate_worktree_checkout "$head"

# The head's own `xtask affected` says what the change is; a selection
# without a `gate=` line or with a gate this script does not know fails
# before any step.
selection=$(cd "$worktree" && cargo run --quiet --locked --bin xtask -- affected --base "origin/$base" --head "$head")
affected=$(output_value gate "$selection")
case $affected in
  quick | docs | full) ;;
  *)
    echo "error: \`xtask affected\` gave the gate $affected, which is quick, docs or full" >&2
    exit 1
    ;;
esac
# Which gate runs, and why, as the log and the merge's message say it. The
# full gate runs once per finished spec, on the pull request of its
# integration branch `spec/<n>`, when `--full` asks for it, and on the tenth
# change a test reads since the last full gate; every other change a test
# reads merges on the quick gate (the owner's decisions of 2026-11-10 for the
# refactor and of 2026-11-11 on the tenth, ADR 0135).
#
# How a merge's message opens, and how it names the full gate and a quick
# merge of a change a test reads: the count below reads back the words the
# message is written with.
merged_on="Merged on the local gate: "
the_full_gate="\`just pre-merge all\`"
quick_of_a_change_a_test_reads="\`just pre-merge quick\` (a test reads the change"
# The merges on the base's first-parent line, since the last one a full gate
# passed, that merged a change a test reads on the quick gate, as their
# messages say it (#2457). A full gate starts the count again whatever asked
# for it: a spec's, `--full` or the tenth's own. A docs merge and a quick
# merge of a change no test reads say other words and are not counted, nor is
# a merge this script did not make, which starts no count either.
quick_merges_since_the_last_full_gate() {
  local last_full
  last_full=$(git rev-list -1 --first-parent --merges --fixed-strings \
    --grep="$merged_on$the_full_gate" "origin/$base") || return
  git rev-list --count --first-parent --merges --fixed-strings \
    --grep="$merged_on$quick_of_a_change_a_test_reads" "origin/$base" ${last_full:+"^$last_full"}
}
why_full=""
if [[ -n $full_asked ]]; then
  why_full="asked for with \`--full\`"
elif [[ $affected == full && $branch =~ ^spec/[0-9]+$ ]]; then
  why_full="the pull request of a spec's branch, \`$branch\`"
elif [[ $affected == full ]]; then
  # More than nine can be there, merged before this script counted or by a
  # checkout that did not: the next change still takes the full gate, and
  # says which change it is. A count that cannot be read is refused: taken
  # for none, it would send the tenth change to the quick gate.
  if ! quick_merges=$(quick_merges_since_the_last_full_gate) || [[ ! $quick_merges =~ ^(0|[1-9][0-9]*)$ ]]; then
    echo "error: the changes a test reads that merged on the quick gate since the last full gate could not be counted on origin/$base (the count read \"$quick_merges\", which is no whole number), so nothing says whether this change is the tenth; run the command again, or ask for the full gate with \`--full\`" >&2
    exit 1
  fi
  if ((quick_merges == 9)); then
    why_full="the tenth change a test reads since the last full gate"
  elif ((quick_merges > 9)); then
    why_full="change $((quick_merges + 1)) a test reads since the last full gate, past the tenth"
  fi
fi
if [[ -n $why_full ]]; then
  gate=all
  ran="$the_full_gate ($why_full), which starts with \`just quick all\`'s lint and Quick suite,"
  # `just stress`, after the whole Pre-merge suite, refuses a selected
  # `lazyfs::` test without a LazyFS build (xtask's `repeat`). Which tests it
  # selects takes a build to find, so every full gate on Linux, the one system
  # LazyFS runs on, asks for one before any step.
  if [[ "$(uname -s)" == Linux && ! (-f ${GOODFETCH_LAZYFS:-} && -x ${GOODFETCH_LAZYFS:-}) ]]; then
    echo "error: a full gate's \`just stress\` needs GOODFETCH_LAZYFS to name an executable LazyFS build, and it is ${GOODFETCH_LAZYFS:-not set}; build one with \`just lazyfs-build\` and name the executable that \`cargo run --bin xtask -- lazyfs build\` prints, or a copy of it, since a gate clears its worktree's target/lazyfs/" >&2
    exit 1
  fi
elif [[ $affected == docs ]]; then
  gate=docs
  ran="\`just pre-merge docs\` (the change is documents alone)"
elif [[ $affected == quick ]]; then
  gate=quick
  ran="\`just pre-merge quick\` (no test, benchmark, model, container or Evaluation reads the change)"
else
  gate=quick
  ran="$quick_of_a_change_a_test_reads, and the full gate runs once per finished spec, on the pull request of its \`spec/<n>\` branch, or with \`--full\`; the Pre-merge suite, perf, the stress run, the visual tests, the Linux-only tests and the full gate's other steps did not run)"
fi
echo "The gate is ${ran%,}"
echo "Running the local gate ($gate) on #$number at $head in $worktree"
started=$(date -u +%s)
status=0
# In the background, since a shell runs a trap only once its foreground
# command has ended and the gate's stop must not wait hours for that, and in a
# process group of its own that it leads (job control, `set -m`, for that one
# command, as ci/pre-merge.sh starts a step beside), since without job control
# a background command ignores SIGINT and SIGQUIT, and the tests would too. Told
# to stop, it ends once its running step has ended, and starts no other.
set -m
(
  trap 'exit 143' TERM
  cd "$worktree"
  # Before any build: whether the gate's checkout reuses the C and C++
  # objects other checkouts compiled (TESTING.md, Compiler cache); a gap
  # fails the gate.
  COMPILER_CACHE_REQUIRED=1 "$compiler_cache"
  just fetch
  if [[ $gate == quick || $gate == docs ]]; then
    just pre-merge "$gate"
  else
    # What the pull request's "Performance change" section and its owner's
    # acceptance comment accept, read at this head for `just perf` (ADR 0134).
    # Until M1's optimization pass it only marks figures accepted in the
    # table: no figure past its threshold fails (ADR 0134, as amended on
    # 2026-10-22). So a read that fails, GitHub's API being down, is said and
    # leaves the verdict to the gate's steps.
    if GITHUB_TOKEN="${GITHUB_TOKEN:-$(gh auth token)}" cargo run --quiet --locked --bin xtask -- \
      perf pull-request --number "$number" --save target/perf-pull-request.json; then
      export GOODFETCH_PERF_PULL_REQUEST="$worktree/target/perf-pull-request.json"
    else
      echo "warning: the pull request's acceptance could not be read; the gate's steps decide" >&2
    fi
    just pre-merge all
  fi
) &
steps=$!
set +m
wait "$steps" || status=$?
steps=""
if ((status != 0)); then
  keep_failed_gate
  exit "$status"
fi
# The incremental gate's verdict combines this run with the green results it
# reused from earlier runs on ancestor heads (ADR 0135); the full gate's plan
# names the nearest such run. A gate that ran on a dirty tree leaves no plan,
# and so reused nothing.
plan_file="$worktree/target/gate-runs/$head.plan"
if [[ $gate == all && -f $plan_file ]]; then
  plan=$(cat "$plan_file")
  reused=$(output_value previous "$plan")
  if [[ $reused != none ]]; then
    ran+=" reusing the green results of the gate run on $reused for the steps the change since did not reach (ADR 0135),"
  fi
fi

past=""
[[ ${#cap[@]} -eq 0 ]] || past=" Merged past the merge cap: the owner passed --past-cap."
gh pr merge "$number" --merge --match-head-commit "$head" \
  --subject "$title (#$number)" \
  --body "$merged_on$ran passed at $head.$past

Signed-off-by: $(git config user.name) <$(git config user.email)>"
echo "Merged #$number at $head"

# The next gate's stress run reads its base's listings from here, the base
# being the tip of the base branch it must contain: this merge, whose tree is
# the head's when the head contained the base's tip, as checked above. Saved
# only then; otherwise the next stress run lists its base from a checkout of
# it, as it does without them. The head's test binaries are built, so listing
# them takes seconds.
base_tip=$(git rev-parse "origin/$base")
git fetch --quiet origin "$base" || true
merged=$(git rev-parse "origin/$base")
listings="$worktree/target/stress-listings"
if [[ $gate == quick && $affected == full ]]; then
  # A quick merge of a change a test reads may have added, renamed or removed
  # tests, and built no test binary to list: the base's listings are not the
  # merge's, so none is kept.
  rm -rf "$listings"
  echo "warning: a test reads the change the quick gate merged, so no stress listings of $merged were saved; the next stress run lists its base from a checkout" >&2
elif [[ $gate == quick || $gate == docs ]]; then
  # No test reads this change, so the base's listings, when saved, are the
  # merge's too, and no test binary was built to list.
  if [[ -d "$listings/$base_tip" ]] && mv "$listings/$base_tip" "$listings/$merged"; then
    echo "Kept the stress listings of $base_tip for $merged"
  else
    rm -rf "$listings"
    echo "warning: no stress listings of $merged were saved; the next stress run lists its base from a checkout" >&2
  fi
elif [[ $(git rev-parse "$merged^{tree}") == $(git rev-parse "$head^{tree}") ]] &&
  (cd "$worktree" && rm -rf "$listings" &&
    cargo run --quiet --locked --bin xtask -- stress listings --cargo-profile test --out "$listings" &&
    mv "$listings/$head" "$listings/$merged"); then
  echo "Saved the stress listings of $merged for the next gate"
else
  rm -rf "$listings"
  echo "warning: the stress listings of $merged were not saved; the next stress run lists its base from a checkout" >&2
fi
