#!/usr/bin/env bash
# The first step of each hosted Linux Release cell, right after the checkout
# (ADR 0108): removes the toolchains the runner image preinstalls and no step
# of the suite uses, so the cell's builds fit on the runner's disk (run
# 39878992032 ran out of it), and builds on the temporary disk when it has
# more room. Prints `df -h` before and after.
#
# Each path below is named nowhere in release-suite.yml, .github/actions/,
# ci/ or the justfile:
#   /usr/share/dotnet, /usr/local/lib/android, /opt/ghc, /usr/local/.ghcup,
#   /usr/local/share/boost, /usr/share/swift: .NET, the Android SDK,
#   Haskell, Boost and Swift (LazyFS builds with fuse3, cmake and g++ only);
#   the tool cache's CodeQL, Go, PyPy and Ruby: Node and CPython, which
#   host-tools may use, stay, as does every JDK, which TLC runs on.
# Its Docker images too: no step uses a preinstalled image.
set -euo pipefail

[[ "$(uname -s)" == Linux ]] || {
  echo "error: ci/host/free-disk.sh frees a hosted Linux runner's disk only" >&2
  exit 2
}

echo "Before:"
df -h /
tools=${AGENT_TOOLSDIRECTORY:-/opt/hostedtoolcache}
sudo rm -rf \
  /usr/share/dotnet \
  /usr/local/lib/android \
  /opt/ghc \
  /usr/local/.ghcup \
  /usr/local/share/boost \
  /usr/share/swift \
  "$tools/CodeQL" \
  "$tools/go" \
  "$tools/PyPy" \
  "$tools/Ruby"
if command -v docker >/dev/null; then
  docker image prune --all --force >/dev/null
fi
echo "After:"
df -h /
# A runner with a larger temporary disk at /mnt (the x64 images have one)
# builds there: a folder of it is bind-mounted at `target`, as Windows'
# `target` is a junction to its Dev Drive, so every path under `target/`
# stays as it is. A bind mount, not a link: the build cache's tar saves and
# restores the folder's files through it, where it would store a link as a
# link and replace it with a folder on restore.
root=$(cd "$(dirname "$0")/../.." && pwd)
if mountpoint -q /mnt && [[ ! -e "$root/target" ]]; then
  free_root=$(df --output=avail -k / | tail -1)
  free_mnt=$(df --output=avail -k /mnt | tail -1)
  if ((free_mnt > free_root)); then
    sudo mkdir -p /mnt/target
    sudo chown "$(id -u):$(id -g)" /mnt/target
    mkdir "$root/target"
    sudo mount --bind /mnt/target "$root/target"
    echo "target is /mnt/target, bind-mounted:"
    df -h /mnt
  fi
fi
