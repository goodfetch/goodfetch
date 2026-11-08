# Goodfetch's one entry point (ADR 0061; TESTING.md, Suites, commands and time
# budgets). CI calls the same recipes. `quick` and `pre-merge` test what
# `xtask affected` selects from the working tree's changes since the merge base
# with `origin/main`, and the whole workspace with `all`.
#
# No global `set dotenv-load`: tests never read `.env` (ADR 0076).
#
# xtask runs as `cargo run --bin xtask`, never `-p xtask`: named alone, its
# package resolves its dependencies' features by itself, so the engine,
# Page and Collie it links compiled once more for it than for the test
# tools; without `-p` they resolve as the workspace's, as the test tools'
# build resolves them, and the two share those crates.

set shell := ["bash", "-euo", "pipefail", "-c"]

# On Windows, a bare `bash` resolves to WSL's `bash.exe` in System32, which
# Windows searches before PATH, so recipes run in Git for Windows' Bash, as
# GitHub's `shell: bash` steps do. Shebang recipes reach it through Git
# Bash's `cygpath` and `/usr/bin/env`.
set windows-shell := ["C:/Program Files/Git/bin/bash.exe", "-euo", "pipefail", "-c"]

# nextest starts each test straight from the runner, not through a stub copy
# of `cargo-nextest` that then executes the test (its double-spawn): on macOS
# under load, a test that starts no process at all kept its output pipes open
# past the leak timeout through the stub and failed as leaky (TESTING.md,
# Before merge). On Linux it makes no difference. The stub only guards
# against a hang on Ctrl-Z.
export NEXTEST_DOUBLE_SPAWN := "0"

# In CI, test builds use the `ci-test` Cargo profile (ADR 0061).
test_profile := if env("CI", "") != "" { "ci-test" } else { "test" }

# The exercise audit's verdict in the Pre-merge and Release suites (ADR 0072):
# `gate`, so a declaring test that never reached its Feature fails them;
# `report` lists the findings and fails nothing, for a local look
# (`just features-audit ran report`).
exercise_audit := "gate"

# Lists the recipes.
default:
    @just --list

# Formats Rust, TypeScript and Python.
fmt:
    cargo fmt --all
    cd kennel && cargo fmt
    pnpm exec oxfmt .
    uv run ruff format .

# Checks formatting only (the pre-commit hook). Each check runs even after
# one failed, and the run ends with the list of those that failed, each with
# its log under target/steps/ (ci/keep-going.sh), as in `lint-documents`
# and `lint`.
fmt-check:
    #!/usr/bin/env bash
    set -euo pipefail
    source ci/keep-going.sh
    steps_begin fmt-check
    step cargo-fmt 'cargo fmt --all --check'
    step kennel-fmt 'cd kennel && cargo fmt --check'
    step oxfmt 'pnpm exec oxfmt --check .'
    step ruff-format 'uv run ruff format --check .'
    steps_end

# The checks of `just lint` that read documents: formatting, typos, the
# Quality register, the Feature and Journey checks, the wording and the
# repository's policy files. The docs gate runs them alone for a change of
# documents (`xtask affected`'s `gate=docs`, ADR 0135). Every check runs
# even after one failed; the `xtask` checks come after xtask's build, and are
# skipped, named so, when it fails.
lint-documents:
    #!/usr/bin/env bash
    set -euo pipefail
    source ci/keep-going.sh
    steps_begin lint-documents
    step fmt-check '{{ just_executable() }} fmt-check'
    step typos 'ci/tool.sh typos typos'
    step xtask 'cargo build --quiet --bin xtask'
    step --after xtask xtask-register 'cargo run --quiet --bin xtask -- register check'
    step --after xtask xtask-features 'cargo run --quiet --bin xtask -- features check'
    step --after xtask xtask-wording 'cargo run --quiet --bin xtask -- wording'
    step --after xtask xtask-journeys 'cargo run --quiet --bin xtask -- journeys check'
    step repository-files 'ci/repository-files.sh'
    steps_end

# Formatting checks and the checks that read documents, then every linter and
# the repository's own static checks. Every check runs even after one failed,
# and the run ends with the list of those that failed, each with its log
# under target/steps/lint/ (ci/keep-going.sh). A check that needs an earlier
# one's output comes after it and is skipped, named so, when it failed:
# Clippy after the lockfile check, the TypeScript checks after the Console's
# build, the `xtask` checks after xtask's build.
lint:
    #!/usr/bin/env bash
    set -euo pipefail
    source ci/keep-going.sh
    steps_begin lint
    step lint-documents '{{ just_executable() }} lint-documents'
    step ignored-sources 'ci/ignored-sources.sh'
    step cargo-lock 'cargo metadata --locked --format-version 1 > /dev/null'
    step kennel-lock 'cd kennel && cargo metadata --locked --offline --format-version 1 > /dev/null'
    step fuzz-lock 'ci/fuzz-check.sh locked'
    step pnpm-lock 'pnpm install --lockfile-only --frozen-lockfile'
    step uv-lock 'uv lock --check'
    step xtask 'cargo build --quiet --bin xtask'
    step --after xtask console-build '{{ just_executable() }} console-build'
    step --after cargo-lock clippy 'cargo clippy --workspace --all-targets --all-features -- -D warnings'
    step --after console-build tsc 'pnpm exec tsc -p tsconfig.json'
    step --after console-build pnpm-typecheck 'pnpm -r --if-present run typecheck'
    step console-messages 'ci/console-messages.sh'
    step oxlint 'pnpm exec oxlint'
    step ruff 'uv run ruff check .'
    step pyright 'uv run pyright'
    step mypy 'uv run mypy .'
    step zizmor 'ci/tool.sh zizmor zizmor --config zizmor.yml --persona=regular .github/workflows .github/actions'
    step actionlint 'ci/tool.sh actionlint actionlint'
    step gitleaks 'ci/tool.sh gitleaks ci/canary-rule.sh'
    step shellcheck 'ci/tool.sh shellcheck shellcheck ci/*.sh ci/deny/*.sh ci/egress/*.sh ci/host/*.sh ci/musl/*.sh ci/musl/cargo ci/release/*.sh ci/results/*.sh ci/runner/*.sh ci/test-health/*.sh ci/vm/*.sh ci/visual/*.sh ci/proofs/run.sh ci/proofs/live.sh kennel/scripts/*.sh .githooks/pre-commit .githooks/pre-push'
    step cargo-deny 'ci/tool.sh cargo-deny cargo deny check -A unused-wrapper -A license-not-encountered bans licenses sources'
    step cargo-deny-products 'ci/tool.sh cargo-deny ci/deny/products.sh'
    # Spider's side of `just compare`, no workspace member, which no
    # gate builds: its lock's licenses, its `chrome` feature's included,
    # read without a build (ADR 0134).
    step cargo-deny-spider 'ci/tool.sh cargo-deny cargo deny --all-features --manifest-path xtask/compare-spider/Cargo.toml --config deny.toml check -A license-not-encountered licenses'
    # Crawlee's and Scrapy's sides, no workspace member either: their locks'
    # licenses against the same allowlist, read from what their frozen
    # installs leave beside them, with each side's own exceptions (ADR 0138).
    step compare-crawlee-install 'pnpm install --frozen-lockfile --reporter=silent --dir xtask/compare-crawlee'
    step compare-scrapy-install 'uv sync --frozen --quiet --directory xtask/compare-scrapy'
    step --after xtask --after compare-crawlee-install --after compare-scrapy-install xtask-notices 'cargo run --quiet --bin xtask -- notices --side xtask/compare-crawlee --side xtask/compare-scrapy'
    # The Kennel's workspace (ADR 0141), read without a build: its lock's
    # licenses against the root allowlist, its bans and sources with its own
    # deny.toml, and its manifest's lints, `rust-version` and shared
    # dependency versions against the root's.
    step cargo-deny-kennel-licenses 'ci/tool.sh cargo-deny cargo deny --manifest-path kennel/Cargo.toml --config deny.toml check -A license-not-encountered licenses'
    step cargo-deny-kennel 'ci/tool.sh cargo-deny cargo deny --manifest-path kennel/Cargo.toml --config kennel/deny.toml check bans sources'
    step --after xtask xtask-kennel-manifest 'cargo run --quiet --bin xtask -- kennel-manifest'
    step cargo-shear 'ci/tool.sh cargo-shear cargo shear'
    # The workspace-hack is current and every member takes it (ADR 0094).
    step workspace-hack 'ci/tool.sh cargo-hakari ci/workspace-hack.sh check'
    step --after xtask xtask-self-hosted 'cargo run --quiet --bin xtask -- self-hosted check'
    step --after xtask xtask-egress 'cargo run --quiet --bin xtask -- egress check'
    step --after xtask xtask-os-services 'cargo run --quiet --bin xtask -- os-services workflows'
    step --after xtask xtask-fixture-size 'cargo run --quiet --bin xtask -- fixture-size'
    step --after xtask xtask-image-pins 'cargo run --quiet --bin xtask -- image-pins'
    step --after xtask xtask-output-scans 'cargo run --quiet --bin xtask -- output-scans'
    step --after xtask xtask-screenshots 'cargo run --quiet --bin xtask -- screenshots check'
    step --after xtask xtask-dependency-direction 'cargo run --quiet --bin xtask -- dependency-direction'
    step --after xtask xtask-package-dependencies 'cargo run --quiet --bin xtask -- package-dependencies'
    step --after xtask xtask-mocking-lockfile 'cargo run --quiet --bin xtask -- mocking-lockfile'
    step --after xtask xtask-telemetry 'cargo run --quiet --bin xtask -- telemetry'
    step --after xtask xtask-no-retry 'cargo run --quiet --bin xtask -- no-retry'
    step --after xtask xtask-received-requests 'cargo run --quiet --bin xtask -- received-requests'
    step --after xtask xtask-quarantine 'GITHUB_TOKEN="${GITHUB_TOKEN:-$(gh auth token 2>/dev/null || true)}" cargo run --quiet --bin xtask -- quarantine check --cargo-profile {{ test_profile }}'
    step --after xtask xtask-axe-exceptions 'GITHUB_TOKEN="${GITHUB_TOKEN:-$(gh auth token 2>/dev/null || true)}" cargo run --quiet --bin xtask -- axe-exceptions check'
    step --after xtask xtask-modules 'cargo run --quiet --bin xtask -- modules'
    step --after xtask xtask-codegen 'cargo run --quiet --bin xtask -- codegen --check'
    step --after xtask xtask-clients 'cargo run --quiet --bin xtask -- clients check'
    step --after xtask xtask-compat 'cargo run --quiet --bin xtask -- compat check'
    step --after xtask xtask-containment 'cargo run --quiet --bin xtask -- containment'
    step tool-versions 'ci/tool-versions.sh'
    steps_end

