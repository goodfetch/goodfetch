#!/usr/bin/env bash
# One cell of the Release suite in CI (ADRs 0061, 0094 and 0108), on its
# runner's own system, after .github/actions/host-tools:
#   build       before the deny: builds the
#               target's tests once with `cargo nextest archive`, the test
#               tool manifest of the executables tests run, and the `ci`
#               executables with the thin-LTO `ci` profile, both Products'
#               among them, from one invocation (ADR 0094), with the manifest
#               the resident-memory check runs and the size report reads;
#   tools       before the deny, for a job that runs no Rust test (the
#               nightly tier's free-threaded CPython, ADR 0069): the test
#               tool manifest and the quarantine's exclusions alone;
#   size       writes the target's size report of both Products, keyed by
#               the tree the cell tested (`xtask size`, ADR 0061), and on a
#               musl cell fails on an executable of theirs that is no
#               static-PIE with packed relocations (`xtask static-pie`, ADR
#               0024);
#   rust        one shard of the archive (`--partition hash:<k>/<n>`, never
#               by timings), then the doctests, of every crate but those
#               GOODFETCH_RELEASE_REUSED names;
#   typescript  Vitest and Playwright, `--shard <k>/<n>`, on each Node line
#               host-tools recorded; the Bun and Deno 2 of the cell's set of
#               ci/runtimes.toml are on PATH for the SDK smoke scripts Vitest
#               spawns (ADR 0069);
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
# (`<k>/<n>`, 1/1 while one job holds the cell), host-tools'
# GOODFETCH_NODE_LINES and GOODFETCH_PYTHONS, and on linux-x64-gnu
# GOODFETCH_RELEASE_REUSED, the crates ci/release/reuse.sh found a hosted
# run passed on this tree, separated by spaces (#3384).
set -euo pipefail

