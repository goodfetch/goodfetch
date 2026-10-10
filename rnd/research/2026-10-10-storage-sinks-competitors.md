# Result storage, export formats and sinks: what competitors actually adopted

Survey of October 10, 2026. It asks which storage abstractions, export formats, live sinks and delivery guarantees competitors ship, and whether any of them fences or deduplicates sink writes. The comparison point is [ADR 0003](../../docs/adr/0003-two-level-fencing-at-least-once-delivery.md): every write leaving the engine carries a run epoch that the destination enforces with a compare-and-set, and outputs become visible exactly once. [ADR 0002](../../docs/adr/0002-portable-run-state-resume-from-checkpoints.md) keeps run state in a per-run SQLite file and leaves PostgreSQL open as a sink for results.

Sources are first-party: source code at a release tag, official documentation and API references. Where a page was read without a version, the reading date (2026-10-10) is the reference. Where no source states a fact and the conclusion comes from reading the code, the text marks it **inference**. Exa had no credits during this survey, so pages were fetched directly.

## Versions checked

| Product | Version | Licence | Source |
| --- | --- | --- | --- |
| Crawlee JS | 3.18.2 (2026-09-29); 4.0.0-rc.1 (2026-10-06) | Apache-2.0 | npm `crawlee`, tags [v3.18.2](https://github.com/apify/crawlee/tree/v3.18.2), [v4.0.0-rc.1](https://github.com/apify/crawlee/tree/v4.0.0-rc.1) |
| Crawlee Python | 1.10.4 (2026-10-06) | Apache-2.0 | PyPI `crawlee`, tag [v1.10.4](https://github.com/apify/crawlee-python/tree/v1.10.4) |
| Apify SDK | Python 4.1.0, JS 3.7.2 | Apache-2.0 (needs platform API) | [apify-sdk-python v4.1.0](https://github.com/apify/apify-sdk-python/tree/v4.1.0) |
| Scrapy | 2.19.0 (2026-09-10), not 2.13.x | BSD | tag [2.19.0](https://github.com/scrapy/scrapy/tree/2.19.0) |
| spider-rs `spider` | 2.53.9 on crates.io (2026-09-05); newest GitHub tag v2.52.2; `main` says 2.54.0, unpublished | MIT | [crates.io](https://crates.io/crates/spider), source read on `main` |
| Firecrawl | v2.11.504 (commit 29b67da, 2026-10-10) | open source + paid cloud | tag v2.11.504 |
| Crawl4AI | 0.9.4 (commit 133e1d9, 2026-09-23) | Apache-2.0 | tag v0.9.4 |
| Stagehand | 4.2.0 (2026-10-08); the v3 line ends at 3.7.3 | MIT | npm `@browserbasehq/stagehand` |

The spider-rs and Stagehand findings were read on `main`, not on a release tag. They may differ slightly from 2.53.9 and 4.2.0.

## Summary table

| Product | Storage abstractions | Export formats | Live sinks in the open-source framework | Sinks only in the paid cloud | Delivery guarantee | Dedup / idempotency on output |
| --- | --- | --- | --- | --- | --- | --- |
| Crawlee JS 3.18 | Dataset, KeyValueStore, RequestQueue on a `StorageClient` (only implementation: MemoryStorage, mirrored to `./storage`) | JSON, CSV | None | Apify platform storage | Immediate `pushData`: a handler that throws after pushing duplicates on retry | None |
| Crawlee JS 4.0-rc | Same three, on a `StorageBackend` (Memory, FileSystem) | JSON, CSV | None (no SQL, no Redis) | Apify platform storage | Per-request transaction; commit is explicitly at-least-once | Rollback on handler failure; RequestQueue `uniqueKey` only |
| Crawlee Python 1.10 | Dataset, KeyValueStore, RequestQueue on `StorageClient`: Memory, FileSystem, **SQL (SQLite, PostgreSQL, MySQL, MariaDB)**, **Redis** | JSON, CSV | The SQL and Redis storage clients | ApifyStorageClient | Handler helpers buffered and replayed after success; replay is not atomic (at-least-once, inference) | None; dataset rows get an autoincrement id |
| Apify platform | Dataset (append-only), KeyValueStore, RequestQueue | json, jsonl, csv, html, xlsx, xml, rss | — | Webhooks; Google Drive, Snowflake native app, S3 via a first-party Actor; Zapier/Make/n8n/Airbyte/Keboola; vector DBs | Webhooks at-least-once (stated) | `X-Apify-Webhook-Dispatch-Id` for webhook dedup; nothing on dataset items |
| Scrapy 2.19 | Item pipelines + feed exports | json, jsonlines, csv, xml, marshal, pickle | Local file, FTP/FTPS, S3, GCS, stdout; Azure/SFTP/Dropbox/Drive/Sheets as plugins | Scrapy Cloud item storage | Remote feeds uploaded at close or per batch, no retry; a crash loses the open batch (inference) | Docs' in-memory `DuplicatesPipeline` example only |
| spider-rs 2.53 | Broadcast channel of `Page`, `get_pages()` buffer, SQLite `disk` feature, caches, WARC writer | CLI: JSON, JSONL, raw, markdown, commonmark, text, xml, WARC 1.1 | None | Spider Cloud data connectors: S3, GCS, Azure Blob, Google Sheets, Supabase; webhooks | Slow subscriber loses pages (`Lagged`) | None |
| Firecrawl | Jobs in Postgres/Redis (self-host); no result store | markdown, html, rawHtml, links, screenshot, json, summary, changeTracking… | Webhooks (single attempt in OSS) | Webhook retries; Zapier/Make/n8n | Cloud webhooks at-least-once (1, 5, 15 min retries) | `webhookId` per delivery; `x-idempotency-key` on job **submission** |
| Crawl4AI 0.9 | `CrawlResult`; SQLite cache keyed by URL | markdown, html variants, extracted JSON, screenshot, pdf, mhtml; NDJSON stream | Docker server webhooks (5 tries) | — (no cloud) | At-least-once (inference: timeout then retry) | None; no signature |
| Stagehand 4 / Browserbase | None; `extract()` returns typed JSON | JSON object | None | Downloads, contexts, recordings; S3 only for Enterprise BYOS; webhooks for Functions only | Webhooks at-least-once (stated) | Event `id` stable across retries |
| Bright Data | Snapshots (30 days) | ndjson, json, jsonl, csv, xlsx, parquet | — (closed) | S3, GCS, Pub/Sub, Azure, Snowflake, SFTP, Alibaba OSS, webhook, email | Webhook retried on non-200 within 30 s; count undocumented | Scraper Studio: "not idempotent"; inputs deduped within a job only |
| Zyte | Scrapy Cloud items and collections | json, jl, xml, csv (Scrapy Cloud); JSON, JSONL, CSV, Parquet (Zyte Data) | Scrapy feed exports | Zyte Data: S3, GCS, Azure Blob, webhook, API pull | Schema check before release; no dedup stated | None found |

**No product fences sink writes with an epoch or fencing token, and none promises exactly-once delivery of results.** The closest mechanisms are Crawlee JS v4's transaction (rollback of a failed attempt, at-least-once commit), dedup ids on webhook deliveries (Apify, Browserbase, Firecrawl), and Firecrawl's idempotency key on job submission.

## Crawlee JS

### Abstractions

- v3: `Dataset`, `KeyValueStore`, `RequestQueue` (V2; `RequestQueueV1` and `RequestProvider` still exported) over the `StorageClient` interface. The only implementation is `MemoryStorage` from `@crawlee/memory-storage`, which mirrors to `./storage` unless `persistStorage: false`. [memory-storage.ts @ v3.18.2](https://github.com/apify/crawlee/blob/v3.18.2/packages/memory-storage/src/memory-storage.ts)
- v4 renames `StorageClient` to `StorageBackend`: 4 classes instead of 7, with async `create*Backend` factories. `MemoryStorage` is split in two:
  - `MemoryStorageBackend`, in `@crawlee/core`: memory only, no lock expiry.
  - `FileSystemStorageBackend`, in the new `@crawlee/fs-storage` with a native `@crawlee/fs-storage-native`: always persists.
- Other v4 changes: `@crawlee/memory-storage` is removed, request queues V1 and V2 are merged, and `purge()` is new. [upgrading_v4.md @ v4.0.0-rc.1](https://github.com/apify/crawlee/blob/v4.0.0-rc.1/docs/upgrading/upgrading_v4.md)
- JS has no SQL or Redis backend in either version.

### Export formats

JSON and CSV only. `Dataset.exportTo`, `exportToJSON` and `exportToCSV` write one record into a key-value store, and `crawler.exportData(path, 'json'|'csv')` writes a file. [dataset.ts @ v4.0.0-rc.1](https://github.com/apify/crawlee/blob/v4.0.0-rc.1/packages/core/src/storages/dataset.ts)

### Delivery semantics

- v3: `context.pushData` writes immediately (basic-crawler.ts around line 1331 at v3.18.2). A handler that pushes, then throws and is retried, produces duplicate items. Only `AdaptivePlaywrightCrawler` buffers through its `RequestHandlerResult` (crawler_commons.ts around line 175).
- v4 adds **transactional storage access**: PR [apify/crawlee#3953](https://github.com/apify/crawlee/pull/3953) "feat!: Transactional storage access", merged 2026-08-06, with follow-ups #4110 (`afterCommit` helper) and #4093. Code: [transaction.ts @ v4.0.0-rc.1](https://github.com/apify/crawlee/blob/v4.0.0-rc.1/packages/core/src/storages/transaction.ts); guide: `docs/guides/result_storage.mdx`, "Transactional storage".
  - Each request runs in a transaction carried by `AsyncLocalStorage`. Dataset and key-value-store writes are always deferred and cloned with `structuredClone`.
  - Request-queue additions default to write-through: they are not rolled back, and `uniqueKey` dedups them. `transactionalStorage: { requestQueue: 'deferred' }` defers them too.
  - `commit()` runs before `markRequestAsHandled` (basic-crawler.ts around line 2621), in the order key-value store, request queue, dataset. The commit has a 300 s timeout and is not retried automatically.
  - A handler that throws is rolled back, so a retried request does not duplicate its items.
  - The commit itself is explicitly at-least-once: if part of it fails and the request is retried, writes can be applied again.
  - Outside the transaction: `useState`, error and failed handlers, and deferred cleanups. `transactionalStorage: false` opts out, and `withDirectStorageAccess()` bypasses it.

### Locks and fencing

- v3 locks requests with `listAndLockHead`, `prolongRequestLock` and `deleteRequestLock` under a `clientKey`. MemoryStorage encodes the lock expiry inside `orderNo`.
- v4 moves locking inside the backend (`fetchNextRequest`, `markRequestAsHandled`, `reclaimRequest`, `extendRequestProcessingTimeSecs`). The default lock lasts 3 minutes, and `clientKey` is removed (issue #3328).
- Neither version has an epoch or fencing token, so an expired lock does not stop a late commit (as ADR 0003 already noted).

## Crawlee Python

### Abstractions

`StorageClient` implementations, listed in [storage_clients/__init__.py @ v1.10.4](https://github.com/apify/crawlee-python/blob/v1.10.4/src/crawlee/storage_clients/__init__.py):

| Client | Since | Backend |
| --- | --- | --- |
| `MemoryStorageClient` | 1.0.0 | Memory |
| `FileSystemStorageClient` | 1.0.0 (#1194) | Files; writes use `atomic_write` |
| `SqlStorageClient` | 1.0.0 (#1339) | SQLAlchemy 2. Dialects: SQLite (default file `crawlee.db`), PostgreSQL, MySQL and MariaDB. MySQL and MariaDB were added in 1.4.0 (#1749); the version that first added PostgreSQL was not confirmed. |
| `RedisStorageClient` | 1.1.0 (#1406) | Redis 8+ |

The Apify platform clients live in the Apify SDK: `ApifyStorageClient`, `SmartApifyStorageClient` and `ApifyFileSystemStorageClient` ([apify-sdk-python v4.1.0](https://github.com/apify/apify-sdk-python/blob/v4.1.0/src/apify/storage_clients/__init__.py)).

### Export formats

JSON and CSV. `Dataset.export_to(content_type='json'|'csv')`, with `collect_all_keys` since 1.9.0. `BasicCrawler.export_data(path)` picks the format from the file extension. [_dataset.py @ v1.10.4, L300-365](https://github.com/apify/crawlee-python/blob/v1.10.4/src/crawlee/storages/_dataset.py#L300-L365)

### Delivery semantics

- `RequestHandlerRunResult` (`_types.py` around line 264) buffers the context helpers `push_data`, `add_requests` and `get_key_value_store`.
- After the handler succeeds, `_commit_request_handler_result` replays the buffered calls (requests, then data, then key-value store) and only then marks the request handled (`_basic_crawler.py` L1419-1440 and around L1538). A failed handler's buffer is discarded.
- A direct `Dataset.open().push_data()` is not buffered.
- **Inference:** the replay is sequential and not atomic, so it is at-least-once. The docs do not state the guarantee.
- No item dedup or idempotency key. SQL dataset rows take an autoincrement `item_id`.

### Locks and crash persistence

- SQL request queue: rows carry `is_handled`, `time_blocked_until` (now + 300 s), `client_key` (32 characters) and `sequence_number`. PostgreSQL, MySQL and MariaDB fetch with `FOR UPDATE SKIP LOCKED`; SQLite uses a conditional `UPDATE … RETURNING`. [_request_queue_client.py @ v1.10.4, L420-500](https://github.com/apify/crawlee-python/blob/v1.10.4/src/crawlee/storage_clients/_sql/_request_queue_client.py#L420-L500)
- Redis request queue: Lua scripts (`atomic_fetch_request`, `reclaim_stale_requests`), a 300 s block, and dedup through a Bloom filter or a set.
- These are leases only (client key plus expiry), with no epoch.
- What survives a crash:
  - The request queue, in the FileSystem, SQL and Redis backends. The FileSystem state sits under the key-value-store key `__RQ_STATE_{id}`.
  - Statistics, under `__CRAWLER_STATISTICS_{id}` in Python, `SDK_CRAWLER_STATISTICS_{id}` in JS v3 and `CRAWLEE_CRAWLER_STATISTICS_{id}` in JS v4.
  - Session-pool state.
- `purge_on_start` wipes the default storages.

## Apify platform

### Dataset export

- `GET /v2/datasets/{id}/items?format=` accepts `json` (the default), `jsonl`, `csv`, `html`, `xlsx`, `xml` and `rss`. [dataset-items-get](https://docs.apify.com/api/v2/dataset-items-get)
- CSV, XLSX and HTML are capped at 2,000 columns and 200-character column names.
- Other parameters: `clean`, `fields`, `unwind`, `offset`, `limit` and `desc`.
- Items come back in insertion order, so a reader can page through them while the run is still writing.
- Datasets are append-only: items cannot be modified or deleted. Unnamed datasets expire after 7 days. [Dataset docs](https://docs.apify.com/platform/storage/dataset)
- Pushing items accepts up to 5 MB per request and documents no idempotency parameter. [dataset-items-post](https://docs.apify.com/api/v2/dataset-items-post)

### Integrations

The [integrations catalog](https://docs.apify.com/integrations) ([llms.txt](https://docs.apify.com/llms.txt)) has first-party pages for:

| Category | Integrations |
| --- | --- |
| Automation | Zapier, Make, n8n, Pipedream, IFTTT, Power Automate, Workato, Kestra, Windmill, Activepieces, Albato, viaSocket, Gumloop |
| Data platforms | Airbyte, Keboola, Snowflake, Airtable |
| Apps and messaging | Google Drive, HubSpot, Slack, Gmail, GitHub, Telegram, Bubble, Lovable, Lindy |
| AI frameworks | LangChain/LangGraph, LlamaIndex, Haystack, CrewAI, Mastra, Agno, Dify, Flowise, Langflow, OpenAI Agents, Vercel AI SDK, Google ADK, AWS Bedrock, Strands, MCP |
| Vector databases | Pinecone, Qdrant, Milvus |
| Generic | Webhooks |

- Google Drive is a native integration ("Upload results to GDrive"). [drive](https://docs.apify.com/integrations/drive)
- Snowflake is a Snowflake Native App with a Streamlit interface. [snowflake](https://docs.apify.com/integrations/snowflake)
- S3 has no docs page. It is served by the first-party Store Actor [`apify/s3-uploader`](https://api.apify.com/v2/acts/apify~s3-uploader).
- **No first-party sink exists for PostgreSQL, MySQL, MongoDB, BigQuery or Elasticsearch.** The only database sink found is the community Actor `nerolabs/dataset-to-database` (Postgres, Supabase, MySQL; [store search](https://api.apify.com/v2/store?search=postgres)).

### Webhooks

- Events: `ACTOR.RUN.{CREATED,SUCCEEDED,FAILED,ABORTED,TIMED_OUT,RESURRECTED}` and `ACTOR.BUILD.*`. [events](https://docs.apify.com/integrations/webhooks/events)
- The payload carries run identifiers; the consumer fetches the items itself.
- Any non-2XX response is retried with exponential back-off (about 1 minute up to about 32 hours), 11 retries at most, with a 2-minute request timeout. [actions](https://docs.apify.com/integrations/webhooks/actions)
- The docs state at-least-once: "In rare cases, the webhook might be invoked more than once. Design your code to be idempotent." The `X-Apify-Webhook-Dispatch-Id` header can serve as a dedup key.
- An ad-hoc webhook's `idempotencyKey` stops a restarted Actor from **creating** the webhook twice. It does not dedup deliveries. [ad-hoc webhooks](https://docs.apify.com/integrations/webhooks/ad-hoc-webhooks)

### Restart and migration

- On migration, the run's default dataset, key-value store and request queue are kept, in-memory state is lost and the run restarts.
- The guidance is to save state with `Actor.useState`, handle the `migrating` event, or call `Actor.reboot()`. [state persistence](https://docs.apify.com/platform/actors/development/builds-and-runs/state-persistence)
- **Inference:** an append-only dataset with no idempotency key on `pushData` can gain duplicate items when work is redone after a restart. No platform guidance on deduplicating items was found.

## Scrapy 2.19.0

Sources at tag 2.19.0: [feedexport.py](https://github.com/scrapy/scrapy/blob/2.19.0/scrapy/extensions/feedexport.py), [default_settings.py](https://github.com/scrapy/scrapy/blob/2.19.0/scrapy/settings/default_settings.py), [feed-exports.rst](https://github.com/scrapy/scrapy/blob/2.19.0/docs/topics/feed-exports.rst), [item-pipeline.rst](https://github.com/scrapy/scrapy/blob/2.19.0/docs/topics/item-pipeline.rst), [jobs.rst](https://github.com/scrapy/scrapy/blob/2.19.0/docs/topics/jobs.rst).

### Feed exports

| Aspect | What 2.19.0 ships |
| --- | --- |
| Formats (`FEED_EXPORTERS_BASE`) | json, jsonlines / jsonl / jl (the default `FEED_FORMAT` is jsonlines), csv, xml, marshal, pickle |
| Storage (`FEED_STORAGES_BASE`) | local file, ftp, ftps (new in 2.18.0), gs, s3, stdout |
| Plugins only | Azure, SFTP, Dropbox, Google Drive, Google Sheets and OneDrive, as `scrapy-plugins/scrapy-feedexporter-*` repositories, last pushed between 2023 and 2025 |
| Batching | `FEED_EXPORT_BATCH_ITEM_COUNT` (default 0) or a per-feed `batch_item_count`; the URI must then contain `%(batch_id)d` or `%(batch_time)s`. A setting named `FEED_STORAGE_BATCH_ITEM_COUNT` does not exist. |
| Overwrite | Local file: False, so the file is opened in append mode. S3/GCS: True, since append is unsupported. FTP: True. |
| Empty feeds | `FEED_STORE_EMPTY` defaults to True |
| Post-processing | Gzip, LZMA and Bz2 plugins |

### Delivery

- S3, GCS and FTP storage (`BlockingFeedStorage`) write to a `NamedTemporaryFile` and upload it only when a slot closes: at `spider_closed`, or when a batch is full.
- A failed upload is only logged and counted in the `feedexport/failed_count/<Storage>` stat. It is not retried.
- **Inference:** a hard crash loses the batch not yet uploaded, because the temporary file is deleted automatically. The local file storage writes straight to the target, so a crash leaves a partial file, for example truncated JSON.
- `JOBDIR` persists the request queue, the `requests.seen` request dupefilter and `spider.state`. A job can resume only after a clean stop, and only on the same Scrapy version.
- **Inference:** items from requests in flight at the pause can be emitted again on resume, so a local feed in append mode gets duplicates.
- The docs' item pipelines show two examples, neither of them idempotent:
  - `DuplicatesPipeline`: an in-memory `ids_seen` set, not persisted.
  - `MongoPipeline`: a plain `insert_one`, not an upsert.

## spider-rs and Spider Cloud

### Open-source crate (MIT)

- `Website::subscribe(cap)` returns a tokio `broadcast::Receiver<Page>`.
  - A slow receiver gets `Lagged` and loses pages.
  - `subscribe_guard()` returns a `ChannelGuard`, and the crawl hangs unless the receiver calls `inc()` for each page.
  - [website.rs on main](https://github.com/spider-rs/spider/blob/main/spider/src/website.rs), around L14696, L7974 and L14832.
- `get_pages()` is only filled by the `scrape*` entry points. The buffer is unbounded unless `SPIDER_MAX_RETAINED_PAGES` caps it.
- Storage-related features, all present in 2.53.9:
  - `disk`, `disk_native_tls`, `disk_aws`: SQLite via sqlx, described as hybrid disk storage.
  - HTTP caches: `cache` (on-disk cacache), `cache_mem`, `cache_chrome_hybrid`, `etag_cache`.
  - `chrome_screenshot`: writes to `./storage/`.
  - `warc`: `WarcWriter`, WARC 1.1.
  - `agent_skills_s3` stores agent skills in S3; it is not a results sink.
- No database, object-store, queue or webhook sink. `with_retry` and `with_retry_strategy` retry fetches, not deliveries.
- `spider_cli`:
  - `crawl -o` writes JSON and `scrape` writes JSONL.
  - `--return-format` takes raw, markdown, commonmark, text or xml.
  - `-W` writes a WARC 1.1 file.

### Spider Cloud (paid)

- `return_format` values: raw, markdown, commonmark, text, html2text, xml, bytes, screenshot, empty. There is no `html` value, and unknown values silently fall back to raw.
- A request content-type of `application/xml`, `text/csv` or `application/jsonl` changes how the response is encoded. ([llms-full.txt](https://spider.cloud/llms-full.txt))
- [Webhooks](https://spider.cloud/docs/core/webhooks):
  - Event flags: `on_find`, `on_find_metadata`, `on_website_status`, `on_credits_depleted`, `on_credits_half_depleted`.
  - Signature: `X-Spider-Signature: t=…,v1=…`, an HMAC-SHA256 of `ts.body` keyed with the API key.
  - Deliveries are "batched and asynchronous". No retry policy or idempotency key is documented.
- [Data connectors](https://spider.cloud/docs/core/data-connectors):
  - Targets: S3, GCS, Google Sheets, Azure Blob, and Supabase (a PostgREST insert).
  - S3 receives one JSON object per page, keyed by domain and timestamp.
  - Connectors run on `on_find` and in parallel with webhooks.
  - No delivery guarantee is documented.

## Firecrawl

- Formats ([scrape](https://docs.firecrawl.dev/features/scrape.md)):
  - Page content: markdown, summary, html, rawHtml, rawBase64, links.
  - Media: screenshot (URL expires after 24 h); images, audio and video (signed URLs, expire after 1 h).
  - Structured output: json, branding, product, query.
  - changeTracking: its snapshots persist per team and tag and never expire ([change tracking](https://docs.firecrawl.dev/features/change-tracking.md)).
- Webhooks ([overview](https://docs.firecrawl.dev/webhooks/overview.md), [events](https://docs.firecrawl.dev/webhooks/events.md), [security](https://docs.firecrawl.dev/webhooks/security.md)):
  - Events: `crawl.{started,page,completed}`, `batch_scrape.*`, `extract.*`, `agent.*`, `monitor.*`.
  - `X-Firecrawl-Signature: sha256=<HMAC of the raw body>`, with no timestamp.
  - The endpoint must return 2xx within 10 s. Failures are retried after 1, 5 and 15 minutes, then marked failed: at-least-once.
  - The source adds a `webhookId: randomUUID()` per delivery, which works as a dedup key (`apps/api/src/services/webhook/delivery.ts` @ v2.11.504).
- **Open-source versus cloud:**
  - The open-source direct path makes a single attempt. Retries go through a RabbitMQ queue whose consumer is not in the repository.
  - **Inference:** webhook retries are cloud-only.
  - Self-hosting runs Postgres (NuQ queue), Redis and RabbitMQ, with no persistent volumes by default ([self-host](https://docs.firecrawl.dev/contributing/self-host.md), [open source or cloud](https://docs.firecrawl.dev/contributing/open-source-or-cloud.md)).
- **Idempotency key:** `x-idempotency-key` must be a UUID, and a reused key returns 409.
  - It applies to v1 `/crawl` and `/batch/scrape`, and only to `/crawl` in v2.
  - The check runs against a read replica, followed by an insert that is not awaited, so it is **not atomic**.
  - It deduplicates job submission, not result delivery.
- Integrations: n8n, Zapier, Make, LangChain, LlamaIndex and an MCP server. No native S3, GCS or database sink.

## Crawl4AI 0.9.4

- `CrawlResult` (`crawl4ai/models.py`):
  - Content: html, fit_html, cleaned_html, markdown, extracted_content (a JSON string), media, links, tables.
  - Captures: screenshot, pdf, mhtml, downloaded_files.
  - Cache fields: `cache_status`, `cached_at`.
- Cache:
  - SQLite (aiosqlite) at `~/.crawl4ai/crawl4ai.db`, table `crawled_data`, with `url` as the primary key and upserts `ON CONFLICT(url)`. Only the latest version of each URL is kept.
  - Large fields are stored as files addressed by content hash.
  - `CacheMode` is ENABLED, DISABLED, READ_ONLY, WRITE_ONLY or BYPASS; `CrawlerRunConfig` defaults to BYPASS.
- Docker server:
  - Jobs (`/crawl/job`, `/llm/job`) are kept in Redis with a TTL of 3,600 s.
  - `/crawl/stream` returns NDJSON.
- Webhooks (`deploy/docker/webhook.py`):
  - Fire on job completion or failure only.
  - 5 attempts with back-off of 1, 2, 4, 8 and 16 s and a 30 s timeout; 4xx responses are not retried.
  - No signature, no delivery id, no idempotency key.
  - **Inference:** a timeout followed by a retry can deliver twice, so delivery is at-least-once.
- No S3, database or queue sink.

## Stagehand and Browserbase

- Stagehand stores nothing. In v4, `act`, `extract` and `observe` return `{ data, metadata }`, and `extract(instruction, zodSchema)` returns the typed object ([v3 to v4 migration](https://github.com/browserbase/stagehand/blob/main/packages/docs/v4/migrations/v3.mdx)).
- v4 removed the local `cacheDir`. Caching is now server-side on Browserbase, which saves LLM calls but is not result storage. v3's caching is described in [v3 caching](https://github.com/browserbase/stagehand/blob/main/packages/docs/v3/best-practices/caching.mdx).
- Browserbase (paid) stores browser artifacts, not results:
  - Downloads, with list, get and delete APIs ([downloads](https://docs.browserbase.com/platform/browser/files/downloads.md)).
  - Contexts: the persisted user-data directory ([contexts](https://docs.browserbase.com/platform/browser/core-features/contexts.md)).
  - Recordings.
  - Enterprise BYOS sends these artifacts to the customer's S3, the only destination offered ([BYOS](https://docs.browserbase.com/account/enterprise/byos-setup-guide.md)).
- [Webhooks](https://docs.browserbase.com/platform/webhooks/overview.md) cover Functions invocations and builds only, and deliver no scraped data.
  - Signed per the Standard Webhooks spec.
  - 8 attempts over about 32 hours.
  - The docs state at-least-once, with no ordering, and recommend deduplicating on the envelope `id`, which stays the same across retries ([verifying deliveries](https://docs.browserbase.com/platform/webhooks/verifying-deliveries.md)).

## Bright Data (closed, paid)

- Formats: NDJSON (the default), json, jsonl, csv. Deliver Snapshot also accepts xlsx and parquet as file extensions. Gzip is optional. [data delivery](https://docs.brightdata.com/products/scrapers/scrapers-library/data-delivery.md)
- [Deliver Snapshot](https://docs.brightdata.com/api-reference/scrapers/delivery-apis/deliver-snapshot.md), `POST /datasets/v3/deliver/{snapshot_id}`:
  - `type` values: s3, gcs, gcp_pubsub, azure, snowflake, sftp, webhook, email, ali_oss.
  - S3 is reached by assuming a role in the customer's account, and `endpoint_url` allows S3-compatible storage.
  - BigQuery is not a destination.
  - Each destination needs its own call.
- Snapshot ids and limits:
  - Every job has a `snapshot_id`, and each delivery has its own id with a status endpoint.
  - Snapshots are kept 30 days, and each delivered file is capped at 5 GB. [limits](https://docs.brightdata.com/products/scrapers/limits.md)
- Webhooks:
  - Auth is an `auth_header` value sent as the Authorization header; there is no signature.
  - The `dca-collection-id` header carries the snapshot id.
  - The endpoint must return 200 within 30 s or the delivery is retried. The retry count and schedule are undocumented.
- Streamed delivery sends batches of 10 to 100,000 lines while the job runs. ([stream and file delivery](https://docs.brightdata.com/products/scrapers/scrapers-library/stream-and-file-delivery.md))
- Scraper Studio's trigger endpoint "is not idempotent and does not deduplicate inputs across runs". Inputs are deduplicated within a single job only. ([llms-full.txt](https://docs.brightdata.com/llms-full.txt))

## Zyte

- Zyte API stores nothing; results come back in the response. ([Zyte API](https://docs.zyte.com/zyte-api/usage/index.md))
- Scrapy Cloud (paid hosting):
  - Items are stored per job and downloaded from `storage.zyte.com/items/<job>?format=json|jl|xml|csv`; csv needs `fields`. [download](https://docs.zyte.com/scrapy-cloud/usage/items/download.md), [items API](https://docs.zyte.com/scrapy-cloud/usage/reference/http/items.md)
  - Collections is a key-value store keyed by `_key`. [collections](https://docs.zyte.com/scrapy-cloud/usage/reference/http/collections.md)
  - Retention is 7 days free, or up to 120 days with paid units. [pricing](https://docs.zyte.com/scrapy-cloud/pricing.md)
- Self-serve export means Scrapy feed exports plus plugins. The only documented database destination is BigQuery, through the third-party `scrapy-bigquery` package. [export guide](https://docs.zyte.com/web-scraping/guides/export/index.md)
- Zyte Data (managed) delivers JSON, JSONL, CSV or Parquet to S3, GCS, Azure Blob, a webhook or by API pull, after checking each run against the agreed schema. Dedup is not mentioned.
  - The source is a marketing page, not API docs: [product scraper](https://www.zyte.com/data-types/product-scraper/).
  - An older page lists FTP, SFTP, S3, GCS, email, Dropbox and Drive instead: [business automation](https://www.zyte.com/business-automation/).

## Sinks by destination

"OSS" means available in the open-source framework; "cloud" means available only in the paid service.

| Destination | Crawlee JS | Crawlee Py | Apify | Scrapy | spider | Firecrawl | Crawl4AI | Bright Data | Zyte |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| Local files | OSS | OSS | — | OSS | OSS (CLI, WARC) | — | OSS (cache) | — | OSS (Scrapy) |
| SQLite | — | OSS | — | — | OSS (`disk`) | — | OSS (cache only) | — | — |
| PostgreSQL / MySQL / MariaDB | — | OSS | community Actor | pipeline you write | cloud (Supabase) | — | — | — | — |
| Redis | — | OSS | — | — | — | — | — | — | — |
| MongoDB | — | — | — | docs example | — | — | — | — | — |
| BigQuery | — | — | — | third-party | — | — | — | — | third-party |
| Snowflake | — | — | cloud (native app) | — | — | — | — | cloud | — |
| Elasticsearch | — | — | — | — | — | — | — | — | — |
| DuckDB | — | — | — | — | — | — | — | — | — |
| S3 | — | — | cloud (Actor) | OSS | cloud | — | — | cloud | cloud / OSS |
| GCS | — | — | — | OSS | cloud | — | — | cloud | cloud / OSS |
| Azure Blob | — | — | — | plugin | cloud | — | — | cloud | cloud / plugin |
| Pub/Sub or a queue | — | — | — | — | — | — | — | cloud (Pub/Sub) | — |
| Webhook | — | — | cloud | — | cloud | OSS (1 try) / cloud | OSS | cloud | cloud |
| Vector databases | — | — | cloud | — | — | via frameworks | — | — | — |

None of these products has a first-party Elasticsearch or DuckDB sink.

## Duplicate writes after retries and crashes

| Product | Retry of a failed handler | Crash or migration | Mechanism |
| --- | --- | --- | --- |
| Crawlee JS 3.18 | Duplicates items | Duplicates items | None |
| Crawlee JS 4.0-rc | No duplicates (rollback) | A partial commit can be re-applied | Per-request transaction; at-least-once commit |
| Crawlee Python 1.10 | No duplicates through the context helpers | A replay that stops partway can be re-applied (inference) | Buffered results replayed after success |
| Apify | Same as the framework | Duplicates possible (inference) | Append-only dataset; webhook dispatch id |
| Scrapy | Items are emitted per response | Batch lost (remote storage) or partial file (local); duplicates on resume (inference) | In-memory dupefilter example only |
| spider | — | Pages lost to `Lagged` | None |
| Firecrawl | — | — | `webhookId`; idempotency key on submission only |
| Crawl4AI | — | Webhook can fire twice | URL-keyed cache upsert only |
| Browserbase | — | — | Stable webhook event `id` |
| Bright Data | — | — | `snapshot_id`; Scraper Studio not idempotent |

## Relevance to Automan

- **No competitor enforces a run epoch or fencing token at the destination.** Every lock found is a lease with an expiry: Crawlee's client key, Crawlee Python's `time_blocked_until`, and the Redis block. None of them stops a late commit or a zombie copy. ADR 0003's compare-and-set at the destination has no precedent in this market; its precedents are the workflow engines that ADR 0003 cites.
- **Crawlee JS v4 has converged on staging handler outputs**, merged in August 2026: deferred writes, rolled back on failure, committed before the request is marked handled. Crawlee Python does the same by buffering and replaying. Both stop at an at-least-once commit. ADR 0003's "visible exactly once" goes one step further.
- **The de facto standard for webhooks is at-least-once delivery with a dedup id**: Apify's `X-Apify-Webhook-Dispatch-Id`, Browserbase's event `id` and Firecrawl's `webhookId`. Signatures vary: Standard Webhooks (Browserbase), a timestamped HMAC (Spider Cloud), an HMAC with no timestamp (Firecrawl), an Authorization header (Bright Data), or nothing (Crawl4AI).
- **The common export formats are JSON, JSONL and CSV.** XML is in Scrapy, Apify and Zyte; XLSX in Apify and Bright Data; Parquet only in Bright Data and Zyte Data; RSS and HTML tables only in Apify.
- **Database sinks are rare in open source.** Crawlee Python's SQL storage client is the only first-party PostgreSQL/MySQL path, and it is a storage backend, not a fenced sink. Apify offers no first-party database sink. Object stores are the common delivery target in paid offerings.

## Gaps

- The spider-rs and Stagehand source was read on `main`, not at the published tags.
- The Crawlee Python version that added PostgreSQL was not confirmed.
- Bright Data's webhook retry count and schedule are undocumented.
- The Zyte Data delivery options come from marketing pages, not API docs.
- Browserbase Director was not found in the current docs.
- The version that introduced Crawl4AI webhooks was not verified.
- The claim that Firecrawl webhook retries are cloud-only is an inference from the open-source repository.
