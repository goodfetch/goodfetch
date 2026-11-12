#!/usr/bin/env bash
# One cell of the Release suite in CI (ADRs 0061, 0094 and 0108), on its
# runner's own system, after .github/actions/host-tools:
#   build       before the deny: builds Collie's Console first (`just
#               console-build`, ADR 0057), as the local gate does, then the
#               target's tests once with `cargo nextest archive`, the test
#               tool manifest of the executables tests run, and the `ci`
#               executables with the thin-LTO `ci` profile, both Products'
#               among them, from one invocation (ADR 0094), with the manifest
#               the resident-memory check runs and the size report reads;
#   size        writes the target's size report of both Products, keyed by
#               the tree the cell tested (`xtask size`, ADR 0061), and on a
#               musl cell fails on an executable of theirs that is no
#               static-PIE with packed relocations (`xtask static-pie`, ADR
#               0024);
#   rust        one shard of the archive (`--partition hash:<k>/<n>`, never
#               by timings), then the doctests;
#   typescript  Vitest and Playwright, `--shard <k>/<n>`, on each Node line
#               host-tools recorded; Bun and Deno 2 are on PATH for the SDK
#               smoke scripts Vitest spawns (ADR 0069);
#   python      pytest on each CPython host-tools recorded, split across
#               processes by pytest-xdist;
#   lazyfs      on linux-x64-gnu, the LazyFS power cuts (ADR 0060): the
#               archive's `lazyfs::` tests, which nextest's profiles leave
#               out of `rust`, on the LazyFS ci/host/lazyfs.sh built;
#   rss         the resident-memory check (ADRs 0071, 0094 and 0134): each
#               scenario on each Product's `ci` executable it runs on, each
#               peak held to its ceiling in ci/rss-ceilings.toml, reporting
#               and gating nothing while that file does not exist; its JUnit
#               file and figures land in target/junit/.
# The test parts run under ADR 0043's deny through ci/egress/under-deny.sh,
# which the workflow step names. Each runner's JUnit file lands in
# target/junit/ under a name that says its runtime.
#
# It keeps to bash 3.2, which stock macOS runs on the owner's machine.
#
# Environment: CELL (the cell's name), TARGET (its Rust target), SHARD
# (`<k>/<n>`, 1/1 while one job holds the cell), and host-tools'
# GOODFETCH_NODE_LINES and GOODFETCH_PYTHONS.
set -euo pipefail

part=${1:?usage: ci/release/cell.sh build|size|rust|lazyfs|typescript|python|rss}
cell=${CELL:?the job sets CELL}
target=${TARGET:?the job sets TARGET}
shard=${SHARD:-1/1}
root=$(cd "$(dirname "$0")/../.." && pwd)
cd "$root"

archive=target/release-suite/tests.tar.zst
rss_tools="$root/target/release-suite/rss-tools.json"
rss_peaks=target/release-suite/rss-peaks.jsonl
exclusions=target/release-suite/quarantine.json
mkdir -p target/junit target/release-suite

# A selection may hold no test of a runner yet; every test it holds runs.
export NEXTEST_NO_TESTS=pass INSTA_UPDATE=no

# Every runner's tests read the suite, and find the commit's executables
# through the test tool manifest `build` writes, as under `just test`
# (ADRs 0062 and 0094).
export GOODFETCH_TEST_SUITE=release
export GOODFETCH_TEST_TOOLS="$root/target/release-suite/test-tools.json"
# Each Journey variant writes its result document, `<id>@<surface>.json`,
# into the JUnit folder, so the results store keeps it with the cell's
# JUnit files and `xtask journeys report` reads it (ADR 0063).
export GOODFETCH_TEST_RESULT_DOCUMENTS="$root/target/junit/result-documents"

# Vitest, Playwright and pytest order their tests by the tree under test's
# seed, as `just test` does (TESTING.md, Before merge).
GOODFETCH_ORDER_SEED=$(ci/order-seed.sh)
export GOODFETCH_ORDER_SEED

# Each runner skips the tests on the test quarantine list through its own
# filter (ADR 0064), as `just test` does; `build` writes the list's
# exclusions, `xtask quarantine exclude`'s JSON, before the deny.
skip() {
  [[ -f "$exclusions" ]] || {
    echo "error: $exclusions is missing; run ci/release/cell.sh build first" >&2
    exit 2
  }
  jq -r --arg runner "$1" '.[$runner][]' "$exclusions"
}

