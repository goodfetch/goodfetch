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
# `all` is incremental (ADR 0135): on a head that descends from a head whose
# gate already ran, it reuses the green results of the steps, and of the
# members' tests, that the change since then does not reach, and reruns the
# rest; see the plan below. `GOODFETCH_GATE_FULL=1` reuses nothing.
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

# The incremental gate (ADR 0135), over the whole workspace only: `xtask
# gate plan` reads the results earlier gate runs recorded in
# target/gate-runs/ and, from what `xtask affected` selects for the change
# since the head each green result ran on, says for each step whether this
# run reruns it in full (`run`), reruns its tests of some members only
# (`part`), or reuses its green results (`reuse`); it logs each choice with
# the run and head it reuses from. Every step's result is recorded here
# (`just _gate-record`). Without a plan line a step runs in full.
plan=""
if [[ $all == all ]]; then
  plan=$(just _gate-plan)
fi
field() {
  awk -F'\t' -v step="$1" -v column="$2" '$1 == step { print $column }' <<<"$plan"
}
action() {
  local planned
  planned=$(field "$1" 2)
  echo "${planned:-run}"
}
previous=$(field previous 2)
reused() {
  echo "Gate step $1: reused ($(field "$1" 4))"
}
record() {
  [[ $all == all ]] || return 0
  if [[ $2 -eq 0 ]]; then just _gate-record "$1" pass; else just _gate-record "$1" fail; fi
}
# Runs a step's command, records its result, and fails the suite when it
# fails.
gated() {
  local step=$1 status=0
  shift
  "$@" || status=$?
  record "$step" "$status"
  return "$status"
}
# A step its plan may reuse: runs it unless the plan reuses it.
whole() {
  if [[ $(action "$1") == reuse ]]; then reused "$1"; else gated "$@"; fi
}
# Collie's executable embeds its Console build (ADR 0057), which the steps
# beside, in their containers, find in the workspace they mount.
just console-build
for step in perf linux visual; do
  if [[ $(action "$step") == reuse ]]; then reused "$step"; else beside "$step"; fi
done
gated dco just dco
gated commit-messages just commit-messages
gated lint just lint
gated feature-combinations just feature-combinations
gated audit just audit
gated commit-leaks just commit-leaks
gated fuzz-check just fuzz-check
gated proofs just proofs
case $(action quick) in
  reuse) reused quick ;;
  part)
    read -ra members <<<"$(field quick 3)"
    gated quick just test quick "${members[@]}"
    ;;
  *) gated quick just _suite quick "$all" ;;
esac
# The Pre-merge suite's tests, then the exercise audit of the tests that
# ran (ADR 0072): one step, whose last result is the audit's.
case $(action pre-merge) in
  reuse) reused pre-merge ;;
  part)
    read -ra members <<<"$(field pre-merge 3)"
    gated pre-merge just test pre-merge "${members[@]}"
    gated pre-merge just features-audit
    ;;
  *)
    gated pre-merge just _suite pre-merge "$all"
    gated pre-merge just features-audit
    ;;
esac
# The stress run of a part: the new and changed tests of the members the
# change since the earlier run reaches, and each test that change changed.
case $(action stress) in
  reuse) reused stress ;;
  part)
    read -ra members <<<"$(field stress 3)"
    gated stress just stress "" "$previous" "${members[@]}"
    ;;
  *) gated stress just stress ;;
esac
gated journeys-tasks just journeys-tasks --check
# One invocation, so the listings `journeys-tests` writes serve both.
gated features-tests just features-tests clients-tests
whole tlc just _tlc "$all"
whole msrv just msrv
whole miri just miri
gated notices just notices

status=0
for i in "${!names[@]}"; do
  echo "Waiting for ${names[i]}…"
  step=0
  wait "${pids[i]}" || step=$?
  record "${names[i]}" "$step"
  if [[ $step -ne 0 ]]; then
    echo "error: just ${names[i]} failed with exit status $step" >&2
    [[ $status -ne 0 ]] || status=$step
  fi
done
pids=()
exit "$status"
