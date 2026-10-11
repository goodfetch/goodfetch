Crate versions checked on crates.io API, 2026-10-11: elasticsearch 9.1.0-alpha.1 / 8.19.0-alpha.1 (2025-08-08), opensearch 2.4.0 (2026-03-24), clickhouse 0.15.2 (2026-08-28), klickhouse 0.15.3 (2026-05-26), snowflake-api 0.14.0 (2025-10-23), snowflake-connector-rs 1.1.0 (2026-07-19), google-cloud-bigquery 0.18.0 (2026-09-30), google-cloud-bigquery-v2 1.0.0 (2026-09-24), gcp-bigquery-client 0.28.0 (2026-02-01), gcloud-bigquery 1.7.0 (2026-06-02), aws-sdk-redshiftdata 1.121.0 (2026-10-01), tokio-postgres 0.7.18 (2026-06-12), sqlx 0.9.0 (2026-05-21), testcontainers 0.28.0 (2026-08-06), testcontainers-modules 0.15.0 (2026-02-21).

# Warehouse and search destinations: write guarantees, Rust clients, test doubles

Classes used:
- **exactly-once fenced**: the write and a stale-epoch check commit atomically.
- **idempotent keyed**: a retry overwrites by key deterministically, but no atomic epoch check covers the whole delivery.
- **at-least-once**: duplicates are possible.

A pattern recurs for the SQL warehouses. One transaction does three things: it advances a fence row (`UPDATE fence SET epoch = :e WHERE run = :r AND epoch <= :e`), it writes the data with a guard (`MERGE ... ON delivery_id` plus `WHERE EXISTS (SELECT 1 FROM fence WHERE run = :r AND epoch = :e)`), and it commits. A zombie's transaction then writes zero rows. Applying this pattern to each warehouse below is **inference**; the vendor docs only document the building blocks.

## Elasticsearch

**Class: idempotent keyed, with a per-document stale-epoch guard. No global fence.**

