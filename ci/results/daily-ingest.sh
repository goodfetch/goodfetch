#!/usr/bin/env bash
# The daily ingest (ADR 0135): every attempt of the runs finished since
# yesterday that the store lacks goes into the results store's checkout,
# chiefly the runs that ran only on the owner's machine, which no per-run
# ingest follows: `ci/results/daily-ingest.sh <store>`.
set -euo pipefail

store=${1:?usage: ci/results/daily-ingest.sh <store>}
here=$(cd "$(dirname "$0")" && pwd)

since=$(date -u -d yesterday +%F)
pending=$(cargo run --quiet --locked -p xtask -- results pending \
  --repository "${GITHUB_REPOSITORY:?}" --store "$store" --since "$since")

ingested=0
while read -r run attempt; do
  [[ -n "$run" ]] || continue
  "$here/ingest.sh" "$store" "$run" "$attempt"
  ingested=$((ingested + 1))
done <<<"$pending"
echo "Ingested $ingested run attempts finished since $since."
