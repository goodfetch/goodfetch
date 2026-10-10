### S1 paired ratios (in-process ÷ RPC)

| Comparison | Blocks | Ratio (median) | 95% CI |
| --- | ---: | ---: | --- |
| fast-default: throughput napi/rpc-node (>1: in-process faster) | 5 | **1.46** | 0.36–4.81 |
| fast-default: p50 latency napi/rpc-node (<1: in-process faster) | 5 | **0.69** | 0.14–3.37 |
| fast-default: throughput pyo3/rpc-py (>1: in-process faster) | 5 | **1.18** | 0.12–2.39 |
| fast-default: p50 latency pyo3/rpc-py (<1: in-process faster) | 5 | **0.00** | 0.00–0.00 |
| fast-batched: throughput napi/rpc-node (>1: in-process faster) | 5 | **0.94** | 0.12–1.15 |
| fast-batched: p50 latency napi/rpc-node (<1: in-process faster) | 5 | **1.91** | 0.61–6.47 |
| fast-batched: throughput pyo3/rpc-py (>1: in-process faster) | 5 | **0.65** | 0.10–6.35 |
| fast-batched: p50 latency pyo3/rpc-py (<1: in-process faster) | 5 | **0.04** | 0.01–0.79 |
| napi: throughput batched/default | 5 | **1.78** | 0.36–5.51 |
| pyo3: throughput batched/default | 5 | **6.81** | 0.04–49.45 |
| rpc-node: throughput batched/default | 5 | **2.50** | 0.87–13.76 |
| rpc-py: throughput batched/default | 5 | **2.51** | 0.68–35.96 |

### S1 medians per runner and variant (100k events, 64 B payload; slow = 20 µs busy-wait per event)

| Runner | Variant | Throughput (ev/s) | p50 lat (ms) | p99 lat (ms) | Engine done emitting (ms) | Consumer done (ms) | Dropped | SDK peak RSS (MB) | engined peak RSS (MB) | Runs |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| napi | fast-default | 169,769 | 290.96 | 422.03 | 84.03 | 589.04 | 0 | 110 | (same process) | 5 |
| napi | fast-batched | 234,665 | 4.10 | 37.38 | 423.86 | 426.14 | 0 | 91 | (same process) | 5 |
| napi | slow-default | 15,106 | 2,675 | 6,520 | 46.21 | 6,620 | 0 | 114 | (same process) | 5 |
| napi | slow-bounded | 2,634 | 139.55 | 2,262 | 37,782 | 37,969 | 0 | 90 | (same process) | 5 |
| napi | slow-batched | 15,945 | 57.43 | 170.25 | 6,215 | 6,272 | 0 | 96 | (same process) | 5 |
| napi | slow-drop | 8,895 | 81.75 | 113.14 | 29.40 | 115.12 | 98,976 | 91 | (same process) | 5 |
| rpc-node | fast-default | 127,443 | 409.10 | 465.64 | 333.61 | 784.67 | 0 | 92 | 32 | 5 |
| rpc-node | fast-batched | 370,710 | 2.11 | 11.45 | 264.68 | 269.75 | 0 | 90 | 5 | 5 |
| rpc-node | slow-default | 10,914 | 4,561 | 8,520 | 486.21 | 9,162 | 0 | 94 | 32 | 5 |
| rpc-node | slow-bounded | 2,825 | 269.23 | 1,148 | 35,212 | 35,395 | 0 | 90 | 4 | 5 |
| rpc-node | slow-batched | 14,482 | 83.54 | 439.24 | 6,874 | 6,905 | 0 | 99 | 5 | 5 |
| rpc-node | slow-drop | 10,274 | 66.88 | 99.09 | 25.23 | 99.67 | 98,976 | 92 | 4 | 5 |
| pyo3 | fast-default | 135,363 | 0.00 | 0.01 | 738.75 | 738.75 | 0 | 32 | (same process) | 5 |
| pyo3 | fast-batched | 406,027 | 0.10 | 3.33 | 246.27 | 246.29 | 0 | 34 | (same process) | 5 |
| pyo3 | fast-loop | 22,054 | 1.99 | 48.79 | 4,532 | 4,534 | 0 | 35 | (same process) | 5 |
| pyo3 | slow-default | 14,980 | 0.00 | 0.05 | 6,676 | 6,676 | 0 | 30 | (same process) | 5 |
| pyo3 | slow-loop | 8,779 | 1,827 | 2,841 | 6,141 | 11,391 | 0 | 59 | (same process) | 5 |
| pyo3 | slow-bounded | 8,671 | 60.57 | 373.25 | 11,462 | 11,533 | 0 | 30 | (same process) | 5 |
| pyo3 | slow-batched | 15,276 | 2.31 | 91.27 | 6,544 | 6,546 | 0 | 34 | (same process) | 5 |
| pyo3 | slow-drop | 10,072 | 75.76 | 100.93 | 30.92 | 101.67 | 98,976 | 28 | (same process) | 5 |
| rpc-py | fast-default | 81,268 | 629.41 | 948.28 | 279.87 | 1,231 | 0 | 32 | 32 | 5 |
| rpc-py | fast-batched | 165,846 | 4.42 | 35.96 | 600.43 | 602.97 | 0 | 32 | 5 | 5 |
| rpc-py | slow-default | 17,653 | 3,837 | 5,362 | 359.42 | 5,665 | 0 | 28 | 32 | 5 |
| rpc-py | slow-bounded | 6,593 | 118.23 | 756.88 | 15,099 | 15,167 | 0 | 28 | 4 | 5 |
| rpc-py | slow-batched | 23,066 | 52.13 | 309.00 | 4,298 | 4,335 | 0 | 29 | 5 | 5 |
| rpc-py | slow-drop | 13,376 | 52.41 | 75.83 | 24.13 | 76.56 | 98,976 | 26 | 4 | 5 |
