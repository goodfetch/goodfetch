#!/usr/bin/env bash
# The rest of .github/actions/host-tools: pnpm at package.json's version, the
# CPython builds of $RUNTIMES, a set of ci/runtimes.toml, at their pinned
# versions, recorded in GOODFETCH_PYTHONS, then `just fetch` and the locked
# Playwright's browsers, every download before ADR 0043's deny. With ARCHITECTURE=x64 on Apple
# silicon, the CPython builds are x86-64, run under Rosetta 2.
set -euo pipefail

runtimes=${RUNTIMES:-pre-merge}
architecture=${ARCHITECTURE:-native}
root=$(cd "$(dirname "$0")/../.." && pwd)
environment=${GITHUB_ENV:?the runner sets GITHUB_ENV}

pnpm_version=$(sed -n 's/^  "packageManager": "pnpm@\(.*\)"$/\1/p' "$root/package.json")
npm install --global "pnpm@$pnpm_version"

# uv's request of each CPython build: its version, with `t` for a
# free-threaded one, in the set's order.
ids=$("$root/ci/host/runtimes.sh" ids "$runtimes")
lines=()
for id in $ids; do
  [[ "$id" == cpython-* ]] || continue
  version=$("$root/ci/host/runtimes.sh" version "$id")
  if [[ "$id" == *t ]]; then version="${version}t"; fi
  lines+=("$version")
done
if [[ ${#lines[@]} -eq 0 ]]; then
  echo "error: the set $runtimes holds no CPython build" >&2
  exit 2
fi

pythons=()
for line in "${lines[@]}"; do
  if [[ "$architecture" == x64 && "$(uname -s)-$(uname -m)" == Darwin-arm64 ]]; then
    # uv names a free-threaded build `+freethreaded` in a full request.
    if [[ "$line" == *t ]]; then line="${line%t}+freethreaded"; fi
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
# The workspace's environment uses the set's first build, its oldest line,
# as users of it would.
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
