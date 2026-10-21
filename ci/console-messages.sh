#!/usr/bin/env bash
# The Console's message catalog drift check (ADR 0073), in `just lint`:
# `lingui extract` over each Console package must find exactly the messages
# its committed catalog holds. The working tree is left as it was; a catalog
# that differs fails, naming the command that rewrites it.
set -euo pipefail

status=0
for package in console/*/; do
  package=${package%/}
  [[ -f "$package/lingui.config.ts" ]] || continue
  messages="$package/src/messages"
  kept=$(mktemp -d)
  cp -R "$messages/." "$kept/"
  pnpm --dir "$package" exec lingui extract --clean >/dev/null
  if ! diff -r "$kept" "$messages" >/dev/null; then
    echo "error: ${messages} is not what \`lingui extract\` finds; run \`pnpm --dir ${package} run messages\` and commit the catalog" >&2
    diff -r "$kept" "$messages" >&2 || true
    status=1
  fi
  rm -rf "${messages:?}"/*
  cp -R "$kept/." "$messages/"
  rm -rf "$kept"
done
exit "$status"
