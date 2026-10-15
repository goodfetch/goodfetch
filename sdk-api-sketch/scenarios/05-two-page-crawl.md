# 05. Crawl two page types into a Dataset

## Task statement

> Crawl https://books.toscrape.com/. From the home page, follow every category link in the side list except the top-level "Books" one. On each category page, follow its book links and its "next" page link. On each book page, record one item with the title, the price (a decimal number of pounds), the number in stock (an integer), the star rating (one to five), the page's address, and the name of the category it was reached from: that name is passed from the category page, not read again on the book page. Stay on the site. Store the items in a named dataset and export it to a JSON file when the crawl ends. Report how many pages succeeded, failed or were skipped, and why each failed or skipped page did.

## Target

`books.toscrape.com`: 50 categories in the side list, paginated category pages, 1,000 book pages (checked 2026-10-14).

## Leader

Crawlee 3.18.2 (TypeScript): `CheerioCrawler` with a router, labels and `userData`. Scrapy 2.19.0 (Python): a `Spider` with callbacks, `cb_kwargs` and feed exports. Crawlee 4.0.0-rc.1 changes are noted where they matter.

## Pains

P21 passing data between handlers, P49 the handler context, P48 failure, retry and skip, P12 politeness defaults, P04 honest counts; keeps K03 one API across fetch tiers.

## Failure points

1. A book page answers 404 or 500: retried, then failed with its cause.
2. The site slows down or answers 429: the crawl backs off instead of hammering it.
3. A field is missing on one book page.
4. The value passed from the category page is misspelled, missing or not serializable.
5. A link would lead off the site.
6. The same book is reached twice.
7. The export fails (disk full, path not writable).
8. The crawl ends with no item at all.

## Setup

- **Crawlee (TypeScript):** `npm install crawlee@3.18.2 zod@4.6.7 tsx`, then `npx tsx 05-two-page-crawl.crawlee.ts`. Cheerio fetches over HTTP, so no browser is downloaded.
- **Scrapy (Python):** `pip install scrapy==2.19.0`, then `python s05_two_page_crawl_scrapy.py`.
- **Goodfetch TypeScript:** `npm install @goodfetch/foxhound zod tsx`, then `npx tsx 05-two-page-crawl.ts`.
- **Goodfetch Python:** `pip install goodfetch-foxhound pydantic`, then `python -m scenarios.s05_two_page_crawl`.
- **Goodfetch Rust:** `cargo add goodfetch-foxhound schemars serde tokio -F serde/derive,tokio/macros,tokio/rt-multi-thread`, then `cargo run --example s05_two_page_crawl`.

Foxhound's executable comes with each package. A pinned browser is fetched on first use only if a request escalates to the browser tier (ADRs 0024, 0057), and these static pages do not escalate.

Crawlee 4.0.0-rc.1, checked in its published `@crawlee/basic`, changes three points that matter here. A handler's storage writes commit only when the request succeeds (`transactionalStorage`, on by default), so a retried book is pushed once. A 429 is a rate limit of the domain (`RequestThrottledError`), paced by a backoff instead of retiring the session. `respectRobotsTxtFile` is still off by default.

## Measures

| Program | Concepts | Lines | Setup | Failure points: typed / generic / silent |
|---|---|---|---|---|
| Crawlee (TypeScript) | 31 | 69 | 1 command | 3 / 2 / 3 |
| Scrapy (Python) | 36 | 91 | 1 command | 0 / 5 / 3 |
| Goodfetch TypeScript | 45 | 69 | 1 command | 8 / 0 / 0 |
| Goodfetch Python | 48 | 96 | 1 command | 8 / 0 / 0 |
| Goodfetch Rust | 55 | 158 | 1 command | 8 / 0 / 0 |

Lines were counted with the brief's `measure_lines.py` (oxfmt 0.73.0, ruff 0.17.0, rustfmt). Every program reports the same things: the counts and why each failed or skipped page did. Each one exports the Dataset when the crawl ends and is written so scenarios 07 and 10 can import its crawl, which runs only as the entry point. The Scrapy program uses the politeness that `scrapy startproject` writes into a project's settings (robots.txt, one request per domain, a 1 s delay), its best documented configuration. Crawlee's CLI template sets none, so the Crawlee program keeps Crawlee's defaults.

