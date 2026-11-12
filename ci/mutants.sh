#!/usr/bin/env bash
# ADR 0072's in-diff mutation testing (TESTING.md, Code coverage and mutation
# testing): cargo-mutants over the Rust code a change touches, with nextest.
#   ci/mutants.sh plan <base>          prints `mutants-shards=<JSON array>`,
#                                      the shard indices the change needs
#                                      (`[]` when it touches no Rust code), at
#                                      most MUTANTS_PER_SHARD mutants each, so
#                                      each shard keeps to the 15-minute job
#                                      budget; it lists mutants and builds
#                                      nothing;
#   ci/mutants.sh run <base> [<k>/<n>] tests one shard (0-based, `0/1` by
#                                      default, every mutant), and its report
#                                      goes to target/mutants/, with the
#                                      surviving mutants in summary.md.
# A mutant that survives, or whose tests time out, never fails it: the
# report lists it, and the pull request's Evidence section answers it. It
# fails only when it cannot judge a mutant: the unmutated tree's build or
# tests fail, or cargo-mutants is given a wrong argument.
set -euo pipefail

# Mutants per 15-minute shard: each builds the mutated crate and runs its
# package's tests, about 2 minutes for xtask's on the owner's machine
# natively (11 in 21 minutes, #321), which leaves the VM's start and the
# baseline room. Raise or lower it from the jobs' recorded durations.
MUTANTS_PER_SHARD=4

mode=${1:?usage: ci/mutants.sh plan|run <base> [<k>/<n>]}
base=${2:?usage: ci/mutants.sh plan|run <base> [<k>/<n>]}
root=$(cd "$(dirname "$0")/.." && pwd)
cd "$root"
out=target/mutants
mkdir -p "$out"

# The change's Rust lines, which `--in-diff` reads.
git diff --no-ext-diff "$base" HEAD -- '*.rs' >"$out/change.diff"

case "$mode" in
  plan)
    if [[ ! -s "$out/change.diff" ]]; then
      echo "mutants-shards=[]"
      exit 0
    fi
    count=$(cargo mutants --in-diff "$out/change.diff" --list | wc -l | tr -d ' ')
    shards=$(((count + MUTANTS_PER_SHARD - 1) / MUTANTS_PER_SHARD))
    echo "mutants-shards=$(jq --compact-output --null-input --argjson n "$shards" '[range($n)]')"
    ;;
  run)
    shard=${3:-0/1}
    if [[ ! -s "$out/change.diff" ]]; then
      echo "The change touches no Rust code: no mutant to test."
      exit 0
    fi
    # Each mutant is written into the working tree itself and taken out
    # again (`--in-place`), so the tests find this tree's executables and
    # its warm build; uncommitted Rust changes would be in the way.
    if ! git diff --quiet HEAD -- '*.rs'; then
      echo "error: commit the Rust changes first: mutants are tested in the working tree, which git restores" >&2
      exit 2
    fi
    # The tests find the commit's executables as under `just test`.
    export GOODFETCH_TEST_SUITE=pre-merge
    "${JUST:-just}" _test-tools
    # Each mutant changes a source the tools were built from, on purpose: a
    # test would be refused them as older than their sources and every
    # mutant would count as caught. The tests read a copy of the manifest
    # that has no input record, which is read as it is (TESTING.md, Shared
    # helpers); the manifest itself keeps its record.
    cp "$root/target/test-tools.json" "$root/$out/test-tools.json"
    export GOODFETCH_TEST_TOOLS="$root/$out/test-tools.json"
    # nextest skips what `just test` skips: the quarantined tests (ADR 0064)
    # and those needing an OS service this host lacks (ci/nextest-skips.sh).
    skip=()
    exclusions=$(cargo run --quiet --bin xtask -- quarantine exclude)
    skips=$(ci/nextest-skips.sh "$exclusions")
    while IFS= read -r argument; do [[ -z "$argument" ]] || skip+=("$argument"); done <<<"$skips"
    status=0
    cargo mutants --in-diff "$out/change.diff" --test-tool nextest --shard "$shard" \
      --no-shuffle --in-place --output "$out" -- ${skip[@]+"${skip[@]}"} || status=$?
    # 2: a mutant survived; 3: a mutant's tests timed out. Both are listed in
    # the report and fail nothing.
    case "$status" in
      0 | 2 | 3) status=0 ;;
    esac
    missed="$out/mutants.out/missed.txt"
    {
      echo "### Surviving mutants, shard $shard"
      echo
      if [[ -s "$missed" ]]; then
        sed 's/^/- /' "$missed"
      else
        echo "None."
      fi
    } | tee "$out/summary.md"
    exit "$status"
    ;;
  *)
    echo "error: the mode is plan or run, not $mode" >&2
    exit 2
    ;;
esac
