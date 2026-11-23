#!/usr/bin/env bash
# Runs a command of a hosted tier job in its container, in the tree's pinned
# Linux x86-64 CI image (ADR 0135's amendment of 2026-11-22):
# `ci/hosted/exec.sh <command>…`. The command sees the workspace at /work.
# Only CI, the job's identity, the gate's own variables (the layer, the one
# step the job runs, where the records are, the repository and the runner
# image its fingerprint names) and, when the step sets it, GITHUB_TOKEN reach
# it: a token with the read scopes the job grants, never a write scope or a
# secret. Once ci/egress/egress.sh has denied the container's outbound
# traffic (ADR 0043), Cargo, uv and pnpm run offline, from what
# `just fetch` installed.
set -euo pipefail

here=$(cd "$(dirname "$0")" && pwd)
# shellcheck source=ci/hosted/common.sh
source "$here/common.sh"

passed=(GITHUB_REPOSITORY GITHUB_RUN_ID GITHUB_RUN_ATTEMPT GITHUB_SHA GITHUB_REF
  GOODFETCH_GATE_LAYER GOODFETCH_GATE_ONLY GOODFETCH_GATE_RECORDS
  GOODFETCH_GATE_HOSTED_RECORDS GOODFETCH_GATE_REPOSITORY GOODFETCH_GATE_RUNNER_IMAGE)
forwarded=(--env CI=true)
for name in "${passed[@]}"; do
  [[ -n "${!name:-}" ]] && forwarded+=(--env "$name")
done
if [[ -f "$EGRESS_STATE" ]]; then
  forwarded+=(--env CARGO_NET_OFFLINE=true --env UV_OFFLINE=1 --env UV_FROZEN=1 --env npm_config_offline=true)
fi

# The token never goes on a command line, which every process on the runner
# can read: it reaches the container through an environment file in the
# workspace, readable by the runner's user only and removed when the step
# ends.
if [[ -n "${GITHUB_TOKEN:-}" ]]; then
  workspace=${GITHUB_WORKSPACE:-$(pwd)}
  env_file="target/exec-env.$$"
  mkdir -p "$workspace/target"
  trap 'rm -f "$workspace/$env_file"' EXIT
  (umask 077 && printf 'GITHUB_TOKEN=%s\n' "$GITHUB_TOKEN" >"$workspace/$env_file")
  forwarded+=(--env-file "$workspace/$env_file")
fi

docker exec "${forwarded[@]}" "$CONTAINER" /work/ci/vm/in-container.sh run "$@"
