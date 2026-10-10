# Durable workflow and comparison with a shared SQLite

Survey and local trials of October 8, 2026, Paris time. The executed results are consolidated in the [lab report](../lab/reports/workflow-2026-10-08.md). This note explains the controls, without attributing an advantage to the language alone.

## Controlling the engine actually loaded

Comparing Node `DatabaseSync` and Rust SQLx does not automatically control the version, the compilation options, the transactions or the allocations. The new lane builds **a single SQLite 3.53.4 library**, with a common C shim, then three synchronous clients: Node N-API, Rust FFI and Go/cgo. Each insertion makes a call to the same shim; each connection reuses its statement. The SQLite C API exposes these operations; Node-API and cgo provide the boundaries with the runtimes. [SQLite C API](https://www.sqlite.org/c3ref/intro.html), [Node-API 25.5.0](https://nodejs.org/download/release/v25.5.0/docs/api/n-api.html), [cgo](https://pkg.go.dev/cmd/cgo).

The official archive is pinned by SHA-256. Its `sqlite3.c` is also verified against the SHA3-256 of the official release. The shim/binding sources, toolchains, flags and compiled artifacts are recorded. The runner verifies the path actually loaded via `dladdr`, the file's SHA, the source ID, the options and the effective PRAGMAs. SQLite's internal symbols are hidden to avoid interposition with the SQLite embedded in Node. [SQLite 3.53.4 release and official hashes](https://www.sqlite.org/releaselog/3_53_4.html).

The first control used 3.46.0, like the historical SQLx lane. A new check of the documentation identified the WAL-reset bug, fixed from 3.51.3 onward. The control is kept; the main comparison is repeated on 3.53.4. The documented case concerns several connections writing/checkpointing simultaneously. Successful local tests on the old version therefore do not constitute a qualification of this fix. [SQLite: WAL-reset bug](https://www.sqlite.org/wal.html#the_wal_reset_bug).

## What the timings still compare

The three clients build the same records, encode JSON then insert into SQLite. The encoders, record representations, allocations and FFI transitions differ. The experiment therefore compares three implementations of this write pipeline under a common SQL engine. It does not isolate an intrinsic property of Rust, Node or Go. Another comparison will have to separate generation/encoding, binding cost and SQL engine; one FFI call per batch would also change the workload.

The writer processes open the same database per cell. SQLite WAL has only one active writer at any given moment; adding processes can increase lock waiting. FULL and NORMAL remain two distinct contracts. The autocheckpoint is explicitly set to 1,000 pages; the large-batch campaign increases the volume written with this setting, without directly counting each checkpoint. [How WAL works, concurrency and checkpoint](https://www.sqlite.org/wal.html).

## Proving the contract before measuring the workflow

The Node HTTP prototype links acquisition/extraction, private oracle, bounded results, batch, data transaction and job states, then the ack transaction. The tests target the boundaries: expired lease and replaced token, same job replayed with different data, apparently correct false result, atomic rollback, response lost after commit, takeover by a new coordinator process. The tasks concern idempotent HTTP reads. These tests prove neither that an external effect happens exactly once, nor durability after a power cut.

The campaigns use fresh databases, an order randomized by blocks, frozen sources and a final oracle after reopening. Instrumentation, IPC and drain remain visible in the durations. Reconciled acks count in the latency of the same coordinator; the complete coordinator restart is currently a functional qualification. The metrics of the [workflow contract](../lab/docs/workflow.md) specify their denominators and the difference in RSS scope between lanes.

The integrated prototype uses Node; the new Rust/Go workers make up only the write lane. The integrated cost of a Rust/Go core and that of a Rust CDP driver versus co-located Node remain experiments to be run with equal guarantees.

## Pool wakeup in the workflow

The integrated series revealed the cost of periodic waiting when the writer momentarily leaves the pool queue empty. The public API `AutoscaledPool.notify()` allows an immediate check. A premature call during initialization was detected by the real test and fixed; the variant only notifies after entering a first handler. A separate A/B series randomizes the poll/notify modes and keeps all the oracles. The gain observed at C1 does not generalize to all concurrency levels. [Crawlee 3.18 API](https://crawlee.dev/js/api/core/class/AutoscaledPool).
