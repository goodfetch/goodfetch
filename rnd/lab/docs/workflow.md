# Durable workflow prototype

The lab now runs HTTP acquisition and extraction, independent validation, a bounded result queue, batched writing, atomic commit and persistent acknowledgment. This Node prototype establishes the contract to compare; it does not choose the runtime of the future portable core.

```sh
cd /Users/jb/Dev/automan/lab
# Use the qualified Node: 25.5.0, then check node --version.
pnpm workflow campaigns/workflow-throughput.json
pnpm workflow campaigns/workflow-recovery.json
pnpm workflow campaigns/workflow-pool-wakeup.json
```

## Observable contract

Submitted jobs are persisted in SQLite, with their complete task. The same identifier cannot designate a different task. `claim` assigns a lease and a fresh token; an expired token cannot commit after a new assignment. Record validation belongs to the coordinator and its private oracle.

Each producer waits for the ack of its result before taking another job. At most `concurrency` jobs remain active and their results may wait for the writer. The result queue and the in-progress batch share this same bound. The durable backlog corresponds to the finite submitted corpus, bounded separately by the runner's budgets; this prototype does not yet implement continuous backlog admission.

Only one batch can be in progress. It is sent when `batchJobs` results are available or when `maxBatchWaitMs` expires. `batchJobs` must remain less than or equal to `concurrency`. The result batch and the `committed` states of the jobs are recorded in the same transaction. Then a second transaction records the acks from the returned receipts. No job is announced as finished before this ack.

The sink uses WAL, `synchronous=FULL`, `foreign_keys=ON`, `busy_timeout=5000`, `wal_autocheckpoint=1000`. The effective PRAGMAs and the SQLite version are kept in the summary. The final oracle reopens the database, checks the IDs/prices/duplicates/omissions and verifies structural integrity and foreign keys.

## Recovery and errors

After a crash before the commit, no partial result of the batch remains visible. After the lease expires, the job is taken over with a new token. After a commit whose response is lost, the coordinator validates the persisted records then acknowledges the existing receipt. An identical replay returns this receipt; a replay with different records fails. An already acknowledged job is not acquired a second time when the same corpus is submitted again.

An incomplete or incorrect result is recorded as `failed`, with the reason, without insertion into `results`. Acquisition errors and worker losses interrupt the generation; full recovery is limited by `restartLimit`. There is no unlimited retry loop.

The recovery campaigns inject SIGKILL into the writer, during the transaction or after COMMIT and before the response. Two functional tests also stop a coordinator and its workers belonging to the test, then launch a **new coordinator process**: after claim and between commit and ack. The fixture server and the independent oracle remain available outside these processes.

## Measurements

Each cell creates a fresh database and fresh fixtures. The combinations are randomized per block. Startup is separate; `workMs` includes acquisition, extraction, validation, queue wait, commit, ack, any recovery and worker drain. Useful throughput counts the jobs validated after reopening and also includes `finalizationMs`. Throughput including startup is also provided.

Ack latency runs from the first lease acquisition to the ack, including writer retries. Acquisition and extraction remain a combined measurement. The timings of reconciled jobs are included in the events when their first lease acquisition belongs to the same coordinator; latency spanning a complete coordinator stop is not measured yet. The coordinator restart tests qualify a functional result.

RSS covers **the coordinator, the fixtures, the oracle and its descendants**, unlike the storage lane, which measures only the writer processes. Sampling at 250 ms can miss peaks. The events are actually written during the measurement: the instrumentation and the JSONL exchanges are part of the prototype's cost.

The folders contain the manifest, frozen sources, summary, HTML report and, per cell, events, attempts and re-read state. The CLI fails if a cell fails or if sources/executables change during the campaign.

## Limits that remain to be tested

The schema has no migration; only reuse a database created by this version of the prototype. Leases use the wall clock and have no heartbeat. The fixture data and their oracle are not a durable discovery system. The recovery tests concern idempotent HTTP reads; they do not prove exactly-once execution of a click, payment or external effect.

Remaining in particular: full disk, cancellation, crash during the ack, repeated arbitrary stops, heartbeat, durable concurrent readers, multi-domain scheduling, distributed workers, external outputs/outbox, large streaming corpus, CPU/I/O, integrated Rust and Go cores, Chromium backend and deterministic plans. No power-cut test is executed.

For pooled Crawlee, `poolWakeModes` makes it possible to compare `poll` and `notify` in a separate campaign. `notify` uses the public API after new requests are added, as soon as a first handler has demonstrated the end of native initialization. The historical `poll` mode remains the default.
