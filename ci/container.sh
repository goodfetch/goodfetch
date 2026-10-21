#!/usr/bin/env bash
# Shared by the scripts that run a part of the Pre-merge suite in a Linux
# container on a host that is not Linux (ci/perf.sh, ci/linux.sh,
# ci/visual/container.sh). Sourced, never run.

# Lists the part as skipped with the reason, and ends the script (ADR 0061),
# as ci/tool.sh does; in CI (`CI` set) a missing tool fails.
#   skip <part> <reason>
skip() {
  if [[ -n "${CI:-}" ]]; then
    echo "error: $2, and CI runs every part" >&2
    exit 1
  fi
  mkdir -p target
  echo "$1: skipped, $2" >>target/skipped.txt
  echo "skipped: $1 ($2)" >&2
  exit 0
}

# Prints the container engine that answers on this host, Docker or Podman;
# nothing when neither does.
container_engine() {
  local candidate
  for candidate in docker podman; do
    if command -v "$candidate" >/dev/null 2>&1 && "$candidate" info >/dev/null 2>&1; then
      echo "$candidate"
      return
    fi
  done
}
