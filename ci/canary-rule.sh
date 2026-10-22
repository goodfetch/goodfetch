#!/usr/bin/env bash
# Shows the Secret canaries' gitleaks rule (ADR 0067) catching what it exists
# for, and passing what it must pass (`just lint`, through ci/tool.sh):
#
# - each artifact of test-support/fixtures/canaries/artifacts/leaked/ fails a
#   scan on its own, copied out of the repository: a canary raw,
#   JSON-escaped, URL-encoded, in base64 (inside a Basic header too) and in
#   hex;
# - kinds.log fails it with every rule the file declares, one per kind;
# - the clean artifact passes;
# - a scan of the artifacts where they are committed passes, which the
#   rule's allowlist gives a scan of commits.
#
# The rule is test-support/fixtures/canaries/gitleaks.toml, which
# `xtask codegen` generates from the kinds and `--check` keeps current.
set -euo pipefail

root=$(cd "$(dirname "$0")/.." && pwd)
rule="$root/test-support/fixtures/canaries/gitleaks.toml"
artifacts="test-support/fixtures/canaries/artifacts"
scratch=$(mktemp -d)
trap 'rm -rf "$scratch"' EXIT

failed=0

# Scans `folder` with the rule, writing the JSON report to `report`, and
# prints gitleaks' exit code: 1 when it found a canary, 0 when it found none.
scan() {
  local folder=$1 report=$2 code=0
  (cd "$root" && gitleaks dir "$folder" --config "$rule" --no-banner --redact \
    --log-level error --report-format json --report-path "$report") >/dev/null 2>&1 || code=$?
  echo "$code"
}

expect() {
  local what=$1 wanted=$2 code=$3
  if [[ "$code" != "$wanted" ]]; then
    echo "error[canary-rule] $what: gitleaks exited $code, not $wanted"
    failed=1
  fi
}

for artifact in "$root/$artifacts"/leaked/*; do
  name=$(basename "$artifact")
  mkdir "$scratch/$name.d"
  cp "$artifact" "$scratch/$name.d/"
  expect "$artifacts/leaked/$name" 1 "$(scan "$scratch/$name.d" "$scratch/$name.json")"
done

# Both in byte order, whatever the locale's collation.
declared=$(sed -n 's/^id = "\(.*\)"$/\1/p' "$rule" | LC_ALL=C sort)
found=$(jq -r '.[].RuleID' "$scratch/kinds.log.json" | LC_ALL=C sort -u)
if [[ "$declared" != "$found" ]]; then
  echo "error[canary-rule] $artifacts/leaked/kinds.log: the rules found differ from the rules declared"
  diff <(echo "$declared") <(echo "$found") || true
  failed=1
fi

mkdir "$scratch/clean"
cp "$root/$artifacts"/clean/* "$scratch/clean/"
expect "$artifacts/clean" 0 "$(scan "$scratch/clean" "$scratch/clean.json")"

expect "$artifacts where they are committed" 0 "$(scan "$artifacts" "$scratch/committed.json")"

exit "$failed"
