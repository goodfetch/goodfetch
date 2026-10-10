### S2c one handler busy-loops 3 s (40 pages, concurrency 4, timeout 1 s)

| Runner | Done | Timed out | Run elapsed (ms) | Pages committed during the block | …of which timeouts | Max event lag (ms) | Runs |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| napi | 28 | 12 | 3,160 | 8 | 8 | 2,002 | 3 |
| rpc-node | 28 | 12 | 3,195 | 8 | 8 | 2,007 | 3 |
| pyo3 | 32 | 8 | 3,170 | 8 | 8 | 12 | 3 |
| rpc-py | 28 | 12 | 3,101 | 8 | 8 | 1,998 | 3 |
