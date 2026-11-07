#!/usr/bin/env bash
# peak-cuts: the engine started and left idle 3 s before any request, sampled.
# usage: idle.sh <tag> <exe> [VAR=value...]
set -uo pipefail
R=/Users/jb/agent-targets/peak-cuts/runs
tag=$1 exe=$2
shift 2
[[ ${EVICT:-0} == 1 ]] && python3 -I /Users/jb/agent-targets/peak-cuts/scripts/evict.py "$exe"
python3 -I -c 'import time; time.sleep(3)' | env "$@" "$exe" attached-engine > /dev/null 2> "$R/$tag.err" &
p=$!
python3 -I /Users/jb/agent-targets/peak-cuts/scripts/sampler.py "$p" "$R/$tag" "$(basename "$exe")"
wait "$p"; echo "$tag rc=$?"
