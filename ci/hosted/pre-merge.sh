#!/usr/bin/env bash
# The Pre-merge suite of a batch's hosted run (issues 3378 and 3380, ADR
# 0135's note of 2026-11-23), in the job's container (ci/hosted/exec.sh),
# over every member the hosted plan holds it for, those whose tests launch
# Chrome for Testing or use an OS service among them: the container boots
# systemd and ci/os-services.sh starts every service the gate host's suite
# uses. Its parts, each a job's step:
#   build <member>…  before the deny: builds the crates' tests among those
#                    members once with `cargo nextest archive`, the test
#                    tool manifest and its executables, and lists the tests
#                    the suite selects, with its skips (`cargo nextest
#                    list`), into target/pre-merge/; the other members,
#                    Vitest's, Playwright's and pytest's packages and the
#                    tests no member owns (`:unowned`), are the runners';
#   doctests         the crates' doctests, which no archive holds;
#   runners          when the run holds a member that is no crate: Vitest,
#                    Playwright and pytest, each over every test, as the
#                    gate host's `just test pre-merge --shuffled-all` runs
#                    them for a part of the suite;
#   shard <k> <n>    one hash partition of the archive (`--partition
#                    hash:<k>/<n>`), with the exercise records and the insta
#                    references its tests leave, into target/pre-merge/shards/;
#   record <n>       outside the deny, once all n shards have ended: plans the
#                    batch form again, checks that the build archived or
#                    gave the runners each member the plan runs and that the
#                    shards ran the listing's tests, each once (`xtask gate
#                    shards`), runs the exercise audit over every shard's
#                    tests, and records the step's result for each member as
#                    the gate does (`xtask gate record`), so `just merge`
#                    reuses its green units (ci/hosted-records.sh).
#
# The archive's executables keep the build's absolute paths: a shard
# extracts the archive into the container's /var/tmp, whose target/ is
# CARGO_TARGET_DIR (ci/vm/in-container.sh), so a path a test compiled in
# (`env!("CARGO_BIN_EXE_…")`) and the test tool manifest's are the build's.
#
# Each part leaves its status in target/pre-merge/ (`doctests.status`,
# `runners.status`, `shards/status-<k>`); `record` reads a missing one as a
# part that never ended, which records every member red. A red runner
# records the members that are no crate red and judges each crate by the
# shards' JUnit files, as a red nextest run is judged.
set -euo pipefail

part=${1:?usage: ci/hosted/pre-merge.sh build <member>…|doctests|runners|shard <k> <n>|record <n>}
shift
cd "$(dirname "$0")/../.."
# shellcheck source=ci/output-value.sh
source ci/output-value.sh

folder=target/pre-merge
archive=$folder/archive.tar.zst
tools=$folder/test-tools.json
mkdir -p "$folder/shards"

# The skips every suite takes, one argument a line (ci/nextest-skips.sh),
# from the quarantine's exclusions the build listed.
skips() {
  local exclusions
  exclusions=$(cat "$folder/quarantine.json")
  ci/nextest-skips.sh "$exclusions"
}
# The members the build archived, one per line.
members() { tr ' ' '\n' <"$folder/members.txt" | sed '/^$/d'; }

