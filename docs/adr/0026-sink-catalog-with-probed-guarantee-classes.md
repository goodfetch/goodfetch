# A broad catalog of Sinks, each held to the delivery contract at the guarantee class a probe proves

V1 ships first-party Sinks for about sixty destinations, in seven families:
- **Relational**: PostgreSQL, pgvector, Supabase, MySQL/MariaDB, SQL Server, SQLite.
- **Document and search**: MongoDB, Elasticsearch, OpenSearch, Redis.
- **Warehouses and lakes**: Snowflake, BigQuery, Redshift, ClickHouse, Databricks/Delta, Iceberg, DuckDB/MotherDuck.
- **Vector stores**: Qdrant, Pinecone, Weaviate, Milvus, Chroma, LanceDB.
- **Files and apps**: S3-compatible stores, GCS, Azure Blob, SFTP, FTP/FTPS, Google Drive, Google Sheets, Dropbox, OneDrive/SharePoint, Box, Airtable, Notion.
- **Queues and streams**: Kafka/Redpanda, SQS, SNS, Kinesis, Firehose, Pub/Sub, Service Bus, Event Hubs, RabbitMQ, NATS JetStream, Redis Streams.
- **Email and webhooks**: SMTP, SES, Resend, Postmark, SendGrid, Mailgun and signed webhooks.

Every Sink keeps ADR 0008's contract: a delivery ID and the run epoch on every write, and retries.

A destination's name does not fix what it can guarantee. AWS S3 and R2 have conditional writes while Backblaze B2 and Wasabi document none. MongoDB fences only on a replica set, and SQS dedups only for five minutes. So each Sink is **probed** when it is configured and again at run start, and it earns one of four **guarantee classes**:
- **Fenced**: the epoch check and the whole batch are atomic.
- **Record-fenced**: a stale epoch cannot overwrite a record but can add one.
- **Keyed**: a retry lands on the same key, sometimes only within a dedup window.
- **At-least-once**: the receiver dedups on the delivery ID.

A run can require a minimum class and is refused before it starts if a Sink cannot meet it. No surveyed product fences sink writes with a run epoch; the closest are Kafka transactions and Snowpipe Streaming channels, which fence a producer or channel but not a resumed run. Users' two most frequent complaints are silent failure and duplicates. Decided in [Export connectors and integrations catalog](https://github.com/goodfetch/goodfetch/issues/88).

