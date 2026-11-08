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
# crate's tests rerun alone and a red confirms the miss.
#
# Each run appends one line to `<log>/results.txt`, `<UTC day> <commit>
# gate=<pass|fail> misses=<n> suspects=<n> unchecked=<n>`, or
# `gate=skipped (…)` when the disk lacks room, and each miss, suspect or
# unchecked result to `<log>/misses.txt`; the run's own log is
# `<log>/<day>.log`, with `<log>`
# GOODFETCH_GATE_BACKSTOP_LOG or ~/gate-backstop. It fails when the full run
# fails or finds a miss. It keeps to bash 3.2, which stock macOS runs.
set -euo pipefail

ref=${1:-origin/main}
logs=${GOODFETCH_GATE_BACKSTOP_LOG:-$HOME/gate-backstop}
mkdir -p "$logs"
day=$(date -u +%F)
exec >>"$logs/$day.log" 2>&1
echo "== $(date -u +%FT%TZ) the incremental gate's backstop on $ref"

git fetch --quiet origin
commit=$(git rev-parse --verify "$ref^{commit}")

gate_worktree_name="gate-backstop"
# shellcheck source=ci/gate-worktree.sh
source "$(dirname "$0")/gate-worktree.sh"
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
trap 'rm -rf "$lock"' EXIT
gate_worktree_checkout "$commit"
# The incremental runs it compares with: `just merge`'s, read only.
merged_runs="$root/.claude/worktrees/merge-gate/target/gate-runs"
cd "$worktree"
just fetch

gate=pass
GOODFETCH_GATE_FULL=1 just pre-merge all || gate=fail
echo "== the full run on $commit: $gate"

misses=0 suspects=0 unchecked=0
# A JSON array of the results found, each with its kind, unit, step,
# member, run and origin (`xtask gate compare`).
compared=$(just _gate-compare "$commit" "$merged_runs") || {
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

echo "$day $commit gate=$gate misses=$misses suspects=$suspects unchecked=$unchecked" >>"$logs/results.txt"
[[ $gate == pass && $misses -eq 0 ]]
