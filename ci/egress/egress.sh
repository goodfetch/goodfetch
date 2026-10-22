#!/usr/bin/env bash
# ADR 0043's outbound-traffic deny for a job of CI. The job's entry in
# ci/egress/endpoints.toml says what it may reach.
#
#   deny    after the job's installs and pinned downloads and before its
#           first test step: puts the deny in place for the job's product.
#   canary  right after the deny: the Egress canary tries its forbidden
#           connection from where the product runs, and fails the job unless
#           the deny refused it and its record shows so.
#   check   at the job's end, whatever happened: fails the job on any other
#           connection the deny refused, named from the deny's record.
#
# Where the job runs decides how (TESTING.md, Egress and hostile input):
#   - in the job's Lima VM on the owner's machine (the pull-request tier, ADR
#     0135): nftables rules in the VM, its kernel log the record;
#   - on a GitHub-hosted Linux runner: the product's steps run in a network
#     namespace whose nftables rules refuse all but the allowed endpoints, so
#     the runner's own agent keeps its connection; its kernel log the record;
#   - on macOS without a VM (the owner's-machine cells of the Release suite,
#     ADR 0108): the product's steps run under a sandbox profile that refuses
#     every connection but loopback, since the runner's standard user cannot
#     change the firewall. The sandbox's refusal, EPERM, is the canary's
#     record; macOS keeps no record of refusals that user can read, so the
#     end-of-job check cannot name them, and a refused connection fails only
#     the step that made it. The allowed endpoints cannot be named in the
#     profile, so every download comes before the deny;
#   - on a GitHub-hosted Windows runner: Windows Firewall rules block the
#     product's programs from all but loopback and the allowed endpoints;
#     the firewall's log of dropped packets is the record (ci/egress/windows.ps1).
# A test step after the deny runs its command through ci/vm/exec.sh in a VM,
# or ci/egress/under-deny.sh otherwise; `xtask egress check` enforces both.
set -euo pipefail

here=$(cd "$(dirname "$0")" && pwd)
root=$(cd "$here/../.." && pwd)
# shellcheck source=ci/egress/host.sh
source "$here/host.sh"

# Files of the deny, in the workspace.
files=target/egress

