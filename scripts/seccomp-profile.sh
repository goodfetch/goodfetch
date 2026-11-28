#!/usr/bin/env bash
# Regenerates `packaging/oci/seccomp-profile.json` from the pinned moby version
# of Docker's default seccomp profile plus Chromium's three user-namespace rules
# (ADR 0049; issue 1260).
#
#   scripts/seccomp-profile.sh                     regenerate from the pinned default
#   scripts/seccomp-profile.sh --version <tag>     bump the pinned moby version and regenerate
#   scripts/seccomp-profile.sh --upstream <path>   regenerate from a local moby default JSON
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root"

if [[ -n "${GOODFETCH_XTASK_BIN:-}" ]]; then
  exec "$GOODFETCH_XTASK_BIN" seccomp generate "$@"
fi

exec cargo run --quiet --bin xtask -- seccomp generate "$@"
