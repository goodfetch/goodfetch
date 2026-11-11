#!/usr/bin/env bash
# peak-cuts: re-links a saved musl link (link-save.sh) with extra lld flags.
# usage: relink.sh <save> <output> [lld flags...]
set -euo pipefail
source /Users/jb/agent-targets/peak-cuts/scripts/env.sh
save=$(realpath "$1") out=$(realpath -m "$2")
shift 2
mapfile -t args < <(sed "s|^@OUT@\$|$out|" "$save/args")
cd "$(cat "$save/cwd")"
"$ZIG" ld.lld "${args[@]}" "$@"
