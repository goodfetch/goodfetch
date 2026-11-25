#!/usr/bin/env bash
# Refuses Rust code under crates/ that copies code the tree already holds
# (`just lint`; TESTING.md, Quality register, Floor). jscpd finds each pair of
# blocks of at least 15 lines and 100 tokens that match token for token, and
# ci/copies.json lists the copies `main` held when the check came, each by a
# fingerprint of its text, which survives a moved line. The check fails on a
# copy the list does not hold, naming both sides, and on a listed copy the
# tree no longer holds, so the list only shrinks.
#
#   ci/copies.sh            the check
#   ci/copies.sh --shrink   removes from the list the copies that are gone
#                           (`just copies-shrink`); it never adds one
#
# A copy a decision asks for is not listed: it sits between
# `// jscpd:ignore-start` and `// jscpd:ignore-end`, with a comment that names
# the decision, where a review reads it.
#
# Left out: generated code, tests, benches, fixtures, the test-support and
# conformance crates, and the two Rust SDK crates, whose façades mirror each
# other by contract (ADR 0095).
#
# The check reads the tree alone, never the change's base: the gate reuses a
# green lint on the same tree, and the Release suite's checkout has no
# `origin/main`. The proofs (ci/proofs/run.sh) give it a folder and a list of
# their own: `ci/copies.sh [--shrink] <folder> <list>`.
set -euo pipefail
cd "$(dirname "$0")/.."

shrink=0
if [[ ${1:-} == --shrink ]]; then
  shrink=1
  shift
fi
scanned=${1:-crates}
list=${2:-ci/copies.json}

left_out='**/generated/**,**/tests/**,**/benches/**,**/fixtures/**,**/test/**,**/dist/**,**/tests.rs,**/goodfetch-test-support/**,**/goodfetch-conformance/**,**/goodfetch-collie/src/**,**/goodfetch-foxhound/src/**'

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

# One scan: the report marks each copy the list does not hold, and the list's
# copy in $work becomes the list of what the tree holds now.
cp "$list" "$work/found.json"
if ! pnpm exec jscpd "$scanned" --format rust --ignore "$left_out" \
  --min-lines 15 --min-tokens 100 --fail-on-empty \
  --reporters json --output "$work" \
  --baseline "$work/found.json" --update-baseline > "$work/jscpd.log" 2>&1; then
  cat "$work/jscpd.log" >&2
  echo "error[copies] jscpd could not scan $scanned" >&2
  exit 2
fi

# The list's entries the tree no longer holds, or holds fewer times.
gone=$(jq -n --slurpfile listed "$list" --slurpfile found "$work/found.json" \
  '[$listed[0].fingerprints | to_entries[] | select(.value > ($found[0].fingerprints[.key] // 0))] | length')

if [[ $shrink == 1 ]]; then
  jq --slurpfile found "$work/found.json" \
    '.fingerprints |= with_entries(
       .value = ([.value, ($found[0].fingerprints[.key] // 0)] | min)
       | select(.value > 0))' "$list" > "$work/shrunk.json"
  cat "$work/shrunk.json" > "$list"
  echo "$list: $gone of its copies are gone from the tree and were removed."
  exit 0
fi

failed=0
unlisted=$(jq -r --arg root "$scanned" '.duplicates[] | select(.isNew)
  | "error[copies] \($root)/\(.firstFile.name):\(.firstFile.start)-\(.firstFile.end) copies \($root)/\(.secondFile.name):\(.secondFile.start)-\(.secondFile.end) (\(.lines) lines)"' \
  "$work/jscpd-report.json")
if [[ -n $unlisted ]]; then
  echo "$unlisted" >&2
  echo "Use the code that is already there. A copy a decision asks for goes between \`// jscpd:ignore-start\` and \`// jscpd:ignore-end\`, with a comment that names the decision (ci/copies.sh)." >&2
  failed=1
fi
if [[ $gone != 0 ]]; then
  echo "error[copies] $gone of the copies in $list are gone from the tree: run \`just copies-shrink\` and commit the list" >&2
  failed=1
fi
exit "$failed"
