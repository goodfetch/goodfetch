### Glue code (non-blank, non-comment lines; engine-core is shared: 499 lines)

| Binding | Native side | SDK side | Total |
| --- | ---: | ---: | ---: |
| napi-rs (Node) | 79 (napi-addon/src/lib.rs) | 35 (sdk-ts/native.ts + types.ts) | 114 |
| PyO3 (Python) | 179 (pyo3-ext/src/lib.rs) | 47 (native.py + __init__.py) | 226 |
| RPC (engined, both SDKs) | 239 (engined/src/main.rs, incl. ~50 for inspect/check) | TS 126 (rpc.ts + types.ts), Py 115 (rpc.py + __init__.py) | 480 |

### Clean release build (no LTO), load at start: 622.21 404.42 787.00

| Artifact | Clean build (s) | Size | Stripped |
| --- | ---: | ---: | ---: |
| engined binary | 268 | 3.4M | 3.0M |
| napi-rs addon (.node) | 59 | 3.1M | 2.8M |
| PyO3 abi3 wheel | 72 | 1.5M (wheel), 3.1M (.so) | n/a |
