#!/usr/bin/env bash
# `just gate-backstop [ref]`: the incremental gate's backstop (ADR 0135),
# daily from the gate host's cron, on `main`'s head by default. In a gate
# worktree of its own (ci/gate-worktree.sh's `gate-backstop`), so it never
# holds `just merge`'s, it runs `just pre-merge all` in full
# (`GOODFETCH_GATE_FULL=1`), then compares that run with each incremental
# run on the same tree, the gated head of the merge, in `just merge`'s
# records (`xtask gate compare`): a result that run reused and the full run
# found red is a selection miss, a defect of the gate's selection to fix. A
# red step run per member only makes the reused crates suspects, since the
# members the incremental run reran may hold the red, so each suspect
# crate's tests rerun alone and a red confirms the miss. The full run's stress
# run has `main`'s head as its base, so it stresses nothing: each crate whose
# result in a merge since the last green run counted passes from an earlier
# head (#2233), whatever that merge's tree, is stressed again here, at its whole
# count, against that merge's base. A red there is a miss, though not proof of
# a bad reuse: it runs the tests of every merge since that base, so a flake in a
# later merge's test reads the same. A day with a miss, a failure or a skip is
# no green run, so the next day reaches the same merges. A test renamed since
# runs at its new name; a test or crate gone since has nothing left to stress,
# and the selection notes it in this run's log. A record `just merge` should have
# and does not, a full gate's merge or the whole folder, is a miss too, since it
# may have held earned passes; with no green run yet only the folder is judged.
#
# A run is the scripts of the commit it tests, whichever script started it:
# nothing fast-forwards the checkout cron starts in, and an old script could
# compare, skip what a later one stresses and write a green line.
#
# Its way past a miss nothing can clear, a base that can never run again or a
# record gone for good, is the owner's: GOODFETCH_GATE_BACKSTOP_SINCE=<commit>
# `just gate-backstop`, once, after stressing by hand what can still run, starts
# from that commit instead of the last green run and writes `since=<commit>` at
# the end of its line. Nobody edits results.txt.
#
# Each run appends one line to `<log>/results.txt`, `<UTC day> <commit>
# gate=<pass|fail> misses=<n> suspects=<n> unchecked=<n> earned=<n>`, or
# `gate=skipped (…)` when the disk lacks room or Linux lacks a LazyFS build, and
# each miss, suspect or unchecked result to `<log>/misses.txt`; the run's own
# log is `<log>/<day>.log`, with `<log>`
# GOODFETCH_GATE_BACKSTOP_LOG or ~/gate-backstop. It fails when the full run
# fails or finds a miss. It keeps to bash 3.2, which stock macOS runs.
set -euo pipefail

ref=${1:-origin/main}
logs=${GOODFETCH_GATE_BACKSTOP_LOG:-$HOME/gate-backstop}
mkdir -p "$logs"
day=$(date -u +%F)
exec >>"$logs/$day.log" 2>&1

git fetch --quiet origin
commit=$(git rev-parse --verify "$ref^{commit}")

if [[ ${GOODFETCH_GATE_SCRIPTS_OF:-} != "$commit" ]]; then
  scripts=$(mktemp -d)
  git archive "$commit" ci | tar -x -C "$scripts"
  GOODFETCH_GATE_SCRIPTS_OF=$commit GOODFETCH_GATE_SCRIPTS=$scripts \
    exec bash "$scripts/ci/gate-backstop.sh" "$commit" </dev/null
fi
scripts=${GOODFETCH_GATE_SCRIPTS:-}
unset GOODFETCH_GATE_SCRIPTS GOODFETCH_GATE_SCRIPTS_OF
trap 'rm -rf "$scripts"' EXIT
echo "== $(date -u +%FT%TZ) the incremental gate's backstop on $ref"

