# Lab worker protocol, version 1

A process is launched directly, without a shell, in its own POSIX group. stdin and stdout carry one JSON object per line. stderr contains the diagnostics, with a bounded log tail kept by the coordinator.

```json
{"version":1,"id":1,"method":"hello","params":{}}
{"version":1,"id":1,"result":{"backend":"example","protocol":1,"capabilities":["http-catalog"],"versions":{}}}
```

`id` is an integer and correlates the responses, which may arrive out of order. A response contains `result` or `error`. Methods: `hello`, `prepare`, `run`, `close`. `prepare` precedes all runs; `close` follows their completion. The client accepts at most 128 active calls. Requests are bounded to 128,000 characters on the Node side / bytes on the Rust side, and the Node stdout buffer to 8 million characters. An adapter must reserve stdout for the protocol.

For the browser, `prepare` receives `executablePath`, `headless`, `concurrency`. `run` receives scenario, fixture URL, seed, pages/actions, variant and timeout. `hello.capabilities` must declare only the scenarios actually implemented. `maxConcurrency`, when present, bounds the plan. The run return value is:

```json
{"status":"ok","records":[{"id":"P-001","price":1201}],"metrics":{"taskMs":12.3}}
```

Or `{"status":"unsupported","reason":"..."}`. All metric numbers must be finite and positive or zero. An `ok` is not a business verdict: only the coordinator delivers `verified` after the oracle. The expected items and the private validation route are not provided to the worker.

For storage, `prepare` receives the path of a fresh sink, format, busyTimeoutMs. `run` receives batchId, seed, rows and payloadBytes. The expected IDs are `<batchId>:<index>`, value is seed+index, payload contains payloadBytes ASCII `x` characters. Each batch is a SQLite transaction or an append/snapshot unit. The worker acknowledges the rows and its timings; the oracle then re-reads the actual content. The storage `attempts.jsonl` lines keep `acknowledged/error/rejected`; the independent terminal verdict is in `summary.json`.

A protocol error or a timeout fails all the worker's active calls. The coordinator sends SIGTERM to the group then SIGKILL, and cleans up the streams. This isolation is specific to the lab's processes; it is not used to attach a user's browser. On Windows, the fallback does not provide the same complete termination of descendants as the POSIX group.

The protocol makes it possible to add Rust, Go or Python without changing the oracle or interpreting internal durations as the only performance evidence. The main time is measured on the coordinator side with a monotonic clock; the internal metrics complement this time.

## Prototype workflow extension

The `workflow-node` worker reuses this envelope: `prepare` receives `path` and `leaseMs`, then `run.op` receives `submit`, `claim`, `commit`, `ack`, `reject` or `snapshot`. `commit` receives a batch `{jobId, token, records}` and returns receipts; `ack` receives these receipts. The contract and the recovery limits are detailed in [workflow.md](workflow.md). The crash markers belong exclusively to the local lab.
