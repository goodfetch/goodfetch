#!/usr/bin/env bash
# Proves that the live audit's own tools still refuse a published advisory
# (`just audit-live`, the daily Live check of ADR 0064), on fixtures under
# ci/proofs/ that lock a vulnerable version: pnpm audit and pip-audit, which
# read their registries' newest advisories online. The gate's tools, over
# the pinned snapshot, have their proofs in ci/proofs/run.sh, which runs
# cargo-deny and cargo-audit too.
set -uo pipefail
cd "$(dirname "$0")/../.." || exit 2

failed=0
pass() { echo "proved: $1"; }
fail() {
  echo "error[unproved] $1: $2"
  failed=1
}

if output=$(ci/audit.sh live npm ci/proofs/npm-audit 2>&1); then
  fail npm-audit "the audit passed"
elif grep -q 'lodash' <<<"$output"; then
  pass "pnpm audit refuses a package version with a published advisory"
else
  fail npm-audit "the audit failed without naming lodash"
fi

if output=$(ci/audit.sh live python ci/proofs/pip-audit/requirements.txt 2>&1); then
  fail pip-audit "the audit passed"
elif grep -q '^urllib3 *1.24.1' <<<"$output"; then
  pass "pip-audit refuses a package version with a published advisory"
else
  fail pip-audit "the audit failed without naming urllib3"
fi

exit "$failed"
