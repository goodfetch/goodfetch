#!/usr/bin/env bash
# Proves that the repository's static checks refuse what they must, by running
# each on a fixture under ci/proofs/ that breaks it, with the repository's own
# configuration (`just proofs`, in the Pre-merge suite). Each proof passes only
# when its check fails for the expected reason and writes nothing.
#   - ADR 0062's mocking bans: cargo-deny on each banned crate, oxlint on each
#     refused `vi` method, ruff's TID251 on `unittest.mock`,
#     `pytest.MonkeyPatch` and `_pytest.monkeypatch` (pytest-mock in uv.lock:
#     `xtask mocking-lockfile`'s own tests);
#   - ADR 0068's test-file bans: oxlint on each property that gives a test a
#     browser or context of its own, sleeps or redefines a property;
#   - snapshots never written in CI: a mismatched, a missing and an orphaned
#     snapshot for insta and Vitest, a missing and a mismatched one for
#     inline-snapshot (whose inline snapshots cannot be orphaned);
#   - zizmor refusing an Action pinned by tag (story 1-71);
#   - the supply-chain and boundary gates (#133): cargo-deny's license
#     allowlist (ADR 0018), sources, advisories and ADR 0087's workspace-wide
#     direction; each Product's bans (ci/deny/products.sh); cargo-audit, pnpm
#     audit and pip-audit (ci/audit.sh, which need their advisory databases,
#     so this runs before the egress deny); cargo-hack over feature
#     combinations; the MSRV build; cargo-shear on an unused dependency; the
#     Console packages (ADR 0087).
set -uo pipefail
cd "$(dirname "$0")/../.." || exit 2

scratch=$(mktemp -d)
trap 'rm -rf "$scratch"' EXIT

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

# ADR 0018's allowlist, through deny.toml's copy of license-allowlist.toml:
# every license off it fails, CC BY-SA too (Filter lists only), while MPL-2.0
# and the permissive ones pass.
codes=$(cargo deny --manifest-path ci/proofs/licenses/Cargo.toml --format json check licenses 2>&1 |
  jq -r 'select(.type == "diagnostic" and .fields.code == "rejected") | .fields.graphs[0].Krate.name' | sort | tr '\n' ' ')
if [[ "$codes" == "agpl-dependency busl-dependency cc-by-sa-dependency gpl-dependency lgpl-dependency sspl-dependency " ]]; then
  pass "cargo-deny refuses the GPL family, SSPL, BUSL and CC BY-SA, and allows MPL-2.0"
else
  fail cargo-deny-licenses "rejected crates: ${codes:-none}"
fi

# ADR 0087's workspace-wide direction: no crate but Foxhound's and xtask
# depends on a Foxhound crate, as a normal, build or dev dependency.
codes=$(cargo deny --manifest-path ci/proofs/foxhound-direction/Cargo.toml --format json check -A unused-wrapper bans 2>&1 |
  jq -r 'select(.type == "diagnostic" and .fields.code == "unmatched-wrapper") | .fields.message | capture("parent .(?<name>[a-z-]+) =").name' | sort | tr '\n' ' ')
if [[ "$codes" == "builds-with-it takes-it tests-with-it " ]]; then
  pass "cargo-deny refuses a Foxhound crate under any parent but Foxhound's crates and xtask"
else
  fail cargo-deny-direction "refused parents: ${codes:-none}"
fi

# Only crates.io: a git source fails. Its repository is made here, since
# Cargo takes only an absolute URL.
mkdir -p "$scratch/git-dependency/src" "$scratch/uses-git/src"
printf '[package]\nname = "git-dependency"\nversion = "0.0.0"\nedition = "2024"\nlicense = "MIT"\n' >"$scratch/git-dependency/Cargo.toml"
printf '//! A crate fetched from a git repository.\n' >"$scratch/git-dependency/src/lib.rs"
git -C "$scratch/git-dependency" init --quiet
git -C "$scratch/git-dependency" add --all
git -C "$scratch/git-dependency" -c user.name=Proof -c user.email=proof@example.invalid \
  -c commit.gpgsign=false commit --quiet --no-verify --message "A git dependency"
printf '[package]\nname = "uses-git"\nversion = "0.0.0"\nedition = "2024"\nlicense = "MIT"\n\n[dependencies]\ngit-dependency = { git = "file://%s/git-dependency" }\n\n[workspace]\n' \
  "$scratch" >"$scratch/uses-git/Cargo.toml"
printf '//! Takes a git dependency.\n' >"$scratch/uses-git/src/lib.rs"
codes=$(cargo deny --manifest-path "$scratch/uses-git/Cargo.toml" --format json check sources 2>&1 |
  jq -r 'select(.type == "diagnostic") | .fields.code' | tally)
