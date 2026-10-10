#!/usr/bin/env bash
# PROTOTYPE (throwaway): what does INSTALLING each engine binding strategy feel like on a clean machine?
#   (A) native: napi-rs .node in npm platform packages + PyO3 extension wheels
#   (B) rpc:    one `engined` binary in npm platform packages + py3-none wheels, thin pure-JS/Python SDKs
#
# Usage: ./run.sh [phase ...] [--only GLOB]
#   phases (default: all, in this order):
#     images    build the builder image and the "fresh machine + package manager" images
#     build     build every artifact (Linux in Docker via zig, macOS arm64 on the host)
#     registry  start verdaccio (npm) + a PEP 503 index (pip/uv/poetry), publish everything
#     fixtures  produce lockfiles/caches on macOS for the cross-OS cells
#     node      Node cells (arm64 containers)        python   Python cells (arm64 containers)
#     amd64     the few emulated linux/amd64 cells   macos    host install + Gatekeeper/codesign checks
#     report    print the matrix and copy logs into ./logs
#     down      stop the registry containers
#   --only GLOB   run only cells whose name matches the shell glob (e.g. --only 'node24-slim:*rpc*')
set -uo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
OUT="$HERE/out"; RES="$OUT/results"; NET=automan-proto
ONLY='*'; PHASES=()
while [ $# -gt 0 ]; do
  case "$1" in --only) ONLY="$2"; shift 2 ;; -h|--help) sed -n '2,20p' "$0"; exit 0 ;; *) PHASES+=("$1"); shift ;; esac
done
[ ${#PHASES[@]} -eq 0 ] && PHASES=(images build registry fixtures node python amd64 macos report)
mkdir -p "$RES"

row() { # cell -> one matrix line from its kv
  local c=$1 f="$RES/$1/kv"; [ -f "$f" ] || { printf '%-44s (no result)\n' "$c"; return; }
  # shellcheck disable=SC1090
  local install_rc test_rc secs rx_bytes disk_kb installed; install_rc=; test_rc=; secs=; rx_bytes=; disk_kb=; installed=
  eval "$(grep -E '^(install_rc|test_rc|secs|rx_bytes|disk_kb)=' "$f")"
  installed=$(sed -n 's/^installed=//p' "$f")
  local ins=ok use=ok err=""
  [ "$install_rc" = 0 ] || ins="FAIL($install_rc)"
  [ "$test_rc" = 0 ] || { use="FAIL"; err=$(grep -v throw "$RES/$c/test.txt" | grep -m1 -E '^[A-Za-z]*Error|Error:|ERR_|denied' | cut -c1-110); }
  [ "$install_rc" = 0 ] || err=$(grep -v '^\$ ' "$RES/$c/log.txt" | grep -m1 -E '^ *(ERROR|error|Error|ERR_|×|➤ YN0[0-9]*[1-9]:|Caused by)' | cut -c1-110)
  printf '%-44s %-9s %-5s %6ss %10s %7sK  %s %s\n' "$c" "$ins" "$use" "$secs" "$rx_bytes" "$disk_kb" "$installed" "${err:+| $err}"
}
header() { printf '\n%-44s %-9s %-5s %7s %10s %8s  %s\n' "CELL" "INSTALL" "USE" "TIME" "RX_BYTES" "DISK" "INSTALLED | first error"; }
want() { case "$1" in $ONLY) return 0 ;; esac; return 1; }

# ncell NAME IMAGE PKG PM MODE SPEC [docker args...]
ncell() {
  local name=$1 img=$2 pkg=$3 pm=$4 mode=$5 spec=$6; shift 6; want "$name" || return 0
  rm -rf "${RES:?}/$name"
  local net=(--network "$NET"); case " $* " in *" --network none "*) net=() ;; esac
  docker run --rm "${net[@]}" -e CELL="$name" -e PKG="$pkg" -e PM="$pm" -e MODE="$mode" -e SPEC="$spec" \
    -e REG=http://verdaccio:4873/ -v "$HERE/scripts:/scripts:ro" -v "$OUT/fixtures:/fixtures:ro" -v "$RES:/results" \
    "$@" "$img" /scripts/cell-node.sh > /dev/null 2>&1 || true
  row "$name"
}
# pcell NAME IMAGE PKG INST VARIANT [docker args...]
pcell() {
  local name=$1 img=$2 pkg=$3 inst=$4 variant=$5; shift 5; want "$name" || return 0
  rm -rf "${RES:?}/$name"
  docker run --rm --network "$NET" -e CELL="$name" -e PKG="$pkg" -e INST="$inst" -e VARIANT="$variant" \
    -e IDX=http://pyidx:8000 -v "$HERE/scripts:/scripts:ro" -v "$RES:/results" "$@" "$img" /scripts/cell-py.sh > /dev/null 2>&1 || true
  row "$name"
}

