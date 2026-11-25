#!/usr/bin/env bash
# The rest of .github/actions/host-tools: pnpm at package.json's version, the
# CPython builds of $RUNTIMES, a set of ci/runtimes.toml, at their pinned
# versions, recorded in GOODFETCH_PYTHONS, then `just fetch` and the locked
# Playwright's browsers, every download before ADR 0043's deny.
set -euo pipefail

runtimes=${RUNTIMES:-pre-merge}
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

pythons=("${lines[@]}")
uv python install "${pythons[@]}"
echo "GOODFETCH_PYTHONS=${pythons[*]}" >>"$environment"

cd "$root"
# Each line's wheels reach uv's cache now, so the test steps can make each
# line's environment offline, under the deny. A free-threaded line leaves out
# the `langchain` group: `orjson`, which it needs, has no free-threaded build
# (issue 3738).
for python in "${pythons[@]:1}"; do
  groups=()
  if [[ "$python" == *t ]]; then groups=(--no-group langchain); fi
  uv sync --frozen --python "$python" ${groups[@]+"${groups[@]}"}
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