**Concepts counted.**

- **Crawlee:** `CheerioCrawler`, `Dataset`, `createCheerioRouter`, its route-schema map (label → schema), `addDefaultHandler`, `addHandler`, `$`, `toArray`, `attr`, `text`, `find`, `hasClass`, `request`, `loadedUrl`, `url`, `userData`, `addRequests`, `label`, `enqueueLinks`, `selector`, `pushData` (with its dataset-name argument), `requestHandler`, `failedRequestHandler`, `onSkippedRequest`, its `reason`, `run`, `requestsFinished`, `requestsFailed`, `Dataset.open`, `exportToJSON`, and `run()` resolving only at the crawl's end.
- **Scrapy:** `Spider`, `CrawlerProcess`, `IgnoreRequest`, `HttpError`, Twisted's `Failure`, `name`, `allowed_domains`, `start_urls`, `custom_settings`, `ROBOTSTXT_OBEY`, `CONCURRENT_REQUESTS_PER_DOMAIN`, `DOWNLOAD_DELAY`, `FEEDS`, `format`, `"json"`, `encoding`, `overwrite`, `parse`, `css`, `::text`, `get`, `follow`, `cb_kwargs`, `errback`, `follow_all`, `callback`, `::attr()`, `getall`, `response.url`, `failure.request`, `failure.value`, `response.status`, `closed`, `crawl`, `start`, and `start()` blocking on the reactor until the crawl ends.
- **Goodfetch TypeScript:** `Crawl`, `Foxhound`, `seed`, `dataset`, `schema`, `key`, `label`, `payload`, `handle`, `page`, `enqueueLinks`, the link's `text`, `request`, `request.label`, `push`, `skip`, `locator`, `textContent`, `getAttribute`, `url`, `connect`, `startCrawl`, `seeds`, `finish`, `export`, `format`, `"json"`, `saveAs`, `records`, `outcomes`, `kinds`, `"failed"`, `"skipped"`, `"blocked"`, `kind`, `error`, `code`, `reason`, `message`, `blockCause`, `requests`, `succeeded`, `nothingDone`, plus two obligations: `await using` disposal, and `finish()`, without which leaving the scope stops the Run.
- **Goodfetch Python:** `AsyncCrawl`, `AsyncCrawlContext`, `AsyncFoxhound`, `CrawlRequestBlocked`, `CrawlRequestFailed`, `CrawlRequestSkipped`, `seed`, `dataset`, `schema`, `key`, `label`, `payload`, `handle`, `page`, `enqueue_links`, the link's `text`, `request`, `request.label`, `push`, `skip`, `locator`, `text_content`, `get_attribute`, `url`, `connect`, `start_crawl`, `seeds`, `finish`, `export`, `format`, `"json"`, `save_as`, `records`, `outcomes`, `kinds`, `"failed"`, `"skipped"`, `"blocked"`, `error`, `code`, `reason`, `message`, `block_cause`, `requests`, `succeeded`, `nothing_done`, plus two obligations: `async with`, and `finish()`.
- **Goodfetch Rust:** `ConnectOptions`, `Crawl`, `CrawlContext`, `CrawlRequestOutcomeKind`, `Dataset`, `DatasetDeclaration`, `Error`, `ExportFormat`, `Foxhound`, `Label`, `LinkInfo`, `OutcomeKind`, `OutcomesFilter`, `Seed`, `new`, `dataset`, `key`, `label`, `handle`, `page`, `enqueue_links`, `enqueue_links_with`, `request`, `payload`, `skip`, `push`, the link's `text`, `locator`, `text_content`, `get_attribute`, `url`, `scope`, `start_crawl`, `serve`, `export`, `Json`, `save_as`, `records`, `outcomes`, `kinds`, `Failed`, `Skipped`, `Blocked`, `default`, `next`, `kind`, `code`, `message`, `reason`, `requests`, `succeeded`, `run`, `nothing_done`, plus two obligations: `scope` closing the connection, and builders that send nothing until awaited. Rust ends the crawl with `crawl.serve(&run)`, which returns the Run report once the Run ends (ADR 0102's Rust rule).

