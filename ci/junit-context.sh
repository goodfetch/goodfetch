#!/usr/bin/env bash
# Writes target/junit/context.json, which the results store's ingest reads
# beside a job's JUnit files (ADR 0064): the commit the job checked out and
# tested, with its tree (a run's head may be another commit), the job's ID,
# the order and
# `Entropy` seeds its tests ran with (GOODFETCH_ORDER_SEED and
# GOODFETCH_ENTROPY_SEED, when set) and, after `just repeat`, how many times
# each test ran: `ci/junit-context.sh [<count>]`. It also copies nextest's
# JUnit files into target/junit/ when nextest ran on the host rather than in
# the job's VM. Run on the host, pass or fail, before the JUnit upload.
set -euo pipefail

repeat=${1:-}

mkdir -p target/junit
# A job that copied nextest's file itself (the VM, ci/release/cell.sh) keeps
# its copy, so no test run is counted twice.
copied=(target/junit/nextest-*.xml)
if [[ ! -f "${copied[0]}" ]]; then
  for report in target/nextest/*/junit.xml; do
    [[ -f "$report" ]] || continue
    profile=$(basename "$(dirname "$report")")
    cp "$report" "target/junit/nextest-$profile.xml"
  done
fi

jq --null-input \
  --arg commit "$(git rev-parse HEAD)" \
  --arg tree "$(git rev-parse 'HEAD^{tree}')" \
  --arg job "${GITHUB_JOB:?the runner sets GITHUB_JOB}" \
  --arg order "${GOODFETCH_ORDER_SEED:-}" \
  --arg entropy "${GOODFETCH_ENTROPY_SEED:-}" \
  --arg repeat "$repeat" \
  '{
    commit: $commit,
    tree: $tree,
    job: $job,
    order_seed: (if $order == "" then null else $order end),
    entropy_seed: (if $entropy == "" then null else $entropy end),
    repeat: (if $repeat == "" then null else ($repeat | tonumber) end)
  }' >target/junit/context.json
