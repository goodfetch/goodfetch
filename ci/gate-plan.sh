#!/usr/bin/env bash
# Prints the gate's plan for the checked-out head (`xtask gate plan`, ADR
# 0135): `ci/gate-plan.sh <form> <layer>`, where the form is `all`, `quick`
# or `docs` and the layer is the one that would run the plan's steps
# (`merge`, `preflight`, `backstop` or `hosted`). `just _gate-plan` runs it
# on the owner's machine and the gate host; the hosted tier's plan job runs
# it on its bare runner, which has no `just`.
#
# A plan made where a listing cannot run (Vitest's, Playwright's) marks every
# member it would have narrowed, so it never runs less than the gate would.
set -euo pipefail

form=${1:?usage: ci/gate-plan.sh <form> <layer>}
layer=${2:?usage: ci/gate-plan.sh <form> <layer>}
cd "$(dirname "$0")/.."

records=$(ci/gate-records.sh)
mkdir -p target "$records"
echo '{}' >target/gate-listing-tools.json
# The variables that change what the steps run or how join the host
# fingerprint (ci/gate-variables.sh).
variables=$(ci/gate-variables.sh)
# With a GitHub token in the run's environment, lint's zizmor also runs its
# online audits, which read GitHub: such a lint is never reused.
zizmor_token="${GH_TOKEN:-}${GITHUB_TOKEN:-}${ZIZMOR_GITHUB_TOKEN:-}"
hosted=()
if [[ -n ${GOODFETCH_GATE_HOSTED_RECORDS:-} ]]; then
  hosted=(--hosted-records "$GOODFETCH_GATE_HOSTED_RECORDS")
fi
optional=()
[[ -n ${GOODFETCH_GATE_FULL:-} ]] && optional+=(--full)
[[ -n $zizmor_token ]] && optional+=(--zizmor-online)

GITHUB_TOKEN="${GITHUB_TOKEN:-$(gh auth token 2>/dev/null || true)}" \
  GOODFETCH_TEST_SUITE=pre-merge GOODFETCH_TEST_TOOLS="$PWD/target/gate-listing-tools.json" \
  cargo run --quiet --bin xtask -- gate plan --records "$records" --now "$(date +%s)" \
  --form "$form" --layer "$layer" --head-repository "${GOODFETCH_GATE_REPOSITORY:-}" \
  --runner-image "${GOODFETCH_GATE_RUNNER_IMAGE:-}" ${hosted[@]+"${hosted[@]}"} \
  --environment "$variables" ${optional[@]+"${optional[@]}"}
