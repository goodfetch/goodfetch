# SDK binding microbenchmark

This harness backs §3 and §3.1 of [the SDK binding note](../2026-10-10-sdk-binding.md). It compares one engine-to-handler call made in-process (napi-rs, PyO3) with the same call over stdio JSON-RPC to a Node or Python handler host. Toolchain used: macOS arm64, Rust stable, Node 24, Python 3.13, maturin 1.15.

## Build

From this folder:

```sh
export CARGO_TARGET_DIR="$PWD/target"
cargo build --release --manifest-path ipc_bench/Cargo.toml
cargo build --release --manifest-path napi_bench/Cargo.toml
cp target/release/libnapi_bench.dylib napi_bench/napi_bench.node
python3 -m venv pyo3_bench/.venv
pyo3_bench/.venv/bin/pip install maturin
(cd pyo3_bench && VIRTUAL_ENV="$PWD/.venv" .venv/bin/maturin develop --release)
```

## Run

- `./run_all.sh`: one unpaired pass of the four runners (§3).
- `./run_paired.sh`: eight paired blocks with each block's runners in random order. It writes `paired/b<block>_<runner>.jsonl` (§3.1).
- `python3 analyze_paired.py`: per-block ratios, the median across blocks and a bootstrap 95% CI.

`paired/` holds the raw results of the 2026-10-10 run, on a heavily loaded host: durations are indicative, the ratios are paired.
