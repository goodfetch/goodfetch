#!/usr/bin/env bash
# `just pre-merge [all|quick|docs]`: the Pre-merge suite on this host
# (TESTING.md, The three suites), which `just merge` runs before every merge
# (ADR 0135); `all` runs it over the whole workspace, else over what changed.
# `quick` is the gate of a change that no test, benchmark, model, container
# or Evaluation reads (`xtask affected`'s `gate=quick`): only the checks every
# change needs, over the whole workspace. `docs` is the gate of such a change
# made of documents alone (`gate=docs`): the checks that read its commits and
# the bar, then `just lint-documents`, the checks of `just lint` that read
# documents; no code lint and no suite.
#
# The checks that need Linux run beside the host steps, each in its own
# container, with its lines marked `[<step>]`: `just perf`, the counted
# figures (ci/perf.sh); `just linux`, the tests only Linux decides
# (ci/linux.sh), which on a Linux host run among the host's tests, after the
# OS-service setup below; `just visual`, the visual projects in the pinned Playwright
# image (ci/visual/container.sh). The suite waits for each and fails when one
# fails. The host's checks that take seconds and judge the whole change
# (sign-offs, commit messages, the floor guard, secrets in commits,
# advisories, notices) run first, before the Console build and the steps
# beside, so their reds show in the gate's first minute; the other checks of
# the whole change (lint, the SDK crates' features, the fuzz targets, the
# proofs) run before the suites; the stress run of the new and changed tests
# (`just stress`) runs after the Pre-merge suite, whose build its listings
# reuse.
#
# A red step ends no form of the gate (ADR 0135's amendment of 2026-11-10):
# every other step still runs, the steps beside to their own end, each step
# of the full gate records its result, and the gate's end names every red
# step and fails with the first one's exit status, as `just quick` reports
# every failure of its run. So a change meets all its reds in one gate, and
# the next gate reuses each green result the fix does not reach. Any red
# still means no merge. What ends the gate where it is, stopping the steps
# beside, is a fault of the gate itself: a setup of the OS services that
# fails, a step list or a plan the script cannot read, a result that cannot
# be recorded, or a stop.
#
# `all` is incremental (ADR 0135): on a head that descends from a head whose
# gate already ran, it reuses the green results of the steps, and of the
# members' tests, that the change since then does not reach, and reruns the
# rest; see the plan below. `GOODFETCH_GATE_FULL=1` reuses nothing.
#
# Each form's steps, their order and each step's `just` command come from
# `xtask gate steps` (`just _gate-steps`, xtask/src/gate/steps.rs), and the
# full gate's plan from `xtask gate plan`; the script reads both through
# ci/output-value.sh, so a missing key fails the gate instead of skipping
# or rerunning a step.
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

