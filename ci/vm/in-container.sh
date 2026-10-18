#!/usr/bin/env bash
# Runs inside the job's container in its VM (ADR 0135), in one of two modes:
#   prepare  once, when start.sh starts the container: restores the build
#            cache from /cache, then makes every download the job needs
#            (`just fetch`), all before ADR 0043's deny;
#   run      for each step (exec.sh): runs the step's command and copies the
#            JUnit files where the host uploads them. In the cache-warming
#            job (/cache writable) it then saves the cache; no other job
#            writes it.
set -uo pipefail

mode=${1:?usage: in-container.sh prepare|run <command>…}
shift
cd /work

# The cache folder holds Cargo's registry and the build's target folder, the
# pnpm store and uv's cache, each saved whole by the cache-warming job.
# Pull-request jobs copy what is there into the container's own disk.
export CARGO_TARGET_DIR=/var/tmp/target
export npm_config_store_dir=/var/tmp/pnpm-store
export UV_CACHE_DIR=/var/tmp/uv-cache

if [[ "$mode" == prepare ]]; then
  set -e
  mkdir -p "$CARGO_TARGET_DIR" "$npm_config_store_dir" "$UV_CACHE_DIR"
  if [[ -d /cache/cargo-registry ]]; then cp -a /cache/cargo-registry "$CARGO_HOME/registry"; fi
  if [[ -d /cache/target ]]; then cp -a /cache/target/. "$CARGO_TARGET_DIR/"; fi
  if [[ -d /cache/pnpm-store ]]; then cp -a /cache/pnpm-store/. "$npm_config_store_dir/"; fi
  if [[ -d /cache/uv ]]; then cp -a /cache/uv/. "$UV_CACHE_DIR/"; fi
  just fetch
  exit 0
fi

[[ "$mode" == run ]] || {
  echo "error: the mode is prepare or run, not $mode" >&2
  exit 2
}

"$@"
status=$?

# Every runner's JUnit XML reaches target/junit/ in the workspace, which the
# host uploads pass or fail.
mkdir -p target/junit
for report in "$CARGO_TARGET_DIR"/nextest/*/junit.xml; do
  [[ -f "$report" ]] || continue
  profile=$(basename "$(dirname "$report")")
  cp "$report" "target/junit/nextest-$profile.xml"
done

if [[ "${GOODFETCH_CACHE_MODE:-}" == writable && $status -eq 0 ]]; then
  # Each folder is replaced whole, through a temporary name, so a failed save
  # never leaves a half-written cache behind.
  save() {
    rm -rf "/cache/$2.partial"
    cp -a "$1" "/cache/$2.partial"
    rm -rf "/cache/$2"
    mv "/cache/$2.partial" "/cache/$2"
  }
  save "$CARGO_HOME/registry" cargo-registry
  save "$CARGO_TARGET_DIR" target
  save "$npm_config_store_dir" pnpm-store
  save "$UV_CACHE_DIR" uv
fi

exit "$status"