phase_images() {
  docker build -q -t automan-proto-builder -f "$HERE/docker/builder.Dockerfile" "$HERE/docker"
  for b in 24-slim 24-alpine 20-slim; do
    docker build -q --build-arg BASE="node:$b" -t "automan-proto-node:$b" -f "$HERE/docker/node-tools.Dockerfile" "$HERE/docker"
  done
  docker build -q --build-arg BASE=python:3.13-slim --build-arg WITH_POETRY=1 -t automan-proto-py:313-slim -f "$HERE/docker/py-tools.Dockerfile" "$HERE/docker"
  for b in 3.13-alpine 3.9-slim 3.9-alpine; do
    docker build -q --build-arg BASE="python:$b" -t "automan-proto-py:${b/./}" -f "$HERE/docker/py-tools.Dockerfile" "$HERE/docker"
  done
  docker build -q --build-arg BASE=debian:bookworm-slim --build-arg FT=3.14t -t automan-proto-py:314t -f "$HERE/docker/py-tools.Dockerfile" "$HERE/docker"
}

phase_build() {
  mkdir -p "$OUT"
  echo "== Linux artifacts (Docker, zig; x86_64 is cross-compiled, no emulation)"
  docker run --rm -v "$HERE/src:/src" -v "$OUT:/out" -v "$HERE/scripts:/scripts:ro" \
    -v automan-proto-cargo:/usr/local/cargo/registry automan-proto-builder /scripts/build-linux.sh 2>&1 | tee "$OUT/build-linux.log" \
    | grep -E '^==|built in|total|!!|💥'
  echo "== macOS arm64 artifacts (host)"
  "$HERE/scripts/build-macos.sh" 2>&1 | tee "$OUT/build-macos.log" | grep -E 'built in|total|💥'
}

phase_registry() {
  docker network inspect "$NET" >/dev/null 2>&1 || docker network create "$NET" >/dev/null
  docker rm -f automan-proto-verdaccio automan-proto-pyidx >/dev/null 2>&1
  rm -rf "$OUT/verdaccio" && mkdir -p "$OUT/verdaccio"
  docker run -d --name automan-proto-verdaccio --network "$NET" --network-alias verdaccio -p 4873:4873 \
    -v "$HERE/docker/verdaccio.yaml:/verdaccio/conf/config.yaml:ro" -v "$OUT/verdaccio:/verdaccio/storage" \
    verdaccio/verdaccio:6 >/dev/null
  python3 "$HERE/scripts/make-pyindex.py" "$OUT"
  docker run -d --name automan-proto-pyidx --network "$NET" --network-alias pyidx -p 8000:8000 \
    -v "$OUT/pyindex:/idx:ro" python:3.13-alpine python -m http.server 8000 -d /idx >/dev/null
  for _ in $(seq 60); do curl -sf http://localhost:4873/-/ping >/dev/null && break; sleep 1; done
  "$HERE/scripts/publish-npm.sh"
}

phase_fixtures() { "$HERE/scripts/macos.sh" fixtures; }

phase_node() {
  header
  local N=@automan-proto/native@0.1.0 P=@automan-proto/rpc@0.1.1
  for img in 24-slim 24-alpine 20-slim; do
    local I="automan-proto-node:$img"
    for pkg in native rpc; do
      local S=$N; [ $pkg = rpc ] && S=$P
      for pm in npm pnpm yarn yarn-pnp; do ncell "node$img:$pkg:$pm" "$I" $pkg $pm default "$S"; done
      ncell "node$img:$pkg:npm:ignore-scripts" "$I" $pkg npm ignore-scripts "$S"
      ncell "node$img:$pkg:npm:omit-optional" "$I" $pkg npm omit-optional "$S"
      ncell "node$img:$pkg:pnpm:no-optional" "$I" $pkg pnpm omit-optional "$S"
    done
  done
  for img in 1-slim 1-alpine; do
    for pkg in native rpc; do
      local S=$N; [ $pkg = rpc ] && S=$P
      ncell "bun$img:$pkg:bun" "oven/bun:$img" $pkg bun default "$S"
      ncell "bun$img:$pkg:bun:omit-optional" "oven/bun:$img" $pkg bun omit-optional "$S"
    done
  done
  # Lockfile produced on macOS, installed on Linux.
  for pkg in native rpc; do
    for lock in npm11 npm11regen npm10 npm10regen; do
      ncell "node24-slim:$pkg:npm:maclock-$lock" automan-proto-node:24-slim $pkg npm maclock - -e LOCK="$lock-$pkg"
      ncell "node20-slim:$pkg:npm:maclock-$lock" automan-proto-node:20-slim $pkg npm maclock - -e LOCK="$lock-$pkg"
    done
    ncell "node24-alpine:$pkg:npm:maclock-npm11" automan-proto-node:24-alpine $pkg npm maclock - -e LOCK="npm11-$pkg"
    ncell "node24-slim:$pkg:pnpm:maclock" automan-proto-node:24-slim $pkg pnpm maclock -
    # Same lockfile without the pnpm-workspace.yaml pnpm wrote next to it (release-age exclusions).
    ncell "node24-slim:$pkg:pnpm:maclock-agegate-default" automan-proto-node:24-slim $pkg pnpm maclock - -e LOCK="pnpmnoexclude-$pkg"
    ncell "node24-slim:$pkg:yarn:maclock" automan-proto-node:24-slim $pkg yarn maclock -
    ncell "bun1-slim:$pkg:bun:maclock" oven/bun:1-slim $pkg bun maclock -
  done
  # Offline: vendored main tarball only; cache primed on Linux; cache primed on macOS.
  for pkg in native rpc; do
    local S=$N; [ $pkg = rpc ] && S=$P
    ncell "node24-slim:$pkg:npm:offline-tarball" automan-proto-node:24-slim $pkg npm offline-tarball - --network none
    rm -rf "$OUT/cache/linux-$pkg"; mkdir -p "$OUT/cache/linux-$pkg"
    ncell "node24-slim:$pkg:npm:prime-cache" automan-proto-node:24-slim $pkg npm prime-cache "$S" -v "$OUT/cache/linux-$pkg:/cache"
    ncell "node24-slim:$pkg:npm:offline-linuxcache" automan-proto-node:24-slim $pkg npm offline-cache - --network none -v "$OUT/cache/linux-$pkg:/cache"
    rm -rf "$OUT/cache/mac-$pkg"; mkdir -p "$OUT/cache"; cp -R "$OUT/fixtures/maccache-$pkg-npm11" "$OUT/cache/mac-$pkg"
    ncell "node24-slim:$pkg:npm:offline-maccache" automan-proto-node:24-slim $pkg npm offline-cache - --network none -v "$OUT/cache/mac-$pkg:/cache"
  done
  # Release-age gates: what a user sees in the first 24h after a release (both strategies alike).
  ncell "node24-slim:native:yarn:agegate-default" automan-proto-node:24-slim native yarn default "$N" -e AGEGATE=default
  ncell "node24-slim:rpc:yarn:agegate-default" automan-proto-node:24-slim rpc yarn default "$P" -e AGEGATE=default
  # RPC-only packaging variants.
  ncell "node24-slim:rpc0.1.0-no-preferUnplugged:yarn-pnp" automan-proto-node:24-slim rpc yarn-pnp default @automan-proto/rpc@0.1.0
  for img in 24-slim 24-alpine; do
    ncell "node$img:rpc0.2.0-static-musl:npm" "automan-proto-node:$img" rpc npm default @automan-proto/rpc@0.2.0
  done
  ncell "bun1-alpine:rpc0.2.0-static-musl:bun" oven/bun:1-alpine rpc bun default @automan-proto/rpc@0.2.0
}

