#!/usr/bin/env bash
# Installs the static musl build's toolchain at the versions
# ci/musl/toolchain.sh pins (ADR 0024's note of 2026-11-06): Zig's release
# archive for this host, checked against its SHA-256 from ziglang.org's
# index, and cargo-zigbuild from crates.io with its lockfile, both into one
# folder outside the checkout; and rustup's llvm-tools component of the
# checkout's toolchain, whose llvm-objcopy and llvm-objdump ci/musl/link-cc
# orders a profile-guided build with (ADR 0057's note of 2026-11-14).
# `just musl-toolchain` runs it on a developer's Linux machine, the
# host-tools action on a musl cell. An install already there is kept. All
# are free.
set -euo pipefail

here=$(cd "$(dirname "$0")" && pwd)
# shellcheck source=ci/musl/toolchain.sh
source "$here/toolchain.sh"

case "$(uname -s)-$(uname -m)" in
  Linux-x86_64) arch=x86_64 sha256=$ZIG_SHA256_X86_64 ;;
  Linux-aarch64) arch=aarch64 sha256=$ZIG_SHA256_AARCH64 ;;
  *)
    echo "error: the static musl build is made on Linux, not on $(uname -s) $(uname -m)" >&2
    exit 2
    ;;
esac

if [[ ! -x "$musl_toolchain/zig/zig" ]]; then
  mkdir -p "$musl_toolchain"
  download=$(mktemp -d)
  trap 'rm -rf "$download"' EXIT
  archive="$download/zig.tar.xz"
  curl --proto '=https' --tlsv1.2 -fsSLo "$archive" \
    "https://ziglang.org/download/$ZIG_VERSION/zig-$arch-linux-$ZIG_VERSION.tar.xz"
  echo "$sha256  $archive" | sha256sum -c - >/dev/null
  mkdir "$download/zig"
  tar -xJf "$archive" -C "$download/zig" --strip-components 1
  rm -rf "$musl_toolchain/zig"
  mv "$download/zig" "$musl_toolchain/zig"
fi
if [[ ! -x "$musl_toolchain/bin/cargo-zigbuild" ]]; then
  cargo install --quiet --locked --root "$musl_toolchain" "cargo-zigbuild@$CARGO_ZIGBUILD_VERSION"
fi
# rustup reads the checkout's rust-toolchain.toml from the working folder.
llvm_tools="$(cd "$here/../.." && rustc --print sysroot)/lib/rustlib/$(cd "$here/../.." && rustc --print host-tuple)/bin"
if [[ ! -x "$llvm_tools/llvm-objcopy" ]]; then
  (cd "$here/../.." && rustup component add llvm-tools)
fi
echo "Zig $("$musl_toolchain/zig/zig" version) and $("$musl_toolchain/bin/cargo-zigbuild" --version) in $musl_toolchain."