case $part in
  build)
    [[ $# -gt 0 ]] || {
      echo "error: build takes the members the plan holds the Pre-merge suite of" >&2
      exit 2
    }
    crates=$(cargo metadata --no-deps --format-version 1 --locked | jq -r '.packages[].name')
    archived=() others=()
    for member in "$@"; do
      if grep -qxF -- "$member" <<<"$crates"; then archived+=("$member"); else others+=("$member"); fi
    done
    printf '%s\n' "${archived[*]-}" >"$folder/members.txt"
    printf '%s\n' "${others[*]-}" >"$folder/runners.txt"
    cargo run --quiet --locked --bin xtask -- quarantine exclude >"$folder/quarantine.json"
    skips >"$folder/skips.txt"
    if [[ ${#archived[@]} -gt 0 ]]; then
      packages=()
      for member in "${archived[@]}"; do packages+=(-p "$member"); done
      skipped=()
      while IFS= read -r argument; do [[ -z $argument ]] || skipped+=("$argument"); done <"$folder/skips.txt"
      cargo nextest archive --locked "${packages[@]}" --profile ci --cargo-profile ci-test \
        --archive-file "$archive"
      cargo nextest list --locked "${packages[@]}" --profile ci --cargo-profile ci-test \
        --message-format json ${skipped[@]+"${skipped[@]}"} >"$folder/listing.json"
    else
      # No crate's tests to archive: the shards' check then reads a listing
      # that selects none.
      echo '{"rust-suites": {}}' >"$folder/listing.json"
    fi
    cargo run --quiet --locked --bin xtask -- test-tools build --cargo-profile ci-test --out "$PWD/$tools"
    # The tools at their absolute paths, which the manifest names.
    jq -r '.[]' "$tools" | tar --create --absolute-names --file "$folder/test-tools.tar" --files-from -
    ls -l "$folder/test-tools.tar"
    [[ ! -f $archive ]] || ls -l "$archive"
    df -h /var/tmp /work
    ;;
  doctests)
    # `cargo test --doc` refuses a selection with no library target, as
    # `just test` notes, so it takes the members that have one.
    libraries=$(cargo metadata --no-deps --format-version 1 --locked |
      jq -r '.packages[] | select(any(.targets[].kind[]; IN("lib", "rlib", "dylib", "proc-macro"))) | .name')
    packages=()
    while IFS= read -r member; do
      if grep -qxF -- "$member" <<<"$libraries"; then packages+=(-p "$member"); fi
    done < <(members)
    status=0
    if [[ ${#packages[@]} -gt 0 ]]; then
      cargo test --locked "${packages[@]}" --doc --profile ci-test || status=$?
    else
      echo "Doctests: no member the run holds has a library target"
    fi
    echo "$status" >"$folder/doctests.status"
    exit "$status"
    ;;
  runners)
    if [[ ! -s $folder/runners.txt || -z $(tr -d ' \n' <"$folder/runners.txt") ]]; then
      echo "Runners: every member the run holds is a crate, which the shards run"
      exit 0
    fi
    # Given no crate, `just test` runs no nextest; `--shuffled-all` runs
    # every Vitest, Playwright and pytest test in the head's order.
    export GOODFETCH_OS_SERVICES=required
    status=0
    just test pre-merge --shuffled-all || status=$?
    echo "$status" >"$folder/runners.status"
    exit "$status"
    ;;
  shard)
    shard=${1:?usage: ci/hosted/pre-merge.sh shard <k> <n>}
    count=${2:?usage: ci/hosted/pre-merge.sh shard <k> <n>}
    out=$folder/shards
    rm -f "$out/status-$shard" "$out/junit-$shard.xml" "$out/references-$shard.txt"
    # The shard skips what the listing skipped, or its tests are not the
    # listing's.
    if ! skips | cmp --silent - "$folder/skips.txt"; then
      echo "error: this shard's skips differ from those the build listed the tests with ($folder/skips.txt)" >&2
      exit 1
    fi
    skipped=()
    while IFS= read -r argument; do [[ -z $argument ]] || skipped+=("$argument"); done <"$folder/skips.txt"
    tar --extract --absolute-names --file "$folder/test-tools.tar"
    # As under `just test pre-merge`: the suite, the executables, where the
    # tests leave their result documents, case logs and exercise records,
    # and snapshots checked, never written.
    export GOODFETCH_TEST_SUITE=pre-merge GOODFETCH_TEST_TOOLS="$PWD/$tools"
    export GOODFETCH_TEST_RESULT_DOCUMENTS="$PWD/target/result-documents"
    export GOODFETCH_TEST_CASE_LOGS="$PWD/target/case-logs"
    export GOODFETCH_TEST_RECORDS="$PWD/target/test-records"
    export GOODFETCH_OS_SERVICES=required
    export NEXTEST_NO_TESTS=pass INSTA_UPDATE=no
    export INSTA_SNAPSHOT_REFERENCES_FILE="$PWD/$out/references-$shard.txt"
    rm -rf target/test-records target/nextest/ci/junit.xml
    mkdir -p target/case-logs
    : >target/case-logs/nextest.jsonl
    status=0
    cargo nextest run --archive-file "$archive" --workspace-remap . --extract-to /var/tmp \
      --extract-overwrite --profile ci --partition "hash:$shard/$count" \
      ${skipped[@]+"${skipped[@]}"} || status=$?
    [[ ! -f target/nextest/ci/junit.xml ]] || cp target/nextest/ci/junit.xml "$out/junit-$shard.xml"
    echo "$status" >"$out/status-$shard"
    exit "$status"
    ;;
  record)
    records=$(ci/gate-records.sh)
    plan=$(ci/gate-plan.sh batch hosted)
    planned=$(output_value head "$plan")
    action=$(output_value pre-merge.action "$plan")
    case $action in
      run) planned_members=$(output_value members "$plan") ;;
      part) planned_members=$(output_value pre-merge.members "$plan") ;;
      *)
        echo "error: the plan's Pre-merge suite is $action here, so no shard should have run" >&2
        exit 1
        ;;
    esac
    # Each part's verdict: `pass` once every part ended green, `tests` when
    # nextest's tests or the runners alone failed, `fail` otherwise.
    verdict=pass
    worse() { [[ $verdict == fail ]] || verdict=$1; }
    count=${1:?usage: ci/hosted/pre-merge.sh record <the count of shards>}
    junit=() references=() audit=()
    doctests=$(cat "$folder/doctests.status" 2>/dev/null || echo missing)
    if [[ $doctests != 0 ]]; then
      echo "Pre-merge: the doctests ended $doctests"
      worse fail
    fi
    for ((shard = 1; shard <= count; shard++)); do
      status=$(cat "$folder/shards/status-$shard" 2>/dev/null || echo missing)
      case $status in
        0) ;;
        100) worse tests ;;
        *)
          echo "Pre-merge: shard $shard ended $status"
          worse fail
          ;;
      esac
      file=$folder/shards/junit-$shard.xml
      if [[ -f $file ]]; then
        junit+=(--junit "$file")
        audit+=(--junit "$file")
      else
        echo "Pre-merge: shard $shard left no JUnit file"
        worse fail
      fi
      references+=(--references "$folder/shards/references-$shard.txt")
    done
    # Unreferenced snapshots are judged once every test passed, as `cargo
    # insta test --unreferenced` judges them, and only warn while the
    # quarantine list skips a test (TESTING.md, Snapshots).
    unreferenced=reject
    [[ ! -s $folder/skips.txt && $verdict == pass ]] || unreferenced=warn
    # A red runner leaves the crates' verdict to their JUnit files.
    runners=$(cat "$folder/runners.txt" 2>/dev/null || true)
    if [[ -n $(tr -d ' \n' <<<"$runners") ]]; then
      status=$(cat "$folder/runners.status" 2>/dev/null || echo missing)
      case $status in
        0) ;;
        missing)
          echo "Pre-merge: the runners never ended"
          worse fail
          ;;
        *)
          echo "Pre-merge: the runners ended $status"
          worse tests
          ;;
      esac
    fi
    # The build's listing is there once it ended; without it, it is red.
    if [[ -f $folder/listing.json ]]; then
      cargo run --quiet --locked --bin xtask -- gate shards --listing "$folder/listing.json" \
        --archived "$(cat "$folder/members.txt")" --runners "$runners" --members "$planned_members" \
        ${junit[@]+"${junit[@]}"} ${references[@]+"${references[@]}"} --unreferenced "$unreferenced" || worse fail
    else
      echo "Pre-merge: the build left no listing"
      worse fail
    fi
    if [[ ${#audit[@]} -gt 0 ]]; then
      cargo run --quiet --locked --bin xtask -- features audit --records target/test-records \
        "${audit[@]}" || worse fail
    fi
    evidence=()
    case $verdict in
      pass) status=pass ;;
      tests)
        status=fail
        evidence=(${junit[@]+"${junit[@]}"})
        ;;
      *) status=fail ;;
    esac
    cargo run --quiet --locked --bin xtask -- gate record --records "$records" --by hosted \
      --head "$planned" --now "$(date +%s)" --step pre-merge --status "$status" \
      ${evidence[@]+"${evidence[@]}"}
    echo "Pre-merge: recorded $status for $planned_members"
    [[ $status == pass ]]
    ;;
  *)
    echo "error: the part is build, doctests, runners, shard or record, not $part" >&2
    exit 2
    ;;
esac
