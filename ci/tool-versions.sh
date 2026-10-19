#!/usr/bin/env bash
# Checks that every tool pinned in more than one place is pinned at one
# version (`just lint`): the CI image's (ci/image/Containerfile), the hosted
# jobs' host-tools action, ci/host/runtimes.env, package.json's
# packageManager and the workspace's gungraun, which gungraun's runner must
# match. The image's tag hashes only its Containerfile, so a version changed
# elsewhere alone would leave the image and the hosted jobs apart.
set -euo pipefail

root=$(cd "$(dirname "$0")/.." && pwd)
containerfile="$root/ci/image/Containerfile"
action="$root/.github/actions/host-tools/action.yml"

failed=0
agree() {
  local tool=$1 first=$2 second=$3 where=$4
  if [[ -z "$first" || -z "$second" ]]; then
    echo "error[tool-versions] $tool: no version found in $where"
    failed=1
  elif [[ "$first" != "$second" ]]; then
    echo "error[tool-versions] $tool: $first and $second in $where"
    failed=1
  fi
}

# `cargo install … <tool>@<version>` in the image; `tool: …,<tool>@<version>` in the action.
image_tool() { grep -oE "(^|[[:space:]])$1@[0-9][0-9.]*" "$containerfile" | head -n1 | sed 's/.*@//'; }
action_tool() { grep -E '^ *tool: ' "$action" | grep -oE "(^|[ ,])$1@[0-9][0-9.]*" | head -n1 | sed 's/.*@//'; }
image_arg() { sed -n "s/^ARG $1=//p" "$containerfile"; }

for tool in just cargo-nextest cargo-insta; do
  agree "$tool" "$(image_tool "$tool")" "$(action_tool "$tool")" "the CI image and the host-tools action"
done
agree uv "$(image_arg UV_VERSION)" "$(action_tool uv)" "the CI image and the host-tools action"
agree node "$(image_arg NODE_VERSION)" "$(sed -n 's/^NODE_24=//p' "$root/ci/host/runtimes.env")" \
  "the CI image and ci/host/runtimes.env"
agree pnpm "$(image_arg PNPM_VERSION)" "$(sed -n 's/^  "packageManager": "pnpm@\(.*\)"$/\1/p' "$root/package.json")" \
  "the CI image and package.json"
agree mold "$(image_arg MOLD_VERSION)" "$(sed -n 's/^ *mold-version: //p' "$action")" \
  "the CI image and the host-tools action"
agree gungraun "$(image_tool gungraun-runner)" "$(sed -n 's/^gungraun = { version = "=\([0-9.]*\)".*/\1/p' "$root/Cargo.toml")" \
  "the CI image's gungraun-runner and the workspace's gungraun"

exit "$failed"
