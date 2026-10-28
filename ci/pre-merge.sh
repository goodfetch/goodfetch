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
# fails, and stops them when a host step fails first, keeping the result of
# each that had already ended for the gate's record. The host's checks that
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
# Each step logs when it started and when it ended, with its exit status,
# so the gate's log says where its time went.
#
# Every step runs through the `just` on PATH, where the recipe puts its own.
# It keeps to bash 3.2, which stock macOS runs.
set -euo pipefail

# Runs a step's command between the lines that say when it started and
# ended, in UTC, and returns its exit status.
timed() {
  local step=$1 status=0 started
  shift
  started=$(date -u +%s)
  echo "Gate step $step: started at $(date -u +%Y-%m-%dT%H:%M:%SZ)"
  "$@" || status=$?
  echo "Gate step $step: ended at $(date -u +%Y-%m-%dT%H:%M:%SZ) after $(($(date -u +%s) - started)) s, exit status $status"
  return "$status"
}

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
  timed os-services just _os-services
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
  timed dco just dco
  timed commit-messages just commit-messages
  timed lint just lint
  timed audit just audit
  timed commit-leaks just commit-leaks
  timed quick just _suite quick all
  timed journeys-tasks just journeys-tasks --check
  timed notices just notices
  exit 0
fi
names=()
pids=()
# Starts a step beside the host's, its lines marked with its name, in a
# process group of its own that it leads (job control, `set -m`, for that
# one command), so stopping the group stops every process the step started:
# `just` passes a stop on to its recipe's shell alone, and `just perf`'s
# `xtask perf build` and its `cargo bench` outlived it.
beside() {
  [[ -n $ended ]] || ended=$(mktemp -d)
  set -m
  timed "$1" beside_step "$1" > >(awk -v mark="[$1] " '{ print mark $0; fflush() }') 2>&1 &
  set +m
  names+=("$1")
  pids+=("$!")
}
# A step beside the host's notes its exit status in `ended` once it has
# ended, before its end is logged, so a host step that fails first cannot
# lose the result of a step beside that had finished.
ended=""
# shellcheck disable=SC2329 # called through timed
beside_step() {
  local status=0
  just "$1" || status=$?
  echo "$status" >"$ended/$1"
  return "$status"
}
# At the end, stops the process groups of the steps still running beside
# and records each step beside that had already ended, releases the gate's
# records, then lists what this host skipped. A step stopped while it ran
# records nothing, so the next run reruns it. `kill` signals each group it
# is given, even past one that has ended.
lock=""
# shellcheck disable=SC2329 # called through the EXIT trap
on_exit() {
  if [[ ${#pids[@]} -gt 0 ]]; then
    kill -TERM -- "${pids[@]/#/-}" 2>/dev/null || true
    local name
    for name in "${names[@]}"; do
      if [[ -f $ended/$name ]]; then record "$name" "$(cat "$ended/$name")" || true; fi
    done
  fi
  [[ -z $ended ]] || rm -rf "$ended"
  [[ -z $lock ]] || rm -rf "$lock"
  just _skips-report
}
trap on_exit EXIT

# A whole-workspace gate run that narrows what runs or swaps the
# executables under test would record results a normal run then reuses, so
# it fails before any step (ADR 0135). The rest of the environment that
# changes how tests run is part of the record's host fingerprint.
if [[ $all == all ]]; then
  for variable in GOODFETCH_TEST_SELECTION GOODFETCH_TEST_ENGINE GOODFETCH_ORDER_SEED; do
    if [[ -n "${!variable:-}" ]]; then
      echo "error: $variable is set, which narrows or changes what \`just pre-merge all\` runs; unset it for a gate run" >&2
      exit 2
    fi
  done
fi

# One gate run at a time records into a workspace's target/gate-runs/: a
# second run while the first's process lives fails before any step, since
# both would share the tests' JUnit files and exercise records too.
if [[ $all == all ]]; then
  mkdir -p target/gate-runs
  if ! mkdir target/gate-runs/lock 2>/dev/null; then
    holder=$(cat target/gate-runs/lock/pid 2>/dev/null || true)
    if [[ -n "$holder" ]] && kill -0 "$holder" 2>/dev/null; then
      echo "error: another gate run (process $holder) holds target/gate-runs/; run it again when that run ends" >&2
      exit 1
    fi
    # The run that held it ended without releasing it.
    rm -rf target/gate-runs/lock
    mkdir target/gate-runs/lock
  fi
  lock=target/gate-runs/lock
  echo "$$" >"$lock/pid"
fi

os_services

# The incremental gate (ADR 0135), over the whole workspace only: `xtask
# gate plan` reads the results earlier gate runs recorded in
# target/gate-runs/ and, from what `xtask affected` selects for the change
# since the head each green result ran on, says for each step whether this
# run reruns it in full (`run`), reruns its tests of some members only
# (`part`), or reuses its green results (`reuse`); it logs each choice with
# the run and head it reuses from, and checks each reused member against the
# evidence of what it read (`xtask gate`'s `evidence`). Every step's result
# is recorded once its whole step has ended (`just _gate-record`), for the
# planned head only. Without a plan line a step runs in full.
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
planned=$(field head 2)
# A part's members as its runners take them: the tests no member owns
# (`:unowned`) by their files, for `just test`.
members_of() {
  local member
  for member in $(field "$1" 3); do
    if [[ $member == :unowned ]]; then
      local unowned
      unowned=$(field unowned 2)
      [[ $unowned == - ]] || echo "$unowned"
    else
      echo "$member"
    fi
  done
}
reused() {
  echo "Gate step $1: reused ($(field "$1" 4))"
}
# A failed Quick suite or stress run is recorded with the files its crates
# are judged by, crate by crate (`xtask gate record --junit`): its nextest
# JUnit and, for the stress run, its selection. Each crate that passed in
# full records green, every other member red; a failure they do not account
# for records every member red.
record() {
  [[ $all == all && -n $planned ]] || return 0
  if [[ $2 -eq 0 ]]; then
    just _gate-record "$planned" "$1" pass
    return
  fi
  local evidence=() folder
  case $1 in
    quick) evidence=(--junit target/nextest/quick/junit.xml) ;;
    stress)
      evidence=(--selection target/stress-selection.tsv)
      for folder in target/junit/repeat-*/; do
        [[ ! -d $folder ]] || evidence+=(--junit "$folder")
      done
      ;;
  esac
  just _gate-record "$planned" "$1" fail ${evidence[@]+"${evidence[@]}"}
}
# Removes what an earlier run of a step left of the files its record reads,
# so they are this run's.
clear_evidence() {
  case $1 in
    quick) rm -f target/nextest/quick/junit.xml ;;
    stress) rm -rf target/stress-selection.tsv target/junit/repeat-* ;;
  esac
}
# Runs a step's command, records its result, and fails the suite when it
# fails.
gated() {
  local step=$1 status=0
  shift
  clear_evidence "$step"
  timed "$step" "$@" || status=$?
  record "$step" "$status"
  return "$status"
}
# A step its plan may reuse: runs it unless the plan reuses it.
whole() {
  if [[ $(action "$1") == reuse ]]; then reused "$1"; else gated "$@"; fi
}
# Collie's executable embeds its Console build (ADR 0057), which the steps
# beside, in their containers, find in the workspace they mount.
timed console-build just console-build
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
# A part reruns every Vitest, Playwright and pytest test, as a full run
# does, in the head's order (`--shuffled-all`), and nextest over its crates.
case $(action quick) in
  reuse) reused quick ;;
  part)
    read -ra members <<<"$(members_of quick | tr '\n' ' ')"
    gated quick just test quick --shuffled-all "${members[@]}"
    ;;
  *) gated quick just _suite quick "$all" ;;
esac
# The Pre-merge suite's tests, then the exercise audit of the tests that
# ran (ADR 0072): one step, recorded once both have ended.
case $(action pre-merge) in
  reuse) reused pre-merge ;;
  *)
    # shellcheck disable=SC2329 # called through timed
    suite_and_audit() { just "${suite[@]}" && just features-audit; }
    if [[ $(action pre-merge) == part ]]; then
      read -ra members <<<"$(members_of pre-merge | tr '\n' ' ')"
      suite=(test pre-merge --shuffled-all "${members[@]}")
    else
      suite=(_suite pre-merge "$all")
    fi
    status=0
    timed pre-merge suite_and_audit || status=$?
    record pre-merge "$status"
    [[ $status -eq 0 ]] || exit "$status"
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
