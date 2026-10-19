#!/usr/bin/env bash
# Runs a command of the job in its container, inside its VM, in the Linux
# x86-64 CI image (ADR 0135): `ci/vm/exec.sh <command>…`. The command sees
# the workspace at /work and the cache folder at /cache, mounted as start.sh
# chose. Only CI, the job's identity and, when the step sets it, GITHUB_TOKEN
# reach it: a token with the read scopes ADR 0135 names, never a write scope
# or a secret. Once ci/egress/egress.sh has denied outbound traffic (ADR
# 0043), Cargo, uv and pnpm run offline, from what `just fetch` installed.
set -euo pipefail

here=$(cd "$(dirname "$0")" && pwd)
# shellcheck source=ci/vm/common.sh
source "$here/common.sh"

identity=(GITHUB_REPOSITORY GITHUB_RUN_ID GITHUB_RUN_ATTEMPT GITHUB_SHA GITHUB_REF)
forwarded=(--env CI=true)
for name in "${identity[@]}"; do
  [[ -n "${!name:-}" ]] && forwarded+=(--env "$name")
done
if [[ -f "$EGRESS_STATE" ]]; then
  forwarded+=(--env CARGO_NET_OFFLINE=true --env UV_OFFLINE=1 --env UV_FROZEN=1 --env npm_config_offline=true)
fi

# `limactl shell` passes the host's environment of the named variables only
# through the command line, so they are exported into the VM's command.
exports=(CI=true)
for name in "${identity[@]}"; do
  [[ -n "${!name:-}" ]] && exports+=("$name=${!name}")
done

# The token never goes on a command line, which every process on the host
# and in the VM can read: it reaches the container through an environment
# file in the workspace, readable by the runner's user only and removed when
# the step ends.
if [[ -n "${GITHUB_TOKEN:-}" ]]; then
  workspace=${GITHUB_WORKSPACE:-$(pwd)}
  env_file="target/exec-env.$$"
  mkdir -p "$workspace/target"
  trap 'rm -f "$workspace/$env_file"' EXIT
  (umask 077 && printf 'GITHUB_TOKEN=%s\n' "$GITHUB_TOKEN" >"$workspace/$env_file")
  forwarded+=(--env-file "/work/$env_file")
fi

vm_shell env "${exports[@]}" nerdctl exec "${forwarded[@]}" "$CONTAINER" \
  /work/ci/vm/in-container.sh run "$@"
