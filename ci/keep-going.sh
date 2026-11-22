# shellcheck shell=bash
# Keep-going steps, sourced by `just quick` (ci/quick.sh), `just lint`,
# `just lint-documents` and `just fmt-check`: every step runs even after one
# failed, so one run reports every failure at once, and the run then lists
# each failed step with its log and fails (the 2026-11-06 retro: a `just`
# recipe of plain lines stopped at its first red line, so a ticket needed one
# `just quick` per defect).
#
#   steps_begin <label>               start a run; logs go to target/steps/<label>/,
#                                     else to a temporary folder
#   step [--after <step>]... <name> <command>
#                                     run <command> with `bash -euo pipefail -c`,
#                                     as a recipe line runs, its output shown and
#                                     kept in <name>.log; skipped, and named so,
#                                     when a step it comes --after did not pass
#   steps_end                         print the summary; fails when a step did not pass
#   steps_mark                        print a mark of the steps that did not pass so far
#   steps_red_since <mark>            succeed when a step did not pass after the mark
#
# A run started inside another run's step (`just quick`'s step `just lint`)
# records its steps in the outer run's list, and only the outer run prints
# the summary, naming the inner run's failed steps rather than the step that
# started it. With GOODFETCH_STEPS_LIST=1, `step` prints its command and the
# steps it comes after instead of running it (`just quick-steps`).
#
# It keeps to bash 3.2, which stock macOS runs.

steps_begin() {
  _steps_label=$1
  _steps_bad='|'
  _steps_red=0
  _steps_parent=${GOODFETCH_STEP:-}
  [[ ${GOODFETCH_STEPS_LIST:-} == 1 ]] && return 0
  if [[ -n ${GOODFETCH_STEPS_DIR:-} ]]; then
    _steps_outer=
  else
    _steps_outer=1
    # A target/ that cannot hold the logs (a link to a removed build folder)
    # never fails the run: the logs go to a new temporary folder instead.
    if mkdir -p "target/steps/$_steps_label" 2>/dev/null; then
      GOODFETCH_STEPS_DIR=$(cd "target/steps/$_steps_label" && pwd -P)
      rm -rf "${GOODFETCH_STEPS_DIR:?}"/*
    else
      GOODFETCH_STEPS_DIR=$(mktemp -d "${TMPDIR:-/tmp}/goodfetch-steps.XXXXXX")
    fi
    : >"$GOODFETCH_STEPS_DIR/results"
    export GOODFETCH_STEPS_DIR
  fi
}

# Appends a record to the run's list: `<status>\t<name>\t<detail>`.
_steps_record() {
  printf '%s\t%s\t%s\n' "$1" "$2" "$3" >>"$GOODFETCH_STEPS_DIR/results"
}

step() {
  local needs=() need name command log status
  while [[ ${1:-} == --after ]]; do
    needs+=("$2")
    shift 2
  done
  name=$1
  command=$2
  if [[ ${GOODFETCH_STEPS_LIST:-} == 1 ]]; then
    if [[ ${#needs[@]} -gt 0 ]]; then
      printf '%s  # %s, after %s\n' "$command" "$name" "${needs[*]}"
    else
      printf '%s  # %s\n' "$command" "$name"
    fi
    return 0
  fi
  for need in ${needs[@]+"${needs[@]}"}; do
    case $_steps_bad in
      *"|$need|"*)
        echo "== $name: skipped, because $need did not pass"
        _steps_record skipped "$name" "$need did not pass"
        _steps_bad="$_steps_bad$name|"
        _steps_red=1
        return 0
        ;;
    esac
  done
  log=$GOODFETCH_STEPS_DIR/$(printf '%s' "$name" | tr -c 'A-Za-z0-9._-' '-')
  if [[ -e $log.log ]]; then
    log=$log-$$-$RANDOM
  fi
  log=$log.log
  echo "== $name: $command"
  status=0
  GOODFETCH_STEP=$name bash -euo pipefail -c "$command" 2>&1 | tee "$log" || status=$?
  if [[ $status -eq 0 ]]; then
    _steps_record passed "$name" "$log"
  else
    echo "== $name: failed with status $status (log: $log)"
    _steps_record failed "$name" "$log"
    _steps_bad="$_steps_bad$name|"
    _steps_red=1
  fi
}

steps_mark() {
  printf '%s\n' "$_steps_bad"
}

steps_red_since() {
  [[ $_steps_bad != "$1" ]]
}

steps_end() {
  [[ ${GOODFETCH_STEPS_LIST:-} == 1 ]] && return 0
  if [[ -z $_steps_outer ]]; then
    # The outer run's summary names this run's failed steps, not its own
    # step that started this run.
    if [[ $_steps_red -ne 0 && -n $_steps_parent ]]; then
      _steps_record inner "$_steps_parent" ""
    fi
    return "$_steps_red"
  fi
  local results=$GOODFETCH_STEPS_DIR/results inner status name detail passed=0 failed=0 skipped=0 lines=
  inner=$(awk -F '\t' '$1 == "inner" { print $2 }' "$results")
  while IFS="$(printf '\t')" read -r status name detail; do
    case $status in
      passed) passed=$((passed + 1)) ;;
      failed | skipped)
        if printf '%s\n' "$inner" | grep -Fqx -- "$name"; then
          continue
        fi
        if [[ $status == failed ]]; then
          failed=$((failed + 1))
          lines="$lines  failed   $name: $detail"$'\n'
        else
          skipped=$((skipped + 1))
          lines="$lines  skipped  $name: $detail"$'\n'
        fi
        ;;
    esac
  done <"$results"
  echo
  echo "== $_steps_label: $passed passed, $failed failed, $skipped skipped."
  if [[ -n $lines ]]; then
    printf '%s' "$lines"
  fi
  return "$_steps_red"
}
