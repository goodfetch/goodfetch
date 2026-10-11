# Sinks, Notifications and Integrations: research for the catalog

Research of October 10–11, 2026, behind the decision [Export connectors and integrations catalog](https://github.com/Jbmanllr/automan/issues/88). It asks, for about 60 destinations, which primitive lets a write be exactly-once and reject a stale run epoch (ADR 0003, ADR 0008), which Rust client exists, which test double CI can use, and who already ships the destination first-party; then how comparable tools are extended, what the delivery leaders do, and what users complain about. Exa ran out of credits early, so most reads used direct fetches, `gh`, crates.io and source at release tags. Inferences are marked **inference** in each note. This branch is never merged.

| Note | Covers |
| --- | --- |
| [a-sql-doc-kv.md](a-sql-doc-kv.md) | MySQL, MariaDB, SQLite, SQL Server, MongoDB, Redis |
| [b-search-warehouse.md](b-search-warehouse.md) | Elasticsearch, OpenSearch, ClickHouse, Snowflake, BigQuery, Redshift |
| [c-lakehouse.md](c-lakehouse.md) | Databricks/Delta, Iceberg, DuckDB/MotherDuck, Supabase |
| [d-vector.md](d-vector.md) | pgvector, Qdrant, Pinecone, Weaviate, Milvus, Chroma, LanceDB |
| [a1-files.md](a1-files.md) | S3 and S3-compatibles, GCS, Azure Blob, SFTP, FTP, Drive, Sheets, Dropbox, OneDrive, Box, Airtable, Notion |
| [a2-messaging.md](a2-messaging.md) | Kafka, Redpanda, SQS, SNS, Kinesis, Firehose, Pub/Sub, Service Bus, Event Hubs, RabbitMQ, NATS, Redis Streams |
| [a3-notify-vendors.md](a3-notify-vendors.md) | Email providers, Slack, Teams, Discord, Telegram, Standard Webhooks; competitors' destinations |
| [e-first-party.md](e-first-party.md) | Who ships each database, warehouse and vector destination first-party |
| [b1-integrations.md](b1-integrations.md) | n8n, Zapier, Make, Pipedream, Activepieces, Power Automate, IFTTT; Airbyte sources; Airbyte, Singer and dlt bridges; AI loaders |
| [b5-plugins.md](b5-plugins.md) | Plugin models: Vector, OpenTelemetry Collector, Terraform, Redpanda Connect, Kafka Connect, Airbyte, Fluent Bit, Debezium, WASM component model |
| [g-best-practice.md](g-best-practice.md) | Fivetran, Census, Segment, RudderStack, Hightouch, Airbyte, dlt, Singer, Kafka Connect, Vector, Svix, Hookdeck, Stripe; destination quotas; gap analysis |
| [f-user-pains.md](f-user-pains.md) | 65 dated public signals, coded and ranked |

## Main findings

- **Nobody else fences sink writes.** No scraping or data-movement product in the survey rejects a stale run at the destination; the closest are Kafka transactions, Snowpipe Streaming channels and Kafka Connect's source exactly-once.
- **A destination's name does not fix its guarantee.** AWS S3, R2, MinIO and Tigris support conditional writes, Backblaze B2, Wasabi and DigitalOcean Spaces document none; MongoDB fences only on a replica set; Redis can lose a fence write on failover; Elasticsearch, Qdrant and LanceDB fence per record but not per batch; SQS, Service Bus, NATS and Redis dedup only within a time window. The class must be probed per deployment.
- **Database sinks are rare among scrapers.** Apify's are almost all vector stores; Firecrawl, Crawl4AI, Browse AI, ScrapingBee and Zyte ship none; dlt and Airbyte cover most destinations.
- **Rust clients are uneven.** Strong official crates for AWS, Google Cloud, Azure Blob, MongoDB, Qdrant, Milvus, Chroma, LanceDB and BigQuery's Storage Write API; no official Snowflake driver; Elasticsearch's official crate is alpha; Pinecone's is stale; Weaviate, Airtable and Box are HTTP only.
- **No surveyed system uses WASM for output plugins**; separate processes over RPC (Terraform, Redpanda Connect) and compiled-in sinks (Vector, OpenTelemetry Collector) dominate.
- **Users' top pains** are silent failure (15 signals) and duplicates on recurring runs (13); quota errors cluster on Google Sheets.
