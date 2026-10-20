#!/usr/bin/env bash
# `just visual-repeatability [browser,…]`: the measurements ADR 0073 asks for
# before a browser joins the `visual` projects, rerunnable on any host with
# Docker or Podman. On a copy of HEAD under target/visual-repeatability/, in
# the pinned Playwright image (ci/visual/image.toml, Linux x86-64, so under
# emulation on an arm64 host):
#   1. it names the host and the emulation the image runs under;
#   2. it compares every visual case with the Reference screenshots committed
#      at HEAD, which only the `update-screenshots` workflow writes, so with
#      CI's pixels; with none committed yet, it says so;
#   3. it runs every Playwright visual case 20 times on each browser
#      (chromium, firefox and webkit by default) against a Reference it
#      writes first in the copy (ci/visual/run.sh measure), and prints each
#      browser's verdict.
# Record what it prints in test-support/typescript/src/visual-config.ts's
# REPEATABILITY. It exits 1 when a browser's screenshots do not repeat or the
# emulated pixels differ from CI's.
set -euo pipefail

browsers=${1:-chromium,firefox,webkit}
cd "$(dirname "$0")/../.."

engine=
for candidate in docker podman; do
  if command -v "$candidate" >/dev/null; then
    engine=$candidate
    break
  fi
done
if [[ -z "$engine" ]]; then
  echo "error: the measurement runs the pinned Playwright image through Docker or Podman; install one of them" >&2
  exit 2
fi

key() { sed -n "s/^$1 = \"\\(.*\\)\"\$/\\1/p" ci/visual/image.toml; }
digest=$(key digest)
image="$(key repository)@$digest"
commit=$(git rev-parse HEAD)

copy=target/visual-repeatability
rm -rf "$copy"
mkdir -p "$copy"
git archive HEAD | tar -x -C "$copy"
references=$(git ls-files | grep -c -E '(-snapshots|__screenshots__)/[^/]+\.png$' || true)

host="$(uname -s) $(uname -m)"
if [[ "$(uname -s)" == Darwin ]]; then host="$host, $(sysctl -n machdep.cpu.brand_string)"; fi
echo "Commit $commit, image $digest, host $host, $engine."

# shellcheck disable=SC2016 # the script expands inside the container
"$engine" run --rm --platform linux/amd64 --ipc=host \
  --volume "$PWD/$copy:/work" --env CI=true --env "GOODFETCH_VISUAL_IMAGE=$digest" \
  --env "BROWSERS=$browsers" --env "REFERENCES=$references" \
  "$image" bash -c '
    set -uo pipefail
    cd /work
    # Rosetta for Linux names its processor "VirtualApple", QEMU "QEMU Virtual CPU".
    echo "In the image: $(uname -m), processor $(sed -n "s/^model name[[:space:]]*: //p" /proc/cpuinfo | head -1)."
    ci/visual/run.sh install >target/visual-install.log 2>&1 || { tail -40 target/visual-install.log; exit 2; }
    status=0
    if [[ "$REFERENCES" -eq 0 ]]; then
      echo "CI pixels: not compared, HEAD holds no Reference screenshot from the update-screenshots workflow yet."
    elif ci/visual/run.sh test >target/visual-compare.log 2>&1; then
      echo "CI pixels: the same, every visual case matches the $REFERENCES committed References."
    else
      tail -40 target/visual-compare.log
      echo "CI pixels: different, a visual case does not match the committed References; local visual runs on such a host dispatch the visual job to CI instead (ADR 0073)."
      status=1
    fi
    ci/visual/run.sh measure "$BROWSERS" || status=1
    exit "$status"
  '