# Regenerates every file generated from goodfetch-protocol's types: the JSON
# Schema bundle and catalog, each Product's projection, the names only
# Foxhound carries, deny.toml's wrappers list and each SDK's generated/ layer
# (ADR 0038). Commit what it writes.
codegen:
    pnpm install --frozen-lockfile
    uv sync --frozen
    cargo run --quiet --bin xtask -- codegen

# Writes the Proving ground's task list of each seed the Journey catalog uses
# to journeys/tasks/, from the commit's Proving ground, and removes the list of
# a seed no Journey uses (ADR 0063). Commit what it writes. `--check` writes
# nothing and fails on a list that differs: the Pre-merge suite's drift check.
journeys-tasks *check:
    {{ just_executable() }} _test-tools
    cargo run --quiet --bin xtask -- journeys tasks --proving-ground "$(jq -r .proving_ground target/test-tools.json)" {{ check }}

# ADR 0063's rule on the tests of each Journey variant, in the Pre-merge
# suite: lists the four runners' tests into target/journey-listings/ (nextest
# builds them with the tests' profile), then runs the catalog check with them.
# It fails on a test of an unknown or retired Journey or of an unlisted
# surface, on two tests of one variant, and notes each untested variant.
journeys-tests:
    cargo run --quiet --bin xtask -- journeys listings --out target/journey-listings --cargo-profile {{ test_profile }}
    cargo run --quiet --bin xtask -- journeys check --listings target/journey-listings

# ADR 0072's Feature check over every runner's declarations, in the Pre-merge
# suite: after `journeys-tests`, it reads the same listings, so the Vitest
# tags, Playwright annotations and pytest markers count beside the Rust
# `Covers:` lines and the Journeys' and conformance cases' `covers` lists. A
# generated Feature none of them declares fails it; the Quick suite's check,
# which reads no listing, only notes one.
features-tests: journeys-tests
    cargo run --quiet --bin xtask -- features check --listings target/journey-listings

# ADR 0069's Client conformance case-list check with the runners' listings,
# in the Pre-merge suite: after `journeys-tests`, it reads Vitest's listing,
# so a case listing `sdk-typescript` fails without a Vitest test of the
# TypeScript SDK packages tagged `client_case/<id>` in each of its modes; the
# Quick suite's `xtask clients check`, which reads no listing, only notes it.
clients-tests: journeys-tests
    cargo run --quiet --bin xtask -- clients check --listings target/journey-listings

# ADR 0072's exercise audit, after `just test pre-merge` or `release`: each
# Rust test that declares an `operation:`, `error:`, `event:`, `command:` or
# `exit:` Feature must have reached it, as the exercise record test support
# wrote for it shows. It reads the tests the run's JUnit file shows ran,
# which both suites run (the Release suite runs every test but the
# quarantined ones and those its host cannot run), or, for a local look,
# every declaring test with `all`, where a test that did not run is
# `record-missing`. The suites run it with the verdict `exercise_audit`
# gives; `gate` fails on a finding, `report` lists the findings and passes.
# Runs the exercise audit: `just features-audit [ran|all] [gate|report]`.
features-audit scope="ran" verdict=exercise_audit:
    #!/usr/bin/env bash
    set -euo pipefail
    case "{{ verdict }}" in
      gate | report) ;;
      *) echo "error: the verdict is gate or report, not {{ verdict }}" >&2; exit 2 ;;
    esac
    case "{{ scope }}" in
      ran)
        # nextest writes its JUnit file under the workspace's own
        # target/nextest/, even when CARGO_TARGET_DIR (CI's VM) puts the
        # build elsewhere.
        junit=target/nextest/ci/junit.xml
        if [[ ! -f "$junit" ]]; then
          if [[ -n "$(ls -A target/test-records 2>/dev/null)" ]]; then
            echo "error: target/test-records holds exercise records, so tests ran, but no $junit names them: the audit cannot tell which tests to judge" >&2
            exit 1
          fi
          echo "The exercise audit has nothing to read: no nextest run of the suite left $junit."
          exit 0
        fi
        audit=(--junit "$junit")
        ;;
      all) audit=() ;;
      *) echo "error: the scope is ran or all, not {{ scope }}" >&2; exit 2 ;;
    esac
    status=0
    cargo run --quiet --bin xtask -- features audit ${audit[@]+"${audit[@]}"} || status=$?
    if [[ $status -eq 1 && "{{ verdict }}" == report ]]; then
      echo "The exercise audit reports its findings above and fails nothing: the verdict is report, not the suites' gate."
      exit 0
    fi
    exit "$status"

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
    # The working tree's listings name each chosen variant no test carries,
    # and fail a choice none of whose variants has one; a tag's tests are
    # its own, so a `--tag` run lists none.
    listings=()
    if [[ -z "$tag" ]]; then
      cargo run --quiet --bin xtask -- journeys listings --out "$scratch/listings" \
        --cargo-profile {{ test_profile }}
      listings=(--listings "$scratch/listings")
    fi
    cargo run --quiet --bin xtask -- journeys select ${select[@]+"${select[@]}"} \
      ${listings[@]+"${listings[@]}"} > "$scratch/selection"
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

