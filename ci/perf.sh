#!/usr/bin/env bash
# `just perf [base]` (ADR 0071): `xtask perf` over the bench targets
# `xtask affected` selects, against `base` (the merge base with
# `origin/main` by default), with CI's thresholds.
#
#   - On Linux it runs natively, and needs Valgrind and gungraun-runner.
#   - Elsewhere it runs this script in the pinned Linux x86-64 CI image
#     (ci/image/Containerfile, built once under its hash tag) through Docker
#     or Podman, with the workspace mounted at its own path; Valgrind
#     supports Linux arm64, so an Apple-silicon machine runs it emulated.
#   - Without a container engine, or locally on Linux without the tools, it
#     lists itself as skipped with the reason (ADR 0061), as ci/tool.sh does;
#     in CI (`CI` set) a missing tool fails.
set -euo pipefail

root=$(cd "$(dirname "$0")/.." && pwd)
cd "$root"

skip() {
  if [[ -n "${CI:-}" ]]; then
    echo "error: $1, and CI runs every part" >&2
    exit 1
  fi
  mkdir -p target
  echo "just perf: skipped, $1" >>target/skipped.txt
  echo "skipped: just perf ($1)" >&2
  exit 0
}

requested=${1:-}
base=$requested
change=(--local)
if [[ -n "$base" ]]; then
  change=(--base "$base" --head HEAD)
else
  base=$(git merge-base origin/main HEAD)
fi

if [[ "$(uname -s)" != Linux ]]; then
  engine=
  for candidate in docker podman; do
    if command -v "$candidate" >/dev/null 2>&1 && "$candidate" info >/dev/null 2>&1; then
      engine=$candidate
      break
    fi
  done
  [[ -n "$engine" ]] || skip "it runs on Linux, and no container engine (Docker or Podman) is running on this host"
  # shellcheck source=ci/vm/common.sh
  source "$root/ci/vm/common.sh"
  if ! "$engine" image inspect "$CI_IMAGE" >/dev/null 2>&1; then
    echo "Building the CI image $CI_IMAGE once (ci/image/Containerfile)…" >&2
    "$engine" build --platform linux/amd64 --tag "$CI_IMAGE" --file ci/image/Containerfile ci/image
  fi
  # The workspace at its own path, so a worktree's `.git` file still points
  # at its repository; Linux build outputs and Cargo's registry in volumes of
  # their own, never in the host's target/.
  mounts=(--volume "$root:$root")
  common=$(git rev-parse --path-format=absolute --git-common-dir)
  [[ "$common" == "$root"/* ]] || mounts+=(--volume "$common:$common")
  # `--init`, so stopping this command (as `just pre-merge` does when another
  # step fails) stops everything in the container.
  exec "$engine" run --rm --init --platform linux/amd64 "${mounts[@]}" --workdir "$root" \
    --volume goodfetch-perf-target:/var/tmp/target --env CARGO_TARGET_DIR=/var/tmp/target \
    --volume goodfetch-perf-cargo:/usr/local/cargo/registry \
    --env GIT_CONFIG_COUNT=1 --env GIT_CONFIG_KEY_0=safe.directory --env GIT_CONFIG_VALUE_0='*' \
    "$CI_IMAGE" ci/perf.sh ${requested:+"$requested"}
fi

command -v valgrind >/dev/null 2>&1 || skip "Valgrind is not installed on this host"
command -v gungraun-runner >/dev/null 2>&1 || skip "gungraun-runner is not installed on this host"

selection=$(cargo run --quiet -p xtask -- affected "${change[@]}")
targets=$(sed -n 's/^bench-targets=//p' <<<"$selection")
cargo run --quiet -p xtask -- perf build --base "$base" --bench-targets "$targets"
cargo run --quiet -p xtask -- perf run
