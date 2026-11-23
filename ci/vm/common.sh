#!/usr/bin/env bash
# Shared by the ci/vm and ci/egress scripts: the job VM's and container's
# names, the CI image's tag, the deny's marker and a shell inside the VM.
# Sourced, never run.

# One job runs on the machine at a time, so the VM has one name per user.
VM_NAME=goodfetch-job

# The job's container inside the VM, which start.sh starts and every step's
# command runs in (exec.sh); and the pinned Playwright image's container,
# which `ci/vm/visual.sh install` starts for the visual projects.
CONTAINER=goodfetch-job
VISUAL_CONTAINER=goodfetch-visual

# The CI image is tagged by its Containerfile's hash, so a changed
# Containerfile never reuses an older image from the cache folder.
ci_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
CI_IMAGE="goodfetch-ci:$(shasum -a 256 "$ci_root/ci/image/Containerfile" | cut -c1-16)"

# Written on the host once ci/egress/egress.sh has denied outbound traffic,
# holding the job's name, so exec.sh runs later steps offline and the
# end-of-job check knows the deny ran.
EGRESS_STATE="${RUNNER_TEMP:-${TMPDIR:-/tmp}}/goodfetch-egress-denied"

# The pinned Playwright image of the visual projects (ADR 0073), by digest,
# from ci/visual/image.toml.
image_key() {
  sed -n "s/^$1 = \"\\(.*\\)\"\$/\\1/p" "$ci_root/ci/visual/image.toml"
}
PLAYWRIGHT_DIGEST=$(image_key digest)
PLAYWRIGHT_IMAGE="$(image_key repository)@$PLAYWRIGHT_DIGEST"

vm_shell() {
  limactl shell --workdir /work "$VM_NAME" -- "$@"
}

export VM_NAME CONTAINER VISUAL_CONTAINER CI_IMAGE EGRESS_STATE PLAYWRIGHT_DIGEST PLAYWRIGHT_IMAGE
