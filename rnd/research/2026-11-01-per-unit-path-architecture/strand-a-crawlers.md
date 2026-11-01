# Strand A: how proven crawl frameworks structure their shared per-unit paths

Research of 2026-11-01. Goal: give Goodfetch, for each of its own per-unit paths, an owner module, a cost contract (counted work per unit) and a benchmark, taken from how Scrapy, Crawlee and Spider do it. StormCrawler and Nutch appear only where they show a pattern the three lack.

## Method, pins and what this builds on

- **Versions pinned** (registry checks on 2026-11-01):
  - Scrapy **2.19.0** (PyPI, 2026-09-10), tag `2.19.0`; parsel **1.12.1**; queuelib **1.10.0**.
  - Crawlee JS **3.18.2** (npm `latest`, 2026-09-29), tag `v3.18.2`. npm also carries `4.0.0-rc.1`, not examined. Crawlee Python **1.11.0**, tag `v1.11.0`, noted only where it differs materially.
  - Spider **2.53.9** (crates.io, 2026-09-05), commit `e4b53c5b582357e46433fa9ba3d91f930fb67cb4`, the commit the earlier dossier identified from the published archive.
  - StormCrawler **3.7.0** (tag `stormcrawler-3.7.0`), Nutch **1.23** (tag `release-1.23`).
