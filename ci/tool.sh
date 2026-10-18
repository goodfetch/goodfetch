#!/usr/bin/env bash
# Runs one part of a `just` recipe that needs a tool: `ci/tool.sh <tool> <command>…`.
#
# On a local host without the tool, the part is skipped and recorded in
# target/skipped.txt with its reason, and `just quick`, `just pre-merge` and
# `just release` list it at the end (ADR 0061: a local run lists what it could
# not run, and its exit code covers what ran). In CI (`CI` set) a missing tool
# fails the job: CI is the verdict.
set -euo pipefail

tool=$1
shift

if command -v "$tool" >/dev/null 2>&1; then
  exec "$@"
fi

if [[ -n "${CI:-}" ]]; then
  echo "error: $tool is not installed, and CI runs every part" >&2
  exit 1
fi

mkdir -p target
echo "$*: skipped, $tool is not installed on this host" >>target/skipped.txt
echo "skipped: $* ($tool is not installed on this host)" >&2
