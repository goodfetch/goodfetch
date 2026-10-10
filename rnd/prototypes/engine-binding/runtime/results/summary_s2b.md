### S2b handlers that never resolve (20 pages, 4 hang, timeout 500 ms, concurrency 4)

| Runner | Done | Timed out | Run elapsed (ms, median) | Hung handlers cancelled in user code | Pending tasks after run | Process exit after main (ms) | Runs |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| napi | 16 | 4 | 611 | 0 of 4 | n/a | 19 | 3 |
| rpc-node | 16 | 4 | 652 | 4 of 4 | n/a | 29 | 3 |
| pyo3 | 16 | 4 | 621 | 4 of 4 | 0 | 25 | 3 |
| rpc-py | 16 | 4 | 555 | 4 of 4 | 0 | 32 | 3 |
