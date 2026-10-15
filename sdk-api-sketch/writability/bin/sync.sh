#!/bin/bash
# Copies the runs' outputs (prompts, programs, check logs, agent transcripts) from the work folder
# into writability/runs/, leaving out links and caches. Usage: sync.sh [WORK]
WORK=${1:-$WRITABILITY_WORK}
DEST=$(cd "$(dirname "$0")/.." && pwd)/runs
mkdir -p "$DEST"
rsync -a --delete --exclude node_modules --exclude mypy_cache --exclude 'pyright.*' --exclude 'tsconfig.*.json' \
  --exclude __pycache__ --exclude check --exclude 'golden-*' "$WORK/runs/" "$DEST/"
