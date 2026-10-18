#!/usr/bin/env bash
# Runs a command of the job in its VM, inside the Linux x86-64 CI image
# (ADR 0135): `ci/vm/exec.sh <command>…`. The command sees the workspace at
# /work and the cache folder at /cache, mounted as start.sh chose. Only CI,
# the job's identity and, when the step sets it, GITHUB_TOKEN reach it.
set -euo pipefail

here=$(cd "$(dirname "$0")" && pwd)
# shellcheck source=ci/vm/common.sh
source "$here/common.sh"

cache_mode=$(cat "$VM_STATE")
cache_flag=ro
[[ "$cache_mode" == writable ]] && cache_flag=rw

forwarded=(--env CI=true)
for name in GITHUB_TOKEN GITHUB_REPOSITORY GITHUB_RUN_ID GITHUB_RUN_ATTEMPT GITHUB_SHA GITHUB_REF; do
  [[ -n "${!name:-}" ]] && forwarded+=(--env "$name")
done

# `limactl shell` passes the host's environment of the named variables only
# through the command line, so they are exported into the VM's command.
exports=()
for name in GITHUB_TOKEN GITHUB_REPOSITORY GITHUB_RUN_ID GITHUB_RUN_ATTEMPT GITHUB_SHA GITHUB_REF; do
  [[ -n "${!name:-}" ]] && exports+=("$name=${!name}")
done

vm_shell env "${exports[@]}" nerdctl run --rm --platform linux/amd64 \
  --volume /work:/work --volume "/cache:/cache:$cache_flag" \
  "${forwarded[@]}" --env "GOODFETCH_CACHE_MODE=$cache_mode" \
  "$CI_IMAGE" /work/ci/vm/in-container.sh "$@"