### Failure points, scored

| Failure point | Crawlee | Scrapy | Goodfetch (TypeScript, Python, Rust) |
|---|---|---|---|
| 1. 404 or 500 | silent: Crawlee 3 throws only for a 5xx, so a 404 reaches the handler as a page, which here reports a missing price; a 500 fails, after its retries, as an `Error` message | generic: `HttpError` in the errback, with the status but no reason; the 500 is retried first | typed: `response_not_ok` (not retried, `fix` names `crawl.handle_statuses`) or `response_unavailable` after retries |
| 2. Slows down or 429 | silent: a 429 retires the session and retries at once; autoscaling follows the machine's CPU and memory, not the site | silent: a 429 is retried after the fixed 1 s delay, with no backoff and no report | typed: the domain backs off by default (ADR 0086), and `progress().waiting` names `backoff` and its setting |
| 3. A field missing | generic: the program's own list; Crawlee counts the page as finished | generic: the program's own list; Scrapy counts nothing | typed: `skip(message)` ends it `skipped`, reason `handler`; a missing element escalates over HTTP, then fails `timeout` naming the locator |
| 4. Bad value passed | typed: the route schema throws `RequestValidationError` when the request is added, at run time only, since `userData` on `enqueueLinks` is not type-checked | generic: a misspelled `cb_kwargs` key is a `TypeError` when the callback is called | typed: a type error before anything runs, then `PayloadInvalidError` (schema, JSON, size) at the enqueue |
| 5. Off-site link | typed: `enqueueLinks` keeps to the hostname, and `onSkippedRequest` reports `filters` or `redirect` | generic: `IgnoreRequest` ("Filtered offsite request") in the errback, as text | typed: the enqueue answers `out_of_scope`; an off-site redirect ends `skipped`, reason `out_of_scope` |
| 6. Same book twice | typed: the second add answers `wasAlreadyPresent`, and the book is pushed once | generic: the dupefilter drops it, logged and counted in `dupefilter/filtered` | typed: the enqueue answers `duplicate`, and the Dataset's `key` is the URL |
| 7. Export fails | generic: `exportToJSON` throws Node's file error | silent: the feed exporter logs "Error storing feed" and the crawl still ends `finished` | typed: `saveAs` throws `path_not_writable` (a `ProgramError` naming the path, its reason and what to change) or `file_write_failed` (an `UnavailableError` naming the side and what to free), and `books.json` is never left half-written (ADR 0116) |
| 8. No item at all | silent: `run()` resolves with its counts and writes `[]`, saying nothing about why | silent: `finished`, with an empty feed | typed: `nothingDone` gives `no_seed_handled` with each seed's reason; otherwise the Export's `records()` and each `succeeded` outcome's `records` show 0 |

**Verdict.**

- **Concepts:** Goodfetch's programs name more concepts than the leaders' (45 against Crawlee's 31, 48 against Scrapy's 36). The report reads typed outcomes, with their kind, Error code, skip reason and Block cause, and it lists `blocked` pages, a fourth ending the leaders have no word for. The leaders give back an exception's text and the program's own lists instead (G1, G6).
- **Lines:** the TypeScript programs are the same length. Goodfetch's Python is five lines longer than Scrapy's, because ruff splits the typed `Book` and the `match` on outcome classes, where Scrapy needs its own counters and errback (G1).
- **Failure points:** Goodfetch is better on every failure point: all eight are typed by default with no code, none is silent, and Politeness and scope need no setting (G5). A failed Export, first reported as a gap, raises `path_not_writable` or `file_write_failed` and never leaves a partial file ([ADR 0116](https://github.com/goodfetch/goodfetch/blob/main/docs/adr/0116-a-saved-file-is-written-whole-or-not-at-all-and-a-failed-write-says-whether-its-path-or-its-storage-failed.md)).

**Raised while writing it:** [Error code of a failed Export write](https://github.com/goodfetch/goodfetch/issues/527), [A Rust crawl handler's own error](https://github.com/goodfetch/goodfetch/issues/528).

## After the sketch

The SDK variants of J-002, or a new Journey if two page types make it a different goal; Proving ground families: `catalog` and the crawl families.
