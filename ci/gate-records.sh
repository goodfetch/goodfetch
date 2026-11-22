#!/usr/bin/env bash
# Prints the folder of the gate's records (ADR 0135's amendment of
# 2026-11-22): `goodfetch-gate-runs/` in the repository's common directory,
# which every worktree of the repository shares, so a unit any local layer
# recorded green on a tree, in whichever worktree, runs on that tree no more.
# `GOODFETCH_GATE_RECORDS` names another folder: the backstop's full run keeps
# its record in its own worktree, apart from the runs it judges.
set -euo pipefail

if [[ -n "${GOODFETCH_GATE_RECORDS:-}" ]]; then
  printf '%s\n' "$GOODFETCH_GATE_RECORDS"
  exit 0
fi
common=$(git rev-parse --path-format=absolute --git-common-dir)
printf '%s\n' "$common/goodfetch-gate-runs"