# The re-stress refuses a selected `lazyfs::` test without a LazyFS build (xtask's
# `repeat`), and which tests it selects takes a build to find, so a run on Linux,
# the one system LazyFS runs on, asks for one first, as `just merge`'s full gate
# does. Cron's entry names it (ci/gate-backstop.crontab).
if [[ "$(uname -s)" == Linux && ! (-f ${GOODFETCH_LAZYFS:-} && -x ${GOODFETCH_LAZYFS:-}) ]]; then
  echo "error: the backstop's re-stress needs GOODFETCH_LAZYFS to name an executable LazyFS build, and it is ${GOODFETCH_LAZYFS:-not set}; build one with \`just lazyfs-build\`, copy its target/lazyfs/bin/ outside every checkout and name it in the crontab (ci/gate-backstop.crontab)" >&2
  echo "$day $commit gate=skipped (no LazyFS build: GOODFETCH_LAZYFS is ${GOODFETCH_LAZYFS:-not set})" >>"$logs/results.txt"
  exit 1
fi

# The owner's way past a miss nothing can clear: where this run starts instead of
# the last green run, which `main` must hold.
told_since=""
if [[ -n ${GOODFETCH_GATE_BACKSTOP_SINCE:-} ]]; then
  told_since=$(git rev-parse --verify "$GOODFETCH_GATE_BACKSTOP_SINCE^{commit}")
  if ! git merge-base --is-ancestor "$told_since" "$commit"; then
    echo "error: GOODFETCH_GATE_BACKSTOP_SINCE is $told_since, which $commit does not hold" >&2
    exit 1
  fi
  echo "== the owner named $told_since: the merges it holds are not checked"
fi

gate_worktree_name="gate-backstop"
# shellcheck source=ci/gate-worktree.sh
source "$(dirname "$0")/gate-worktree.sh"
# shellcheck source=ci/backstop-green.sh
source "$(dirname "$0")/backstop-green.sh"
# The host keeps 60 GB free: the backstop starts only with room for its
# build on top, about 25 GB for its worktree's first, 5 GB for a later one
# (GOODFETCH_GATE_BACKSTOP_MIN_FREE_GB sets the floor itself). A skipped day
# is listed and fails.
if [[ -d "$worktree/target" ]]; then needed=65; else needed=85; fi
needed=${GOODFETCH_GATE_BACKSTOP_MIN_FREE_GB:-$needed}
free=$(df -Pk "$root" | awk 'NR == 2 { print int($4 / 1048576) }')
if ((free < needed)); then
  echo "== skipped: $free GB free, the backstop needs $needed GB"
  echo "$day $commit gate=skipped (free disk $free GB < $needed GB)" >>"$logs/results.txt"
  exit 1
fi
gate_worktree_lock
trap 'rm -rf "$lock" "$scripts"' EXIT
gate_worktree_checkout "$commit"
# The incremental runs it compares with: `just merge`'s, read only.
merged_runs="$root/.claude/worktrees/merge-gate/target/gate-runs"
cd "$worktree"
just fetch

gate=pass
GOODFETCH_GATE_FULL=1 just pre-merge all || gate=fail
echo "== the full run on $commit: $gate"

misses=0 suspects=0 unchecked=0 earned=0
# Read before this run's line joins them. What the last green run held is done.
since=${told_since:-$(latest_green "$(backstop_greens "$logs/results.txt")" "$commit")}
# A JSON array of the results found, each with its kind, unit, step,
# member, run, origin and base (`xtask gate compare`).
compared=$(just _gate-compare "$commit" "$merged_runs" ${since:+"$since"}) || {
  echo "error: the full run on $commit could not be compared" >&2
  compared="[]"
  gate=fail
}
note() {
  echo "$day $commit $*" >>"$logs/misses.txt"
  echo "== $*"
}
# The suspect crates of each step run per member, by step.
suspect_crates() {
  jq -r --arg step "$1" '.[] | select(.kind == "suspect" and .step == $step) | .member' <<<"$compared" |
    sort -u
}
missed=$(jq -r '.[] | select(.kind == "miss") | "miss \(.unit): the run on \(.run) reused it from \(.origin); the full run found it red"' <<<"$compared")
while IFS= read -r line; do
  [[ -n $line ]] || continue
  misses=$((misses + 1))
  note "$line"
