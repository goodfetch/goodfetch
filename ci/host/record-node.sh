#!/usr/bin/env bash
# Records where the Node line just installed lives, for the test steps that
# run each line (.github/actions/host-tools): appends `<line>=<bin folder>` to
# GOODFETCH_NODE_LINES in the job's environment.
set -euo pipefail

line=${1:?usage: ci/host/record-node.sh <line>}
folder=$(dirname "$(command -v node)")
version=$(node --version)
if [[ "$version" != "v$line."* ]]; then
  echo "error: the node on PATH is $version, not Node $line" >&2
  exit 1
fi
echo "GOODFETCH_NODE_LINES=${GOODFETCH_NODE_LINES:+$GOODFETCH_NODE_LINES }$line=$folder" >>"${GITHUB_ENV:?the runner sets GITHUB_ENV}"
