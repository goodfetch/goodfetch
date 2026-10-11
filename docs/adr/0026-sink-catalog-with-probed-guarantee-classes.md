# A broad catalog of Sinks, each held to the delivery contract at the guarantee class a probe proves

V1 ships about sixty first-party Sinks in seven families:
- **Relational**: PostgreSQL, pgvector, Supabase, MySQL/MariaDB, SQL Server, SQLite.
- **Document and search**: MongoDB, Elasticsearch, OpenSearch, Redis.
- **Warehouses and lakes**: Snowflake, BigQuery, Redshift, ClickHouse, Databricks/Delta, Iceberg, DuckDB/MotherDuck.
- **Vector stores**: Qdrant, Pinecone, Weaviate, Milvus, Chroma, LanceDB.
- **Files and apps**: S3-compatible stores, GCS, Azure Blob, SFTP, FTP/FTPS, Google Drive, Google Sheets, Dropbox, OneDrive/SharePoint, Box, Airtable, Notion.
- **Queues and streams**: Kafka/Redpanda, SQS, SNS, Kinesis, Firehose, Pub/Sub, Service Bus, Event Hubs, RabbitMQ, NATS JetStream, Redis Streams.
- **Email and webhooks.**

Every Sink keeps ADR 0008's contract: a delivery ID and the run epoch on every write, and retries.

A destination's name does not fix what it can guarantee. AWS S3 and R2 have conditional writes while Backblaze B2 and Wasabi document none. MongoDB fences only on a replica set, and SQS dedups only for five minutes. So each Sink is **probed** when it is configured and again at run start, and it earns one of four **guarantee classes**:
- **Fenced**: the epoch check and the whole batch are atomic.
- **Record-fenced**: a stale epoch cannot overwrite a record but can add one.
- **Keyed**: a retry lands on the same key, sometimes only within a dedup window.
- **At-least-once**: the receiver dedups on the delivery ID.

A run can require a minimum class and is refused before it starts if a Sink cannot meet it. No surveyed product fences sink writes at all, and users' two most frequent complaints are silent failure and duplicates. Decided in [Export connectors and integrations catalog](https://github.com/Jbmanllr/automan/issues/88).

