#!/usr/bin/env bash
# ADR 0043's outbound-traffic deny for a job of the pull-request tier, which
# runs in a throwaway Linux VM on the owner's machine (ADR 0135). The job's
# entry in ci/egress/endpoints.toml says what it may reach.
#
#   deny    after the VM's installs and pinned downloads (`just fetch`) and
#           before the job's first test step: resolves the job's allowed
#           hosts, loads the nftables rules of `xtask egress rules` in the VM
#           and pins those hosts in the container's /etc/hosts, so nothing is
#           looked up afterwards.
#   canary  right after the deny: the Egress canary tries its forbidden
#           connection from the job's container, and fails the job unless the
#           deny refused it.
#   check   at the job's end, whatever happened: fails the job on any other
#           connection the deny refused, named from the VM's kernel log.
#
# The rules run in the VM's own network namespace, where the rootless
# container's connections leave as the VM user's; the fixtures are local to
# the VM and never reach those rules.
set -euo pipefail

here=$(cd "$(dirname "$0")" && pwd)
root=$(cd "$here/../.." && pwd)
# shellcheck source=ci/vm/common.sh
source "$root/ci/vm/common.sh"

# Files of the deny, in the workspace: the host and the VM's /work see them.
files=target/egress

xtask() {
  "$root/ci/vm/exec.sh" cargo run --quiet --locked -p xtask -- egress "$@"
}

counters() {
  vm_shell sudo nft --json list counters table inet goodfetch_egress
}

# owner/name/.github/workflows/<file>@<ref> and the job's ID give <file>#<id>.
job_name() {
  local workflow=${GITHUB_WORKFLOW_REF:?the runner sets GITHUB_WORKFLOW_REF}
  workflow=${workflow#"${GITHUB_REPOSITORY:?the runner sets GITHUB_REPOSITORY}/.github/workflows/"}
  echo "${workflow%%@*}#${GITHUB_JOB:?the runner sets GITHUB_JOB}"
}

deny() {
  local job uid
  job=$(job_name)
  mkdir -p "$root/$files"
  xtask endpoints --job "$job" >"$root/$files/endpoints.txt"

  # A host that does not resolve stays unreachable; the step that needs it fails.
  vm_shell bash -uo pipefail -c "
    while read -r host port; do
      getent ahosts \"\$host\" | awk -v host=\"\$host\" -v port=\"\$port\" '{ print host, port, \$1 }' ||
        echo \"warning: \$host did not resolve, so the job cannot reach it\" >&2
    done <$files/endpoints.txt | sort -u
  " >"$root/$files/resolved.txt"

  uid=$(vm_shell id -u)
  xtask rules --job "$job" --resolved "$files/resolved.txt" --uid "$uid" >"$root/$files/rules.nft"
  vm_shell sudo nft -f "$files/rules.nft"
  awk '{ print $3, $1 }' "$root/$files/resolved.txt" |
    vm_shell nerdctl exec --interactive --user 0 "$CONTAINER" sh -c 'cat >>/etc/hosts'
  echo "$job" >"$EGRESS_STATE"
  echo "Outbound traffic denied for $job but to $(wc -l <"$root/$files/endpoints.txt" | tr -d ' ') allowed endpoints."
}

canary() {
  [[ -f "$EGRESS_STATE" ]] || {
    echo "error: the deny is not in place; run ci/egress/egress.sh deny first" >&2
    exit 2
  }
  counters >"$root/$files/canary-before.json"
  xtask canary attempt --outcome "$files/canary-outcome.txt"
  counters >"$root/$files/canary-after.json"
  xtask canary verdict --outcome "$files/canary-outcome.txt" \
    --before "$files/canary-before.json" --after "$files/canary-after.json"
  echo "The Egress canary was refused."
}

check() {
  if [[ ! -f "$EGRESS_STATE" ]]; then
    echo "The deny never started, so no step ran under it."
    return
  fi
  counters >"$root/$files/counters.json"
  vm_shell sudo journalctl --dmesg --no-pager --output cat >"$root/$files/kernel.log"
  xtask report --counters "$files/counters.json" --log "$files/kernel.log"
  echo "The deny refused nothing but the Egress canary."
}

case "${1:-}" in
  deny) deny ;;
  canary) canary ;;
  check) check ;;
  *)
    echo "usage: ci/egress/egress.sh deny|canary|check" >&2
    exit 2
    ;;
esac
