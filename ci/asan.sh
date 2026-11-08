#!/usr/bin/env bash
# The AddressSanitizer run of the Release suite (ADR 0067; TESTING.md,
# Fuzzing): the tests of every crate that opts into `unsafe` (STYLE.md,
# `unsafe`), built with `-Zsanitizer=address` on the pinned nightly of
# ci/sanitizer-toolchain.toml, the standard library rebuilt with it
# (`-Zbuild-std`, since the sanitizer must match across every crate). It
# covers the FFI and system-call modules Miri cannot run. CI runs it on
# Linux glibc (the linux-x64-gnu cell); locally it runs for this host.
#
#   ci/asan.sh build   builds the tests, before the egress deny;
#   ci/asan.sh test    runs them through nextest's `ci` profile, under it,
#                      its JUnit file going to target/junit/nextest-asan.xml;
#                      `just asan` gives them the test tool manifest.
#
# A crate opts in when a file of its `src/` carries `#![expect(unsafe_code,
# …)]`. The run takes the crate's unit tests, which run in their test
# process, and its integration tests in a module named after a module on the
# opted-in file's path (`src/rss/peak.rs`: `rss::` and `peak::`). The other
# integration tests mostly drive the crate's own executables, which this
# build sanitizes too and whose output they compare byte for byte: ASan's
# own notes on stderr, and the sanitized build's paths, would fail them for
# no fault of the code. Leak detection is off: the run looks for memory
# errors, and a test's deliberate leak (a `Box::leak`, a static) is none.
set -euo pipefail

root=$(cd "$(dirname "$0")/.." && pwd)
cd "$root"

pinned() { sed -n "s/^$1 = \"\(.*\)\"\$/\1/p" ci/sanitizer-toolchain.toml; }
toolchain=$(pinned toolchain)
# As ci/tool.sh does for a tool: a local host without the pinned toolchain
# lists the run as skipped; in CI it fails.
if ! rustup toolchain list | grep -q "^$toolchain"; then
  if [[ -n "${CI:-}" ]]; then
    echo "error: the pinned toolchain $toolchain is not installed, and CI runs every part" >&2
    exit 1
  fi
  mkdir -p target
  echo "ci/asan.sh: skipped, the pinned toolchain $toolchain is not installed on this host" >>target/skipped.txt
  echo "skipped: ci/asan.sh ($toolchain is not installed on this host)" >&2
  exit 0
fi
host=$(rustc "+$toolchain" -vV | sed -n 's/^host: //p')

# The workspace's packages, as "<manifest folder>\t<name>" lines.
packages=$(cargo metadata --locked --offline --format-version 1 --no-deps |
  jq -r '.packages[] | "\(.manifest_path | rtrimstr("/Cargo.toml"))\t\(.name)"')

crates=() modules=()
while IFS= read -r file; do
  folder="$root/${file%/*}"
  # The innermost package whose folder holds the file.
  package=$(awk -F '\t' -v folder="$folder" 'index(folder "/", $1 "/") == 1 && length($1) > best { best = length($1); found = $0 } END { print found }' <<<"$packages")
  [[ -n "$package" ]] || continue
  name=${package#*$'\t'} manifest_folder=${package%%$'\t'*}
  # The file's module path under the package's src/; a file elsewhere (a
  # test, a build script) opts no module in.
  relative=${root}/${file}
  relative=${relative#"$manifest_folder/src/"}
  [[ "$relative" != "$root/$file" ]] || continue
  [[ " ${crates[*]} " == *" $name "* ]] || crates+=("$name")
  relative=${relative%.rs}
  relative=${relative%/mod}
  IFS=/ read -ra parts <<<"$relative"
  for part in "${parts[@]}"; do
    if [[ "$part" != lib && "$part" != main && " ${modules[*]} " != *" $part "* ]]; then
      modules+=("$part")
    fi
  done
done < <(git grep -l -E '(expect\(unsafe_code|^[[:space:]]*unsafe_code,)' -- '*.rs')

if [[ ${#crates[@]} -eq 0 ]]; then
  echo "No crate opts into unsafe: nothing to run under AddressSanitizer."
  exit 0
fi

arguments=()
for crate in "${crates[@]}"; do arguments+=(--package "$crate"); done
filter="kind(lib) | kind(bin)"
if [[ ${#modules[@]} -gt 0 ]]; then
  filter+=" | (kind(test) & test(/^($(IFS='|' && echo "${modules[*]}"))::/))"
fi
target_dir="$root/target/asan"
# Only the sanitized build takes these flags: xtask, which lists the
# quarantined tests, builds with the workspace's own toolchain.
sanitized() {
  RUSTFLAGS="-Zsanitizer=address" RUSTDOCFLAGS="-Zsanitizer=address" CARGO_UNSTABLE_BUILD_STD=std \
    CARGO_TARGET_DIR="$target_dir" ASAN_OPTIONS="detect_leaks=0:abort_on_error=1" \
    cargo "+$toolchain" nextest run --locked --profile ci --target "$host" "${arguments[@]}" "$@"
}
case "${1:-}" in
  build)
    echo "AddressSanitizer: building the tests of ${crates[*]} for $host"
    sanitized --no-run
    ;;
  test)
    echo "AddressSanitizer: testing ${crates[*]} on $host: $filter"
    # As `just test` does: quarantined tests (ADR 0064) and, on a local host,
    # the tests of an OS service it lacks are skipped (ci/nextest-skips.sh).
    skips=()
    exclusions=$(cargo run --quiet --bin xtask -- quarantine exclude)
    listed=$(ci/nextest-skips.sh "$exclusions")
    while IFS= read -r argument; do [[ -z "$argument" ]] || skips+=("$argument"); done <<<"$listed"
    mkdir -p target/junit
    # nextest writes its JUnit file under the workspace's target folder,
    # where the suite's own run left one, which this run replaces.
    report=target/nextest/ci/junit.xml
    rm -f "$report"
    status=0
    sanitized -E "$filter" ${skips[@]+"${skips[@]}"} || status=$?
    if [[ -f "$report" ]]; then
      mv "$report" target/junit/nextest-asan.xml
    fi
    exit "$status"
    ;;
  *)
    echo "usage: ci/asan.sh build|test" >&2
    exit 2
    ;;
esac
