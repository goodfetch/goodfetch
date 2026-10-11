# What users complain about when scraped data leaves the tool (2024–2026)

Survey of October 11, 2026. 65 distinct dated public signals plus 4 undated ones, coded by pain. Sources: GitHub issues and pull requests (Scrapy, Crawlee, Apify clients and integrations, Firecrawl, Crawl4AI, Airbyte, n8n), the Make and n8n community forums, Trustpilot and G2 excerpts, and three Reddit threads. Reddit (403 and rate limits), Capterra (403) and the Apify Store issue tabs (rendered client-side) were mostly unreachable, so reviews and Reddit are thin. Several Airbyte and n8n signals concern generic connectors (Sheets, Postgres, BigQuery) that scraper pipelines use. Vendor help pages are counted as weak evidence: they exist because the pain is common.

Codes: DUP duplicates · LOST lost data · SCHEMA field or header breaks · QUOTA quota or rate limit · SILENT silent failure · AUTH credential expiry or webhook auth · LIMIT row, cell or column limits · ORDER ordering · LARGE large files, streaming, timeouts · PARTIAL partial data or files · REPLAY no backfill or redelivery · COST · OTHER (named).

## Signals

| # | Date | Source | Paraphrase | Codes |
| --- | --- | --- | --- | --- |
| 1 | 2026-07-11 | [scrapy#7730](https://github.com/scrapy/scrapy/issues/7730) | Batched S3 feeds upload only at spider close, not when a batch fills | LARGE, late delivery |
| 2 | 2026-08-11 | [scrapy#7978](https://github.com/scrapy/scrapy/issues/7978) | A crash or double Ctrl-C loses queued requests and exported items; asks for "re-run loses no data" | LOST, PARTIAL, REPLAY |
| 3 | 2026-07-30 | [scrapy#7822](https://github.com/scrapy/scrapy/issues/7822) | Asks for append on S3 feeds | append vs overwrite |
| 4 | 2026-09-17 | [scrapy PR#8173](https://github.com/scrapy/scrapy/pull/8173) | Append to GCS feeds through composite objects | append vs overwrite |
| 5 | 2026-06-12 | [scrapy#7607](https://github.com/scrapy/scrapy/issues/7607) | Three S3 components with different settings and credentials | AUTH, config |
| 6 | 2026-06-27 | [scrapy PR#7688](https://github.com/scrapy/scrapy/pull/7688) | S3 credentials expire before the export stage runs | AUTH, LOST |
| 7 | 2026-06-22 | [scrapy PR#7650](https://github.com/scrapy/scrapy/pull/7650) | Restarting with JOBDIR overwrites earlier batch files | LOST, overwrite |
| 8 | 2026-06-23 | [scrapy PR#7651](https://github.com/scrapy/scrapy/pull/7651) | "Fix silent data loss": CSV kept only the first item's fields | SCHEMA, SILENT, LOST |
| 9 | 2026-09-28 | [scrapy PR#8242](https://github.com/scrapy/scrapy/pull/8242) | FEED_EXPORT_FIELDS silently dropped (name, output) pairs | SCHEMA, SILENT |
| 10 | 2026-09-22 | [scrapy PR#8205](https://github.com/scrapy/scrapy/pull/8205) | One bad URI placeholder broke every feed | LOST |
| 11 | 2026-08-06 | [scrapy PR#7902](https://github.com/scrapy/scrapy/pull/7902) | Feeds configured but exporter disabled gave no output and no error | SILENT |
| 12 | 2024-03-11 | [scrapy#6289](https://github.com/scrapy/scrapy/issues/6289) | GZip post-processing fails on S3 feeds | LARGE, compression |
| 13 | 2024-07-02 | [scrapy#6425](https://github.com/scrapy/scrapy/issues/6425) | FeedExporter throws on Path objects with URI parameters | config |
| 14 | 2025-04-26 | [crawlee#2945](https://github.com/apify/crawlee/issues/2945) | `exportToCSV` keeps only the first item's fields | SCHEMA, LOST |
| 15 | 2024-10-18 | [crawlee#2718](https://github.com/apify/crawlee/issues/2718) | CSV columns follow object key order and get mixed | SCHEMA, ORDER |
| 16 | 2026-08-24 | [crawlee#4065](https://github.com/apify/crawlee/issues/4065) | Transactional `pushData` failures surface at commit, uncatchable | SILENT, LOST |
| 17 | 2026-10-07 | [apify-client-js#1117](https://github.com/apify/apify-client-js/issues/1117) | Paginated `listItems` stops at the first page's total while the dataset grows | PARTIAL, SILENT |
| 18 | 2026-08-03 | [apify-client-python#990](https://github.com/apify/apify-client-python/issues/990) | Client retries non-idempotent POSTs on 429/5xx | DUP |
| 19 | 2024-01-24 | [apify-sdk-js#275](https://github.com/apify/apify-sdk-js/issues/275) | Users ask for auth headers on webhooks | AUTH |
| 20 | 2026-04-02, 2026-09-22 | [apify-zapier#139](https://github.com/apify/apify-zapier-integration/issues/139), [#175](https://github.com/apify/apify-zapier-integration/issues/175) | Zapier "Finished Task Run" trigger times out loading records | LARGE, timeout |
| 21 | 2026-03-17 | [apify-zapier#134](https://github.com/apify/apify-zapier-integration/issues/134) | Errors do not say whether to retry, go async or give up | SILENT |
| 22 | 2026-01-13 | [airtable-data-import-actor#2](https://github.com/apify/airtable-data-import-actor/issues/2) | Confusing dataset wiring when used as a webhook integration | config UX |
| 23 | 2026-10-08 | [apify-client-js#1127](https://github.com/apify/apify-client-js/issues/1127) | Asks for streamed dataset export | LARGE |
| 24 | 2026-09-16 | [crawlee-python#2240](https://github.com/apify/crawlee-python/issues/2240) | Streaming uploads and downloads across storage clients | LARGE |
| 25 | 2026-09-27 | [firecrawl#4774](https://github.com/firecrawl/firecrawl/issues/4774) | Receiver down at the final webhook: job succeeds, pipeline never starts; asks to inspect and redeliver | REPLAY, LOST |
| 26 | 2026-08-03 | [firecrawl#4212](https://github.com/firecrawl/firecrawl/issues/4212) | Python SDK returns failed or cancelled jobs as successes | SILENT |
| 27 | 2026-05-26 | [firecrawl#3614](https://github.com/firecrawl/firecrawl/issues/3614) | Cancelling floods `crawl.failed` for every queued URL | misleading events |
| 28 | 2025-12-18 | [firecrawl#2576](https://github.com/firecrawl/firecrawl/issues/2576) | Per-page failures in a batch emit no webhook | SILENT, PARTIAL |
| 29 | 2024-10-23 | [firecrawl#813](https://github.com/firecrawl/firecrawl/issues/813) | No documented way to verify a webhook's origin | AUTH |
| 30 | 2025-09-28 | [firecrawl#2222](https://github.com/firecrawl/firecrawl/issues/2222) | Self-hosted webhook to n8n never fires | SILENT, LOST |
| 31 | 2025-03-29 | [firecrawl#1388](https://github.com/firecrawl/firecrawl/issues/1388) | Batch stalls after "Calling webhook…" despite a 200 | LOST, stall |
| 32 | 2025-09-23 | [firecrawl#2194](https://github.com/firecrawl/firecrawl/issues/2194) | Webhooks to internal Docker addresses rejected by a private-IP check | network policy |
| 33 | 2026-10-05 | [firecrawl#4942](https://github.com/firecrawl/firecrawl/issues/4942) | Appending URLs to a finished batch reports success, nothing runs | SILENT |
| 34 | 2025-11-14 | [crawl4ai#1616](https://github.com/unclecode/crawl4ai/issues/1616) | Webhook job API drops crawler and browser config | config |
| 35 | 2024-07-16 | [airbyte#41977](https://github.com/airbytehq/airbyte/issues/41977) | Sheets destination appends empty rows to the 10M-cell limit | LIMIT |
| 36 | 2024-07-31 | [airbyte#42904](https://github.com/airbytehq/airbyte/issues/42904) | Append + Deduped on Sheets hits a 100,000-request batch error | QUOTA, DUP |
| 37 | 2025-03-31 | [airbyte#56915](https://github.com/airbytehq/airbyte/issues/56915) | Every Sheets sync broke after a connector update | regression |
| 38 | 2025-04-02 | [airbyte#56964](https://github.com/airbytehq/airbyte/issues/56964) | Long-running BigQuery→Sheets sync starts failing | regression |
| 39 | 2025-07-03 | [airbyte#62501](https://github.com/airbytehq/airbyte/issues/62501) | Raw tables hold up to 7x duplicates | DUP |
| 40 | 2024-09-09 | [airbyte#45345](https://github.com/airbytehq/airbyte/issues/45345) | Postgres destination name collisions fail the sync | SCHEMA |
| 41 | 2026-03-12 | [airbyte#74802](https://github.com/airbytehq/airbyte/issues/74802) | A 4.5M-row BigQuery stream never gets a final status | LARGE, SILENT |
| 42 | 2026-06-23 | [n8n#32869](https://github.com/n8n-io/n8n/issues/32869) | Sheets append hits the per-project write quota | QUOTA |
| 43 | 2025-06-26 | [n8n#16750](https://github.com/n8n-io/n8n/issues/16750) | Sheets read 429s after an upgrade | QUOTA, regression |
| 44 | 2025-07-08 | [n8n#17120](https://github.com/n8n-io/n8n/issues/17120) | Sheets rate limits at about one request a minute | QUOTA |
| 45 | 2025-08-11 | [n8n#18233](https://github.com/n8n-io/n8n/issues/18233) | Quota errors only on scheduled runs | QUOTA |
| 46 | 2026-08-26 | [n8n#37094](https://github.com/n8n-io/n8n/issues/37094) | On timeout the Sheets node fires both success and error outputs | ambiguous outcome |
| 47 | 2026-09-18 | [n8n#39011](https://github.com/n8n-io/n8n/issues/39011) | Lookup returns only the first match, breaking lookup-then-insert dedup | SILENT, PARTIAL |
| 48 | 2025-02-25 | [n8n#13487](https://github.com/n8n-io/n8n/issues/13487) | Duplicate sheet headers silently lose data | SCHEMA, LOST |
| 49 | 2024-02-13 | [n8n community 40181](https://community.n8n.io/t/quota-overflow-problem-with-google-sheets-api-in-n8n/40181) | Sheets 429 despite pauses; user plans to leave Sheets | QUOTA |
| 50 | 2025-03-23 | [n8n community 91565](https://community.n8n.io/t/how-to-avoid-scraping-duplicate-reddit-posts-in-n8n-automation/91565) | Reddit scrape into Sheets needs dedup by URL | DUP |
| 51 | 2026-01-12 | [n8n community 248760](https://community.n8n.io/t/avoiding-duplicate-job-records-in-n8n-when-scraping-and-storing-data-in-google-sheets-crm/248760) | Rolling job scrape re-inserts the same tracking ID | DUP |
| 52 | 2024-12-23 | [Make community 64655](https://community.make.com/t/how-to-prevent-duplicate-data-in-google-sheets-when-scraping-multiple-times-daily/64655) | Sitemap scraped 3x a day duplicates URLs in Sheets | DUP |
| 53 | 2025-01-10 | [Make community 65968](https://community.make.com/t/issue-with-filtering-duplicates/65968) | Apify leads into Sheets: hand-built lookup lets duplicates in; no upsert | DUP |
| 54 | 2026-01-06 | [Make community 100240](https://community.make.com/t/google-sheet-is-not-updating/100240) | Green scenario, no rows written (renamed tab, columns, re-auth) | SILENT, SCHEMA, AUTH |
| 55 | 2024-10-21 | [Make community 58456](https://community.make.com/t/scraping-timeout-roadblock/58456) | Apify crawler hits Make's 120 s timeout; workaround is a webhook | LARGE, timeout |
| 56 | ~2026-03 | [r/n8n](https://www.reddit.com/r/n8n/comments/1ruqrok/how_do_you_handle_this_in_n8n_without_creating) | Retries create duplicates; advice: unique key and `ON CONFLICT` | DUP |
| 57 | ~2025-09 | [r/webscraping](https://www.reddit.com/r/webscraping/comments/1nkxx1r/how_do_you_clean_largescale_scraped_data) | Millions of records: duplicates, gaps, slow loads into Postgres | DUP, LARGE |
| 58 | ~2025 | [r/n8n](https://www.reddit.com/r/n8n/comments/1nzd8g4/tried_scraping_36000_pages_with_n8n_heres_what) | 36K pages into Sheets logged half-empty rows unnoticed | PARTIAL, SILENT |
| 59 | 2025-08-30 | [Trustpilot Browse AI](https://www.trustpilot.com/review/browse.ai) | Successful runs captured only part of the data | PARTIAL |
| 60 | 2024-05-01 | [Trustpilot Browse AI](https://www.trustpilot.com/review/browse.ai) | Exported fields cannot be adjusted; CSV fixed by hand | no transform |
| 61 | 2026-03-24 | [Trustpilot Octoparse](https://www.trustpilot.com/review/octoparse.com) | Credits spent on duplicates in large projects | COST, DUP |
| 62 | 2026-03-27 | [Trustpilot Octoparse](https://www.trustpilot.com/review/octoparse.com) | "Rarely get all the data I need" | PARTIAL |
| 63 | 2025-05-15 | [G2 Browse AI](https://www.g2.com/products/browse-ai/reviews) | Cost climbs at volume | COST |
| 64 | 2025-02-21 | [Browse AI help](https://help.browse.ai/en/articles/10524274-how-to-fix-google-sheets-add-on-errors) | A dedicated page for Sheets add-on errors (vendor doc) | SILENT |
| 65 | 2026-03-13 | [Browse AI help](https://help.browse.ai/en/articles/10524277-how-to-sync-only-new-or-updated-data-to-google-sheets) | "Only sync changes" added against duplicates; default appends all (vendor doc) | DUP |

Undated, supporting only: Octoparse's Sheets export stops at 1,048,576 rows and sends only unexported records ([docs](https://www.octoparse.com/docs/en/platform/google-sheets)); Octoparse's free plan caps exports at 10K ([Software Advice](https://www.softwareadvice.com/bi/octoparse-profile/reviews/)); third-party Apify actors sell upsert and dedup for Sheets ([1](https://apify.com/ayf/gdriveintegration), [2](https://apify.com/zentrafoundry/apify-dataset-to-google-sheets-sync)); Browse AI exports only recent runs ([review](https://www.hackceleration.com/labs/review/browse-ai)).

## Ranking

| Rank | Pain | Signals |
| --- | --- | --- |
| 1 | SILENT: a green run with no or wrong data | 15 |
| 2 | DUP: duplicates, mostly on recurring runs | 13 |
| 3 | LOST | 11 |
| 4 | LARGE: end-only upload, streaming, timeouts | 9 |
| 5 | PARTIAL | 8 |
| 6 | SCHEMA: first-item fields, key order, duplicate headers, collisions | 8 |
| 7 | QUOTA, nearly all Google Sheets | 7 |
| 8 | AUTH | 5 |
| 9 | Regression after a connector update; append vs overwrite; hand-off timeouts | 3 each |
| 12 | REPLAY; COST | 2 each (+1 undated) |
| 14 | Misleading or ambiguous events | 2 |
| 15 | LIMIT | 1 (+2 undated) |
| 16 | ORDER | 1 |

## What users ask for

- Upsert or skip by a stable key (URL, product or tracking ID). It is the most common ask; vendors answer with "only sync changes" (Browse AI), "unexported records only" (Octoparse) and paid third-party Apify writers.
- Explicit append, overwrite and replace modes per destination.
- Visible per-item and per-page failures, typed errors instead of empty successes, unambiguous outcomes, errors that say what to do.
- Automatic retry for short outages, plus inspecting and redelivering a failed final webhook; idempotent retries; crash-safe re-runs; backfill from history.
- Delivery as the run goes, streaming for large datasets, asynchronous hand-off to automation tools.
- A stable schema: the union of fields, column order independent of key order, safe duplicate headers, no name collisions.
- Quota-aware writers with batching and backoff, and a warning before Sheets' and XLSX's size ceilings.
- Credentials resolved at upload time, signed webhooks, custom headers, a clear re-authorization signal.
