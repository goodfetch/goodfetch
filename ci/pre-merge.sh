#!/usr/bin/env bash
# `just pre-merge [all|quick]`: the Pre-merge suite on this host (TESTING.md,
# The three suites), which `just merge` runs before every merge (ADR 0135);
# `all` runs it over the whole workspace, else over what changed. `quick` is
# the gate of a change that no test, benchmark, model, container or
# Evaluation reads (`xtask affected`'s `gate=quick`): only the checks every
# change needs, over the whole workspace.
#
# The checks that need Linux run beside the host steps, each in its own
# container, with its lines marked `[<step>]`: `just perf`, the counted
# figures (ci/perf.sh); `just linux`, the tests only Linux decides
# (ci/linux.sh), which on a Linux host run among the host's tests, after the
# OS-service setup below; `just visual`, the visual projects in the pinned Playwright
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
  all | quick | "") ;;
  *)
    echo "error: the argument is \`all\` or \`quick\`, not $all" >&2
    exit 2
    ;;
esac

# On Linux the host runs the tests only Linux decides itself (ci/linux.sh),
# so it sets up every OS service they use first, as a CI job does, and every
# later step finds them through the setup's environment file. A service that
# goes missing fails the suite instead of skipping its tests
# (`GOODFETCH_OS_SERVICES=required`), and the DNS namespace's way is declared
# (TESTING.md, `DnsNamespace`), `sudo -n` as the setup itself needs, so a
# silent fallback fails too.
os_services() {
  [[ "$(uname -s)" == Linux ]] || return 0
  just _os-services
  set -a
  # shellcheck source=/dev/null
  source "${RUNNER_TEMP:-/tmp}/goodfetch-os-services/env"
  set +a
  export GOODFETCH_OS_SERVICES=required
  export GOODFETCH_TEST_DNS_NAMESPACE=${GOODFETCH_TEST_DNS_NAMESPACE:-sudo}
}

rm -f target/skipped.txt
if [[ $all == quick ]]; then
  trap 'just _skips-report' EXIT
  os_services
  echo "The quick gate: no test reads this change, so perf, the Linux-only and visual tests, the Pre-merge suite, the stress run, TLC, the MSRV build and Miri do not run (xtask affected's gate=quick, ADR 0135)."
  just dco
  just commit-messages
  just lint
  just audit
  just commit-leaks
  just _suite quick all
  just journeys-tasks --check
  just notices
  exit 0
fi
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

os_services
# Collie's executable embeds its Console build (ADR 0057), which the steps
# beside, in their containers, find in the workspace they mount.
just console-build
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
