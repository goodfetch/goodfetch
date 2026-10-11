Crate versions checked on crates.io on 2026-10-11: `deltalake` 1.1.1 (2026-10-04), `iceberg` 0.10.1 and `iceberg-catalog-rest` 0.10.1 (2026-08-01), `duckdb` 1.10506.0 (2026-09-30), `databricks-zerobus-ingest-sdk` 2.11.0 (2026-09-29), `postgrest` 1.6.0 (2023-07-25), `supabase_rs` 0.8.0 (2026-10-02), `tokio-postgres` 0.7.18 (2026-06-12), `sqlx` 0.9.0 (2026-05-21), `testcontainers` 0.28.0 (2026-08-06), `testcontainers-modules` 0.15.0 (2026-02-21).

Classes used below:
- **exactly-once fenced**: the destination can reject a stale epoch in the same atomic commit as the data.
- **idempotent keyed**: a stable key makes a replay harmless, but a zombie is not rejected.
- **at-least-once**: duplicates are possible, so deduplication has to happen downstream.

## Databricks (Delta Lake tables)

### Primitives

There are two paths, and they differ.

**Path A: SQL against Unity Catalog managed tables.**
- Multi-statement, multi-table transactions became GA for UC managed Delta tables on 2026-07-22. Iceberg-table writes are still Private Preview ([release notes, July 2026](https://docs.databricks.com/aws/en/release-notes/product/2026/july)).
- The participating tables need catalog commits enabled ([release notes](https://docs.databricks.com/aws/en/release-notes/product/2026/july), [transaction modes](https://docs.databricks.com/aws/en/transactions/transaction-modes)).
- `BEGIN ATOMIC ... END` commits or rolls back as a unit. It allows DML such as `MERGE`, plus `IF` and `SIGNAL` ([ATOMIC compound statement](https://docs.databricks.com/aws/en/sql/language-manual/sql-ref-syntax-txn-begin-atomic)).
- A `SIGNAL` inside the block rolls back the whole transaction ([tutorial](https://docs.databricks.com/aws/en/transactions/tutorial)).
- Non-interactive (`BEGIN ATOMIC`) transactions work through the SQL Statement Execution API (`POST /api/2.0/sql/statements`) and use row-level conflict detection ([transaction modes](https://docs.databricks.com/aws/en/transactions/transaction-modes)).
- Each Statement Execution API call carries one `statement` string. It supports named `:param` parameters ([tutorial](https://docs.databricks.com/aws/en/dev-tools/sql-execution-tutorial)).
- Fencing pattern (**inference**): one `BEGIN ATOMIC` block does three things:
  1. Updates a fence table with `SET epoch = :e WHERE run = :r AND epoch <= :e`.
  2. Runs `SIGNAL` if the row count is 0.
  3. Merges the Records, keyed by delivery ID.
- If a zombie updates the same fence row at the same time, it should hit a row-level conflict and abort (**inference**).
- Moving bulk data inline in SQL is costly. Staging files first, for example in a UC volume, and then running `MERGE` from them is likely needed (**inference**).

**Path B: writing Delta files directly with delta-rs (external or non-UC tables).**
- The Delta protocol defines `txn` (SetTransaction) actions as `appId`/`version` pairs, recorded atomically with the data ([PROTOCOL.md, Transaction Identifiers](https://github.com/delta-io/delta/blob/master/PROTOCOL.md#transaction-identifiers)).
- The protocol guarantees only that the latest `version` per `appId` is in the snapshot. It does not require `version` to increase ([PROTOCOL.md](https://github.com/delta-io/delta/blob/master/PROTOCOL.md#transaction-identifiers)).
- In Spark, the matching writer options are `txnAppId` and `txnVersion`. A duplicate write is skipped, and the options work for batch writes too ([Delta docs](https://docs.delta.io/latest/delta-streaming#idempotent-table-writes-in-foreachbatch)).
- delta-rs exposes this as `CommitProperties::with_application_transaction(Transaction)` ([docs.rs](https://docs.rs/deltalake-core/latest/deltalake_core/kernel/transaction/struct.CommitProperties.html)).
- The delta-rs conflict checker fails a commit with `ConcurrentTransaction` when a winning commit touched an `appId` that the losing commit also writes ([conflict_checker.rs](https://github.com/delta-io/delta-rs/blob/main/crates/core/src/kernel/transaction/conflict_checker.rs)). That makes the flow below an atomic compare-and-set on the `appId` version.
- Fencing pattern (**inference**):
  1. Read the `txn` version for `appId = automan:<run>:<sink>`.
  2. Reject if the stored epoch is higher than ours. Skip if the delivery is already covered.
  3. Commit with `version` encoding (epoch, sequence), for example `epoch << 32 | seq`.
- delta-rs knows the `delta.setTransactionRetentionDuration` table property ([config.rs](https://github.com/delta-io/delta-rs/blob/main/crates/core/src/table/config.rs)). If it is set, old `txn` entries can expire. Keep it unset on fenced tables (**inference**).
- delta-rs 1.0 removed the S3 DynamoDB log store and uses S3 conditional put by default ([rust-v1.0.0 release](https://github.com/delta-io/delta-rs/releases/tag/rust-v1.0.0), [discussion #4482](https://github.com/delta-io/delta-rs/discussions/4482)). MinIO and R2 need `aws_conditional_put=etag` ([S3-like docs](https://delta-io.github.io/delta-rs/integrations/object-storage/s3-like/)).
- delta-rs cannot write to catalog-managed (UC managed) tables yet. Issue [#4549](https://github.com/delta-io/delta-rs/issues/4549) is open, and [#4591](https://github.com/delta-io/delta-rs/issues/4591) was closed as its duplicate. So Path B does not reach UC managed tables.

**Zerobus Ingest (official Databricks push API).**
- Zerobus Ingest is at-least-once. The docs say to deduplicate downstream with `MERGE` on a stable ID ([Zerobus quotas](https://docs.databricks.com/aws/en/ingestion/zerobus-quotas), [recovery](https://docs.databricks.com/aws/en/ingestion/zerobus-recovery)).

### Classification
- Path A (UC managed, SQL `BEGIN ATOMIC`): **exactly-once fenced**.
- Path B (delta-rs, `txn` action): **exactly-once fenced**.
- `MERGE` alone: **idempotent keyed**.
- Zerobus: **at-least-once**.

### Rust client
- There is no general Databricks Rust SDK on crates.io: `databricks-sdk` and `databricks` do not exist (crates.io API).
- `databricks-zerobus-ingest-sdk` 2.11.0 is official ([databricks/zerobus-sdk](https://github.com/databricks/zerobus-sdk)). It was released 2026-09-29, has about 10^5 downloads, and is Apache-2.0. It covers Zerobus only ([Zerobus overview](https://docs.databricks.com/aws/en/ingestion/zerobus-overview)).
- Path A needs a hand-written REST client for the Statement Execution API (**inference**).
- `deltalake` (delta-rs) 1.1.1 is community-run under delta-io. It was released 2026-10-04, has about 3×10^6 downloads, and is Apache-2.0.

### Local test double
- There is no Databricks emulator (none found).
- For Path B, use delta-rs against the local filesystem or MinIO (`testcontainers-modules` feature `minio`).
- For Path A, Databricks Free Edition gives serverless compute only, one 2X-Small SQL warehouse, and no commercial use ([limitations](https://docs.databricks.com/aws/en/getting-started/free-edition-limitations)). It is usable as a manual or nightly check, not as a CI dependency (**inference**).

## Apache Iceberg (REST catalog, object storage)

### Primitives
- A commit is `POST .../tables/{table}` with `requirements` and `updates`. For example, `assert-ref-snapshot-id` checks that a ref points to a given snapshot, which is a compare-and-set on table metadata ([REST OpenAPI](https://github.com/apache/iceberg/blob/main/open-api/rest-catalog-open-api.yaml)).
- The defined requirements are:
  - `assert-create`
  - `assert-table-uuid`
  - `assert-ref-snapshot-id`
  - `assert-last-assigned-field-id`
  - `assert-current-schema-id`
  - `assert-last-assigned-partition-id`
  - `assert-default-spec-id`
  - `assert-default-sort-order-id`
- There is no requirement that asserts a property value ([REST OpenAPI](https://github.com/apache/iceberg/blob/main/open-api/rest-catalog-open-api.yaml)).
- `POST /v1/{prefix}/transactions/commit` commits to several tables atomically, with the same requirement semantics ([REST OpenAPI](https://github.com/apache/iceberg/blob/main/open-api/rest-catalog-open-api.yaml)).
- An optional `Idempotency-Key` header (UUIDv7) replays finalized responses, including 409. It does not replay 5xx ([REST OpenAPI](https://github.com/apache/iceberg/blob/main/open-api/rest-catalog-open-api.yaml)). The spec is in the apache-iceberg-1.12.0 release (2026-09-30).
- Snapshot summaries are string maps ([spec](https://iceberg.apache.org/spec/)). Flink stores its last committed checkpoint ID there for exactly-once ([Flink writes](https://iceberg.apache.org/docs/nightly/flink-writes/)).
- Fencing pattern (**inference**):
  1. Load the table. Read `automan.epoch` and `automan.delivery` from the summaries of the current snapshot, walking ancestors if another writer such as compaction committed.
  2. Reject a stale epoch.
  3. Commit with `assert-ref-snapshot-id = <the snapshot you checked>`.
  4. On 409, reload and re-check.
- This serializes all writers on the branch.
- A 5xx response means the commit state is unknown. Resolve it by finding the delivery ID in the snapshot summaries (**inference**).

### Classification
- With raw REST commits that pin the checked snapshot: **exactly-once fenced**.
- With iceberg-rust 0.10.1 as shipped: **idempotent keyed** at best. See the caveat below.

### Rust client
- `iceberg` and `iceberg-catalog-rest` 0.10.1 come from apache/iceberg-rust, the official ASF project. They were released 2026-08-01, are Apache-2.0, and have about 3.5×10^6 and 2×10^6 downloads.
- Writes are supported through `Transaction::fast_append()`. `FastAppendAction::set_snapshot_properties` sets summary keys, and `with_check_duplicate` checks data file paths ([append.rs v0.10.1](https://github.com/apache/iceberg-rust/blob/v0.10.1/crates/iceberg/src/transaction/append.rs)).
- The transaction actions are append, update properties, update schema, sort order, location, statistics, upgrade format version, and expire snapshots ([transaction/mod.rs v0.10.1](https://github.com/apache/iceberg-rust/blob/v0.10.1/crates/iceberg/src/transaction/mod.rs)).
- Caveat: `Transaction::commit` reloads the table on every attempt and re-applies the actions. The `assert-ref-snapshot-id` it sends is therefore taken from the reloaded table, not from the snapshot the caller checked ([mod.rs `do_commit`](https://github.com/apache/iceberg-rust/blob/v0.10.1/crates/iceberg/src/transaction/mod.rs)).
- `TableCommit::builder().build()` and the `TransactionAction` trait are `pub(crate)`. So a fenced compare-and-set cannot be expressed through the public API ([catalog/mod.rs](https://github.com/apache/iceberg-rust/blob/v0.10.1/crates/iceberg/src/catalog/mod.rs), [action.rs](https://github.com/apache/iceberg-rust/blob/v0.10.1/crates/iceberg/src/transaction/action.rs)).
- The REST client has no multi-table `transactions/commit` and sends no `Idempotency-Key` (grep of [catalog.rs v0.10.1](https://github.com/apache/iceberg-rust/blob/v0.10.1/crates/catalog/rest/src/catalog.rs)).
- Workarounds (**inference**):
  - Write data files and manifests with iceberg-rust, then send the commit with a hand-built REST call.
  - Or contribute upstream.

### Local test double
- `apache/iceberg-rest-fixture` is on Docker Hub (latest 2026-04-29, tag 1.10.1). iceberg-rust's own tests use it with MinIO ([dev/docker-compose.yaml](https://github.com/apache/iceberg-rust/blob/v0.10.1/dev/docker-compose.yaml)).
- `tabulario/iceberg-rest` is stale. Its last tag is 1.6.0 from 2024-08-28 (Docker Hub API).
- Production-like catalogs:
  - [Lakekeeper](https://github.com/lakekeeper/lakekeeper) v0.14.0 (2026-10-07).
  - `apache/polaris` 1.8.0 (2026-09-28).
- `testcontainers-modules` 0.15.0 has no Iceberg, Polaris, or Lakekeeper module. Use `GenericImage` with the `minio` feature (crates.io features list).

## DuckDB / MotherDuck

### Primitives
- In-process DuckDB is ACID with MVCC and optimistic concurrency. Two writers to the same row conflict, and the second one fails ([concurrency](https://duckdb.org/docs/current/connect/concurrency.html)).
- Only one process may open a database file read-write ([concurrency](https://duckdb.org/docs/current/connect/concurrency.html)).
- Multi-process writes are possible through the beta Quack remote protocol, or through DuckLake with a Postgres catalog ([concurrency](https://duckdb.org/docs/current/connect/concurrency.html)).
- `INSERT ... ON CONFLICT DO NOTHING | DO UPDATE` works, and so do `INSERT OR IGNORE` and `INSERT OR REPLACE`. They need a PRIMARY KEY or UNIQUE constraint ([INSERT](https://duckdb.org/docs/current/sql/statements/insert.html)).
- Fencing pattern (**inference**): a fence table updated with `WHERE epoch <= :e`, plus the data inserted with `ON CONFLICT (delivery_id) DO NOTHING`, all in one transaction. Same pattern as PostgreSQL.
- The file lock alone already blocks a zombie in another process from writing to a local file.
- MotherDuck routes writes to one read-write "Duckling" per account. Read-scaling replicas lag ([scaling patterns](https://motherduck.com/docs/concepts/scaling-patterns/)).
- For DuckLake on MotherDuck, concurrent writers in one account succeed only for append-only writes. For conflicting updates, the first to commit wins ([DuckLake](https://motherduck.com/docs/integrations/file-formats/ducklake/)).
- No MotherDuck page explicitly confirms that `BEGIN`/`COMMIT` over `md:` gives the same guarantees as local DuckDB. This is **unverified** and needs a probe test.

### Classification
- Local DuckDB: **exactly-once fenced**.
- MotherDuck: **exactly-once fenced**, likely but **unverified**.

### Rust client
- `duckdb` (duckdb-rs) 1.10506.0 is official ([duckdb/duckdb-rs](https://github.com/duckdb/duckdb-rs)). It was released 2026-09-30, has about 4.7×10^6 downloads, and is MIT.
- The version encodes the DuckDB version, so 1.10506 means DuckDB v1.5.6. Pin with `~` ([README](https://github.com/duckdb/duckdb-rs)).
- It has an `appender-arrow` feature (crates.io features).
- MotherDuck officially supports the same crate with `Connection::open("md:...")` ([MotherDuck Rust](https://motherduck.com/docs/integrations/language-apis-and-drivers/rust/)).

### Local test double
- DuckDB runs in-process (`:memory:` or a temp file), so no container is needed.
- MotherDuck's Lite plan is $0, with 10 GB storage and 10 hours of compute per month, and needs no card ([pricing](https://motherduck.com/product/pricing/)). It could serve a nightly check (**inference**).

## Supabase (PostgREST vs direct Postgres)

### Primitives

**Direct Postgres connection.**
- Full Postgres transactions work, so the fencing table pattern works exactly as for PostgreSQL.
- Direct connections use IPv6 by default, or IPv4 with the paid add-on ([connecting](https://supabase.com/docs/guides/database/connecting-to-postgres)).
- The shared pooler session mode (port 5432) keeps session state and is IPv4 ([connecting](https://supabase.com/docs/guides/database/connecting-to-postgres)).
- Transaction mode (port 6543) does not support prepared statements. Disable them in sqlx or tokio-postgres ([connecting](https://supabase.com/docs/guides/database/connecting-to-postgres)).

**PostgREST (Data API).**
- Every request runs inside one transaction. Any error, or a `RAISE` in a function, rolls it back ([transactions](https://docs.postgrest.org/en/stable/references/transactions.html)).
- `POST /rpc/fn` on a `VOLATILE` function runs read-write ([transactions](https://docs.postgrest.org/en/stable/references/transactions.html)).
- A fencing function that checks and updates the epoch and inserts the Records, raising on a stale epoch, is therefore atomic (**inference** from the docs).
- Upsert: `POST` with `Prefer: resolution=merge-duplicates` or `ignore-duplicates`, with `on_conflict` for a UNIQUE column ([tables and views](https://docs.postgrest.org/en/stable/references/api/tables_views.html)).
- A bulk insert is a single `INSERT` statement ([tables and views](https://docs.postgrest.org/en/stable/references/api/tables_views.html)).
- The Data API requires RLS. With no policies, every request is denied ([connecting](https://supabase.com/docs/guides/database/connecting-to-postgres)).

### Classification
- Direct connection: **exactly-once fenced**.
- PostgREST RPC to a fencing function: **exactly-once fenced**.
- PostgREST upsert alone: **idempotent keyed**.

### Rust client
- There is no official Supabase Rust SDK. Supabase's client-libraries page lists none ([client libs](https://supabase.com/docs/guides/api/rest/client-libs)), and the Rust entries live in the [supabase-community](https://github.com/supabase-community) org.
- Recommended: `sqlx` 0.9.0 (2026-05-21, about 1.6×10^8 downloads, MIT/Apache-2.0) or `tokio-postgres` 0.7.18 (2026-06-12, about 7×10^7 downloads, MIT/Apache-2.0) over a direct or session-pooler connection.
- `postgrest` (postgrest-rs) 1.6.0 is community. Its last release was 2023-07-25, it has about 3×10^5 downloads, and is MIT/Apache-2.0. The repo is not archived.
- `supabase_rs` 0.8.0 is community (xylex-group). It was released 2026-10-02, has about 10^5 downloads, and is MIT.
- PostgREST itself is at v16.4 (2026-09-24, GitHub releases).

### Local test double
- `supabase start` (CLI v2.120.0, 2026-10-06) runs Postgres on `127.0.0.1:54322` and PostgREST at `127.0.0.1:54321/rest/v1` in Docker ([CLI getting started](https://supabase.com/docs/guides/local-development/cli/getting-started)).
- For the direct-connection path, a plain Postgres testcontainer is enough. `testcontainers-modules` 0.15.0 has a `postgres` feature, but it depends on `testcontainers` ^0.27, not the latest 0.28 (crates.io API).
