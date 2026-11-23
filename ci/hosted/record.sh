#!/usr/bin/env bash
# Records the result of a gate step that a hosted tier job ran outside the
# gate (#3382), in the job's container: `ci/hosted/record.sh <form> <step>
# <outcome>`, the outcome the job's test step ended with, `success` or
# `failure`. The visual tests run in the pinned Playwright image beside the
# job's container (ci/hosted/visual.sh), which `just pre-merge <form>` cannot
# start from inside it, so their job records the step here, as the gate
# records a step it ran (`just _gate-record`): on the head the plan names,
# by the hosted layer, green on `success` and red otherwise, so `just merge`
# reuses a green one and ends red on a red one (ci/hosted-records.sh).
set -euo pipefail

usage="usage: ci/hosted/record.sh <form> <step> <outcome>"
form=${1:?$usage}
step=${2:?$usage}
outcome=${3:?$usage}
cd "$(dirname "$0")/../.."
# shellcheck source=ci/output-value.sh
source ci/output-value.sh

plan=$(ci/gate-plan.sh "$form" hosted)
action=$(output_value "$step.action" "$plan")
if [[ $action != run ]]; then
  echo "error: the plan's action for $step is $action here, so the job should not have run it" >&2
  exit 1
fi
status=fail
[[ $outcome != success ]] || status=pass
just _gate-record "${GOODFETCH_GATE_LAYER:?the job sets GOODFETCH_GATE_LAYER}" \
  "$(output_value head "$plan")" "$step" "$status"
echo "Gate step $step: recorded $status"
