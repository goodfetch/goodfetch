# Goodfetch's one entry point (ADR 0061; TESTING.md, Suites, commands and time
# budgets). CI calls the same recipes. Until `xtask affected` exists, `quick`
# and `pre-merge` run over the whole workspace, so `all` changes nothing yet.
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
    cargo clippy --workspace --all-targets --all-features -- -D warnings
    pnpm exec tsc -p tsconfig.json
    pnpm -r --if-present run typecheck
    pnpm exec oxlint
    uv run ruff check .
    uv run pyright
    uv run mypy .
    ci/tool.sh typos typos
    ci/tool.sh zizmor zizmor --config zizmor.yml --persona=regular .github/workflows
    ci/tool.sh actionlint actionlint
    ci/tool.sh shellcheck shellcheck ci/*.sh ci/runner/*.sh ci/vm/*.sh ci/proofs/run.sh .githooks/pre-commit .githooks/pre-push
    ci/tool.sh cargo-deny cargo deny check bans
    ci/tool.sh cargo-shear cargo shear
    cargo run --quiet -p xtask -- self-hosted check
    cargo run --quiet -p xtask -- fixture-size
    cargo run --quiet -p xtask -- dependency-direction
    cargo run --quiet -p xtask -- mocking-lockfile
    ci/repository-files.sh

# Builds Rust (with the given Cargo profile), TypeScript and Python.
build profile="dev":
    cargo build --workspace --all-targets --profile {{ profile }}
    pnpm -r --if-present run build
    uv build --all-packages --out-dir target/python-dist

# Runs one suite's tests without lints: `quick`, `pre-merge` or `release`.
# Packages, when given, narrow the Rust tests (`-p <crate>`).
test suite *packages:
    #!/usr/bin/env bash
    set -euo pipefail
    packages=()
    for package in {{ packages }}; do packages+=(-p "$package"); done
    [[ ${#packages[@]} -eq 0 ]] && packages=(--workspace)
    # A selection may hold no test of a suite yet (NEXTEST_NO_TESTS=pass);
    # every test it holds still runs.
    case "{{ suite }}" in
      quick)
        # In-process tests only (nextest's quick profile); snapshots are
        # checked, never written. Unreferenced snapshots are judged only
        # when every test runs, in the Pre-merge suite.
        NEXTEST_NO_TESTS=pass INSTA_UPDATE=no cargo insta test "${packages[@]}" --check \
          --test-runner nextest --nextest-profile quick --profile {{ test_profile }} \
          --disable-nextest-doctest
        pnpm exec vitest run --project unit --slowTestThreshold=1000
        # pytest exits 5 when it collects no test, as before any Python test exists.
        uv run pytest --timeout=10 --ignore-glob='*/tests/integration/*' || [[ $? -eq 5 ]]
        ;;
      pre-merge|release)
        NEXTEST_NO_TESTS=pass INSTA_UPDATE=no cargo insta test "${packages[@]}" --check --unreferenced reject \
          --test-runner nextest --nextest-profile ci --profile {{ test_profile }} \
          --disable-nextest-doctest
        # nextest runs no doctests; cargo-insta's own doctest pass would take
        # nextest's profile flag, which `cargo test` refuses.
        cargo test "${packages[@]}" --doc --profile {{ test_profile }}
        pnpm exec vitest run
        pnpm exec playwright test --pass-with-no-tests
        uv run pytest || [[ $? -eq 5 ]]
        ;;
      *)
        echo "error: the suite is quick, pre-merge or release, not {{ suite }}" >&2
        exit 2
        ;;
    esac

# The Quick suite: lints and in-process tests (`all`: the whole workspace).
quick *all: _skips-reset lint (test "quick") _skips-report

# The Pre-merge suite on this host: the Quick suite, then every test and the
# static checks' proofs (`all`: the whole workspace).
pre-merge *all: _skips-reset lint (test "quick") (test "pre-merge") proofs _skips-report

# This host's cell of the Release suite. The other cells run in CI only:
# `just release-ci`.
release: _skips-reset lint (test "quick") (test "release") proofs _skips-report
    @echo "skipped: every other OS and runtime cell of the Release suite runs only in CI (just release-ci)"

# Dispatches the Release suite's workflow on the pushed branch and follows it;
# the exit code is the run's verdict (ADRs 0061 and 0108). The workflow comes
# with #145.
release-ci:
    #!/usr/bin/env bash
    set -euo pipefail
    branch=$(git rev-parse --abbrev-ref HEAD)
    if ! gh workflow view release.yml >/dev/null 2>&1; then
      echo "error: the Release workflow (release.yml) does not exist yet; #145 adds it" >&2
      exit 2
    fi
    gh workflow run release.yml --ref "$branch"
    sleep 5
    run=$(gh run list --workflow release.yml --branch "$branch" --event workflow_dispatch --limit 1 --json databaseId --jq '.[0].databaseId')
    gh run watch "$run" --exit-status

# The cache-warming job's build (ADRs 0061 and 0071): the pull-request tier's
# dependencies, tests and benchmark targets with its profiles, run by nothing.
warm:
    pnpm exec tsc -p tsconfig.json
    cargo clippy --workspace --all-targets --all-features
    cargo build -p xtask
    cargo nextest run --workspace --profile ci --cargo-profile ci-test --no-run
    cargo build --workspace --profile ci
    cargo bench --workspace --no-run

# Runs the static checks against fixtures that each must fail (ADR 0062's bans
# and the snapshot rules), proving the configurations refuse what they should.
proofs:
    ci/proofs/run.sh

# Opts in to the git hooks: pre-commit checks formatting, pre-push runs `just quick`.
hooks:
    git config core.hooksPath .githooks

# Installs the owner's copy of the self-hosted allowlist and the runner's
# pre-job and post-job scripts into a folder the runner user can read but not
# write (ADRs 0108 and 0135). Run by the owner on the runner's machine.
runner-install folder:
    ci/runner/install.sh "{{ folder }}"

# Runs ADR 0135's Rosetta trial in a throwaway Lima VM: Callgrind, the pinned
# Playwright image and `cargo fuzz check` under Rosetta for Linux.
rosetta-trial:
    ci/vm/rosetta-trial.sh

_skips-reset:
    @rm -f target/skipped.txt

_skips-report:
    #!/usr/bin/env bash
    if [[ -s target/skipped.txt ]]; then
      echo "Not run on this host:"
      sed 's/^/  - /' target/skipped.txt
    fi
