#!/usr/bin/env bash
# The constrained test-health run's tests (ADR 0064): the Pre-merge suite on
# hosted Linux under a cgroup that halves the runner's CPU and memory, with a
# fresh order seed (GOODFETCH_ORDER_SEED, which the workflow draws and
# records) and Vitest's `detectAsyncLeaks`; `just test` hands the seed to
# Vitest, Playwright and pytest-randomly, and nextest runs each test in its
# own process and has no order.
set -euo pipefail

seed=${GOODFETCH_ORDER_SEED:?the workflow draws a fresh order seed}
# The suite's first step checks the OS services, after its build: without
# these it fails there, so the run stops here.
bus=${DBUS_SESSION_BUS_ADDRESS:?ci/os-services.sh setup names the session bus}
runtime=${XDG_RUNTIME_DIR:?ci/os-services.sh setup names the runtime folder}
cpus=$(nproc)
memory_kib=$(awk '/^MemTotal:/ { print $2 }' /proc/meminfo)

# systemd-run --scope runs the suite as this user, in a transient scope whose
# limits are half the runner's. sudo resets the environment, so the suite gets
# only what is named here: the OS services' variables, and, when the caller
# set them, the DNS namespace's declared way (TESTING.md, `DnsNamespace`) and
# the temporary folder, since a test's files in a tmpfs count against the
# memory limit. Set empty, the first would fail every namespace's test.
exec sudo env "PATH=$PATH" "HOME=$HOME" \
  "DBUS_SESSION_BUS_ADDRESS=$bus" "XDG_RUNTIME_DIR=$runtime" \
  ${GOODFETCH_TEST_DNS_NAMESPACE:+"GOODFETCH_TEST_DNS_NAMESPACE=$GOODFETCH_TEST_DNS_NAMESPACE"} \
  ${TMPDIR:+"TMPDIR=$TMPDIR"} \
  "GOODFETCH_ORDER_SEED=$seed" GOODFETCH_DETECT_ASYNC_LEAKS=1 \
  "PYTEST_ADDOPTS=-p no:cacheprovider" CI=true \
  systemd-run --scope --quiet --same-dir \
  --uid="$(id -u)" --gid="$(id -g)" \
  --property="CPUQuota=$((cpus * 50))%" \
  --property="MemoryMax=$((memory_kib / 2))K" \
  -- just test pre-merge
