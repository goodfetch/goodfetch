#!/usr/bin/env bash
# Runs INSIDE automan-proto-builder. /src = crates (read-only is fine), /out = artifacts.
# Builds every Linux artifact with zig so gnu targets are manylinux2014 (glibc 2.17) and nothing
# needs amd64 emulation: x86_64 is a cross-compile, only *running* it would need QEMU.
set -euo pipefail
export CARGO_TARGET_DIR=/out/target-linux
TARGETS="${TARGETS:-aarch64-unknown-linux-gnu aarch64-unknown-linux-musl x86_64-unknown-linux-gnu x86_64-unknown-linux-musl}"
mkdir -p /out/napi /out/engined /out/wheels/native /out/wheels/rpc /out/sdist
t0=$(date +%s)

for t in $TARGETS; do
  case "$t" in
    aarch64-*) arch=arm64 ;; x86_64-*) arch=x64 ;;
  esac
  case "$t" in
    *-gnu) libc=gnu; zt="$t.2.17"; compat=manylinux2014 ;;
    *-musl) libc=musl; zt="$t"; compat=musllinux_1_2 ;;
  esac
  plat="linux-$arch-$libc"
  echo "== $plat ($t)"

  # (A) napi addon. A musl cdylib needs dynamic CRT (napi-rs docs: -crt-static).
  s=$(date +%s)
  if [ "$libc" = musl ]; then
    RUSTFLAGS="-C target-feature=-crt-static" cargo zigbuild -q --release -p automan-native-node --target "$zt"
  else
    cargo zigbuild -q --release -p automan-native-node --target "$zt"
  fi
  cp "/out/target-linux/$t/release/libautoman_native_node.so" "/out/napi/automan-native.$plat.node"
  echo "   napi .node built in $(( $(date +%s) - s ))s"

  # (B) engined binary (musl = fully static by default).
  s=$(date +%s)
  cargo zigbuild -q --release -p engined --target "$zt"
  cp "/out/target-linux/$t/release/engined" "/out/engined/engined-$plat"
  echo "   engined built in $(( $(date +%s) - s ))s"

  # (A) PyO3 wheels: abi3-py39, plus version-specific cp313 and free-threaded cp314t for comparison.
  s=$(date +%s)
  maturin build -q --release --zig --target "$t" --compatibility "$compat" \
    -m native-py/Cargo.toml --features abi3 -o /out/wheels/native
  for py in python3.13 python3.14t; do
    maturin build -q --release --zig --target "$t" --compatibility "$compat" \
      -m native-py/Cargo.toml -i "$py" -o /out/wheels/native || echo "   !! $py wheel failed for $t"
  done
  echo "   pyo3 wheels built in $(( $(date +%s) - s ))s"

  # (B) engined py3-none wheel (ruff/uv style: binary in .data/scripts + pure-Python SDK).
  s=$(date +%s)
  maturin build -q --release --zig --target "$t" --compatibility "$compat" \
    -m engined/Cargo.toml -o /out/wheels/rpc
  echo "   rpc wheel built in $(( $(date +%s) - s ))s"
done

maturin sdist -m native-py/Cargo.toml -o /out/sdist
maturin sdist -m engined/Cargo.toml -o /out/sdist
echo "linux build total $(( $(date +%s) - t0 ))s"
