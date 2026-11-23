#!/usr/bin/env bash
# `ci/spec-sub-issues.sh <spec> <pull request>`: how many of spec #<spec>'s
# sub-issues stay open once the pull request's closing issues are closed, the
# rule that says whether a `spec/<n>` pull request finishes its spec and so
# takes the full gate (ADR 0135's note of 2026-11-13). `just merge`
# (ci/merge-pr.sh) and the hosted tier's planning job (ci/hosted/plan.sh)
# both ask it, so they pick the same form.
#
# One query of both, read whole: it prints the count, or "more than a page"
# when either list holds more than a page. A caller takes any answer that
# is no whole number as unread, never as none or some left. Needs `gh` with a
# token that reads the repository's issues and pull requests.
set -euo pipefail

usage="usage: ci/spec-sub-issues.sh <spec> <pull request>"
spec=${1:?$usage}
pr=${2:?$usage}

read -r -d '' query <<'GRAPHQL' || true
query($owner: String!, $name: String!, $spec: Int!, $pr: Int!) {
  repository(owner: $owner, name: $name) {
    issue(number: $spec) { subIssues(first: 100) { pageInfo { hasNextPage } nodes { number state } } }
    pullRequest(number: $pr) { closingIssuesReferences(first: 100) { pageInfo { hasNextPage } nodes { number } } }
  }
}
GRAPHQL
read -r -d '' filter <<'JQ' || true
.data.repository
| [.pullRequest.closingIssuesReferences.nodes[].number] as $closed
| if .issue.subIssues.pageInfo.hasNextPage or .pullRequest.closingIssuesReferences.pageInfo.hasNextPage then "more than a page"
  else [.issue.subIssues.nodes[] | select(.state == "OPEN") | .number | select(. as $n | $closed | any(. == $n) | not)] | length
  end
JQ
gh api graphql -F owner='{owner}' -F name='{repo}' -F spec="$spec" -F pr="$pr" -f query="$query" --jq "$filter"
