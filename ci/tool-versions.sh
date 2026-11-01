#!/usr/bin/env bash
# Checks that every tool pinned in more than one place is pinned at one
# version (`just lint`): the CI image's (ci/image/Containerfile), the hosted
# jobs' host-tools action, ci/host/runtimes.env, package.json's
# packageManager, the pnpm catalog's Playwright, the workspace's gungraun,
# which gungraun's runner must match, the hosted jobs' gitleaks
# (ci/host/gitleaks.sh), the hosts' OSV-Scanner (ci/host/osv-scanner.sh)
# and the sanitizer toolchain and cargo-fuzz of
# ci/sanitizer-toolchain.toml. The image's tag hashes only its Containerfile, so a version changed
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
agree playwright "$(image_arg PLAYWRIGHT_VERSION)" "$(sed -n 's/^  playwright: //p' "$root/pnpm-workspace.yaml")" \
  "the CI image and the pnpm catalog"
agree gitleaks "$(image_arg GITLEAKS_VERSION)" "$(sed -n 's/^version=//p' "$root/ci/host/gitleaks.sh")" \
  "the CI image and ci/host/gitleaks.sh"
agree gitleaks-linux-x64 "$(image_arg GITLEAKS_SHA256)" \
  "$(sed -n 's/^ *Linux-x86_64) platform=linux_x64 sha256=\([0-9a-f]*\) ;;$/\1/p' "$root/ci/host/gitleaks.sh")" \
  "the CI image and ci/host/gitleaks.sh"
agree osv-scanner "$(image_arg OSV_SCANNER_VERSION)" "$(sed -n 's/^version=//p' "$root/ci/host/osv-scanner.sh")" \
  "the CI image and ci/host/osv-scanner.sh"
agree osv-scanner-linux-x64 "$(image_arg OSV_SCANNER_SHA256)" \
  "$(sed -n 's/^ *Linux-x86_64) platform=linux_amd64 sha256=\([0-9a-f]*\) ;;$/\1/p' "$root/ci/host/osv-scanner.sh")" \
  "the CI image and ci/host/osv-scanner.sh"
sanitizer() { sed -n "s/^$1 = \"\(.*\)\"\$/\1/p" "$root/ci/sanitizer-toolchain.toml"; }
agree sanitizer-toolchain "$(image_arg SANITIZER_TOOLCHAIN)" "$(sanitizer toolchain)" \
  "the CI image and ci/sanitizer-toolchain.toml"
agree cargo-fuzz "$(image_arg CARGO_FUZZ_VERSION)" "$(sanitizer cargo-fuzz)" \
  "the CI image and ci/sanitizer-toolchain.toml"

exit "$failed"
