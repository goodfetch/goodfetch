### S4 medians across blocks: time from engine handing the page over to handler entry (delivered) and to the full body in hand (body)

| Size | Runner | Variant | Delivered p50 (µs) | Body in hand p50 (µs) | Handler round trip p50 (µs) | Blocks |
| ---: | --- | --- | ---: | ---: | ---: | ---: |
| 100,000 | napi | ref | 12 | 65 | 73 | 5 |
| 100,000 | napi | zero-copy | 16 | 17 | 28 | 5 |
| 100,000 | rpc-node | inline | 282 | 282 | 305 | 5 |
| 100,000 | rpc-node | ref | 40 | 100 | 119 | 5 |
| 100,000 | pyo3 | bytes | 49 | 49 | 80 | 5 |
| 100,000 | pyo3 | memoryview | 46 | 47 | 76 | 5 |
| 100,000 | pyo3 | ref | 47 | 90 | 117 | 5 |
| 100,000 | rpc-py | inline | 373 | 373 | 399 | 5 |
| 100,000 | rpc-py | ref | 79 | 127 | 148 | 5 |
| 1,000,000 | napi | ref | 28 | 229 | 256 | 5 |
| 1,000,000 | napi | zero-copy | 39 | 40 | 64 | 5 |
| 1,000,000 | rpc-node | inline | 2,469 | 2,469 | 2,523 | 5 |
| 1,000,000 | rpc-node | ref | 97 | 290 | 331 | 5 |
| 1,000,000 | pyo3 | bytes | 141 | 142 | 188 | 5 |
| 1,000,000 | pyo3 | memoryview | 128 | 129 | 190 | 5 |
| 1,000,000 | pyo3 | ref | 110 | 319 | 381 | 5 |
| 1,000,000 | rpc-py | inline | 3,197 | 3,199 | 3,253 | 5 |
| 1,000,000 | rpc-py | ref | 158 | 371 | 424 | 5 |
| 10,000,000 | napi | ref | 52 | 1,526 | 1,578 | 5 |
| 10,000,000 | napi | zero-copy | 68 | 68 | 117 | 5 |
| 10,000,000 | rpc-node | inline | 22,070 | 22,071 | 22,146 | 5 |
| 10,000,000 | rpc-node | ref | 141 | 1,470 | 1,557 | 5 |
| 10,000,000 | pyo3 | bytes | 474 | 475 | 534 | 5 |
| 10,000,000 | pyo3 | memoryview | 176 | 179 | 242 | 5 |
| 10,000,000 | pyo3 | ref | 193 | 1,628 | 1,758 | 5 |
| 10,000,000 | rpc-py | inline | 30,099 | 30,101 | 30,170 | 5 |
| 10,000,000 | rpc-py | ref | 184 | 1,624 | 1,700 | 5 |

### S4 paired ratios on 'body in hand' vs RPC by reference (<1: faster than RPC by reference)

| Size | Comparison | Blocks | Ratio (median) | 95% CI |
| ---: | --- | ---: | ---: | --- |
| 100,000 | rpc-node inline ÷ rpc-node ref | 5 | **2.75** | 1.80–3.31 |
| 100,000 | napi zero-copy ÷ rpc-node ref | 5 | **0.17** | 0.08–0.22 |
| 100,000 | napi ref ÷ rpc-node ref | 5 | **0.66** | 0.29–0.77 |
| 100,000 | rpc-py inline ÷ rpc-py ref | 5 | **2.94** | 2.65–3.10 |
| 100,000 | pyo3 bytes ÷ rpc-py ref | 5 | **0.38** | 0.26–0.51 |
| 100,000 | pyo3 memoryview ÷ rpc-py ref | 5 | **0.34** | 0.23–0.45 |
| 100,000 | pyo3 ref ÷ rpc-py ref | 5 | **0.63** | 0.43–0.87 |
| 1,000,000 | rpc-node inline ÷ rpc-node ref | 5 | **9.10** | 7.21–10.51 |
| 1,000,000 | napi zero-copy ÷ rpc-node ref | 5 | **0.12** | 0.10–0.16 |
| 1,000,000 | napi ref ÷ rpc-node ref | 5 | **0.86** | 0.55–1.10 |
| 1,000,000 | rpc-py inline ÷ rpc-py ref | 5 | **9.81** | 7.82–11.67 |
| 1,000,000 | pyo3 bytes ÷ rpc-py ref | 5 | **0.35** | 0.24–0.58 |
| 1,000,000 | pyo3 memoryview ÷ rpc-py ref | 5 | **0.36** | 0.20–0.52 |
| 1,000,000 | pyo3 ref ÷ rpc-py ref | 5 | **0.87** | 0.48–1.37 |
| 10,000,000 | rpc-node inline ÷ rpc-node ref | 5 | **14.78** | 13.85–18.85 |
| 10,000,000 | napi zero-copy ÷ rpc-node ref | 5 | **0.04** | 0.04–0.05 |
| 10,000,000 | napi ref ÷ rpc-node ref | 5 | **1.03** | 0.85–1.07 |
| 10,000,000 | rpc-py inline ÷ rpc-py ref | 5 | **17.24** | 15.84–19.87 |
| 10,000,000 | pyo3 bytes ÷ rpc-py ref | 5 | **0.29** | 0.22–0.34 |
| 10,000,000 | pyo3 memoryview ÷ rpc-py ref | 5 | **0.11** | 0.09–0.11 |
| 10,000,000 | pyo3 ref ÷ rpc-py ref | 5 | **0.97** | 0.92–1.04 |
