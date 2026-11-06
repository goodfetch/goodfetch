# shellcheck shell=bash
# The toolchain of the static musl build the Products ship on Linux (ADR
# 0024's note of 2026-11-06): Zig's C and C++ compilers and linker, which
# carry musl's headers and a libc++ built for it, through cargo-zigbuild.
# ci/musl/install.sh installs these versions and ci/musl/cargo runs them;
# both source this file.
export ZIG_VERSION=0.17.1
export ZIG_SHA256_X86_64=3581863e3cbcfd0fce0efc682ce7e9e93da96668a977791d356a26d2a7e9b17a
export ZIG_SHA256_AARCH64=a1546d5646eb184437918d6635ca6b762a0e889eac09cef720ee7a1ed3fe7795
export CARGO_ZIGBUILD_VERSION=0.24.1
# The folder they are installed in, outside every checkout and named by
# their versions, so worktrees share one install and a new pin installs
# beside the old one.
export musl_toolchain="${XDG_DATA_HOME:-$HOME/.local/share}/goodfetch/musl-toolchain/zig-$ZIG_VERSION-cargo-zigbuild-$CARGO_ZIGBUILD_VERSION"