phase_python() {
  header
  for img in 313-slim 313-alpine 39-slim 314t; do
    for pkg in native rpc; do
      for inst in pip uv; do pcell "py$img:$pkg:$inst" "automan-proto-py:$img" $pkg $inst full; done
    done
  done
  for pkg in native rpc; do
    pcell "py313-slim:$pkg:poetry" automan-proto-py:313-slim $pkg poetry full
    # A release that ships only abi3 wheels (+/- sdist), on a free-threaded interpreter.
    pcell "py314t:$pkg:pip:abi3+sdist" automan-proto-py:314t $pkg pip abi3sdist
    pcell "py314t:$pkg:uv:abi3-only" automan-proto-py:314t $pkg uv abi3
    # Old pip that predates the wheel's platform tag.
    pcell "py39-slim:$pkg:pip19.2(pre-manylinux2014)" automan-proto-py:39-slim $pkg pip full -e OLDPIP=19.2.3
    pcell "py39-alpine:$pkg:pip20.2(pre-musllinux)" automan-proto-py:39-alpine $pkg pip full -e OLDPIP=20.2.4
    pcell "py39-alpine:$pkg:pip" automan-proto-py:39-alpine $pkg pip full
  done
  # RPC: one static-musl wheel per arch, dual-tagged manylinux+musllinux (uv style).
  pcell "py313-slim:rpc-dualtag:pip" automan-proto-py:313-slim rpc pip abi3
  pcell "py313-alpine:rpc-dualtag:uv" automan-proto-py:313-alpine rpc uv abi3
  pcell "py39-slim:rpc-dualtag:pip19.2" automan-proto-py:39-slim rpc pip abi3 -e OLDPIP=19.2.3
}

phase_amd64() {
  header
  echo "(linux/amd64 under QEMU emulation on an arm64 host: timings are NOT comparable)"
  for pkg in native rpc; do
    local S=@automan-proto/native@0.1.0; [ $pkg = rpc ] && S=@automan-proto/rpc@0.1.1
    ncell "amd64-node24-slim:$pkg:npm" node:24-slim $pkg npm default "$S" --platform linux/amd64
    pcell "amd64-py313-slim:$pkg:pip" python:3.13-slim $pkg pip full --platform linux/amd64
  done
}

phase_macos() {
  "$HERE/scripts/macos.sh" cells
  header; for c in "$RES"/mac-*; do row "$(basename "$c")"; sed -n 's/^gatekeeper=/    gatekeeper: /p' "$c/kv"; done
}

phase_report() {
  python3 "$HERE/scripts/report.py" "$OUT" "$HERE/logs"
}

phase_down() { docker rm -f automan-proto-verdaccio automan-proto-pyidx >/dev/null 2>&1; }

for p in "${PHASES[@]}"; do echo; echo "######## $p"; "phase_$p"; done
