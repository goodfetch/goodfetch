#!/usr/bin/env bash
# The rest of .github/actions/host-tools: pnpm at package.json's version, the
# CPython lines of $RUNTIMES (one, ends or all; the free-threaded builds from
# 3.14t join on Linux), recorded in GOODFETCH_PYTHONS, then `just fetch`,
# every download before ADR 0043's deny. With ARCHITECTURE=x64 on Apple
# silicon, the CPython builds are x86-64, run under Rosetta 2.
set -euo pipefail

runtimes=${RUNTIMES:-one}
architecture=${ARCHITECTURE:-native}
root=$(cd "$(dirname "$0")/../.." && pwd)
environment=${GITHUB_ENV:?the runner sets GITHUB_ENV}

pnpm_version=$(sed -n 's/^  "packageManager": "pnpm@\(.*\)"$/\1/p' "$root/package.json")
npm install --global "pnpm@$pnpm_version"

case "$runtimes" in
  one) lines=(3.14) ;;
  ends) lines=(3.11 3.14) ;;
  all) lines=(3.11 3.12 3.13 3.14) ;;
  *)
    echo "error: the runtimes are one, ends or all, not $runtimes" >&2
    exit 2
    ;;
esac
[[ "$(uname -s)" == Linux ]] && lines+=(3.14t)

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
