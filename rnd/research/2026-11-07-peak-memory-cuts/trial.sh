#!/usr/bin/env bash
# peak-cuts driver (copied from /g/gap-http/scripts/trial.sh; binary paths and CONC changed): one `xtask compare trial` against a fresh Proving ground.
# usage: trial.sh <scenario> <cell> <executable> <pages> <mode> [output]
set -euo pipefail
wt=/Users/jb/Dev/goodfetch/.claude/worktrees/confident-edison-3e91c2
scenario=$1 cell=$2 exe=$3 pages=$4 mode=$5 out=${6:-}
pg=/Users/jb/agent-targets/peak-cuts/bin/proving-ground
xtask=/Users/jb/agent-targets/peak-cuts/bin/xtask
coproc PG { exec "$pg" serve --seed "${SEED:-1574}" --copy local --address 127.0.0.1:0 --network-profile none 2>/dev/null; }
pgpid=$PG_PID
read -r line <&"${PG[0]}"
addr=$(python3 -I -c 'import json,sys; print(json.loads(sys.argv[1])["address"])' "$line")
extra=()
[[ -n $out ]] && extra=(--output "$out")
"$xtask" compare trial --scenario "$scenario" --cell "$cell" --executable "$exe" \
  --address "$addr" --pages "$pages" --concurrency "${CONC:-16}" --mode "$mode" "${extra[@]}" || rc=$?
[[ -n ${OBS:-} ]] && curl -s "http://$addr/observations" > "$OBS"
kill "$pgpid" 2>/dev/null || true
wait "$pgpid" 2>/dev/null || true
exit ${rc:-0}
