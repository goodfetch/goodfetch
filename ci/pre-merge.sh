#!/usr/bin/env bash
# `just pre-merge [all|quick|batch|docs]`: the Pre-merge suite on this host
# (TESTING.md, The three suites), which `just merge` runs before every merge
# (ADR 0135); `all` runs it over the whole workspace, else over what changed.
# `quick` is the gate of a change that no test, benchmark, model, container
# or Evaluation reads (`xtask affected`'s `gate=quick`): only the checks every
# change needs, over the whole workspace, then xtask's codegen tests when the
# change reaches what only they check (`just codegen-tests`, which reads the
# change itself, issue 2892). `batch` is the hosted tier's form for a
# batch's pull request: the quick form's steps and the Pre-merge suite, each
# member's Pre-merge unit covering its Quick unit (`xtask gate`'s plan; the
# hosted shards run the suite itself, .github/workflows/pull-request.yml,
# issue 3378). `docs` is the gate of such a change
# made of documents alone (`gate=docs`): the checks that read its commits and
# the bar, then `just lint-documents`, the checks of `just lint` that read
# documents; no code lint and no suite. `preflight` is `just preflight`: the
# full gate's units that only the gate host runs, the counted rows and the
# stress run, planned and recorded as `all`'s are, by the layer `preflight`,
# so the full gate on the same tree, in whichever worktree, reuses them (ADR
# 0135's amendment of 2026-11-22).
#
# The checks that need Linux run beside the host steps, each in its own
# container, with its lines marked `[<step>]`: `just perf`, the counted
# figures, and `just counted-rows`, the Quality register's counted rows
# (ci/perf.sh); `just linux`, the tests only Linux decides
# (ci/linux.sh), which on a Linux host run among the host's tests, after the
# OS-service setup below; `just visual`, the visual projects in the pinned Playwright
# image (ci/visual/container.sh); `just windows`, the workspace built for
# Windows and its tests under Wine (ci/windows.sh, issue 2767). TLC, Miri
# (issue 2090's F7) and loom (issue 2200) run beside them too, on the host:
# each builds and writes in folders of its own (`target/tlc/`,
# `target/miri/`, `target/loom/`; `target/windows/` for Windows), which
# no other step reads, and its verdict
# follows from the tree and its pinned tools; what it shares with the host's
# steps it only reads or waits for (ADR 0135's note of 2026-11-13). The
# Kennel's check does not: it
# builds the test tools into the folder the suites' own are in, so it runs
# after them. The suite waits for each and fails when one
# fails. The host's checks that take seconds and judge the whole change
# (sign-offs, commit messages, the floor guard, secrets in commits,
# advisories, notices) run first, before the steps
# beside, so their reds show in the gate's first minute; the other checks of
# the whole change (lint, the SDK crates' features, the fuzz targets, the
# proofs) run before the suites; the stress run of the new and changed tests
# (`just stress`) runs after the Pre-merge suite, whose build its listings
# reuse. In the full gate it takes the tests new and changed since the last
# full gate's head, when a test reads what merged since (the plan's
# `stress.base`), so those a quick merge added get their runs there.
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
# `all` and `quick` are incremental (ADR 0135): on a head that descends from
# a head whose gate already ran, they reuse the green results of the steps,
# and of the members' tests, that the change since then does not reach, and
# rerun the rest; see the plan below. They also reuse the hosted-owned
# units a hosted run of the pull-request workflow holds, whose records
# `just merge` downloaded (GOODFETCH_GATE_HOSTED_RECORDS), and end red on
# one such run's red unit on the head's tree (ADR 0135's amendment of
# 2026-11-22). `GOODFETCH_GATE_FULL=1` reuses nothing.
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
  all | quick | batch | docs | preflight | "") ;;
  *)
    echo "error: the argument is \`all\`, \`quick\`, \`batch\`, \`docs\` or \`preflight\`, not $all" >&2
    exit 2
    ;;
esac
form=${all:-changed}
# The forms the plan reads and the record keeps: the full gate's,
# preflight's units of it, and the quick gate's and a batch's, whose
# hosted-owned units a hosted run on the tree already holds (ADR 0135's
# amendment of 2026-11-22).
planning=""
[[ $form != all && $form != preflight && $form != quick && $form != batch ]] || planning=yes
# The layer the record names for each result this run gives (`xtask gate
# record --by`): `preflight` for that form, else `GOODFETCH_GATE_LAYER`'s,
# which the backstop and the hosted jobs set, and `merge` by default. A
# hosted job runs one step, `GOODFETCH_GATE_ONLY`, and every other step of
# its form is another job's (.github/workflows/pull-request.yml).
layer=${GOODFETCH_GATE_LAYER:-merge}
[[ $form != preflight ]] || layer=preflight

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
if [[ $form != changed ]]; then
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

