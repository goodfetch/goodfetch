#!/usr/bin/env bash
# Ingests one run attempt into the results store's checkout (ADR 0064):
# `ci/results/ingest.sh <store> <run> <attempt>`. Downloads the run's JUnit
# artifacts once per run (GH_TOKEN with `actions: read`), then writes the
# attempt's file with `xtask results ingest`, which keeps only the artifacts
# uploaded during that attempt. Runs outside ADR 0043's deny: it runs none
# of the product.
set -euo pipefail

store=${1:?usage: ci/results/ingest.sh <store> <run> <attempt>}
run=${2:?usage: ci/results/ingest.sh <store> <run> <attempt>}
attempt=${3:?usage: ci/results/ingest.sh <store> <run> <attempt>}

folder="target/artifacts/$run"
if [[ ! -d "$folder" ]]; then
  mkdir -p "$folder"
  # A run with no JUnit artifact has nothing to download, and gh fails on
  # that; the ingest then fails on any JUnit artifact it could not find.
  gh run download "$run" --repo "${GITHUB_REPOSITORY:?}" --dir "$folder" --pattern 'junit-*' ||
    echo "Run $run has no JUnit artifact to download."
fi

cargo run --quiet --locked --bin xtask -- results ingest \
  --repository "$GITHUB_REPOSITORY" --run-id "$run" --attempt "$attempt" \
  --artifacts "$folder" --store "$store"
