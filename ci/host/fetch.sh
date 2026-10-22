#!/usr/bin/env bash
# The rest of .github/actions/host-tools: pnpm at package.json's version, the
# CPython lines of $RUNTIMES (one, ends or all; the free-threaded build of
# 3.14 joins on Linux) at ci/host/runtimes.env's exact versions, recorded in
# GOODFETCH_PYTHONS, then `just fetch` and the locked Playwright's browsers,
# every download before ADR 0043's deny. With ARCHITECTURE=x64 on Apple
# silicon, the CPython builds are x86-64, run under Rosetta 2.
set -euo pipefail

runtimes=${RUNTIMES:-one}
architecture=${ARCHITECTURE:-native}
root=$(cd "$(dirname "$0")/../.." && pwd)
environment=${GITHUB_ENV:?the runner sets GITHUB_ENV}

pnpm_version=$(sed -n 's/^  "packageManager": "pnpm@\(.*\)"$/\1/p' "$root/package.json")
npm install --global "pnpm@$pnpm_version"

pinned() { sed -n "s/^$1=//p" "$root/ci/host/runtimes.env"; }
cpython_311=$(pinned CPYTHON_311) cpython_312=$(pinned CPYTHON_312)
cpython_313=$(pinned CPYTHON_313) cpython_314=$(pinned CPYTHON_314)
case "$runtimes" in
  one) lines=("$cpython_314") ;;
  ends) lines=("$cpython_311" "$cpython_314") ;;
  all) lines=("$cpython_311" "$cpython_312" "$cpython_313" "$cpython_314") ;;
  *)
    echo "error: the runtimes are one, ends or all, not $runtimes" >&2
    exit 2
    ;;
esac
[[ "$(uname -s)" == Linux ]] && lines+=("${cpython_314}t")

pythons=()
for line in "${lines[@]}"; do
  if [[ "$architecture" == x64 && "$(uname -s)-$(uname -m)" == Darwin-arm64 ]]; then
    pythons+=("cpython-${line}-macos-x86_64-none")
  else
    pythons+=("$line")
  fi
done
uv python install "${pythons[@]}"
echo "GOODFETCH_PYTHONS=${pythons[*]}" >>"$environment"

cd "$root"
# Each line's wheels reach uv's cache now, so the test steps can make each
# line's environment offline, under the deny.
for python in "${pythons[@]:1}"; do
  uv sync --frozen --python "$python"
done
# The workspace's environment uses the oldest line, as users of it would.
export UV_PYTHON=${pythons[0]}
echo "UV_PYTHON=$UV_PYTHON" >>"$environment"
just fetch
# The browsers of the locked Playwright: the Release suite runs every
# Playwright test on Chromium, Firefox and WebKit (TESTING.md, Browsers), and
# none can be downloaded under the deny. On Linux, with the system libraries
# they need.
browsers=(chromium firefox webkit)
if [[ "$(uname -s)" == Linux ]]; then
  pnpm exec playwright install --with-deps "${browsers[@]}"
else
  pnpm exec playwright install "${browsers[@]}"
fi
