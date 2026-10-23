#!/usr/bin/env bash
# The V8 memory-corruption scripts, a slot of the Release suite on Linux x64
# (ADR 0067; TESTING.md, Confinement): each script, run by the Script sandbox
# worker on the test V8 build (`v8_enable_memory_corruption_api`), must end
# as `script_crashed`, so a broken V8 stays inside the worker's confinement.
#
#   ci/v8-corruption.sh fetch   fetches the test V8 build by the digest its
#                               pin commits, before the deny (the pinned
#                               artifact mirror's CI-only entry, ADR 0043);
#   ci/v8-corruption.sh test    runs the scripts on it, under the deny.
#
# The test V8 build job (#210) has not published a build yet, so no pin
# exists (ci/v8/test-build.toml) and both pass and say so. Once the pin
# exists, both fail until this script fetches the build and runs the
# scripts: a scheduled security test never skips (TESTING.md, Other rules).
set -euo pipefail

root=$(cd "$(dirname "$0")/.." && pwd)
pin="$root/ci/v8/test-build.toml"

case "${1:-}" in
  fetch | test) ;;
  *)
    echo "usage: ci/v8-corruption.sh fetch|test" >&2
    exit 2
    ;;
esac

if [[ ! -f "$pin" ]]; then
  echo "V8 memory-corruption scripts: no test V8 build is published yet (#210), so there is nothing to run them on."
  exit 0
fi

echo "error: ci/v8/test-build.toml pins a test V8 build, but ci/v8-corruption.sh does not fetch it and run the scripts yet (ADR 0067)" >&2
exit 1
