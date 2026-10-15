# Goodfetch Lab — JSON, JSONL and SQLite storage

**October 7, 2026 — research and proposed protocol.** This document complements the [general lab method](2026-10-07-lab-testing-engineering.md). It contains no benchmark results. The matrix and size choices are proposed local prescriptions, adapted to the request to measure writing and concurrency.

## 1. Separate four costs

The lab must measure **corpus generation, serialization, writing and publication/acknowledgement** separately, then their combination in the scraper pipeline. A loop that builds new objects, encodes JSON and inserts each row with its own commit does not measure SQLite's speed alone.

| Proposed rail | Input | Measurement | Use of the conclusion |
| --- | --- | --- | --- |
| Serialization only | Deterministic in-memory objects | Encoding, bytes produced, CPU/memory | Compare encoders and data structures |
| Sink only | Corpus of prepared bytes | Write/commit/sync and queue wait | Compare sinks and durability policies |
| Serialization + sink | Same logical corpus | Time to publication under contract | Compare complete useful implementations |
| Crawl + extraction + sink | Same fixture graph | Correct results persisted/second | Measure backpressure and the system bottleneck |

Include the preparation cost in a separate measurement; a prebuilt buffer is not free in the product. JSON/JSONL and SQLite do not provide the same functions: expose their contracts instead of producing a global "fastest storage" ranking.

## 2. SQLite: what the sources establish

