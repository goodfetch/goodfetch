#!/usr/bin/env bash
# `just windows`: the workspace built for x86_64-pc-windows-msvc and its
# tests run under Wine, on a Linux host, while GitHub's hosted runners are
# refused (issue 2767). It runs in the image of ci/windows/Containerfile,
# through Docker or Podman, so the host's packages, which the gate's host
# fingerprint hashes, stay as they are. In the image:
#   - cargo-xwin sets up the compiler, linker and the Microsoft CRT and
#     Windows SDK at the versions pinned below, and its environment drives
#     Cargo, nextest and xtask alike;
#   - clippy over the workspace, every target and feature, `-D warnings`;
#   - the test tools built for Windows, and the Pre-merge suite's Rust tests
#     run by nextest under Wine, but the tests ci/windows/skips.toml lists
#     and those on the test quarantine list (ADR 0064).
# What Wine does not prove, docs/testing/suites-commands-and-time-budgets.md
# says (`just windows`).
#
# The build folder is `<the build folder>/windows/`, which no other step
# reads. The CRT and SDK, which cargo-xwin downloads once, stay in
# `GOODFETCH_XWIN_CACHE` (`~/.cache/goodfetch/xwin` by default; a link to
# the volume on the gate host), outside every build folder. Downloading
# them accepts Microsoft's licence for them,
# https://go.microsoft.com/fwlink/?LinkId=2086102, which the owner accepted
# on 2026-11-17 for building and testing under Wine alone, with no
# redistribution (issue 2767).
#
# It takes the whole workspace, as the full gate runs it, or the packages
# named as its arguments (`just windows goodfetch-engine goodfetch-page`),
# whose build is smaller where disk is short.
#
# Without a container engine it lists itself as skipped with the reason
# (ADR 0061), as ci/linux.sh does.
set -euo pipefail

root=$(cd "$(dirname "$0")/.." && pwd)
cd "$root"

# shellcheck source=ci/container.sh
source "$root/ci/container.sh"

target=x86_64-pc-windows-msvc

