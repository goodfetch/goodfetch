#!/usr/bin/env bash
# Installs the owner's copy of the self-hosted allowlist and the runner's
# pre-job and post-job scripts (ADRs 0108 and 0135): `just runner-install
# <folder>`, run by the owner, as the owner, on the runner's machine, from a
# checkout whose allowlist the owner has reviewed. The folder ends up owned by
# the owner, readable by every user and writable only by the owner, so no job
# can change it.
set -euo pipefail

folder=${1:?usage: ci/runner/install.sh <folder>}
root=$(cd "$(dirname "$0")/../.." && pwd)
lock_parent=/Users/Shared/goodfetch-runner-lock

mkdir -p "$folder"
cargo run --quiet --manifest-path "$root/Cargo.toml" --bin xtask -- self-hosted export >"$folder/allowlist.tsv.partial"
mv "$folder/allowlist.tsv.partial" "$folder/allowlist.tsv"
cp "$root/ci/runner/pre-job.sh" "$root/ci/runner/post-job.sh" "$folder/"
chmod 755 "$folder" "$folder/pre-job.sh" "$folder/post-job.sh"
chmod 644 "$folder/allowlist.tsv"

# The runner instances' shared lock lives here, writable by each runner user.
mkdir -p "$lock_parent"
chmod 777 "$lock_parent"

cat <<EOF
Installed into $folder:
$(sed 's/^/  /' "$folder/allowlist.tsv")

Add these lines to each runner instance's .env, then restart the runner:
  ACTIONS_RUNNER_HOOK_JOB_STARTED=$folder/pre-job.sh
  ACTIONS_RUNNER_HOOK_JOB_COMPLETED=$folder/post-job.sh
  GOODFETCH_CACHE_FOLDER=<the cache folder #1562 creates>
EOF
