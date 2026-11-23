#!/usr/bin/env bash
# Destroys the job's VM (ADR 0135). Every job runs it, pass or fail; the next
# start.sh destroys a VM a cancelled job left behind.
set -euo pipefail

here=$(cd "$(dirname "$0")" && pwd)
# shellcheck source=ci/vm/common.sh
source "$here/common.sh"

limactl delete --force "$VM_NAME" >/dev/null 2>&1 || true
rm -f "$EGRESS_STATE"