rm -rf target/skipped.txt target/red-steps
# The docs form runs its steps one after the other on the host, recording
# nothing.
if [[ $form == docs ]]; then
  trap 'just _skips-report' EXIT
  echo "The docs gate: the change is documents alone, so only the checks that read its commits, the bar and documents run; the code lints and every suite do not (xtask affected's gate=docs, ADR 0135)."
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
# At the end, releases the gate's locks, then lists what this host
# skipped. A gate that ends before it has waited for its steps beside, on a
# fault of its own or a stop, first stops the process groups of those still
# running and records each that had already ended. A step stopped while it
# ran records nothing, so the next run reruns it. `kill` signals each group
# it is given, even past one that has ended.
locks=()
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
  [[ ${#locks[@]} -eq 0 ]] || rm -rf "${locks[@]}"
  just _skips-report
}
trap on_exit EXIT

# A whole-workspace gate run that narrows what runs or swaps the
# executables under test would record results a normal run then reuses, so
# it fails before any step (ADR 0135). The rest of the environment that
# changes how tests run is part of the record's host fingerprint.
if [[ -n $planning ]]; then
  for variable in GOODFETCH_TEST_SELECTION GOODFETCH_TEST_ENGINE GOODFETCH_ORDER_SEED; do
    if [[ -n "${!variable:-}" ]]; then
      echo "error: $variable is set, which narrows or changes what \`just pre-merge $form\` runs; unset it for a gate run" >&2
      exit 2
    fi
  done
fi

# Takes the lock `$1` for this run, a folder holding its process ID, or
# fails before any step naming what it guards (`$2`) while the process that
# holds it lives; a lock whose holder ended without releasing it is taken.
take_lock() {
  local holder
  mkdir -p "$(dirname "$1")"
  if ! mkdir "$1" 2>/dev/null; then
    holder=$(cat "$1/pid" 2>/dev/null || true)
    if [[ -n "$holder" ]] && kill -0 "$holder" 2>/dev/null; then
      echo "error: another gate run (process $holder) holds $2; run it again when that run ends" >&2
      exit 1
    fi
    rm -rf "$1"
    mkdir "$1"
  fi
  locks+=("$1")
  echo "$$" >"$1/pid"
}
# One gate run at a time in a worktree, since two would share the tests'
# JUnit files and exercise records; and one at a time on a head, in whichever
# worktree, since every worktree records into the one folder
# (ci/gate-records.sh), where a second run's plan would replace the first's
# record and plan of that head while the first still records into them.
records=""
if [[ -n $planning ]]; then
  records=$("$(dirname "${BASH_SOURCE[0]}")/gate-records.sh")
  take_lock target/gate-lock "this worktree's gate"
  take_lock "$records/$(git rev-parse --verify --quiet HEAD 2>/dev/null || echo none).lock" "the record of this head"
fi

os_services

# The incremental gate (ADR 0135), over the whole workspace only: `xtask
# gate plan` reads the results earlier gate runs of every layer recorded in
# the shared folder (ci/gate-records.sh) and, from what `xtask affected` selects for the change
# since the head each green result ran on, says for each step whether this
# run reruns it in full (`run`), reruns its tests of some members only
# (`part`), or reuses its green results (`reuse`); it logs each choice with
# the run and head it reuses from, and checks each reused member against the
# evidence of what it read (`xtask gate`'s `evidence`). Every step's result
# is recorded once its whole step has ended (`just _gate-record`), for the
# planned head only, which is `none` on a working tree with uncommitted
# changes. Over what changed there is no plan, and every step runs in full.
#
# The plan also names the head the stress run compares this one with in
# place of the change's base (`stress.base`, `-` for the change's base): the
# head of the last full gate `main` holds, so the run covers the tests the
# quick merges since then added or changed (ADR 0135's amendment of
# 2026-11-10). `stress_base` is empty for the change's base, as `just
# stress` takes it.
plan=""
previous=""
planned=""
stress_base=""
if [[ -n $planning ]]; then
  [[ $form != preflight ]] || echo "Preflight: the full gate's units only the gate host runs, the counted rows and the stress run, recorded by the layer preflight in $records, so the full gate on the same tree reuses them (ADR 0135)."
  [[ $form != quick ]] || echo "The quick gate: no test reads this change, so perf, the Linux-only and visual tests, the Pre-merge suite, the stress run, TLC, the MSRV build, Miri, loom and the Windows build under Wine do not run (xtask affected's gate=quick, ADR 0135)."
  plan=$(just _gate-plan "$form" "$layer")
  previous=$(output_value previous "$plan")
  planned=$(output_value head "$plan")
  stress_base=$(output_value stress.base "$plan")
  [[ $stress_base != - ]] || stress_base=""
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
# A failed Pre-merge suite or stress run is recorded with the files its
# crates are judged by, crate by crate (`xtask gate record --junit`): its
# nextest JUnit and, for the stress run, its selection, the commands that went
# red and when it began. Each crate that passed in full records green, every other
# member red; a failure they do not account for records every member red. A
# crate a red stress run left short of its counts, with no failing test, keeps
# its clean passes for the next run, unless that failure takes them all.
record() {
  [[ -n $planning && $planned != none ]] || return 0
  if [[ $2 -eq 0 ]]; then
    just _gate-record "$layer" "$planned" "$1" pass
    return
  fi
  local evidence=() folder
  case $1 in
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
  just _gate-record "$layer" "$planned" "$1" fail ${evidence[@]+"${evidence[@]}"}
}
# Removes what an earlier run of a step left of the files its record reads,
# so they are this run's.
clear_evidence() {
  case $1 in
    stress) rm -rf target/stress-selection.tsv target/stress-red-commands.tsv target/junit/repeat-* ;;
  esac
}
# When the gated step began, which a failed stress run's clean passes age from.
started=""
# Copies what the test runners have written, as it is when a step ends red,
# to target/red-steps/<step>/, for the failing gate's kept results
# (ci/merge-pr.sh): the gate goes on, and the steps after a red one clear or
# write over those files. The stress run removes nextest's JUnit files before
# each of its counts, and each suite writes Vitest's, Playwright's and
# pytest's anew. It covers the host's steps the gate records; the steps
# beside and the docs form get no folder. A
# folder or a copy that cannot be made only warns: it changes no verdict.
keep_results() {
  local kept=target/red-steps/$1 results
  for results in target/nextest target/junit; do
    [[ -e $results ]] || continue
    if ! { mkdir -p "$kept" && cp -R "$results" "$kept/"; }; then
      echo "warning: $results, as the red step $1 left it, was not kept in $kept" >&2
    fi
  done
}
# Runs a step's command, records its result, and, when it ended red, keeps
# its result files and notes the step. A result that cannot be recorded ends
# the gate.
gated() {
  local step=$1 status=0
  shift
  clear_evidence "$step"
  started=$(date +%s)
  timed "$step" "$@" || status=$?
  record "$step" "$status"
  [[ $status -eq 0 ]] || keep_results "$step"
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
  [[ $status -eq 0 ]] || keep_results pre-merge
  [[ $status -eq 0 ]] || went_red pre-merge "$status"
}
# A part of a step run per member: the Pre-merge suite reruns every Vitest,
# Playwright and pytest test, as a full run does, in the head's order
# (`--shuffled-all`), and nextest over its crates; the stress
# run takes the new and changed tests of the members the change since the
# earlier run reaches, and each test that change changed, each running what
# its count still owes after the clean passes the plan kept (#2233); new and
# changed since the plan's `stress.base`, when it names one.
run_part() {
  local part members=()
  case $1 in
    pre-merge)
      part=$(members_of "$1")
      read -ra members <<<"$part"
      pre_merge_suite test pre-merge --shuffled-all "${members[@]}"
      ;;
    stress)
      part=$(output_value stress.members "$plan")
      read -ra members <<<"$part"
      gated stress just stress "$stress_base" "$previous" "$records/$planned.earned" "" "${members[@]}"
      ;;
    quick)
      part=$(members_of "$1")
      read -ra members <<<"$part"
      gated quick just test quick --shuffled-all "${members[@]}"
      ;;
    *)
      echo "error: the plan runs a part of $1, which runs no member's tests" >&2
      exit 1
      ;;
  esac
}
# A step the plan runs nowhere here: covered by another step of the form,
# or, in a hosted job, another layer's (`left`); its why says which.
not_run() {
  local why
  why=$(output_value "$1.why" "$plan") || return
  echo "Gate step $1: not run ($why)"
}
# A step a hosted run on the head's tree ended red (`red`): the gate is
# red, naming it, and runs it not again (ADR 0064).
red_elsewhere() {
  local why
  why=$(output_value "$1.why" "$plan") || return
  echo "Gate step $1: red ($why)"
  went_red "$1 ($why)" 1
}
# A step on the host, as its plan has it: reused, run in part, or run in
# full with its `just` command, the stress run's with the plan's
# `stress.base` when it names one.
run_step() {
  local step=$1 planned_action
  shift
  planned_action=$(action "$step")
  case $planned_action in
    reuse) reused "$step" ;;
    covered | left) not_run "$step" ;;
    red) red_elsewhere "$step" ;;
    part) run_part "$step" ;;
    run)
      if [[ $step == pre-merge ]]; then
        pre_merge_suite "$@"
      elif [[ $step == stress && -n $stress_base ]]; then
        gated stress just "$@" "$stress_base"
      else
        gated "$step" just "$@"
      fi
      ;;
    *)
      echo "error: the plan's action for $step is $planned_action, not run, part, reuse, covered, left or red" >&2
      exit 1
      ;;
  esac
}
# The form's steps in their order: the whole change's checks, the steps
# beside, then the host's others.
for step in "${steps[@]}"; do
  if [[ -n ${GOODFETCH_GATE_ONLY:-} && $step != "$GOODFETCH_GATE_ONLY" ]]; then
    echo "Gate step $step: not run (this job runs $GOODFETCH_GATE_ONLY alone)"
    continue
  fi
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
      case $planned_action in
        reuse) reused "$step" ;;
        covered | left) not_run "$step" ;;
        red) red_elsewhere "$step" ;;
        *) beside "$step" "${command[@]}" ;;
      esac
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
