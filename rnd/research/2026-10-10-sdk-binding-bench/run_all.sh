#!/bin/zsh
# Runs every microbenchmark sequentially; prints machine load before and after each.
set -u
B=${0:A:h}
echo "## start $(date -u +%FT%TZ) $(uptime)"
echo "## napi in-process"
node "$B/napi_bench/bench.cjs"
echo "## load $(uptime)"
echo "## pyo3 in-process"
"$B/pyo3_bench/.venv/bin/python" "$B/pyo3_bench/bench.py"
echo "## load $(uptime)"
echo "## stdio node"
cd "$B/ipc_bench" && "$B/target/release/ipc_bench" node_stdio node handler.mjs
echo "## load $(uptime)"
echo "## stdio python"
cd "$B/ipc_bench" && "$B/target/release/ipc_bench" py_stdio "$B/pyo3_bench/.venv/bin/python" -I handler.py
echo "## end $(date -u +%FT%TZ) $(uptime)"
