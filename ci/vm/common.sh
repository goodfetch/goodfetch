#!/usr/bin/env bash
# Shared by the ci/vm scripts: the job VM's name, the CI image's tag and a
# shell inside the VM. Sourced, never run.

# One job runs on the machine at a time (the pre-job scripts' shared lock),
# so the VM has one name per runner user.
VM_NAME=goodfetch-job

# The CI image is tagged by its Containerfile's hash, so a changed
# Containerfile never reuses an older image from the cache folder.
ci_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
CI_IMAGE="goodfetch-ci:$(shasum -a 256 "$ci_root/ci/image/Containerfile" | cut -c1-16)"

# Where start.sh records the VM's cache mode for exec.sh.
VM_STATE="${RUNNER_TEMP:-${TMPDIR:-/tmp}}/goodfetch-vm-cache-mode"

vm_shell() {
  limactl shell --workdir /work "$VM_NAME" -- "$@"
}

export VM_NAME CI_IMAGE VM_STATE
