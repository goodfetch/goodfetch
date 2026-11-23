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
#   - in the job's Lima VM on the owner's machine (ADR 0135): nftables rules
#     in the VM, its kernel log the record;
#   - in the job's container on a standard hosted Linux runner (the hosted
#     tier, ADR 0135's amendment of 2026-11-22): nftables rules in the
#     container's network namespace, set from the runner, so the runner's
#     own agent keeps its connection; the runner's kernel log the record;
#   - on a GitHub-hosted Linux runner without the container: the product's
#     steps run in a network
#     namespace whose nftables rules refuse all but the allowed endpoints, so
#     the runner's own agent keeps its connection; its kernel log the record;
#   - on macOS without a VM there is no deny: the runner's standard user
#     cannot change the firewall, and a sandbox profile would stop the
#     browser's and Plugins' own sandboxes, so the Release suite's macOS
#     cells run outside it (ADR 0043's amendment of 2026-11-22);
#   - on a GitHub-hosted Windows runner: Windows Firewall rules block the
#     product's programs from all but loopback and the allowed endpoints;
#     the firewall's log of dropped packets is the record (ci/egress/windows.ps1).
# A test step after the deny runs its command through ci/vm/exec.sh in a VM,
# ci/hosted/exec.sh in the hosted tier's container, or ci/egress/under-deny.sh
# otherwise; `xtask egress check` enforces all three.
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
  (cd "$root" && cargo run --quiet --locked --bin xtask -- egress "$@")
}

# xtask where the product runs, under the deny: offline.
denied_xtask() {
  (cd "$root" && "$here/under-deny.sh" cargo run --quiet --locked --offline --bin xtask -- egress "$@")
}

### In the job's VM on the owner's machine.

vm_xtask() {
  "$root/ci/vm/exec.sh" cargo run --quiet --locked --bin xtask -- egress "$@"
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

### In the hosted tier's container on a standard hosted Linux runner.

container_xtask() {
  "$root/ci/hosted/exec.sh" cargo run --quiet --locked --bin xtask -- egress "$@"
}

# A command in the container's network namespace, run from the runner, whose
# rules the container's own root cannot see change.
container_netns() {
  local pid
  pid=$(docker inspect --format '{{.State.Pid}}' "$CONTAINER")
  sudo nsenter --target "$pid" --net -- "$@"
}

container_counters() {
  container_netns nft --json list counters table inet goodfetch_egress
}

container_deny() {
  local job uid
  job=$(job_name)
  mkdir -p "$root/$files"
  container_xtask endpoints --job "$job" >"$root/$files/endpoints.txt"

  # A host that does not resolve stays unreachable; the step that needs it fails.
  docker exec "$CONTAINER" bash -uo pipefail -c "
    while read -r host port; do
      getent ahosts \"\$host\" | awk -v host=\"\$host\" -v port=\"\$port\" '{ print host, port, \$1 }' ||
        echo \"warning: \$host did not resolve, so the job cannot reach it\" >&2
    done </work/$files/endpoints.txt | LC_ALL=C sort -u
  " >"$root/$files/resolved.txt"

  uid=$(docker exec "$CONTAINER" id -u)
  container_xtask rules --job "$job" --resolved "$files/resolved.txt" --uid "$uid" >"$root/$files/rules.nft"
  if ! command -v nft >/dev/null; then
    sudo apt-get install -y -qq nftables
  fi
  # The container's kernel log lines reach the runner's log.
  sudo sysctl -q -w net.netfilter.nf_log_all_netns=1
  container_netns nft -f "$root/$files/rules.nft"
  awk '{ print $3, $1 }' "$root/$files/resolved.txt" |
    docker exec --interactive --user 0 "$CONTAINER" sh -c 'cat >>/etc/hosts'
  echo "$job" >"$EGRESS_STATE"
  echo "Outbound traffic denied for $job but to $(wc -l <"$root/$files/endpoints.txt" | tr -d ' ') allowed endpoints."
}

container_canary() {
  [[ -f "$EGRESS_STATE" ]] || {
    echo "error: the deny is not in place; run ci/egress/egress.sh deny first" >&2
    exit 2
  }
  container_counters >"$root/$files/canary-before.json"
  container_xtask canary attempt --outcome "$files/canary-outcome.txt"
  container_counters >"$root/$files/canary-after.json"
  container_xtask canary verdict --outcome "$files/canary-outcome.txt" \
    --before "$files/canary-before.json" --after "$files/canary-after.json"
  echo "The Egress canary was refused."
}

container_check() {
  if [[ ! -f "$EGRESS_STATE" ]]; then
    echo "The deny never started, so no step ran under it."
    return
  fi
  container_counters >"$root/$files/counters.json"
  kernel_log
  container_xtask report --counters "$files/counters.json" --log "$files/kernel.log"
  echo "The deny refused nothing but the Egress canary."
}

### On a GitHub-hosted Linux runner without the container: a network namespace.

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

# The runner's kernel log, which names each connection a deny on it refused.
kernel_log() {
  # shellcheck disable=SC2024 # the runner's user writes the copy; root reads the log
  sudo journalctl --dmesg --no-pager --output cat >"$root/$files/kernel.log"
}

namespace_check() {
  namespace_counters >"$root/$files/counters.json"
  kernel_log
  host_xtask report --counters "$files/counters.json" --log "$files/kernel.log"
  echo "The deny refused nothing but the Egress canary."
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

if [[ "$1" != deny && "$backend" != vm && "$backend" != container && ! -f "$DENY_STATE" ]]; then
  if [[ "$1" == check ]]; then
    echo "The deny never started, so no step ran under it."
    exit 0
  fi
  echo "error: the deny is not in place; run ci/egress/egress.sh deny first" >&2
  exit 2
fi
"${backend}_$1"
