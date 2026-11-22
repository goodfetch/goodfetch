#!/usr/bin/env bash
# Shared by ci/egress/egress.sh and ci/egress/under-deny.sh: which form of
# ADR 0043's deny the job takes, and where its state lives. Sourced, never run.

host_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)

# Written once the deny is in place on a runner's own system, holding the
# job's name; under-deny.sh refuses to run a step before it exists.
DENY_STATE="$host_root/target/egress/denied"

# The hosted Linux runner's network namespace and its link to the runner.
NAMESPACE=goodfetch-egress
HOST_LINK=gf-egress-host
JOB_LINK=gf-egress-job
HOST_ADDRESS=10.203.0.1
JOB_ADDRESS=10.203.0.2

# The owner's machine's pull-request jobs run in a Lima VM (ADR 0135); the
# Release suite's macOS cells run on macOS itself (ADR 0108).
if [[ "$(uname -s)" == Darwin ]]; then
  # shellcheck source=ci/vm/common.sh
  source "$host_root/ci/vm/common.sh"
fi

# vm, namespace or windows.
deny_backend() {
  case "${RUNNER_OS:-$(uname -s)}" in
    Linux) echo namespace ;;
    Windows | MINGW* | MSYS*) echo windows ;;
    macOS | Darwin)
      if command -v limactl >/dev/null && limactl list --quiet 2>/dev/null | grep -qx "$VM_NAME"; then
        echo vm
      else
        echo "error: no deny on macOS without the job's VM; such a job is declared outside it (ADR 0043's amendment of 2026-11-22)" >&2
        return 2
      fi
      ;;
    *)
      echo "error: no deny for ${RUNNER_OS:-$(uname -s)}" >&2
      return 2
      ;;
  esac
}

export DENY_STATE NAMESPACE HOST_LINK JOB_LINK HOST_ADDRESS JOB_ADDRESS
