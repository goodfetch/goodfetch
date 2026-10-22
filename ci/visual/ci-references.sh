#!/usr/bin/env bash
# Counts the Reference screenshots committed at HEAD, and those of them the
# `update-screenshots` workflow wrote: a Reference whose last commit names
# the run it came from ("update-screenshots run <id>", which `just
# screenshots-fetch` asks for). `just screenshots-update` writes the others
# on a local host, so their pixels are not CI's (ADR 0073).
# Prints "<from CI> <committed>".
set -euo pipefail
cd "$(dirname "$0")/../.."

committed=0
from_ci=0
while IFS= read -r reference; do
  committed=$((committed + 1))
  if git log -1 --format=%B -- "$reference" | grep -q -E 'update-screenshots run [0-9]+'; then
    from_ci=$((from_ci + 1))
  fi
done < <(git ls-files | grep -E '(-snapshots|__screenshots__)/[^/]+\.png$' || true)
echo "$from_ci $committed"