case "$part" in
  build)
    # Collie's executable embeds its Console build (ADR 0057), which the
    # archive's and the `ci` profile's builds of it read: without it they fail
    # to compile. It builds from the packages `just fetch` installed.
    just console-build
    # A musl cell builds with the static musl toolchain (ADR 0024's note of
    # 2026-11-06): nextest runs its Cargo, ci/musl/cargo, through `CARGO`,
    # which `cargo nextest` would reset to Cargo itself; `test-tools` picks
    # it for the target.
    nextest=(cargo nextest)
    if [[ "$target" == *-linux-musl ]]; then
      nextest=(env CARGO="$root/ci/musl/cargo" cargo-nextest nextest)
    fi
    "${nextest[@]}" archive --workspace --locked --target "$target" \
      --profile ci --cargo-profile ci-test --archive-file "$archive"
    # `test-tools` builds the `ci` executables itself, in one invocation of
    # their own (ADR 0094's note), so no `cargo build --workspace` of that
    # profile runs before it: its executables were built again and
    # overwritten there, and its xtask never ran (6.5 of the 15 minutes the
    # build took on the server on 2026-10-23).
    cargo run --quiet --locked --bin xtask -- test-tools build --cargo-profile ci-test \
      --target "$target" --out "$GOODFETCH_TEST_TOOLS"
    cargo run --quiet --locked --bin xtask -- quarantine exclude >"$exclusions"
    cargo run --quiet --locked --bin xtask -- test-tools build --cargo-profile ci \
      --target "$target" --out "$rss_tools"
    ;;
  size)
    tree=$(git rev-parse 'HEAD^{tree}')
    [[ -n "${GITHUB_OUTPUT:-}" ]] && echo "tree=$tree" >>"$GITHUB_OUTPUT"
    ci/release/size-manifest.sh "$target" >target/release-suite/size-files.toml
    cargo run --quiet --locked --bin xtask -- size estimate \
      --files target/release-suite/size-files.toml \
      --save "target/release-suite/size-report-$tree-$target.json"
    # A musl cell's executables, both Products', are static-PIEs with
    # packed relocations (ADR 0024's note of 2026-11-12); the other cells'
    # are not read.
    cargo run --quiet --locked --bin xtask -- static-pie --target "$target" \
      --files target/release-suite/size-files.toml
    ;;
  rust)
    # Each test leaves its exercise record here, which `just features-audit`
    # reads after this step with nextest's JUnit file (ADR 0072), so it
    # judges the tests this run ran; a run starts with neither.
    rm -rf target/test-records target/nextest/ci/junit.xml
    export GOODFETCH_TEST_RECORDS="$root/target/test-records"
    status=0
    skipped=()
    while IFS= read -r argument; do skipped+=("$argument"); done < <(skip nextest)
    cargo nextest run --archive-file "$archive" --workspace-remap . \
      --profile ci --partition "hash:$shard" ${skipped[@]+"${skipped[@]}"} || status=$?
    [[ -f target/nextest/ci/junit.xml ]] && cp target/nextest/ci/junit.xml "target/junit/nextest-$cell.xml"
    # nextest's archive holds no doctests; the first shard runs them.
    if [[ "$shard" == 1/* ]]; then
      cargo test --workspace --locked --doc --target "$target" --profile ci-test || status=$?
    fi
    exit "$status"
    ;;
  lazyfs)
    export GOODFETCH_LAZYFS="$root/target/lazyfs/bin/lazyfs"
    [[ -x "$GOODFETCH_LAZYFS" ]] || {
      echo "error: $GOODFETCH_LAZYFS is missing; run ci/host/lazyfs.sh before the deny" >&2
      exit 2
    }
    status=0
    cargo nextest run --archive-file "$archive" --workspace-remap . \
      --profile ci --ignore-default-filter -E 'kind(test) & test(/^lazyfs::/)' || status=$?
    [[ -f target/nextest/ci/junit.xml ]] && cp target/nextest/ci/junit.xml "target/junit/nextest-$cell-lazyfs.xml"
    exit "$status"
    ;;
  typescript)
    status=0
    skip_vitest=() skip_playwright=()
    while IFS= read -r argument; do skip_vitest+=("$argument"); done < <(skip vitest)
    while IFS= read -r argument; do skip_playwright+=("$argument"); done < <(skip playwright)
    vitest_order=()
    while IFS= read -r argument; do vitest_order+=("$argument"); done < <(ci/order-arguments.sh vitest)
    playwright_order=()
    while IFS= read -r argument; do playwright_order+=("$argument"); done < <(ci/order-arguments.sh playwright)
    for entry in ${GOODFETCH_NODE_LINES:?host-tools sets GOODFETCH_NODE_LINES}; do
      line=${entry%%=*}
      folder=${entry#*=}
      echo "Node $line ($folder)"
      PATH="$folder:$PATH" pnpm exec vitest run --shard="$shard" "${vitest_order[@]}" \
        --outputFile.junit="target/junit/vitest-node$line.xml" \
        ${skip_vitest[@]+"${skip_vitest[@]}"} || status=$?
      PATH="$folder:$PATH" PLAYWRIGHT_JUNIT_OUTPUT_FILE="target/junit/playwright-node$line.xml" \
        pnpm exec playwright test --shard="$shard" --pass-with-no-tests "${playwright_order[@]}" \
        ${skip_playwright[@]+"${skip_playwright[@]}"} || status=$?
    done
    exit "$status"
    ;;
  python)
    status=0
    skip_pytest=()
    while IFS= read -r argument; do skip_pytest+=("$argument"); done < <(skip pytest)
    pytest_order=()
    while IFS= read -r argument; do pytest_order+=("$argument"); done < <(ci/order-arguments.sh pytest)
    for python in ${GOODFETCH_PYTHONS:?host-tools sets GOODFETCH_PYTHONS}; do
      echo "CPython $python"
      # pytest exits 5 when it collects no test, as before any Python test exists.
      code=0
      uv run --frozen --python "$python" pytest -n auto "${pytest_order[@]}" \
        --junitxml="target/junit/pytest-$python.xml" ${skip_pytest[@]+"${skip_pytest[@]}"} || code=$?
      [[ $code -eq 0 || $code -eq 5 ]] || status=$code
    done
    exit "$status"
    ;;
  rss)
    rm -f "$rss_peaks"
    cargo run --quiet --locked --bin xtask -- rss measure --tools "$rss_tools" --out "$rss_peaks"
    summary=()
    [[ -n "${GITHUB_STEP_SUMMARY:-}" ]] && summary=(--summary "$GITHUB_STEP_SUMMARY")
    cargo run --quiet --locked --bin xtask -- rss check --peaks "$rss_peaks" ${summary[@]+"${summary[@]}"}
    ;;
  *)
    echo "error: the part is build, size, rust, lazyfs, typescript, python or rss, not $part" >&2
    exit 2
    ;;
esac
