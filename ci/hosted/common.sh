#!/usr/bin/env bash
# Shared by the ci/hosted and ci/egress scripts: the hosted tier's job
# container, the CI image's tag and the deny's marker (ADR 0135's amendment
# of 2026-11-22). Sourced, never run.

# The job's container on the standard hosted runner, which start.sh starts
# from the tree's pinned CI image and every step's command runs in
# (exec.sh).
CONTAINER=goodfetch-job

# The CI image, tagged by its Containerfile's hash as the VM's is
# (ci/vm/common.sh), built from the tree the job checked out.
hosted_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
CI_IMAGE="goodfetch-ci:$(sha256sum "$hosted_root/ci/image/Containerfile" | cut -c1-16)"

# The image's compressed archive, which a `ci image` job saves into the
# Actions cache on `main` and pull-request jobs restore before their
# container starts (ci/hosted/image.sh, #3480). The workflows' cache steps
# name its folder, ~/.cache/goodfetch/ci-image.
CI_IMAGE_ARCHIVE="$HOME/.cache/goodfetch/ci-image/ci-image.tar.zst"

# Written on the runner once ci/egress/egress.sh has denied the container's
# outbound traffic, holding the job's name, so exec.sh runs later steps
# offline and the end-of-job check knows the deny ran.
EGRESS_STATE="${RUNNER_TEMP:-/tmp}/goodfetch-egress-denied"

# Whether the job's container runs on this host: the deny then applies in
# its network namespace (ci/egress/host.sh).
hosted_container_running() {
  command -v docker >/dev/null &&
    [[ "$(docker inspect --format '{{.State.Running}}' "$CONTAINER" 2>/dev/null)" == true ]]
}

export CONTAINER CI_IMAGE CI_IMAGE_ARCHIVE EGRESS_STATE