done <<<"$missed"
# A merged run whose record cannot be read may have counted earned passes.
unreadable=$(jq -r '.[] | select(.kind == "unreadable") | "miss record \(.run): merged since the last green run, and its record cannot be read"' <<<"$compared")
while IFS= read -r line; do
  [[ -n $line ]] || continue
  misses=$((misses + 1))
  note "$line"
done <<<"$unreadable"
# So may one that is gone: a green line over it would move past that merge for
# good.
gone=$(jq -r --arg folder "$merged_runs" '.[] | select(.kind == "missing") | if .run == "" then "miss records: \($folder) is gone, with commits merged since the last green run" else "miss record \(.run): a full gate merged it since the last green run, and its record is gone" end' <<<"$compared")
while IFS= read -r line; do
  [[ -n $line ]] || continue
  misses=$((misses + 1))
  note "$line"
done <<<"$gone"
unreached=$(jq -r '.[] | select(.kind == "unchecked") | "unchecked \(.unit): the run on \(.run) reused it from \(.origin); the full run stopped before it"' <<<"$compared")
while IFS= read -r line; do
  [[ -n $line ]] || continue
  unchecked=$((unchecked + 1))
  note "$line"
done <<<"$unreached"
# A suspect crate's tests rerun alone, in the step's suite, with the OS
# services set up as `just pre-merge` sets them up, so a missing one fails
# instead of skipping its tests: red confirms the miss. The stress run's
# suspects are reported as they are.
if [[ "$(uname -s)" == Linux ]]; then
  just _os-services
  set -a
  # shellcheck source=/dev/null
  source "${RUNNER_TEMP:-/tmp}/goodfetch-os-services/env"
  set +a
  export GOODFETCH_OS_SERVICES=required
  export GOODFETCH_TEST_DNS_NAMESPACE=${GOODFETCH_TEST_DNS_NAMESPACE:-sudo}
fi
for step in quick pre-merge stress; do
  crates=$(suspect_crates "$step" | tr '\n' ' ')
  [[ -n ${crates// /} ]] || continue
  for crate in $crates; do
    if [[ $step != stress ]] && ! just test "$step" "$crate"; then
      misses=$((misses + 1))
      note "miss $step:$crate: reused by an incremental run on this tree, red when its tests rerun alone"
    elif [[ $step == stress ]]; then
      suspects=$((suspects + 1))
      note "suspect $step:$crate: reused by an incremental run on this tree, and the full run's stress run is red"
    fi
  done
done
# The crates whose stress result counted passes from an earlier head, by the
# base their run compared with: their whole counts, on this tree, whose change
# since that base holds their merge's and any later merge's. A stress run that
# cannot run is a miss too, so the day is no green run and the next one retries.
bases=$(jq -r '[.[] | select(.kind == "earned") | .base] | unique | .[]' <<<"$compared")
for base in $bases; do
  listed=$(jq -r --arg base "$base" '[.[] | select(.kind == "earned" and .base == $base) | .member] | unique | .[]' <<<"$compared" | tr '\n' ' ')
  read -ra crates <<<"$listed"
  earned=$((earned + ${#crates[@]}))
  if ! just stress "$base" "" "" merged "${crates[@]}"; then
    misses=$((misses + 1))
    note "miss stress: ${crates[*]}: a merge since the last green run counted their earlier passes, and their whole counts against $base are red or could not run here; a red may be any merge's since that base, and a base that can never run takes GOODFETCH_GATE_BACKSTOP_SINCE (TESTING.md)"
  fi
done

echo "$day $commit gate=$gate misses=$misses suspects=$suspects unchecked=$unchecked earned=$earned${told_since:+ since=$told_since}" >>"$logs/results.txt"
[[ $gate == pass && $misses -eq 0 ]]