if [[ "$codes" == "1 source-not-allowed" ]]; then
  pass "cargo-deny refuses a source other than crates.io"
else
  fail cargo-deny-sources "findings: ${codes:-none}"
fi

# What each Product ships: Collie's executable takes DuckDB, a test-only crate,
# a Foxhound crate and BoringSSL through a crate other than the relay's, and a
# test adapter only as a dev-dependency (not shipped); Foxhound's takes wreq,
# its own, and a test adapter.
output=$(DENY_FORMAT=json ci/deny/products.sh ci/proofs/product-bans/Cargo.toml 2>&1)
codes=$(jq -R -r 'if startswith("product: ") then .
  else (fromjson? | select(.type == "diagnostic" and .fields.code == "banned")
    | .fields.message | capture("crate .(?<name>[a-z-]+) =").name) end' <<<"$output" | tr '\n' ' ')
if [[ "$codes" == "product: collie duckdb goodfetch-frontier goodfetch-test-support tokio-btls product: foxhound goodfetch-scripted-model " ]]; then
  pass "each Product's bans refuse what its executable must not ship, dev-dependencies aside"
else
  fail product-bans "banned per Product: ${codes:-none}"
fi

# RustSec's advisories, through cargo-deny and cargo-audit both.
output=$(ci/audit.sh rust ci/proofs/advisories/Cargo.toml 2>&1)
status=$?
if [[ $status -ne 0 ]] && grep -q '^error\[vulnerability\]' <<<"$output" && grep -q '^Crate: *smallvec' <<<"$output"; then
  pass "cargo-deny and cargo-audit refuse a crate version with a RustSec advisory"
else
  fail rust-advisories "exit $status; cargo-deny and cargo-audit must each report smallvec"
fi

if output=$(ci/audit.sh npm ci/proofs/npm-audit 2>&1); then
  fail npm-audit "the audit passed"
elif grep -q 'lodash' <<<"$output"; then
  pass "pnpm audit refuses a package version with a published advisory"
else
  fail npm-audit "the audit failed without naming lodash"
fi

if output=$(ci/audit.sh python ci/proofs/pip-audit/requirements.txt 2>&1); then
  fail pip-audit "the audit passed"
elif grep -q '^urllib3 *1.24.1' <<<"$output"; then
  pass "pip-audit refuses a package version with a published advisory"
else
  fail pip-audit "the audit failed without naming urllib3"
fi

# cargo-hack: two features of Collie's SDK crate that clash only together.
if output=$(ci/feature-combinations.sh ci/proofs/feature-combinations/Cargo.toml 2>&1); then
  fail cargo-hack "every combination built"
elif grep -q 'error\[E0428\]' <<<"$output"; then
  pass "cargo-hack refuses features that break each other"
else
  fail cargo-hack "the check failed for another reason"
fi

# The MSRV build: an API stabilized after the rust-version fails there, and
# builds with the pinned toolchain.
if output=$(ci/msrv.sh ci/proofs/msrv/Cargo.toml 2>&1); then
  fail msrv "the MSRV build passed"
elif ! grep -q 'error\[E0658\]' <<<"$output"; then
  fail msrv "the MSRV build failed for another reason"
elif ! CARGO_TARGET_DIR=target/proofs cargo build --quiet --manifest-path ci/proofs/msrv/Cargo.toml 2>/dev/null; then
  fail msrv "the fixture does not build with the pinned toolchain either"
else
  pass "the MSRV build refuses code that needs a newer Rust"
fi

# cargo-shear (#133): a dependency a crate declares and never uses fails.
codes=$(cargo shear --format json ci/proofs/cargo-shear 2>/dev/null | jq -r '.findings[] | .code + " " + .file' | tally)
if [[ "$codes" == "1 shear/unused_dependency declares-it/Cargo.toml" ]]; then
  pass "cargo-shear refuses a dependency no code uses"
else
  fail cargo-shear "findings: ${codes:-none}"
fi

# The Console's packages (ADR 0087): Collie's build cannot resolve Foxhound's
# screens' package, which Foxhound's build declares and resolves; the lint
# configuration refuses a relative import that leaves its package.
if ! (cd ci/proofs/console-packages && pnpm install --offline --frozen-lockfile --silent); then
  fail console-packages "installing the fixture's workspace failed"
else
  if output=$(pnpm exec tsc -p ci/proofs/console-packages/collie/tsconfig.json 2>&1) ||
    ! grep -q 'TS2307' <<<"$output"; then
    fail console-packages "Collie's build resolved Foxhound's screens"
  elif ! pnpm exec tsc -p ci/proofs/console-packages/foxhound/tsconfig.json >/dev/null 2>&1; then
    fail console-packages "Foxhound's build did not resolve its screens"
  else
    pass "Collie's Console build cannot import Foxhound's screens' package"
  fi
fi
codes=$(pnpm exec oxlint -c ci/proofs/oxlint/proof.oxlintrc.json --format json ci/proofs/oxlint/console/collie/src/screens.ts 2>/dev/null |
  jq -r '.diagnostics[].code' | tally)
if [[ "$codes" == "1 import(no-relative-parent-imports)" ]]; then
  pass "oxlint refuses a relative import that leaves a Console package"
else
  fail oxlint-console "findings: ${codes:-none}"
fi

codes=$(pnpm exec oxlint -c ci/proofs/oxlint/proof.oxlintrc.json --format json ci/proofs/oxlint/mocking.test.ts 2>/dev/null |
  jq -r '.diagnostics[].code' | grep -c 'no-restricted-vi-methods')
if [[ "$codes" == 7 ]]; then
  pass "oxlint refuses vi.mock, doMock, importMock, mockObject, spyOn, stubGlobal and stubEnv"
else
  fail oxlint "$codes of the 7 refused vi methods reported"
fi

# ADR 0068's test-file bans: every browser and context comes from test
# support's fixtures, no test sleeps, and none redefines a property.
codes=$(pnpm exec oxlint -c ci/proofs/oxlint/proof.oxlintrc.json --format json ci/proofs/oxlint/contexts.spec.ts 2>/dev/null |
  jq -r '.diagnostics[] | select(.code == "eslint(no-restricted-properties)") | .message | capture("^.(?<name>[A-Za-z.]+). is restricted").name' | tally | tr '\n' ' ')
if [[ "$codes" == "1 Object.defineProperties 1 Object.defineProperty 1 Reflect.defineProperty 1 browser.newPage 1 connect 1 connectOverCDP 1 launch 1 launchPersistentContext 2 newContext 1 waitForTimeout " ]]; then
  pass "oxlint refuses a test file's own browser or context, waitForTimeout and redefined properties, and allows context.newPage()"
else
  fail oxlint-contexts "findings: ${codes:-none}"
fi

codes=$(uv run ruff check --no-cache --output-format json ci/proofs/ruff/mocking.py 2>/dev/null | jq -r '.[].code' | tally)
if [[ "$codes" == "3 TID251" ]]; then
  pass "ruff refuses unittest.mock, pytest.MonkeyPatch and _pytest.monkeypatch"
else
  fail ruff "findings: ${codes:-none}"
fi

# Each snapshot proof fails for its own reason, the message its tool gives
# that fault, and writes nothing.
snapshot_proof() {
  local name=$1 expected=$2 output
  shift 2
  if output=$("$@" 2>&1); then
    fail "$name" "the run passed"
  elif ! grep -q -- "$expected" <<<"$output"; then
    fail "$name" "the run failed without saying \"$expected\""
  elif ! unchanged; then
    fail "$name" "the run wrote a file"
  else
    pass "$name fails for its fault and writes nothing"
  fi
}
# shellcheck disable=SC2329 # called through snapshot_proof
insta_run() {
  (cd ci/proofs/insta && CI=true INSTA_UPDATE=no cargo insta test -p "$1" --check --unreferenced reject \
    --test-runner nextest --disable-nextest-doctest)
}
# shellcheck disable=SC2329 # called through snapshot_proof
vitest_run() { CI=true pnpm exec vitest run --root "ci/proofs/vitest/$1"; }
# shellcheck disable=SC2329 # called through snapshot_proof
inline_run() {
  CI=true uv run pytest -p no:randomly -p no:cacheprovider -o addopts=--import-mode=importlib -q \
    "ci/proofs/inline-snapshot/$1"
}
snapshot_proof insta-mismatched "-old snapshot" insta_run mismatched
snapshot_proof insta-missing "+new results" insta_run missing
snapshot_proof insta-orphaned "aborting because of unreferenced snapshots" insta_run orphaned
snapshot_proof vitest-mismatched "Snapshot \`greeting 1\` mismatched" vitest_run mismatched
snapshot_proof vitest-missing "Snapshot \`greeting 1\` mismatched" vitest_run missing
snapshot_proof vitest-orphaned "Obsolete snapshots found" vitest_run orphaned
snapshot_proof inline-snapshot-mismatched "assert 'goodbye' == 'hello'" inline_run mismatched
snapshot_proof inline-snapshot-missing "your snapshot is missing a value" inline_run missing

codes=$(zizmor --config zizmor.yml --format json ci/proofs/zizmor/tag-pinned.yml 2>/dev/null | jq -r '.[].ident' | tally)
if [[ "$codes" == "1 unpinned-uses" ]]; then
  pass "zizmor refuses an Action pinned by tag"
else
  fail zizmor "findings: ${codes:-none}"
fi

exit "$failed"
