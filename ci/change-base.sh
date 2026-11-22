#!/usr/bin/env bash
# Sourced by ci/quick.sh and .githooks/pre-push: the branch's own base, the
# one its change goes into. `branch_base` prints the checked out branch's
# upstream when that branch is not `main` and its upstream is another branch
# (the integration branch a ticket goes into), else `origin/main`. The
# branch's own remote copy, its upstream after `git push -u`, is never the
# base: against it a pushed branch changed nothing. `own_remote_copy` prints
# the branch's name and succeeds when its upstream is that copy.
# It keeps to bash 3.2, which stock macOS runs.

own_remote_copy() {
  local branch
  branch=$(git symbolic-ref --quiet --short HEAD 2>/dev/null || true)
  [[ -n $branch && $branch != main ]] || return 1
  [[ $(git config --get "branch.$branch.merge" 2>/dev/null || true) == "refs/heads/$branch" ]] || return 1
  echo "$branch"
}

branch_base() {
  local branch upstream=""
  branch=$(git symbolic-ref --quiet --short HEAD 2>/dev/null || true)
  if [[ -n $branch && $branch != main ]] && ! own_remote_copy > /dev/null; then
    upstream=$(git rev-parse --abbrev-ref --symbolic-full-name '@{upstream}' 2>/dev/null || true)
  fi
  echo "${upstream:-origin/main}"
}
