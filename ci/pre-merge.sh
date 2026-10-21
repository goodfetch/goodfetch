#!/usr/bin/env bash
# `just pre-merge [all]`: the Pre-merge suite on this host (TESTING.md, The
# three suites), which `just merge` runs before every merge (ADR 0135); `all`
# runs it over the whole workspace, else over what changed.
#
# The checks that need Linux run beside the host steps, each in its own
# container, with its lines marked `[<step>]`: `just perf`, the counted
# figures (ci/perf.sh); `just linux`, the tests only Linux decides
# (ci/linux.sh); `just visual`, the visual projects in the pinned Playwright
# image (ci/visual/container.sh). The suite waits for each and fails when one
# fails, and stops them when a host step fails first. The host's checks that
# take seconds and judge the whole change (sign-offs, commit messages, the SDK
# crates' features, advisories, secrets in commits, the fuzz targets, the
# proofs) run before the suites; the stress
# run of the new and changed tests (`just stress`) runs after the Pre-merge
# suite, whose build its listings reuse.
#
# Every step runs through the `just` on PATH, where the recipe puts its own.
# It keeps to bash 3.2, which stock macOS runs.
set -euo pipefail

all=${1:-}
case "$all" in
  all | "") ;;
  *)
    echo "error: the only argument is \`all\`, not $all" >&2
    exit 2
    ;;
esac

rm -f target/skipped.txt
# Collie's executable embeds its Console build (ADR 0057), which the steps
# beside, in their containers, find in the workspace they mount.
just console-build
names=()
pids=()
# Starts a step beside the host's, its lines marked with its name.
beside() {
  just "$1" > >(awk -v mark="[$1] " '{ print mark $0; fflush() }') 2>&1 &
  names+=("$1")
  pids+=("$!")
}
# At the end, stops the steps still running beside, then lists what this
# host skipped.
# `kill` signals each process it is given, even past one that has ended.
trap '[[ ${#pids[@]} -eq 0 ]] || kill "${pids[@]}" 2>/dev/null || true; just _skips-report' EXIT

beside perf
beside linux
beside visual
just dco
just commit-messages
just lint
just feature-combinations
just audit
just commit-leaks
just fuzz-check
just proofs
just _suite quick "$all"
just _suite pre-merge "$all"
# Reads the Pre-merge suite's JUnit file and exercise records (ADR 0072).
just features-audit
just stress
just journeys-tasks --check
# One invocation, so the listings `journeys-tests` writes serve both.
just features-tests clients-tests
just _tlc "$all"
just msrv
just miri
just notices

status=0
for i in "${!names[@]}"; do
  echo "Waiting for ${names[i]}…"
  step=0
  wait "${pids[i]}" || step=$?
  if [[ $step -ne 0 ]]; then
    echo "error: just ${names[i]} failed with exit status $step" >&2
    [[ $status -ne 0 ]] || status=$step
  fi
done
pids=()
exit "$status"
