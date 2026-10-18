#!/usr/bin/env bash
# Checks the repository's published policy files (stories 1-77 and 1-100):
# both licence files name "The Goodfetch contributors", the trademark policy
# exists, and SECURITY.md carries the sections ADR 0067 requires.
set -euo pipefail

failed=0
fail() {
  echo "error[repository-files] $1: $2"
  failed=1
}

for licence in LICENSE-MIT LICENSE-APACHE; do
  grep -q "The Goodfetch contributors" "$licence" 2>/dev/null ||
    fail "$licence" 'does not name "The Goodfetch contributors"'
done

[[ -s TRADEMARKS.md ]] || fail TRADEMARKS.md "the trademark policy is missing"

for section in "## Reporting a vulnerability" "## Acknowledgement" "## Coordinated disclosure" "## What counts as a vulnerability"; do
  grep -qxF "$section" SECURITY.md 2>/dev/null || fail SECURITY.md "the section \"$section\" is missing"
done
for term in "private vulnerability reporting" "three business days" "90 days" "trust boundary" "memory unsafety" "limit that never fires"; do
  grep -qiF "$term" SECURITY.md 2>/dev/null || fail SECURITY.md "it does not state \"$term\""
done

exit "$failed"
