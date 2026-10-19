#!/usr/bin/env bash
# Commits what the ingest wrote into the results store's checkout and pushes
# it (ADR 0064): `ci/results/push.sh <store> <message>`, with the store's
# token in TEST_RESULTS_TOKEN. Each run attempt has a file of its own, so two
# ingests never write the same file: a push that lost the race to another
# rebases onto it and pushes again. Nothing is ever removed from the store.
set -euo pipefail

store=${1:?usage: ci/results/push.sh <store> <message>}
message=${2:?usage: ci/results/push.sh <store> <message>}
token=${TEST_RESULTS_TOKEN:?the workflow passes the token of the store}

cd "$store"
git add runs
if git diff --cached --quiet; then
  echo "The store already holds every attempt."
  exit 0
fi

# The checkout keeps no credential; the token goes in the push's and pull's
# header only, through git's configuration variables rather than its command
# line, which other processes can read.
authorization="AUTHORIZATION: basic $(printf 'x-access-token:%s' "$token" | base64 | tr -d '\n')"
remote() {
  GIT_CONFIG_COUNT=1 GIT_CONFIG_KEY_0="http.https://github.com/.extraheader" \
    GIT_CONFIG_VALUE_0="$authorization" git "$@"
}

# The store's commits, and their rebases, are the workflow's own.
export GIT_AUTHOR_NAME="github-actions[bot]" GIT_COMMITTER_NAME="github-actions[bot]"
export GIT_AUTHOR_EMAIL="41898282+github-actions[bot]@users.noreply.github.com"
export GIT_COMMITTER_EMAIL="$GIT_AUTHOR_EMAIL"
git commit --quiet --message "$message"
for _ in 1 2 3 4 5; do
  if remote push --quiet origin HEAD:main; then
    echo "Pushed: $message"
    exit 0
  fi
  remote pull --quiet --rebase origin main
done
echo "error: the store kept moving; the next ingest pushes this attempt again" >&2
exit 1