if [[ "${1:-}" == in-container ]]; then
  shift
  packages=(--workspace)
  [[ $# -eq 0 ]] || packages=()
  for package in "$@"; do packages+=(-p "$package"); done
  # The Visual Studio 17 manifest's CRT 14.44 (14.44.35220) and the
  # Windows 11 SDK 10.0.26100 (10.0.26100.18), as read on 2026-11-18.
  export XWIN_VERSION=17 XWIN_CRT_VERSION=14.44.17.14 XWIN_SDK_VERSION=10.0.26100
  eval "$(cargo xwin env --target "$target")"
  echo "$(wine --version), $(cargo xwin --version), CRT $XWIN_CRT_VERSION, SDK $XWIN_SDK_VERSION"
  status=0
  cargo clippy --locked --target "$target" "${packages[@]}" --all-targets --all-features -- -D warnings || status=$?

  export GOODFETCH_TEST_SUITE=pre-merge
  # insta finds the workspace by running `cargo metadata`, which a test under
  # Wine cannot start: Linux's Cargo is no Windows program. The root is Wine's
  # drive path of the checkout: a Windows program reads a path with no drive
  # as relative, and wrote a unit test's snapshots under a doubled folder.
  # A failing snapshot writes no `.snap.new` into the checkout, which the
  # Linux suites would then read as one to review.
  export INSTA_WORKSPACE_ROOT="Z:$PWD" INSTA_UPDATE=no
  export GOODFETCH_TEST_TOOLS="$CARGO_TARGET_DIR/test-tools.json"
  # xtask builds with the tests' profile, so it shares their host-built
  # dependencies.
  xtask=(cargo run --quiet --locked --profile ci-test --bin xtask --)
  "${xtask[@]}" test-tools build --cargo-profile ci-test \
    --target "$target" --out "$GOODFETCH_TEST_TOOLS"
  skips=()
  exclusions=$("${xtask[@]}" quarantine exclude)
  quarantined=$(jq -r '.nextest[]' <<<"$exclusions")
  while IFS= read -r argument; do
    [[ -z "$argument" ]] || skips+=("$argument")
  done <<<"$quarantined"
  # Wine's session starts here, apart from every test, and lasts the run: a
  # test whose `wine` boots it leaves wineserver and Wine's services (explorer,
  # services, winedevice) holding the test's output after it exits, and nextest
  # fails it as leaky.
  wineserver -p </dev/null >/dev/null 2>&1
  wine wineboot </dev/null >/dev/null 2>&1
  nextest=(--locked --target "$target" "${packages[@]}" --profile ci --cargo-profile ci-test
    --tool-config-file "goodfetch-windows:$PWD/ci/windows/nextest.toml")
  filter=()
  if [[ -n "$GOODFETCH_WINE_SKIPS" ]]; then
    echo "The tests skipped under Wine:"
    cargo nextest list "${nextest[@]}" -E "$GOODFETCH_WINE_SKIPS"
    filter=(-E "not ($GOODFETCH_WINE_SKIPS)")
  fi
  cargo nextest run "${nextest[@]}" ${filter[@]+"${filter[@]}"} ${skips[@]+"${skips[@]}"} || status=$?
  exit "$status"
fi

engine=$(container_engine)
[[ -n "$engine" ]] || skip "just windows" "it builds and tests in a container, and no container engine (Docker or Podman) is running on this host"

image="goodfetch-windows:$(shasum -a 256 ci/windows/Containerfile | cut -c1-16)"
if ! "$engine" image inspect "$image" >/dev/null 2>&1; then
  echo "Building the Windows image $image once (ci/windows/Containerfile)…" >&2
  "$engine" build --platform linux/amd64 --tag "$image" --file ci/windows/Containerfile ci/windows
fi

# The skipped tests, each entry's filterset joined into one, and each entry
# printed with its reason and issue.
read_skips() { uv run --frozen --quiet python -c "import tomllib; skips = tomllib.load(open('ci/windows/skips.toml', 'rb')).get('skip', []); $1"; }
read_skips 'print("\n".join("Skipped under Wine, issue " + str(s["issue"]) + ": " + s["reason"] for s in skips))'
wine_skips=$(read_skips 'print(" | ".join("(" + s["filter"] + ")" for s in skips))')

build="${CARGO_TARGET_DIR:-$root/target}/windows"
cache="${GOODFETCH_XWIN_CACHE:-${XDG_CACHE_HOME:-$HOME/.cache}/goodfetch/xwin}"
mkdir -p "$build" "$cache"
# As ci/linux.sh mounts it, with Cargo's registry in a volume of its own.
mounts=(--volume "$root:$root" --volume "$build:/var/tmp/target" --volume "$cache:/root/.cache/cargo-xwin")
common=$(git rev-parse --path-format=absolute --git-common-dir)
[[ "$common" == "$root"/* ]] || mounts+=(--volume "$common:$common")
environment=(--env GIT_CONFIG_COUNT=1 --env GIT_CONFIG_KEY_0=safe.directory --env GIT_CONFIG_VALUE_0='*'
  --env CARGO_TARGET_DIR=/var/tmp/target --env CARGO_INCREMENTAL=0 --env "GOODFETCH_WINE_SKIPS=$wine_skips")
[[ -z "${CARGO_BUILD_JOBS:-}" ]] || environment+=(--env "CARGO_BUILD_JOBS=$CARGO_BUILD_JOBS")
# Windows' copies of the pinned Toxiproxy and Claude Code, which tests under
# Wine start, fetched beside this machine's (`just fetch`) and read through
# Wine's drive path; they test nothing, so their fetch may retry (ADR 0064).
toxiproxy="${GOODFETCH_TOXIPROXY_DOWNLOADS:-$HOME/.cache/goodfetch/toxiproxy}"
claude_code="${GOODFETCH_CLAUDE_CODE_DOWNLOADS:-$HOME/.cache/goodfetch/claude-code}"
ci/toxiproxy-fetch.sh windows-x64
ci/claude-code-fetch.sh win32-x64
mounts+=(--volume "$toxiproxy:$toxiproxy:ro" --volume "$claude_code:$claude_code:ro")
environment+=(--env "GOODFETCH_TOXIPROXY_DOWNLOADS=Z:$toxiproxy" --env "GOODFETCH_CLAUDE_CODE_DOWNLOADS=Z:$claude_code")
exec "$engine" run --rm --init --platform linux/amd64 "${mounts[@]}" --workdir "$root" \
  --volume goodfetch-windows-cargo:/usr/local/cargo/registry \
  "${environment[@]}" "$image" ci/windows.sh in-container "$@"
