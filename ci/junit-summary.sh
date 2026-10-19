#!/usr/bin/env bash
# Writes the failed tests of a failing job to its page (TESTING.md: a failing
# job writes a summary of its failed tests), from every JUnit file in
# target/junit/ and its repeat-<count>/ folders (`just stress`). Runs on the
# host after the job's VM has run the tests.
set -euo pipefail

summary=${GITHUB_STEP_SUMMARY:-/dev/stdout}
failed=""
for report in target/junit/*.xml target/junit/repeat-*/*.xml; do
  [[ -f "$report" ]] || continue
  names=$(xmllint --xpath '//testcase[failure or error]/@name' "$report" 2>/dev/null |
    sed -E 's/ name="([^"]*)"/\1\n/g' || true)
  [[ -n "$names" ]] && failed+="$(printf '%s\n' "$names" | sed "/^$/d; s|^|- \`${report#target/junit/}\`: |")"$'\n'
done

{
  echo "### Failed tests"
  if [[ -n "$failed" ]]; then
    printf '%s' "$failed"
  else
    echo "No test failed; a step outside the tests failed."
  fi
} >>"$summary"
