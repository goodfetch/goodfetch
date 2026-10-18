#!/usr/bin/env bash
# The constrained test-health run's tests (ADR 0064): the Pre-merge suite on
# hosted Linux under a cgroup that halves the runner's CPU and memory, with a
# fresh order seed (GOODFETCH_ORDER_SEED, which the workflow draws and
# records) and Vitest's `detectAsyncLeaks`. pytest-randomly takes the same
# seed; nextest runs each test in its own process and has no order.
set -euo pipefail

seed=${GOODFETCH_ORDER_SEED:?the workflow draws a fresh order seed}
cpus=$(nproc)
memory_kib=$(awk '/^MemTotal:/ { print $2 }' /proc/meminfo)

# systemd-run --scope runs the suite as this user, in this environment, in a
# transient scope whose limits are half the runner's.
exec sudo env "PATH=$PATH" "HOME=$HOME" \
  "GOODFETCH_ORDER_SEED=$seed" GOODFETCH_DETECT_ASYNC_LEAKS=1 \
  "PYTEST_ADDOPTS=-p randomly -p no:cacheprovider --randomly-seed=$seed" CI=true \
  systemd-run --scope --quiet --same-dir \
  --uid="$(id -u)" --gid="$(id -g)" \
  --property="CPUQuota=$((cpus * 50))%" \
  --property="MemoryMax=$((memory_kib / 2))K" \
  -- just test pre-merge
