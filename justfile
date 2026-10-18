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
    ci/tool.sh shellcheck shellcheck ci/*.sh ci/egress/*.sh ci/host/*.sh ci/release/*.sh ci/runner/*.sh ci/vm/*.sh ci/visual/*.sh ci/proofs/run.sh .githooks/pre-commit .githooks/pre-push
    ci/tool.sh cargo-deny cargo deny check bans
    ci/tool.sh cargo-shear cargo shear
    cargo run --quiet -p xtask -- self-hosted check
    cargo run --quiet -p xtask -- egress check
    cargo run --quiet -p xtask -- fixture-size
    cargo run --quiet -p xtask -- dependency-direction
    cargo run --quiet -p xtask -- mocking-lockfile
    cargo run --quiet -p xtask -- modules
    ci/repository-files.sh

# Each job's VM runs this when it starts, before ADR 0043's outbound-traffic
# deny: the locked dependencies of each workspace and, as areas pin them, the
# Playwright browsers, the Clean-up rules' inputs and other pinned inputs.
# These may retry: they test nothing (ADR 0064).
# Makes every download the pull-request tier needs, before the egress deny.
fetch:
    cargo fetch --locked
    pnpm install --frozen-lockfile
    uv sync --frozen

# Builds Rust (with the given Cargo profile), TypeScript and Python.
build profile="dev":
    cargo build --workspace --all-targets --profile {{ profile }}
    pnpm -r --if-present run build
    uv build --all-packages --out-dir target/python-dist

# Runs one suite's tests without lints: `quick`, `pre-merge` or `release`.
# Packages, when given, narrow the tests: a crate's name the Rust tests
# (`-p <crate>`), a pnpm package's folder Vitest's and Playwright's, a uv
# package's folder pytest's; a runner given none of its packages is skipped.
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
            --disable-nextest-doctest
        fi
        if $typescript; then
          pnpm exec vitest run --project unit --slowTestThreshold=1000 ${vitest[@]+"${vitest[@]}"}
        fi
        # pytest exits 5 when it collects no test, as before any Python test exists.
        if $python; then
          uv run pytest --timeout=10 --ignore-glob='*/tests/integration/*' ${pytest[@]+"${pytest[@]}"} || [[ $? -eq 5 ]]
        fi
        ;;
      pre-merge|release)
        if $rust; then
          NEXTEST_NO_TESTS=pass INSTA_UPDATE=no cargo insta test "${packages[@]}" --check --unreferenced reject \
            --test-runner nextest --nextest-profile ci --profile {{ test_profile }} \
            --disable-nextest-doctest
          # nextest runs no doctests; cargo-insta's own doctest pass would take
          # nextest's profile flag, which `cargo test` refuses.
          cargo test "${packages[@]}" --doc --profile {{ test_profile }}
        fi
        if $typescript; then
          pnpm exec vitest run ${vitest[@]+"${vitest[@]}"}
          pnpm exec playwright test --pass-with-no-tests ${vitest[@]+"${vitest[@]}"}
        fi
        if $python; then
          uv run pytest ${pytest[@]+"${pytest[@]}"} || [[ $? -eq 5 ]]
        fi
        ;;
      *)
        echo "error: the suite is quick, pre-merge or release, not {{ suite }}" >&2
        exit 2
        ;;
    esac

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

# The Quick suite: lints and in-process tests of what changed (`all`: the
# whole workspace).
quick *all: _skips-reset lint (_suite "quick" all) _skips-report

# The Pre-merge suite on this host: the Quick suite, then the tests of what
# changed and the static checks' proofs (`all`: the whole workspace). The
# visual projects run only in the pinned Playwright image, in CI's `visual`
# job (ADR 0073).
pre-merge *all: _skips-reset lint (_suite "quick" all) (_suite "pre-merge" all) proofs _visual-elsewhere _skips-report

# This host's cell of the Release suite. The other cells run in CI only:
# `just release-ci`.
release: _skips-reset lint (test "quick") (test "release") proofs _visual-elsewhere _skips-report
    @echo "skipped: every other OS and runtime cell of the Release suite runs only in CI (just release-ci)"

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

# The cache-warming job's build (ADRs 0061 and 0071): the pull-request tier's
# dependencies, tests and benchmark targets with its profiles, run by nothing.
warm:
    pnpm exec tsc -p tsconfig.json
    cargo clippy --workspace --all-targets --all-features
    cargo build -p xtask
    cargo nextest run --workspace --profile ci --cargo-profile ci-test --no-run
    cargo build --workspace --profile ci
    cargo bench --workspace --no-run

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

# One suite's tests over the whole workspace with `all`, else over what changed.
_suite suite all:
    #!/usr/bin/env bash
    set -euo pipefail
    case "{{ all }}" in
      all) {{ just_executable() }} test {{ suite }} ;;
      "") {{ just_executable() }} test-affected {{ suite }} ;;
      *) echo "error: the only argument is \`all\`, not {{ all }}" >&2; exit 2 ;;
    esac

_skips-reset:
    @rm -f target/skipped.txt

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