WAL allows simultaneous readers and a writer, but **only one active writer per database**. Long-running readers can prevent a checkpoint from progressing; WAL requires the processes to share the same machine and is not designed for this mode on a network filesystem. The automatic checkpoint triggers by default at 1,000 pages, with a default that can be changed at build time. [SQLite WAL documentation](https://www.sqlite.org/wal.html).

In WAL mode, `synchronous=FULL` adds a WAL sync at each commit. `NORMAL` preserves consistency but can lose commits after a power failure or OS crash; SQLite distinguishes this from an application crash. `integrity_check` verifies structure and constraints, not foreign key errors: those require `foreign_key_check`. [PRAGMA reference](https://www.sqlite.org/pragma.html#pragma_synchronous), [integrity check](https://www.sqlite.org/pragma.html#pragma_integrity_check).

`busy_timeout` installs a handler that waits before returning `SQLITE_BUSY`, and does not turn the database into an engine with several active writers. The handler may not be called if SQLite detects a deadlock risk. [Timeout](https://www.sqlite.org/c3ref/busy_timeout.html), [handler and deadlocks](https://www.sqlite.org/c3ref/busy_handler.html).

`BEGIN IMMEDIATE` requests a write transaction from the start and can fail with `SQLITE_BUSY`; `DEFERRED` starts differently, and a read→write upgrade can fail. [SQLite transactions](https://www.sqlite.org/lang_transaction.html).

The SQLite FAQ stresses the gain from grouping several inserts in one transaction. It also states that its old HDD figures are no longer contemporary references: **reuse the mechanism, not the old throughputs**. [FAQ, question 19 and update](https://www.sqlite.org/faq.html#q19).

## 3. Node and Rust: avoiding asymmetric defaults

| Backend examined | Verified fact | Consequence for the lab |
| --- | --- | --- |
| Node 24.19.0 `node:sqlite` | `DatabaseSync` exposes synchronous operations; default timeout `0`; prepared statements available; stability "release candidate" 1.2 | Configure timeout and PRAGMAs; reuse the connection and the statement; document the exact version |
| SQLx 0.8.6 SQLite | `busy_timeout` defaults to 5 s; journal mode not set; the comment announces synchronous FULL but the pragma initialization is `None`, so it depends on the compiled SQLite | Use WAL/FULL or NORMAL and explicit timeouts; check the effective values |
| SQLx 0.8.6 connection | Blocking SQLite is executed via a background thread and channels; an explicit close reports errors that a simple drop does not surface | Measure the driver/thread overhead and await the close |

Sources: [Node at version 24.19.0](https://nodejs.org/download/release/v24.19.0/docs/api/sqlite.html), [SQLx options at tag v0.8.6](https://github.com/transact-rs/sqlx/blob/v0.8.6/sqlx-sqlite/src/options/mod.rs), [SQLx connection at the same tag](https://github.com/transact-rs/sqlx/blob/v0.8.6/sqlx-sqlite/src/connection/mod.rs).

**Local prescription:** `Promise.all` around `DatabaseSync` in the same thread proves no writer concurrency. For the sink integrated into the crawler, place the synchronous Node writer in a dedicated worker or process with a bounded queue. Publish this topology and a minimal synchronous sink loop separately; the IPC cost is real. On the Rust side, a SQLite connection pool remains limited by the database's write lock.

Node versus SQLx is not enough to isolate a "language" effect: SQLite versions/builds, synchronous binding versus thread/channel, statements and topology differ. Record `sqlite_version()`, `PRAGMA compile_options`, the schema and the effective options to explain these variables.

## 4. Matrix of comparable contracts

| Proposed cell | Output / publication unit | Measured acknowledgement | Announced atomicity | Scope |
| --- | --- | --- | --- | --- |
| JSON document buffered | A complete array in a run-specific file | End of writes/close | No usable partial result guaranteed | Exploratory batch export |
| JSON document published | Temporary file in the same folder, then rename to a unique name | Writes then file sync, rename, folder sync if supported | Publication of a complete document | Complete export; OS guarantee made explicit |
| JSON per object | One uniquely named file per record | Writes/close; separate sync variant | Publication according to each file's strategy | Cost per object and namespace |
| JSONL buffered | Complete lines appended by one writer | Writes/flush of application buffers | No multi-record transaction | Streaming; no power-loss evidence |
| JSONL synced per batch | Batches of `B` lines, one writer | All lines written then file sync | No all-or-nothing for the batch | Verifiable prefix of lines after a crash |
| SQLite WAL NORMAL | Rows in a transaction of `B` records | Commit | SQLite transaction | Series without power-loss durability for the last commits |
| SQLite WAL FULL | Same schema and batches | Commit, sync according to SQLite/VFS | SQLite transaction | Reinforced durability series, explicit OS/device assumptions |

The buffered, NORMAL and FULL cells have different contracts and remain visible as such. JSONL sync and SQLite FULL can meet a common goal of **acknowledged records being kept**, but JSONL alone does not offer an atomic batch transaction. If all-or-nothing is required, this missing capability must appear; do not add it implicitly to the score.

A returned write, a flushed application buffer, a synced file and a record verified after reopen are four distinct observations. `close()` and a rename do not by themselves prove a durable publication.

## 5. Fsync and rename: the important limits on this Mac

The Linux documentation for `rename()` describes atomic replacement; `fsync(file)` does not guarantee that the directory entry has reached the disk: the directory must also be synced. This justifies a strategy of temp file in the same folder → sync file → rename → sync directory **on systems where these operations are supported**. [rename](https://man7.org/linux/man-pages/man2/rename.2.html), [fsync](https://man7.org/linux/man-pages/man2/fsync.2.html).

Node `FileHandle.sync()` requests the flush to the device with an OS/device-dependent implementation; `writeFile({flush:true})` calls this operation, not a complete namespace publication protocol. Avoid several concurrent `writeFile()` calls on the same file without awaiting their resolution. [Node 24.19 filesystem](https://nodejs.org/download/release/v24.19.0/docs/api/fs.html#filehandlesync).

The archived Apple manual distinguishes `fsync()` from `F_FULLFSYNC`, which also requests a flush of the device's buffers. The lab must not announce that a Node `sync()` proves this guarantee on current APFS/macOS. Record the method and its errors/actual support; the SQLite options `fullfsync`/`checkpoint_fullfsync` remain explicit variables. [Apple manual](https://developer.apple.com/library/archive/documentation/System/Conceptual/ManPages_iPhoneOS/man2/fsync.2.html), [PRAGMA fullfsync](https://www.sqlite.org/pragma.html#pragma_fullfsync).

Never make a sync/directory sync error disappear by treating it as a success. If this level is not available, label the cell according to the operations actually performed.

## 6. Concurrency must have several axes

| Proposed axis | First campaign points | Question |
| --- | --- | --- |
| Logical producers `P` | 1 / 4 / 16 / 64 | Does the writer impose backpressure on the crawler? |
| Real writers `W` | 1 / 2 / 4 / 8 processes or workers | Where is the contention and what does it cost? |
| Databases / shards `D` | 1 then `D=W` in another series | Does partitioning allow more throughput, and at what cost? |
| Batch `B` | 1 / 100 / 1,000 records | Amortization versus latency and replay window |
| Payload | ~256 B / 4 KiB / 64 KiB, actual bytes recorded | Binding/metadata or bandwidth? |
| Concurrent reading | No reader, then short/long readers | Impact of the reader on checkpoint and WAL growth |

**Do not run the full Cartesian product.** First local track: `W=1`, sweep `P` and `B`, then the same database with `W=2/4`, finally separate shards. The sharded series includes merge/final read, number of files and overall cost; it does not have the transactional contract of a single database.

JSONL starts with a single writer receiving from the producers. Several appenders to the same file are a separate experimental cell, with interleaving/truncation validation; do not assume that a serialized line always corresponds to a single atomic write. JSON per object can parallelize distinct files with disjoint names; no race on the same path.

Publish `P`, `W`, connections, processes/threads, `D`, max queue, actual batch, batch fill delay and timeout. Queue and lock wait time belong to useful latency; an unbounded queue that inflates RAM is not a sustainable improvement.

## 7. Minimal local configuration to try

Start with the same corpus/schema, `W=1`, prepared statement, `BEGIN IMMEDIATE`, bound inserts, commit per batch. WAL, synchronous and busy timeout set at opening, then read back on **each connection**; a campaign-specific DB, without silent reuse of a previous profile. Example contract: `busy_timeout=5000 ms`, `B=100`, a higher and explicit total deadline; the values remain tunable.

For SQLite, keep the declared automatic checkpoint as the first baseline; measure the `.db` + WAL + SHM size and checkpoint/close separately, plus the complete duration up to the final drain. An experiment that defers all checkpoints beyond its window must display this deferred work. The PASSIVE checkpoint can remain incomplete; FULL/RESTART/TRUNCATE have different blocking behaviors. [Official checkpoint API](https://www.sqlite.org/c3ref/wal_checkpoint_v2.html).

To compare retention of the same payload, a table `id TEXT PRIMARY KEY, payload TEXT NOT NULL` is a useful baseline: JSON/JSONL and SQLite store the same logical data. Then add business columns/indexes in a separate series; their cost and their read capabilities have real product value. Do not replace a failed insert with `INSERT OR IGNORE`, which masks a duplicate.

## 8. Oracle, metrics and incidents

Shared corpus: stable IDs, Unicode/escaping, null, collections and numeric values within an identical representable range. Semantic oracle on all records; counting alone is insufficient. Identical bytes only for the pre-encoded rail; otherwise encoders can order/escape differently without losing the value. Verify IDs, payload and an independent normalized hash.

After the timed window: reopen from a fresh process, parse the JSON/each JSONL line, query all SQLite rows, compare exact sets/values/versions; `integrity_check` and `foreign_key_check` depending on the schema. Validation results kept separate from write duration, plus a validated end-to-end metric.

Metrics: validated records/s, logical and physical bytes/s, artifact bytes, CPU-s, writer and producer memory, queue wait, lock wait, commit/sync p50/p95 with sample counts, `BUSY`/retries/rejections, checkpoint and end of publication. Keep raw results, configurations and randomized order; do not compare a NORMAL throughput with a FULL one without stating the contract.

Local faults: writer killed before/after commit or sync, before/after publication and before/after ack; keep a ledger of acknowledged IDs on the supervisor side. After reopen, verify that they are retained and report additional unacknowledged records as a possible effect of the cut before ack. JSONL can have an incomplete tail: keep the artifact and diagnosis, apply a declared recovery policy, not a silent repair that alters the oracle.

**A SIGKILL tests a process crash, not a power loss or kernel crash.** The OS cache and the device remain active. This local track can demonstrate protocol errors, integrity after incidents and recovery behavior; it does not experimentally validate persistence promises under power loss. No reboot or power cut of the work machine in this batch.

## 9. Proposed decision for Goodfetch

First hypothesis: concurrent producers → bounded queue → single batched writer; JSONL for sequential export/log, SQLite WAL FULL for transactional local state, NORMAL as an option with an explicit acceptable-loss contract. These are candidates to measure, not a definitively qualified choice. Shards or several writers come in only after contention curves and merge cost are known.

The local track must deliver record fixtures, Node `node:sqlite`, Rust SQLx 0.8.6 separate from the crawler adapters, JSON/JSONL files, a limited matrix, full validation, manifests and raw results. It is not necessary to introduce PostgreSQL, a broker or remote storage to answer this initial question.
