### S2a cancel → run resolved (engine stopped), median of 5 cancels per run

| Runner | Median cancel latency (ms) | Min–max of per-run medians | Runs |
| --- | ---: | --- | ---: |
| napi | 0.38 | 0.19–6.39 | 5 |
| rpc-node | 0.36 | 0.22–1.37 | 5 |
| pyo3 | 0.40 | 0.32–2.33 | 5 |
| rpc-py | 0.52 | 0.34–2.64 | 5 |

| Comparison | Blocks | Ratio in-process/RPC (median) | 95% CI |
| --- | ---: | ---: | --- |
| napi/rpc-node cancel latency | 5 | **1.05** | 0.60–4.67 |
| pyo3/rpc-py cancel latency | 5 | **0.88** | 0.50–1.04 |
