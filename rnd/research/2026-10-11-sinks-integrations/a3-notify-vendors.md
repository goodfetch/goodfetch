# A3: Notification and email destinations, and the competitors' first-party destination matrix

Research date: 2026-10-11. Sources are primary: vendor docs, crates.io API, docs.rs, and GitHub at release tags. Anything marked **inference** is my own reading, not a documented fact.

Class legend:
- **idempotent keyed**: the receiver dedups on a key we send, within a stated window.
- **at-least-once**: the receiver has no documented dedup. A retry after an ambiguous failure (timeout, reset) can produce a duplicate.
- **fenced**: none found. No destination below offers a compare-and-set (CAS) on our epoch.

## Part 1: notification and email destinations

### Summary table

| Destination | Primitive (doc) | Class | Rust crate (version, date, license, owner) | CI double | First-party in |
|---|---|---|---|---|---|
| SMTP | `Message-ID` header (RFC 5322). The server does not dedup. | at-least-once | `lettre` 0.11.23, 2026-08-03, MIT, community (lettre org) | Mailpit v1.31.4 (2026-10-03, MIT, maintained). smtp4dev 3.15.0 (2026-03-03, BSD-3). MailHog last release v1.0.1 in 2020, so treat it as unmaintained. | Bright Data (email delivery type), Zyte managed delivery (email), Apify (Gmail) |
| Amazon SES v2 `SendEmail` | None. The request has no client token or idempotency field. | at-least-once | `aws-sdk-sesv2` 1.140.1, 2026-10-08, Apache-2.0, official (AWS) | LocalStack SES: v1 has 46/71 ops, v2 has 14/116 ops. Sent mail is readable at `GET /_aws/ses`. The docs say "Available from Hobby", and the SMTP relay needs the Ultimate plan. | none of the competitors |
| Resend | `Idempotency-Key` header, 1-256 chars, kept 24 h. Same key and same payload returns the original response. Same key with a different payload returns 409 `invalid_idempotent_request`. A concurrent request returns 409 `concurrent_idempotent_requests`. Works on send and batch send. Over SMTP, use the `Resend-Idempotency-Key` header. | idempotent keyed (24 h) | `resend-rs` 0.34.0, 2026-10-01, MIT, official (resend org) | No emulator. Test addresses `delivered@` / `bounced@` / `complained@` / `suppressed@resend.dev` count against quota. | none |
| Postmark | None. The support article says Postmark "does not currently support an idempotency key feature" (modified 2026-10-06). | at-least-once | `postmark` 2.0.1, 2026-05-28, MIT OR Apache-2.0, community | `X-Postmark-Server-Token: POSTMARK_API_TEST` validates a request without delivering it. | none |
| SendGrid v3 `/mail/send` | None documented. `custom_args` and `batch_id` are metadata and grouping, not dedup. | at-least-once | `sendgrid` 0.27.2, 2026-09-24, MIT, community | `mail_settings.sandbox_mode` (**inference**: not re-verified this session) | none |
| Mailgun `/messages` | None. The feature request "Add Idempotency-Key" is marked "planned" since 2024-07-10. | at-least-once | `mailgun-rs` 2.0.2, 2026-02-06, MIT, community | `o:testmode` (**inference**: not re-verified this session) | none |
| Slack `chat.postMessage` | No idempotency argument. `client_msg_id` appears only in two error codes (`duplicate_message_not_found`, `duplicate_channel_not_found`) and is not a documented argument. | at-least-once | `slack-morphism` 2.31.0, 2026-10-10, Apache-2.0, community | No emulator. Use an HTTP mock (`wiremock` 0.6.5 or `httpmock` 0.8.3). Slack developer sandboxes are free Enterprise orgs that last 1 to 6 months. | Apify (Slack), Browse AI (via Zapier/Make) |
| Slack incoming webhook | None. The response is plain `ok` with no `ts`. Limit is 1 per second with short bursts. | at-least-once | plain HTTP (`reqwest`) or `slack-morphism` | HTTP mock | Apify, Spider (generic webhook only) |
| Microsoft Teams, Workflows webhook | None. It is a Power Automate "When a Teams webhook request is received" trigger. Limit is about 4 requests per second and 28 KB per message. | at-least-once | plain HTTP. There is no official Microsoft Graph Rust SDK (the microsoftgraph GitHub org has no Rust repo). | HTTP mock | Apify (Power Automate) |
| Teams, Graph `chatMessage` (channel) | None. Application permission (`Teamwork.Migrate.All`) is allowed only for migration. Normal sends need delegated `ChannelMessage.Send`. | at-least-once | `graph-rs-sdk` 3.0.1, 2025-04-20, MIT, community | HTTP mock | none |
| Discord bot Create Message | `nonce` (up to 25 chars) plus `enforce_nonce: true`. The nonce is "checked for uniqueness in the past few minutes". On a repeat from the same author, Discord returns the earlier message and creates no new one. | idempotent keyed (window "a few minutes", not stated exactly) | `serenity` 0.12.5, 2025-12-20, ISC, community. It has `nonce()` and `enforce_nonce()`. `twilight-http` 0.17.1, 2025-12-13, ISC, community: `nonce(u64)` only, no `enforce_nonce` in the 0.17.1 tag or on main. | HTTP mock | none |
| Discord Execute Webhook | No nonce or idempotency param. `?wait=true` returns the created message. | at-least-once | `serenity` or `twilight-http`, or plain HTTP | HTTP mock | none |
| Telegram Bot API `sendMessage` | None among the params (Bot API 10.3, 2026-08-24). `update_id` dedups inbound updates only. | at-least-once | `teloxide` 0.17.0, 2025-07-11, MIT, community. `frankenstein` 0.52.1, 2026-09-25, WTFPL, community. | Telegram test environment (`https://api.telegram.org/bot<token>/test/METHOD`, separate accounts). Self-hosted `telegram-bot-api` server. HTTP mock. | Apify (via Zapier) |
| Generic webhook (Standard Webhooks) | `webhook-id`, the same across retries, which "is often used as an idempotency key" by the receiver. | at-least-once from the sender. The receiver dedups on `webhook-id`. | `standardwebhooks` 1.0.1 on crates.io (2024-03-04, MIT, official spec repo). The repo's Cargo.toml is at 1.0.2 but that version is not published. `svix` 2.8.0, 2026-10-08, MIT, official Svix. Both have `sign` and `verify`. | HTTP mock. `svix-server` is open source and can be self-hosted (**inference**: usable as a CI sender). | Apify, Bright Data, Firecrawl, Spider, Crawl4AI, Browse AI |

