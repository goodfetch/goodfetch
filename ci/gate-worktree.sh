#!/usr/bin/env bash
# A worktree of the local gate (ADR 0135), shared by `just merge`
# (ci/merge-pr.sh), whose worktree is `merge-gate`, and the backstop's full
# run (ci/gate-backstop.sh), whose own is `gate-backstop`, named by
# `gate_worktree_name` before this file is sourced. Each is at a fixed path
# beside the other worktrees in the main checkout, so Cargo's fingerprints,
# sccache's keys and the perf container's build volume, all keyed by
# absolute paths, still hold at its next run, and the gate's records in its
# `target/gate-runs/` serve the next run. Locked, so a `git worktree prune`
# from another session cannot drop it, and held by one run at a time
# through a lock folder. Sourced, never run. It keeps to bash 3.2, which
# stock macOS runs.

root=$(cd "$(git rev-parse --path-format=absolute --git-common-dir)/.." && pwd)
worktree="$root/.claude/worktrees/${gate_worktree_name:-merge-gate}"
lock="$worktree.lock"

# Takes the worktree's lock for this process, or fails while another run's
# process holds it. The caller releases it (`rm -rf "$lock"`).
gate_worktree_lock() {
  local holder
  mkdir -p "${lock%/*}"
  until mkdir "$lock" 2>/dev/null; do
    holder=$(cat "$lock/pid" 2>/dev/null || true)
    if [[ -n "$holder" ]] && kill -0 "$holder" 2>/dev/null; then
      echo "error: another gate (process $holder) is running in $worktree; run the command again when it ends" >&2
      exit 1
    elif [[ -d $lock ]]; then
      # The run that held it ended without releasing it.
      rm -rf "$lock"
    else
      echo "error: the gate's lock $lock cannot be made" >&2
      exit 1
    fi
  done
  echo "$$" >"$lock/pid"
}

# Resets the worktree to `commit` and nothing else: tracked files as
# committed, every other file removed but the build caches (`target/`, the
# Python environment and the npm packages, which `just fetch` brings to the
# head's lockfiles, without Vite's pre-bundled dependencies), and no step's
# output from an earlier gate in `target/`.
gate_worktree_checkout() {
  local commit=$1 entry
  if [[ -e "$worktree/.git" ]]; then
    git -C "$worktree" checkout --quiet --force --detach "$commit"
    git -C "$worktree" clean -ffdxq -e /target/ -e /.venv/ -e node_modules/
  else
    git worktree add --quiet --lock --detach "$worktree" "$commit"
  fi
  shopt -s nullglob
  for entry in "$worktree"/target/*; do
    case ${entry##*/} in
      # Cargo's build folders (the profiles', the MSRV's, perf's base's and
      # the stress run's base listing's), perf's base worktree, which its next
      # run moves to the next base (xtask perf build), the tests' temporary
      # folder with its fixture builds, the TLA+ tools, each checked against
      # its pinned SHA-256 when used,
      # the listings saved for the base of the next gate (ci/merge-pr.sh),
      # and the results each gate run recorded, which the next run on a
      # descendant head reuses where the change since does not reach them.
      CACHEDIR.TAG | debug | release | ci-test | msrv | perf-base | perf-base-tree | stress-base | tmp | tla-tools | stress-listings | gate-runs) ;;
      *) rm -rf "$entry" ;;
    esac
  done
  shopt -u nullglob
  # Vite's pre-bundled dependencies (Vitest's in `node_modules/.vite/`,
  # Storybook's in `node_modules/.cache/storybook/`, any `deps` folder with
  # Vite's `_metadata.json`) are not kept: Vite keys them by the lockfile and
  # the config alone, so a head that imports a package the last gate's head
  # did not would have Vite bundle it mid-run and reload, which fails every
  # browser test file at import ("Vitest failed to find the current suite").
  find "$worktree" \( -path "$worktree/target" -o -path "$worktree/.venv" -o -name .pnpm \) -prune -o \
    -type f -path '*/node_modules/*/deps/_metadata.json' -print |
    while IFS= read -r entry; do rm -rf "${entry%/_metadata.json}"; done
  if [[ -n "$(git -C "$worktree" status --porcelain)" || $(git -C "$worktree" rev-parse HEAD) != "$(git rev-parse "$commit^{commit}")" ]]; then
    echo "error: the gate's worktree $worktree is not a clean checkout of $commit" >&2
    exit 1
  fi
}
