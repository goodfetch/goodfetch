#!/usr/bin/env bash
# `just compiler-cache`: whether this checkout's builds reuse the C and C++
# objects other checkouts compiled (TESTING.md, Compiler cache). `just quick`
# and `just merge` run it before any build.
#
# Where the Rust compiler wrapper (RUSTC_WRAPPER) is sccache, it fails when:
# - sccache's base directories, which its server reads once at its start,
#   do not list this checkout's folder itself: sccache then keeps the
#   checkout's path in every C/C++ key (DuckDB's sources and include folders
#   live under target/), so nothing another checkout compiled hits here;
# - CMAKE_C_COMPILER_LAUNCHER or CMAKE_CXX_COMPILER_LAUNCHER does not go
#   through sccache: CMake then compiles BoringSSL outside the cache. Cargo
#   takes them from the environment when it sets them, else from
#   `.cargo/config.toml`'s `[env]`, which names ci/compiler-launcher.sh.
#
# Where no build uses sccache (another wrapper or none, or sccache missing),
# there is no cache to cover: it says so and passes.
#
# It keeps to bash 3.2, which stock macOS runs.
set -euo pipefail

wrapper=${RUSTC_WRAPPER:-}
case $wrapper in
  sccache | */sccache | sccache.exe | */sccache.exe) ;;
  *)
    echo "The compiler cache check is skipped: the Rust compiler wrapper (RUSTC_WRAPPER) is not sccache, so no build here uses it."
    exit 0
    ;;
esac
if ! command -v "$wrapper" >/dev/null 2>&1; then
  echo "The compiler cache check is skipped: sccache is not installed on this host."
  exit 0
fi

failed=0
fail() {
  echo "error[compiler-cache] $1: $2"
  failed=1
}

for variable in CMAKE_C_COMPILER_LAUNCHER CMAKE_CXX_COMPILER_LAUNCHER; do
  if [[ -n ${!variable+set} ]]; then
    launcher=${!variable}
    from="the environment"
  else
    launcher=$(sed -n "s/^$variable = { value = \"\([^\"]*\)\".*/\1/p" .cargo/config.toml 2>/dev/null || true)
    from=".cargo/config.toml"
  fi
  case $launcher in
    sccache | */sccache | ci/compiler-launcher.sh | */ci/compiler-launcher.sh) ;;
    "") fail "$variable" "unset in $from, so CMake compiles BoringSSL without sccache; .cargo/config.toml's [env] sets it to ci/compiler-launcher.sh" ;;
    *) fail "$variable" "$from sets it to $launcher, which does not go through sccache, so CMake compiles BoringSSL outside the cache; unset it" ;;
  esac
done

root=$(pwd -P)
stats=$("$wrapper" --show-stats --stats-format json)
if ! jq -e 'has("basedirs")' >/dev/null <<<"$stats"; then
  fail "$root" "this sccache ($(jq -r '.version // "unknown"' <<<"$stats")) has no base directories; install sccache 0.18 or later"
else
  # sccache strips the longest listed prefix, so this checkout is covered
  # only when that prefix is the checkout itself.
  longest=""
  listed=()
  while IFS= read -r folder; do
    folder=${folder%/}
    listed+=("$folder")
    if [[ ($root == "$folder" || $root == "$folder"/*) && ${#folder} -gt ${#longest} ]]; then
      longest=$folder
    fi
  done < <(jq -r '.basedirs[]' <<<"$stats")
  if [[ $longest != "$root" ]]; then
    if [[ ${#listed[@]} -eq 0 ]]; then
      shown="none"
    else
      shown="${listed[*]}"
    fi
    fail "$root" "sccache's base directories ($shown) do not list this checkout, so the C/C++ objects other checkouts compiled (DuckDB, BoringSSL) miss here. Work in a listed checkout, such as a slot worktree, or add this folder to basedirs in sccache's config file and restart its server when no build runs (TESTING.md, Compiler cache)"
  fi
fi

if [[ $failed -eq 0 ]]; then
  echo "The compiler cache covers this checkout: sccache lists $root among its base directories, and CMake compiles C and C++ through it."
fi
exit "$failed"
