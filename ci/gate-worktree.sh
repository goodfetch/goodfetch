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
# through a lock folder, which the other worktree's runs read before they
# start. Sourced, never run. It keeps to bash 3.2, which stock macOS runs.

root=$(cd "$(git rev-parse --path-format=absolute --git-common-dir)/.." && pwd)
worktree="$root/.claude/worktrees/${gate_worktree_name:-merge-gate}"
lock="$worktree.lock"

# Prints when process `$1` started, in a form two shells print alike for one
# process and no later process of that number shares: on Linux its start in
# clock ticks since the boot (/proc/<pid>/stat, the 22nd field, past the
# command's name in parentheses), elsewhere `ps`'s start time in UTC and the C
# locale. Nothing when the process no longer runs.
process_started() {
  local stat
  if [[ -d /proc/self ]]; then
    stat=$(cat "/proc/$1/stat" 2>/dev/null) || return 0
    awk '{ print $20 }' <<<"${stat##*) }"
  else
    TZ=UTC LC_ALL=C ps -o lstart= -p "$1" 2>/dev/null | awk '{ $1 = $1; print }'
  fi
}

# Prints the process that holds the lock folder `$1`, when it still runs; a
# lock whose holder ended holds nothing, and prints nothing. Neither does a
# lock whose process number another process has since taken, as one may after
# a reboot: the lock names when its holder started (`started`), which that
# process does not match, so no run waits for ever for a run long gone.
#
# Only two readings that differ make a lock stale. A start that cannot be
# told, the lock's or the process's, leaves the lock judged by its process
# alone, as a lock with no `started` is, which a script from before it wrote
# one takes. The two mistakes do not cost the same: a live lock read as stale
# lets two heavy suites run together, with nothing to show it but tests that
# fail on timing; a dead lock read as live makes a caller wait, which its log
# says and a stop ends, and the process is looked at again each second.
gate_worktree_holder() {
  local holder recorded current
  holder=$(cat "$1/pid" 2>/dev/null || true)
  if [[ -z $holder ]] || ! kill -0 "$holder" 2>/dev/null; then return 0; fi
  recorded=$(cat "$1/started" 2>/dev/null || true)
  current=$(process_started "$holder")
  if [[ -n $recorded && -n $current && $recorded != "$current" ]]; then return 0; fi
  echo "$holder"
}

# `just merge`'s gate and the runs in the backstop's worktree read each
# other's lock, whichever worktree this file was sourced for, so no two of
# them run their suites on the host at once.

# Prints the process of `just merge`'s gate, when one runs.
merge_gate_holder() {
  gate_worktree_holder "$root/.claude/worktrees/merge-gate.lock"
}

# Prints the process of the nightly fuzzing (ci/fuzz-nightly.sh), when one
# runs: a gate stops it before it starts (ci/merge-pr.sh).
fuzz_nightly_holder() {
  gate_worktree_holder "$root/.claude/worktrees/fuzz-nightly.lock"
}

# Prints the run that holds the backstop's worktree and that `just merge`'s
# gate, process `$1`, ends or lets end before it starts, as `the post-merge
# run, process <n>`, `the test-health run, process <n>` or `the backstop,
# process <n>`; nothing when the gate may start. ci/test-health/scheduled.sh
# marks the lock it takes with a `test-health` file, and a post-merge run's
# with a `post-merge` file too, which the gate stops. The backstop's own run that found a gate running names
# it in `waits-for` and lets it end first (ci/gate-backstop.sh): that gate
# does not wait for the backstop in turn, and any later gate does, since the
# backstop was there before it. A run that died holds nothing.
run_in_the_backstops_worktree() {
  local gate=$1 holder backstop_lock="$root/.claude/worktrees/gate-backstop.lock"
  holder=$(gate_worktree_holder "$backstop_lock")
  if [[ -z $holder ]]; then
    return 0
  elif [[ -f $backstop_lock/post-merge ]]; then
    echo "the post-merge run, process $holder"
  elif [[ -f $backstop_lock/test-health ]]; then
    echo "the test-health run, process $holder"
  elif [[ $(cat "$backstop_lock/waits-for" 2>/dev/null || true) != "$gate" ]]; then
    echo "the backstop, process $holder"
  fi
}

# Takes the worktree's lock for this process, or fails while another run's
# process holds it. The caller releases it (`rm -rf "$lock"`).
gate_worktree_lock() {
  local holder
  mkdir -p "${lock%/*}"
  until mkdir "$lock" 2>/dev/null; do
    holder=$(gate_worktree_holder "$lock")
    if [[ -n "$holder" ]]; then
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
  # When it started first: a lock is read from the moment it names a process.
  process_started "$$" >"$lock/started"
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
  # Vite's pre-bundled dependencies (Vitest's in `node_modules/.vite/`, any
  # `deps` folder with Vite's `_metadata.json`) are not kept: Vite keys them
  # by the lockfile and the config alone, so a head that imports a package the last gate's head
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
