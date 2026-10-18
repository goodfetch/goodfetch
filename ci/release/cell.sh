#!/usr/bin/env bash
# One cell of the Release suite in CI (ADRs 0061, 0094 and 0108), on its
# runner's own system, after .github/actions/host-tools:
#   build       before the deny: builds the target's tests once with
#               `cargo nextest archive`, and both Products' executables with
#               the thin-LTO `ci` profile, from one invocation (ADR 0094);
#   size        writes the target's size report of both Products, keyed by
#               the tree the cell tested (`xtask size`, ADR 0061);
#   rust        one shard of the archive (`--partition hash:<k>/<n>`, never
#               by timings), then the doctests;
#   typescript  Vitest and Playwright, `--shard <k>/<n>`, on each Node line
#               host-tools recorded; Bun and Deno 2 are on PATH for the SDK
#               smoke scripts Vitest spawns (ADR 0069);
#   python      pytest on each CPython host-tools recorded, split across
#               processes by pytest-xdist.
# The test parts run under ADR 0043's deny through ci/egress/under-deny.sh,
# which the workflow step names. Each runner's JUnit file lands in
# target/junit/ under a name that says its runtime.
#
# Environment: CELL (the cell's name), TARGET (its Rust target), SHARD
# (`<k>/<n>`, 1/1 while one job holds the cell), and host-tools'
# GOODFETCH_NODE_LINES and GOODFETCH_PYTHONS.
set -euo pipefail

part=${1:?usage: ci/release/cell.sh build|size|rust|typescript|python}
cell=${CELL:?the job sets CELL}
target=${TARGET:?the job sets TARGET}
shard=${SHARD:-1/1}
root=$(cd "$(dirname "$0")/../.." && pwd)
cd "$root"

archive=target/release-suite/tests.tar.zst
mkdir -p target/junit target/release-suite

# A selection may hold no test of a runner yet; every test it holds runs.
export NEXTEST_NO_TESTS=pass INSTA_UPDATE=no

case "$part" in
  build)
    cargo nextest archive --workspace --locked --target "$target" \
      --profile ci --cargo-profile ci-test --archive-file "$archive"
    cargo build --workspace --locked --target "$target" --profile ci
    ;;
  size)
    tree=$(git rev-parse 'HEAD^{tree}')
    [[ -n "${GITHUB_OUTPUT:-}" ]] && echo "tree=$tree" >>"$GITHUB_OUTPUT"
    ci/release/size-manifest.sh "$target" >target/release-suite/size-files.toml
    cargo run --quiet --locked -p xtask -- size estimate \
      --files target/release-suite/size-files.toml \
      --save "target/release-suite/size-report-$tree-$target.json"
    ;;
  rust)
    status=0
    cargo nextest run --archive-file "$archive" --workspace-remap . \
      --profile ci --partition "hash:$shard" || status=$?
    [[ -f target/nextest/ci/junit.xml ]] && cp target/nextest/ci/junit.xml "target/junit/nextest-$cell.xml"
    # nextest's archive holds no doctests; the first shard runs them.
    if [[ "$shard" == 1/* ]]; then
      cargo test --workspace --locked --doc --target "$target" --profile ci-test || status=$?
    fi
    exit "$status"
    ;;
  typescript)
    status=0
    for entry in ${GOODFETCH_NODE_LINES:?host-tools sets GOODFETCH_NODE_LINES}; do
      line=${entry%%=*}
      folder=${entry#*=}
      echo "Node $line ($folder)"
      PATH="$folder:$PATH" pnpm exec vitest run --shard="$shard" \
        --outputFile.junit="target/junit/vitest-node$line.xml" || status=$?
      PATH="$folder:$PATH" PLAYWRIGHT_JUNIT_OUTPUT_FILE="target/junit/playwright-node$line.xml" \
        pnpm exec playwright test --shard="$shard" --pass-with-no-tests || status=$?
    done
    exit "$status"
    ;;
  python)
    status=0
    for python in ${GOODFETCH_PYTHONS:?host-tools sets GOODFETCH_PYTHONS}; do
      echo "CPython $python"
      # pytest exits 5 when it collects no test, as before any Python test exists.
      code=0
      uv run --frozen --python "$python" pytest -n auto \
        --junitxml="target/junit/pytest-$python.xml" || code=$?
      [[ $code -eq 0 || $code -eq 5 ]] || status=$code
    done
    exit "$status"
    ;;
  *)
    echo "error: the part is build, size, rust, typescript or python, not $part" >&2
    exit 2
    ;;
esac