- `op_type=create` makes the index operation fail if the `_id` already exists ([Index API](https://www.elastic.co/docs/api/doc/elasticsearch/operation/operation-index)).
- `if_seq_no` + `if_primary_term` give compare-and-set on one document. A mismatch returns `VersionConflictException`, HTTP 409 ([Index API](https://www.elastic.co/docs/api/doc/elasticsearch/operation/operation-index), [optimistic concurrency control](https://www.elastic.co/docs/reference/elasticsearch/rest-apis/optimistic-concurrency-control)).
- `version_type=external` writes only if the given version is strictly higher than the stored one. A lower or equal version is rejected with a 409 conflict. `external_gte` also accepts an equal version, and the docs warn it "can result in loss of data" if misused ([Index API](https://www.elastic.co/docs/api/doc/elasticsearch/operation/operation-index)).
- **Inference:** with `version = epoch` and `external_gte`, a same-epoch retry overwrites the document and a lower epoch gets a 409. That is per-document epoch fencing. It does not stop a zombie from creating a document that the new epoch has not written yet. No multi-document transaction exists.
- Bulk supports `version`, `version_type`, `if_seq_no`/`if_primary_term`, and a `create` action per item. The response has an `errors` flag and a status per item, so a bulk request is not atomic ([Bulk API](https://www.elastic.co/docs/api/doc/elasticsearch/operation/operation-bulk)). That bulk applies items independently is **inference** from the documented response shape.

Rust client: `elasticsearch`, official ([repo](https://github.com/elastic/elasticsearch-rs)), Apache-2.0, about 24M downloads (10^7). Every release is `-alpha`; the newest are 9.1.0-alpha.1 and 8.19.0-alpha.1, from 2025-08-08. The README calls the client "in an alpha state". No release has shipped in about 14 months.

Test double: the official Docker image, plus the `elastic_search` feature of [testcontainers-modules 0.15.0](https://crates.io/crates/testcontainers-modules) (feature list taken from the crates.io API).

## OpenSearch

**Class: idempotent keyed, with a per-document stale-epoch guard. No global fence.**

- It has the same primitives: `op_type=create`, `if_seq_no`/`if_primary_term`, and `version_type` = `external` ("only indexes if the specified version is greater than the stored version") or `external_gte` ("should be used with care") ([Index Document API](https://docs.opensearch.org/latest/api-reference/document-apis/index-document/)).
- The per-document fencing note from Elasticsearch applies here too (**inference**).

Rust client: `opensearch` 2.4.0 (2026-03-24), official ([opensearch-rs](https://github.com/opensearch-project/opensearch-rs)), Apache-2.0, about 3.4M downloads (10^6). The previous release was 2.3.0 in 2024-08, so releases are slow.

Test double: the official `opensearchproject/opensearch` Docker image. testcontainers-modules 0.15.0 has no OpenSearch feature, so use `GenericImage` (checked against the crate's feature list).

## ClickHouse

**Class: idempotent keyed, only within a window or eventually. Not fenced.**

- Insert deduplication is on by default for `Replicated*MergeTree`. It is bounded by `replicated_deduplication_window` (a count) and `replicated_deduplication_window_seconds` (a time) ([dedup on retries](https://clickhouse.com/docs/guides/developer/deduplicating-inserts-on-retries)).
- A plain `MergeTree` table "deduplicates nothing" until `non_replicated_deduplication_window` is set above 0 (same page).
- `insert_deduplication_token` replaces the data hash with a string you supply. A delivery ID fits here. Reusing the token drops the insert even if the data differs (same page).
- Since 26.2, async inserts deduplicate like sync inserts through `deduplicate_insert`, which defaults to `enable`; `async_insert_deduplicate` is legacy. Materialized views follow `deduplicate_blocks_in_dependent_materialized_views` (same page).
- If more inserts than the window size happen during a retry sequence, the data can be inserted twice (same page).
- `ReplacingMergeTree(ver)` keeps the row with the highest `ver`, but only at merge time, "at an unknown time". The docs say "you should not rely on it"; use `FINAL` at read time ([ReplacingMergeTree](https://clickhouse.com/docs/engines/table-engines/mergetree-family/replacingmergetree)). **Inference:** with `ver = epoch`, reads with `FINAL` converge to the highest epoch for each key. Zombie rows are hidden, not rejected.
- Transactions are experimental (`allow_experimental_transactions`). They require non-replicated MergeTree, an Atomic database, and Keeper, and are "Not supported in ClickHouse Cloud" ([transactions](https://clickhouse.com/docs/guides/developer/transactional)). No atomic fence check exists in production.

Rust client: `clickhouse` 0.15.2 (2026-08-28), official ([ClickHouse/clickhouse-rs](https://github.com/ClickHouse/clickhouse-rs)), MIT OR Apache-2.0, about 15M downloads (10^7). `Insert::with_setting(name, value)` applies a setting to one INSERT ([docs.rs](https://docs.rs/clickhouse/0.15.2/clickhouse/insert/struct.Insert.html)), so `insert_deduplication_token` can be set for each delivery. The `test-util` feature provides `Client::with_mock` ([docs.rs](https://docs.rs/clickhouse/0.15.2/clickhouse/struct.Client.html)). The community alternative is `klickhouse` 0.15.3 (native protocol, about 0.3M downloads).

Test double: the `clickhouse/clickhouse-server` Docker image and the `clickhouse` feature of testcontainers-modules. A single node needs `non_replicated_deduplication_window > 0` to exercise dedup.

## Snowflake

**Class: exactly-once fenced with SQL transactions (inference). Snowpipe Streaming gives exactly-once per channel, with weak fencing.**

- Explicit transactions use `BEGIN` / `COMMIT` / `ROLLBACK`. READ COMMITTED is the only isolation level for tables ([transactions](https://docs.snowflake.com/en/sql-reference/transactions)).
- On standard tables, UPDATE, DELETE, and MERGE "hold locks that generally prevent them from running in parallel" with each other. INSERT is not blocked. Hybrid tables lock individual rows (same page).
- **Inference:** two epochs' fence UPDATEs are serialized, so the guarded MERGE of the stale epoch writes zero rows. Snowflake does not document whether a blocked UPDATE re-reads the newest committed row. Verify this with a lab test.
- PRIMARY KEY and UNIQUE are "not enforced" on standard tables but are enforced on hybrid tables ([constraints](https://docs.snowflake.com/en/sql-reference/constraints-overview)). Idempotency therefore needs `MERGE` on the delivery ID, or a hybrid ledger table.
- The SQL API runs several statements in one request with `MULTI_STATEMENT_COUNT`. When one fails, the earlier statements have already run and the later ones do not run ([multiple statements](https://docs.snowflake.com/en/developer-guide/sql-api/submitting-multiple-statements)). Wrap them in BEGIN/COMMIT.
- Snowpipe Streaming, high-performance architecture: Named Channels give "ordered, exactly-once ingestion within each channel by using offset tokens". Elastic Channels are at-least-once ([overview](https://docs.snowflake.com/en/user-guide/snowpipe-streaming/data-load-snowpipe-streaming-overview)).
- Reopening a channel returns the last committed offset token and discards uncommitted data ([Named Channels](https://docs.snowflake.com/en/user-guide/snowpipe-streaming/snowpipe-streaming-channels)).
- The REST Open Channel call "bumps the client sequencer". A request with an old continuation token fails with `STALE_CONTINUATION_TOKEN_SEQUENCER` ([REST API](https://docs.snowflake.com/en/user-guide/snowpipe-streaming/snowpipe-streaming-high-performance-rest-api)). A 409 "channel invalidated" can mean a superseded client ([error handling](https://docs.snowflake.com/en/user-guide/snowpipe-streaming/snowpipe-streaming-high-performance-error-handling)).
- **Inference:** reopening the channel at each resume fences the zombie's channel handle. This is fencing by most recent open, not by epoch comparison: a zombie that reopens later wins.
- Classic architecture: reopen the channel and restart from the latest committed offset token ([classic best practices](https://docs.snowflake.com/en/user-guide/snowpipe-streaming/snowpipe-streaming-classic-recommendation)). I found no page that documents classic-specific invalidation.

Rust client: there is no official Rust driver. The official list covers Go, JDBC, .NET, Node.js, ODBC, PHP, and Python; Rust appears only as the internal "Universal Core" ([drivers](https://docs.snowflake.com/en/developer-guide/drivers)). Snowpipe Streaming SDKs exist for Java, Python, and Node.js on a shared Rust core, with no public Rust SDK. From Rust, the REST API is the route.
- Community crates:
  - `snowflake-connector-rs` 1.1.0 (2026-07-19), estie-inc, MIT, about 4.5M downloads (10^6), with key-pair auth.
  - `snowflake-api` 0.14.0 (2025-10-23), mycelial, Apache-2.0, about 7.5M downloads (10^6).
- The official SQL REST API can also be called directly.

Test double:
- No official emulator.
- [fakesnow](https://github.com/tekumara/fakesnow) v0.11.21 (2026-10-10), Apache-2.0, built on DuckDB. It has a standalone HTTP server mode (`ghcr.io/tekumara/fakesnow`) that implements the connector endpoints `/session/v1/login-request` and `/queries/v1/query-request`, and it has transaction tests. That it works with the Rust crates is **inference**. It does not cover the SQL API v2 or Snowpipe Streaming.
- LocalStack for Snowflake is paid: Base costs $29 per license per month and there is no free tier ([pricing](https://www.localstack.cloud/pricing)). It lists Transaction Management, Snowpipe, and the SQL API ([features](https://docs.localstack.cloud/snowflake/features/)).

## Google BigQuery

**Class: exactly-once fenced through multi-statement transactions. Storage Write API committed streams with offsets give exactly-once per stream. `insertAll` is at-least-once.**

- Storage Write API:
  - The default stream is at-least-once.
  - A committed-type stream with client offsets gives exactly-once, because "the write operation is only performed if the offset value matches the next append offset" ([Write API](https://docs.cloud.google.com/bigquery/docs/write-api)).
  - A pending-type commit "is an atomic operation" through `BatchCommitWriteStreams`.
  - `FinalizeWriteStream` means "no new data can be appended to it".
- `ALREADY_EXISTS` means the offset was already written ("can be ignored"). `OUT_OF_RANGE` means the offset is past the end of the stream ([storage.proto](https://github.com/googleapis/googleapis/blob/master/google/cloud/bigquery/storage/v1/storage.proto)).
- **Inference:** a resumed run can finalize the old epoch's stream, after which zombie appends get `STREAM_FINALIZED`. That is a fence, but only if the stream name is checkpointed.
- Legacy `tabledata.insertAll` `insertId` gives "best effort de-duplication for up to one minute", which is not guaranteed ([streaming](https://docs.cloud.google.com/bigquery/docs/streaming-data-into-bigquery)).
- Multi-statement transactions provide ACID with snapshot isolation. Of two transactions that mutate the same table, the conflicting one is cancelled. The limit is 100 tables and 100,000 partition modifications per transaction ([transactions](https://docs.cloud.google.com/bigquery/docs/transactions)).
- **Inference:** the fence-plus-guarded-MERGE transaction rejects a stale epoch atomically. Rows from the Write API cannot join that transaction, so a fenced path is to stage the rows first and then run the MERGE in a transaction.
- Primary keys are not enforced ([keys](https://docs.cloud.google.com/bigquery/docs/primary-foreign-keys)).

Rust client:
- Official: `google-cloud-bigquery` 0.18.0 (2026-09-30), Apache-2.0, from [googleapis/google-cloud-rust](https://github.com/googleapis/google-cloud-rust), about 0.26M downloads (10^5). Those downloads include the yoshidan releases at 0.15.0 and earlier; the publisher changed to `cloud-sdk-rust-bot` at 0.16.1-preview. It has a `Write` client with `CommittedWriter`, `PendingWriter`, and `BufferedWriter`, plus `finalize()`. `AppendWithOffset` is documented "with an optional stream offset to ensure exactly-once semantics" ([docs.rs](https://docs.rs/google-cloud-bigquery/0.18.0/google_cloud_bigquery/builder/write/struct.AppendWithOffset.html)).
- The generated REST client is `google-cloud-bigquery-v2` 1.0.0 (2026-09-24).
- Community clients:
  - `gcp-bigquery-client` 0.28.0, MIT OR Apache-2.0, about 8M downloads (10^6). Its `StorageApi::append_rows` takes no offset ([docs.rs](https://docs.rs/gcp-bigquery-client/0.28.0/gcp_bigquery_client/storage/struct.StorageApi.html)).
  - `gcloud-bigquery` 1.7.0 (yoshidan), MIT, about 1.7M downloads. It has committed, pending, buffered, and default stream modules, and `AppendRowsRequestBuilder::with_offset`.

Test double:
- No official emulator. The testcontainers `google_cloud_sdk_emulators` module covers Bigtable, Datastore, Firestore, Pub/Sub, and Spanner only (checked against the module source).
- [goccy/bigquery-emulator](https://github.com/goccy/bigquery-emulator) v0.8.1 (2026-06-13), MIT, beta. It supports `insertAll`, the gRPC Storage Read and Write APIs, and DML including MERGE. "Sessions / multi-statement transactions over the REST API" is marked unsupported ([feature matrix](https://github.com/goccy/bigquery-emulator/blob/main/docs/feature-support.md)). Whether it reproduces offset `ALREADY_EXISTS` behaviour is unverified.
- The BigQuery sandbox, which is free, does not support DML or streaming ([sandbox](https://docs.cloud.google.com/bigquery/docs/sandbox)).

## Amazon Redshift

**Class: exactly-once fenced with SQL transactions (inference based on the isolation docs).**

- Redshift has two isolation levels, SNAPSHOT and SERIALIZABLE. SNAPSHOT is now the default for new clusters and workgroups ([isolation](https://docs.aws.amazon.com/redshift/latest/dg/c_serial_isolation.html)).
- SERIALIZABLE lets one transaction commit and cancels the concurrent ones with error 1023. Under SNAPSHOT, the documented example lets updates to different rows both commit (same page). **Inference:** two epochs updating the same fence row conflict and one aborts. Verify this under SNAPSHOT.
- The Data API `BatchExecuteStatement` runs up to 40 SQL statements "as a single transaction" by default (`ExecutionMode=TRANSACTION`). It has a `ClientToken` for request idempotency ([API](https://docs.aws.amazon.com/redshift-data/latest/APIReference/API_BatchExecuteStatement.html)).
- `MERGE` exists, including the `REMOVE DUPLICATES` mode. A target row cannot match several source rows ([MERGE](https://docs.aws.amazon.com/redshift/latest/dg/r_MERGE.html)).
- Unique and primary key constraints are "informational only" ([constraints](https://docs.aws.amazon.com/redshift/latest/dg/t_Defining_constraints.html)), so dedup must use MERGE on the delivery ID.

Rust client:
- `aws-sdk-redshiftdata` 1.121.0 (2026-10-01), official AWS SDK for Rust, Apache-2.0, about 0.27M downloads (10^5).
- PostgreSQL wire through `tokio-postgres` 0.7.18 (10^7) or `sqlx` 0.9.0 (10^8). Compatibility with Redshift is **inference**; AWS does not list Rust drivers.

Test double:
- No official emulator.
- LocalStack Redshift appears in every LocalStack for AWS tier, including Hobby, which is for non-commercial use ([pricing](https://www.localstack.cloud/pricing)). It implements 6 of 12 Data API operations, and the docs point to the Ultimate plan for advanced Data API features ([Redshift](https://docs.localstack.cloud/aws/services/redshift/)). Its SQL backend fidelity is unverified.
- A Postgres container (`postgres` feature of testcontainers-modules) only approximates Redshift (**inference**: dialect and isolation differ).