- **How sources were read.** Single source files were downloaded from `raw.githubusercontent.com` at those tags into the session scratchpad, so line numbers could be cited exactly. These are the same read-only GETs WebFetch makes. No repository was cloned and nothing was built or run. Every line link below points at the pinned tag or commit.
- **Already established, not repeated here:**
  - Crawlee's queue, lock, retry and `pushData` contracts, and Spider's `disk` schema, broadcast queue and `ChannelGuard`: [crawlee-spider-contracts-followup](../2026-10-07-crawlee-spider-contracts-followup.md) §4 and §6.
  - The engines' layer map: [crawl-engines](../2026-10-07-crawl-engines.md).
  - One parse shared by readers (Nutch, StormCrawler, Scrapy's `_cached_selector`), Crawlee 3.18.2's double parse, the pipeline shape, and the finding that no crawler syncs run state: ADR 0138's "How proven crawlers shape this work" (branch `docs/per-page-cost-structure`).
- **Unverified** marks a claim I could not check against source.

---

## 1. Scrapy 2.19.0

### 1.1 The per-unit paths

| Unit | Path from entry to result | Source |
|---|---|---|
| Request (admission) | `engine.crawl()` → `_schedule_request()` fires the `request_scheduled` signal, where `OffsiteMiddleware` can raise `IgnoreRequest` → `Scheduler.enqueue_request()`: dupefilter `request_seen()`, then disk-queue push (`JOBDIR` set) or memory-queue push, then three stats increments | [engine.py#L449-L469](https://github.com/scrapy/scrapy/blob/2.19.0/scrapy/core/engine.py#L449-L469), [scheduler.py#L364-L385](https://github.com/scrapy/scrapy/blob/2.19.0/scrapy/core/scheduler.py#L364-L385), [offsite.py#L63-L80](https://github.com/scrapy/scrapy/blob/2.19.0/scrapy/downloadermiddlewares/offsite.py#L63-L80) |
| Request (download) | `_start_scheduled_requests()` loops while `needs_backout()` is false (downloader slots, and the scraper's byte budget) → `scheduler.next_request()` → `_download()` → downloader middlewares `process_request` chain → handler → `process_response` chain in reverse → `logformatter.crawled` (DEBUG) and the `response_received` signal | [engine.py#L351-L418](https://github.com/scrapy/scrapy/blob/2.19.0/scrapy/core/engine.py#L351-L418), [engine.py#L502-L535](https://github.com/scrapy/scrapy/blob/2.19.0/scrapy/core/engine.py#L502-L535), [downloader/middleware.py#L73-L138](https://github.com/scrapy/scrapy/blob/2.19.0/scrapy/core/downloader/middleware.py#L73-L138) |
| Response (parse) | `Scraper.enqueue_scrape()` adds the response to a slot whose `active_size` counts body bytes (backout above 5,000,000) → `_scrape()` → spider middleware chain wraps the callback → `handle_spider_output_async()` runs outputs `CONCURRENT_ITEMS` (100) at a time. Parsing is lazy: `response.selector` builds one lxml tree and caches it | [scraper.py#L62-L104](https://github.com/scrapy/scrapy/blob/2.19.0/scrapy/core/scraper.py#L62-L104), [scraper.py#L247-L286](https://github.com/scrapy/scrapy/blob/2.19.0/scrapy/core/scraper.py#L247-L286), [scraper.py#L399-L438](https://github.com/scrapy/scrapy/blob/2.19.0/scrapy/core/scraper.py#L399-L438), [text.py#L161-L167](https://github.com/scrapy/scrapy/blob/2.19.0/scrapy/http/response/text.py#L161-L167) |
| Extracted link batch | No batch object: each `Request` the callback yields goes through `_process_spidermw_output_async()` → `engine.crawl()` (row 1). `LinkExtractor` reads the shared `response.selector` | [scraper.py#L450-L462](https://github.com/scrapy/scrapy/blob/2.19.0/scrapy/core/scraper.py#L450-L462), [lxmlhtml.py#L405-L427](https://github.com/scrapy/scrapy/blob/2.19.0/scrapy/linkextractors/lxmlhtml.py#L405-L427) |
| Item | `start_itemproc_async()` → `ItemPipelineManager.process_item` chain → `DropItem`/error/success → `logformatter.scraped` (DEBUG) → `item_scraped` signal, whose receivers include `CoreStats` and `FeedExporter.item_scraped`, which writes one line through the open exporter | [scraper.py#L496-L560](https://github.com/scrapy/scrapy/blob/2.19.0/scrapy/core/scraper.py#L496-L560), [pipelines/__init__.py#L48-L66](https://github.com/scrapy/scrapy/blob/2.19.0/scrapy/pipelines/__init__.py#L48-L66), [feedexport.py#L667-L700](https://github.com/scrapy/scrapy/blob/2.19.0/scrapy/extensions/feedexport.py#L667-L700) |
| Stats / log entry | `stats.inc_value(key)` is one dict update. `LogStats` logs every `LOGSTATS_INTERVAL` (60 s). The stats are dumped once at close | [statscollectors.py#L77-L83](https://github.com/scrapy/scrapy/blob/2.19.0/scrapy/statscollectors.py#L77-L83), [statscollectors.py#L102-L113](https://github.com/scrapy/scrapy/blob/2.19.0/scrapy/statscollectors.py#L102-L113), [default_settings.py#L480](https://github.com/scrapy/scrapy/blob/2.19.0/scrapy/settings/default_settings.py#L480) |

### 1.2 Who owns each path: the plug-in rule

- **Four ordered chains, each owned by one manager:** downloader middlewares, spider middlewares, item pipelines and extensions.
  - Each manager is built once by `MiddlewareManager.from_crawler()` from a priority dict (`*_BASE` merged with the user's) and sorted ([middleware.py#L88-L114](https://github.com/scrapy/scrapy/blob/2.19.0/scrapy/middleware.py#L88-L114), [utils/conf.py#L20-L62](https://github.com/scrapy/scrapy/blob/2.19.0/scrapy/utils/conf.py#L20-L62)).
  - `_add_middleware()` registers only the hook methods a component actually defines, so a component without `process_response` adds nothing to that chain ([downloader/middleware.py#L43-L52](https://github.com/scrapy/scrapy/blob/2.19.0/scrapy/core/downloader/middleware.py#L43-L52)).
  - Per unit, the manager walks a precomputed `deque` of bound methods ([middleware.py#L131-L153](https://github.com/scrapy/scrapy/blob/2.19.0/scrapy/middleware.py#L131-L153)).
- **Signals** are the second plug-in surface. Extensions connect receivers at build time, for example `CoreStats` on `item_scraped` and `response_received` ([corestats.py#L28-L34](https://github.com/scrapy/scrapy/blob/2.19.0/scrapy/extensions/corestats.py)).
  - Each send looks up the live receivers again (`liveReceivers(getAllReceivers(...))`, [utils/signal.py#L36-L53](https://github.com/scrapy/scrapy/blob/2.19.0/scrapy/utils/signal.py#L36-L53)). Signals cost per unit even when nobody listens: three per item path and two per request path.
- **The rule:** a feature adds per-unit work by adding a chain component with a priority, or a signal receiver. The engine's own loop never grows.

### 1.3 Built once versus per unit

| Built once (where) | Per unit, memoised on the unit (where) |
|---|---|
| Settings frozen after the extensions are built ([crawler.py#L210-L211](https://github.com/scrapy/scrapy/blob/2.19.0/scrapy/crawler.py#L210-L211), [settings/__init__.py#L615-L618](https://github.com/scrapy/scrapy/blob/2.19.0/scrapy/settings/__init__.py#L615-L618)) | Request fingerprint: SHA-1 of the canonical URL, method and body, cached per `Request` in a `WeakKeyDictionary` ([utils/request.py#L34-L111](https://github.com/scrapy/scrapy/blob/2.19.0/scrapy/utils/request.py#L34-L111)) |
| The middleware, pipeline and extension chains (above) | `urlparse_cached`: one URL parse per request or response ([httpobj.py#L13-L23](https://github.com/scrapy/scrapy/blob/2.19.0/scrapy/utils/httpobj.py)) |
| `LinkExtractor` allow/deny/restrict regexes compiled in its constructor ([lxmlhtml.py#L337-L358](https://github.com/scrapy/scrapy/blob/2.19.0/scrapy/linkextractors/lxmlhtml.py#L337-L358)) | `response.selector`: one tree per response ([text.py#L161-L167](https://github.com/scrapy/scrapy/blob/2.19.0/scrapy/http/response/text.py#L161-L167)) |
| Offsite host regex compiled on `spider_opened`, recompiled only if `allowed_domains` changes ([offsite.py#L70-L77](https://github.com/scrapy/scrapy/blob/2.19.0/scrapy/downloadermiddlewares/offsite.py#L70-L77)) | `get_base_url` and `get_meta_refresh` cached per response ([utils/response.py#L28-L55](https://github.com/scrapy/scrapy/blob/2.19.0/scrapy/utils/response.py)) |
| Process-wide parsel caches: CSS→XPath translation in an LRU of 256 per translator ([csstranslator.py#L89-L100](https://github.com/scrapy/parsel/blob/v1.12.1/parsel/csstranslator.py#L89-L100)), compiled `etree.XPath` in an LRU of 2,048 ([selector.py#L402-L421](https://github.com/scrapy/parsel/blob/v1.12.1/parsel/selector.py#L402-L421)) | |
| Retry codes as a `set` ([retry.py#L144-L148](https://github.com/scrapy/scrapy/blob/2.19.0/scrapy/downloadermiddlewares/retry.py#L144-L148)), the JSON encoder per exporter ([exporters.py#L137-L146](https://github.com/scrapy/scrapy/blob/2.19.0/scrapy/exporters.py)), the dupefilter's `requests.seen` handle opened once ([dupefilters.py#L117-L120](https://github.com/scrapy/scrapy/blob/2.19.0/scrapy/dupefilters.py#L117-L120)) | |

**Residual per-unit rework:**

- `OffsiteMiddleware` runs twice per request: on `request_scheduled` and again as the first downloader middleware ([offsite.py#L65-L66](https://github.com/scrapy/scrapy/blob/2.19.0/scrapy/downloadermiddlewares/offsite.py#L65-L66), [default_settings.py#L329](https://github.com/scrapy/scrapy/blob/2.19.0/scrapy/settings/default_settings.py#L329)).
- Each `should_follow()` copies and compares the `allowed_domains` list before searching the regex ([offsite.py#L73-L77](https://github.com/scrapy/scrapy/blob/2.19.0/scrapy/downloadermiddlewares/offsite.py#L73-L77), [#L121-L125](https://github.com/scrapy/scrapy/blob/2.19.0/scrapy/downloadermiddlewares/offsite.py#L121-L125)).
- `FeedExporter.item_scraped` rebuilds its slot list for every item ([feedexport.py#L667-L700](https://github.com/scrapy/scrapy/blob/2.19.0/scrapy/extensions/feedexport.py#L667-L700)).

### 1.4 Is "off" free?

**Mostly by construction.**

- A component disables itself by raising `NotConfigured` in its constructor, and the manager then leaves it out of every chain ([middleware.py#L98-L104](https://github.com/scrapy/scrapy/blob/2.19.0/scrapy/middleware.py#L98-L104)). Examples:
  - `ROBOTSTXT_OBEY=False` (the default) and `COOKIES_ENABLED=False` ([robotstxt.py#L39-L41](https://github.com/scrapy/scrapy/blob/2.19.0/scrapy/downloadermiddlewares/robotstxt.py#L39-L41), [cookies.py#L51-L53](https://github.com/scrapy/scrapy/blob/2.19.0/scrapy/downloadermiddlewares/cookies.py#L51-L53));
  - `HTTPCACHE_ENABLED=False`, `RETRY_ENABLED=False` and `DOWNLOADER_STATS=False` ([httpcache.py#L48-L50](https://github.com/scrapy/scrapy/blob/2.19.0/scrapy/downloadermiddlewares/httpcache.py#L48-L50), [retry.py#L144-L146](https://github.com/scrapy/scrapy/blob/2.19.0/scrapy/downloadermiddlewares/retry.py#L144-L146), [stats.py#L43-L45](https://github.com/scrapy/scrapy/blob/2.19.0/scrapy/downloadermiddlewares/stats.py#L43-L45));
  - no `FEEDS` ([feedexport.py#L493-L494](https://github.com/scrapy/scrapy/blob/2.19.0/scrapy/extensions/feedexport.py#L493-L494)) and `LOGSTATS_INTERVAL=0` ([logstats.py#L37-L39](https://github.com/scrapy/scrapy/blob/2.19.0/scrapy/extensions/logstats.py#L37-L39)).
- Setting a component's priority to `None` removes it (`without_none_values`, [utils/conf.py#L61](https://github.com/scrapy/scrapy/blob/2.19.0/scrapy/utils/conf.py#L61)).
- Without `JOBDIR`, there is no disk queue and no fingerprint file.

**Not free:**

- `LOG_LEVEL` defaults to `DEBUG` ([default_settings.py#L464](https://github.com/scrapy/scrapy/blob/2.19.0/scrapy/settings/default_settings.py#L464)). One "Crawled" line is formatted per response, and one "Scraped from … item" line per item, which renders the item ([logformatter.py#L77-L115](https://github.com/scrapy/scrapy/blob/2.19.0/scrapy/logformatter.py#L77-L115)). Scrapy's own benchmarks set `LOG_ENABLED: False`.
- The offsite regex search runs even with no `allowed_domains`: it compiles `re.compile("")` ([offsite.py#L127-L130](https://github.com/scrapy/scrapy/blob/2.19.0/scrapy/downloadermiddlewares/offsite.py#L127-L130)).
- Signals are dispatched with or without receivers.

### 1.5 Persistence and logging per unit

- **Request queue (`JOBDIR`).**
  - Each admitted request is pickled and pushed to a `queuelib` disk queue first; the memory queue takes only unserializable requests ([scheduler.py#L377-L385](https://github.com/scrapy/scrapy/blob/2.19.0/scrapy/core/scheduler.py#L377-L385), [#L414-L436](https://github.com/scrapy/scrapy/blob/2.19.0/scrapy/core/scheduler.py#L414-L436)).
  - The default `PickleLifoDiskQueue` keeps one file handle open and writes on every push, with no `fsync` anywhere in queuelib ([queue.py#L224-L260](https://github.com/scrapy/queuelib/blob/v1.10.0/queuelib/queue.py#L224-L260)).
  - `active.json` (the priority-queue state) is written only on a clean close ([scheduler.py#L353-L362](https://github.com/scrapy/scrapy/blob/2.19.0/scrapy/core/scheduler.py#L353-L362), [#L496-L498](https://github.com/scrapy/scrapy/blob/2.19.0/scrapy/core/scheduler.py#L496-L498)).
- **Dupefilter.** One append per new fingerprint, as a 2-byte big-endian length followed by the binary fingerprint (new in this release; earlier releases stored hex lines). The append goes through a buffered handle opened once, never flushed per write ([dupefilters.py#L86-L147](https://github.com/scrapy/scrapy/blob/2.19.0/scrapy/dupefilters.py#L86-L147)).
- **Items.** `FileFeedStorage` opens the file lazily on the first item and keeps it open until the spider closes ([feedexport.py#L186-L202](https://github.com/scrapy/scrapy/blob/2.19.0/scrapy/extensions/feedexport.py#L186-L202), [#L435-L460](https://github.com/scrapy/scrapy/blob/2.19.0/scrapy/extensions/feedexport.py#L435-L460)).
  - JSON Lines writes one encoded line per item ([exporters.py#L143-L146](https://github.com/scrapy/scrapy/blob/2.19.0/scrapy/exporters.py)).
  - Remote storages (S3, GCS, FTP) write a temporary file and upload it at close ([feedexport.py#L144-L158](https://github.com/scrapy/scrapy/blob/2.19.0/scrapy/extensions/feedexport.py#L144-L158)).
  - `FEED_EXPORT_BATCH_ITEM_COUNT` (default 0, [default_settings.py#L371](https://github.com/scrapy/scrapy/blob/2.19.0/scrapy/settings/default_settings.py#L371)) rotates files by item count; it does not batch writes.
- **Stats** are in memory until close.

### 1.6 How Scrapy guards per-unit cost

- **CodSpeed benchmarks in CI since 2026-07-31.**
  - `codspeed.yml` runs `tox -e benchmark` on every pull request touching `scrapy/**`. It runs in CodSpeed's `simulation` mode (instruction counting, deterministic) on a GitHub runner, and in `walltime` mode on CodSpeed's macro runner for `walltime`-marked tests ([codspeed.yml](https://github.com/scrapy/scrapy/blob/2.19.0/.github/workflows/codspeed.yml), [tox.ini#L392-L411](https://github.com/scrapy/scrapy/blob/2.19.0/tox.ini#L392-L411); first commit to the workflow dated 2026-07-31 per the GitHub commits API).
  - The suite is designed around per-unit costs ([tests/benchmarks/test_crawl.py](https://github.com/scrapy/scrapy/blob/2.19.0/tests/benchmarks/test_crawl.py), [test_urls.py](https://github.com/scrapy/scrapy/blob/2.19.0/tests/benchmarks/test_urls.py)):
    - `test_overhead_engine` measures per-request overhead with a `NullDownloadHandler` that does no I/O, yields to the loop once per request and records `benchmark/peak_concurrency` ([tests/benchmarks/__init__.py](https://github.com/scrapy/scrapy/blob/2.19.0/tests/benchmarks/__init__.py));
    - `test_overhead_broad` covers shallow and deep broad crawls;
    - `test_overhead_concurrency` and `test_overhead_delay`;
    - `test_overhead_items`, with 1 item per response, 100, and a high `CONCURRENT_ITEMS`, plus `test_overhead_item_concurrency`;
    - `test_parse_engine` and `test_parse_http`, on a saved 50 KB listing snapshot;
    - `test_extract_links`, `test_requests` and `test_fingerprints`.
  - Each benchmark asserts that its workload still happened. `item_scraped_count == PAGES + 1`, peak concurrency above 1, and the book spider checks that its selectors still match, "otherwise silently leave the benchmark with less parsing to do".
- **Regression policy: unverified.**
  - CodSpeed reports a status per pull request. Whether that check is required for merge is a repository setting I could not see.
  - `docs/contributing.rst` at 2.19.0 does not mention CodSpeed or benchmarks.
- **`scrapy bench`.**
  - A local smoke figure: a link-following spider against a local server for 10 s, logging stats every second ([commands/bench.py](https://github.com/scrapy/scrapy/blob/2.19.0/scrapy/commands/bench.py)).
  - The separate [scrapy-bench](https://github.com/scrapy/scrapy-bench) CLI was last pushed on 2025-06-28. Neither gates anything.

---

## 2. Crawlee 3.18.2 (JS), with the Python 1.11.0 differences

### 2.1 The per-unit paths

| Unit | Path | Source |
|---|---|---|
| Request (task) | `AutoscaledPool` calls `_runTaskFunction()`: fetch the next request from the request manager, take a session, check robots.txt (skipped when off), `stats.startJob`, build a fresh `crawlingContext` holding closures (`enqueueLinks`, `addRequests`, `pushData`, `sendRequest`…), run the handler under a timeout, `markRequestHandled`, `stats.finishJob`; errors go to `_requestFunctionErrorHandler` | [basic-crawler.ts#L1601-L1747](https://github.com/apify/crawlee/blob/v3.18.2/packages/basic-crawler/src/internals/basic-crawler.ts#L1601-L1747) |
| Navigation (HTTP and browser) | `_handleNavigation`: `_executeHooks(preNavigationHooks)`, then the fetch or `page.goto`, then `_executeHooks(postNavigationHooks)`. `HttpCrawler` prepends its own `_abortDownloadOfBody` post-hook | [http-crawler.ts#L455-L460](https://github.com/apify/crawlee/blob/v3.18.2/packages/http-crawler/src/internals/http-crawler.ts#L455-L460), [http-crawler.ts#L625-L650](https://github.com/apify/crawlee/blob/v3.18.2/packages/http-crawler/src/internals/http-crawler.ts#L625-L650), [browser-crawler.ts#L653-L679](https://github.com/apify/crawlee/blob/v3.18.2/packages/browser-crawler/src/internals/browser-crawler.ts#L653-L679), [basic-crawler.ts#L2025-L2034](https://github.com/apify/crawlee/blob/v3.18.2/packages/basic-crawler/src/internals/basic-crawler.ts#L2025-L2034) |
| Response (parse) | `HttpCrawler` installs `parseWithCheerio`, which runs `cheerio.load(body)` **on every call** and is not memoised. With `retryOnBlocked`, `isRequestBlocked` parses once and the handler's call parses again. The double parse of `CheerioCrawler` is in ADR 0138 | [http-crawler.ts#L528-L543](https://github.com/apify/crawlee/blob/v3.18.2/packages/http-crawler/src/internals/http-crawler.ts#L528-L543), [#L581-L584](https://github.com/apify/crawlee/blob/v3.18.2/packages/http-crawler/src/internals/http-crawler.ts#L581-L584), [#L600-L609](https://github.com/apify/crawlee/blob/v3.18.2/packages/http-crawler/src/internals/http-crawler.ts#L600-L609) |
| Handler dispatch | The `Router` function validates `request.userData` against the label's Standard Schema (when one exists), runs `router.use()` middlewares in order, then `Map.get(label)` | [router.ts#L335-L347](https://github.com/apify/crawlee/blob/v3.18.2/packages/core/src/router.ts#L335-L347), [router.ts#L423-L434](https://github.com/apify/crawlee/blob/v3.18.2/packages/core/src/router.ts#L423-L434) |
| Extracted link batch | `enqueueLinks()`: build glob, regexp and strategy pattern objects, `createRequests()` (exclude, then include, then `new Request` per link), `filterRequestsByPatterns()` with the strategy, robots filter, `transformRequestFunction`, then `requestQueue.addRequestsBatched()`. `crawler.addRequests()` adds max depth, robots and **userData schema validation** per request in an async generator | [enqueue_links.ts#L235-L470](https://github.com/apify/crawlee/blob/v3.18.2/packages/core/src/enqueue_links/enqueue_links.ts#L235-L470), [shared.ts#L169-L236](https://github.com/apify/crawlee/blob/v3.18.2/packages/core/src/enqueue_links/shared.ts#L169-L236), [basic-crawler.ts#L1249-L1290](https://github.com/apify/crawlee/blob/v3.18.2/packages/basic-crawler/src/internals/basic-crawler.ts#L1249-L1290) |
| Admission into the queue | `addRequestsBatched` chunks the stream (`batchSize` 1000; the first chunk is awaited and the rest go in the background with a 1,000 ms wait between batches) → `addRequests` → per request, `getRequestId(uniqueKey)` (SHA-256, base64, 15 characters), a `requestCache` LRU lookup, then a fixed-size dedup cache, then one `client.batchAddRequests()` per chunk | [request_provider.ts#L415-L600](https://github.com/apify/crawlee/blob/v3.18.2/packages/core/src/storages/request_provider.ts#L415-L600), [#L291-L400](https://github.com/apify/crawlee/blob/v3.18.2/packages/core/src/storages/request_provider.ts#L291-L400), [storages/utils.ts#L117-L121](https://github.com/apify/crawlee/blob/v3.18.2/packages/core/src/storages/utils.ts#L117-L121), [request_dedup_cache.ts](https://github.com/apify/crawlee/blob/v3.18.2/packages/core/src/storages/request_dedup_cache.ts) |
| Item (record) | `context.pushData` → `Dataset.pushData` → `checkAndSerialize` (one `JSON.stringify` per item, for the size check) → `chunkBySize` → `client.pushItems` per chunk, in series | [dataset.ts#L27-L60](https://github.com/apify/crawlee/blob/v3.18.2/packages/core/src/storages/dataset.ts#L27-L60), [dataset.ts#L276-L300](https://github.com/apify/crawlee/blob/v3.18.2/packages/core/src/storages/dataset.ts#L276-L300) |
| Stats / log | `Statistics.startJob`/`finishJob` are Map and counter updates. A stats log every `logIntervalSecs` (60), a status message every `statusMessageLoggingInterval` (10 s), state persisted on `PERSIST_STATE` | [statistics.ts#L217-L240](https://github.com/apify/crawlee/blob/v3.18.2/packages/core/src/crawlers/statistics.ts#L217-L240), [#L123](https://github.com/apify/crawlee/blob/v3.18.2/packages/core/src/crawlers/statistics.ts#L123), [basic-crawler.ts#L681](https://github.com/apify/crawlee/blob/v3.18.2/packages/basic-crawler/src/internals/basic-crawler.ts#L681) |

### 2.2 Who owns each path: the plug-in rule

- **`BasicCrawler._runTaskFunction` owns the request path.**
  - Subclasses override `_runRequestHandler` ([basic-crawler.ts#L1426-L1428](https://github.com/apify/crawlee/blob/v3.18.2/packages/basic-crawler/src/internals/basic-crawler.ts#L1426-L1428)).
  - User features plug in as arrays of hooks given as options: `preNavigationHooks` and `postNavigationHooks`, walked in order and skipped when empty.
  - The handler plugs in as the `Router`: label → handler map, middlewares through `use()`, schemas per label.
  - The default router is `crawler.router` ([basic-crawler.ts#L539](https://github.com/apify/crawlee/blob/v3.18.2/packages/basic-crawler/src/internals/basic-crawler.ts#L539), [#L721](https://github.com/apify/crawlee/blob/v3.18.2/packages/basic-crawler/src/internals/basic-crawler.ts#L721)).
- **`RequestProvider` owns admission** (its caches and batching). **`Dataset` owns records.** **`Statistics` owns counters.**
- **Persistence sits behind one storage-client interface,** `@crawlee/memory-storage` locally.
- **The rule:** a feature is a constructor option on the crawler, a hook in an array, or a router middleware. There is no priority-ordered component registry like Scrapy's.

### 2.3 Built once versus per unit

- **Built once:**
  - in `_init()`: `AutoscaledPool`, `SessionPool` and the request manager ([basic-crawler.ts#L1408-L1424](https://github.com/apify/crawlee/blob/v3.18.2/packages/basic-crawler/src/internals/basic-crawler.ts#L1408-L1424));
  - in the constructor: the robots.txt cache, an LRU of 1,000 origins ([#L708](https://github.com/apify/crawlee/blob/v3.18.2/packages/basic-crawler/src/internals/basic-crawler.ts#L708));
  - route maps and schema maps when the router is created ([router.ts#L233-L235](https://github.com/apify/crawlee/blob/v3.18.2/packages/core/src/router.ts#L233-L235)). The schema objects are compiled by the user's library before Crawlee sees them.
- **Cached across calls, but partly:**
  - `enqueueLinksPatternCache`, at most 1,000 entries, keeps validated glob, regexp and pseudo-URL objects "only constructed once" ([shared.ts#L14-L22](https://github.com/apify/crawlee/blob/v3.18.2/packages/core/src/enqueue_links/shared.ts#L14-L22)).
  - But `createPatternObjectMatcher()` runs `new Minimatch(glob)` for every pattern on **every** `enqueueLinks` call, that is per page ([shared.ts#L176-L177](https://github.com/apify/crawlee/blob/v3.18.2/packages/core/src/enqueue_links/shared.ts#L176-L177), [#L224](https://github.com/apify/crawlee/blob/v3.18.2/packages/core/src/enqueue_links/shared.ts#L224), [#L281-L293](https://github.com/apify/crawlee/blob/v3.18.2/packages/core/src/enqueue_links/shared.ts#L281-L293)).
  - The default strategy pattern (`same-hostname`) is itself built from each page's `baseUrl` ([enqueue_links.ts#L326-L335](https://github.com/apify/crawlee/blob/v3.18.2/packages/core/src/enqueue_links/enqueue_links.ts#L326-L335)).
  - **This is the "cache the input, compile per use" anti-pattern.**
- **Per link, deliberately cheapened but still per unit:**
  - The `Request` constructor validates its options with `ow`, through a hand-written loop over the present keys only, "speeds up the validation cca 3-fold" ([request.ts#L145-L164](https://github.com/apify/crawlee/blob/v3.18.2/packages/core/src/request.ts#L145-L164)).
  - Then `computeUniqueKey` (`normalizeUrl`, and a payload hash when there is a payload, [request.ts#L418-L438](https://github.com/apify/crawlee/blob/v3.18.2/packages/core/src/request.ts#L418-L438)) and a SHA-256 request ID.
- **userData schema validation runs twice per request:**
  - at admission (`validateRequestUserData`, on every add path the crawler owns, [basic-crawler.ts#L1165-L1188](https://github.com/apify/crawlee/blob/v3.18.2/packages/basic-crawler/src/internals/basic-crawler.ts#L1165-L1188), [#L1280-L1283](https://github.com/apify/crawlee/blob/v3.18.2/packages/basic-crawler/src/internals/basic-crawler.ts#L1280-L1283));
  - again at dispatch (`router.validateRequest`). The second run also covers requests added to the queue directly.
  - A schema that declares `label` is validated a second time with the label included ([router.ts#L75-L98](https://github.com/apify/crawlee/blob/v3.18.2/packages/core/src/router.ts#L75-L98)).
  - The validated value replaces `userData`, so the queue stores the coerced payload.

### 2.4 Is "off" free?

**Mostly, through guards at the top of each step:**

- `respectRobotsTxtFile` false returns `true` before any lookup ([basic-crawler.ts#L1441-L1444](https://github.com/apify/crawlee/blob/v3.18.2/packages/basic-crawler/src/internals/basic-crawler.ts#L1441-L1444)).
- `retryOnBlocked` (default `false`, [#L661](https://github.com/apify/crawlee/blob/v3.18.2/packages/basic-crawler/src/internals/basic-crawler.ts#L661)) skips the blocked-page parse ([http-crawler.ts#L581-L584](https://github.com/apify/crawlee/blob/v3.18.2/packages/http-crawler/src/internals/http-crawler.ts#L581-L584)).
- `useSessionPool` false skips sessions.
- Schema validation is a no-op without a schema-router. A `typeof getSchema` check remains per request.
- `persistStorage: false` (or `CRAWLEE_PERSIST_STORAGE=0`) makes every storage entry a memory entry with no file I/O ([memory-storage.ts#L79-L83](https://github.com/apify/crawlee/blob/v3.18.2/packages/memory-storage/src/memory-storage.ts#L79-L83), [fs/dataset/index.ts](https://github.com/apify/crawlee/blob/v3.18.2/packages/memory-storage/src/fs/dataset/index.ts)).
- Metadata files are off by default: `writeMetadata` is false unless `DEBUG` asks for it ([memory-storage.ts#L74-L78](https://github.com/apify/crawlee/blob/v3.18.2/packages/memory-storage/src/memory-storage.ts#L74-L78)).

**Not free:**

- Per-request context construction builds about eight closures whether or not the handler uses them.
- The `Request` constructor's `ow` checks.
- The two interval timers: system info every 1 s, persist state every 60 s ([configuration.ts#L295-L299](https://github.com/apify/crawlee/blob/v3.18.2/packages/core/src/configuration.ts#L295-L299)).

### 2.5 Persistence and logging per unit

**JS, with the default `persistStorage: true`:**

- **Request files.** Each request is one pretty-printed JSON file (`{requestId}.json`), written through `proper-lockfile`, without `fsync` ([fs/request-queue/fs.ts#L27](https://github.com/apify/crawlee/blob/v3.18.2/packages/memory-storage/src/fs/request-queue/fs.ts#L27), [#L52-L66](https://github.com/apify/crawlee/blob/v3.18.2/packages/memory-storage/src/fs/request-queue/fs.ts#L52-L66), [fs-utils.ts#L44-L66](https://github.com/apify/crawlee/blob/v3.18.2/packages/memory-storage/src/background-handler/fs-utils.ts#L44-L66)).
- **Writes per request:** that file is written on add ([request-queue.ts#L470](https://github.com/apify/crawlee/blob/v3.18.2/packages/memory-storage/src/resource-clients/request-queue.ts#L470)), on lock in `listAndLockHead` ([#L281](https://github.com/apify/crawlee/blob/v3.18.2/packages/memory-storage/src/resource-clients/request-queue.ts#L281)) and on handled ([#L540](https://github.com/apify/crawlee/blob/v3.18.2/packages/memory-storage/src/resource-clients/request-queue.ts#L540)). That is **about three file writes, each with a lock, per request.**
- **Memory.** An entry's in-memory copy is swept after 60 s idle ([fs.ts#L75-L85](https://github.com/apify/crawlee/blob/v3.18.2/packages/memory-storage/src/fs/request-queue/fs.ts#L75-L85)).
- **Dataset items:** one file per item (`000000001.json`), written in a loop, one awaited write after another ([resource-clients/dataset.ts#L229-L243](https://github.com/apify/crawlee/blob/v3.18.2/packages/memory-storage/src/resource-clients/dataset.ts#L229-L243), [fs/dataset/fs.ts#L16](https://github.com/apify/crawlee/blob/v3.18.2/packages/memory-storage/src/fs/dataset/fs.ts#L16)).
- **Statistics and session pool state:** written to the key-value store on `PERSIST_STATE`, every 60 s ([event_manager.ts#L43-L47](https://github.com/apify/crawlee/blob/v3.18.2/packages/core/src/events/event_manager.ts#L43-L47), [statistics.ts#L325-L347](https://github.com/apify/crawlee/blob/v3.18.2/packages/core/src/crawlers/statistics.ts#L325-L347)).
- **Head cache.** The queue head is fetched and locked 25 at a time (`LIST_AND_LOCK_HEAD_LIMIT`). A 1,000-entry recently-handled cache sits in front ([request_queue_v2.ts#L17-L28](https://github.com/apify/crawlee/blob/v3.18.2/packages/core/src/storages/request_queue_v2.ts#L17-L28)).

**Python 1.11.0 differs materially in persistence only.** It has pluggable storage clients: file system, memory, SQL and Redis.

- **File system:** each request file is written with `atomic_write`, a temporary file then `replace`, on a thread, without `fsync` ([_request_queue_client.py#L384-L385](https://github.com/apify/crawlee-python/blob/v1.11.0/src/crawlee/storage_clients/_file_system/_request_queue_client.py#L384-L385), [_utils/file.py#L39-L62](https://github.com/apify/crawlee-python/blob/v1.11.0/src/crawlee/_utils/file.py#L39-L62)). The queue's in-progress and handled sets live in a `RecoverableState` persisted on the persist-state event, not per operation ([#L137-L147](https://github.com/apify/crawlee-python/blob/v1.11.0/src/crawlee/storage_clients/_file_system/_request_queue_client.py#L137-L147)).
- **SQL:** `push_data` is one multi-row `INSERT` per call. Instead of updating the dataset's metadata row on every push, it appends to a `*_metadata_buffer` table that is folded into the row later, under a 1 s processing lock ([_sql/_dataset_client.py#L40-L60](https://github.com/apify/crawlee-python/blob/v1.11.0/src/crawlee/storage_clients/_sql/_dataset_client.py#L40-L60), [#L150-L159](https://github.com/apify/crawlee-python/blob/v1.11.0/src/crawlee/storage_clients/_sql/_dataset_client.py#L150-L159), [_sql/_client_mixin.py#L75](https://github.com/apify/crawlee-python/blob/v1.11.0/src/crawlee/storage_clients/_sql/_client_mixin.py#L75), [#L405-L428](https://github.com/apify/crawlee-python/blob/v1.11.0/src/crawlee/storage_clients/_sql/_client_mixin.py#L405-L428)). **This is a hot-row avoidance pattern** the JS version lacks.

### 2.6 How Crawlee guards per-unit cost

**None found.**

- The `v3.18.2` tree contains no file whose path matches `bench` or `perf` (GitHub trees API, recursive).
- Its CI (`test-ci.yml`) builds, type-checks and runs unit tests, and `test-e2e.yml` runs functional end-to-end actors.
- The crawlee-python `v1.11.0` tree has no benchmark files either.
- Per-unit cost awareness shows only in code comments: the `ow` 3-fold note, the pattern cache comment, and the dedup cache's issue reference (#3120).
- A web search (WebSearch, 2026-11-01) found no Apify benchmark or regression repository for Crawlee. Apify's published comparisons, such as [Crawl4AI vs Website Content Crawler](https://blog.apify.com/crawl4ai-vs-website-content-crawler/), are product benchmarks, not regression gates.

---

## 3. Spider 2.53.9 (`spider` crate)

### 3.1 The per-unit paths

One coordinator loop per crawl, `crawl_concurrent_raw` for HTTP ([website.rs#L8155](https://github.com/spider-rs/spider/blob/e4b53c5b582357e46433fa9ba3d91f930fb67cb4/spider/src/website.rs#L8155)), owns admission. Fetch and parse run in spawned tasks.

| Unit | Path | Source |
|---|---|---|
| Link (admission) | In the coordinator's `tokio::select!`: `is_allowed(&link)` checks, in order, the `links_visited` set, then the whitelist and blacklist `RegexSet`s, then robots, then depth, then the budget. Then `is_allowed_disk` (feature `disk`), then `insert_link_unchecked`, then a semaphore permit, then `spawn_set("page_fetch", …)` | [website.rs#L2090-L2244](https://github.com/spider-rs/spider/blob/e4b53c5b582357e46433fa9ba3d91f930fb67cb4/spider/src/website.rs#L2090-L2244), [website.rs#L8345-L8415](https://github.com/spider-rs/spider/blob/e4b53c5b582357e46433fa9ba3d91f930fb67cb4/spider/src/website.rs#L8345-L8415) |
| Page (fetch and parse) | In the task: `Page::new_page_streaming*` fetches and feeds body chunks to a `lol_html` rewriter that extracts links, `<base>` and metadata **while streaming**, with no tree. An optional `hash_html` signature runs when `normalize` is set. Then `channel_send_page()`, and the task returns `(links, signature)` to the coordinator, which merges them into the next round | [page.rs#L4722-L4790](https://github.com/spider-rs/spider/blob/e4b53c5b582357e46433fa9ba3d91f930fb67cb4/spider/src/page.rs#L4722-L4790), [page.rs#L4266-L4345](https://github.com/spider-rs/spider/blob/e4b53c5b582357e46433fa9ba3d91f930fb67cb4/spider/src/page.rs#L4266-L4345), [website.rs#L8466-L8500](https://github.com/spider-rs/spider/blob/e4b53c5b582357e46433fa9ba3d91f930fb67cb4/spider/src/website.rs#L8466-L8500) |
| Result (no item concept) | `channel_send_page`: with `balance`, large pages are spooled to disk. Then `broadcast::Sender::send(page)` if anyone subscribed; otherwise the page is dropped. Every consumer (user code, the WARC writer) is a subscriber | [website.rs#L15080-L15122](https://github.com/spider-rs/spider/blob/e4b53c5b582357e46433fa9ba3d91f930fb67cb4/spider/src/website.rs#L15080-L15122) |
| Log | `utils::log()` formats only if `log_enabled!(Info)`. The HTTP loop logs failures, not each page | [utils/mod.rs#L9528-L9533](https://github.com/spider-rs/spider/blob/e4b53c5b582357e46433fa9ba3d91f930fb67cb4/spider/src/utils/mod.rs#L9528-L9533), [website.rs#L7560-L7600](https://github.com/spider-rs/spider/blob/e4b53c5b582357e46433fa9ba3d91f930fb67cb4/spider/src/website.rs#L7560-L7600) |

### 3.2 Who owns each path: the plug-in rule

- **`Website` owns everything; there is no middleware chain.** A feature plugs in at one of three levels:
  - a **Cargo feature**: 128 of them, with `default = ["basic", "io_uring", "tcp_fastopen", "splice", "numa", "zero_copy"]` ([Cargo.toml#L214-L215](https://github.com/spider-rs/spider/blob/e4b53c5b582357e46433fa9ba3d91f930fb67cb4/spider/Cargo.toml#L214-L215)). It compiles alternative bodies of the same function, for example `is_allowed` under `regex` or not ([website.rs#L2088-L2118](https://github.com/spider-rs/spider/blob/e4b53c5b582357e46433fa9ba3d91f930fb67cb4/spider/src/website.rs#L2088-L2118));
  - a **runtime `Configuration` field**, such as `respect_robots_txt`, budgets, `normalize` and `return_page_links`;
  - an **`Option<…>` callback or strategy** on `Website`: `on_should_crawl_callback`, `on_link_find_callback`, `RetryStrategy`, `RemoteFetcher`, `QualityValidator`.
- **The coordinator snapshots everything once** into a shared `Arc` tuple (client, selectors, channel, external domains, guard, callbacks…). It clones `Arc`s into each spawned task, about 20 refcount bumps per spawn ([website.rs#L8236-L8414](https://github.com/spider-rs/spider/blob/e4b53c5b582357e46433fa9ba3d91f930fb67cb4/spider/src/website.rs#L8236-L8414)).
- **The rule for consumers is "subscribe to the page broadcast".** The WARC writer is a subscriber with its own backpressure ([warc.rs#L540-L565](https://github.com/spider-rs/spider/blob/e4b53c5b582357e46433fa9ba3d91f930fb67cb4/spider/src/utils/warc.rs#L540-L565)).

### 3.3 Built once versus per unit

- **Process-wide, once:**
  - the link, XML, `<base>`, title, description and og:image `lol_html::Selector`s are each compiled in a `OnceLock` and borrowed per page through `Cow::Borrowed` ([css_selectors.rs#L89-L142](https://github.com/spider-rs/spider/blob/e4b53c5b582357e46433fa9ba3d91f930fb67cb4/spider/src/utils/css_selectors.rs#L89-L142), [page.rs#L3971-L4000](https://github.com/spider-rs/spider/blob/e4b53c5b582357e46433fa9ba3d91f930fb67cb4/spider/src/page.rs#L3971-L4000));
  - error matchers as `lazy_static` Aho-Corasick automata ([page.rs#L146-L300](https://github.com/spider-rs/spider/blob/e4b53c5b582357e46433fa9ba3d91f930fb67cb4/spider/src/page.rs#L146-L300)).
  - **Exception:** some handlers still use the `element!("script", …)` macro, which parses its selector when the rewriter is built ([page.rs#L4420](https://github.com/spider-rs/spider/blob/e4b53c5b582357e46433fa9ba3d91f930fb67cb4/spider/src/page.rs#L4420), [#L4441](https://github.com/spider-rs/spider/blob/e4b53c5b582357e46433fa9ba3d91f930fb67cb4/spider/src/page.rs#L4441)). Which branches reach them was not traced.
- **Per crawl, once:**
  - the HTTP client and the compiled custom anti-bot patterns in `setup_base` ([website.rs#L4021-L4031](https://github.com/spider-rs/spider/blob/e4b53c5b582357e46433fa9ba3d91f930fb67cb4/spider/src/website.rs#L4021-L4031));
  - robots.txt fetched and parsed in `setup`, only when `respect_robots_txt` is set and only for multi-page crawls ([website.rs#L4139-L4156](https://github.com/spider-rs/spider/blob/e4b53c5b582357e46433fa9ba3d91f930fb67cb4/spider/src/website.rs#L4139-L4156), [#L2793-L2794](https://github.com/spider-rs/spider/blob/e4b53c5b582357e46433fa9ba3d91f930fb67cb4/spider/src/website.rs#L2793-L2794));
  - the domain selectors (`setup_selectors`) and the allow and block `RegexSet`s (`configure_allowlist`) ([website.rs#L8188](https://github.com/spider-rs/spider/blob/e4b53c5b582357e46433fa9ba3d91f930fb67cb4/spider/src/website.rs#L8188), [#L8233](https://github.com/spider-rs/spider/blob/e4b53c5b582357e46433fa9ba3d91f930fb67cb4/spider/src/website.rs#L8233), [configuration.rs#L1142-L1191](https://github.com/spider-rs/spider/blob/e4b53c5b582357e46433fa9ba3d91f930fb67cb4/spider/src/configuration.rs#L1142-L1191));
  - the budget map and the semaphore.
- **Per unit:** one visited-set probe per link.
  - The `ListBucket` interns URLs and, with feature `bloom`, puts an mmap bloom filter in front of the `HashSet` as a negative cache. It switches to bloom-only mode past `LINKS_VISITED_MEMORY_LIMIT` ([interner.rs#L40-L86](https://github.com/spider-rs/spider/blob/e4b53c5b582357e46433fa9ba3d91f930fb67cb4/spider/src/utils/interner.rs#L40-L86), [bloom.rs#L1-L11](https://github.com/spider-rs/spider/blob/e4b53c5b582357e46433fa9ba3d91f930fb67cb4/spider/src/utils/bloom.rs#L1-L11)).
  - Then one regex-set match and one robots `can_fetch`.

### 3.4 Is "off" free?

**Yes, and the code says so explicitly.**

- A feature compiled out costs nothing.
- At runtime, a `None` hook is documented as identical to the path without it:
  - "This branch is a no-op when `remote_fetcher` is `None` (the default)" ([website.rs#L8156-L8166](https://github.com/spider-rs/spider/blob/e4b53c5b582357e46433fa9ba3d91f930fb67cb4/spider/src/website.rs#L8156-L8166));
  - "Both `None` is byte-identical to the pre-watchdog code path" ([page.rs#L4729-L4730](https://github.com/spider-rs/spider/blob/e4b53c5b582357e46433fa9ba3d91f930fb67cb4/spider/src/page.rs#L4729-L4730));
  - "default off keeps … the default path is byte-identical" (sitemap budget, [website.rs#L2163-L2166](https://github.com/spider-rs/spider/blob/e4b53c5b582357e46433fa9ba3d91f930fb67cb4/spider/src/website.rs#L2163-L2166)).
  - Such "byte-identical" or "no-op when" notes recur: 13 in `website.rs`, 8 in `page.rs`, 8 in `utils/mod.rs` and 4 in `configuration.rs` (grep count).
- Further cases:
  - robots off: the parser is never built or fetched ([website.rs#L2231-L2244](https://github.com/spider-rs/spider/blob/e4b53c5b582357e46433fa9ba3d91f930fb67cb4/spider/src/website.rs#L2231-L2244));
  - an empty allow or block list is skipped with an `is_empty()` check;
  - with no subscriber, `channel_send_page` does no work beyond the `balance` byte accounting.
- **Cost of this style:** the combinations multiply. The earlier dossier showed `crawl_smart` silently degrading to `crawl` without the right features.

### 3.5 Persistence and logging per unit

- **Visited set on disk (feature `disk`).**
  - The set moves to SQLite only under memory pressure, past `LINKS_VISITED_MEMORY_LIMIT`, or in shared-disk mode ([website.rs#L1888-L1920](https://github.com/spider-rs/spider/blob/e4b53c5b582357e46433fa9ba3d91f930fb67cb4/spider/src/website.rs#L1888-L1920)).
  - In that mode each URL is **one autocommit `INSERT`** ([disk.rs#L203-L218](https://github.com/spider-rs/spider/blob/e4b53c5b582357e46433fa9ba3d91f930fb67cb4/spider/src/features/disk.rs#L203-L218)).
  - The pool is `connect_lazy` with no pragmas ([disk.rs#L94-L146](https://github.com/spider-rs/spider/blob/e4b53c5b582357e46433fa9ba3d91f930fb67cb4/spider/src/features/disk.rs#L94-L146)). With sqlx 0.8's defaults, it leaves `journal_mode` and `synchronous` at SQLite's defaults (rollback journal, `FULL`) and caches 100 prepared statements per connection ([sqlx-sqlite options v0.8.6](https://github.com/launchbadge/sqlx/blob/v0.8.6/sqlx-sqlite/src/options/mod.rs#L177-L199)). The exact sqlx 0.8 patch Spider resolves to is unverified.
  - Only `seed()` groups writes: chunks of 500 rows in one transaction ([disk.rs#L237-L282](https://github.com/spider-rs/spider/blob/e4b53c5b582357e46433fa9ba3d91f930fb67cb4/spider/src/features/disk.rs#L237-L282)).
  - **Disk is a memory overflow, not a journal.**
- **WARC (feature `warc`).**
  - Callers serialize a record and send it on an unbounded MPSC channel. One `spawn_blocking` task owns a `BufWriter` of 256 KiB and one file handle, and flushes on request or at the end, with no `sync_all` ([warc.rs#L1-L12](https://github.com/spider-rs/spider/blob/e4b53c5b582357e46433fa9ba3d91f930fb67cb4/spider/src/utils/warc.rs#L1-L12), [#L26](https://github.com/spider-rs/spider/blob/e4b53c5b582357e46433fa9ba3d91f930fb67cb4/spider/src/utils/warc.rs#L26), [#L397-L438](https://github.com/spider-rs/spider/blob/e4b53c5b582357e46433fa9ba3d91f930fb67cb4/spider/src/utils/warc.rs#L397-L438)).
  - A soft ceiling of 64 MiB of queued bytes with a low-water mark pauses the subscriber, and a 50 ms re-check cannot deadlock ([warc.rs#L540-L565](https://github.com/spider-rs/spider/blob/e4b53c5b582357e46433fa9ba3d91f930fb67cb4/spider/src/utils/warc.rs#L540-L565)).
- **No built-in stats journal.** Counts are the visited set's size and the budgets.

### 3.6 How Spider guards per-unit cost

- **Criterion micro-benchmarks of per-unit pieces** ([benches/core.rs](https://github.com/spider-rs/spider/blob/e4b53c5b582357e46433fa9ba3d91f930fb67cb4/benches/core.rs)):
  - `admission/10k_new` and `extend_links/1k_new_vs_10k_visited` (interner);
  - robots `parse` and `can_fetch`;
  - `convert_abs_path`;
  - `build_selectors/10_selectors` and `css_query_select_map/10_selectors_on_50kb`;
  - trie, anti-bot detection, and the clean-up levels.
- **Feature-level benches in the crate** ([spider/Cargo.toml#L206-L212](https://github.com/spider-rs/spider/blob/e4b53c5b582357e46433fa9ba3d91f930fb67cb4/spider/Cargo.toml#L206-L212)):
  - `streaming_link_extraction` quantifies one streaming pass against the old two-pass path ([streaming_link_extraction.rs#L1-L30](https://github.com/spider-rs/spider/blob/e4b53c5b582357e46433fa9ba3d91f930fb67cb4/spider/benches/streaming_link_extraction.rs#L1-L30));
  - `allocator_bench`.
- **CI** (`bench.yml`) runs only `cargo bench --bench crawl`, a comparison against other crawlers on a locally built tailwindcss.com site, with and without `decentralized`.
  - It stores no baseline and sets no threshold.
  - If the test site fails to clone or build, the bench steps are skipped and the job passes (`|| exit 0`, [bench.yml](https://github.com/spider-rs/spider/blob/e4b53c5b582357e46433fa9ba3d91f930fb67cb4/.github/workflows/bench.yml)).
  - `BENCHMARKS.md` itself warns that GitHub Actions results "may differ across runs" ([BENCHMARKS.md](https://github.com/spider-rs/spider/blob/e4b53c5b582357e46433fa9ba3d91f930fb67cb4/benches/BENCHMARKS.md)).
- **No regression gate, and no written policy.**

---

## 4. Patterns StormCrawler and Nutch show that the three lack

- **Admission cache in front of the persistent frontier, with its own metrics.**
  - StormCrawler's `AbstractStatusUpdaterBolt` drops a `DISCOVERED` URL already in a Caffeine cache before touching the backend. The cache is on by default, with spec `expireAfterAccess=1h,softValues` ([AbstractStatusUpdaterBolt.java#L52-L65](https://github.com/apache/stormcrawler/blob/stormcrawler-3.7.0/core/src/main/java/org/apache/stormcrawler/persistence/AbstractStatusUpdaterBolt.java#L52-L65), [#L151-L168](https://github.com/apache/stormcrawler/blob/stormcrawler-3.7.0/core/src/main/java/org/apache/stormcrawler/persistence/AbstractStatusUpdaterBolt.java#L151-L168), [crawler-default.yaml#L304-L305](https://github.com/apache/stormcrawler/blob/stormcrawler-3.7.0/core/src/main/resources/crawler-default.yaml#L304-L305)).
  - It exports `cache.hits`, `cache.misses` and `cache.size` every 30 s ([#L114-L132](https://github.com/apache/stormcrawler/blob/stormcrawler-3.7.0/core/src/main/java/org/apache/stormcrawler/persistence/AbstractStatusUpdaterBolt.java#L114-L132)).
  - Crawlee has the cache (`requestCache`, the dedup cache) but no hit and miss counters. Scrapy's dupefilter is the whole set.
- **Grouped writes, acknowledged after the group commits.**
  - The OpenSearch `StatusUpdaterBolt` feeds a `BulkProcessor`. The code defaults are `bulkActions` 50 and `flushInterval` 5 s; the shipped sample configuration uses 500, 5 s and `concurrentRequests` 1 ([OpenSearchConnection.java#L264-L276](https://github.com/apache/stormcrawler/blob/stormcrawler-3.7.0/external/opensearch/src/main/java/org/apache/stormcrawler/opensearch/OpenSearchConnection.java#L264-L276), [opensearch-conf.yaml#L75-L77](https://github.com/apache/stormcrawler/blob/stormcrawler-3.7.0/external/opensearch/opensearch-conf.yaml#L75-L77)).
  - Tuples are acked only from `afterBulk`, through a `WaitAckCache`, and the bolt counts `bulks_sent`, `bulks_received` and `bulk_msec` ([StatusUpdaterBolt.java#L151-L163](https://github.com/apache/stormcrawler/blob/stormcrawler-3.7.0/external/opensearch/src/main/java/org/apache/stormcrawler/opensearch/persistence/StatusUpdaterBolt.java#L151-L163), [#L284-L327](https://github.com/apache/stormcrawler/blob/stormcrawler-3.7.0/external/opensearch/src/main/java/org/apache/stormcrawler/opensearch/persistence/StatusUpdaterBolt.java#L284-L327)).
  - This is the closest open-source analogue of Goodfetch's "nothing told outside before its group is durable".
- **Filter chains ordered cheap-first by configuration, instances cached per configuration.**
  - Nutch's `URLFilters` runs its plugins as a logical AND: "To control performance, the URLFilter classes can be ordered by the property `urlfilter.order`" ([URLFilters.java#L27-L61](https://github.com/apache/nutch/blob/release-1.23/src/java/org/apache/nutch/net/URLFilters.java#L27-L61)).
  - `PluginRepository.getOrderedPlugins` caches the instances in `ObjectCache`, keyed by the configuration ([PluginRepository.java#L436-L489](https://github.com/apache/nutch/blob/release-1.23/src/java/org/apache/nutch/plugin/PluginRepository.java#L436-L489)).
  - A plugin left out of `plugin.includes` is never loaded.

---

## 5. Cross-project synthesis

1. **One owner per path: an ordered chain built once at start, walked per unit.**
   - Scrapy has four managers with precomputed method deques. Crawlee has hook arrays and a `Router` map. Nutch has `ObjectCache`'d plugin arrays.
   - Spider instead has one owner, `Website`, with compile-time variants and `Option` hooks.
   - **Where they disagree:** Scrapy's registry is open and priority-ordered, and any component can join any chain. Crawlee offers fixed extension points. Spider has no extension point inside the loop beyond callbacks.
2. **"Off" is made free by construction, not by an early `if` inside shared code.**
   - Scrapy and Nutch remove the component from the chain (`NotConfigured`, `None`, `plugin.includes`). Spider compiles it out, or documents `None` as byte-identical. Crawlee guards each step's first line.
   - All three leave some residue. Scrapy: the `DEBUG` log per response and per item, signals with no receivers, the offsite regex even with no domains. Crawlee: context closures and `ow` checks per request. Spider: a combinatorial feature matrix.
3. **Run invariants are compiled once; the strong form is a process-wide bounded cache keyed by the source text.**
   - parsel keeps an LRU of 256 CSS→XPath translations and 2,048 compiled XPaths. Spider keeps `OnceLock` selectors. Scrapy's `LinkExtractor` and offsite regex, Spider's `RegexSet`, and Crawlee's pools and robots LRU of 1,000 are built per run.
   - **Counter-example:** Crawlee caches its pattern inputs but compiles a `Minimatch` per page.
4. **Per-unit derived values are memoised on the unit, so readers share one result.**
   - Scrapy keys `WeakKeyDictionary` caches on the `Request` or `Response`: fingerprint, URL parse, base URL, meta refresh, selector. Spider computes links inline during the one streaming pass.
   - Crawlee's `HttpCrawler.parseWithCheerio` does not memoise, and re-parses on each call.
5. **Admission is a cheap-first sequence on a single owner, with an in-memory set before any store.**
   - Spider: visited set (bloom, then `HashSet`), then regex set, then robots, then depth, then budget, all in the coordinator. Scrapy: offsite, then fingerprint set, then queue push, all on the reactor thread. Crawlee: depth, robots and schema, then the LRU and the dedup cache, then one batched storage call per 1,000. StormCrawler: a discovered cache before the backend.
   - None of the three takes a lock on this path: one thread or one coordinator owns the frontier state.
6. **Only Crawlee validates payloads at admission.**
   - It validates `userData` against a Standard Schema per label, at add and again at dispatch, and stores the coerced value.
   - Scrapy and Spider validate no request payload. None of the three validates dataset records against a schema; Scrapy leaves that to user pipelines.
7. **Records go through a chain, then a sink that keeps one handle and one encoder per run.**
   - Scrapy writes one buffered line per item to a file opened once. Spider's WARC writer owns one 256 KiB `BufWriter`.
   - Crawlee JS writes one file per item, each with a lock, and that is the outlier. Crawlee Python SQL writes one multi-row `INSERT` per push.
8. **Persistence: per-unit unsynced writes plus snapshots, or grouped writes acknowledged after the group.**
   - Scrapy (a pickle push and a fingerprint append per request, state at clean close) and Crawlee JS (about three JSON files per request, state every 60 s) write per unit and never sync.
   - Grouping appears in StormCrawler (bulk of 50–500 or 5 s, ack after the bulk) and in Spider's `seed` (500 rows per transaction).
   - **Hot-row avoidance** appears in Crawlee Python SQL (a metadata buffer table instead of rewriting the counter row on every push), the same problem as Foxhound's `journal_position`.
9. **Stats and logs: per unit, only a counter increment; formatting and I/O on an interval or in one writer.**
   - Scrapy stats are a dict, logged every 60 s and dumped at close. Crawlee counts in a Map, logs every 60 s and 10 s and persists every 60 s. StormCrawler reports metrics every 30 s. Spider formats a log line only when its level is enabled.
   - **Disagreement:** Scrapy's default `DEBUG` level formats a line per response and per item.
10. **Backpressure is counted in bytes, not units.**
    - Scrapy backs out above 5 MB of response bodies held in the scraper. Spider spools large pages to disk under `balance` and pauses WARC above 64 MiB queued.
11. **Cost guards: only Scrapy gates per-unit cost in CI, and nobody gates structure.**
    - Scrapy's CodSpeed suite isolates per-request, per-item, parse and admission costs. It counts instructions deterministically with a null download handler, and asserts the workload still happens.
    - Spider has the right micro-benchmarks (admission, robots, selector build, one pass against two), but CI runs only an ungated cross-crawler comparison that can skip silently.
    - Crawlee has none.
    - **No project counts structural events per unit** (parses, compilations, prepared statements, commits, syncs) or publishes a regression policy. ADR 0138's structural ceilings go beyond all three.

## 6. Mapping to Foxhound's paths

| Goodfetch path | Pattern that applies | What a competitor counts per unit or keeps once | Benchmark to copy |
|---|---|---|---|
| **F1 Page analysis** (one parse per page, shared by readers) | Synthesis 4 and 3: memoise on the unit, compile selectors once, process-wide; a streaming pass when no tree is needed | Scrapy: at most 1 lxml tree per response (`_cached_selector`), plus w3lib regex scans for `<base>` and meta refresh, each cached per response; 0 selector compilations in steady state (parsel LRUs). Spider: 0 trees, 1 streaming `lol_html` pass, plus a hash pass when `normalize` is set; selectors compiled once in `OnceLock`. Crawlee JS: 2 parses on `CheerioCrawler`; `HttpCrawler` 1 per `parseWithCheerio` call | Scrapy `test_parse_engine`: a saved real snapshot behind a null handler, with an assertion that the selectors still match. Spider `streaming_link_extraction` (one pass against two) and `css_query_select_map/10_selectors_on_50kb` |
| **F2 Run-state writer** (commits per attempt) | Synthesis 8: group commits and acknowledge after the group (StormCrawler `WaitAckCache`); keep handles and statements open; buffer hot counters (Crawlee Python) | Scrapy (`JOBDIR`): 1 pickle write per enqueue, 1 fingerprint append per new request, 1 truncating pop, 0 syncs, state at clean close. Crawlee JS: about 3 locked JSON file writes per request, 0 syncs, state every 60 s. Spider: 0 writes per URL unless spilling, then 1 autocommit `INSERT` (rollback journal, `synchronous=FULL` under SQLite defaults). Kept once: open queue and fingerprint files (Scrapy), a 100-statement cache per connection (sqlx) | None of the three benchmarks persistence: Scrapy's suite runs without `JOBDIR`. Goodfetch's own counts (transactions and syncs per page) are already stricter than any competitor's |
| **F3 Frontier admission** (per link batch: scope, payload validation) | Synthesis 5 and 6: cheap-first order on a single owner; in-memory dedup before any store; patterns compiled once per run; validators compiled once and keyed by label | Scrapy: 1 SHA-1 fingerprint (memoised) and 1 set probe; offsite regex ×2. Spider: 1 visited probe (bloom, then set), 1 `RegexSet` match, 1 robots `can_fetch`, depth and budget checks. Crawlee: 1 `Request` construction (`ow` and `normalizeUrl`), 1 SHA-256 ID, an LRU and a dedup-cache probe, an optional schema validation (twice per request), 1 storage call per 1,000; `Minimatch` compiled per page (avoid). StormCrawler and Nutch: cache hit and miss metrics; filters ordered by cost | Scrapy `test_fingerprints`, `test_requests`, `test_overhead_broad`. Spider `admission/10k_new`, `extend_links/1k_new_vs_10k_visited`, robots `can_fetch` |
| **F4 Per Record** (Dataset validation and commit) | Synthesis 7: a chain built once, one sink handle and one encoder per run, with commit grouped with the attempt (ADR 0138) | Scrapy: the pipeline chain, plus 3 signals around it, 1 JSON encode and 1 buffered line write; no schema validation built in. Crawlee JS: 1 `JSON.stringify` for the size check, then 1 file and 1 lock per item; Python SQL: 1 multi-row `INSERT` per push and 1 buffer row. Kept once: Scrapy's encoder and file handle. No competitor compiles a record schema, so Goodfetch's "validator compiled at crawl start" has no direct precedent beyond Crawlee's per-label request schemas | Scrapy `test_overhead_items` (1, 100 and high-limit items per response) and `test_overhead_item_concurrency` |
| **E3 Per-request usage charge** | Synthesis 9: per unit, an in-memory counter; persistence at the attempt's commit or on an interval | Scrapy: `stats.inc_value` dict updates (`scheduler/enqueued`, `response_received_count`…), dumped at close. Crawlee: `startJob`/`finishJob` Map and counters, persisted every 60 s. StormCrawler: counters reset every 30 s. No open-source crawler here charges per request; Apify platform billing is outside Crawlee and was not examined. A charge that rides in the outcome transaction (ADR 0138) adds no write, as competitors' counters add none | Scrapy `test_overhead_engine` (per-request overhead with stats on) is the shape: per-request cost with the counter path enabled and no I/O |
| **E5 Per trace or provenance entry** | Synthesis 9 and 10: format lazily, gated by level; one writer task owning one buffered handle; a bounded byte queue with hysteresis | Spider WARC: 1 record serialized by the caller, then 1 channel send; 1 writer task, a 256 KiB `BufWriter`, a 64 MiB soft ceiling, no sync. Spider log: 0 lines per page (failures only). Scrapy: 1 formatted log line per response and per item at the default `DEBUG`, 0 at `INFO` or above. Crawlee: 0 per request; intervals of 10 s and 60 s | No competitor benchmarks trace cost. The nearest are Scrapy's benchmarks, which disable logging (`LOG_ENABLED: False`): measure with provenance on and off as two cells |

## Unverified and limits

- Whether Scrapy's CodSpeed check is a required status for merging (repository settings are not public), and whether any written regression threshold exists.
- The exact sqlx 0.8.x patch Spider resolves to. The pragma defaults were read at v0.8.6.
- Which Spider page branches reach the per-page `element!("script")` selector parses.
- Apify platform pay-per-event charging: not examined. It is not part of Crawlee's code.
- Crawlee `4.0.0-rc.1` and Crawlee's development branch: not examined. ADR 0138 notes the development branch parses once.
- No benchmark of any project was run here. Every "counted per unit" figure above is read from code paths, not measured.
