# Goodfetch's one entry point (ADR 0061; TESTING.md, Suites, commands and time
# budgets). CI calls the same recipes. `quick` and `pre-merge` test what
# `xtask affected` selects from the working tree's changes since the merge base
# with `origin/main`, and the whole workspace with `all`.
#
# No global `set dotenv-load`: tests never read `.env` (ADR 0076).

set shell := ["bash", "-euo", "pipefail", "-c"]

# In CI, test builds use the `ci-test` Cargo profile (ADR 0061).
test_profile := if env("CI", "") != "" { "ci-test" } else { "test" }

# Lists the recipes.
default:
    @just --list

# Formats Rust, TypeScript and Python.
fmt:
    cargo fmt --all
    pnpm exec oxfmt .
    uv run ruff format .

# Checks formatting only (the pre-commit hook).
fmt-check:
    cargo fmt --all --check
    pnpm exec oxfmt --check .
    uv run ruff format --check .

# Formatting checks, then every linter and the repository's own static checks.
lint: fmt-check
    cargo metadata --locked --format-version 1 > /dev/null
    pnpm install --lockfile-only --frozen-lockfile
    uv lock --check
    cargo clippy --workspace --all-targets --all-features -- -D warnings
    pnpm exec tsc -p tsconfig.json
    pnpm -r --if-present run typecheck
    pnpm exec oxlint
    uv run ruff check .
    uv run pyright
    uv run mypy .
    ci/tool.sh typos typos
    ci/tool.sh zizmor zizmor --config zizmor.yml --persona=regular .github/workflows .github/actions
    ci/tool.sh actionlint actionlint
    ci/tool.sh gitleaks ci/canary-rule.sh
    ci/tool.sh shellcheck shellcheck ci/*.sh ci/deny/*.sh ci/egress/*.sh ci/host/*.sh ci/release/*.sh ci/results/*.sh ci/runner/*.sh ci/test-health/*.sh ci/vm/*.sh ci/visual/*.sh ci/proofs/run.sh .githooks/pre-commit .githooks/pre-push
    ci/tool.sh cargo-deny cargo deny check -A unused-wrapper -A license-not-encountered bans licenses sources
    ci/tool.sh cargo-deny ci/deny/products.sh
    ci/tool.sh cargo-shear cargo shear
    cargo run --quiet -p xtask -- self-hosted check
    cargo run --quiet -p xtask -- egress check
    cargo run --quiet -p xtask -- os-services workflows
    cargo run --quiet -p xtask -- fixture-size
    cargo run --quiet -p xtask -- screenshots check
    cargo run --quiet -p xtask -- dependency-direction
    cargo run --quiet -p xtask -- package-dependencies
    cargo run --quiet -p xtask -- mocking-lockfile
    cargo run --quiet -p xtask -- no-retry
    GITHUB_TOKEN="${GITHUB_TOKEN:-$(gh auth token 2>/dev/null || true)}" \
      cargo run --quiet -p xtask -- quarantine check --cargo-profile {{ test_profile }}
    GITHUB_TOKEN="${GITHUB_TOKEN:-$(gh auth token 2>/dev/null || true)}" \
      cargo run --quiet -p xtask -- axe-exceptions check
    cargo run --quiet -p xtask -- modules
    cargo run --quiet -p xtask -- codegen --check
    cargo run --quiet -p xtask -- features check
    cargo run --quiet -p xtask -- clients check
    cargo run --quiet -p xtask -- compat check
    cargo run --quiet -p xtask -- wording
    cargo run --quiet -p xtask -- journeys check
    cargo run --quiet -p xtask -- containment
    ci/repository-files.sh
    ci/tool-versions.sh

# Regenerates every file generated from goodfetch-protocol's types: the JSON
# Schema bundle and catalog, each Product's projection, the names only
# Foxhound carries, deny.toml's wrappers list and each SDK's generated/ layer
# (ADR 0038). Commit what it writes.
codegen:
    pnpm install --frozen-lockfile
    uv sync --frozen
    cargo run --quiet -p xtask -- codegen

# Writes the Proving ground's task list of each seed the Journey catalog uses
# to journeys/tasks/, from the commit's Proving ground, and removes the list of
# a seed no Journey uses (ADR 0063). Commit what it writes. `--check` writes
# nothing and fails on a list that differs: the Pre-merge suite's drift check.
journeys-tasks *check:
    {{ just_executable() }} _test-tools
    cargo run --quiet -p xtask -- journeys tasks --proving-ground "$(jq -r .proving_ground target/test-tools.json)" {{ check }}

# ADR 0063's rule on the tests of each Journey variant, in the Pre-merge
# suite: lists the four runners' tests into target/journey-listings/ (nextest
# builds them with the tests' profile), then runs the catalog check with them.
# It fails on a test of an unknown or retired Journey or of an unlisted
# surface, on two tests of one variant, and notes each untested variant.
journeys-tests:
    cargo run --quiet -p xtask -- journeys listings --out target/journey-listings --cargo-profile {{ test_profile }}
    cargo run --quiet -p xtask -- journeys check --listings target/journey-listings

# ADR 0072's Feature check over every runner's declarations, in the Pre-merge
# suite: after `journeys-tests`, it reads the same listings, so the Vitest
# tags, Playwright annotations and pytest markers count beside the Rust
# `Covers:` lines and the Journeys' and conformance cases' `covers` lists. A
# generated Feature none of them declares fails it; the Quick suite's check,
# which reads no listing, only notes one.
features-tests: journeys-tests
    cargo run --quiet -p xtask -- features check --listings target/journey-listings

# Runs chosen Journey variants through `just test pre-merge` (ADRs 0063 and
# 0066): `--id J-001`, `--surface cli`, `--milestone M1` and `--area 3`, each
# repeatable; values of one flag add variants, different flags intersect.
# `--engine <path>` runs them against that executable, for both Products.
# `--tag <tag>` (with `--engine`) runs the tag's own Journey tests and helpers,
# checked out in a temporary worktree that is removed afterwards.
[positional-arguments]
journeys *arguments:
    #!/usr/bin/env bash
    set -euo pipefail
    just="{{ just_executable() }}"
    select=() engine="" tag=""
    while [[ $# -gt 0 ]]; do
      case "$1" in
        --engine) engine="${2:?--engine takes a path}"; shift 2 ;;
        --engine=*) engine="${1#--engine=}"; shift ;;
        --tag) tag="${2:?--tag takes a tag}"; select+=(--tag "$tag"); shift 2 ;;
        --tag=*) tag="${1#--tag=}"; select+=(--tag "$tag"); shift ;;
        *) select+=("$1"); shift ;;
      esac
    done
    if [[ -n "$tag" && -z "$engine" ]]; then
      echo "error: --tag runs an old tag's Journeys against a current executable: give --engine <path>" >&2
      exit 2
    fi
    scratch=$(mktemp -d)
    tree="$scratch/tree"
    cleanup() {
      if [[ -d "$tree" ]]; then git worktree remove --force "$tree"; fi
      rm -rf "$scratch"
    }
    trap cleanup EXIT
    cargo run --quiet -p xtask -- journeys select ${select[@]+"${select[@]}"} > "$scratch/selection"
    if [[ ! -s "$scratch/selection" ]]; then
      echo "No chosen variant has a runner yet."
      exit 0
    fi
    echo "Filters (<runner>, <argument>):"
    cat "$scratch/selection"
    export GOODFETCH_TEST_SELECTION="$scratch/selection"
    if [[ -n "$engine" ]]; then
      GOODFETCH_TEST_ENGINE=$(cd "$(dirname "$engine")" && pwd)/$(basename "$engine")
      [[ -x "$GOODFETCH_TEST_ENGINE" ]] || { echo "error: $engine is not an executable" >&2; exit 2; }
      export GOODFETCH_TEST_ENGINE
    fi
    if [[ -z "$tag" ]]; then
      "$just" test pre-merge
    else
      git worktree add --quiet --detach "$tree" "$tag^{commit}"
      cd "$tree"
      pnpm install --frozen-lockfile
      uv sync --frozen --all-extras
      "$just" test pre-merge
    fi

# What the changed packages declare, then the Feature check's verdict (ADR
# 0072): the Features each crate `xtask affected` selects declares, one
# `<feature> <level> <source> <test>` line each, and the check's findings over
# the workspace, which are the Features missing a test, an exception or an
# area. It reads no runner listing, so it notes rather than fails a generated
# Feature only a Vitest, Playwright or pytest test might declare; `just
# features-tests` reads those too. Takes `xtask affected`'s change, `--local`
# by default.
features *change:
    #!/usr/bin/env bash
    set -euo pipefail
    change=({{ change }})
    [[ ${#change[@]} -gt 0 ]] || change=(--local)
    selection=$(cargo run --quiet -p xtask -- affected "${change[@]}")
    read -ra crates <<< "$(sed -n 's/^crates=//p' <<< "$selection")"
    if [[ ${#crates[@]} -gt 0 ]]; then
      echo "Declared by ${crates[*]}:"
      cargo run --quiet -p xtask -- features scan "${crates[@]/#/--package=}"
    else
      echo "The change reaches no crate."
    fi
    echo "Missing:"
    cargo run --quiet -p xtask -- features check && echo "nothing"

# Each job's VM runs this when it starts, before ADR 0043's outbound-traffic
# deny: the locked dependencies of each workspace and, as areas pin them, the
# Playwright browsers, the Clean-up rules' inputs and other pinned inputs.
# These may retry: they test nothing (ADR 0064).
# Makes every download the pull-request tier needs, before the egress deny.
fetch:
    cargo fetch --locked
    pnpm install --frozen-lockfile
    uv sync --frozen --all-extras

# Builds Rust (with the given Cargo profile), TypeScript and Python.
build profile="dev":
    cargo build --workspace --all-targets --profile {{ profile }}
    pnpm -r --if-present run build
    uv build --all-packages --out-dir target/python-dist

# Runs one suite's tests without lints: `quick`, `pre-merge` or `release`.
# Packages, when given, narrow the tests: a crate's name the Rust tests
# (`-p <crate>`), a pnpm package's folder Vitest's and Playwright's, a uv
# package's folder pytest's; a runner given none of its packages is skipped.
# `just journeys` narrows them further through two variables (ADRs 0063 and
# 0066): GOODFETCH_TEST_SELECTION names a file of `<runner>\t<argument>`
# lines, `xtask journeys select`'s filters, and only the runners it names run,
# each with its filter (`pre-merge` and `release` only); GOODFETCH_TEST_ENGINE
# names an executable that replaces each Product's in the test tool manifest.
test suite *packages:
    #!/usr/bin/env bash
    set -euo pipefail
    packages=() vitest=() pytest=()
    for package in {{ packages }}; do
      if [[ -f "$package/package.json" ]]; then vitest+=("$package/")
      elif [[ -f "$package/pyproject.toml" ]]; then pytest+=("$package")
      else packages+=(-p "$package"); fi
    done
    rust=true typescript=true python=true
    if [[ -n "{{ packages }}" ]]; then
      [[ ${#packages[@]} -gt 0 ]] || rust=false
      [[ ${#vitest[@]} -gt 0 ]] || typescript=false
      [[ ${#pytest[@]} -gt 0 ]] || python=false
    else
      packages=(--workspace)
    fi
    # Every runner's tests read the suite from GOODFETCH_TEST_SUITE, and find
    # the commit's executables through the test tool manifest (ADRs 0062 and
    # 0094): those the workspace has, built here with the tests' profile.
    # Each Rust Journey variant writes its result document, `<id>@<surface>.json`,
    # into target/result-documents/ (ADR 0063).
    export GOODFETCH_TEST_SUITE="{{ suite }}"
    {{ just_executable() }} _test-tools
    export GOODFETCH_TEST_TOOLS="$PWD/target/test-tools.json"
    export GOODFETCH_TEST_RESULT_DOCUMENTS="$PWD/target/result-documents"
    if [[ -n "${GOODFETCH_TEST_ENGINE:-}" ]]; then
      manifest=$(jq --arg engine "$GOODFETCH_TEST_ENGINE" '.collie = $engine | .foxhound = $engine' target/test-tools.json)
      printf '%s\n' "$manifest" > target/test-tools.json
      echo "Engine: $GOODFETCH_TEST_ENGINE, for both Products"
    fi
    select_nextest=() select_vitest=() select_playwright=() select_pytest=()
    vitest_runs=$typescript playwright_runs=$typescript
    if [[ -n "${GOODFETCH_TEST_SELECTION:-}" ]]; then
      if [[ "{{ suite }}" == quick ]]; then
        echo "error: a selection runs in the pre-merge or release suite, where Journey tests run" >&2
        exit 2
      fi
      while IFS=$'\t' read -r runner argument; do
        case "$runner" in
          nextest) select_nextest+=("$argument") ;;
          vitest) select_vitest+=("$argument") ;;
          playwright) select_playwright+=("$argument") ;;
          pytest) select_pytest+=("$argument") ;;
        esac
      done < "$GOODFETCH_TEST_SELECTION"
      [[ ${#select_nextest[@]} -gt 0 ]] || rust=false
      [[ ${#select_vitest[@]} -gt 0 ]] || vitest_runs=false
      [[ ${#select_playwright[@]} -gt 0 ]] || playwright_runs=false
      [[ ${#select_pytest[@]} -gt 0 ]] || python=false
    fi
    # Vitest, Playwright and pytest run their tests in an order seeded from
    # the tree under test (TESTING.md, Before merge); nextest runs each test
    # in its own process.
    GOODFETCH_ORDER_SEED=$(ci/order-seed.sh)
    export GOODFETCH_ORDER_SEED
    echo "Test order seed: $GOODFETCH_ORDER_SEED (set GOODFETCH_ORDER_SEED to repeat an order)"
    # Each runner skips the tests on the test quarantine list through its own
    # filter (ADR 0064): `xtask quarantine exclude` prints `<runner>\t<argument>`.
    skip_nextest=() skip_vitest=() skip_playwright=() skip_pytest=()
    exclusions=$(cargo run --quiet -p xtask -- quarantine exclude)
    while IFS=$'\t' read -r runner argument; do
      case "$runner" in
        nextest) skip_nextest+=("$argument") ;;
        vitest) skip_vitest+=("$argument") ;;
        playwright) skip_playwright+=("$argument") ;;
        pytest) skip_pytest+=("$argument") ;;
      esac
    done <<< "$exclusions"
    # A local host without an OS service the tests use (an extra loopback
    # address, the Secret Service) lists it as skipped, and nextest skips the
    # tests that need it; in CI a missing one fails (ci/os-services.sh).
    services=$(ci/os-services.sh skipped target/skipped.txt)
    if [[ -n "$services" ]]; then
      [[ ${#skip_nextest[@]} -gt 0 ]] || skip_nextest=(--)
      while read -r pattern; do skip_nextest+=(--skip "$pattern"); done <<< "$services"
    fi
    # A selection may hold no test of a suite yet (NEXTEST_NO_TESTS=pass);
    # every test it holds still runs.
    case "{{ suite }}" in
      quick)
        # In-process tests only (nextest's quick profile); snapshots are
        # checked, never written. Unreferenced snapshots are judged only
        # when every test runs, in the Pre-merge suite.
        if $rust; then
          NEXTEST_NO_TESTS=pass INSTA_UPDATE=no cargo insta test "${packages[@]}" --check \
            --test-runner nextest --nextest-profile quick --profile {{ test_profile }} \
            --disable-nextest-doctest ${skip_nextest[@]+"${skip_nextest[@]}"}
        fi
        if $typescript; then
          pnpm exec vitest run --project unit --slowTestThreshold=1000 ${vitest[@]+"${vitest[@]}"} ${skip_vitest[@]+"${skip_vitest[@]}"}
        fi
        # pytest exits 5 when it collects no test, as before any Python test exists.
        if $python; then
          uv run pytest --timeout=10 -p randomly --randomly-seed="$GOODFETCH_ORDER_SEED" --ignore-glob='*/tests/integration/*' ${pytest[@]+"${pytest[@]}"} ${skip_pytest[@]+"${skip_pytest[@]}"} || [[ $? -eq 5 ]]
        fi
        ;;
      pre-merge|release)
        # A quarantined test that owns an insta snapshot does not run, so its
        # snapshot looks unreferenced: while nextest skips any quarantined
        # test, unreferenced snapshots warn instead of failing (TESTING.md,
        # Snapshots).
        unreferenced=reject
        if [[ ${#skip_nextest[@]} -gt 0 ]]; then
          unreferenced=warn
          echo "insta: unreferenced snapshots warn while the quarantine list skips a nextest test"
        fi
        if $rust && [[ ${#select_nextest[@]} -gt 0 ]]; then
          # `cargo insta test` hands nextest no filterset, so a selection runs
          # nextest itself: the selected tests' snapshots are still checked,
          # never written, and unreferenced ones are judged when all run.
          NEXTEST_NO_TESTS=pass INSTA_UPDATE=no cargo nextest run "${packages[@]}" --profile ci \
            --cargo-profile {{ test_profile }} "${select_nextest[@]}" ${skip_nextest[@]+"${skip_nextest[@]}"}
        elif $rust; then
          NEXTEST_NO_TESTS=pass INSTA_UPDATE=no cargo insta test "${packages[@]}" --check --unreferenced "$unreferenced" \
            --test-runner nextest --nextest-profile ci --profile {{ test_profile }} \
            --disable-nextest-doctest ${skip_nextest[@]+"${skip_nextest[@]}"}
          # nextest runs no doctests; cargo-insta's own doctest pass would take
          # nextest's profile flag, which `cargo test` refuses. `cargo test
          # --doc` also refuses a selection with no library target ("no
          # library targets found"), such as a binary-only crate alone, so it
          # gets only the selected packages that have one, and none skips it.
          doc_packages=("${packages[@]}")
          if [[ "${packages[0]}" == -p ]]; then
            libraries=$(cargo metadata --no-deps --format-version 1 \
              | jq -r '.packages[] | select(any(.targets[].kind[]; IN("lib", "rlib", "dylib", "proc-macro"))) | .name')
            doc_packages=()
            for ((i = 1; i < ${#packages[@]}; i += 2)); do
              if grep -qxF -- "${packages[i]}" <<< "$libraries"; then doc_packages+=(-p "${packages[i]}"); fi
            done
          fi
          if [[ ${#doc_packages[@]} -gt 0 ]]; then
            cargo test "${doc_packages[@]}" --doc --profile {{ test_profile }}
          else
            echo "Doctests: no selected package has a library target"
          fi
        fi
        if $vitest_runs; then
          pnpm exec vitest run ${vitest[@]+"${vitest[@]}"} ${select_vitest[@]+"${select_vitest[@]}"} ${skip_vitest[@]+"${skip_vitest[@]}"}
        fi
        if $playwright_runs; then
          pnpm exec playwright test --pass-with-no-tests --shuffle="$GOODFETCH_ORDER_SEED" ${vitest[@]+"${vitest[@]}"} ${select_playwright[@]+"${select_playwright[@]}"} ${skip_playwright[@]+"${skip_playwright[@]}"}
        fi
        if $python; then
          uv run pytest -p randomly --randomly-seed="$GOODFETCH_ORDER_SEED" ${pytest[@]+"${pytest[@]}"} ${select_pytest[@]+"${select_pytest[@]}"} ${skip_pytest[@]+"${skip_pytest[@]}"} || [[ $? -eq 5 ]]
        fi
        ;;
      *)
        echo "error: the suite is quick, pre-merge or release, not {{ suite }}" >&2
        exit 2
        ;;
    esac

# Updates snapshots only through each tool's own command, never by hand
# (ADR 0062; TESTING.md, Snapshots): insta runs the tests and reviews what
# changed (`cargo insta test --review`, which ends in `cargo insta review`),
# Vitest updates the changed pnpm packages' snapshots (`-u`), and
# inline-snapshot reviews its own single-process (`-n0`), since under xdist
# it can neither create nor fix one. `accept` writes every change without
# review and deletes unreferenced insta snapshots; inline-snapshot still fails
# each test whose snapshot it changed, so run them again. Packages, when
# given, narrow it as `just test`'s do. CI never runs it.
snapshots *arguments:
    #!/usr/bin/env bash
    set -euo pipefail
    arguments=({{ arguments }})
    mode=review
    if [[ ${#arguments[@]} -gt 0 && "${arguments[0]}" == accept ]]; then
      mode=accept
      arguments=("${arguments[@]:1}")
    fi
    packages=() vitest=() pytest=()
    for package in ${arguments[@]+"${arguments[@]}"}; do
      if [[ -f "$package/package.json" ]]; then vitest+=("$package/")
      elif [[ -f "$package/pyproject.toml" ]]; then pytest+=("$package")
      else packages+=(-p "$package"); fi
    done
    rust=true typescript=true python=true
    if [[ ${#arguments[@]} -gt 0 ]]; then
      [[ ${#packages[@]} -gt 0 ]] || rust=false
      [[ ${#vitest[@]} -gt 0 ]] || typescript=false
      [[ ${#pytest[@]} -gt 0 ]] || python=false
    else
      packages=(--workspace)
      read -ra vitest <<< "$(cargo run --quiet -p xtask -- affected --local | sed -n 's/^vitest-packages=//p')"
      [[ ${#vitest[@]} -gt 0 ]] || typescript=false
    fi
    # The tests find the commit's executables as under `just test`.
    export GOODFETCH_TEST_SUITE=pre-merge
    {{ just_executable() }} _test-tools
    export GOODFETCH_TEST_TOOLS="$PWD/target/test-tools.json"
    case "$mode" in
      review) insta=(--review --unreferenced warn) inline=review ;;
      accept) insta=(--accept --unreferenced delete) inline=create,fix,trim,update ;;
    esac
    # Every tool runs even when another's tests failed; the exit code is the
    # last failure's.
    status=0
    if $rust; then
      cargo insta test "${packages[@]}" "${insta[@]}" --test-runner nextest --nextest-profile ci \
        --profile {{ test_profile }} --disable-nextest-doctest || status=$?
    fi
    if $typescript; then
      pnpm exec vitest run -u "${vitest[@]}" || status=$?
    fi
    if $python; then
      uv run pytest -n0 --inline-snapshot="$inline" ${pytest[@]+"${pytest[@]}"} || status=$?
    fi
    exit "$status"

# `--test <runner>=<test ID>`, repeatable, or `--quarantined <n>`, the
# quarantine list's n oldest entries. The diagnostic runs (20) and the daily
# test-health job (50) call it; quarantined tests run too, as they must there.
# Every runner runs even when another failed; the exit code is the last
# failure's.
# Runs chosen tests `count` times each, whatever each run's result (ADR 0064).
[positional-arguments]
repeat count *selection:
    #!/usr/bin/env bash
    set -euo pipefail
    shift
    # The tests run as `just test` runs them: in their suite (the Pre-merge
    # suite's unless GOODFETCH_TEST_SUITE names another), with the test tool
    # manifest, and in the order seeded from the tree under test.
    export GOODFETCH_TEST_SUITE="${GOODFETCH_TEST_SUITE:-pre-merge}"
    {{ just_executable() }} _test-tools
    export GOODFETCH_TEST_TOOLS="$PWD/target/test-tools.json"
    GOODFETCH_ORDER_SEED=$(ci/order-seed.sh)
    export GOODFETCH_ORDER_SEED
    echo "Test order seed: $GOODFETCH_ORDER_SEED (set GOODFETCH_ORDER_SEED to repeat an order)"
    nextest=() vitest=() playwright=() pytest=()
    arguments=$(cargo run --quiet -p xtask -- repeat --count "{{ count }}" "$@")
    while IFS=$'\t' read -r runner argument; do
      case "$runner" in
        nextest) nextest+=("$argument") ;;
        vitest) vitest+=("$argument") ;;
        playwright) playwright+=("$argument") ;;
        pytest) pytest+=("$argument") ;;
      esac
    done <<< "$arguments"
    {{ just_executable() }} _test-tools
    export GOODFETCH_TEST_TOOLS="$PWD/target/test-tools.json"
    status=0
    if [[ ${#nextest[@]} -gt 0 ]]; then
      cargo nextest run --workspace --profile ci --cargo-profile {{ test_profile }} "${nextest[@]}" || status=$?
    fi
    if [[ ${#vitest[@]} -gt 0 ]]; then
      pnpm exec vitest run "${vitest[@]}" || status=$?
    fi
    if [[ ${#playwright[@]} -gt 0 ]]; then
      pnpm exec playwright test --shuffle="$GOODFETCH_ORDER_SEED" "${playwright[@]}" || status=$?
    fi
    if [[ ${#pytest[@]} -gt 0 ]]; then
      uv run pytest -p randomly --randomly-seed="$GOODFETCH_ORDER_SEED" "${pytest[@]}" || status=$?
    fi
    exit "$status"

# The stress run before merge (ADR 0064): each test the change since `base`
# (the merge base with `origin/main` by default) adds or changes runs 20
# times, and each test it takes off the quarantine list 200 times, through
# `just repeat`, in the order the tree under test seeds. `xtask stress select`
# chooses them; the base's listings are those the cache-warming job saved,
# else listed from a checkout of the base. Each count's JUnit files go to
# target/junit/repeat-<count>/, which the results store's ingest reads as run
# that many times. Nothing skips it and no count is lowered; it counts
# against the Pre-merge budget.
stress base="":
    #!/usr/bin/env bash
    set -euo pipefail
    base="{{ base }}"
    [[ -n "$base" ]] || base=$(git merge-base origin/main HEAD)
    target=$(cargo metadata --format-version 1 --no-deps | jq -r .target_directory)
    selection=$(cargo run --quiet -p xtask -- stress select --base "$base" \
      --saved "$target/stress-listings" --cargo-profile {{ test_profile }})
    echo "Stressed tests (<count>, <runner>, <test ID>):"
    echo "$selection"
    # The JUnit files an earlier step of the job wrote stay where they are.
    mkdir -p target/junit
    earlier=$(mktemp -d)
    find target/junit -maxdepth 1 -name '*.xml' -exec mv {} "$earlier/" \;
    status=0
    for count in 20 200; do
      tests=()
      while IFS=$'\t' read -r runs runner test; do
        if [[ "$runs" == "$count" ]]; then tests+=(--test "$runner=$test"); fi
      done <<< "$selection"
      echo "Stress run: $(( ${#tests[@]} / 2 )) test(s), $count times each"
      [[ ${#tests[@]} -gt 0 ]] || continue
      rm -f "$target"/nextest/*/junit.xml
      {{ just_executable() }} repeat "$count" "${tests[@]}" || status=$?
      folder="target/junit/repeat-$count"
      mkdir -p "$folder"
      find target/junit -maxdepth 1 -name '*.xml' -exec mv {} "$folder/" \;
      for report in "$target"/nextest/*/junit.xml; do
        [[ -f "$report" ]] || continue
        mv "$report" "$folder/nextest-$(basename "$(dirname "$report")").xml"
      done
    done
    find "$earlier" -name '*.xml' -exec mv {} target/junit/ \;
    rmdir "$earlier"
    exit "$status"

# Runs one suite's tests over what `xtask affected` selects (ADR 0061): the
# working tree, uncommitted and untracked files included, against the merge
# base with `origin/main`, or the change CI gives as `--base <rev> --head <rev>`.
test-affected suite *change:
    #!/usr/bin/env bash
    set -euo pipefail
    change=({{ change }})
    [[ ${#change[@]} -gt 0 ]] || change=(--local)
    selection=$(cargo run --quiet -p xtask -- affected "${change[@]}")
    echo "$selection"
    selected() { sed -n "s/^$1=//p" <<< "$selection"; }
    read -ra packages <<< "$(selected crates) $(selected vitest-packages) $(selected pytest-packages)"
    if [[ ${#packages[@]} -eq 0 ]]; then
      echo "The change needs no {{ suite }} tests."
      exit 0
    fi
    {{ just_executable() }} test {{ suite }} "${packages[@]}"

# Each suite lists the parts it could not run on this host, with the reason,
# even when a part fails: the list is printed when the suite stops.

# The Quick suite: lints and in-process tests of what changed (`all`: the
# whole workspace).
quick *all:
    #!/usr/bin/env bash
    set -euo pipefail
    just="{{ just_executable() }}"
    rm -f target/skipped.txt
    trap '"$just" _skips-report' EXIT
    "$just" lint
    "$just" _suite quick "{{ all }}"

# The Pre-merge suite on this host: the Quick suite, then the tests of what
# changed and the static checks' proofs (`all`: the whole workspace). The
# visual projects run only in the pinned Playwright image, in CI's `visual`
# job (ADR 0073).
#
# `perf` runs beside the other steps, in its container on Linux x86-64
# (ci/perf.sh), with its lines marked `[perf]`; the suite waits for it and
# fails when it fails, and stops it when another step fails first. The checks
# that take seconds and judge the whole change (sign-offs, commit messages,
# the SDK crates' features, advisories, the proofs) run before the suites.
pre-merge *all:
    #!/usr/bin/env bash
    set -euo pipefail
    just="{{ just_executable() }}"
    rm -f target/skipped.txt
    "$just" perf > >(awk '{ print "[perf] " $0; fflush() }') 2>&1 &
    perf=$!
    trap '[[ -z "$perf" ]] || kill "$perf" 2>/dev/null || true; "$just" _skips-report' EXIT
    "$just" dco
    "$just" commit-messages
    "$just" lint
    "$just" feature-combinations
    "$just" audit
    "$just" proofs
    "$just" _suite quick "{{ all }}"
    "$just" _suite pre-merge "{{ all }}"
    "$just" journeys-tasks --check
    "$just" features-tests
    "$just" _tlc "{{ all }}"
    "$just" msrv
    "$just" miri
    "$just" notices
    echo "Waiting for perf…"
    status=0
    wait "$perf" || status=$?
    perf=
    if [[ $status -ne 0 ]]; then
      echo "error: just perf failed with exit status $status" >&2
      exit "$status"
    fi
    "$just" _visual-elsewhere

# This host's cell of the Release suite. The other cells run in CI only:
# `just release-ci`.
release:
    #!/usr/bin/env bash
    set -euo pipefail
    just="{{ just_executable() }}"
    rm -f target/skipped.txt
    trap '"$just" _skips-report' EXIT
    "$just" lint
    "$just" test quick
    "$just" test release
    "$just" _tlc all
    "$just" proofs
    "$just" rss
    "$just" _visual-elsewhere
    echo "skipped: every other OS and runtime cell of the Release suite runs only in CI (just release-ci)"

# Dispatches the Release suite's workflow on the current branch and follows
# it; the exit code is the run's verdict (ADRs 0061 and 0108). `cells` is
# `all` or cell names separated by commas: linux-x64-gnu, linux-x64-musl,
# linux-arm64-gnu, linux-arm64-musl, windows-x64, macos-arm64, macos-x64. On
# `main`, `commit` is a commit of `main` to test, its head by default, and the
# macOS cells run on the owner's machine; off `main` the run tests the
# branch's pushed head with the hosted cells alone. On `main`, the commit's
# verdict across all its dispatches follows the run's.
release-ci cells="all" commit="":
    #!/usr/bin/env bash
    set -euo pipefail
    workflow=release-suite.yml
    branch=$(git rev-parse --abbrev-ref HEAD)
    commit="{{ commit }}"
    if [[ "$branch" != main && -n "$commit" ]]; then
      echo "error: a commit can be chosen only on main; off main the run tests the branch's pushed head" >&2
      exit 2
    fi
    git fetch --quiet origin "$branch"
    if [[ "$(git rev-parse HEAD)" != "$(git rev-parse "origin/$branch")" ]]; then
      echo "error: HEAD is not origin/$branch's head; push it first, since the run tests the pushed head" >&2
      exit 2
    fi
    repository=$(gh repo view --json nameWithOwner --jq .nameWithOwner)
    before=$(gh run list --workflow "$workflow" --branch "$branch" --event workflow_dispatch --limit 1 --json databaseId --jq '.[0].databaseId // 0')
    gh workflow run "$workflow" --ref "$branch" -f cells="{{ cells }}" -f commit="$commit"
    run=$before
    for _ in $(seq 60); do
      sleep 2
      run=$(gh run list --workflow "$workflow" --branch "$branch" --event workflow_dispatch --limit 1 --json databaseId --jq '.[0].databaseId // 0')
      [[ "$run" != "$before" ]] && break
    done
    if [[ "$run" == "$before" ]]; then
      echo "error: the dispatched run did not appear within two minutes" >&2
      exit 2
    fi
    echo "Release run $run on $branch, cells {{ cells }}."
    if [[ "$branch" == main ]]; then
      echo "The macOS cells wait for the owner's machine (owner-machine); while it is off, they stay queued and fail after 24 hours."
    else
      echo "Off main, the macOS cells do not run (ADR 0108)."
    fi
    status=0
    gh run watch "$run" --exit-status || status=$?
    if [[ "$branch" == main ]]; then
      [[ -n "$commit" ]] || commit=$(gh run view "$run" --json headSha --jq .headSha)
      echo "Commit $commit across its dispatches:"
      GITHUB_TOKEN=$(gh auth token) cargo run --quiet -p xtask -- release-suite verdict \
        --repository "$repository" --commit "$commit" && echo "Every cell is green on $commit." || true
    fi
    exit "$status"

# The resident-memory check (ADRs 0071, 0094 and 0134): builds both Products'
# executables with the `ci` profile the Release suite's cells build, runs the
# scenario (every scenario when none is named: idle-attached-engine) on each
# executable it runs on, then holds each peak to its ceiling in
# ci/rss-ceilings.toml and shows it against its Performance target. With no
# ceilings file it reports and gates nothing. The JUnit file and the figures
# the results store keeps go to target/junit/.
# Runs the resident-memory check: `just rss [scenario…]`.
rss *scenarios:
    cargo run --quiet -p xtask -- test-tools --cargo-profile ci --out "$PWD/target/rss/test-tools.json"
    rm -f target/rss/peaks.jsonl
    cargo run --quiet -p xtask -- rss measure {{ scenarios }} --tools target/rss/test-tools.json --out target/rss/peaks.jsonl
    cargo run --quiet -p xtask -- rss check --peaks target/rss/peaks.jsonl

# Proposes the resident-memory ceilings of this platform from 20 runs of each
# scenario on each executable it runs on: 1.5 times the highest peak, rounded
# up to a whole MiB, as ci/rss-ceilings.toml writes them. Only the owner
# commits a ceiling (ADR 0071), measured on the build after M1's optimization
# pass (ADR 0134).
# Proposes this platform's resident-memory ceilings from 20 runs.
rss-propose *scenarios:
    cargo run --quiet -p xtask -- test-tools --cargo-profile ci --out "$PWD/target/rss/test-tools.json"
    rm -f target/rss/propose.jsonl
    cargo run --quiet -p xtask -- rss measure {{ scenarios }} --runs 20 --tools target/rss/test-tools.json --out target/rss/propose.jsonl
    cargo run --quiet -p xtask -- rss propose --peaks target/rss/propose.jsonl

# The counted performance figures of what changed, against `base` (the merge
# base with `origin/main` by default), with CI's thresholds (ADR 0071):
# natively on Linux, in the pinned CI image through Docker or Podman
# elsewhere, and listed as skipped without either (ci/perf.sh).
perf base="":
    ci/perf.sh {{ base }}

# Keeps each tool of test-support/models/tools.toml in target/tla-tools/,
# downloading only a file that lacks its pinned SHA-256; a download whose
# SHA-256 differs fails (ADR 0060).
# Downloads the pinned pair of TLA+ tools, before the outbound-traffic deny.
tlc-fetch:
    cargo run --quiet -p xtask -- tlc fetch

# Each model of test-support/models/ runs with its small.cfg; `--config
# large` takes the nightly job's constants, and `--config <variant>` a
# model's own variant (ADR 0060). Needs Java 11 or later.
# Model-checks TLA+ models with TLC: `just tlc <model>…` or `just tlc --all`.
tlc *arguments:
    {{ just_executable() }} tlc-fetch
    cargo run --quiet -p xtask -- tlc run {{ arguments }}

# A job that passes the models is so shown able to fail.
# The TLC job's self-test: the sample's broken variant must fail its invariant.
tlc-self-test:
    {{ just_executable() }} tlc-fetch
    cargo run --quiet -p xtask -- tlc run sample --config broken --expect invariant-violated

# The cache-warming job's build (ADRs 0061 and 0071): the pull-request tier's
# dependencies, tests and benchmark targets with its profiles, run by nothing:
# the Pre-merge job's own `just build ci` among them; the benchmark targets
# twice, the second time into the target folder's perf-base/, where `xtask
# perf build` builds the base. It also saves the four runners' listings of
# the commit into the target folder's stress-listings/, where `just stress`
# reads a change's base listings (ADR 0064).
warm:
    pnpm exec tsc -p tsconfig.json
    cargo clippy --workspace --all-targets --all-features
    cargo build -p xtask
    cargo nextest run --workspace --profile ci --cargo-profile ci-test --no-run
    cargo run --quiet -p xtask -- stress listings --cargo-profile ci-test \
      --out "$(cargo metadata --format-version 1 --no-deps | jq -r .target_directory)/stress-listings"
    {{ just_executable() }} build ci
    cargo bench --workspace --no-run
    CARGO_TARGET_DIR="$(cargo metadata --format-version 1 --no-deps | jq -r .target_directory)/perf-base" cargo bench --workspace --no-run
    ci/msrv.sh

# Applies the Reference screenshots of an `update-screenshots` run of this
# branch (ADR 0073), refusing a run made in another image than the pinned one
# or on another commit than the head. Commit them with your own sign-off,
# naming the run in the message.
screenshots-fetch run:
    #!/usr/bin/env bash
    set -euo pipefail
    folder=$(mktemp -d)
    trap 'rm -rf "$folder"' EXIT
    gh run download "{{ run }}" --name reference-screenshots --dir "$folder"
    cargo run --quiet -p xtask -- screenshots fetch "$folder"

# The pull-request tier's supply-chain job (#133), each part also a recipe:
# the advisories of every lockfile, the MSRV build, cargo-hack over each
# Product's SDK crate, Miri and the notice inventory's license check. The
# audits read public advisory databases, so CI runs this before any egress
# deny. `pre-merge` runs the same parts, cheapest first.
supply-chain: audit msrv feature-combinations miri notices

# The third-party notice inventory (ADRs 0018 and 0047): fails on a
# dependency of any ecosystem, or an input a `third-party.toml` declares, whose
# license ADR 0018's allowlist refuses for its use. `--out <file>` writes the
# inventory, and `--inputs <file>` adds a file of inputs.
notices *arguments:
    cargo run --quiet -p xtask -- notices {{ arguments }}

# The security advisories of the Cargo, pnpm and uv lockfiles: cargo-deny's
# and cargo-audit's (RustSec), pnpm audit's and pip-audit's (ci/audit.sh).
audit:
    ci/tool.sh cargo-audit ci/audit.sh rust
    ci/audit.sh npm
    ci/audit.sh python

# Builds the workspace with the toolchain of its `rust-version`, the MSRV of
# ADR 0030 (latest stable minus two).
msrv:
    ci/msrv.sh

# cargo-hack over every feature combination of each Product's SDK crate
# (ADR 0087).
feature-combinations:
    ci/tool.sh cargo-hack ci/feature-combinations.sh

# Miri over the pure-Rust `unsafe` modules that opt in through ci/miri.toml
# (STYLE.md, `unsafe`); none does yet, so it runs nothing.
miri:
    #!/usr/bin/env bash
    set -euo pipefail
    read_list() { uv run --frozen --quiet python -c "import tomllib; miri = tomllib.load(open('ci/miri.toml', 'rb')); $1"; }
    modules=$(read_list 'print("\n".join(m["crate"] + "\t" + m["filter"] for m in miri["module"]))')
    if [[ -z "$modules" ]]; then
      echo "Miri: no module opts in yet (ci/miri.toml)"
      exit 0
    fi
    toolchain=$(read_list 'print(miri["toolchain"])')
    # Each module runs once; a `for` loop with no `break` retries nothing
    # (xtask no-retry).
    set -f
    IFS=$'\n'
    for module in $modules; do
      cargo "+$toolchain" miri test --package "${module%%$'\t'*}" --lib -- "${module#*$'\t'}"
    done

# Builds Miri's sysroot, which fetches the standard library's dependencies,
# so CI runs it before the egress deny; nothing while no module opts in.
miri-setup:
    #!/usr/bin/env bash
    set -euo pipefail
    opted_in=$(uv run --frozen --quiet python -c "import tomllib; miri = tomllib.load(open('ci/miri.toml', 'rb')); print(miri['toolchain'] if miri['module'] else '')")
    if [[ -n "$opted_in" ]]; then
      cargo "+$opted_in" miri setup
    fi

# Checks that every commit from `base` to `head`, merges aside, is signed off
# by its author (ADR 0018). Locally: since the merge base with `origin/main`.
dco base="" head="HEAD":
    #!/usr/bin/env bash
    set -euo pipefail
    base="{{ base }}"
    [[ -n "$base" ]] || base=$(git merge-base origin/main "{{ head }}")
    cargo run --quiet -p xtask -- dco --base "$base" --head "{{ head }}"

# The recorded starting point of the Conventional Commits rule
# (CONTRIBUTING.md): this commit, the tip of spec #130's integration branch
# when the owner adopted the rule on 2026-10-19, and its ancestors keep their
# messages, since history is never rewritten.
conventional_commits_since := "9e62a6c527577f8dc08b119bdf2acb952fcc62b1"

# Checks that every commit from `base` to `head`, merges aside, after the
# rule's starting point has a Conventional Commits subject (CONTRIBUTING.md).
# Locally: since the merge base with `origin/main`.
commit-messages base="" head="HEAD":
    #!/usr/bin/env bash
    set -euo pipefail
    base="{{ base }}"
    [[ -n "$base" ]] || base=$(git merge-base origin/main "{{ head }}")
    cargo run --quiet -p xtask -- commit-messages --base "$base" --head "{{ head }}" \
      --since "{{ conventional_commits_since }}"

# Runs the static checks against fixtures that each must fail (ADR 0062's bans
# and the snapshot rules), proving the configurations refuse what they should.
proofs:
    ci/proofs/run.sh

# Opts in to the git hooks: pre-commit checks formatting, pre-push runs `just quick`.
hooks:
    git config core.hooksPath .githooks

# Merges a pull request only when `just pre-merge all`, which starts with
# `just quick all`'s lint and Quick suite, passes on its exact head, in a
# clean checkout in the gate's own worktree (AGENTS.md, "Merging").
merge number:
    ci/merge-pr.sh {{ number }}

# Installs the owner's copy of the self-hosted allowlist and the runner's
# pre-job and post-job scripts into a folder the runner user can read but not
# write (ADRs 0108 and 0135). Run by the owner on the runner's machine.
runner-install folder:
    ci/runner/install.sh "{{ folder }}"

# Runs ADR 0135's Rosetta trial in a throwaway Lima VM: Callgrind, the pinned
# Playwright image and `cargo fuzz check` under Rosetta for Linux.
rosetta-trial:
    ci/vm/rosetta-trial.sh

# Builds the commit's executables that tests run (each Product's, the Proving
# ground's, goodfetch-test-env, goodfetch-scripted-engine and
# goodfetch-conformance, those the workspace has) with the tests' Cargo profile, and writes the test tool
# manifest of their paths to target/test-tools.json.
_test-tools:
    cargo run --quiet -p xtask -- test-tools --cargo-profile {{ test_profile }} --out "$PWD/target/test-tools.json"

# One suite's tests over the whole workspace with `all`, else over what changed.
_suite suite all:
    #!/usr/bin/env bash
    set -euo pipefail
    case "{{ all }}" in
      all) {{ just_executable() }} test {{ suite }} ;;
      "") {{ just_executable() }} test-affected {{ suite }} ;;
      *) echo "error: the only argument is \`all\`, not {{ all }}" >&2; exit 2 ;;
    esac

# TLC on the models the change reaches (`all`: every model), then the job's
# self-test, as CI's `tlc` job runs them; listed as skipped without Java.
_tlc all:
    #!/usr/bin/env bash
    set -euo pipefail
    just="{{ just_executable() }}"
    case "{{ all }}" in
      all) models=(--all) ;;
      "")
        selection=$(cargo run --quiet -p xtask -- affected --local)
        read -ra models <<< "$(sed -n 's/^models=//p' <<< "$selection")"
        if [[ ${#models[@]} -eq 0 ]]; then
          echo "The change reaches no TLA+ model."
          exit 0
        fi
        ;;
      *) echo "error: the only argument is \`all\`, not {{ all }}" >&2; exit 2 ;;
    esac
    ci/tool.sh java "$just" tlc "${models[@]}"
    ci/tool.sh java "$just" tlc-self-test

# Runner configs define the `visual` projects only inside the pinned image
# (ci/visual/run.sh), so no host test command runs them.
_visual-elsewhere:
    @mkdir -p target
    @echo "visual projects: skipped, they run only in the pinned Playwright image (CI's visual job, ADR 0073)" >>target/skipped.txt

_skips-report:
    #!/usr/bin/env bash
    if [[ -s target/skipped.txt ]]; then
      echo "Not run on this host:"
      sed 's/^/  - /' target/skipped.txt
    fi
