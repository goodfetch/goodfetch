#!/usr/bin/env bash
# Starts the job's throwaway VM on the owner's machine (ADR 0135), loads the
# CI image into it and starts the job's container, which restores the build
# cache and makes every download the job needs before ADR 0043's deny
# (in-container.sh prepare): `ci/vm/start.sh read-only|writable`.
#
# The job's workspace is mounted writable at /work. The cache folder
# ($GOODFETCH_CACHE_FOLDER, set in the runner's .env by the owner, #1562) is
# mounted at /cache: read-only for every pull-request job, writable only for
# the cache-warming job on `main`.
set -euo pipefail

cache_mode=${1:?usage: ci/vm/start.sh read-only|writable}
here=$(cd "$(dirname "$0")" && pwd)
# shellcheck source=ci/vm/common.sh
source "$here/common.sh"

case "$cache_mode" in
  read-only) cache_writable=false cache_flag=ro ;;
  writable) cache_writable=true cache_flag=rw ;;
  *) echo "error: the cache mode is read-only or writable, not $cache_mode" >&2; exit 2 ;;
esac

cache_folder=${GOODFETCH_CACHE_FOLDER:?the runner sets GOODFETCH_CACHE_FOLDER in its .env (#1562)}
workspace=${GITHUB_WORKSPACE:-$(pwd)}

# A VM an earlier job left behind is destroyed first (the pre-job script also
# does so before it admits a job).
limactl delete --force "$VM_NAME" >/dev/null 2>&1 || true
rm -f "$EGRESS_STATE"

mounts=$(printf '[{"location":"%s","mountPoint":"/work","writable":true},{"location":"%s","mountPoint":"/cache","writable":%s}]' \
  "$workspace" "$cache_folder" "$cache_writable")
limactl create --tty=false --name="$VM_NAME" --set=".mounts=$mounts" "$here/lima.yaml"
limactl start --tty=false "$VM_NAME"

# Load the CI image from the cache folder, or build it when the folder lacks
# this version (and, for the cache-warming job, save it there).
image_tar="/cache/image/$CI_IMAGE.tar"
vm_shell bash -euo pipefail -c "
  if [ -f '$image_tar' ]; then
    nerdctl load --input '$image_tar'
  else
    nerdctl build --platform linux/amd64 --tag '$CI_IMAGE' /work/ci/image
    if [ '$cache_writable' = true ]; then
      mkdir -p /cache/image
      nerdctl save --output '$image_tar.partial' '$CI_IMAGE'
      mv '$image_tar.partial' '$image_tar'
    fi
  fi
"

# The job's container lives as long as the VM, so what `just fetch` installs
# before the deny serves every later step.
vm_shell nerdctl run --detach --name "$CONTAINER" --platform linux/amd64 \
  --volume /work:/work --volume "/cache:/cache:$cache_flag" \
  --env CI=true --env "GOODFETCH_CACHE_MODE=$cache_mode" \
  "$CI_IMAGE" sleep infinity
vm_shell nerdctl exec "$CONTAINER" /work/ci/vm/in-container.sh prepare
