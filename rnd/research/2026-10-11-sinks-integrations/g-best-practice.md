# Delivery best practice across 15 data-movement products

Survey of October 11, 2026, from vendor docs, source code and specifications: Fivetran (and Census, now Fivetran Activations), Segment, RudderStack, Hightouch, Airbyte, dlt, Singer SDK, Kafka Connect, Vector, Svix, Hookdeck, Stripe, Standard Webhooks, plus the destination limits of Google Sheets, Airtable, Notion, BigQuery and Snowflake. Segment's site blocks fetching, so its official `segmentio/segment-docs` repository was read instead. A few points come from search-engine summaries of official pages and are marked so. Vector databases were not covered here (see [d-vector.md](d-vector.md)).

Corrections found on the way: Google now allows up to 20 million cells or 100 MB per spreadsheet ([Drive help](https://support.google.com/drive/answer/37603)), while its API error still says 10,000,000 ([Hightouch](https://hightouch.com/docs/destinations/google-sheets)). Notion publishes per-connection limits: 600 requests a minute on Business and Enterprise, 180 elsewhere, plus a workspace limit ([Notion](https://developers.notion.com/reference/request-limits)).

## 1. Schema evolution and type conflicts

- **Fivetran** promotes a column to the most specific type that losslessly holds old and new values (INT and DOUBLE give DOUBLE; DOUBLE and BOOLEAN give STRING), by temporary column, copy, drop and rename; narrowing never happens; deleted source columns stay and receive NULL; renames are a new column plus a deleted one; new objects follow `ALLOW_ALL`, `ALLOW_COLUMNS` or `BLOCK_ALL` ([core concepts](https://fivetran.com/docs/core-concepts), [schema config API](https://fivetran.com/docs/rest-api/interactive-api-reference/connector-schema/modify-connector-schema-config)).
- **Airbyte Destinations V2** never fails a sync on content: a bad value is nulled in the final table, kept raw in `_airbyte_data`, and recorded in `_airbyte_meta.changes` as `{field, change: NULLED|TRUNCATED, reason}` ([typing and deduping](https://docs.airbyte.com/platform/using-airbyte/core-concepts/typing-deduping), [protocol](https://github.com/airbytehq/airbyte-protocol/blob/main/protocol-models/src/main/resources/airbyte_protocol/v0/airbyte_protocol.yaml)). Direct-load tables use `ALTER TABLE` and fail when history cannot be cast ([direct load](https://docs.airbyte.com/platform/using-airbyte/core-concepts/direct-load-tables)). Schema changes can propagate, wait for approval or pause the connection; breaking changes always pause ([schema change management](https://docs.airbyte.com/platform/using-airbyte/schema-change-management)).
- **dlt** schema contracts apply to tables, columns and data types, each `evolve`, `freeze`, `discard_row` or `discard_value`; a violation raises `DataValidationError` with the table, column, entity, mode and item; a value that cannot be coerced goes to a variant column `<name>__v_<type>`; removed fields stop loading, columns are never dropped ([schema contracts](https://dlthub.com/docs/general-usage/schema-contracts), [schema](https://dlthub.com/docs/general-usage/schema), [evolution](https://dlthub.com/docs/general-usage/schema-evolution)).
- **RudderStack** fixes a column's type from the first event, stringifies or casts conflicts, and nulls what cannot be cast while copying it to a `rudder_discards` table ([warehouse schema](https://www.rudderstack.com/docs/destinations/warehouse-destinations/warehouse-schema/)).

## 2. Per-row errors, dead letters, reporting

- **Hightouch**: rejected rows give "Completed with errors", distinct from "Failed"; they are retried every run by primary key until they succeed; 400/401/403/422 are not retried within a run; a rejected batch is split in half recursively to isolate bad rows ([retries](https://hightouch.com/docs/syncs/retries)).
- **Census**: invalid rows (NULL or duplicate identifier) are filtered before sending; rejected rows are retried by primary key; split-batch retry; sync history shows sample records with reasons ([retry handling](https://fivetran.com/docs/activations/syncs/retry-handling), [sync tracking](https://fivetran.com/docs/activations/syncs/sync-monitoring/sync-tracking)).
- **Segment Functions**: typed errors, only `RetryError` and timeouts retried; `onBatch` returns a per-event status so only failed events retry; one 429 marks a whole batch ([destination functions](https://github.com/segmentio/segment-docs/blob/develop/src/connections/functions/destination-functions.md)). Delivery Overview shows received, filtered, retried, failed and delivered, with a discard table ([delivery overview](https://github.com/segmentio/segment-docs/blob/develop/src/connections/delivery-overview.md)).
- **Kafka Connect**: `errors.tolerance` `none` (fail) or `all` (skip; without a dead-letter queue, not even logged); dead-letter topics with `__connect.errors.*` context headers; sinks report per-record errors through `ErrantRecordReporter` ([configs](https://kafka.apache.org/41/configuration/kafka-connect-configs/), [KIP-610](https://cwiki.apache.org/confluence/display/KAFKA/KIP-610%3A+Error+Reporting+in+Sink+Connectors)).
- **dlt** has no dead-letter table; loads are complete only when `_dlt_loads.status = 0` ([destination tables](https://dlthub.com/docs/general-usage/destination-tables)).

## 3. Retries, auto-disable, backpressure

| Product | Retry schedule | Gives up / disables |
| --- | --- | --- |
| Standard Webhooks spec | immediately, 5 s, 5 m, 30 m, 2 h, 5 h, 10 h, 14 h, 20 h, 24 h; jitter; honour `retry-after`; 410 disables; 429/502/504 throttle; 15–30 s timeout; payloads under 20 KB | notify by another channel, then disable ([spec](https://github.com/standard-webhooks/standard-webhooks/blob/main/spec/standard-webhooks.md)) |
| Svix | immediately, 5 s, 5 m, 30 m, 2 h, 5 h, 10 h, 10 h; 2xx within 15 s; `webhook-delivery: abort-message` stops retries | endpoint disabled after 5 days of failure, with an event ([retries](https://docs.svix.com/retries)) |
| Hookdeck | linear or exponential, up to 50, by status code; `Retry-After` up to 7 days | manual pause; backpressure issue when queue time exceeds 10 min ([retries](https://hookdeck.com/docs/retries), [issue triggers](https://hookdeck.com/docs/issue-triggers)) |
| Stripe | exponential, up to 3 days | disabled after several days (search summary); resend 15–30 days ([webhooks](https://docs.stripe.com/webhooks)) |
| Segment | 4 hours, randomized exponential | — |
| RudderStack | 3-hour window, backoff 10–300 s, 4xx aborted | — |
| Fivetran | interval by sync frequency, then daily | auto-pause after 14 days ([sync overview](https://fivetran.com/docs/core-concepts/syncoverview)) |
| Airbyte | 10 s, 30 s, 90 s, 4m30s; attempt caps | warning after 4 days, disabled after 7 ([notifications](https://docs.airbyte.com/platform/cloud/managing-airbyte-cloud/manage-airbyte-cloud-notifications)) |
| Kafka Connect | `errors.retry.timeout` default 0 | — |
| Vector | 408, 429, ≥500 except 501; Fibonacci from 1 s | — |

Vector's adaptive request concurrency is an AIMD loop (additive increase, multiplicative decrease) on round-trip time and 429/503, `request.concurrency = "adaptive"`; its buffers block upstream by default and can spill to checksummed disk; end-to-end acknowledgements confirm a source only after every sink has the event ([ARC](https://vector.dev/blog/adaptive-request-concurrency/), [buffers](https://vector.dev/docs/architecture/buffering-model/), [acks](https://vector.dev/docs/architecture/end-to-end-acknowledgements/)).

## 4. Destination limits and batching

- **Google Sheets**: 300 writes a minute per project and 60 per user per project; a `batchUpdate` with its subrequests counts once and is atomic; 2 MB payload recommended; truncated exponential backoff on 429 ([limits](https://developers.google.com/workspace/sheets/api/limits)). Hightouch spills into `sheet_1`, `sheet_2` past the cell limit; Apify's Sheets actor caps at 5 million cells and fails with nothing imported ([Apify](https://apify.com/lukaskrivka/google-sheets)).
- **Airtable**: 5 requests a second per base, 10 records per request, 30 s wait after a 429; upsert by `performUpsert.fieldsToMergeOn` (1–3 fields) ([rate limits](https://airtable.com/developers/web/api/rate-limits), [update records](https://airtable.com/developers/web/api/update-multiple-records)).
- **Notion**: see the correction above; 429 with `Retry-After`; retry 5xx only for idempotent requests; 1,000 blocks and 500 KB per payload.
- **BigQuery**: 1,500 load jobs per table per day; mutating DML 2 concurrent per table with 20 queued; Storage Write API appends up to 20 MB ([quotas](https://cloud.google.com/bigquery/quotas)).
- **Snowflake**: files of about 100–250 MB compressed, or one a minute ([load considerations](https://docs.snowflake.com/en/user-guide/data-load-considerations-prepare)); Snowpipe Streaming named channels give ordered exactly-once ingestion by offset token ([overview](https://docs.snowflake.com/en/user-guide/snowpipe-streaming/data-load-snowpipe-streaming-overview)).
- **Inference**: a token bucket per destination with API-sized chunks is the common pattern.

## 5. Observability and replay

- **Hightouch**: per-row live debugger (row logs 7 days, run summaries 30); alerts on fatal errors, rejected rows, throughput, duration; one notification per status change plus a recovery notice; Slack, email, PagerDuty, webhook ([debugger](https://hightouch.com/docs/syncs/debugger), [alerting](https://hightouch.com/docs/syncs/alerting)).
- **Census**: a `sync_log` written back to the warehouse (record identifier, operation, status, message); alerts on failed syncs and on a failed-record ratio; recovery emails ([writeback](https://fivetran.com/docs/activations/syncs/sync-monitoring/warehouse-writeback), [alerts](https://fivetran.com/docs/activations/syncs/sync-monitoring/alerts)).
- **Svix**: per-message retry, "recover failed since a date", "replay missing" ([retries](https://docs.svix.com/retries)). **Stripe**: list undelivered events for 30 days ([undelivered](https://docs.stripe.com/webhooks/process-undelivered-events)). **Hookdeck**: single and bulk retry by API.
- **Fivetran**: errors email, warnings do not; escalation to support after 48 h.

## 6. System columns

Every product prefixes its own: `_fivetran_synced/_deleted/_id`, `_airbyte_raw_id/_extracted_at/_loaded_at/_generation_id/_meta`, `_dlt_id/_load_id/_valid_from/_valid_to` and the `_dlt_loads` table, Singer's `_sdc_extracted_at/_sequence/_deleted_at`, RudderStack's `received_at/uuid_ts` and `rudder_discards` ([Fivetran](https://fivetran.com/docs/core-concepts/system-columns-and-tables), [Airbyte](https://docs.airbyte.com/platform/understanding-airbyte/airbyte-metadata-fields), [dlt](https://dlthub.com/docs/general-usage/destination-tables), [Singer SDK](https://sdk.meltano.com/en/latest/implementation/record_metadata.html)).

## 7. Upsert and "same item seen again"

- **dlt** `merge` by `primary_key` or `merge_key`: `delete-insert` (default, in one transaction), `upsert`, `insert-only`, `scd2` (versions with valid-from and valid-to), `cdc`; `dedup_sort` picks the winner; `hard_delete` hint ([merge loading](https://dlthub.com/docs/general-usage/merge-loading)).
- **Hightouch** modes: update, insert, upsert, add, remove, archive, mirror, diff; deletes configured apart ([types and modes](https://hightouch.com/docs/syncs/types-and-modes)). **Census**: update or create, update only, create only, mirror, append only, delete ([syncs](https://fivetran.com/docs/activations/syncs)).
- **Airbyte** Append + Deduped keeps the latest row per key by cursor ([append deduped](https://docs.airbyte.com/platform/using-airbyte/core-concepts/sync-modes/incremental-append-deduped)).
- Scraping tools: Apify's Sheets actor has append, replace and modify with dedup by field and backs up old rows; Apify's Airtable actor skips existing IDs; Browse AI syncs only changed or new entries against previous extractions; Octoparse overwrites by primary key in databases and drops rows equal on all fields ([Browse AI](https://help.browse.ai/en/articles/10524277-how-to-sync-only-new-or-updated-data-to-google-sheets), [Octoparse](https://helpcenter.octoparse.com/en/articles/6470955-export-data-to-a-database-automatic)).

## 8. OAuth and token refresh

- Google: refresh tokens of an app in "Testing" status expire after 7 days; tokens also die after 6 months unused; at most 100 refresh tokens per account per client, the oldest silently invalidated ([Google OAuth](https://developers.google.com/identity/protocols/oauth2)).
- Airbyte refreshes tokens itself and, for single-use refresh tokens, writes the new token and expiry back to the config (`refresh_token_updater`) ([OAuth](https://docs.airbyte.com/platform/connector-development/config-based/advanced-topics/oauth)).
- Fivetran and Hightouch document a re-authorize button and per-provider fixes; Hightouch recommends service accounts for Sheets and system-user tokens for Meta's 60-day tokens.
- Standard Webhooks allows several signatures at once for secret rotation.

## Gap analysis against Automan's decisions

| Practice | Precedent | Automan |
| --- | --- | --- |
| Upsert, insert-only, replace, history by a business key | dlt, Hightouch, Census, Octoparse | Added (write modes) |
| Per-row errors never fail a batch; split-batch isolation; "completed with rejections" | Hightouch, Census, Airbyte | Added |
| Lossless widening, variant columns, schema contracts | Fivetran, dlt | Added |
| Vendor-prefixed system columns and a delivery table | all | Added (`_automan_` prefix, delivery table) |
| ~3-day retry schedule, transient codes only, 410 disables, auto-pause with notice | Standard Webhooks, Svix, Stripe, Airbyte | Added |
| Quota presets, API-sized batches, adaptive concurrency, spill-over | Vector, Hightouch, vendor limits | Added |
| Per-attempt log, pipeline view, recover-failed and replay-missing | Hightouch, Segment, Svix, Stripe | Added |
| Token write-back, pre-run auth check | Airbyte, Google's rules | Added |
| Exactly-once with epoch fencing at the destination | none in this survey (Snowpipe channels and Kafka Connect source EOS come closest) | Automan exceeds the field |