Evidence: [sinks, notifications and integrations research](https://github.com/Jbmanllr/automan/blob/research/sinks-integrations-catalog/rnd/research/2026-10-11-sinks-integrations/README.md), including [delivery best practice](https://github.com/Jbmanllr/automan/blob/research/sinks-integrations-catalog/rnd/research/2026-10-11-sinks-integrations/g-best-practice.md) and [65 coded user pains](https://github.com/Jbmanllr/automan/blob/research/sinks-integrations-catalog/rnd/research/2026-10-11-sinks-integrations/f-user-pains.md).

## Considered Options

- **A small catalog plus community plugins** (Scrapy, Crawlee): rejected. V1 is complete scope, and the research shows Apify, Bright Data, Spider Cloud and Octoparse each ship destinations the others lack. Database sinks are rare among scrapers, which leaves them open for Automan to cover.
- **Three classes, as ADR 0008 implied**: rejected. Elasticsearch, Qdrant and LanceDB reject a stale record but let a zombie insert new ones, which fits neither fenced nor keyed.
- **The class looked up from the protocol name**: rejected, because "S3-compatible" and "MongoDB" each cover deployments with different guarantees.
- **Every client compiled into the binary** (Vector, OpenTelemetry Collector): rejected. The AWS, Azure and Google SDKs, Kafka's C library and the database drivers would swell the binary and its notice inventory for everyone.
- **WASM components for third-party Sinks**: rejected for V1. No surveyed system uses WASM for output plugins, Vector removed its WASM transform in 2021, and a Postgres or Kafka sink would need `wasi:sockets`, whose Wasmtime defaults are documented inconsistently.
- **Append-only delivery deduplicated by delivery ID alone**: rejected. It stops retries from duplicating but not the same product scraped again, the most frequent duplicate complaint.

## Consequences

**Classes by destination.** Each class is claimed only once the conformance suite proves it. Until then, a destination marked "lab" is held at the next class down.

| Class | Destinations |
| --- | --- |
| Fenced | PostgreSQL, pgvector, Supabase (direct or through a fencing function), MySQL/MariaDB, SQL Server, SQLite on a local disk, MongoDB replica sets, DuckDB, Databricks (multi-statement transactions), Kafka/Redpanda (a `transactional.id` per run), NATS JetStream (expected sequence), Redis Streams (a script), AWS S3, R2, GCS, Azure Blob, Dropbox, OneDrive/SharePoint, Box. Lab: Snowflake, BigQuery, Redshift, MotherDuck, Iceberg (Automan sends the REST catalog commit itself, because `iceberg-rust` re-asserts against a reloaded snapshot) |
| Record-fenced | Elasticsearch, OpenSearch, Qdrant, LanceDB, standalone MongoDB |
| Keyed | ClickHouse, Pinecone, Weaviate, Milvus, Chroma, SFTP, FTP, Google Drive, Airtable, SQS/SNS FIFO, Service Bus, RabbitMQ Streams, Redis keys (a failover can lose a fence write), S3-compatibles without conditional writes, Resend |
| At-least-once | Webhooks, Google Sheets (lab: developer metadata may make it Keyed), Notion, SQS/SNS standard, Kinesis, Firehose, Pub/Sub, Event Hubs, RabbitMQ classic queues, SMTP, SES and Postmark |

**Reporting the class.** The class appears in `automan sinks list`, in the run report and in the Console. When a retry falls outside a destination's dedup window, that delivery is reported as at-least-once.

**Two modes.** A Sink takes Records mode (rows or messages, delivered while the run goes), Files mode, or both. In Files mode, Exports roll into numbered parts by count, size or time, and a final part is written at run end. Each part's key carries the delivery ID and the epoch.

**Write modes.** Every Sink that can supports these, keyed on user-declared fields:
- append
- upsert
- insert-only
- replace (written apart, then swapped in atomically)
- history (versions with valid-from and valid-to)

The last record in a batch wins. "Only new or changed" delivers what change tracking reports against the Baseline. A record missing since the Baseline can be soft-deleted. The engine refuses a mode a destination cannot support. On Sheets and Notion, the engine does the lookup itself.

**Schema.** The declared schema comes first; otherwise it is inferred.
- **Columns:** only added. A conflicting value widens the column to its lossless supertype, or goes to a variant column `<field>__v_<type>`.
- **Contracts:** dlt-style, per Sink: `evolve` (the default), `freeze`, `discard_row` or `discard_value`.
- **Files mode:** every part carries the union of all fields seen so far.
- **Names:** normalized to snake_case, with collisions suffixed.
- **Changes:** each schema change emits an event.
- **System columns:** `_automan_record_number`, `_automan_delivery_id`, `_automan_run_id`, `_automan_run_epoch`, `_automan_committed_at` and `_automan_meta`. Write modes that need them add `_automan_deleted`, `_automan_valid_from` and `_automan_valid_to`.
- **Delivery table:** each destination gets an `_automan_deliveries` table holding the fence row and the delivery log.

**Failure handling.**
- **Collection:** never stops for a Sink. Each Sink keeps its own cursor.
- **Rejected rows:** a rejected batch is split recursively until the bad rows are isolated. Those rows go to a per-Sink dead-letter list with the destination's reason and are retried on later runs. A value the engine had to null or truncate is recorded in `_automan_meta`.
- **Delivery status:** `complete`, `complete with rejections`, `behind` or `failed`. Every failed page also emits an event.
- **Retry schedule:** the Standard Webhooks schedule (immediately, 5 s, 5 min, 30 min, 2 h, 5 h, 10 h, 14 h, 20 h, 24 h).
  - Only 408, 429 and 5xx responses are retried, and `Retry-After` is honoured.
  - A `410` pauses the Sink.
  - After three days of total failure the Sink pauses and sends a Notification, with another one on recovery.
- **After the run:** a Daemon keeps delivering a finished run's backlog. `automan deliver` can redeliver a delivery, recover failures since a date, replay what was never attempted, or backfill any past run to any Sink.

**Throughput.**
- **Rate limiting:** each Sink ships a token bucket preset to its destination's published quotas, such as Sheets' 60 writes per minute per user and Airtable's 5 requests per second with 10 records each.
- **Batching:** batches are sized for each API, such as BigQuery's Storage Write API and Snowflake files of 100 to 250 MB.
- **Concurrency:** adaptive by default, as in Vector.
- **Quota exhaustion:** the Sink is reported as `behind`, not `failed`.
- **Size limits:** a hard size limit is warned at 80%, then spills into numbered tabs or files.

**Logging and metrics.** Every attempt is logged in the run file. The Console shows each Sink as a pipeline from received to delivered. Delivery metrics go to OpenTelemetry.

**How Sinks are built.**
- **In the binary:** Export formats, the object stores through `object_store`, SFTP/FTP, PostgreSQL, pgvector, Supabase, MySQL/MariaDB, SQLite, DuckDB, webhooks, SMTP and every Notification channel.
- **As first-party sink processes:** every other Sink. They are written in Rust, speak the public sink protocol over stdio JSON-RPC, are released at the engine's version, and are cached by digest like browsers (ADR 0024). Where the Rust client is weak, they call the destination's HTTP API: Snowflake's SQL API, Weaviate, Airtable, Box, Notion, Microsoft Graph.

**Third-party Sinks.** These come in three forms:
- an SDK handler function
- a sink process
- a Rust trait implementation

They share one interface and one conformance suite, and earn the class they pass.

**Bridge Sinks.** Bridges run a user-chosen Airbyte destination image or Singer target. Automan bundles none of them, because Airbyte connectors moved to ELv2 in July 2025 and several Singer targets are AGPL. The delivery ID is the primary key, and the probe reports the class. dlt is reached through a documented SDK handler.

**Vector stores.** The engine chunks and embeds through a Model provider (ADR 0012).
- Every embedding call goes through the Effect journal and counts against money Limits.
- A content-hash cache means a retry never pays twice.
- Chunk IDs are UUIDv5 of the delivery ID and chunk index.
- Provenance goes into chunk metadata.
- A run may supply its own vectors.

**Export formats.** JSON, JSONL, CSV, XLSX, XML and Parquet are joined by HTML table, RSS/Atom, Avro, Arrow IPC and Markdown, with gzip or zstd compression.

**OAuth.** Tokens are Secret references obtained by a loopback `automan auth <provider>` flow, or by service accounts.
- Refreshed and rotated tokens are written back to their Secret provider.
- The run-start probe checks every token and sends an `auth expired` Notification before collection.
- Bring-your-own OAuth clients come first, and the docs warn that Google expires refresh tokens after 7 days while an app is in testing.
- Automan's own Google app must be verified for production as a launch task. It uses the narrowest scopes, such as Drive's `drive.file`.

**Webhook payloads.** Webhook Sinks send record batches capped at 256 KB by default. A thin shape instead carries a record range and a fetch URL, and is the default for workflow-tool Integrations.

**Conformance.** One sink conformance suite covers:
- a crash in mid-delivery
- a retry whose write had already landed
- two run copies with different epochs
- order by record number
- an interrupted part file
- a schema change in mid-run

It runs on every pull request against emulators and containers: LocalStack or RustFS rather than the archived MinIO, Azurite, fake-gcs, Kafka, MongoDB, Elasticsearch. It runs nightly against real accounts, which the owner approves case by case, for Snowflake, BigQuery, Sheets, Drive, Pinecone and the other SaaS destinations.
