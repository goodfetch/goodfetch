#!/usr/bin/env bash
# Starts the job's throwaway VM on the owner's machine (ADR 0135) and loads
# the images the job runs into it:
#   ci/vm/start.sh read-only|writable [ci] [playwright]
# `ci` is the CI image (the default when no image is named); `playwright` is
# the pinned Playwright image of the visual projects (ADR 0073), fetched here,
# before any of the job's own steps. With the CI image, it then starts the
# job's container, which restores the build cache and makes every download
# the job needs (in-container.sh prepare), all before ADR 0043's deny.
#
# The job's workspace is mounted writable at /work. The cache folder
# ($GOODFETCH_CACHE_FOLDER, set in the runner's .env by the owner, #1562) is
# mounted at /cache: read-only for every pull-request job, writable only for
# the cache-warming job on `main`.
set -euo pipefail

cache_mode=${1:?usage: ci/vm/start.sh read-only|writable [ci] [playwright]}
shift
images=("$@")
[[ ${#images[@]} -eq 0 ]] && images=(ci)
here=$(cd "$(dirname "$0")" && pwd)
# shellcheck source=ci/vm/common.sh
source "$here/common.sh"

case "$cache_mode" in
  read-only) cache_writable=false cache_flag=ro ;;
  writable) cache_writable=true cache_flag=rw ;;
  *) echo "error: the cache mode is read-only or writable, not $cache_mode" >&2; exit 2 ;;
esac
for image in "${images[@]}"; do
  case "$image" in
    ci | playwright) ;;
    *) echo "error: the image is ci or playwright, not $image" >&2; exit 2 ;;
  esac
done

cache_folder=${GOODFETCH_CACHE_FOLDER:?the runner sets GOODFETCH_CACHE_FOLDER in its .env (#1562)}
workspace=${GITHUB_WORKSPACE:-$(pwd)}

# A VM an earlier job left behind is destroyed first.
limactl delete --force "$VM_NAME" >/dev/null 2>&1 || true
rm -f "$EGRESS_STATE"

mounts=$(printf '[{"location":"%s","mountPoint":"/work","writable":true},{"location":"%s","mountPoint":"/cache","writable":%s}]' \
  "$workspace" "$cache_folder" "$cache_writable")
limactl create --tty=false --name="$VM_NAME" --set=".mounts=$mounts" "$here/lima.yaml"
limactl start --tty=false "$VM_NAME"

for image in "${images[@]}"; do
  case "$image" in
    ci)
      # Load the CI image from the cache folder, or build it when the folder
      # lacks this version (and, for the cache-warming job, save it there).
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
      ;;
    playwright)
      # Load the pinned Playwright image from the cache folder, or pull it by
      # digest, which fixes its content and its Linux x86-64 platform (and,
      # for the cache-warming job, save it there).
      image_tar="/cache/image/playwright-${PLAYWRIGHT_DIGEST#sha256:}.tar"
      vm_shell bash -euo pipefail -c "
        if [ -f '$image_tar' ]; then
          nerdctl load --input '$image_tar'
        else
          nerdctl pull --quiet --platform linux/amd64 '$PLAYWRIGHT_IMAGE'
          if [ '$cache_writable' = true ]; then
            mkdir -p /cache/image
            nerdctl save --platform linux/amd64 --output '$image_tar.partial' '$PLAYWRIGHT_IMAGE'
            mv '$image_tar.partial' '$image_tar'
          fi
        fi
      "
      ;;
  esac
done

# The job's container lives as long as the VM, so what `just fetch` installs
# before the deny serves every later step. CAP_SYS_ADMIN, inside rootless
# containerd's user namespace, lets the container's default seccomp profile
# allow `unshare` and `mount`, so `DnsNamespace` makes its user, network and
# mount namespaces there as the container's root (ADR 0067), the way the
# job declares.
for image in "${images[@]}"; do
  [[ "$image" == ci ]] || continue
  vm_shell nerdctl run --detach --name "$CONTAINER" --platform linux/amd64 \
    --cap-add NET_ADMIN --cap-add SYS_ADMIN \
    --volume /work:/work --volume "/cache:/cache:$cache_flag" \
    --env CI=true --env "GOODFETCH_CACHE_MODE=$cache_mode" \
    --env GOODFETCH_TEST_DNS_NAMESPACE=user_namespace \
    "$CI_IMAGE" sleep infinity
  vm_shell nerdctl exec "$CONTAINER" /work/ci/vm/in-container.sh prepare
done
