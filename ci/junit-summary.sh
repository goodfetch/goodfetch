#!/usr/bin/env bash
# Writes the job's failed tests to its summary page (ADR 0061; TESTING.md,
# Sharding, caching and reports): `xtask junit-summary` reads every JUnit
# file in target/junit/ and its repeat-<count>/ folders (`just stress`) and
# lists each failed test with its failure's first lines and its seeds, or
# says that none failed. Every test job runs it,
# pass or fail, after ci/junit-context.sh has gathered the JUnit files:
#   ci/junit-summary.sh       on the runner's own system, as the hosted jobs
#                             and the owner's machine's host steps do;
#   ci/junit-summary.sh vm    in the job's VM (ci/vm/exec.sh), where the
#                             owner's machine builds the repository's code.
# The step fails when a JUnit file is malformed; the page still shows the
# rest.
set -euo pipefail

where=${1:-host}
written=target/junit-summary.md
xtask=(cargo run --quiet --locked --bin xtask -- junit-summary --summary "$written")

mkdir -p target
rm -f "$written"
status=0
case "$where" in
  host) "${xtask[@]}" || status=$? ;;
  vm) ci/vm/exec.sh "${xtask[@]}" || status=$? ;;
  *)
    echo "usage: ci/junit-summary.sh [vm]" >&2
    exit 2
    ;;
esac

# The VM writes the summary into the workspace; the page's file is the host's.
if [[ -f "$written" ]]; then
  cat "$written" >>"${GITHUB_STEP_SUMMARY:-/dev/stdout}"
fi
exit "$status"
