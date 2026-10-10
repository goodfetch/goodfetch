#!/bin/zsh
# Paired-block binding benchmark: each block runs the 4 runners in a fresh random order.
set -u
B=${0:A:h}
O=$B/paired
PY=$B/pyo3_bench/.venv/bin/python
run() { # runner -> raw output file
  case $1 in
    napi)      node "$B/napi_bench/bench.cjs" ;;
    pyo3)      "$PY" "$B/pyo3_bench/bench.py" ;;
    stdio_node) (cd "$B/ipc_bench" && "$B/target/release/ipc_bench" node_stdio node handler.mjs) ;;
    stdio_py)  (cd "$B/ipc_bench" && "$B/target/release/ipc_bench" py_stdio "$PY" -I handler.py) ;;
  esac
}
for blk in 1 2 3 4 5 6 7 8; do
  order=($(printf '%s\n' napi pyo3 stdio_node stdio_py | sort -R))
  echo "block $blk order: ${order[*]}"
  for r in $order; do
    fc=$(/bin/ps -Ao pcpu= | awk '{s+=$1} END {printf "%.0f", s}')
    l1=$(sysctl -n vm.loadavg | awk '{print $2}')
    echo "{\"meta\":\"pre\",\"block\":$blk,\"runner\":\"$r\",\"foreign_cpu\":$fc,\"load1\":$l1}" > "$O/b${blk}_$r.jsonl"
    run $r >> "$O/b${blk}_$r.jsonl" 2>"$O/b${blk}_$r.err"
  done
done
echo "DONE $(date -u +%FT%TZ)"