Evidence: [sinks, notifications and integrations research](https://github.com/goodfetch/goodfetch/blob/research/sinks-integrations-catalog/rnd/research/2026-10-11-sinks-integrations/README.md), including [delivery best practice](https://github.com/goodfetch/goodfetch/blob/research/sinks-integrations-catalog/rnd/research/2026-10-11-sinks-integrations/g-best-practice.md) and [65 coded user pains](https://github.com/goodfetch/goodfetch/blob/research/sinks-integrations-catalog/rnd/research/2026-10-11-sinks-integrations/f-user-pains.md).

## Considered Options

- **A small catalog plus community plugins** (Scrapy, Crawlee): rejected. V1 is complete scope, and the research shows Apify, Bright Data, Spider Cloud and Octoparse each ship destinations the others lack. Database sinks are rare among scrapers, which leaves them open for Goodfetch to cover.
- **Three classes, as ADR 0008 implied**: rejected. Elasticsearch, Qdrant and LanceDB reject a stale record but let a zombie insert new ones, which fits neither fenced nor keyed.
- **The class looked up from the protocol name**: rejected, because "S3-compatible" and "MongoDB" each cover deployments with different guarantees.
- **Every client compiled into the binary** (Vector, OpenTelemetry Collector): rejected. The AWS, Azure and Google SDKs, Kafka's C library and the database drivers would swell the binary and its notice inventory for everyone.
- **WASM components for third-party Sinks**: rejected for V1. No surveyed system uses WASM for output plugins, Vector removed its WASM transform in 2021, and a Postgres or Kafka sink would need `wasi:sockets`, whose Wasmtime defaults are documented inconsistently.
- **DuckDB only as a reader**, as ADR 0008 decided: reversed. DuckDB and MotherDuck are now Sinks, because the owner asked for every destination competitors ship (dlt and Airbyte write to them), and DuckDB is fenced with a fence table. DuckDB still reads run files and Parquet Exports directly, as ADR 0008 documents.
- **Append-only delivery deduplicated by delivery ID alone**: rejected. It stops retries from duplicating but not the same product scraped again, the most frequent duplicate complaint.

## Consequences

**Classes by destination.** Every row is the target class. A Sink claims it only once the conformance suite proves it, and until then reports the class the suite has proved so far. "Lab" marks a row whose fence recipe is an inference or untested in the research, and needs a lab proof before its suite is even written.

| Class | Destinations |
| --- | --- |
| Fenced | PostgreSQL, pgvector, Supabase direct, MySQL/MariaDB, SQL Server, SQLite on a local disk, MongoDB replica sets, DuckDB. Lab: Snowflake, BigQuery, Redshift, MotherDuck, Databricks (multi-statement transactions), Supabase through a fencing function, Kafka/Redpanda (one `transactional.id` kept across a run's epochs, so Kafka's own producer epoch fences the zombie), NATS JetStream (expected sequence), AWS S3, R2 (single-part writes; its multipart completion has no condition), GCS, Azure Blob, Dropbox, OneDrive/SharePoint and Box (their conditional writes are documented; using them as an epoch fence through a fence object is an inference), Iceberg (Goodfetch sends the REST catalog commit itself, because `iceberg-rust` re-asserts against a reloaded snapshot) |
| Record-fenced | Elasticsearch, OpenSearch, Qdrant, LanceDB, standalone MongoDB |
| Keyed | ClickHouse, Pinecone, Weaviate, Milvus, Chroma, SFTP, FTP, Google Drive, Airtable, SQS/SNS FIFO, Service Bus, RabbitMQ Streams, Redis keys and Redis Streams (a crash or failover can lose an acknowledged fence write), S3-compatibles without conditional writes, Resend |
| At-least-once | Webhooks, Google Sheets (a lab test may promote it to Keyed through developer metadata), Notion, SQS/SNS standard, Kinesis, Firehose, Pub/Sub, Event Hubs, RabbitMQ classic queues, SMTP, SES and Postmark |

**Reporting the class.** The class appears in `goodfetch sinks list`, in the run report and in the Console. When a retry falls outside a destination's dedup window, that delivery is reported as at-least-once.

> Amended by [ADR 0137](0137-the-console-leaves-both-products-and-the-engine-and-the-owners-monitoring-tool-is-designed-apart.md): no Product carries a Console. The engine behaviour this ADR describes stands and the Console's part does not; a Client reaches it.

**Two modes.** A Sink takes Records mode (rows or messages, delivered while the run goes), Files mode, or both. In Files mode, Exports roll into numbered parts by count, size or time, and a final part is written at run end. Each part's key carries the delivery ID and the epoch.

**Write modes.** Every Sink that can supports these, keyed on user-declared fields:
- append
- upsert
- insert-only
- replace (written apart, then swapped in atomically)
- history (versions with valid-from and valid-to)

> Amended by [ADR 0103](0103-a-handler-pushes-records-staged-with-its-attempt-into-datasets-its-crawl-declares-and-a-reader-never-slows-the-crawl.md): the user-declared fields are the Sink's own `key`, or else the `key` of each Dataset it receives, which the crawl declares. The run's start refuses with `setting_invalid` a Sink in a keyed mode with no key of its own when any of its Datasets has none, and a Sink key naming a field that a schema of one of its Datasets does not require; a Record of a Dataset with no schema that reaches a keyed Sink without its key fields joins the Sink's dead letters. A Record is identified by its Dataset and its number, so record ranges and the order by record number are per Dataset. Sinks are configured as `sinks.<name>.*` settings on a connection or a crawl's Run, refused on a Label and when set on a Collie Run; a Sink inherited from the connection or from configuration is inactive for a Collie Run. A Collie engine refuses one given to a Run or a connection with `not_in_product` (ADR 0087), and leaves configuration's `sinks.*` to Foxhound.

The last record in a batch wins. "Only new or changed" delivers what change tracking reports against the Baseline. A record missing since the Baseline can be soft-deleted, by a complete run only (ADR 0036). The engine refuses a mode a destination cannot support. On Sheets and Notion, the engine does the lookup itself.

**Schema.** The declared schema comes first; otherwise it is inferred.
- **Columns:** only added. A conflicting value widens the column to its lossless supertype, or goes to a variant column `<field>__v_<type>`.
- **Contracts:** dlt-style, per Sink: `evolve` (the default), `freeze`, `discard_row` or `discard_value`.
- **Files mode:** every part carries the union of all fields seen so far.
- **Names:** normalized to snake_case, with collisions suffixed.
- **Nesting:** nested data keeps the destination's native JSON, VARIANT or STRUCT type, with a `flatten` option.
- **Changes:** each schema change emits an event.
- **System columns:** `_goodfetch_record_number`, `_goodfetch_delivery_id`, `_goodfetch_run_id`, `_goodfetch_run_epoch`, `_goodfetch_committed_at` and `_goodfetch_meta`. Write modes that need them add `_goodfetch_deleted`, `_goodfetch_valid_from` and `_goodfetch_valid_to`.
- **Delivery table:** each destination gets an `_goodfetch_deliveries` table holding the fence row and the delivery log.

**Failure handling.**
- **Collection:** does not stop for a Sink unless the run opts in with `on_sink_failure: stop`. Each Sink keeps its own cursor.
- **Rejected Records:** a rejected batch is split recursively until the bad Records are isolated. They go to a per-Sink dead-letter list with the destination's reason and are retried on later runs. A value the engine had to null or truncate is recorded in `_goodfetch_meta`.
- **Delivery status:** `complete`, `complete with rejections`, `behind` or `failed`. Every failed page also emits an event.
- **Retry schedule:** the Standard Webhooks schedule (immediately, 5 s, 5 min, 30 min, 2 h, 5 h, 10 h, 14 h, 20 h, 24 h).
  - Only 408, 429 and 5xx responses are retried, and `Retry-After` is honoured.
  - A `410` pauses the Sink.
  - After three days of total failure the Sink pauses and sends a Notification, with another one on recovery.
- **After the run:** a Daemon keeps delivering a finished run's backlog. `goodfetch deliver` can redeliver a delivery, recover failures since a date, replay what was never attempted, or backfill any past run to any Sink.

**Throughput.**
- **Rate limiting:** each Sink ships a token bucket preset to its destination's published quotas, such as Sheets' 60 writes per minute per user and Airtable's 5 requests per second with 10 records each.
- **Batching:** batches are sized for each API, such as BigQuery's Storage Write API and Snowflake files of 100 to 250 MB.
- **Concurrency:** adaptive by default, as in Vector.
- **Quota exhaustion:** the Sink is reported as `behind`, not `failed`.
- **Size limits:** a hard size limit is warned at 80%, then spills into numbered tabs or files.

**Logging and metrics.** Every attempt is logged in the run file. The Console shows each Sink's deliveries as stages from received to delivered. Delivery metrics go to OpenTelemetry.

> Amended by [ADR 0137](0137-the-console-leaves-both-products-and-the-engine-and-the-owners-monitoring-tool-is-designed-apart.md): no Product carries a Console, so what this says of the Console ends; the rest stands.

**How Sinks are built.**
- **In the binary:** Export formats, the object stores through `object_store`, SFTP/FTP, PostgreSQL, pgvector, Supabase, MySQL/MariaDB, SQLite, DuckDB, webhooks, SMTP and every Notification channel.
- **As first-party sink processes:** every other Sink. They are written in Rust, speak the public sink protocol (the `sink.*` group of the plugin protocol, ADR 0033) over stdio JSON-RPC, are released at the engine's version, and are cached by digest like browsers (ADR 0024). Where the Rust client is weak, they call the destination's HTTP API: Snowflake's SQL API, Weaviate, Airtable, Box, Notion, Microsoft Graph.

**Third-party Sinks.** These come in three forms:
- an SDK handler function
  > Amended by [ADR 0104](0104-a-crawl-declares-its-hooks-per-stage-in-code-a-hook-returns-its-change-and-a-program-registers-its-own-plugins-by-name.md): a program registers its Sink on its connection by name (`connect({ sinks })`; in Rust, `ConnectOptions::sink(name)` and `gf.sink_requests(name)`), which is a Rust program's form too, a linked engine included. It serves the `sinks.<name>` entry of its name, and its own exception is `sink_handler_failed`, retried on ADR 0008's schedule.
- a sink process (a process Plugin, ADR 0033)
- a Rust trait implementation
  > Amended by [ADR 0104](0104-a-crawl-declares-its-hooks-per-stage-in-code-a-hook-returns-its-change-and-a-program-registers-its-own-plugins-by-name.md): not a form of the SDK API. A Rust program's own Sink is the SDK-handler form, with every engine; the Rust-trait form is the engine's seam for its built-ins.

They share one interface and one conformance suite, and earn the class they pass.

**Bridge Sinks.** Bridges run a user-chosen Airbyte destination image or Singer target, as a container image on a rootless runtime, at ADR 0033's `direct` level, each named by image digest in Operator policy. Goodfetch bundles none of them, because Airbyte connectors moved to ELv2 in July 2025 and several Singer targets are AGPL. The delivery ID is the primary key, and the probe reports the class. dlt is reached through a documented SDK handler.

**Vector stores.** The engine chunks and embeds through a Model provider (ADR 0012).
- Every embedding call goes through the Effect journal and counts against a currency Limit.
- A content-hash cache means a retry never pays twice.
- Chunk IDs are UUIDv5 of the delivery ID and chunk index.
- Provenance goes into chunk metadata.
- A run may supply its own vectors.

**Export formats.** JSON, JSONL, CSV, XLSX, XML and Parquet are joined by HTML table, RSS/Atom, Avro, Arrow IPC and Markdown, with gzip or zstd compression.

> Amended by [ADR 0119](0119-every-fetch-tier-decodes-a-crawls-document-as-the-browser-does-a-label-names-the-content-types-its-handler-takes-and-exports-are-utf-8.md): every text Export format is UTF-8, JSON and JSONL with no BOM and characters written as themselves, and CSV with a BOM only when `export.csv.bom` is on (off by default); values are written as decoded, never normalized or repaired.

**OAuth.** Tokens are Secret references obtained by a loopback `goodfetch auth <provider>` flow, or by service accounts.
- Refreshed and rotated tokens are written back to a writable Secret provider (keychain, encrypted file, vaults). With a read-only provider such as `env:`, they are kept in the run's encrypted state (ADR 0015) and the docs advise a writable provider for rotating tokens.
- The run-start probe checks every token and sends an `auth expired` Notification before collection.
- Bring-your-own OAuth clients come first, and the docs warn that Google expires refresh tokens after 7 days while an app is in testing.
- Goodfetch may also register its own Google app as an option. If it does, the app must be verified for production as a launch task and use the narrowest scopes, such as Drive's `drive.file`.

**Webhook payloads.** Webhook Sinks send record batches capped at 256 KB by default. A thin shape instead carries a record range and a fetch URL, and is the default for workflow-tool Integrations.

**Conformance.** One sink conformance suite covers:
- a crash in mid-delivery
- a retry whose write had already landed
- two run copies with different epochs
- order by record number
- an interrupted part file
- a schema change in mid-run

It runs on every pull request against emulators and containers: LocalStack or RustFS rather than the archived MinIO, Azurite, fake-gcs, Kafka, MongoDB, Elasticsearch. It runs nightly against real accounts, which the owner approves case by case, for Snowflake, BigQuery, Sheets, Drive, Pinecone and the other SaaS destinations.
