#!/usr/bin/env bash
# ADR 0072's code coverage, measured and never gated (TESTING.md, Code
# coverage and mutation testing): `ci/coverage.sh [all | <change>…]`, which
# `just coverage` runs. It runs the Pre-merge suite's tests of what changed
# (the arguments of `just test-affected`, `--local` when none), or of the
# whole workspace with `all`:
#   - every workspace crate built for coverage through cargo-llvm-cov's
#     `show-env`, into a target folder of its own, so nextest's tests and
#     every SDK, Journey and Console run start the instrumented executables
#     of both Products (ADR 0094), whose profiles test support's map passes
#     on (LLVM_PROFILE_FILE);
#   - Vitest's `v8` provider, in Node and in Browser Mode
#     (vitest.config.ts reads GOODFETCH_COVERAGE);
#   - coverage.py with branch measurement, through pytest-cov.
# The three reports go to target/coverage/{rust,vitest,python}/, written even
# when a test failed: every runner runs after another's failure
# (GOODFETCH_TEST_KEEP_GOING), the Rust report leaves out a profile it cannot
# read and names it, and the run's failed tests are listed last. The exit
# code is the tests' own. No threshold, no `--fail-under-*` and no coverage
# service: nothing here judges a number.
set -euo pipefail

root=$(cd "$(dirname "$0")/.." && pwd)
cd "$root"
just=${JUST:-just}
out="$root/target/coverage"
rm -rf "$out"
mkdir -p "$out/rust" "$out/python"

# The instrumented build has a target folder of its own, so it never
# replaces the ordinary build's executables.
target=$(cargo metadata --format-version 1 --no-deps | jq -r .target_directory)
export CARGO_TARGET_DIR="$target/llvm-cov-target"
# cargo-llvm-cov's own RUSTC_WRAPPER instruments only the workspace's crates;
# a compiler cache's wrapper would stand in its place.
unset RUSTC_WRAPPER
eval "$(cargo llvm-cov show-env --sh --no-cfg-coverage)"
cargo llvm-cov clean --workspace

export GOODFETCH_COVERAGE="$out"
# coverage.py's data file, which would otherwise land in the workspace's root.
export COVERAGE_FILE="$out/python/.coverage"
export PYTEST_ADDOPTS="${PYTEST_ADDOPTS:+$PYTEST_ADDOPTS }--cov=sdks/python --cov=test-support/python --cov-branch \
--cov-report=term --cov-report=html:$out/python/html --cov-report=lcov:$out/python/lcov.info"

# One red test must not cost the other runners' figures (`just test`).
export GOODFETCH_TEST_KEEP_GOING=1
# The JUnit files this run's runners write, which name its failed tests last;
# an earlier run's would name that run's.
junit=(target/junit/nextest-ci.xml target/junit/vitest.xml target/junit/playwright.xml target/junit/pytest.xml)
rm -f target/nextest/ci/junit.xml "${junit[@]}"

status=0
if [[ "${1:-}" == all ]]; then
  [[ $# -eq 1 ]] || {
    echo "error: \`all\` takes no other argument" >&2
    exit 2
  }
  "$just" test pre-merge || status=$?
else
  "$just" test-affected pre-merge "$@" || status=$?
fi

# nextest writes its JUnit file under the workspace's own target/nextest/,
# not the instrumented build's target folder; the job uploads it with the
# other runners' (ci/junit-context.sh).
mkdir -p target/junit
if [[ -f target/nextest/ci/junit.xml ]]; then
  cp target/nextest/ci/junit.xml target/junit/nextest-ci.xml
fi

# Each report covers what ran; a run that built no Rust test reports none.
# A profile llvm-profdata cannot read, such as one a killed executable left
# half written, is left out with a warning naming it (`--failure-mode all`);
# the report fails only when it can read none.
if compgen -G "$CARGO_TARGET_DIR/*.profraw" >/dev/null; then
  cargo llvm-cov report --failure-mode all --summary-only
  cargo llvm-cov report --failure-mode all --html --output-dir "$out/rust"
  cargo llvm-cov report --failure-mode all --lcov --output-path "$out/rust/lcov.info"
else
  echo "Rust coverage: no instrumented executable ran"
fi

# The run's failed tests, from its own JUnit files only, after the report so
# xtask's own run adds nothing to it.
failures=$(mktemp -d)
for report in "${junit[@]}"; do
  if [[ -f "$report" ]]; then cp "$report" "$failures/"; fi
done
summary=0
cargo run --quiet -p xtask -- junit-summary --junit "$failures" --summary "$failures.md" || summary=$?
if [[ -f "$failures.md" ]]; then cat "$failures.md"; fi
rm -rf "$failures" "$failures.md"
[[ $status -ne 0 ]] || status=$summary

echo "Coverage reports: $out/rust/html, $out/vitest and $out/python/html"
exit "$status"
