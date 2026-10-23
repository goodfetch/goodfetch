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
# crate's tests rerun alone and a red confirms the miss. Last, the
# quarantined tests run as the test-health job runs them, 50 times each of
# the 10 oldest (ADR 0064), which no gate runs; their result blocks nothing
# and is reported.
#
# Each run appends one line to `<log>/results.txt`, `<UTC day> <commit>
# gate=<pass|fail> misses=<n> suspects=<n> unchecked=<n>
# quarantined=<pass|fail>`, and each miss, suspect or unchecked result to
# `<log>/misses.txt`; the run's own log is `<log>/<day>.log`, with `<log>`
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
compared=$(just _gate-compare "$commit" "$merged_runs") || {
  echo "error: the full run on $commit could not be compared" >&2
  compared=""
  gate=fail
}
note() {
  echo "$day $commit $*" >>"$logs/misses.txt"
  echo "== $*"
}
# The suspect crates of each step run per member, by step.
suspect_crates() {
  awk -F'\t' -v step="$1" '$1 == "suspect" { split($2, unit, ":"); if (unit[1] == step) print unit[2] }' <<<"$compared" |
    sort -u
}
while IFS=$'\t' read -r kind unit run origin; do
  case $kind in
    miss)
      misses=$((misses + 1))
      note "miss $unit: the run on $run reused it from $origin; the full run found it red"
      ;;
    unchecked)
      unchecked=$((unchecked + 1))
      note "unchecked $unit: the run on $run reused it from $origin; the full run stopped before it"
      ;;
  esac
done <<<"$compared"
# A suspect crate's tests rerun alone, in the step's suite, and the
# quarantined tests run, with the OS services set up as `just pre-merge`
# sets them up, so a missing one fails instead of skipping its tests: red
# confirms the miss. The stress run's suspects are reported as they are.
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

quarantined=pass
just repeat 50 --quarantined 10 || quarantined=fail
echo "== the quarantined tests: $quarantined (blocks nothing, ADR 0064)"

echo "$day $commit gate=$gate misses=$misses suspects=$suspects unchecked=$unchecked quarantined=$quarantined" >>"$logs/results.txt"
[[ $gate == pass && $misses -eq 0 ]]