### Notes per destination

**SMTP.** `Message-ID` is a client-set header. SMTP servers do not reject or dedup a repeated `Message-ID`. **Inference:** some mailbox providers collapse duplicates with the same `Message-ID`, but this is not specified anywhere and we must not rely on it. A stable `Message-ID` such as `<delivery-id.epoch@automan>` still helps the receiver trace duplicates.

**Amazon SES v2.** The full `SendEmail` request syntax has no token field. Its fields are ConfigurationOverrides, ConfigurationSetName, Content, Destination, EmailTags, EndpointId, the feedback and from fields, ListManagementOptions, ReplyToAddresses and TenantName. Source: https://docs.aws.amazon.com/ses/latest/APIReference-V2/API_SendEmail.html. `EmailTags` can carry our delivery ID so that SNS events map back to it. That traces a send but does not dedup it. LocalStack: https://docs.localstack.cloud/aws/services/ses/

**Resend.** https://resend.com/docs/dashboard/emails/idempotency-keys. It is the only email API in this set with a documented keyed dedup. Key form for us: `{delivery_id}`. The epoch can sit in the payload, because a different payload under the same key returns 409.

**Postmark.** https://postmarkapp.com/support/article/what-is-an-idempotency-key and https://postmarkapp.com/developer/api/email-api. The request has no client message ID. `Metadata` (key/value pairs) can carry the delivery ID for tracing. Test token: https://postmarkapp.com/developer/api/overview