part=${1:?usage: ci/release/cell.sh build|tools|size|rust|lazyfs|typescript|python|rss}
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
  tools)
    cargo run --quiet --locked --bin xtask -- test-tools build --cargo-profile ci-test \
      --target "$target" --out "$GOODFETCH_TEST_TOOLS"
    cargo run --quiet --locked --bin xtask -- quarantine exclude >"$exclusions"
    ;;
  size)
    tree=$(git rev-parse 'HEAD^{tree}')
    if [[ -n "${GITHUB_OUTPUT:-}" ]]; then
      echo "tree=$tree" >>"$GITHUB_OUTPUT"
      if [[ "${GITHUB_EVENT_NAME:-}" == "merge_group" || "${GITHUB_REF:-}" == "refs/heads/main" ]]; then
        echo "save=true" >>"$GITHUB_OUTPUT"
      else
        echo "save=false" >>"$GITHUB_OUTPUT"
      fi
    fi
    mkdir -p target/release-suite/archives target/release-suite/crates target/release-suite/oci
    suffix=""
    [[ "$target" == *-windows-* ]] && suffix=.exe
    for product in collie foxhound; do
      exe="target/$target/ci/goodfetch-$product$suffix"
      if [[ -f "$exe" ]]; then
        tar -cJf "target/release-suite/archives/goodfetch-$product-$target.tar.xz" \
          -C "target/$target/ci" "goodfetch-$product$suffix"
      fi
    done
    size_mode=${GOODFETCH_SIZE_MODE:-estimate}
    if [[ "$size_mode" == "increase" ]]; then
      # Package published crates once on linux-x64-gnu for the 8 MiB ceiling check (ADR 0057).
      if [[ "$cell" == "linux-x64-gnu" ]]; then
        for pkg in goodfetch-collie-client goodfetch-collie goodfetch-foxhound; do
          cargo package --locked --no-verify --allow-dirty -p "$pkg" --target-dir target/release-suite/package-crates >/dev/null
        done
        cp target/release-suite/package-crates/package/*.crate target/release-suite/crates/
      fi
      # Export both Products' 5 OCI image tag manifests on Linux x64 and arm64 (+3 MB check, issue 204).
      if [[ "$target" == *-unknown-linux-musl ]]; then
        arch="amd64"
        [[ "$target" == aarch64-* ]] && arch="arm64"
        for product in collie foxhound; do
          for tag in default slim full node python; do
            oci_tar="target/release-suite/oci/$product-$tag.oci.tar"
            docker buildx build \
              --platform "linux/$arch" \
              --file packaging/oci/Containerfile \
              --target "$tag" \
              --build-arg "PRODUCT_EXECUTABLE=goodfetch-$product" \
              --build-arg "BINARY_DIR=target/$target/ci" \
              --output "type=oci,dest=$oci_tar" \
              .
            # Extract the image manifest JSON from the OCI layout archive for `xtask size`.
            tmp_oci="target/release-suite/oci/extract-$product-$tag"
            rm -rf "$tmp_oci" && mkdir -p "$tmp_oci"
            tar -xf "$oci_tar" -C "$tmp_oci"
            digest=$(jq -r '.manifests[0].digest | sub("^sha256:"; "")' "$tmp_oci/index.json")
            cp "$tmp_oci/blobs/sha256/$digest" "target/release-suite/oci/$product-$tag.manifest.json"
            rm -rf "$tmp_oci" "$oci_tar"
          done
        done
      fi
    fi
    ci/release/size-manifest.sh "$target" >target/release-suite/size-files.toml
    size_args=(--files target/release-suite/size-files.toml --save "target/release-suite/size-report-$tree-$target.json")
    if [[ "$size_mode" == "increase" ]]; then
      if [[ -n "${GOODFETCH_SIZE_MAIN_REPORT:-}" && -f "$GOODFETCH_SIZE_MAIN_REPORT" ]]; then
        size_args+=(--main-report "$GOODFETCH_SIZE_MAIN_REPORT" --main origin/main)
      fi
      if [[ -n "${GOODFETCH_SIZE_PR:-}" ]]; then
        size_args+=(--pull-request "$GOODFETCH_SIZE_PR")
      fi
      cargo run --quiet --locked --bin xtask -- size increase "${size_args[@]}"
    else
      cargo run --quiet --locked --bin xtask -- size estimate "${size_args[@]}"
    fi
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
    # The crates whose Pre-merge unit a hosted run passed on this tree
    # (ci/release/reuse.sh, #3384) run neither their tests nor their
    # doctests again.
    reused=()
    filter=()
    excluded=()
    for krate in ${GOODFETCH_RELEASE_REUSED:-}; do
      reused+=("package($krate)")
      excluded+=(--exclude "$krate")
    done
    if [[ ${#reused[@]} -gt 0 ]]; then
      joined=$(printf ' | %s' "${reused[@]}")
      filter=(-E "not (${joined:3})")
      echo "Release: skipping the tests of the crates a hosted run passed on this tree: ${GOODFETCH_RELEASE_REUSED}"
    fi
    cargo nextest run --archive-file "$archive" --workspace-remap . \
      --profile ci --partition "hash:$shard" ${filter[@]+"${filter[@]}"} ${skipped[@]+"${skipped[@]}"} || status=$?
    [[ -f target/nextest/ci/junit.xml ]] && cp target/nextest/ci/junit.xml "target/junit/nextest-$cell.xml"
    # nextest's archive holds no doctests; the first shard runs them.
    if [[ "$shard" == 1/* ]]; then
      cargo test --workspace ${excluded[@]+"${excluded[@]}"} --locked --doc --target "$target" --profile ci-test || status=$?
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
      # A free-threaded line leaves out LangChain, as ci/host/fetch.sh
      # installed it (issue 3738).
      groups=()
      case "$python" in *t) groups=(--no-group langchain) ;; esac
      uv run --frozen --python "$python" ${groups[@]+"${groups[@]}"} pytest -n auto "${pytest_order[@]}" \
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
    echo "error: the part is build, tools, size, rust, lazyfs, typescript, python or rss, not $part" >&2
    exit 2
    ;;
esac
