#!/bin/sh
# CMake's C and C++ compiler launcher for every Cargo build in this
# repository: `.cargo/config.toml`'s `[env]` sets CMAKE_C_COMPILER_LAUNCHER
# and CMAKE_CXX_COMPILER_LAUNCHER to this file, CMake reads them from the
# environment, and so BoringSSL's CMake build (`btls-sys`) runs each compile
# as `ci/compiler-launcher.sh <compiler> <arguments>…`.
#
# It follows the rule the `cc` crate already applies to DuckDB's C++: when
# the Rust compiler wrapper (RUSTC_WRAPPER) is sccache, C and C++ compile
# through it too, so a checkout listed in sccache's base directories reuses
# the objects other checkouts compiled (TESTING.md, Compiler cache).
# Otherwise, as on a host without sccache or under `just coverage`, which
# unsets the wrapper, the compiler runs itself. Only the cache changes:
# the compiler, its arguments and its output are the same either way.
case ${RUSTC_WRAPPER:-} in
  sccache | */sccache | sccache.exe | */sccache.exe) exec "$RUSTC_WRAPPER" "$@" ;;
esac
exec "$@"
