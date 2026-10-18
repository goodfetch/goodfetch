#!/usr/bin/env bash
# Proves that the repository's static checks refuse what they must, by running
# each on a fixture under ci/proofs/ that breaks it, with the repository's own
# configuration (`just proofs`, in the Pre-merge suite). Each proof passes only
# when its check fails for the expected reason and writes nothing.
#   - ADR 0062's mocking bans: cargo-deny on each banned crate, oxlint on each
#     refused `vi` method, ruff's TID251 on `unittest.mock` and
#     `pytest.MonkeyPatch` (pytest-mock in uv.lock: `xtask mocking-lockfile`'s
#     own tests);
#   - snapshots never written in CI: a mismatched, a missing and an orphaned
#     snapshot for insta and Vitest, a missing and a mismatched one for
#     inline-snapshot (whose inline snapshots cannot be orphaned);
#   - zizmor refusing an Action pinned by tag (story 1-71).
set -uo pipefail
cd "$(dirname "$0")/../.." || exit 2

failed=0
pass() { echo "proved: $1"; }
fail() {
  echo "error[unproved] $1: $2"
  failed=1
}

# Counts how many times each line appears, as "<count> <line>" lines.
tally() { sort | uniq -c | sed -E 's/^ +//'; }

# Every proof must leave the fixtures as they were: no snapshot written or
# changed. Build outputs and caches are not fixtures.
fixtures() {
  find ci/proofs -type f -not -path '*/node_modules/*' -not -path '*/target/*' \
    -not -path '*/__pycache__/*' -not -name Cargo.lock -exec shasum {} + | sort
}
before=$(fixtures)
unchanged() { [[ "$(fixtures)" == "$before" ]]; }

# cargo-deny reads the root deny.toml (it reads deny.toml in the current folder).
codes=$(cargo deny --manifest-path ci/proofs/cargo-deny/Cargo.toml --format json check bans 2>&1 |
  jq -r 'select(.type == "diagnostic" and .fields.code == "banned") | .fields.message | capture("crate .(?<name>[a-z]+) =").name' | sort | tr '\n' ' ')
if [[ "$codes" == "faux mockall mockito mry wiremock " ]]; then
  pass "cargo-deny bans mockall, mockito, wiremock, faux and mry"
else
  fail cargo-deny "banned crates found: ${codes:-none}"
fi

codes=$(pnpm exec oxlint -c ci/proofs/oxlint/proof.oxlintrc.json --format json ci/proofs/oxlint/mocking.test.ts 2>/dev/null |
  jq -r '.diagnostics[].code' | grep -c 'no-restricted-vi-methods')
if [[ "$codes" == 7 ]]; then
  pass "oxlint refuses vi.mock, doMock, importMock, mockObject, spyOn, stubGlobal and stubEnv"
else
  fail oxlint "$codes of the 7 refused vi methods reported"
fi

codes=$(uv run ruff check --no-cache --output-format json ci/proofs/ruff/mocking.py 2>/dev/null | jq -r '.[].code' | tally)
if [[ "$codes" == "2 TID251" ]]; then
  pass "ruff refuses unittest.mock and pytest.MonkeyPatch"
else
  fail ruff "findings: ${codes:-none}"
fi

for fault in mismatched missing orphaned; do
  if (cd ci/proofs/insta && CI=true INSTA_UPDATE=no cargo insta test -p "$fault" --check --unreferenced reject \
    --test-runner nextest --disable-nextest-doctest >/dev/null 2>&1); then
    fail "insta-$fault" "the run passed"
  elif ! unchanged; then
    fail "insta-$fault" "the run wrote a file"
  else
    pass "insta fails its $fault-snapshot case and writes nothing"
  fi

  if CI=true pnpm exec vitest run --root "ci/proofs/vitest/$fault" >/dev/null 2>&1; then
    fail "vitest-$fault" "the run passed"
  elif ! unchanged; then
    fail "vitest-$fault" "the run wrote a file"
  else
    pass "Vitest fails its $fault-snapshot case and writes nothing"
  fi
done

for fault in mismatched missing; do
  if CI=true uv run pytest -p no:randomly -p no:cacheprovider -o addopts=--import-mode=importlib -q \
    "ci/proofs/inline-snapshot/$fault" >/dev/null 2>&1; then
    fail "inline-snapshot-$fault" "the run passed"
  elif ! unchanged; then
    fail "inline-snapshot-$fault" "the run wrote a file"
  else
    pass "inline-snapshot fails its $fault-snapshot case and writes nothing"
  fi
done

codes=$(zizmor --config zizmor.yml --format json ci/proofs/zizmor/tag-pinned.yml 2>/dev/null | jq -r '.[].ident' | tally)
if [[ "$codes" == "1 unpinned-uses" ]]; then
  pass "zizmor refuses an Action pinned by tag"
else
  fail zizmor "findings: ${codes:-none}"
fi

exit "$failed"
