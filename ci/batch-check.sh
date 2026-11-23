#!/usr/bin/env bash
# `just batch-check [base]`: run in a batch branch's worktree before `just
# merge` (AGENTS.md, Merging). A run's tickets push to `spec/<n>` with no
# `just quick` of their own (TESTING.md, Checks by role; the owner's decision
# of 2026-11-23, ADR 0135's note of that day), so the batch runs it, once,
# on its combined tree, against `base` (`origin/main` by default): every step
# a ticket's `just quick` ran, over the whole change of every pull request
# the batch holds, so the tree that merges passes them all and two branches
# green alone are read together (batch h, 2026-11-18: two pull requests
# green alone broke a corpus test of a crate both reached).
#
# It first installs the locked JavaScript and Python dependencies (`pnpm
# install --frozen-lockfile`, `uv sync --frozen --all-extras`), since a
# batch's worktree is new and has neither `node_modules` nor `.venv`
# (batches k and l, 2026-11-19: `just notices` red on a package not
# installed), and `just quick`'s steps read them; it is skipped, and named
# so, when one failed.
#
# Every step runs even after one failed (ci/keep-going.sh), `just quick`'s
# own steps among them; the run then lists each failed step with its log
# under target/steps/batch-check/ and fails. Every step runs through the
# `just` on PATH, where the recipe puts its own. It keeps to bash 3.2, which
# stock macOS runs.
set -euo pipefail

base=${1:-origin/main}

# A hook may start this with git's GIT_DIR, and a build script's `git init`
# under a step (btls-sys's) would then reinitialize the repository as bare.
for variable in $(git rev-parse --local-env-vars); do unset "$variable"; done

# shellcheck source=ci/keep-going.sh
source "$(dirname "${BASH_SOURCE[0]}")/keep-going.sh"

echo "The batch: HEAD against its merge base with $base."
steps_begin batch-check
step pnpm-install 'pnpm install --frozen-lockfile'
step uv-sync 'uv sync --frozen --all-extras'
step --after pnpm-install --after uv-sync quick "GOODFETCH_QUICK_BASE=$(printf '%q' "$base") just quick"
steps_end