**SendGrid.** https://www.twilio.com/docs/sendgrid/api-reference/mail-send/mail-send. It documents no idempotency header.

**Mailgun.** https://feedback.mailgun.com/forums/156243-feature-requests/suggestions/18429739-add-support-for-idempotent-requests-to-the-api ("planned" since 2024-07-10).

**Slack.** https://docs.slack.dev/reference/methods/chat.postMessage. The arguments are token, channel, as_user, attachments, blocks, current_draft_last_updated_ts, icon_*, link_names, markdown_text, metadata, mrkdwn, parse, reply_broadcast, text, thread_ts, unfurl_*, and username. `metadata` (`event_type` + `event_payload`) can carry the delivery ID, and it is visible to the workspace. Rate limit is about 1 message per second per channel, with a 429 and `Retry-After` on excess: https://docs.slack.dev/apis/web-api/rate-limits. Incoming webhooks: https://docs.slack.dev/messaging/sending-messages-using-incoming-webhooks. Sandboxes: https://docs.slack.dev/tools/developer-sandboxes. **Inference:** the `client_msg_id` error codes suggest an undocumented server-side dedup path. Do not build on it.

**Microsoft Teams.**
- Office 365 (Microsoft 365) Connectors, which include the old Incoming Webhook, were switched off in a rollout from 2026-05-18 to 2026-05-22. After that, connectors "will no longer function". Earlier dates (Dec 2025, then Mar 31 and Apr 30 2026) were postponed. Source (updated 2026-04-14): https://devblogs.microsoft.com/microsoft365dev/retirement-of-office-365-connectors-within-microsoft-teams/
- The replacement is the Workflows app (Power Automate). It accepts Adaptive Cards and MessageCard payloads, but MessageCard buttons are not rendered. The flow is owned by a user, so it can be orphaned. Source (2026-05-06): https://learn.microsoft.com/en-us/microsoftteams/platform/webhooks-and-connectors/how-to/add-incoming-webhook
- Graph send: https://learn.microsoft.com/en-us/graph/api/channel-post-messages?view=graph-rest-1.0. Microsoft says not to use Teams "as a log file". In import mode, `createdDateTime` must be unique to the millisecond (409 on clash). That is a migration-only uniqueness check, not an idempotency feature.

**Discord.**
- https://docs.discord.com/developers/resources/message describes `nonce` and `enforce_nonce` on Create Message. It applies to bot (and user) tokens, because dedup is "by the same author". The window is undocumented beyond "the past few minutes".
- https://docs.discord.com/developers/resources/webhook: Execute Webhook has no nonce.
- So a bot-token channel sender gets short-window keyed dedup, and a webhook sender does not. Key form: a nonce of 25 chars or fewer derived from the delivery ID (for example a hash).

**Telegram.** https://core.telegram.org/bots/api#sendmessage (no dedup param). Test env: https://core.telegram.org/bots/webapps#using-bots-in-the-test-environment