# What the changed packages declare and miss (ADR 0072): the Features each
# crate, npm package and Python package `xtask affected` selects declares,
# one `<feature> <level> <source> <test>` line each, then the Feature check's
# findings for those packages: the Features their tests declare that the
# inventory lacks, and the generated Features their sources define with no
# test, exception or area, with any finding on the `features/` files. It
# reads no runner listing, so it notes rather than fails a generated Feature
# only a Vitest, Playwright or pytest test might declare; `just
# features-tests` reads those too. Takes `xtask affected`'s change, `--local`
# by default.
features *change:
    #!/usr/bin/env bash
    set -euo pipefail
    change=({{ change }})
    [[ ${#change[@]} -gt 0 ]] || change=(--local)
    selection=$(cargo run --quiet --bin xtask -- affected "${change[@]}")
    source ci/output-value.sh
    crates=$(output_value crates "$selection")
    vitest=$(output_value vitest-packages "$selection")
    pytest=$(output_value pytest-packages "$selection")
    read -ra packages <<< "$crates $vitest $pytest"
    if [[ ${#packages[@]} -eq 0 ]]; then
      echo "The change reaches no package."
      exit 0
    fi
    echo "Declared by ${packages[*]}:"
    cargo run --quiet --bin xtask -- features scan "${packages[@]/#/--package=}"
    echo "Missing:"
    status=0
    missing=$(cargo run --quiet --bin xtask -- features check "${packages[@]/#/--package=}") || status=$?
    if [[ -n "$missing" ]]; then echo "$missing"; else echo "nothing"; fi
    exit "$status"

# ADR 0072's code coverage, measured and never gated (ci/coverage.sh): the
# Pre-merge suite's tests of what changed, or of the whole workspace with
# `all`, with every Rust crate built for coverage (so each SDK, Journey and
# Console run starts the instrumented executables), Vitest's `v8` provider
# and coverage.py with branches. Reports go to target/coverage/, written even
# when a test fails or a profile is unreadable. Takes `xtask affected`'s
# change, `--local` by default; listed as skipped without cargo-llvm-cov.
# Measures code coverage, never gated: `just coverage [all | <change>…]`.
coverage *change:
    JUST="{{ just_executable() }}" ci/tool.sh cargo-llvm-cov ci/coverage.sh {{ change }}

# ADR 0072's in-diff mutation testing (ci/mutants.sh): cargo-mutants with
# nextest over the Rust code changed since `base` (the merge base with
# `origin/main` by default), every mutant at once; CI's `mutants` job runs
# the same in 15-minute shards. The report goes to target/mutants/; a
# surviving mutant never fails it. Listed as skipped without cargo-mutants.
# Mutation-tests the Rust changes since `base`: `just mutants [base]`.
mutants base="":
    #!/usr/bin/env bash
    set -euo pipefail
    base="{{ base }}"
    [[ -n "$base" ]] || base=$(git merge-base origin/main HEAD)
    JUST="{{ just_executable() }}" ci/tool.sh cargo-mutants ci/mutants.sh run "$base"

# Each job's VM runs this when it starts, before ADR 0043's outbound-traffic
# deny: the locked dependencies of each workspace and, as areas pin them, the
# Playwright browsers, the Clean-up rules' inputs and other pinned inputs.
# These may retry: they test nothing (ADR 0064).
# Makes every download the pull-request tier needs, before the egress deny.
fetch:
    cargo fetch --locked
    cargo fetch --locked --manifest-path xtask/compare-spider/Cargo.toml
    pnpm install --frozen-lockfile --dir xtask/compare-crawlee
    uv sync --frozen --directory xtask/compare-scrapy
    cd kennel && cargo fetch --locked
    ci/fuzz-check.sh fetch
    pnpm install --frozen-lockfile
    uv sync --frozen --all-extras
    ci/lightpanda-fetch.sh

# Collie's Console build, which Collie's executable embeds (ADR 0057):
# brotli files only, in crates/goodfetch-collie/console-build/; nothing runs
# when its inputs did not change. Every recipe that builds the executable
# runs it first, and a `cargo build` without it fails naming it.
console-build:
    cargo run --quiet --bin xtask -- console build

# Builds Rust (with the given Cargo profile), TypeScript and Python.
build profile="dev":
    {{ just_executable() }} console-build
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
    packages=() vitest=() pytest=() vitest_files=() playwright_files=()
    # `--shuffled-all` (the incremental gate, ADR 0135): Vitest, Playwright
    # and pytest run every test, as over the whole workspace, so their order
    # is the full run's; the crates given narrow nextest alone.
    shuffled_all=false
    for package in {{ packages }}; do
      if [[ "$package" == --shuffled-all ]]; then shuffled_all=true
      elif [[ -f "$package/package.json" ]]; then vitest+=("$package/")
      elif [[ -f "$package/pyproject.toml" ]]; then pytest+=("$package")
      # A test file that no package owns, as the incremental gate passes
      # them (ADR 0135), runs in its own runner only.
      elif [[ -f "$package" && "$package" == *.spec.ts ]]; then playwright_files+=("$package")
      elif [[ -f "$package" && "$package" == *.py ]]; then pytest+=("$package")
      elif [[ -f "$package" ]]; then vitest_files+=("$package")
      else packages+=(-p "$package"); fi
    done
    rust=true typescript=true python=true vitest_given=true playwright_given=true
    if [[ -n "{{ packages }}" ]]; then
      [[ ${#packages[@]} -gt 0 ]] || rust=false
      [[ ${#vitest[@]} -gt 0 || ${#vitest_files[@]} -gt 0 ]] || vitest_given=false
      [[ ${#vitest[@]} -gt 0 || ${#playwright_files[@]} -gt 0 ]] || playwright_given=false
      $vitest_given || $playwright_given || typescript=false
      [[ ${#pytest[@]} -gt 0 ]] || python=false
    else
      packages=(--workspace)
    fi
    if $shuffled_all; then
      vitest=() pytest=() vitest_files=() playwright_files=()
      typescript=true python=true vitest_given=true playwright_given=true
    fi
    # Vitest takes the package folders and its own files; Playwright the
    # folders and its own files.
    vitest_folders=(${vitest[@]+"${vitest[@]}"})
    vitest+=(${vitest_files[@]+"${vitest_files[@]}"})
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
    vitest_runs=$vitest_given playwright_runs=$playwright_given
    $typescript || vitest_runs=false playwright_runs=false
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
    # The arguments that order each runner's tests by that seed, one source
    # for every recipe (ci/order-arguments.sh, which xtask's stress tests run).
    mapfile -t vitest_order < <(ci/order-arguments.sh vitest)
    mapfile -t playwright_order < <(ci/order-arguments.sh playwright)
    mapfile -t pytest_order < <(ci/order-arguments.sh pytest)
    # Each runner skips the tests on the test quarantine list through its own
    # filter (ADR 0064): `xtask quarantine exclude` prints each runner's
    # arguments as JSON. nextest's, which also skip the tests of an OS service
    # a local host lacks, come from ci/nextest-skips.sh, as for ci/mutants.sh
    # and ci/asan.sh.
    skip_nextest=() skip_vitest=() skip_playwright=() skip_pytest=()
    exclusions=$(cargo run --quiet --bin xtask -- quarantine exclude)
    for runner in vitest playwright pytest; do
      arguments=$(jq -r --arg runner "$runner" '.[$runner][]' <<< "$exclusions")
      while IFS= read -r argument; do
        [[ -n "$argument" ]] || continue
        case "$runner" in
          vitest) skip_vitest+=("$argument") ;;
          playwright) skip_playwright+=("$argument") ;;
          pytest) skip_pytest+=("$argument") ;;
        esac
      done <<< "$arguments"
    done
    skips=$(ci/nextest-skips.sh "$exclusions")
    while IFS= read -r argument; do [[ -z "$argument" ]] || skip_nextest+=("$argument"); done <<< "$skips"
    # A selection may hold no test of a suite yet (NEXTEST_NO_TESTS=pass);
    # every test it holds still runs.
    case "{{ suite }}" in
      quick)
        # In-process tests only (nextest's quick profile); snapshots are
        # checked, never written. Unreferenced snapshots are judged only
        # when every test runs, in the Pre-merge suite. Only the targets
        # those tests are in are built (ci/quick-targets.sh), no `tests/`
        # binary or example, which the profile never runs, and those of
        # every workspace member: the
        # build then resolves features as the workspace's whatever changed,
        # as `xtask compat check`'s build does, and the two share their
        # crates. ci/quick-nextest.sh then runs the selected members' tests
        # alone, through a nextest filterset.
        if $rust; then
          names=()
          [[ ${packages[0]} == --workspace ]] || for ((i = 1; i < ${#packages[@]}; i += 2)); do names+=("${packages[i]}"); done
          targets=$(ci/quick-targets.sh --workspace)
          mapfile -t quick_targets <<< "$targets"
          GOODFETCH_QUICK_PACKAGES="${names[*]-}" INSTA_CARGO_NEXTEST_BIN="$PWD/ci/quick-nextest.sh" \
            NEXTEST_NO_TESTS=pass INSTA_UPDATE=no cargo insta test --workspace "${quick_targets[@]}" --check \
            --test-runner nextest --nextest-profile quick --profile {{ test_profile }} \
            --disable-nextest-doctest ${skip_nextest[@]+"${skip_nextest[@]}"}
        fi
        if $vitest_runs; then
          pnpm exec vitest run --project unit --slowTestThreshold=1000 "${vitest_order[@]}" ${vitest[@]+"${vitest[@]}"} ${skip_vitest[@]+"${skip_vitest[@]}"}
        fi
        # pytest exits 5 when it collects no test, as before any Python test exists.
        if $python; then
          uv run pytest --timeout=10 "${pytest_order[@]}" --ignore-glob='*/tests/integration/*' ${pytest[@]+"${pytest[@]}"} ${skip_pytest[@]+"${skip_pytest[@]}"} || [[ $? -eq 5 ]]
        fi
        ;;
      pre-merge|release)
        # Each test nextest runs leaves its exercise record here, which
        # `just features-audit` reads (ADR 0072) with the run's JUnit file;
        # a run starts with neither.
        {{ just_executable() }} _clear-exercise-records
        export GOODFETCH_TEST_RECORDS="$PWD/target/test-records"
        # A quarantined test that owns an insta snapshot does not run, so its
        # snapshot looks unreferenced: while nextest skips any quarantined
        # test, unreferenced snapshots warn instead of failing (TESTING.md,
        # Snapshots).
        unreferenced=reject
        if [[ ${#skip_nextest[@]} -gt 0 ]]; then
          unreferenced=warn
          echo "insta: unreferenced snapshots warn while the quarantine list skips a nextest test"
        fi
        # A red nextest run still runs the doctests: the suite then fails
        # with 100, nextest's status for failed tests, when they passed, so
        # the incremental gate can judge each crate by the run's JUnit file
        # (ADR 0135; the test-gate audit's option 4b). Vitest, Playwright and
        # pytest do not run after it, unless GOODFETCH_TEST_KEEP_GOING is set,
        # as `just coverage` sets it so one red test still leaves a figure
        # (ADR 0072): then a failed runner is named and the next one still
        # runs, and the suite fails with the first failure's status once all
        # have run. Without it, a failed doctest, Vitest, Playwright or pytest
        # run ends the suite with its own status.
        failed_runners=() first_failure=0
        runner_failed() {
          [[ -n "${GOODFETCH_TEST_KEEP_GOING:-}" ]] || exit "$2"
          echo "$1 failed with status $2; the other runners still run (GOODFETCH_TEST_KEEP_GOING)" >&2
          failed_runners+=("$1")
          [[ $first_failure -ne 0 ]] || first_failure=$2
        }
        nextest=0
        if $rust && [[ ${#select_nextest[@]} -gt 0 ]]; then
          # `cargo insta test` hands nextest no filterset, so a selection runs
          # nextest itself: the selected tests' snapshots are still checked,
          # never written, and unreferenced ones are judged when all run.
          NEXTEST_NO_TESTS=pass INSTA_UPDATE=no cargo nextest run "${packages[@]}" --profile ci \
            --cargo-profile {{ test_profile }} "${select_nextest[@]}" ${skip_nextest[@]+"${skip_nextest[@]}"} || nextest=$?
        elif $rust; then
          NEXTEST_NO_TESTS=pass INSTA_UPDATE=no cargo insta test "${packages[@]}" --check --unreferenced "$unreferenced" \
            --test-runner nextest --nextest-profile ci --profile {{ test_profile }} \
            --disable-nextest-doctest ${skip_nextest[@]+"${skip_nextest[@]}"} || nextest=$?
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
            cargo test "${doc_packages[@]}" --doc --profile {{ test_profile }} || runner_failed doctests $?
          else
            echo "Doctests: no selected package has a library target"
          fi
        fi
        if [[ $nextest -ne 0 ]]; then
          if [[ -z "${GOODFETCH_TEST_KEEP_GOING:-}" ]]; then
            echo "nextest failed with status $nextest; Vitest, Playwright and pytest do not run after it" >&2
            exit 100
          fi
          runner_failed nextest "$nextest"
        fi
        if $vitest_runs; then
          pnpm exec vitest run "${vitest_order[@]}" ${vitest[@]+"${vitest[@]}"} ${select_vitest[@]+"${select_vitest[@]}"} ${skip_vitest[@]+"${skip_vitest[@]}"} \
            || runner_failed Vitest $?
        fi
        if $playwright_runs; then
          # The harness reads each `conformance` document the run wrote (ADR 0068).
          written=$(mktemp)
          pnpm exec playwright test --pass-with-no-tests "${playwright_order[@]}" ${vitest_folders[@]+"${vitest_folders[@]}"} ${playwright_files[@]+"${playwright_files[@]}"} ${select_playwright[@]+"${select_playwright[@]}"} ${skip_playwright[@]+"${skip_playwright[@]}"} \
            || runner_failed Playwright $?
          ci/read-conformance-documents.sh "$written"
          rm -f "$written"
        fi
        if $python; then
          uv run pytest "${pytest_order[@]}" ${pytest[@]+"${pytest[@]}"} ${select_pytest[@]+"${select_pytest[@]}"} ${skip_pytest[@]+"${skip_pytest[@]}"} \
            || { code=$?; [[ $code -eq 5 ]] || runner_failed pytest "$code"; }
        fi
        if [[ ${#failed_runners[@]} -gt 0 ]]; then
          printf -v named '%s, ' "${failed_runners[@]}"
          echo "Failed runners: ${named%, }" >&2
          exit "$first_failure"
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
      selection=$(cargo run --quiet --bin xtask -- affected --local)
      source ci/output-value.sh
      selected=$(output_value vitest-packages "$selection")
      read -ra vitest <<< "$selected"
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

# `--test <runner>=<test ID>`, repeatable, `--tests-file <file>` (one
# `<runner>=<test ID>` a line, as the stress run hands its selection), or
# `--quarantined <n>`, the quarantine list's n oldest entries. The diagnostic
# runs (20) and the daily test-health job (50) call it; quarantined tests run
# too, as they must there. A runner whose arguments would pass the operating
# system's limits runs in batches, each with the same counts and its part of
# the tests (`xtask repeat`); a batch's JUnit file then goes to
# `target/junit/<runner>.<batch>.xml` (nextest's `nextest-ci.<batch>.xml`), so
# none overwrites another. Every runner and batch runs even when another
# failed; the exit code is the last failure's.
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
    # `--until-red`, first (the stress run's): the run stops at the end of
    # its first red iteration (ADR 0064's amendment of 2026-10-28). nextest
    # runs its iterations one a command, each of its batches once, and after
    # a red one no other iteration or runner runs.
    iterations=1
    if [[ "${1:-}" == --until-red ]]; then iterations={{ count }}; fi
    {{ just_executable() }} _test-tools
    export GOODFETCH_TEST_TOOLS="$PWD/target/test-tools.json"
    GOODFETCH_ORDER_SEED=$(ci/order-seed.sh)
    export GOODFETCH_ORDER_SEED
    echo "Test order seed: $GOODFETCH_ORDER_SEED (set GOODFETCH_ORDER_SEED to repeat an order)"
    # The arguments that order each runner's tests by that seed, one source
    # for every recipe (ci/order-arguments.sh, which xtask's stress tests run).
    mapfile -t vitest_order < <(ci/order-arguments.sh vitest)
    mapfile -t playwright_order < <(ci/order-arguments.sh playwright)
    mapfile -t pytest_order < <(ci/order-arguments.sh pytest)
    # Each runner's commands, as JSON: `{"<runner>": [[<argument>, …], …]}`,
    # one list of arguments a command, a batch of its tests.
    batched=$(cargo run --quiet --bin xtask -- repeat --count "{{ count }}" "$@")
    {{ just_executable() }} _test-tools
    export GOODFETCH_TEST_TOOLS="$PWD/target/test-tools.json"
    status=0
    for runner in nextest vitest playwright pytest; do
      if (( iterations > 1 && status != 0 )); then
        echo "Repeat run: $runner does not run after a red iteration"
        continue
      fi
      batches=$(jq --arg runner "$runner" '.[$runner] | length' <<< "$batched")
      commands=1
      [[ "$runner" != nextest ]] || commands=$iterations
      # Each iteration after the first runs only while the run is green.
      for (( iteration = 1; iteration <= commands && (iteration == 1 || status == 0); iteration++ )); do
        for (( batch = 1; batch <= batches; batch++ )); do
          selected=()
          arguments=$(jq -r --arg runner "$runner" --argjson batch "$batch" '.[$runner][$batch - 1][]' <<< "$batched")
          while IFS= read -r argument; do [[ -z "$argument" ]] || selected+=("$argument"); done <<< "$arguments"
          case "$runner" in
            nextest)
              cargo nextest run --workspace --profile ci --cargo-profile {{ test_profile }} "${selected[@]}" || status=$?
              report=target/nextest/ci/junit.xml name=nextest-ci ;;
            vitest)
              pnpm exec vitest run "${vitest_order[@]}" "${selected[@]}" || status=$?
              report=target/junit/vitest.xml name=vitest ;;
            playwright)
              pnpm exec playwright test "${playwright_order[@]}" "${selected[@]}" || status=$?
              report=target/junit/playwright.xml name=playwright ;;
            pytest)
              uv run pytest "${pytest_order[@]}" "${selected[@]}" || status=$?
              report=target/junit/pytest.xml name=pytest ;;
          esac
          if (( commands > 1 )) && [[ -f "$report" ]]; then
            mkdir -p target/junit
            mv "$report" "target/junit/$name.$batch.$iteration.xml"
          elif (( batches > 1 )) && [[ -f "$report" ]]; then
            mkdir -p target/junit
            mv "$report" "target/junit/$name.$batch.xml"
          fi
        done
      done
      if (( commands > 1 && status != 0 )); then
        echo "Repeat run: stopped at the end of red iteration $(( iteration - 1 )) of $commands"
      fi
    done
    exit "$status"

# The stress run before merge (ADR 0064): each test the change since `base`
# (the merge base with `origin/main` by default) adds or changes runs 20
# times, and each test it takes off the quarantine list 200 times, through
# `just repeat`, in the order the tree under test seeds. `xtask stress select`
# chooses them; the base's listings are those the cache-warming job saved,
# else listed from a checkout of the base. Each count's JUnit files go to
# target/junit/repeat-<count>/, which the results store's ingest reads as run
# that many times. Nothing skips it and no count is lowered; it counts
# against the Pre-merge budget. A red run stops at the end of its first red
# iteration (`just repeat --until-red`): one failure has failed it already.
#
# The incremental gate (ADR 0135) narrows it: with `within` workspace members (crates
# by name, pnpm and uv packages by folder), only their new and changed
# tests run, plus each test whose file changed `since` the earlier gate
# run's head.
# Stresses new and changed tests: `just stress [base] [since] [within…]`.
[positional-arguments]
stress base="" since="" *within:
    #!/usr/bin/env bash
    set -euo pipefail
    shift 2
    base="{{ base }}"
    [[ -n "$base" ]] || base=$(git merge-base origin/main HEAD)
    narrowing=()
    for member in "$@"; do narrowing+=(--within "$member"); done
    [[ -z "{{ since }}" ]] || narrowing+=(--since "{{ since }}")
    target=$(cargo metadata --format-version 1 --no-deps | jq -r .target_directory)
    # Vitest lists a file's tests by loading it, as a run does, so the
    # listings get the variables `just repeat` runs the tests with.
    export GOODFETCH_TEST_SUITE="${GOODFETCH_TEST_SUITE:-pre-merge}"
    {{ just_executable() }} _test-tools
    export GOODFETCH_TEST_TOOLS="$PWD/target/test-tools.json"
    selection=$(cargo run --quiet --bin xtask -- stress select --base "$base" \
      --saved "$target/stress-listings" --cargo-profile {{ test_profile }} \
      ${narrowing[@]+"${narrowing[@]}"})
    echo "Stressed tests (<count>, <runner>, <test ID>):"
    echo "$selection"
    # The JUnit files an earlier step of the job wrote stay where they are.
    mkdir -p target/junit
    # What a red run's crates are judged by in the gate's record (ADR 0135).
    printf '%s\n' "$selection" > target/stress-selection.tsv
    earlier=$(mktemp -d)
    find target/junit -maxdepth 1 -name '*.xml' -exec mv {} "$earlier/" \;
    status=0
    # Each count's tests go to `just repeat` in a file: a change's new and
    # changed tests can pass what one command line holds.
    tests=$(mktemp)
    # Each count `xtask stress select` gives a test, smallest first.
    source ci/output-value.sh
    listed=$(cargo run --quiet --bin xtask -- stress counts)
    counts=$(output_value counts "$listed")
    for count in $counts; do
      # A red run stops at the end of its first red iteration: no later count runs.
      if (( status != 0 )); then
        echo "Stress run: the $count count does not run after a red iteration"
        continue
      fi
      while IFS=$'\t' read -r runs runner test; do
        if [[ "$runs" == "$count" ]]; then printf '%s=%s\n' "$runner" "$test"; fi
      done <<< "$selection" > "$tests"
      echo "Stress run: $(wc -l < "$tests" | tr -d ' ') test(s), $count times each"
      [[ -s "$tests" ]] || continue
      rm -f target/nextest/*/junit.xml
      {{ just_executable() }} repeat "$count" --until-red --tests-file "$tests" || status=$?
      folder="target/junit/repeat-$count"
      mkdir -p "$folder"
      find target/junit -maxdepth 1 -name '*.xml' -exec mv {} "$folder/" \;
      for report in target/nextest/*/junit.xml; do
        [[ -f "$report" ]] || continue
        mv "$report" "$folder/nextest-$(basename "$(dirname "$report")").xml"
      done
    done
    find "$earlier" -name '*.xml' -exec mv {} target/junit/ \;
    rmdir "$earlier"
    rm -f "$tests"
    exit "$status"

# Runs one suite's tests over what `xtask affected` selects (ADR 0061): the
# working tree, uncommitted and untracked files included, against the merge
# base with `origin/main`, or the change CI gives as `--base <rev> --head <rev>`.
test-affected suite *change:
    #!/usr/bin/env bash
    set -euo pipefail
    change=({{ change }})
    [[ ${#change[@]} -gt 0 ]] || change=(--local)
    selection=$(cargo run --quiet --bin xtask -- affected "${change[@]}")
    echo "$selection"
    source ci/output-value.sh
    crates=$(output_value crates "$selection")
    vitest=$(output_value vitest-packages "$selection")
    pytest=$(output_value pytest-packages "$selection")
    read -ra packages <<< "$crates $vitest $pytest"
    if [[ ${#packages[@]} -eq 0 ]]; then
      echo "The change needs no {{ suite }} tests."
      # So `just features-audit` reads no earlier run's records.
      [[ "{{ suite }}" == quick ]] || {{ just_executable() }} _clear-exercise-records
      exit 0
    fi
    {{ just_executable() }} test {{ suite }} "${packages[@]}"

# Each suite lists the parts it could not run on this host, with the reason,
# even when a part fails: the list is printed when the suite stops.

# The pre-push tier (T1): the floor guard and the scan of the change's
# commits for secrets, which take seconds, then the checks the change's own
# gate runs at merge (ci/quick.sh): for a change of documents alone
# (`xtask affected`'s `gate=docs`) the checks that read documents, else
# lints and in-process tests of what changed against the change's base
# (`all`: every lint and the whole workspace). The pre-push hook runs it,
# and skips a tree it already passed on.
quick *all:
    PATH="$(dirname "{{ just_executable() }}"):$PATH" ci/quick.sh {{ all }}

# Prints the commands `just quick` runs, in its order (ci/quick.sh), the
# lints read from their recipes, so an implementer can run by name the ones
# its change reaches before its one final `just quick` (TESTING.md, Commands).
quick-steps:
    #!/usr/bin/env bash
    set -euo pipefail
    just="{{ just_executable() }}"
    echo "# Every step runs even after one failed; the run ends with the list of those that failed (ci/keep-going.sh)."
    echo "# <base>: GOODFETCH_QUICK_BASE, else the branch's upstream off main, else origin/main."
    echo "# First, always:"
    echo "just compiler-cache"
    echo "just floor-guard <base>"
    echo "just commit-leaks <base>"
    echo "# The change's gate, its gate= line:"
    echo "cargo run --quiet --locked --bin xtask -- affected --local --since <base>"
    echo "# gate=docs: just lint-documents, and nothing after it."
    echo "# Any other gate: just lint, then the Quick suite, which runs after a red lint too."
    for recipe in fmt-check lint-documents lint; do
      echo "# just $recipe:"
      GOODFETCH_STEPS_LIST=1 "$just" "$recipe"
    done
    echo "# The Quick suite over what changed:"
    echo "just test-affected quick --local --since <base>"

# The Pre-merge suite on this host: the Quick suite, then the tests of what
# changed, the stress run of new and changed tests and the static checks'
# proofs (`all`: the whole workspace). The checks that need Linux run beside
# the other steps, each in its own container, with their lines marked:
# `perf`, `linux` (the tests only Linux decides) and `visual` (the visual
# projects in the pinned Playwright image, ADR 0073). The suite waits for
# them and fails when one fails (ci/pre-merge.sh). `quick` runs only the
# checks every change needs, for a change no test reads (ADR 0135). `all`
# on a head descending from a head whose gate ran reruns only the steps and
# members' tests the change since then reaches, with every whole-change
# check, and reuses the other green results (ADR 0135, `xtask gate`).
pre-merge *all:
    PATH="$(dirname "{{ just_executable() }}"):$PATH" ci/pre-merge.sh {{ all }}

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
    "$just" features-audit
    "$just" _tlc all
    "$just" _kennel all
    "$just" proofs
    "$just" rss
    "$just" asan
    "$just" v8-corruption
    "$just" zap
    "$just" _visual-elsewhere
    echo "skipped: every other OS and runtime cell of the Release suite runs only in CI (just release-ci)"

# Dispatches the Release suite's workflow on the current branch and follows
# it; the exit code is the run's verdict (ADRs 0061 and 0108). `cells` is
# `all` or cell names separated by commas: linux-x64-gnu, linux-x64-musl,
# linux-arm64-gnu, linux-arm64-musl, windows-x64, macos-arm64, macos-x64. On
# `main`, `commit` is a commit of `main` to test, its head by default, and the
# macOS cells run on the owner's machine; off `main` the run tests the
# branch's pushed head with the hosted cells alone. The commit's verdict
# across all its dispatches follows the run's, naming each cell not green on
# it, the skipped ones included.
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
    # A green run may hold skipped cells (the macOS ones off `main`, those not
    # asked for): the verdict names every cell not green on the commit, so
    # none passes unnoticed.
    [[ -n "$commit" ]] || commit=$(gh run view "$run" --json headSha --jq .headSha)
    echo "Commit $commit across its dispatches:"
    GITHUB_TOKEN=$(gh auth token) cargo run --quiet --bin xtask -- release-suite verdict \
      --repository "$repository" --commit "$commit" && echo "Every cell is green on $commit." || true
    exit "$status"

# The resident-memory check (ADRs 0071, 0094 and 0134): builds both Products'
# executables with the `ci` profile the Release suite's cells build, for the
# Rust target they ship for here (the static musl one on Linux, ADR 0024),
# runs the scenario (every scenario when none is named: idle-attached-engine,
# idle-daemon and crawl, the last on Foxhound against a Proving ground it
# serves) on each executable it runs on, then holds each peak to its ceiling in
# ci/rss-ceilings.toml and shows it against its Performance target. With no
# ceilings file it reports and gates nothing. The JUnit file and the figures
# the results store keeps go to target/junit/.
# Runs the resident-memory check: `just rss [scenario…]`.
rss *scenarios:
    {{ if os() == "linux" { "ci/musl/install.sh" } else { "true" } }}
    cargo run --quiet --bin xtask -- test-tools --cargo-profile ci --shipped-target --out "$PWD/target/rss/test-tools.json"
    rm -f target/rss/peaks.jsonl
    cargo run --quiet --bin xtask -- rss measure {{ scenarios }} --tools target/rss/test-tools.json --out target/rss/peaks.jsonl
    cargo run --quiet --bin xtask -- rss check --peaks target/rss/peaks.jsonl

# Proposes the resident-memory ceilings of this platform from 20 runs of each
# scenario on each executable it runs on, built as `just rss` builds them:
# 1.5 times the highest peak, rounded up to a whole MiB, as
# ci/rss-ceilings.toml writes them. Only the owner
# commits a ceiling (ADR 0071), measured on the build after M1's optimization
# pass (ADR 0134).
# Proposes this platform's resident-memory ceilings from 20 runs.
rss-propose *scenarios:
    {{ if os() == "linux" { "ci/musl/install.sh" } else { "true" } }}
    cargo run --quiet --bin xtask -- test-tools --cargo-profile ci --shipped-target --out "$PWD/target/rss/test-tools.json"
    rm -f target/rss/propose.jsonl
    cargo run --quiet --bin xtask -- rss measure {{ scenarios }} --runs 20 --tools target/rss/test-tools.json --out target/rss/propose.jsonl
    cargo run --quiet --bin xtask -- rss propose --peaks target/rss/propose.jsonl

# The comparison matrix (ADRs 0134 and 0138; the owner's comments of
# 2026-10-30 on #1577), on one machine with nothing else running: each
# scenario sets every tool to its closest equivalent configuration and the
# report writes that mapping, a cell saying so where a tool cannot do the
# scenario and marking pending each Foxhound value that does not exist yet.
# Five scenarios: 1, raw; 2, durable (killed once half the site is served,
# then resumed); 3, polite, on the Proving ground's 50-domain site; 4,
# extraction of each page's title and main text to the tool's dataset; 5,
# headless browser, on its script-rendered site. Foxhound against Spider
# 2.53.9 (xtask/compare-spider, no workspace member: its HTTP mode, and its
# `chrome` feature's executable for scenario 5), both built with the
# release profile (`profile` changes it for a quicker try), Crawlee 3.18.2
# on Node.js (xtask/compare-crawlee: `CheerioCrawler`, and
# `PlaywrightCrawler` for scenario 5) and Scrapy 2.19.0 on CPython
# (xtask/compare-scrapy, with scrapy-playwright for scenario 5), each side
# installed from its own lock, outside the Products'. Scenario 5's tools
# all run the Chrome for Testing build Foxhound pins, as Playwright
# installed it (`pnpm exec playwright install chromium`). A cell a tool
# cannot run says why: 10 trials of each measured cell in random order,
# each tool in a fresh process against a fresh Proving ground at 16
# requests at once over the scenario's 1,000-page site.
# Every metric is measured from outside the tools (the OS's accounting of
# the waited-for children, /proc samples of the process tree, the Proving
# ground's observations, the data folder left), then each cell's system
# calls and syncs are counted under strace, and scenario 1's instructions
# per page under Callgrind (`foxhound.spider_instruction_ratio`, ADR
# 0140); `--growth` adds the 100,000-page crawls whose resident memory is
# read at 1,000, 10,000 and 100,000 pages (hours). Once every trial passed
# the same-work check, it writes each cell's median and spread of every
# metric to target/compare/report.md and report.json, `local-controlled`
# and never published (ADR 0076); otherwise it fails naming the trial. A
# host without strace or Valgrind reports that cell as not run. It gates
# nothing and is no Live check; run it when the 1-minute load is under 4.
# Spider's crate and the sides' packages are downloaded once, free.
# Runs the comparison matrix on this machine.
compare profile="release" *args:
    cargo run --quiet --bin xtask -- test-tools --cargo-profile {{ profile }} --out "$PWD/target/compare/test-tools.json"
    cargo build --quiet --locked --manifest-path xtask/compare-spider/Cargo.toml --profile {{ profile }} --target-dir target/compare-spider/spider
    cargo build --quiet --locked --manifest-path xtask/compare-spider/Cargo.toml --profile {{ profile }} --features chrome --target-dir target/compare-spider/spider-chrome
    pnpm install --frozen-lockfile --reporter=silent --dir xtask/compare-crawlee
    uv sync --frozen --quiet --directory xtask/compare-scrapy
    cargo run --quiet --bin xtask -- compare run --tools target/compare/test-tools.json --spider target/compare-spider/spider/{{ profile }}/goodfetch-compare-spider --spider-chrome target/compare-spider/spider-chrome/{{ profile }}/goodfetch-compare-spider {{ args }}

# The Kennel (ADR 0141), from the repository. On macOS: its release
# executable in `Kennel.app`, signed ad hoc under the stable bundle
# identifier `dev.goodfetch.kennel` (a rebuild keeps the desktop-notice
# permission), installed by rename into ~/Applications and opened with
# `open`, through LaunchServices; a Kennel already running is quit first.
# On Linux and Windows, its executable runs (kennel/scripts/install.sh).
# Builds, installs and opens the Kennel.
kennel:
    kennel/scripts/install.sh

# The Kennel's checks (ADR 0141): its workspace's Clippy and tests, the
# Client's against goodfetch-scripted-engine and the Run controls' against a
# real Daemon of goodfetch-collie, which the root workspace builds with the
# tests' profile. On macOS every crate; elsewhere the
# non-graphical crate, with the graphical one type-checked for macOS and,
# on Linux, for the host, each listed as skipped without its Rust target or
# GTK 3's and libdbus's development files (kennel/scripts/check.sh).
# Runs the Kennel's Clippy and tests.
kennel-check:
    kennel/scripts/check.sh {{ test_profile }}

# The Kennel's part of a release (ADRs 0039, 0094 and 0141), on macOS, in
# `folder`: its `.app` built under its own version for this Mac, signed
# with Developer ID and notarized where the release environment's
# credentials are set and left ad hoc, the step inert, without them, then
# archived with the install script that installs it from this folder
# (kennel/scripts/release.sh build, sign and package). It publishes
# nothing; `bash <folder>/install-kennel.sh` installs this build.
# Builds and packages the Kennel's release files.
kennel-release folder="kennel/target/release-files":
    #!/usr/bin/env bash
    set -euo pipefail
    mkdir -p "{{ folder }}"
    folder=$(cd "{{ folder }}" && pwd)
    rm -rf "$folder"/*-apple-darwin "$folder"/goodfetch-kennel-*.zip "$folder/install-kennel.sh"
    kennel/scripts/release.sh build "$folder"
    kennel/scripts/release.sh sign "$folder"
    kennel/scripts/release.sh package "$folder" "file://$folder"

# The static musl build's toolchain (ADR 0024's note of 2026-11-06): Zig and
# cargo-zigbuild at ci/musl/toolchain.sh's versions, Zig checked against its
# SHA-256, installed once outside the checkout (ci/musl/install.sh). Linux
# only; `just targets`, `just rss` and the Release suite's musl cells build
# through it (ci/musl/cargo).
# Installs the pinned static musl toolchain.
musl-toolchain:
    ci/musl/install.sh

# The Performance targets report (ADR 0134), on the owner's machine before
# each owner test: each Product's release build for the Rust target it ships
# for here (the static musl one on Linux, ADR 0024), its startup to `hello`
# with hyperfine (10 warm-up runs, then 100; median), and its idle Attached
# engine's, idle Daemon's and (Foxhound's) 1,000-page crawl's peaks (median
# of 5 runs each; on Linux, beside each, how much is the executable's own
# file-backed pages), with the pages per second of the latest
# `just compare` report's scenario 1. Writes target/targets/report.md: each
# target, its figure, met or missed with its gap and the optimization
# pass's write-up from ci/performance-target-gaps.toml. Labelled
# `local-controlled`, never published (ADR 0076); it fails on no miss.
# Writes the Performance targets report on this machine.
targets:
    {{ if os() == "linux" { "ci/musl/install.sh" } else { "true" } }}
    cargo run --quiet --bin xtask -- test-tools --cargo-profile release --shipped-target --out "$PWD/target/targets/test-tools.json"
    rm -f target/targets/peaks.jsonl target/targets/startup-*.json
    cargo run --quiet --bin xtask -- targets startup --tools target/targets/test-tools.json --out target/targets
    cargo run --quiet --bin xtask -- rss measure idle-attached-engine idle-daemon crawl --runs 5 --tools target/targets/test-tools.json --out target/targets/peaks.jsonl
    cargo run --quiet --bin xtask -- targets report --measured target/targets

# The Counted work of each row of the Quality register on its benchmark,
# against its limit (ADR 0140; TESTING.md, Quality register); `--prove` plants
# one extra unit of each row's work and fails on a counter that misses it.
counts *args:
    cargo run --quiet --bin xtask -- counts {{ args }}

# The counted performance figures of what changed, against `base` (the merge
# base with `origin/main` by default), with CI's thresholds (ADR 0071):
# natively on Linux, in the pinned CI image through Docker or Podman
# elsewhere, and listed as skipped without either (ci/perf.sh).
perf base="":
    ci/perf.sh {{ base }}

# The Console's lab metrics for the Performance series (ADRs 0071 and 0076):
# Lighthouse 13 against each Product's Console, served by its `ci`-profile
# executable, Collie's then Foxhound's, the median of 5 runs each, appended
# to target/perf-series/console-lighthouse.jsonl with the Product and the
# machine it ran on. On the owner's machine, set
# GOODFETCH_PERF_MACHINE to its label; it times, so no suite runs it.
console-lighthouse:
    {{ just_executable() }} console-build
    cargo run --quiet --bin xtask -- test-tools --cargo-profile ci --out "$PWD/target/perf-series/test-tools.json"
    GOODFETCH_TEST_TOOLS="$PWD/target/perf-series/test-tools.json" pnpm exec playwright test --config console/collie/perf/playwright.config.ts

# The Pre-merge suite's tests that only Linux decides (`tc netem`, the
# Secret Service, the IPv6 Happy Eyeballs check, the extra loopback
# addresses): on a host that is not Linux, in the pinned CI image through
# Docker or Podman, with every Linux OS service; listed as skipped without
# either (ci/linux.sh).
linux:
    ci/linux.sh

# The visual projects in the pinned Playwright image (ADR 0073), through
# Docker or Podman, comparing every Reference screenshot; a host with
# neither fails it (ci/visual/container.sh).
visual:
    ci/visual/container.sh test

# While GitHub Actions is off, stands in for the `update-screenshots`
# workflow (ADR 0073's note of 2026-10-20): rewrites the changed or missing
# Reference screenshots in the pinned Playwright image, through Docker or
# Podman, and copies them into the working tree. Look at each, then commit
# them in a commit of their own, with your own sign-off.
screenshots-update:
    ci/visual/container.sh update

# Keeps each tool of test-support/models/tools.toml in target/tla-tools/,
# downloading only a file that lacks its pinned SHA-256; a download whose
# SHA-256 differs fails (ADR 0060).
# The tools are a release asset of this repository, read with GITHUB_TOKEN
# or the GitHub CLI's token while the repository is private.
# Downloads the pinned pair of TLA+ tools, before the outbound-traffic deny.
tlc-fetch:
    GITHUB_TOKEN="${GITHUB_TOKEN:-$(gh auth token 2>/dev/null || true)}" \
      cargo run --quiet --bin xtask -- tlc fetch

# Each model of test-support/models/ runs with its small.cfg; `--config
# large` takes the nightly job's constants, and `--config <variant>` a
# model's own variant (ADR 0060). Needs Java 11 or later.
# Model-checks TLA+ models with TLC: `just tlc <model>…` or `just tlc --all`.
tlc *arguments:
    {{ just_executable() }} tlc-fetch
    cargo run --quiet --bin xtask -- tlc run {{ arguments }}

# A job that passes the models is so shown able to fail.
# The TLC job's self-test: the sample's broken variant must fail its invariant.
tlc-self-test:
    {{ just_executable() }} tlc-fetch
    cargo run --quiet --bin xtask -- tlc run sample --config broken --expect invariant-violated

# Keeps each archive of test-support/lazyfs/sources.toml in
# target/lazyfs/sources/, downloading only a file that lacks its pinned
# SHA-256; a download whose SHA-256 differs fails (ADR 0060).
# Downloads LazyFS's pinned source archives, before the outbound-traffic deny.
lazyfs-fetch:
    cargo run --quiet --bin xtask -- lazyfs fetch

# Linux, with cmake, a C++17 compiler and libfuse3's headers
# (ci/host/lazyfs.sh installs them on a runner); it builds nothing again
# while target/lazyfs/bin/ holds a build of the same pins.
# Builds LazyFS from its pinned source archives into target/lazyfs/bin/.
lazyfs-build:
    {{ just_executable() }} lazyfs-fetch
    cargo run --quiet --bin xtask -- lazyfs build

# The Release suite runs them on Linux x64 (ADR 0060), and nextest's
# profiles leave them out of every other run; they need /dev/fuse and
# fusermount3, and fail, never skip, where a mount cannot be made.
# Runs the LazyFS power cuts (`lazyfs::` tests) on the LazyFS build.
lazyfs-test:
    #!/usr/bin/env bash
    set -euo pipefail
    {{ just_executable() }} lazyfs-fetch
    {{ just_executable() }} console-build
    GOODFETCH_LAZYFS=$(cargo run --quiet --bin xtask -- lazyfs build)
    export GOODFETCH_LAZYFS GOODFETCH_TEST_SUITE=release
    cargo nextest run -p goodfetch-test-support -p goodfetch-engine -p goodfetch-collie --profile ci \
      --cargo-profile {{ test_profile }} --ignore-default-filter \
      -E 'kind(test) & test(/^lazyfs::/)'

# The cache-warming job's build (ADRs 0061 and 0071): the pull-request tier's
# dependencies, tests and benchmark targets with its profiles, run by nothing:
# the Pre-merge job's own `just build ci` among them; the benchmark targets
# twice, the second time into the target folder's perf-base/, where `xtask
# perf build` builds the base. It also saves the four runners' listings of
# the commit into the target folder's stress-listings/, where `just stress`
# reads a change's base listings (ADR 0064).
warm:
    pnpm exec tsc -p tsconfig.json
    {{ just_executable() }} console-build
    cargo clippy --workspace --all-targets --all-features
    cargo build --bin xtask
    cargo nextest run --workspace --profile ci --cargo-profile ci-test --no-run
    {{ just_executable() }} _test-tools
    GOODFETCH_TEST_SUITE=pre-merge GOODFETCH_TEST_TOOLS="$PWD/target/test-tools.json" \
      cargo run --quiet --bin xtask -- stress listings --cargo-profile ci-test \
      --out "$(cargo metadata --format-version 1 --no-deps | jq -r .target_directory)/stress-listings"
    {{ just_executable() }} build ci
    cargo bench --workspace --no-run
    CARGO_TARGET_DIR="$(cargo metadata --format-version 1 --no-deps | jq -r .target_directory)/perf-base" cargo bench --workspace --no-run
    ci/msrv.sh

# The measurements before a browser joins the `visual` projects (ADR 0073),
# through Docker or Podman, on a copy of HEAD: the host's emulation of the
# pinned image, its pixels against the committed References (CI's), then
# every Playwright visual case 20 times on each browser (chromium, firefox
# and webkit by default, or those named, separated by commas). Record what
# it prints in the shared visual config's REPEATABILITY.
# Measures screenshot repeatability per browser in the pinned image.
visual-repeatability browsers="chromium,firefox,webkit":
    ci/visual/repeatability.sh "{{ browsers }}"

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
    cargo run --quiet --bin xtask -- screenshots fetch "$folder"

# The pull-request tier's supply-chain job (#133), each part also a recipe:
# the advisories of every lockfile, the MSRV build, cargo-hack over each
# Product's SDK crate, Miri and the notice inventory's license check, and the
# security parts it carries (ADR 0067): `gitleaks` over the change's commits
# and `cargo fuzz check`. The audit downloads its pinned advisory snapshot
# on its first run, so CI runs this before any egress deny. `pre-merge` runs
# the same parts, cheapest first.
supply-chain: audit commit-leaks fuzz-check msrv feature-combinations miri notices

# Scans paths a job uploads (its logs, test reports, snapshots and
# artifacts) for secrets with gitleaks' rules and the Secret canaries' rule
# (ADR 0067). CI runs `ci/leaks.sh` itself before each upload.
leaks +paths:
    ci/tool.sh gitleaks ci/leaks.sh {{ paths }}

# Scans every commit from `base` to `head` for secrets, while the repository
# is private (ADR 0067). Locally: since the merge base with `origin/main`.
commit-leaks base="" head="HEAD":
    #!/usr/bin/env bash
    set -euo pipefail
    base="{{ base }}"
    [[ -n "$base" ]] || base=$(git merge-base origin/main "{{ head }}")
    ci/tool.sh gitleaks ci/commit-leaks.sh "$base" "{{ head }}"

# `cargo fuzz check` of every fuzz target on the pinned nightly of
# ci/sanitizer-toolchain.toml (ADR 0067); no fuzzing runs. A no-op while no
# fuzz crate exists.
fuzz-check:
    ci/tool.sh cargo-fuzz ci/fuzz-check.sh check

# The Release suite's AddressSanitizer run of the tests of every crate that
# opts into `unsafe` (ADR 0067), on the pinned nightly; CI runs it on Linux
# glibc. `build` builds them (before the deny in CI), `test` builds and runs
# them with the test tool manifest.
asan phase="test":
    #!/usr/bin/env bash
    set -euo pipefail
    case "{{ phase }}" in
      build) ci/asan.sh build ;;
      test)
        export GOODFETCH_TEST_SUITE=release
        {{ just_executable() }} _test-tools
        export GOODFETCH_TEST_TOOLS="$PWD/target/test-tools.json"
        ci/asan.sh test
        ;;
      *) echo "error: the phase is build or test, not {{ phase }}" >&2; exit 2 ;;
    esac

# The Release suite's V8 memory-corruption scripts on the test V8 build
# (ADR 0067), a slot until that build exists (#210): `fetch` or `test`.
v8-corruption phase="test":
    ci/v8-corruption.sh {{ phase }}

# The Release suite's ZAP baseline scan of the Console (ADR 0067), a slot
# until the Console is built: `fetch` or `scan`.
zap phase="scan":
    ci/zap.sh {{ phase }}

# The third-party notice inventory (ADRs 0018 and 0047): fails on a
# dependency of any ecosystem, or an input a `third-party.toml` declares, whose
# license ADR 0018's allowlist refuses for its use. `--out <file>` writes the
# inventory, and `--inputs <file>` adds a file of inputs.
notices *arguments:
    cargo run --quiet --bin xtask -- notices {{ arguments }}

# The security advisories of the Cargo, pnpm and uv lockfiles (the
# workspace's, and the sides of `just compare`: Spider's xtask/compare-spider,
# Crawlee's xtask/compare-crawlee and Scrapy's xtask/compare-scrapy), against
# the advisory snapshot ci/advisories.toml pins (ADR 0064), offline:
# cargo-deny's and cargo-audit's over RustSec's pinned commit, OSV-Scanner's
# over OSV's pinned npm and PyPI databases (ci/audit.sh). Its first run
# downloads the snapshot into target/advisories/; a fail is always the
# commit's.
audit:
    ci/tool.sh cargo-audit ci/audit.sh rust
    ci/tool.sh cargo-audit ci/audit.sh rust xtask/compare-spider/Cargo.toml
    ci/tool.sh osv-scanner ci/audit.sh npm xtask/compare-crawlee/pnpm-lock.yaml
    ci/tool.sh osv-scanner ci/audit.sh python xtask/compare-scrapy/uv.lock
    ci/tool.sh cargo-audit ci/audit.sh rust kennel/Cargo.toml
    ci/tool.sh osv-scanner ci/audit.sh npm
    ci/tool.sh osv-scanner ci/audit.sh python

# The live advisory audit, a Live check (ADR 0064) that runs daily on
# `main`'s head (from the server's cron while the hosted workflows are off):
# the same lockfiles against the newest RustSec, npm registry and PyPI
# advisories, through cargo-deny, cargo-audit, pnpm audit and pip-audit
# (ci/audit.sh live), then the proofs that pnpm audit and pip-audit still
# refuse a published advisory (ci/proofs/live.sh). A fail is answered by a
# pull request that fixes the dependency and refreshes the snapshot.
audit-live:
    ci/tool.sh cargo-audit ci/audit.sh live rust
    ci/audit.sh live npm
    ci/audit.sh live python
    ci/proofs/live.sh

# The daily Feature and Journey reports (ADRs 0063 and 0072), from the
# server's cron on `main`'s head while the hosted workflows are off (ADR
# 0135): every runner's listings on this build, then `xtask features report`
# at the gate of `milestone`, the milestone whose owner test comes next, and
# `xtask journeys report` with that owner test's "Journeys to try", both from
# `main`'s latest Release run in the results store checked out at `store`.
# Each day's reports are scanned for secrets, then committed into the store
# under `reports/<date>/` and pushed, so the reports keep a history (ADR
# 0072, Reports). The store's checkout keeps the credential that pushes.
reports store milestone="M1":
    #!/usr/bin/env bash
    set -euo pipefail
    store=$(cd "{{ store }}" && pwd)
    git -C "$store" pull --quiet --ff-only
    day=$(date -u +%F)
    out="$store/reports/$day"
    rm -rf "$out"
    mkdir -p "$out"
    {{ just_executable() }} console-build
    cargo run --quiet --locked --bin xtask -- journeys listings --out target/journey-listings \
      --cargo-profile {{ test_profile }}
    cargo run --quiet --locked --bin xtask -- features report --milestone "{{ milestone }}" \
      --store "$store" --listings target/journey-listings --out "$out/features" >"$out/features.txt"
    cargo run --quiet --locked --bin xtask -- journeys report --store "$store" \
      --listings target/journey-listings --out "$out/journeys" >"$out/journeys.md"
    cargo run --quiet --locked --bin xtask -- journeys report --owner-test "{{ milestone }}" \
      >"$out/journeys/journeys-to-try.md"
    {{ just_executable() }} leaks "$out"
    cat "$out/features.txt"
    git -C "$store" add "reports/$day"
    if git -C "$store" diff --cached --quiet; then
      echo "The store already holds the reports of $day."
      exit 0
    fi
    git -C "$store" commit --quiet --message "Reports of $day"
    git -C "$store" push --quiet origin HEAD
    echo "Reports of $day written into $out and pushed."

# Builds the workspace with the toolchain of its `rust-version`, the MSRV of
# ADR 0030 (latest stable minus two).
msrv:
    {{ just_executable() }} console-build
    ci/msrv.sh

# cargo-hack over every feature combination of each Product's SDK crate
# (ADR 0087).
feature-combinations:
    {{ just_executable() }} console-build
    ci/tool.sh cargo-hack ci/feature-combinations.sh

# Miri over the pure-Rust `unsafe` modules that opt in through ci/miri.toml
# (STYLE.md, `unsafe`).
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
    cargo run --quiet --bin xtask -- dco --base "$base" --head "{{ head }}"

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
    cargo run --quiet --bin xtask -- commit-messages --base "$base" --head "{{ head }}" \
      --since "{{ conventional_commits_since }}"

# Whether this checkout's builds reuse the C and C++ objects other checkouts
# compiled (TESTING.md, Compiler cache): where the Rust compiler wrapper is
# sccache, it reports sccache's base directories not listing this checkout, or
# CMake's compiler launcher bypassing sccache, and fails on them only where
# COMPILER_CACHE_REQUIRED=1 (the shared build server, `just merge`); where no
# build uses sccache it is skipped. `just quick` and `just merge` run it
# before any build.
compiler-cache:
    ci/compiler-cache.sh

# The floor guard (ADR 0140; TESTING.md, Quality register): what the change
# since the merge base with `base` does to the bar. A flagged change passes only
# when a `Quality-change: <file or row>: <reason>` trailer of the change names it.
floor-guard base="origin/main" head="HEAD":
    cargo run --quiet --bin xtask -- floor-guard --base "{{ base }}" --head "{{ head }}"

# Runs the static checks against fixtures that each must fail (ADR 0062's bans
# and the snapshot rules), proving the configurations refuse what they should.
proofs:
    ci/proofs/run.sh

# Opts in to the git hooks: pre-commit checks formatting, pre-push runs `just quick`
# unless it already passed on the pushed tree (TESTING.md, Checks by role).
hooks:
    git config core.hooksPath .githooks

# Merges a pull request only when `just pre-merge all`, which starts with
# `just quick all`'s lint and Quick suite, passes on its exact head, in a
# clean checkout in the gate's own worktree (AGENTS.md, "Merging"), or `just
# pre-merge quick` when no test reads the change (`xtask affected`'s
# `gate=quick`, ADR 0135), or `just pre-merge docs` when it is documents
# alone (`gate=docs`). It refuses once 10 merges have landed on `main` since
# the backstop's last green run (ci/merge-cap.sh) unless the owner passes
# `--past-cap`: `just merge <number> --past-cap`. It refuses to start a gate
# under GOODFETCH_GATE_MIN_FREE_GB (30) free on the file system of the gate's
# `target/`, keeps a failing gate's log and test results in the common Git
# folder's `goodfetch-gate-failures/` for 30 days, and sweeps the gate's
# `target/` after the gate, still holding its lock.
merge number *override:
    ci/merge-pr.sh {{ number }} {{ override }}

# Removes a Cargo target folder's dead units, which no build has read for
# `--unused-hours` (48 by default, at least 25) or since the day before the
# Unix time `--read-since` gives (`just merge` passes its gate's start), and
# keeps whole each unit a build read:
# `just sweep-target --target <folder> [--dry-run]`.
sweep-target *args:
    cargo run --quiet --bin xtask -- sweep-target {{ args }}

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
    cargo run --quiet --bin xtask -- test-tools --cargo-profile {{ test_profile }} --out "$PWD/target/test-tools.json"

# Removes the exercise records and the nextest JUnit file of an earlier
# Pre-merge or Release run, which `just features-audit` would read.
_clear-exercise-records:
    rm -rf target/test-records target/nextest/ci/junit.xml

# One suite's tests over the whole workspace with `all`, else over what changed.
_suite suite all="":
    #!/usr/bin/env bash
    set -euo pipefail
    case "{{ all }}" in
      all) {{ just_executable() }} test {{ suite }} ;;
      "") {{ just_executable() }} test-affected {{ suite }} ;;
      *) echo "error: the only argument is \`all\`, not {{ all }}" >&2; exit 2 ;;
    esac

# TLC on the models the change reaches (`all`: every model), then the job's
# self-test, as CI's `tlc` job runs them; listed as skipped without Java.
_tlc all="":
    #!/usr/bin/env bash
    set -euo pipefail
    just="{{ just_executable() }}"
    case "{{ all }}" in
      all) models=(--all) ;;
      "")
        selection=$(cargo run --quiet --bin xtask -- affected --local)
        source ci/output-value.sh
        selected=$(output_value models "$selection")
        read -ra models <<< "$selected"
        if [[ ${#models[@]} -eq 0 ]]; then
          echo "The change reaches no TLA+ model."
          exit 0
        fi
        ;;
      *) echo "error: the only argument is \`all\`, not {{ all }}" >&2; exit 2 ;;
    esac
    ci/tool.sh java "$just" tlc "${models[@]}"
    ci/tool.sh java "$just" tlc-self-test

# The gate's `kennel` step: the Kennel's checks when the change reaches
# its job (`xtask affected`'s `kennel`: a change to `kennel/`, to the root
# crates it is built on or to the executables its tests run, ADR 0141), and
# always with `all`.
_kennel all="":
    #!/usr/bin/env bash
    set -euo pipefail
    case "{{ all }}" in
      all) ;;
      "")
        selection=$(cargo run --quiet --bin xtask -- affected --local)
        source ci/output-value.sh
        jobs=$(output_value jobs "$selection")
        reached=$(jq 'index("kennel") != null' <<< "$jobs")
        if [[ $reached != true ]]; then
          echo "The change does not reach the Kennel."
          exit 0
        fi
        ;;
      *) echo "error: the only argument is \`all\`, not {{ all }}" >&2; exit 2 ;;
    esac
    {{ just_executable() }} kennel-check

# Runner configs define the `visual` projects only inside the pinned image
# (ci/visual/run.sh), so no host test command runs them.
_visual-elsewhere:
    @mkdir -p target
    @echo "visual projects: skipped, they run only in the pinned Playwright image (CI's visual job, ADR 0073)" >>target/skipped.txt

# The steps a form of `just pre-merge` runs (`all`, `changed`, `quick` or
# `docs`), in their order, with each one's `just` command, which
# ci/pre-merge.sh follows (`xtask gate steps`, ADR 0135).
_gate-steps form:
    cargo run --quiet --bin xtask -- gate steps --form {{ form }}

# The incremental gate's plan for HEAD (ADR 0135), from the results earlier
# gate runs recorded in target/gate-runs/, which `just merge` keeps between
# gates; GOODFETCH_GATE_FULL=1 reuses nothing.
# Vitest's and Playwright's listings since a recorded head, the evidence of
# what the TypeScript tests read, load each test file, which reads only the
# keys of the test tool manifest: an empty one serves.
_gate-plan:
    #!/usr/bin/env bash
    set -euo pipefail
    mkdir -p target/gate-runs
    echo '{}' >target/gate-runs/listing-tools.json
    # The variables that change what the steps run or how join the host
    # fingerprint, so a result counts only for a run in the same environment.
    variables=$(env | grep -E '^(CI|GOODFETCH_[A-Z0-9_]+|NEXTEST_[A-Z0-9_]+|CARGO_[A-Z0-9_]+|RUST[A-Z0-9_]*|UV_[A-Z0-9_]+|NODE_[A-Z0-9_]+)=' | LC_ALL=C sort || true)
    GOODFETCH_TEST_SUITE=pre-merge GOODFETCH_TEST_TOOLS="$PWD/target/gate-runs/listing-tools.json" \
      cargo run --quiet --bin xtask -- gate plan --records target/gate-runs --now "$(date +%s)" \
      --environment "$variables" ${GOODFETCH_GATE_FULL:+--full}

# Records a gate step's result (`pass` or `fail`) for the head the plan was
# made for, which must still be checked out on a clean tree (ADR 0135); a
# failed step run per member's `evidence` (`--junit`, `--selection`) records
# each crate that passed in full green.
_gate-record head step status *evidence:
    cargo run --quiet --bin xtask -- gate record --records target/gate-runs --head {{ head }} \
      --now "$(date +%s)" --step {{ step }} --status {{ status }} {{ evidence }}

# The incremental gate's backstop (ADR 0135), daily from the gate host's
# cron on `main`'s head: in a gate worktree of its own, `just pre-merge all`
# in full (`GOODFETCH_GATE_FULL=1`), then each result an incremental run of
# `just merge` on the same tree reused and the full run found red, a
# selection miss to fix (ci/gate-backstop.sh). Its log and results go to
# ~/gate-backstop/ (GOODFETCH_GATE_BACKSTOP_LOG). Run from the main checkout.
gate-backstop ref="origin/main":
    ci/gate-backstop.sh {{ ref }}

# The test-health runs the gate host's cron starts while Actions is off
# (TESTING.md, After merge; ci/test-health.crontab), on `main`'s head in the
# backstop's gate worktree: `constrained`, the constrained test-health run,
# and `mutants`, the advisory mutation run over the last day's Rust changes
# (ci/test-health/scheduled.sh). Logs and results go to ~/test-health/
# (GOODFETCH_TEST_HEALTH_LOG). Run from the main checkout.
test-health run ref="origin/main":
    ci/test-health/scheduled.sh {{ run }} {{ ref }}

# The backstop's comparison (ADR 0135): each result an incremental run in
# `runs` on the full run's tree reused and the full run on `full` found red.
_gate-compare full runs="target/gate-runs":
    cargo run --quiet --locked --bin xtask -- gate compare --records target/gate-runs \
      --runs "{{ runs }}" --full {{ full }}

# Starts and checks the real OS services the tests use (ci/os-services.sh),
# which `just pre-merge` runs first on a Linux host.
_os-services:
    ci/os-services.sh setup

_skips-report:
    #!/usr/bin/env bash
    if [[ -s target/skipped.txt ]]; then
      echo "Not run on this host:"
      sed 's/^/  - /' target/skipped.txt
    fi