# The steps that ended red, each with its exit status, in the order the gate
# met them, and the gate's own status: the first one's.
red=()
verdict=0
# Notes that a step ended red with an exit status; the gate goes on.
went_red() {
  red+=("$1: exit status $2")
  [[ $verdict -ne 0 ]] || verdict=$2
}
# The gate's end: passes when no step ended red, else names each red step
# and fails with the first one's status.
conclude() {
  [[ ${#red[@]} -gt 0 ]] || exit 0
  local noun=steps
  [[ ${#red[@]} -gt 1 ]] || noun=step
  echo "error: the gate is red: ${#red[@]} $noun failed, and every other step ran" >&2
  printf '  %s\n' "${red[@]}" >&2
  exit "$verdict"
}

all=${1:-}
case "$all" in
  all | quick | docs | "") ;;
  *)
    echo "error: the argument is \`all\`, \`quick\` or \`docs\`, not $all" >&2
    exit 2
    ;;
esac
form=${all:-changed}

# shellcheck source=ci/output-value.sh
source "$(dirname "${BASH_SOURCE[0]}")/output-value.sh"

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

# The gate's forms use every thread, whatever the shell that started them
# capped: a caller's NEXTEST_TEST_THREADS or CARGO_BUILD_JOBS, such as the
# server's shared settings export for implementers, would cap nextest and
# Cargo for the whole gate (the test-gate audit's F1). A run over what
# changed keeps the caller's.
if [[ $all == all || $all == quick || $all == docs ]]; then
  removed=()
  for variable in NEXTEST_TEST_THREADS CARGO_BUILD_JOBS; do
    if [[ -n ${!variable+set} ]]; then
      removed+=("$variable=${!variable}")
      unset "$variable"
    fi
  done
  if [[ ${#removed[@]} -gt 0 ]]; then
    echo "The gate removed ${removed[*]} from its environment, so every step uses every thread"
  fi
fi

# The form's list: its steps' names in order, and each one's kind (`step`,
# `beside` or `command`) and `just` command. The checks that judge the
# whole change in seconds come first in every form, before any build and
# any step beside the host's, so the gate's log opens with their verdicts.
listing=$(just _gate-steps "$form")
order=$(output_value steps "$listing")
read -ra steps <<<"$order"

rm -f target/skipped.txt
# The docs and quick forms run their steps one after the other on the host,
# recording nothing.
if [[ $form == docs || $form == quick ]]; then
  trap 'just _skips-report' EXIT
  if [[ $form == docs ]]; then
    echo "The docs gate: the change is documents alone, so only the checks that read its commits, the bar and documents run; the code lints and every suite do not (xtask affected's gate=docs, ADR 0135)."
  else
    os_services
    echo "The quick gate: no test reads this change, so perf, the Linux-only and visual tests, the Pre-merge suite, the stress run, TLC, the MSRV build and Miri do not run (xtask affected's gate=quick, ADR 0135)."
  fi
  for step in "${steps[@]}"; do
    arguments=$(output_value "$step.just" "$listing")
    read -ra command <<<"$arguments"
    status=0
    timed "$step" just "${command[@]}" || status=$?
    [[ $status -eq 0 ]] || went_red "$step" "$status"
  done
  conclude
fi
beside_names=()
beside_pids=()
# Starts a step beside the host's, its command after its name, its lines
# marked with its name, in a process group of its own that it leads (job
# control, `set -m`, for that one command), so stopping the group stops
# every process the step started: `just` passes a stop on to its recipe's
# shell alone, and `just perf`'s `xtask perf build` and its `cargo bench`
# outlived it.
beside() {
  [[ -n $ended ]] || ended=$(mktemp -d)
  set -m
  timed "$1" beside_step "$@" > >(awk -v mark="[$1] " '{ print mark $0; fflush() }') 2>&1 &
  set +m
  beside_names+=("$1")
  beside_pids+=("$!")
}
# A step beside the host's notes its exit status in `ended` once it has
# ended, before its end is logged, so a gate that ends before it has waited
# for the step cannot lose the result of a step beside that had finished.
ended=""
# shellcheck disable=SC2329 # called through timed
beside_step() {
  local name=$1 status=0
  shift
  just "$@" || status=$?
  echo "$status" >"$ended/$name"
  return "$status"
}
# At the end, releases the gate's records, then lists what this host
# skipped. A gate that ends before it has waited for its steps beside, on a
# fault of its own or a stop, first stops the process groups of those still
# running and records each that had already ended. A step stopped while it
# ran records nothing, so the next run reruns it. `kill` signals each group
# it is given, even past one that has ended.
lock=""
# A stop reaches this script twice under `just merge`'s stop, through its
# process group and again from the `just` above it, and the second must not
# end it before it has done this.
# shellcheck disable=SC2329 # called through the EXIT trap
on_exit() {
  trap '' TERM
  if [[ ${#beside_pids[@]} -gt 0 ]]; then
    kill -TERM -- "${beside_pids[@]/#/-}" 2>/dev/null || true
    local name
    for name in "${beside_names[@]}"; do
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
# planned head only, which is `none` on a working tree with uncommitted
# changes. Over what changed there is no plan, and every step runs in full.
plan=""
previous=""
planned=""
if [[ $form == all ]]; then
  plan=$(just _gate-plan)
  previous=$(output_value previous "$plan")
  planned=$(output_value head "$plan")
fi
# A step's action in the plan: `run` without one.
action() {
  if [[ -z $plan ]]; then
    echo run
  else
    output_value "$1.action" "$plan"
  fi
}
# A part's members as its runners take them, on one line: the tests no
# member owns (`:unowned`) by their files, for `just test`.
members_of() {
  local listed unowned member members=""
  listed=$(output_value "$1.members" "$plan") || return
  unowned=$(output_value unowned "$plan") || return
  for member in $listed; do
    if [[ $member == :unowned ]]; then
      [[ $unowned == - ]] || members="$members $unowned"
    else
      members="$members $member"
    fi
  done
  echo "$members"
}
reused() {
  local why
  why=$(output_value "$1.why" "$plan") || return
  echo "Gate step $1: reused ($why)"
}
# A failed Quick suite or stress run is recorded with the files its crates
# are judged by, crate by crate (`xtask gate record --junit`): its nextest
# JUnit and, for the stress run, its selection, the commands that went red and
# when it began. Each crate that passed in full records green, every other
# member red; a failure they do not account for records every member red. A
# crate a red stress run left short of its counts, with no failing test, keeps
# its clean passes for the next run, unless that failure takes them all.
record() {
  [[ $form == all && $planned != none ]] || return 0
  if [[ $2 -eq 0 ]]; then
    just _gate-record "$planned" "$1" pass
    return
  fi
  local evidence=() folder
  case $1 in
    quick) evidence=(--junit target/nextest/quick/junit.xml) ;;
    # Only after nextest's tests alone failed (the suite's 100, kept when
    # the exercise audit passed).
    pre-merge) [[ $2 -ne 100 ]] || evidence=(--junit target/nextest/ci/junit.xml) ;;
    stress)
      evidence=(--selection target/stress-selection.tsv
        --red-commands target/stress-red-commands.tsv --started "$started")
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
    stress) rm -rf target/stress-selection.tsv target/stress-red-commands.tsv target/junit/repeat-* ;;
  esac
}
# When the gated step began, which a failed stress run's clean passes age from.
started=""
# Runs a step's command, records its result, and notes the step when it
# ended red. A result that cannot be recorded ends the gate.
gated() {
  local step=$1 status=0
  shift
  clear_evidence "$step"
  started=$(date +%s)
  timed "$step" "$@" || status=$?
  record "$step" "$status"
  [[ $status -eq 0 ]] || went_red "$step" "$status"
}
# The Pre-merge suite's tests (`just` with these arguments), then the
# exercise audit of the tests that ran (ADR 0072): one step, recorded once
# both have ended. The audit also runs after the suite's 100, nextest's
# tests red and its doctests green, which stays the step's status when the
# audit passes: `record` then judges each crate by the run's JUnit file (the
# test-gate audit's option 4b).
suite=()
# shellcheck disable=SC2329 # called through timed
suite_and_audit() {
  local status=0
  just "${suite[@]}" || status=$?
  [[ $status -eq 0 || $status -eq 100 ]] || return "$status"
  just features-audit || return
  return "$status"
}
pre_merge_suite() {
  local status=0
  suite=("$@")
  timed pre-merge suite_and_audit || status=$?
  record pre-merge "$status"
  [[ $status -eq 0 ]] || went_red pre-merge "$status"
}
# A part of a step run per member: the Quick and Pre-merge suites rerun
# every Vitest, Playwright and pytest test, as a full run does, in the
# head's order (`--shuffled-all`), and nextest over its crates; the stress
# run takes the new and changed tests of the members the change since the
# earlier run reaches, and each test that change changed, each running what
# its count still owes after the clean passes the plan kept (#2233).
run_part() {
  local part members=()
  case $1 in
    quick | pre-merge)
      part=$(members_of "$1")
      read -ra members <<<"$part"
      if [[ $1 == quick ]]; then
        gated quick just test quick --shuffled-all "${members[@]}"
      else
        pre_merge_suite test pre-merge --shuffled-all "${members[@]}"
      fi
      ;;
    stress)
      part=$(output_value stress.members "$plan")
      read -ra members <<<"$part"
      gated stress just stress "" "$previous" "target/gate-runs/$planned.earned" "" "${members[@]}"
      ;;
    *)
      echo "error: the plan runs a part of $1, which runs no member's tests" >&2
      exit 1
      ;;
  esac
}
# A step on the host, as its plan has it: reused, run in part, or run in
# full with its `just` command.
run_step() {
  local step=$1 planned_action
  shift
  planned_action=$(action "$step")
  case $planned_action in
    reuse) reused "$step" ;;
    part) run_part "$step" ;;
    run)
      if [[ $step == pre-merge ]]; then
        pre_merge_suite "$@"
      else
        gated "$step" just "$@"
      fi
      ;;
    *)
      echo "error: the plan's action for $step is $planned_action, not run, part or reuse" >&2
      exit 1
      ;;
  esac
}
# The form's steps in their order: the whole change's checks, Collie's
# Console build, which the steps beside, in their containers, find in the
# workspace they mount (ADR 0057), the steps beside, then the host's others.
for step in "${steps[@]}"; do
  arguments=$(output_value "$step.just" "$listing")
  read -ra command <<<"$arguments"
  kind=$(output_value "$step.kind" "$listing")
  case $kind in
    command)
      status=0
      timed "$step" just "${command[@]}" || status=$?
      [[ $status -eq 0 ]] || went_red "$step" "$status"
      ;;
    beside)
      planned_action=$(action "$step")
      if [[ $planned_action == reuse ]]; then reused "$step"; else beside "$step" "${command[@]}"; fi
      ;;
    step) run_step "$step" "${command[@]}" ;;
    *)
      echo "error: the kind of $step is $kind, not step, beside or command" >&2
      exit 1
      ;;
  esac
done

# The steps beside run to their own end, whatever the host's steps gave.
for i in "${!beside_names[@]}"; do
  echo "Waiting for ${beside_names[i]}…"
  status=0
  wait "${beside_pids[i]}" || status=$?
  record "${beside_names[i]}" "$status"
  [[ $status -eq 0 ]] || went_red "${beside_names[i]}" "$status"
done
beside_pids=()
conclude