# owner/name/.github/workflows/<file>@<ref> and the job's ID give <file>#<id>.
job_name() {
  local workflow=${GITHUB_WORKFLOW_REF:?the runner sets GITHUB_WORKFLOW_REF}
  workflow=${workflow#"${GITHUB_REPOSITORY:?the runner sets GITHUB_REPOSITORY}/.github/workflows/"}
  echo "${workflow%%@*}#${GITHUB_JOB:?the runner sets GITHUB_JOB}"
}

# xtask on the runner's own system, before the deny.
host_xtask() {
  (cd "$root" && cargo run --quiet --locked -p xtask -- egress "$@")
}

# xtask where the product runs, under the deny: offline.
denied_xtask() {
  (cd "$root" && "$here/under-deny.sh" cargo run --quiet --locked --offline -p xtask -- egress "$@")
}

### In the job's VM on the owner's machine.

vm_xtask() {
  "$root/ci/vm/exec.sh" cargo run --quiet --locked -p xtask -- egress "$@"
}

vm_counters() {
  vm_shell sudo nft --json list counters table inet goodfetch_egress
}

vm_deny() {
  local job uid
  job=$(job_name)
  mkdir -p "$root/$files"
  vm_xtask endpoints --job "$job" >"$root/$files/endpoints.txt"

  # A host that does not resolve stays unreachable; the step that needs it fails.
  vm_shell bash -uo pipefail -c "
    while read -r host port; do
      getent ahosts \"\$host\" | awk -v host=\"\$host\" -v port=\"\$port\" '{ print host, port, \$1 }' ||
        echo \"warning: \$host did not resolve, so the job cannot reach it\" >&2
    done <$files/endpoints.txt | LC_ALL=C sort -u
  " >"$root/$files/resolved.txt"

  uid=$(vm_shell id -u)
  vm_xtask rules --job "$job" --resolved "$files/resolved.txt" --uid "$uid" >"$root/$files/rules.nft"
  vm_shell sudo nft -f "$files/rules.nft"
  awk '{ print $3, $1 }' "$root/$files/resolved.txt" |
    vm_shell nerdctl exec --interactive --user 0 "$CONTAINER" sh -c 'cat >>/etc/hosts'
  echo "$job" >"$EGRESS_STATE"
  echo "Outbound traffic denied for $job but to $(wc -l <"$root/$files/endpoints.txt" | tr -d ' ') allowed endpoints."
}

vm_canary() {
  [[ -f "$EGRESS_STATE" ]] || {
    echo "error: the deny is not in place; run ci/egress/egress.sh deny first" >&2
    exit 2
  }
  vm_counters >"$root/$files/canary-before.json"
  vm_xtask canary attempt --outcome "$files/canary-outcome.txt"
  vm_counters >"$root/$files/canary-after.json"
  vm_xtask canary verdict --outcome "$files/canary-outcome.txt" \
    --before "$files/canary-before.json" --after "$files/canary-after.json"
  echo "The Egress canary was refused."
}

vm_check() {
  if [[ ! -f "$EGRESS_STATE" ]]; then
    echo "The deny never started, so no step ran under it."
    return
  fi
  vm_counters >"$root/$files/counters.json"
  vm_shell sudo journalctl --dmesg --no-pager --output cat >"$root/$files/kernel.log"
  vm_xtask report --counters "$files/counters.json" --log "$files/kernel.log"
  echo "The deny refused nothing but the Egress canary."
}

### On a GitHub-hosted Linux runner: a network namespace.

namespace_counters() {
  sudo ip netns exec "$NAMESPACE" nft --json list counters table inet goodfetch_egress
}

namespace_deny() {
  local job
  job=$(job_name)
  mkdir -p "$root/$files"
  host_xtask endpoints --job "$job" >"$root/$files/endpoints.txt"
  # IPv4 only: the namespace's link to the runner carries no IPv6 route.
  while read -r host port; do
    getent ahostsv4 "$host" | awk -v host="$host" -v port="$port" '{ print host, port, $1 }' ||
      echo "warning: $host did not resolve, so the job cannot reach it" >&2
  done <"$root/$files/endpoints.txt" | LC_ALL=C sort -u >"$root/$files/resolved.txt"
  host_xtask rules --job "$job" --resolved "$files/resolved.txt" --uid "$(id -u)" >"$root/$files/rules.nft"
  if ! command -v nft >/dev/null; then
    sudo apt-get install -y -qq nftables
  fi

  # The namespace, linked to the runner by a veth pair, its traffic masqueraded
  # out of the runner; its own nftables rules do the refusing, so the runner's
  # rules only forward it. Its kernel log lines reach the runner's log.
  sudo ip netns add "$NAMESPACE"
  sudo ip link add "$HOST_LINK" type veth peer name "$JOB_LINK"
  sudo ip link set "$JOB_LINK" netns "$NAMESPACE"
  sudo ip addr add "$HOST_ADDRESS/30" dev "$HOST_LINK"
  sudo ip link set "$HOST_LINK" up
  sudo ip netns exec "$NAMESPACE" ip link set lo up
  sudo ip netns exec "$NAMESPACE" ip addr add "$JOB_ADDRESS/30" dev "$JOB_LINK"
  sudo ip netns exec "$NAMESPACE" ip link set "$JOB_LINK" up
  sudo ip netns exec "$NAMESPACE" ip route add default via "$HOST_ADDRESS"
  sudo sysctl -q -w net.ipv4.ip_forward=1 net.netfilter.nf_log_all_netns=1
  sudo iptables -I FORWARD 1 -i "$HOST_LINK" -j ACCEPT
  sudo iptables -I FORWARD 1 -o "$HOST_LINK" -m conntrack --ctstate RELATED,ESTABLISHED -j ACCEPT
  sudo iptables -t nat -A POSTROUTING -s "$JOB_ADDRESS/30" -j MASQUERADE
  sudo ip netns exec "$NAMESPACE" nft -f "$root/$files/rules.nft"

  # Names resolve from the namespace's own hosts file, pinned before the
  # deny; there is no resolver to ask after it.
  sudo mkdir -p "/etc/netns/$NAMESPACE"
  {
    printf '127.0.0.1 localhost\n::1 localhost\n'
    awk '{ print $3, $1 }' "$root/$files/resolved.txt"
  } | sudo tee "/etc/netns/$NAMESPACE/hosts" >/dev/null
  echo "nameserver 127.0.0.1" | sudo tee "/etc/netns/$NAMESPACE/resolv.conf" >/dev/null

  echo "$job" >"$DENY_STATE"
  echo "Outbound traffic denied for $job but to $(wc -l <"$root/$files/endpoints.txt" | tr -d ' ') allowed endpoints."
}

namespace_canary() {
  namespace_counters >"$root/$files/canary-before.json"
  denied_xtask canary attempt --outcome "$files/canary-outcome.txt"
  namespace_counters >"$root/$files/canary-after.json"
  host_xtask canary verdict --outcome "$files/canary-outcome.txt" \
    --before "$files/canary-before.json" --after "$files/canary-after.json"
  echo "The Egress canary was refused."
}

namespace_check() {
  namespace_counters >"$root/$files/counters.json"
  # shellcheck disable=SC2024 # the runner's user writes the copy; root reads the log
  sudo journalctl --dmesg --no-pager --output cat >"$root/$files/kernel.log"
  host_xtask report --counters "$files/counters.json" --log "$files/kernel.log"
  echo "The deny refused nothing but the Egress canary."
}

### On macOS without a VM: a sandbox profile.

sandbox_deny() {
  local job
  job=$(job_name)
  mkdir -p "$root/$files"
  host_xtask endpoints --job "$job" >"$root/$files/endpoints.txt"
  # Loopback only. A sandbox profile names no remote host but localhost, so
  # the allowed endpoints stay unreachable: every download comes before the
  # deny. Name lookups through mDNSResponder are refused too.
  cat >"$SANDBOX_PROFILE" <<'PROFILE'
(version 1)
(allow default)
(deny network-outbound)
(allow network-outbound (remote ip "localhost:*"))
(allow network-outbound (remote unix-socket))
(deny network-outbound (remote unix-socket (path-literal "/private/var/run/mDNSResponder")))
PROFILE
  echo "$job" >"$DENY_STATE"
  if [[ -s "$root/$files/endpoints.txt" ]]; then
    echo "The sandbox admits loopback only, so these allowed endpoints stay unreachable under it:"
    sed 's/^/  - /' "$root/$files/endpoints.txt"
  fi
  echo "Outbound traffic denied for $job but to loopback."
}

sandbox_canary() {
  denied_xtask canary attempt --outcome "$files/canary-outcome.txt"
  host_xtask canary verdict --outcome "$files/canary-outcome.txt" --sandbox
  echo "The Egress canary was refused by the sandbox."
}

sandbox_check() {
  echo "macOS keeps no record of the sandbox's refusals that the runner's user can read: a refused connection failed the step that made it, if that step did not handle the refusal."
}

### Dispatch.

backend=$(deny_backend)
case "${1:-}" in
  deny | canary | check) ;;
  *)
    echo "usage: ci/egress/egress.sh deny|canary|check" >&2
    exit 2
    ;;
esac

if [[ "$backend" == windows ]]; then
  pwsh -NoProfile -NonInteractive -File "$here/windows.ps1" "$1" "$(job_name)"
  exit
fi

if [[ "$1" != deny && "$backend" != vm && ! -f "$DENY_STATE" ]]; then
  if [[ "$1" == check ]]; then
    echo "The deny never started, so no step ran under it."
    exit 0
  fi
  echo "error: the deny is not in place; run ci/egress/egress.sh deny first" >&2
  exit 2
fi
"${backend}_$1"
