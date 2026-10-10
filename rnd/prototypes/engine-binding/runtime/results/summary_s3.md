### S3 crash isolation (handler kills its process at page 100)

| Runner | Topology | Crash | SDK exit | integrity_check | Journal rows after | Acked pages | Acked but not durable | Notes |
| --- | --- | --- | ---: | --- | --- | ---: | ---: | --- |
| napi | in-process | exit | 1 | ok | {'done': 99, 'leased': 4} | 99 | 0 | leases stay 'leased' (nobody left to release them) |
| napi | in-process | abort | -6 | ok | {'done': 100, 'leased': 4} | 100 | 0 | leases stay 'leased' (nobody left to release them) |
| rpc-node | sidecar | exit | 1 | ok | {'done': 100, 'queued': 4} | 100 | 0 | [engined] session sidecar-52335 closed; requeued 4 leased items in 0.2 ms; sidecar exited 0 ms after the SDK died |
| rpc-node | sidecar | abort | -6 | ok | {'done': 98, 'queued': 4} | 98 | 0 | [engined] session sidecar-52338 closed; requeued 4 leased items in 0.4 ms; sidecar exited 0 ms after the SDK died |
| pyo3 | in-process | exit | 1 | ok | {'done': 100, 'leased': 4} | 100 | 0 | leases stay 'leased' (nobody left to release them) |
| pyo3 | in-process | abort | -6 | ok | {'done': 100, 'leased': 4} | 100 | 0 | leases stay 'leased' (nobody left to release them) |
| rpc-py | sidecar | exit | 1 | ok | {'done': 100, 'queued': 4} | 100 | 0 | [engined] session sidecar-52349 closed; requeued 4 leased items in 0.2 ms; sidecar exited 0 ms after the SDK died |
| rpc-py | sidecar | abort | -6 | ok | {'done': 100, 'queued': 4} | 100 | 0 | [engined] session sidecar-52359 closed; requeued 4 leased items in 0.2 ms; sidecar exited 0 ms after the SDK died |
| daemon: rpc-node crashes (exit), rpc-py keeps running | daemon | exit | 1 | ok | A: {'done': 97, 'queued': 4} | 97 | 0 | B: {'done': 300}; probe after crash: ok; [engined] session conn-2 closed; requeued 4 leased items in 0.3 ms; [engined] session conn-1 closed; requeued 0 leased items in 0.1 ms; [engined] session conn-3 closed; requeued 0 leased items in 0.0 ms |
| daemon: rpc-py crashes (abort), rpc-node keeps running | daemon | abort | -6 | ok | A: {'done': 100, 'queued': 4} | 100 | 0 | B: {'done': 300}; probe after crash: ok; [engined] session conn-1 closed; requeued 4 leased items in 0.3 ms; [engined] session conn-2 closed; requeued 0 leased items in 0.5 ms; [engined] session conn-3 closed; requeued 0 leased items in 0.0 ms |
