#!/usr/bin/env bash
# Runs a step's command where ADR 0043's deny applies, on a runner's own
# system (ci/egress/egress.sh says how each is denied):
#   ci/egress/under-deny.sh <command>…
# On a hosted Linux runner, in the deny's network namespace, as the runner's
# own user; on Windows, as is, since the firewall's rules block the
# product's programs wherever they run.
# Cargo, uv and pnpm run offline, from what the job fetched before the deny.
# It refuses to run before the deny is in place. In the owner's machine's VM,
# steps run through ci/vm/exec.sh instead.
set -euo pipefail

here=$(cd "$(dirname "$0")" && pwd)
# shellcheck source=ci/egress/host.sh
source "$here/host.sh"

[[ $# -gt 0 ]] || {
  echo "usage: ci/egress/under-deny.sh <command>…" >&2
  exit 2
}
[[ -f "$DENY_STATE" ]] || {
  echo "error: the deny is not in place; run ci/egress/egress.sh deny first" >&2
  exit 2
}
export CARGO_NET_OFFLINE=true UV_OFFLINE=1 npm_config_offline=true

backend=$(deny_backend)
case "$backend" in
  namespace)
    exec sudo --preserve-env ip netns exec "$NAMESPACE" \
      setpriv --reuid="$(id -u)" --regid="$(id -g)" --init-groups -- env "PATH=$PATH" "$@"
    ;;
  windows) exec "$@" ;;
  vm)
    echo "error: in the job's VM, run the step through ci/vm/exec.sh" >&2
    exit 2
    ;;
esac
