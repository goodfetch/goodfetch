# E. First-party sinks for scrapers and loaders

Research date: 2026-10-11. Only vendor docs, vendor catalogs, vendor registries and vendor GitHub orgs were used.

"First-party" here means the vendor ships or maintains the sink itself. It excludes community or marketplace entries and anything that goes through Zapier, Make or n8n.

Destinations checked: MySQL, MariaDB, SQLite, Microsoft SQL Server, MongoDB, Elasticsearch, OpenSearch, Redis, ClickHouse, Snowflake, BigQuery, Amazon Redshift, Databricks (Delta), Apache Iceberg, DuckDB/MotherDuck, Supabase, pgvector, Qdrant, Pinecone, Weaviate, Milvus, Chroma, LanceDB.

## Apify

- Catalog: the [integrations index](https://docs.apify.com/platform/integrations) plus Actors that the `apify` account publishes in the Store. I listed those Actors with the public Store API (`api.apify.com/v2/store`, filtered on `username == "apify"`).
- Pinecone: Apify ships [apify/pinecone-integration](https://apify.com/apify/pinecone-integration) and documents it in the [integrations index](https://docs.apify.com/platform/integrations).
- Qdrant: Apify ships [apify/qdrant-integration](https://apify.com/apify/qdrant-integration).
- Weaviate: Apify ships [apify/weaviate-integration](https://apify.com/apify/weaviate-integration).
- Milvus: Apify ships [apify/milvus-integration](https://apify.com/apify/milvus-integration).
- Chroma: Apify ships [apify/chroma-integration](https://apify.com/apify/chroma-integration).
- pgvector: Apify ships [apify/pgvector-integration](https://apify.com/apify/pgvector-integration). Its page talks only about PostgreSQL with pgvector and never names Supabase. Supabase runs on Postgres, so it should work there too (**inference**).
- OpenSearch: [apify/opensearch-integration](https://apify.com/apify/opensearch-integration) exists, but the Actor API returns `isDeprecated: true`. I counted it as deprecated, not as current.
- Snowflake: the [Snowflake page](https://docs.apify.com/platform/integrations/snowflake) describes an Apify Snowflake Native App from the Snowflake Marketplace. It can create a new table or insert into an existing one. The page also says "This integration uses a third-party service", yet sends support requests to Apify. I counted it as first-party, with that caveat.
- Not first-party: MongoDB and MySQL exist only as community Actors ([drobnikj/mongodb-import](https://apify.com/drobnikj/mongodb-import), [petr_cermak/mysql-insert](https://apify.com/petr_cermak/mysql-insert)), as the [integrations index](https://docs.apify.com/platform/integrations) notes. Zapier, Make and n8n are listed separately as workflow tools.
- I searched for these names and found no Apify-owned Actor: `apify/supabase-integration`, `elasticsearch-integration`, `mongodb-integration`, `lancedb-integration`, `redis-integration`, `bigquery-integration` and `snowflake-integration`. All returned `record-not-found`.
- Semantics, vector Actors: with `deltaUpdates`, the Actor identifies each item by key fields and compares checksums. A changed item is deleted and re-inserted. An unchanged item only gets `last_seen_at` updated. Items not seen for `expiredObjectDeletionPeriodDays` are deleted ([pgvector Actor](https://apify.com/apify/pgvector-integration)).
- Semantics, webhooks: a non-2xx response triggers retries with exponential backoff. The first retry comes after about 1 minute and the 11th after about 32 hours, then retries stop. Each request times out after 2 minutes. "The webhook might be invoked more than once", and every request carries `X-Apify-Webhook-Dispatch-Id` ([webhook actions](https://docs.apify.com/platform/integrations/webhooks/actions)).

## Scrapy

- Catalog: [feed exports](https://docs.scrapy.org/en/latest/topics/feed-exports.html), plus the [scrapy-plugins](https://github.com/scrapy-plugins) and [scrapy](https://github.com/scrapy) GitHub orgs, listed with `gh repo list`.
- Built-in formats: JSON, JSON lines, CSV, XML, Pickle and Marshal ([feed exports](https://docs.scrapy.org/en/latest/topics/feed-exports.html)).
- Built-in storages: `file`, `ftp`, `ftps`, `s3`, `gs` and `stdout`. None of the checked databases is built in ([feed exports](https://docs.scrapy.org/en/latest/topics/feed-exports.html)).
- Snowflake: the only checked database with a plugin in the scrapy-plugins org is [scrapy-snowflake-stage-exporter](https://github.com/scrapy-plugins/scrapy-snowflake-stage-exporter). It buffers JSON, runs `PUT` into a stage, then runs `COPY INTO`. Its last push was 2021-12-03, and you install it from git. Count it as an org-hosted plugin, not part of Scrapy itself.
- Other scrapy-plugins exporters target only file stores or apps: Azure Storage, Dropbox, Google Drive, Google Sheets, OneDrive, SFTP, WARC, BigML and Django models ([org](https://github.com/scrapy-plugins)).
- Semantics: `overwrite` defaults to False for local files and True for FTP, S3 and GCS. S3 and GCS cannot append. `batch_item_count` splits output into several files, and `FEED_STORE_EMPTY` controls whether an empty feed still writes a file ([feed exports](https://docs.scrapy.org/en/latest/topics/feed-exports.html)).

## Spider Cloud

- Catalog: `data_connectors` in the [API reference](https://spider.cloud/docs/api/), also mentioned in the [overview](https://spider.cloud/docs/overview/).
- Connectors: S3, GCS, Google Sheets, Azure Blob and Supabase ([API reference](https://spider.cloud/docs/api/)).
- From the checked list, only Supabase is covered. It takes `url`, `anon_key` and `table`.
- Semantics: the reference documents no retries, upsert behavior or batching ([API reference](https://spider.cloud/docs/api/)).

## Bright Data

- Catalog, Scraper API: webhook, S3, GCS, Snowflake, Google Pub/Sub, Azure Storage, SFTP/FTP and API download ([Scraper API delivery](https://docs.brightdata.com/products/scrapers/scrapers-library/data-delivery)).
- Catalog, datasets/marketplace: email, API download, webhook, S3, GCS, Pub/Sub, Azure Blob, Snowflake, SFTP, Alibaba OSS and OCI ([marketplace delivery](https://docs.brightdata.com/products/marketplace/data-delivery-and-export)).
- From the checked list, only Snowflake is covered. It loads through an internal named stage. No BigQuery, Redshift, Databricks or OLTP database appears on these pages.
- I did not check Scraping Browser delivery separately. It is a remote browser, so delivery should be the caller's job (**inference**).
- Semantics: "If your endpoint fails or times out, Bright Data retries delivery." The retry count is not stated. Return 200 within 30 seconds. Snapshots stay downloadable by `snapshot_id` for 30 days ([Scraper API delivery](https://docs.brightdata.com/products/scrapers/scrapers-library/data-delivery)).
- Dataset output formats: JSON, NDJSON, CSV, XLSX and Parquet, optionally gzipped ([marketplace delivery](https://docs.brightdata.com/products/marketplace/data-delivery-and-export)).

## Zyte

- Catalog: the [export guide](https://docs.zyte.com/web-scraping/guides/export/index.html) for Scrapy on Scrapy Cloud. Destinations are S3, Azure Storage, Dropbox, FTP, GCS, Google Drive, Google Sheets, SFTP and Google BigQuery. For databases, the guide says to write a custom pipeline.
- BigQuery: the guide uses `scrapy-bigquery` from a personal GitHub account ([BigQuery guide](https://docs.zyte.com/web-scraping/guides/export/item-storage/google-bigquery.html)). That makes it documented but third-party, not first-party.
- Zyte Data (managed service): delivery to "GCS, S3, CSV, Azure, AWS", plus API ([data extraction](https://www.zyte.com/data-extraction/)). None of the checked databases is named.
- Result: no first-party sink for any checked destination. The scrapy-plugins Snowflake exporter is listed under Scrapy.

## Firecrawl

- Catalog: the [docs index](https://docs.firecrawl.dev/llms.txt) has no page about delivering data to a database, warehouse or vector store. Delivery means API responses and webhooks.
- Result: no first-party sink for any checked destination. Vector stores appear only in blog tutorials that use n8n or LangChain (**not first-party**).
- Semantics: 3 retries, at 1, 5 and 15 minutes. The endpoint must answer 2xx within 10 seconds, or the delivery counts as failed ([webhooks](https://docs.firecrawl.dev/webhooks/overview)).

## Crawl4AI

- Catalog: the [CrawlResult docs](https://docs.crawl4ai.com/core/crawler-result/) and the [CLI docs](https://docs.crawl4ai.com/core/cli/). Output is an in-memory result: HTML, markdown, `extracted_content` JSON, PDF and MHTML. The CLI can write JSON or markdown.
- Result: no first-party sink for any checked destination. The [deep crawling docs](https://docs.crawl4ai.com/core/deep-crawling/) use Redis only in an example that saves crawl state, not as a data sink.

## Browse AI

- Catalog: the [integrations page](https://www.browse.ai/integrations). Direct options are Google Sheets, Airtable, Amazon S3, webhooks, API and JSON/CSV export. Everything else goes through Zapier, Make, Pabbly or Integrately.
- Result: no first-party sink for any checked destination.

## Octoparse

- Catalog: the [platform docs](https://www.octoparse.com/docs/en/platform/tools-and-services) say "Databases such as MySQL, PostgreSQL, SQL Server, and Oracle", with Google Sheets listed separately.
- Auto-export supports MySQL, SQL Server, PostgreSQL and Google Sheets, and only for Cloud data ([auto export](https://helpcenter.octoparse.com/en/articles/6470955-export-data-to-a-database-automatic)).
- From the checked list: MySQL and SQL Server. Supabase could work through the PostgreSQL export (**inference**). For anything else, Octoparse points to its Open API or Zapier.
- Semantics: in a manual export, an optional primary key makes "duplicate keys … overwrite existing records". Each scheduled auto-export transfers "only new/unexported data". The auto-export tool runs on your own machine and has to stay open ([auto export](https://helpcenter.octoparse.com/en/articles/6470955-export-data-to-a-database-automatic)).

## ScrapingBee

- Catalog: the [integrations page](https://www.scrapingbee.com/integrations/). It lists API, CLI, MCP, SDKs, LangChain, LLM and agent tools, plus n8n, Make and Zapier.
- Result: no first-party sink for any checked destination. Google Sheets examples run through Zapier, Make or n8n ([Zapier](https://www.scrapingbee.com/documentation/zapier/), [n8n](https://www.scrapingbee.com/documentation/n8n/)).

## Airbyte

- Catalog: the official connector registry, [oss_registry.json](https://connectors.airbyte.com/files/registries/v0/oss_registry.json) and [cloud_registry.json](https://connectors.airbyte.com/files/registries/v0/cloud_registry.json), read on 2026-10-11. The [destinations page](https://docs.airbyte.com/integrations/destinations) only renders with JavaScript.
- Support levels: `certified` in the registry is what the docs call "Airbyte", meaning "maintained and supported by Airbyte". `community` is "Marketplace", meaning "maintained by Airbyte's community members" ([support levels](https://docs.airbyte.com/integrations/connector-support-levels)).
- Certified (first-party): MS SQL Server, ClickHouse, Snowflake, BigQuery, Redshift, Databricks Lakehouse, Iceberg, MotherDuck and Pinecone. Iceberg comes through [S3 Data Lake](https://docs.airbyte.com/integrations/destinations/s3-data-lake), which uses Nessie, Glue, REST or Polaris catalogs, and [GCS Data Lake](https://docs.airbyte.com/integrations/destinations/gcs-data-lake). The registry marks Databricks, S3 Data Lake, MotherDuck and Pinecone as alpha or beta, even though they are certified.
- Community (marketplace, **not first-party**): MySQL, Local SQLite, MongoDB, ElasticSearch, Redis, DuckDB, PGVector, Qdrant, Weaviate, Milvus and Chroma.
- Cloud availability: the cloud registry does not list MySQL, Local SQLite, ElasticSearch or Chroma. Those four are OSS only.
- Not in either registry: MariaDB, OpenSearch, Supabase and LanceDB. Supabase could be reached through the Postgres destination, which is certified (**inference**, not a named connector).
- Semantics: Airbyte has five sync modes. They are Full Refresh Overwrite, Full Refresh Append, Full Refresh Overwrite + Deduped, Incremental Append, and Incremental Append + Deduped, which keeps "a de-duplicated view mirroring the state of the stream in the source" ([sync modes](https://docs.airbyte.com/platform/using-airbyte/core-concepts/sync-modes)).
- Per the registry's `supported_destination_sync_modes`, most SQL and vector destinations support `append_dedup`. MongoDB, Redis, SQLite, DuckDB and ElasticSearch support only `overwrite` and `append`.

## dlt (dlthub)

- Catalog: the [destinations index](https://dlthub.com/docs/dlt-ecosystem/destinations/).
- First-party destinations: BigQuery, ClickHouse, Databricks, Delta Lake, DuckDB, DuckLake, Iceberg, Lance, LanceDB, MS SQL, MotherDuck, Postgres, Redshift, Snowflake, Qdrant and Weaviate. It also has filesystem, Athena, Dremio, Fabric, Synapse and Hugging Face destinations.
- MySQL and SQLite: covered by the [SQLAlchemy destination](https://dlthub.com/docs/dlt-ecosystem/destinations/sqlalchemy). MySQL, SQLite and DuckDB "are considered to have full support and are tested as part of the dlt CI suite". MariaDB is not named, so treat it as untested (**inference**: it may work through the MySQL dialect).
- Supabase: the [Postgres page](https://dlthub.com/docs/dlt-ecosystem/destinations/postgres) links a Supabase setup guide. That page does not mention pgvector.
- Community, **not first-party**: Starrocks, Clickhouse Distributed, CrateDB and Hotdata.dev ([community destinations](https://dlthub.com/docs/dlt-ecosystem/destinations/community-destinations)).
- Not covered: MongoDB, Elasticsearch, OpenSearch, Redis, Pinecone, Milvus, Chroma and pgvector. For targets like these you can write a custom sink with `@dlt.destination` ([custom destination](https://dlthub.com/docs/dlt-ecosystem/destinations/destination)).
- Semantics: `write_disposition` can be `append`, `replace` or `merge`. Merge strategies are `delete-insert`, `upsert`, `scd2`, `insert-only` and `cdc` ([incremental loading](https://dlthub.com/docs/general-usage/incremental-loading)). Postgres supports all of them ([Postgres](https://dlthub.com/docs/dlt-ecosystem/destinations/postgres)).
- On SQLAlchemy targets, `merge` offers only `delete-insert` and `scd2`, and `replace` uses `truncate-and-insert` or `insert-from-staging` ([SQLAlchemy](https://dlthub.com/docs/dlt-ecosystem/destinations/sqlalchemy)).
- In a custom destination, a failing call is retried 5 times. Duplicates are possible unless each batch is atomic ([custom destination](https://dlthub.com/docs/dlt-ecosystem/destinations/destination)).

## Matrix: destination → vendors with a first-party sink

Firecrawl, Crawl4AI, Browse AI, ScrapingBee and Zyte have no first-party sink for any destination below.

| Destination | First-party sinks | Community or other routes only (not first-party) |
|---|---|---|
| MySQL | dlt (SQLAlchemy, CI-tested), Octoparse | Airbyte (community), Apify (community Actor) |
| MariaDB | none | dlt SQLAlchemy may work (**inference**) |
| SQLite | dlt (SQLAlchemy, CI-tested) | Airbyte Local SQLite (community) |
| MS SQL Server | Airbyte (certified), dlt, Octoparse | |
| MongoDB | none | Airbyte (community), Apify (community Actor) |
| Elasticsearch | none | Airbyte (community, OSS only) |
| OpenSearch | none current | Apify Actor exists but is deprecated |
| Redis | none | Airbyte (community) |
| ClickHouse | Airbyte (certified), dlt | |
| Snowflake | Airbyte (certified), dlt, Bright Data, Apify (Native App, see caveat) | Scrapy: scrapy-plugins exporter (org-hosted, stale since 2021) |
| BigQuery | Airbyte (certified), dlt | Zyte guide uses the third-party scrapy-bigquery |
| Amazon Redshift | Airbyte (certified), dlt | |
| Databricks (Delta) | Airbyte (certified, alpha), dlt (Databricks and Delta Lake) | |
| Apache Iceberg | Airbyte (S3 and GCS Data Lake, certified), dlt | |
| DuckDB / MotherDuck | dlt (both), Airbyte (MotherDuck certified) | Airbyte DuckDB (community) |
| Supabase | Spider Cloud (connector), dlt (Postgres plus Supabase guide) | via Postgres: Airbyte, Octoparse, Apify pgvector (**inference**) |
| pgvector | Apify | Airbyte PGVector (community) |
| Qdrant | Apify, dlt | Airbyte (community) |
| Pinecone | Apify, Airbyte (certified, beta) | |
| Weaviate | Apify, dlt | Airbyte (community) |
| Milvus | Apify | Airbyte (community) |
| Chroma | Apify | Airbyte (community, OSS only) |
| LanceDB | dlt | |
