#!/usr/bin/env bash
# The daily ingest (ADR 0135): every attempt of the finished runs the store
# lacks goes into the results store's checkout, chiefly the runs that ran
# only on the owner's machine, which no per-run ingest follows:
# `ci/results/daily-ingest.sh <store>`. The runs are listed from the store's
# newest month, so a missed day or a run queued for a day loses nothing.
set -euo pipefail

store=${1:?usage: ci/results/daily-ingest.sh <store>}
here=$(cd "$(dirname "$0")" && pwd)

pending=$(cargo run --quiet --locked --bin xtask -- results pending \
  --repository "${GITHUB_REPOSITORY:?}" --store "$store")

ingested=0
while read -r run attempt; do
  [[ -n "$run" ]] || continue
  "$here/ingest.sh" "$store" "$run" "$attempt"
  ingested=$((ingested + 1))
done <<<"$pending"
echo "Ingested $ingested run attempts the store lacked."