**Standard Webhooks.** Spec: https://github.com/standard-webhooks/standard-webhooks/blob/main/spec/standard-webhooks.md
- Headers: `webhook-id`, `webhook-timestamp` (unix seconds), and `webhook-signature`. The signature header holds a space-delimited list, which allows key rotation.
- The signed content is `id.timestamp.payload`. Symmetric signatures are `v1` (HMAC-SHA256, 24 to 64 byte secret). Asymmetric signatures are `v1a` (ed25519).
- The receiver should check timestamp tolerance and use `webhook-id` as an idempotency key (the spec's example is "save the IDs in redis for 5 minutes").
- Recommended retry schedule: immediately, 5 s, 5 min, 30 min, 2 h, 5 h, 10 h, 14 h, 20 h, 24 h (about 3 days in total). Recommended timeout is 15 to 30 s. Only a 2xx counts as success. A 410 should disable the endpoint. Honour `retry-after`.
- Libraries list: https://github.com/standard-webhooks/standard-webhooks (README lists `standardwebhooks` on crates.io). **Inference:** the Rust lib depends only on `hmac-sha256`, so it likely covers `v1` and not `v1a`.

## Part 2: first-party destinations of competitors

"First-party" here means the vendor ships and documents the destination itself. Entries marked "via X" go through a third-party automation platform.

### Apify
Integrations index: https://docs.apify.com/integrations (full list in https://docs.apify.com/llms.txt).
- **Programming:** REST API, Webhooks (ad-hoc and actions), GitHub (it builds Actors and opens issues on failure).
- **Data and storage:** Google Drive, Airtable (Airtable Data Import Actor), Snowflake, Keboola, Airbyte (Apify Dataset source connector), HubSpot.
- **Notifications:** Slack, Gmail. Telegram goes via Zapier.
- **Workflow platforms:** Zapier, Make, n8n, Pipedream, IFTTT, Microsoft Power Automate, Activepieces, Albato, Workato, Windmill, Kestra, viaSocket, Gumloop, Lindy, Bubble, Dify, Flowise, Langflow, Lovable.
- **Vector and AI:** Pinecone, Qdrant, Milvus, OpenAI Assistants (vector store), LangChain, LangGraph, LlamaIndex, Haystack, CrewAI, Agno, Mastra, Vercel AI SDK, Google ADK, OpenAI Agents SDK, Amazon Bedrock, Strands, Upsonic.
- **Agents:** the MCP server, plus Claude, ChatGPT/Codex, Cursor, Copilot and others.
- Asana, which was named in the brief, is not in the current index.
- Dataset export formats are JSON, JSONL, CSV, XML, Excel (xlsx), HTML table and RSS: https://docs.apify.com/platform/storage/dataset
- Webhook delivery: header `X-Apify-Webhook-Dispatch-Id`. Up to 11 retries with exponential backoff (about 1 min up to about 32 h). The docs say that "in rare cases, the webhook might be invoked more than once", so receivers should be idempotent: https://docs.apify.com/integrations/webhooks/actions.md

### Scrapy (2.19.0, 2026-09-10)
- `FEED_STORAGES_BASE` in `scrapy/settings/default_settings.py` at tag 2.19.0 maps these schemes:
  - `file` (and bare paths), `ftp`, `ftps`
  - `gs` (GCS)
  - `s3`
  - `stdout`
- Feed exporters are json, jsonlines/jsonl/jl, csv, xml, marshal and pickle.
- Media pipelines (FilesPipeline and ImagesPipeline) store to `file`, `s3`, `gs` and `ftp`. Source: `scrapy/pipelines/files.py` `STORE_SCHEMES`.
- Item pipelines (`ITEM_PIPELINES`) are user code. None that write to a sink ships enabled.
- The scrapy-plugins org has these sink plugins:
  - Feed storages: scrapy-feedexporter-azure-storage, -sftp, -google-drive, -google-sheets, -dropbox, -onedrive.
  - scrapy-feedexport-warc.
  - scrapy-snowflake-stage-exporter.
  - scrapy-bigml and scrapy-djangoitem (pipelines).
  - scrapy-dotpersistence (syncs `.scrapy` to S3).
  - scrapy-pagestorage.
  - The rest of the org is downloaders and middlewares (scrapy-zyte-api, scrapy-playwright, scrapy-splash, etc.).
  - Source: https://github.com/orgs/scrapy-plugins/repositories

### Spider Cloud
- `data_connectors` streams crawl results to S3, GCS, Google Sheets, Azure Blob and Supabase (table). It has `on_find` and `on_find_metadata` flags. Source: https://spider.cloud/docs/api/
- `webhooks` takes a `destination` and events `on_credits_depleted`, `on_credits_half_depleted`, `on_website_status`, `on_find` and `on_find_metadata`. No signing or retry guarantees are documented.
- Also listed: n8n, Zapier, LangChain, LlamaIndex, CrewAI, AutoGen, Agno, Julep, Flowise, Langflow, Dify, and an MCP server. Source: https://spider.cloud/integrations/

### Bright Data
- `deliver_types` (Marketplace dataset API) are `api_pull`, `webhook`, `email`, `gcs`, `gcp_pubsub`, `s3`, `snowflake`, `ali_oss`, `sftp` and `azure`. File extensions are json, csv, xlsx, ndjson, parquet and jsonl. Source: https://docs.brightdata.com/api-reference/marketplace-dataset-api/get-delivery-options
- Scraper API delivery: webhook, S3 (assumed IAM role or keys), GCS, Snowflake, Pub/Sub, Azure Storage, SFTP/FTP, and API download. Streamed batches of 10 to 100,000 lines are supported. Source: https://docs.brightdata.com/products/scrapers/scrapers-library/data-delivery
- Webhook delivery details:
  - `auth_header` sets the `Authorization` header, and a `dca-collection-id` header carries the snapshot ID. There is no HMAC signing.
  - On failure "Bright Data retries delivery". No schedule is given.
  - Snapshots stay downloadable for 30 days.
  - The receiver must answer 200 within 30 s.
- Scraper Studio also lists OCI PAR: https://docs.brightdata.com/products/scraper-studio/initiate-collection-and-delivery-options

### Zyte
- Scrapy Cloud stores items itself. They download as CSV, JSON, JSON Lines or XML, from the UI or `storage.zyte.com/items/<job>`, plus the Items API. Source: https://docs.zyte.com/scrapy-cloud/usage/items/download.html
- Export guides via Scrapy FEEDS: Amazon S3, Azure Storage, Dropbox, FTP, Google Cloud Storage, Google Drive, Google Sheets and SFTP (the plugin-based ones use scrapy-plugins). Item storage goes to Google BigQuery. Source: https://docs.zyte.com/web-scraping/guides/export/index.html
- Managed "Zyte Data" delivers via S3, SFTP, API or direct warehouse, in JSONL, Parquet or CSV. These are marketing pages, so **inference** for exact scope: https://www.zyte.com/data-types/web-data-for-ai/

### Firecrawl (v2.11.0, AGPL-3.0)
- Webhooks cover crawl, batch, extract, agent and monitor events. The receiver must answer 2xx within 10 s. Retries come at 1, 5 and 15 min, then the delivery is marked failed. No event ID or idempotency is documented. Source: https://docs.firecrawl.dev/webhooks/overview
- Signing: `X-Firecrawl-Signature: sha256=<HMAC-SHA256 of raw body>`. It is not Standard Webhooks. Source: https://docs.firecrawl.dev/webhooks/security
- Integrations: Zapier, Make, n8n, Dify, LangChain, LangGraph, LlamaIndex, Mastra, OpenAI, Gemini, Vercel AI SDK, Lovable, and about 35 MCP clients. There are no storage sinks. Source: https://www.firecrawl.dev/integrations

### Crawl4AI (v0.9.4, Apache-2.0)
- The library returns `CrawlResult` objects and has no storage sinks.
- The Docker server's `/crawl/job` and `/llm/job` take a `webhook_config` (`webhook_url`, `webhook_data_in_payload`, `webhook_headers`). It makes 5 attempts with exponential backoff and has no signing. The docs tell handlers to be idempotent. Sources: https://docs.crawl4ai.com/core/self-hosting/ and https://docs.crawl4ai.com/blog/releases/0.7.6/

### Browse AI
- Integrations: Google Sheets, Airtable, Amazon S3, Webhooks, API, and Zapier, Make, Pabbly and Integrately. Excel and JSON are export formats. ChatGPT and Claude go via Zapier. Source: https://www.browse.ai/integrations
- Snowflake and PostgreSQL appear only in search snippets of other Browse AI pages, not on the integrations page (**inference**: unconfirmed).

### Octoparse
- Cloud export: files (CSV, Excel, JSON, HTML, XML), databases (MySQL, SQL Server, PostgreSQL, Oracle), Google Sheets, cloud storage (Google Drive, Dropbox, Amazon S3; Professional and Enterprise plans only), and OpenAPI with a non-exported-data cursor ("Mark Data as Exported"). Source: https://www.octoparse.com/docs/en/platform/automate-cloud-data-exports.md
- Zapier is listed for cloud data in the older help center: https://helpcenter.octoparse.com/en/articles/6470971-in-what-format-can-i-download-the-extracted-data

### Airbyte destinations
- The OSS registry has 54 destinations: https://connectors.airbyte.com/files/registries/v0/oss_registry.json
- Relevant ones, with support level:
  - S3 (certified, GA), S3 Data Lake (certified), S3 Glue (community).
  - Azure Blob Storage (certified, GA).
  - GCS (community, beta) and GCS Data Lake (certified, GA).
  - SFTP-JSON, Kafka, Google PubSub, RabbitMQ, Redis and Google Sheets (all community, alpha).
  - Pinecone (certified, beta).
  - Also Qdrant, Milvus, Weaviate, Chroma, PGVector, Typesense, Vectara, Astra DB, ElasticSearch, MongoDB, DynamoDB, Firestore, Postgres, MySQL, MS SQL, Oracle, BigQuery, Snowflake, Redshift, ClickHouse, Databricks, DuckDB/MotherDuck, Local CSV/JSON/SQLite, HubSpot, Salesforce and Customer IO.
- Kinesis exists in the repo (`destination-kinesis`) but is `supportLevel: archived` and disabled in both registries.
- The Cloud registry has 43 destinations. It drops Kafka, RabbitMQ, the local ones, and some others.

### dlt (1.31.0, Apache-2.0)
- Destinations at tag 1.31.0: postgres, snowflake, filesystem, duckdb, ducklake, dummy, mssql, fabric, bigquery, athena, redshift, qdrant, lancedb, lance, motherduck, weaviate, synapse, databricks, dremio, clickhouse, sqlalchemy, and `destination` (the custom sink decorator). Docs also cover Iceberg/Delta and Hugging Face. Source: `dlt/destinations/__init__.py`
- `filesystem` (fsspec) supports `s3://` (including R2, MinIO and Tigris), `gs://`, `az://` and `abfss://` (Azure and OneLake), `sftp://`, `file://` and `hf://`. Formats are jsonl (default), parquet and csv. A load is marked complete with a `_dlt_loads` marker file.
- `@dlt.destination` is a custom-sink decorator with these delivery semantics:
  - After a failure, the sink restarts from the whole failed batch, so "you can still get duplicated data".
  - Making each batch atomic is the user's job. It retries 5 times. `DestinationTerminalException` stops retries.
  - So this is an at-least-once, batch-granular sink. Source: https://github.com/dlt-hub/dlt/blob/1.31.0/docs/website/docs/dlt-ecosystem/destinations/destination.md

## Takeaways for Automan
1. Only Resend (24 h) and Discord bot messages (`enforce_nonce`, a few minutes) give keyed dedup. Every other notification channel is at-least-once, and none is fenced. So for these sinks, exactly-once per delivery ID has to come from Automan's own outbox and ledger. The receiver alone cannot provide it.
2. Teams Incoming Webhooks ended in May 2026. A Teams sink should target the Workflows webhook URL (plain HTTP POST, Adaptive Card). Graph app-only sends are blocked outside migration.
3. Competitors' webhooks are inconsistent: Apify has a dispatch-ID header, Bright Data an auth header, Firecrawl its own HMAC, and Crawl4AI and Spider no signing at all. None of them uses the Standard Webhooks headers. Shipping Standard Webhooks (`webhook-id` = delivery ID) is a differentiator. **Inference**, from the docs read here.
4. For CI: use Mailpit for SMTP, LocalStack for SES (v2 coverage is thin, 14/116 ops), and `wiremock` or `httpmock` for Slack, Teams, Discord, Telegram, Resend and webhooks. Postmark (`POSTMARK_API_TEST`) and Telegram (`/test/` env) offer vendor test modes.
