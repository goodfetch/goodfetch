# Consumer API of crawl and scraping frameworks: Crawlee, Scrapy, Spider-rs and peers

State observed on **2026-10-14**. Purpose: inform the developer-facing API of Goodfetch's Foxhound (crawling and extraction at scale) and, where it overlaps, Collie (one page with AI primitives). The question is not "which engine is fastest" (see the earlier crawl engines audit ([`2026-10-07-crawl-engines.md`](../../2026-10-07-crawl-engines.md)) and capability matrix (`rnd/research/2026-10-07-crawl-capability-matrix.md`)), but **what a developer writes, sees, configures and fights with** when using these frameworks, and what users praise and complain about.

Everything below about Goodfetch is analysis, not decision. Nothing here is an adopted term.

---

## 1. Sources and method

### 1.1 Versions pinned (registries queried 2026-10-14)

| Project | Version used for API claims | Published | Pin |
|---|---|---|---|
| Crawlee JS/TS (`crawlee`, `@crawlee/*`) | **3.18.2** stable; **4.0.0-rc.1** for the v4 changes | 3.18.2: 2026-09-29; 4.0.0-rc.1: 2026-10-06 (4.0.0-beta.0 was 2025-05-20) | tag `v3.18.2` = `f92bb005`; `v4.0.0-rc.1` = `8f57dd3c`; upgrade guide read on `master` = `8cb2eb61` |
| Crawlee Python (`crawlee`) | **1.10.4** | 2026-10-06 (1.0.0 was 2025-09-29) | tag `v1.10.4` = `df1fe086` |
| Scrapy | **2.19.0** | 2026-09-10 | tag `2.19.0` |
| scrapy-playwright | 0.0.48 | 2026-07-10 | `main` README |
| scrapy-poet / web-poet | 0.27.2 / 0.24.1 | 2026-04-28 | `master` docs |
| Spider (`spider` crate) | **2.53.9** | 2026-09-05 | published crate SHA `e4b53c5b` (same as prior audits) |
| spider-nodejs (`@spider-rs/spider-rs`) | 0.0.163 | 2026-01-24 | `main` |
| spider-py (`spider_rs` on PyPI) | 0.0.57 | **2025-01-28** | `main` (repo says 0.0.58, unpublished) |
| spider.cloud clients (`spider_client` PyPI, `@spider-cloud/spider-client` npm) | 0.1.93 / 0.2.5 | 2026-07-18 / 2026-07-19 | `spider-rs/spider-clients` `main` |
| Colly | v2.3.0 (Go proxy); last GitHub Release v2.2.0 | v2.2.0: 2025-03-27 | tag `v2.3.0` |
| Crawl4AI | 0.9.5 | 2026-10-14 (released today) | tag `v0.9.5` |
| Firecrawl SDKs (`firecrawl-py` / `firecrawl` PyPI, `@mendable/firecrawl-js` / `firecrawl` npm) | Py 4.50.0, JS 4.46.0 | 2026-10-08 | `main` SDK READMEs and `v2/types.py` |
| Scrapling | 0.4.15 | 2026-08-23 | tag `v0.4.15` |
| Katana | v1.8.0 | 2026-10-05 | tag `v1.8.0` |
| Apify SDK | Python `apify` 4.1.0 (2026-10-08); JS `apify` 3.7.2 (2026-05-11) | | |

Repository stars (GitHub API, 2026-10-14), for scale only: Firecrawl 191k, Scrapling 87k, Crawl4AI 86k, Scrapy 65k, Crawlee JS 26k, Colly 26k, Katana 18k, Crawlee Python 9.6k, Spider 2.8k. Stars are not usage.

### 1.2 What was read

- **Docs and source at the tags above**: crawling-context interfaces (Crawlee `crawler_commons.ts`, `http-crawler.ts`, `browser-crawler.ts`, `playwright-utils.ts`; crawlee-python `_types.py` and per-crawler contexts), crawler constructors and defaults, Crawlee v4 upgrade guide (12 files, about 200 KB), crawlee-python v1 upgrade guide and its *Migrating from Scrapy* guide, Scrapy release notes 2.12–2.19, `default_settings.py`, project template, `jobs.rst`, `asyncio.rst`, `contracts.rst`, `agents.rst`; scrapy-playwright README; scrapy-poet tutorial and web-poet fields/testing docs; Spider README, `Cargo.toml` (132 features), `website.rs`, `page.rs`; spider-nodejs/spider-py READMEs; Firecrawl Python/JS SDK READMEs and `CrawlRequest` type; Crawl4AI deep-crawling docs; Scrapling spiders docs (getting started, sessions, advanced); Katana library section; Colly `colly.go`.
- **GitHub issues**: top 50 per repo by reactions and top 40 by comments via `gh api search/issues` for apify/crawlee, apify/crawlee-python, scrapy/scrapy, spider-rs/spider, spider-nodejs, spider-py, scrapy-playwright, scrapy-poet, firecrawl, crawl4ai, Scrapling, colly, katana, apify-sdk-js, apify-sdk-python; then about 60 issues read in full with maintainer replies (marked MEMBER/COLLABORATOR/OWNER), plus targeted keyword searches (adaptive, resume, v4, ReactorNotRestartable, start_requests, SDK, stuck, feature flags).
- **GitHub Discussions** via GraphQL (all discussions for Crawlee JS 341, Crawlee Python 71, Scrapy 24, Spider 34, Crawl4AI 431, Firecrawl 73, Scrapling 47; Colly has Discussions disabled), ranked locally by upvotes, reactions and comments.
- **Forums** (Hacker News Algolia, Stack Exchange API, Reddit, independent blogs): collected in a separate pass; see §5–§6 and the notes file [forum-evidence.md](forum-evidence.md) in the same scratchpad folder.
- Prior Goodfetch research reused and re-verified where repeated: `rnd/research/2026-10-07-crawl-engines.md`, `-crawlee-spider-contracts-followup.md`, `-stagehand-crawlee-spider-comparison.md`, `-reddit-evidence-followup.md`, `-crawl-capability-matrix.md`. The parallel `research/crawl-framework-products` branch had **no draft** (worktree clean at `origin/main`, branch not on `origin`).

### 1.3 Rules applied

- Counts are reactions (`r`) and comments (`c`) as returned by the API on 2026-10-14. A reaction is weak evidence; "independent reports" counts distinct issues/threads by different authors.
- Quotes are under 15 words, copied from the page that was opened.
- **Unverified** marks anything not confirmed in a primary source.
- Vendor-authored material (Apify about Crawlee, Zyte about Scrapy, spider.cloud about Spider, Firecrawl, Crawl4AI) is flagged.

---

## 2. Framework walkthroughs

### 2.1 Crawlee for JavaScript/TypeScript (Apify)

**Entry point.** A crawler object per fetch tier (`BasicCrawler`, `HttpCrawler`, `CheerioCrawler`, `JSDOMCrawler`, `LinkeDOMCrawler`, `PlaywrightCrawler`, `PuppeteerCrawler`, `AdaptivePlaywrightCrawler`, `StagehandCrawler`, `FileDownload`), configured by one options object, with a `requestHandler` function or a **router** (`createCheerioRouter()`, `createPlaywrightRouter()`) that dispatches on `request.label`. `await crawler.run(urls)` resolves with final statistics.

```ts
// Crawlee 3.18 style (TypeScript, ESM)
import { PlaywrightCrawler, createPlaywrightRouter, Dataset } from 'crawlee';

const router = createPlaywrightRouter();

router.addDefaultHandler(async ({ enqueueLinks, log }) => {
  log.info('listing');
  await enqueueLinks({ selector: 'a.product', label: 'DETAIL' });   // same-hostname by default
  await enqueueLinks({ selector: 'a.next' });                         // pagination, default label
});

router.addHandler('DETAIL', async ({ request, page, pushData }) => {
  await pushData({
    url: request.loadedUrl,
    title: await page.locator('h1').textContent(),
    price: await page.locator('.price').textContent(),
  });
});

const crawler = new PlaywrightCrawler({
  requestHandler: router,
  maxRequestsPerCrawl: 500,
  maxConcurrency: 20,
  failedRequestHandler: async ({ request, log }, error) =>
    log.error(`gave up on ${request.url}: ${error.message}`),
});

await crawler.run(['https://shop.example.com/']);
await Dataset.exportToCSV('products');
```

Moving the same crawl to plain HTTP means changing the class and the router factory to `CheerioCrawler`/`createCheerioRouter`, and rewriting every `page.locator(...)` to `$('...')`. The browser contexts also expose `parseWithCheerio()` so Cheerio-style code can be shared.

**Handler context (verified in 3.18.2 source).** Common to all crawlers (`crawler_commons.ts`): `id`, `request`, `session`, `proxyInfo`, `log`, `pushData()`, `enqueueLinks()`, `addRequests()`, `useState()`, `getKeyValueStore()`, `sendRequest()`, and `crawler` (removed in v4). HTTP crawlers add `body`, `json`, `contentType`, `response`, `waitForSelector()`, `parseWithCheerio()`; Cheerio adds `$`. Browser crawlers add `page`, `response`, `browserController`; Playwright adds utilities `injectFile`, `injectJQuery`, `blockRequests`, `infiniteScroll`, `saveSnapshot`, `enqueueLinksByClickingElements`, `compileScript`, `closeCookieModals`, `handleCloudflareChallenge`, `listDownloads`. The **context shape changes per tier**, which is exactly what makes tier switching a code change ([crawler_commons.ts](https://github.com/apify/crawlee/blob/v3.18.2/packages/core/src/crawlers/crawler_commons.ts), [http-crawler.ts](https://github.com/apify/crawlee/blob/v3.18.2/packages/http-crawler/src/internals/http-crawler.ts), [playwright-utils.ts](https://github.com/apify/crawlee/blob/v3.18.2/packages/playwright-crawler/src/internals/utils/playwright-utils.ts)).

**Enqueueing and scope.** `enqueueLinks({ selector, strategy, globs, regexps, pseudoUrls, exclude, label, userData, transformRequestFunction, limit })`; strategies `all`, `same-domain`, `same-hostname` (default), `same-origin`. In v3, giving any `globs/regexps/pseudoUrls` **replaced** the strategy, a frequent source of surprise. Deduplication is by `uniqueKey` (normalized URL by default, overridable). Data travels to the next handler in `request.userData`; the label is `userData.label`. `maxCrawlDepth` and `maxRequestsPerCrawl` cap the crawl. `respectRobotsTxtFile` exists and defaults to **false** ([basic-crawler.ts L646–L662](https://github.com/apify/crawlee/blob/v3.18.2/packages/basic-crawler/src/internals/basic-crawler.ts#L646)).

**Output.** Append-only `Dataset` (`pushData`, `exportToCSV/JSON`, `crawler.exportData()`), `KeyValueStore` (records, `useState` auto-saved state, `getAutoSavedValue`), `RequestQueue`. Local storage lives in `./storage` (`CRAWLEE_STORAGE_DIR`), persisted to disk, and **purged at start** by default (`purgeOnStart=true`). No item pipeline or schema; the Apify SDK adds dataset schemas on the platform side.

**Configuration.** Constructor options plus a global `Configuration` (env vars `CRAWLEE_*`, optional `crawlee.json`). Defaults confirmed in 3.18.2: `maxRequestRetries=3`, `sameDomainDelaySecs=0`, `retryOnBlocked=false`, `respectRobotsTxtFile=false`, `useSessionPool=true`, `maxSessionRotations=10`; autoscaling via `AutoscaledPool` over CPU, memory, event loop and client load. In v3, **environment variables override constructor options**; v4 reverses this.

**Tier switching.** Three routes: (1) change crawler class and rewrite selectors; (2) keep a browser crawler and use `parseWithCheerio()`; (3) `AdaptivePlaywrightCrawler`, a restricted context (`querySelector`, `waitForSelector`, `parseWithCheerio`, `enqueueLinks`, `pushData`, `useState`, `getKeyValueStore`) where touching `page` in HTTP mode throws and triggers browser fallback. A logistic-regression predictor per label/URL decides HTTP vs browser; `renderingTypeDetectionRatio` (0.1) re-checks; `resultChecker`/`resultComparator` validate; storage writes are buffered so the losing attempt leaves nothing behind ([adaptive-playwright-crawler.ts](https://github.com/apify/crawlee/blob/v3.18.2/packages/playwright-crawler/src/internals/adaptive-playwright-crawler.ts)). Prior audit details stand.

**Lifecycle.** `run()` → statistics; `crawler.stop()` for graceful stop; `teardown()`; persisted state (`persistStateIntervalMillis`) allows resume when the purge is disabled or a named queue is used. Resume semantics are a documented weak point (§6.3).

**Errors.** `errorHandler` (before each retry), `failedRequestHandler` (after retries), error classes `NonRetryableError`, `CriticalError` (stops crawl), `MissingRouteError`, `RetryRequestError`, `SessionError` (rotates session) ([errors.ts](https://github.com/apify/crawlee/blob/v3.18.2/packages/core/src/errors.ts)). A failed request is marked handled.

**Extensibility.** `preNavigationHooks` / `postNavigationHooks`, BrowserPool hooks, custom HTTP client (`got-scraping`, `impit`), custom storage client, router middleware (`router.use`), proxy configuration with `newUrlFunction`, `SessionPool`. No plugin registry; extension is by composition and subclassing (v4 replaces subclassing with `extendContext`).

**Testing and debugging.** No official test harness or fixture recorder; users mock or hit local servers. Debugging via `headless: false`, `CRAWLEE_VERBOSE_LOG`, `saveSnapshot()`, statistics logs. v4 adds an `@crawlee/otel` package for OpenTelemetry tracing.

**Typing.** TypeScript-first; generic `UserData` on `Request` and routers. In v3 the context extends a `Record`, so any property access type-checks; v4 makes it strict.

**Crawlee v4 (RC since 2026-08-13; rc.1 2026-10-06) is a large consumer-API rewrite driven by known pains.** From the official upgrade guide on `master` ([upgrading-v4.md](https://github.com/apify/crawlee/blob/master/docs/upgrading/upgrading-v4/upgrading-v4.md)); `master` may be ahead of rc.1:

| v4 change | The v3 pain it addresses |
|---|---|
| Navigation and handler timed separately; per-route `requestHandlerTimeoutSecs`; `context.extendTimeout()`; internal whole-request timeout (≥5 min) | "Setting `requestHandlerTimeoutSecs: 60` … produced errors complaining about 130 seconds"; hooks had no timeout; requests could hang forever |
| **Storage writes in a handler are transactional**: buffered and committed only if the handler succeeds; `afterStorageCommit()`; `withDirectStorageAccess()` escape hatch; commit is at-least-once | Duplicate or half-written results on retry (cf. [#1216](https://github.com/apify/crawlee/issues/1216)) |
| Each crawler instance gets its own default queue; repeated `run()` no longer empties the queue; a crawl that processes nothing **warns and says why**; new `purge()` | Second run silently crawls nothing or everything ([#1602](https://github.com/apify/crawlee/issues/1602), Discussion [#3254](https://github.com/apify/crawlee/discussions/3254)) |
| `enqueueLinks({ include })` replaces `globs/regexps/pseudoUrls`; `include` is AND-ed with `strategy` (default same-hostname), "mirrors Crawlee for Python"; `extractLinks()` without enqueue; `context.skipRequest()` | Patterns silently replacing strategy; no way to skip without faking a failure |
| Constructor options beat env vars; `Configuration` immutable; per-crawler `ServiceLocator` | Env var silently overriding code |
| Strictly typed context; `extendContext` replaces subclassing; `context.crawler` removed | Untyped `Record` context |
| Default HTTP client `impit` (Rust, TLS impersonation); `Response` is the standard Fetch `Response` | `got-scraping` unmaintained |
| One `ConcurrencySystem` shareable across crawlers; `SessionPool` lazy; "the session is the rotation unit" | Several crawlers oversubscribing the host |
| ESM only, Node 22.13+, TS 5.8+, native optional deps (no `--omit=optional`) | Packaging debt (cost: a migration wall) |
| robots.txt 4xx/5xx per RFC 9309; `sameDomainDelaySecs` backed by `ThrottlingRequestManager` (throws above 100 domains by default) | Politeness semantics |

The guide states plainly that there are "many" breaking changes and splits them into "most users will hit" vs "Only if you…" sections, an approach worth copying. Early v4 RC issues (e.g. [#4241](https://github.com/apify/crawlee/issues/4241) "RequestList persisted state is not picked up from v3", [#4240](https://github.com/apify/crawlee/issues/4240) FileDownload "silently started verifying TLS certificates") show the cost of renaming persisted keys. The Apify SDK side tracked a "compatibility layer for v3 `enqueueLinks` options on Crawlee v4" ([apify-sdk-js#687](https://github.com/apify/apify-sdk-js/issues/687)).

### 2.2 Crawlee for Python (Apify)

**Entry point.** Same model, Pythonic: crawler classes (`BasicCrawler`, `HttpCrawler`, `BeautifulSoupCrawler`, `ParselCrawler`, `PlaywrightCrawler`, `AdaptivePlaywrightCrawler`, `StagehandCrawler`, `FileDownload`, and the **experimental `PydanticAiCrawler`**), handlers registered with decorators on `crawler.router`.

```python
# Crawlee for Python 1.10 style
import asyncio
from urllib.parse import urljoin
from crawlee import Request
from crawlee.crawlers import ParselCrawler, ParselCrawlingContext

crawler = ParselCrawler(max_requests_per_crawl=500, max_request_retries=3)

@crawler.router.default_handler
async def listing(ctx: ParselCrawlingContext) -> None:
    # per-link data needs explicit Request objects; enqueue_links only takes one user_data for all links
    await ctx.add_requests([
        Request.from_url(urljoin(ctx.request.url, card.css('a::attr(href)').get()),
                         label='DETAIL',
                         user_data={'list_price': card.css('.price::text').get()})   # must be JSON-serializable
        for card in ctx.selector.css('div.card')
    ])
    await ctx.enqueue_links(selector='a.next')   # resolves relative URLs, same-hostname by default

@crawler.router.handler('DETAIL')
async def detail(ctx: ParselCrawlingContext) -> None:
    await ctx.push_data({
        'url': ctx.request.url,
        'title': ctx.selector.css('h1::text').get(),
        'list_price': ctx.request.user_data.get('list_price'),
    })

@crawler.failed_request_handler
async def failed(ctx, error: Exception) -> None:
    ctx.log.error(f'gave up on {ctx.request.url}: {error}')

asyncio.run(crawler.run(['https://shop.example.com/']))
```

`user_data` must be JSON-serializable: a maintainer confirmed values were silently stringified ("you need to put JSON-serializable data into `user_data`") in [#524](https://github.com/apify/crawlee-python/issues/524) (20 c), the most-commented crawlee-python issue, titled "Implement/document a way how to pass information between handlers".

**Context (verified, `_types.py` at v1.10.4).** `BasicCrawlingContext`: `request`, `session`, `proxy_info`, `send_request`, `add_requests`, `push_data`, `use_state`, `get_key_value_store`, `log` (stdlib `logging.Logger`), `register_deferred_cleanup`. HTTP: `http_response`, plus `parsed_content` and `enqueue_links`/`extract_links` on parsed crawlers; `selector` (Parsel) or `soup` (BeautifulSoup). Playwright: `page`, `infinite_scroll`, `block_requests`, `goto_options` (pre-navigation), `enqueue_links`, `extract_links`. Adaptive: `page`, `response` and `infinite_scroll` raise `AdaptiveContextError` in static mode; `query_selector_one/all`, `wait_for_selector`, `parse_with_static_parser` work in both ([adaptive context](https://github.com/apify/crawlee-python/blob/v1.10.4/src/crawlee/crawlers/_adaptive_playwright/_adaptive_playwright_crawling_context.py)).

**AI extraction inside the crawl handler (new, experimental).** `PydanticAiCrawler` adds `context.extract(Model)` returning a validated Pydantic instance and `ai_usage` token stats; default model string `'openai:gpt-5.4-nano'`; a `PydanticAiSelectorExtractor` asks the model for a **tree of Parsel CSS selectors** mirroring the schema and caches them per `(schema, scope, cache_tag)` and markup variant, so repeat pages cost "near-zero LLM" ([crawler source](https://github.com/apify/crawlee-python/blob/v1.10.4/src/crawlee/crawlers/_pydantic_ai/_pydantic_ai_crawler.py), [selector extractor](https://github.com/apify/crawlee-python/blob/v1.10.4/src/crawlee/crawlers/_pydantic_ai/_selector_extractor.py)). It warns at construction that its API may change. Directly comparable to a Goodfetch "extract with schema, compile to selectors" primitive.

**Storage.** v1.0 introduced explicit storage clients: memory, file system, **SQL** (SQLAlchemy; `FOR UPDATE SKIP LOCKED` on PG/MySQL), **Redis** (Lua claims) ([storage_clients](https://github.com/apify/crawlee-python/tree/v1.10.4/src/crawlee/storage_clients)). JS has memory/file only (v4: `fs-storage` native).

**Parity with JS.** Python is not a line-for-line port: it has SQL/Redis storage, PydanticAI crawler, BeautifulSoup/Parsel instead of Cheerio/JSDOM, no Puppeteer, a REST-based Stagehand integration (prior audit). It is now **chasing JS v4 semantics**: [#2305](https://github.com/apify/crawlee-python/issues/2305) (2026-10-13) "Stop purging the request queue on repeated `run()` calls… JS v4 stopped doing that"; [#2133](https://github.com/apify/crawlee-python/issues/2133) "Share one concurrency budget between multiple crawlers". JS v4 in turn adopted Python's `include`+strategy AND semantics. Parity is maintained by humans filing mirror issues, not by a shared core.

**v1.0 (2025-09-29) breaking changes** ([upgrading_to_v1.md](https://github.com/apify/crawlee-python/blob/v1.10.4/docs/upgrading/upgrading_to_v1.md)): default HTTP client switched from `httpx` to `impit`; storages reworked (`get_metadata`, `purge`, `export_to`; `Request.id` removed; queue keyed by `unique_key`); new storage-client architecture; `ServiceLocator` stricter; "browser" terminology split between Playwright and fingerprint contexts.

**Official Scrapy-to-Crawlee mapping (vendor-authored).** The Python docs ship a *Migrating from Scrapy* guide with a concept table: `scrapy.Spider`→`ParselCrawler`, `callback=`→`@router.handler(label)`, `cb_kwargs`/`meta`→`Request.user_data`, `yield {...}`→`push_data`, `scrapy.Item`→"A plain `dict`", item pipelines→`Dataset`, middlewares→`router.use()`/hooks/HTTP clients, `settings.py`→`Configuration` + crawler arguments, AutoThrottle→`ThrottlingRequestManager`, `errback`→`failed_request_handler`, `scrapy-playwright`→`PlaywrightCrawler`, `scrapy crawl`→`python main.py` ([scrapy_migration.mdx](https://github.com/apify/crawlee-python/blob/v1.10.4/docs/guides/scrapy_migration.mdx)). It is the clearest single statement of where the two models line up, and where Crawlee simply has no equivalent (typed items, pipelines).

### 2.3 Scrapy (Zyte and community)

**Entry point.** Subclass `scrapy.Spider` (or `CrawlSpider`, `SitemapSpider`, `XMLFeedSpider`, `CSVFeedSpider`); callbacks are generators that **yield** items and `Request`s; run with `scrapy crawl name` inside a project (settings module) or from a script with `CrawlerProcess`/`CrawlerRunner` (and since 2.13+ `AsyncCrawlerProcess`/`AsyncCrawlerRunner`).

```python
# Scrapy 2.19 style
import scrapy

class ProductsSpider(scrapy.Spider):
    name = "products"
    allowed_domains = ["shop.example.com"]
    custom_settings = {"DOWNLOAD_DELAY": 1, "CONCURRENT_REQUESTS_PER_DOMAIN": 1,
                       "FEEDS": {"products.jsonl": {"format": "jsonlines"}}}

    async def start(self):                       # replaces start_requests(), removed in 2.16
        yield scrapy.Request("https://shop.example.com/", callback=self.parse)

    def parse(self, response):
        for card in response.css("div.card"):
            yield response.follow(card.css("a::attr(href)").get(), self.parse_detail,
                                  cb_kwargs={"list_price": card.css(".price::text").get()})
        yield from response.follow_all(css="a.next", callback=self.parse)

    def parse_detail(self, response, list_price):
        yield {"url": response.url, "title": response.css("h1::text").get(), "list_price": list_price}
```

```python
# Rule-based variant
from scrapy.spiders import CrawlSpider, Rule
from scrapy.linkextractors import LinkExtractor

class ShopSpider(CrawlSpider):
    name = "shop"
    start_urls = ["https://shop.example.com/"]
    rules = [Rule(LinkExtractor(restrict_css="a.next")),                       # follow
             Rule(LinkExtractor(restrict_css="a.product"), callback="parse_item")]
    def parse_item(self, response):
        yield {"url": response.url, "title": response.css("h1::text").get()}
```

**"Context".** There is no context object: the callback receives a `Response` (`css`, `xpath`, `json()`, `jmespath`, `follow`, `follow_all`, `urljoin`, `meta`, `request`, `cb_kwargs`). Everything else is reached through the spider (`self.crawler.stats`, `self.settings`, `self.logger`, `self.state`) or through components. Data passes by `cb_kwargs` (since 1.7; the outcome of [#1138](https://github.com/scrapy/scrapy/issues/1138), 18 r) or `meta`; 2.18 added `MetaCopyDetectionMiddleware` to catch accidental meta sharing.

**Enqueueing and scope.** `yield Request(...)`, `response.follow`, `follow_all`; `allowed_domains` + offsite middleware (2.13 added `allow_offsite` meta key); `LinkExtractor(allow, deny, restrict_css, restrict_xpaths, …)` with `CrawlSpider.rules`; `DEPTH_LIMIT`; `dont_filter=True` bypasses the fingerprint dupefilter; request priority; 2.14 made `DownloaderAwarePriorityQueue` the default.

**Output.** Items (dicts, `scrapy.Item`, dataclasses, attrs) → **item pipelines** (`process_item`, ordered by integer priority in `ITEM_PIPELINES`) → **feed exports** (`FEEDS` setting or `-O file.json`; JSON, JSONL, CSV, XML, S3, GCS, FTP; batching; item filters). Media pipelines for files/images. Item loaders and processors. This is the most developed typed-output story of all frameworks surveyed.

**Configuration.** Layered `Settings` with priorities (defaults < project < spider `custom_settings`/`update_settings` < command line `-s`), add-ons (`ADDONS`), and component dicts keyed by import path with integer order (`DOWNLOADER_MIDDLEWARES`, `SPIDER_MIDDLEWARES`, `ITEM_PIPELINES`, `EXTENSIONS`, `DOWNLOAD_HANDLERS`). **Library defaults vs template defaults diverge** (verified 2.19.0): `ROBOTSTXT_OBEY=False`, `CONCURRENT_REQUESTS=16`, `CONCURRENT_REQUESTS_PER_DOMAIN=8`, `DOWNLOAD_DELAY=0`, `AUTOTHROTTLE_ENABLED=False`, `RETRY_TIMES=2`, `DOWNLOAD_TIMEOUT=180`, `USER_AGENT="Scrapy/<v> (+https://scrapy.org)"` in [default_settings.py](https://github.com/scrapy/scrapy/blob/2.19.0/scrapy/settings/default_settings.py), while the [project template](https://github.com/scrapy/scrapy/blob/2.19.0/scrapy/templates/project/module/settings.py.tmpl) sets `ROBOTSTXT_OBEY=True`, `CONCURRENT_REQUESTS_PER_DOMAIN=1`, `DOWNLOAD_DELAY=1` (template change in **2.13.3**, 2025-07-02, after [#6597](https://github.com/scrapy/scrapy/issues/6597)). A spider run from a script or another tool does **not** get the template values.

**Tier switching (browser).** Not built in. **scrapy-playwright** replaces the `https` download handler in settings, requires the asyncio reactor, and is opted into **per request** via `meta={"playwright": True}`; page actions via `meta["playwright_page_methods"] = [PageMethod("click", "…")]`; the live page via `playwright_include_page=True`, after which **you must close it** (with an errback too) or the crawl can freeze at `PLAYWRIGHT_MAX_PAGES_PER_CONTEXT`; the response body is the serialized DOM, so a JSON endpoint fetched through Playwright must be parsed out of `<pre>`; on Windows Playwright runs in a separate thread because Twisted needs `SelectorEventLoop` ([README](https://github.com/scrapy-plugins/scrapy-playwright/blob/main/README.md)). Page interactions after `goto` "bypass the Scrapy request workflow (Scheduler, Middlewares, etc)". Per-request opt-in is a good design; the settings, reactor and page-lifecycle burden is the pain.

```python
# scrapy-playwright: settings.py
DOWNLOAD_HANDLERS = {"https": "scrapy_playwright.handler.ScrapyPlaywrightDownloadHandler"}
# spider
yield scrapy.Request(url, meta={"playwright": True,
                                "playwright_page_methods": [PageMethod("wait_for_selector", "div.card")]})
```

**Page objects (scrapy-poet / web-poet).** Zyte's API evolution moves extraction out of callbacks into framework-independent page objects with dependency injection by type annotation:

```python
import attrs
from web_poet import ItemPage, HttpResponse, field, handle_urls

@handle_urls("shop.example.com")
@attrs.define
class ProductPage(ItemPage):
    response: HttpResponse
    @field
    def title(self) -> str | None:
        return self.response.css("h1::text").get()

# in a spider callback, scrapy-poet injects the page object by type:
def parse_detail(self, response, page: ProductPage):
    yield page.to_item()           # or: yield await page.to_item() when fields are async
```

Fields can be sync or async (async when a field needs an additional request; `ensure_awaitable` hides the difference), page objects are testable from **saved fixtures** (serialized inputs + expected `to_item()` output, pytest plugin, `scrapy savefixture`) ([fields](https://github.com/scrapinghub/web-poet/blob/master/docs/page-objects/fields.rst), [testing](https://github.com/scrapinghub/web-poet/blob/master/docs/page-objects/testing.rst), [scrapy-poet tutorial](https://github.com/scrapinghub/scrapy-poet/blob/master/docs/intro/basic-tutorial.rst)). The separation "extraction code does no I/O and is testable offline" is the most transferable idea in the Scrapy ecosystem.

**Lifecycle and resume.** `JOBDIR` persists the scheduler queue, the dupefilter and `spider.state`; resume by re-running the same command. resume is supported only after a clean stop, and unclean shutdown "can lead to data corruption" ([jobs.rst](https://github.com/scrapy/scrapy/blob/2.19.0/docs/topics/jobs.rst)). Requests must be serializable (callbacks must be spider methods). Stats collector; `CLOSESPIDER_*` conditions; signals `spider_closed` etc.

**Errors.** Retry middleware (`RETRY_TIMES`, `RETRY_HTTP_CODES`, `RETRY_EXCEPTIONS`); `errback=` per request receives a `Failure`; `get_retry_request()` (2.5+) answers the old "retry from a callback" request ([#3590](https://github.com/scrapy/scrapy/issues/3590), 8 r, 21 c); `HttpError` middleware filters non-2xx before callbacks unless allowed; `CloseSpider`, `IgnoreRequest`, `DropItem` exceptions. 2.15 made download handlers raise Scrapy-specific exceptions instead of Twisted ones.

**Extensibility.** The richest of all: downloader middlewares, spider middlewares, item pipelines, extensions, signals, download handlers (documented API since 2.14), add-ons, custom schedulers/queues/dupefilters, contracts, commands. This power is also the configuration burden (import-path dicts with magic integer orders).

**Testing and debugging.** **Contracts** in callback docstrings (`@url`, `@returns items 1 16`, `@scrapes Title Price`, `@cb_kwargs`, `@meta`), run by `scrapy check`; since 2.19 they support `async def` callbacks ([contracts.rst](https://github.com/scrapy/scrapy/blob/2.19.0/docs/topics/contracts.rst)). `scrapy shell <url>` and `inspect_response()` for interactive selector work; `scrapy parse --callback`; `scrapy fetch`, `view`; telnet console; `HTTPCACHE_ENABLED` for offline iteration. **2.19 (2026-09-10) added a `RemoteControl` extension and an official Scrapy MCP server** so coding agents can inspect stats, scheduler and memory of a running crawl, and even modify it ([agents.rst](https://github.com/scrapy/scrapy/blob/2.19.0/docs/topics/agents.rst)).

**Async model.** Twisted underneath; asyncio reactor default since **2.13.0** (2025-05-08); `async def start()` replaced `start_requests()` (deprecated 2.13, **removed in 2.16.0**, 2026-05-19, "no longer called"); synchronous `process_spider_output` in spider middlewares removed in 2.16; **2.15.0** (2026-04-09) added an **experimental mode without a Twisted reactor** (`TWISTED_REACTOR_ENABLED=False`, aiohttp download handler default since 2.19), which lifts the "reactor cannot restart" limit and helps Jupyter ([asyncio.rst](https://github.com/scrapy/scrapy/blob/2.19.0/docs/topics/asyncio.rst)). A maintainer closed the 2020 Jupyter issue in 2026-07 noting 2.15+ works with `AsyncCrawlerRunner({"TWISTED_REACTOR_ENABLED": False})` ([#4299](https://github.com/scrapy/scrapy/issues/4299)).

**Typing.** `py.typed` and near-full annotations since **2.12.0** (2024-11-18), after [#4041](https://github.com/scrapy/scrapy/issues/4041) (15 r, 21 c).

### 2.4 Spider-rs (spider crate, bindings, spider.cloud)

**Entry point.** A `Website` value built from a URL, configured by **direct field mutation** (`website.configuration.respect_robots_txt = true`) or a **builder** (`.with_limit(…)…build()`, added in 1.33.0 after [#115](https://github.com/spider-rs/spider/issues/115) "Change API to builder pattern"). Crawl verbs are methods with different semantics: `crawl()`, `crawl_smart()`, `crawl_raw()`, `scrape()` (keeps HTML), `crawl_sitemap()`. Results are pulled after the fact (`get_links()`, `get_pages()`) or streamed through a Tokio broadcast channel (`subscribe(capacity)`).

```rust
// spider 2.53 (Cargo: spider = { version = "2", features = ["smart"] })
use spider::tokio;
use spider::website::Website;

#[tokio::main]
async fn main() {
    let mut website = Website::new("https://shop.example.com")
        .with_respect_robots_txt(true)
        .with_limit(500)                                   // budget "*" ; 0 = unlimited
        .with_depth(3)
        .with_budget(Some(spider::hashbrown::HashMap::from([("*", 500), ("/blog", 20)])))
        .with_blacklist_url(Some(vec!["/cart".into()]))
        .with_retry(2)                                     // default 0
        .build()
        .unwrap();

    let mut rx = website.subscribe(64);                    // broadcast::Receiver<Page>
    let consumer = tokio::spawn(async move {
        while let Ok(page) = rx.recv().await {
            println!("{} {}", page.status_code, page.get_url());
            // parse page.get_html() yourself (e.g. with `scraper`); no item/pipeline concept
        }
    });

    website.crawl_smart().await;   // HTTP first, Chrome when heuristics say so
    website.unsubscribe();
    let _ = consumer.await;
}
```

```ts
// spider-nodejs (@spider-rs/spider-rs 0.0.163)
import { Website } from '@spider-rs/spider-rs'
const website = new Website('https://shop.example.com').withBudget({ '*': 500 }).withBlacklistUrl(['/cart']).build()
await website.crawl((_err, page) => website.pushData({ url: page.url, status: page.statusCode }))
await website.exportJsonlData('./storage/spider.jsonl')
```

```python
# spider-py (spider_rs 0.0.57, last PyPI release 2025-01-28)
from spider_rs import Website
website = Website("https://shop.example.com")
website.crawl()          # README calls it without await inside an async main; semantics not verified
print(website.get_links())
```

```python
# spider.cloud client (spider_client 0.1.93): one-shot API, untyped params dict
from spider import Spider
app = Spider()  # SPIDER_API_KEY
pages = app.crawl_url("https://shop.example.com", params={"limit": 50, "request": "smart_mode", "return_format": "markdown"})
```

**Handler context.** None in the Scrapy/Crawlee sense. The unit is `Page` (URL, `status_code`, HTML/bytes, `error_status`, `final_redirect_destination`, `should_retry`, `blocked_crawl`, `waf_check`, `bytes_transferred`, links); callbacks exist for link discovery (`with_on_link_find_callback`) and admission (`with_on_should_crawl_callback`). Extraction and output are the user's job (or spider.cloud's `return_format`). The Node binding adds a Crawlee-like `pushData`/`exportJsonlData`.

**Enqueueing and scope.** Declarative: `subdomains`, `tld`, `external_domains`, `with_whitelist_url`/`with_blacklist_url` (string or regex with the `regex` feature), `with_depth`, `with_limit`, `with_budget` per path prefix, `glob` feature for URL globbing, `with_redirect_limit`, robots. Mid-crawl injection via `website.queue(capacity)` → broadcast sender (only with `sync`). No per-link labels or user data: there is one "handler", the subscriber.

**Output.** Broadcast channel of `Page` (backpressure by capacity; slow consumers lag), `get_pages()` after `scrape()`, optional disk cache, screenshots (`chrome_screenshot` feature writes to `./storage/`). No dataset, item or sink abstractions in the crate.

**Configuration: Cargo features are the main switchboard.** The published 2.53.9 manifest declares **132 features**; default `["basic", "io_uring", "tcp_fastopen", "splice", "numa", "zero_copy"]`, where `basic` pulls `sync`, `cookies`, `ua_generator`, `encoding`, `balance`, `real_browser`, `disk_native_tls`. `smart = ["chrome", "chrome_intercept"]` ([Cargo.toml](https://github.com/spider-rs/spider/blob/e4b53c5b582357e46433fa9ba3d91f930fb67cb4/spider/Cargo.toml)). Verified consequences in source:
- **Features change public types and signatures.** `Page.error_status` is `Option<String>` normally and `Option<Arc<reqwest::Error>>` with `page_error_status_details`; `Website::queue()` returns `Option<broadcast::Sender<String>>` with `sync` and `Option<Arc<(Sender<Page>, Receiver<Page>)>>` (always `None`) without; `unsubscribe()` is an empty function without `sync`; `crawl_smart()` has four `cfg` variants and is an alias of `crawl()` without `smart` ([page.rs L2440](https://github.com/spider-rs/spider/blob/e4b53c5b582357e46433fa9ba3d91f930fb67cb4/spider/src/page.rs#L2440), [website.rs L14676–L14720](https://github.com/spider-rs/spider/blob/e4b53c5b582357e46433fa9ba3d91f930fb67cb4/spider/src/website.rs#L14676)). A missing feature compiles fine and silently does less.
- **Feature combinations break builds.** [#349](https://github.com/spider-rs/spider/issues/349) (2026-02-06): 2.44.19 failed to compile with `chrome` but without `agent`; the maintainer was pointed to `cargo hack --feature-powerset` and shipped 2.44.20, 2.44.21 and a further update the same day.
- **Default `io_uring` breaks in containers.** [#243](https://github.com/spider-rs/spider/issues/243) (2025-01, 15 c): after 2.23.7 introduced tokio-uring, crawls panicked with "Operation not permitted" in Docker unless privileged or `seccomp=unconfined`.
- **Docs drift**: the README still says `request_timeout` "Defaults to 15000 ms" while `Configuration::new()` sets 120 s (prior audit, re-checked).

**Tier switching.** Compile with `chrome` (or `smart`), then `crawl_smart()` for HTTP-first with heuristic upgrade (threshold-based score of framework markers, `noscript`, script DOM mutations), or Chrome everywhere with `chrome`. JS loading in Chrome is partly blocked by default interception; a user whose SPA links were missing had to discover `RequestInterceptConfiguration` ([#293](https://github.com/spider-rs/spider/issues/293), 11 c). Remote Chrome via `CHROME_URL` env var. The maintainer described smart re-render as swapping "between http and chrome on retry" ([#291](https://github.com/spider-rs/spider/issues/291)).

**Lifecycle.** `crawl().await` completes when the frontier drains; `control` feature adds `pause/resume/shutdown(url)` keyed by URL string; `cron` feature schedules; `website.stop()`; no durable resume of the frontier (prior audit: SQLite `disk` stores visited URLs and signatures only). The Python binding's `website.stop()` "does not stop the crawl" ([spider-py#14](https://github.com/spider-rs/spider-py/issues/14), open).

**Errors.** No exceptions or failed-request callback: errors are fields on `Page` (`status_code`, `error_status`), retries default **0**, a pluggable `RetryStrategy` (prior audit). The user filters the stream.

**Bindings and cloud.** spider-py's last PyPI release is **0.0.57 (2025-01-28)** while the crate moved from about 2.27 to 2.53; open issues ask for type hints ([#16](https://github.com/spider-rs/spider-py/issues/16), [#2](https://github.com/spider-rs/spider-py/issues/2)) and report install failures ([#21](https://github.com/spider-rs/spider-py/issues/21) GitHub Actions, [#12](https://github.com/spider-rs/spider-py/issues/12) Ubuntu, [#1](https://github.com/spider-rs/spider-py/issues/1)). spider-nodejs had a 16-month gap between 0.0.157 (2024-09-24) and 0.0.163 (2026-01-24) and an issue titled "Builds since 0.0,70?" ([#2](https://github.com/spider-rs/spider-nodejs/issues/2), 10 c). The bindings' READMEs still show a benchmark table where Crawlee crawls 18,779 pages in 30 min vs Spider 150,387 in 1 min (vendor, protocol not audited; prior audit found the Node competitor in the Spider benchmark script was `crawler`, not Crawlee). The spider.cloud SDKs are thin REST clients with untyped `params` dicts; tier is a string (`"request": "http" | "chrome" | "smart_mode"`).

**Release cadence.** 2,227 published versions of the crate: 86 in 2022, 329 in 2023, 659 in 2024, 626 in 2025, 516 in 2026 to date; only two majors (1.0.0 2018, 2.0.0 2024-08-14), so breaking surface moves in minors and patches. Regressions reported as "used to work": [#268](https://github.com/spider-rs/spider/issues/268) subscribe + `scrape()` never finishing (fixed same day in 2.27.63), [#252](https://github.com/spider-rs/spider/issues/252) "Queued links are ignored if first url is invalid (again)", [#232](https://github.com/spider-rs/spider/issues/232) delay "totally breaks crawl" (fixed in hours). The prompt's "frequent majors" framing is inaccurate: it is **very frequent minors/patches with breaking effects**, which is arguably harder for consumers to track.

### 2.5 Colly (Go)

**Entry point.** A `Collector` built with functional options; behaviour is registered as **callbacks by event**: `OnRequest`, `OnResponseHeaders`, `OnResponse`, `OnHTML(selector, fn)`, `OnXML`, `OnScraped`, `OnError`. `Visit()` seeds; `Wait()` joins async work.

```go
// Colly v2.3
c := colly.NewCollector(
    colly.AllowedDomains("shop.example.com"),
    colly.MaxDepth(3),
    colly.Async(true),
)
c.Limit(&colly.LimitRule{DomainGlob: "*", Parallelism: 2, RandomDelay: time.Second})
detail := c.Clone()

c.OnHTML("a.product[href]", func(e *colly.HTMLElement) {
    ctx := colly.NewContext()
    ctx.Put("list_price", e.ChildText(".price"))
    detail.Request("GET", e.Request.AbsoluteURL(e.Attr("href")), nil, ctx, nil)
})
c.OnHTML("a.next[href]", func(e *colly.HTMLElement) { e.Request.Visit(e.Attr("href")) })
detail.OnHTML("h1", func(e *colly.HTMLElement) {
    fmt.Println(e.Request.URL, e.Text, e.Request.Ctx.Get("list_price"))
})
c.OnError(func(r *colly.Response, err error) { log.Println(r.Request.URL, err) })
c.Visit("https://shop.example.com/")
c.Wait(); detail.Wait()
```

Context is `*colly.HTMLElement` / `*colly.Response` with `Request.Ctx` (string-keyed `interface{}` map); routing by page type is done with **separate collectors** (`Clone()`), not labels. Scope: `AllowedDomains`, `DisallowedDomains`, `URLFilters` (regex), `MaxDepth`, `AllowURLRevisit`. Politeness per domain glob with `LimitRule`. Storage interface (visited, cookies) with memory default and Redis/SQLite/Mongo backends; a `queue` package. **`IgnoreRobotsTxt` is set to true in `Init()`** ([colly.go L504](https://github.com/gocolly/colly/blob/v2.3.0/colly.go#L504)); [#614](https://github.com/gocolly/colly/issues/614) "Robots.txt respect should be ON by default" is open. No JS rendering: the top issue of all time is [#4](https://github.com/gocolly/colly/issues/4) "JavaScript execution" (49 r, 30 c, open since 2017); the maintainer: "I'm not planning to add remote code execution to Colly." No `context.Context` in callbacks ([#240](https://github.com/gocolly/colly/issues/240), 8 r, open), so cancellation and deadlines are awkward. Maintenance risk: releases v2.1.0 (2020-06) → v2.2.0 (2025-03-27) after issues "is it Dead?" ([#833](https://github.com/gocolly/colly/issues/833)), "Why the latest version was released on 2020?" ([#825](https://github.com/gocolly/colly/issues/825)), "Looking for maintainers" ([#840](https://github.com/gocolly/colly/issues/840), 2025-03, open). Still actively used and praised for speed (§5).

### 2.6 Crawl4AI

**Entry point.** An async context manager `AsyncWebCrawler(config=BrowserConfig(...))`; `await crawler.arun(url, config=CrawlerRunConfig(...))` for one page, `arun_many(urls, config, dispatcher=…)` for many; crawling is a **strategy on the run config** (`deep_crawl_strategy=BFSDeepCrawlStrategy(max_depth, include_external, filter_chain, url_scorer)`, also DFS and BestFirst). Extraction strategies: `JsonCssExtractionStrategy(schema)`, `LLMExtractionStrategy(llm_config, schema=Model.model_json_schema(), instruction)`; Markdown generation with content filters is the core product.

```python
# Crawl4AI 0.9.x style
from crawl4ai import AsyncWebCrawler, CrawlerRunConfig, CacheMode
from crawl4ai.deep_crawling import BFSDeepCrawlStrategy

config = CrawlerRunConfig(
    deep_crawl_strategy=BFSDeepCrawlStrategy(max_depth=2, include_external=False),
    cache_mode=CacheMode.BYPASS,
    check_robots_txt=True,     # default False
    stream=True,
)
async with AsyncWebCrawler() as crawler:
    async for result in await crawler.arun("https://shop.example.com/", config=config):
        if result.success:
            print(result.url, len(result.markdown or ""))
```

The **return type of `arun()` depends on configuration**: one result, a list when a deep-crawl strategy is set, or an async generator when `stream=True` (docs: "Results are returned as a list after all crawling is complete"; [deep-crawling.md](https://github.com/unclecode/crawl4ai/blob/v0.9.5/docs/md_v2/core/deep-crawling.md)). Type checkers could not follow it until 0.9.1 ([#1898](https://github.com/unclecode/crawl4ai/issues/1898): `CrawlResultContainer` used `__getattr__`, "invisible to type checkers"); deep crawl combined with `arun_many` failed ([#1509](https://github.com/unclecode/crawl4ai/issues/1509), [#940](https://github.com/unclecode/crawl4ai/issues/940)). Resume is user-owned: deep-crawl strategies accept `resume_state` and an `on_state_change` async callback ("Called after each URL") so the user persists a JSON state wherever they like. Defaults: `check_robots_txt=False`, `semaphore_count=5`, `mean_delay=0.1` ([async_configs.py](https://github.com/unclecode/crawl4ai/blob/v0.9.5/crawl4ai/async_configs.py)). Its logger wrote to stdout and broke MCP stdio hosts until fixed in 2026-05 ([#1968](https://github.com/unclecode/crawl4ai/issues/1968)). License adds an attribution clause to Apache-2.0 (prior audit).

### 2.7 Firecrawl SDKs (Python, JS, others)

**Entry point.** A remote API client. `Firecrawl(api_key=…)`; verbs `scrape`, `crawl` (waiter that polls), `start_crawl` + `get_crawl_status` + `wait_crawl` + `cancel_crawl`, a WebSocket watcher, `map`, `search`, `batch_scrape`, `extract`, `parse` (upload files). v2 renamed v1's `FirecrawlApp.crawl_url` etc. to `Firecrawl.crawl` (the JS README prose still mentions `FirecrawlApp`).

```python
from firecrawl import Firecrawl, CrawlJobTimeoutError
from firecrawl.types import ScrapeOptions
fc = Firecrawl()   # FIRECRAWL_API_KEY
try:
    job = fc.crawl("https://shop.example.com", limit=200, include_paths=["^/products/"],
                   scrape_options=ScrapeOptions(formats=["markdown", {"type": "json", "schema": Product.model_json_schema()}]),
                   timeout=600)
except CrawlJobTimeoutError as e:
    fc.cancel_crawl(e.job_id)
```

```ts
import { Firecrawl } from 'firecrawl'
import { z } from 'zod'
const app = new Firecrawl({ apiKey: process.env.FIRECRAWL_API_KEY })
const job = await app.crawl('https://shop.example.com', { limit: 200, scrapeOptions: { formats: ['markdown'] } })
const data = await app.extract({ urls: ['https://shop.example.com/p/1'], prompt: 'Extract the product', schema: z.object({ title: z.string() }) })
```

No handler runs in the user's process; scope is declarative on `CrawlRequest`: `include_paths`/`exclude_paths` (regex on path, or full URL with `regex_on_full_url`), `max_discovery_depth`, `sitemap: "skip"|"include"|"only"`, `ignore_query_parameters`, `deduplicate_similar_urls=True`, `crawl_entire_domain`, `allow_external_links`, `allow_subdomains`, `ignore_robots_txt=False`, `delay`, `max_concurrency`, `webhook`, and **`prompt`**: a natural-language description from which the server derives crawl options ([v2/types.py](https://github.com/firecrawl/firecrawl/blob/main/apps/python-sdk/firecrawl/v2/types.py)). Output is formats per page (`markdown`, `html`, `links`, `screenshot`, `json` with schema, deterministic `product`/`menu` extractors). Jobs and timeouts are explicit (`CrawlJobTimeoutError` carries `job_id`; asyncio cancellation sends a best-effort cancel). Pain concentrates on self-hosting parity: 15 of the top 25 issues by reactions are tagged `[Self-Host]` (e.g. [#1129](https://github.com/firecrawl/firecrawl/issues/1129) "we need proxy on self-hosted version", 18 r; [#2350](https://github.com/firecrawl/firecrawl/issues/2350) "permanent loop when blocked by anti-bot", 10 r, 25 c) and on identification ([#1169](https://github.com/firecrawl/firecrawl/issues/1169) "Add an identifiable user agent", open, 7 r, 18 c). Core AGPL-3.0; SDKs MIT.

### 2.8 Scrapling

**Entry point.** Two layers: fetchers (`Fetcher`/`FetcherSession` HTTP with curl_cffi, `DynamicFetcher`/`AsyncDynamicSession` Playwright, `StealthyFetcher`/`AsyncStealthySession` Patchright) with an adaptive parser; and since 0.4 a Scrapy-like **`Spider`** class (`name`, `start_urls`, `async def parse(self, response)` async-generator callbacks, `response.follow(…, callback=…)`). Notable API choices:

```python
# Scrapling 0.4.15
from scrapling.spiders import Spider, Response
from scrapling.fetchers import FetcherSession, AsyncStealthySession

class Products(Spider):
    name = "products"
    start_urls = ["https://shop.example.com/"]
    robots_txt_obey = True                     # default False
    development_mode = True                    # cache responses to .scrapling_cache/ and replay

    def configure_sessions(self, manager):
        manager.add("http", FetcherSession())                                   # default
        manager.add("stealth", AsyncStealthySession(headless=True), lazy=True)  # browser started on first use

    async def parse(self, response: Response):
        for href in response.css("a.product::attr(href)").getall():
            yield response.follow(href, sid="stealth", callback=self.parse_product)   # per-request tier
        if nxt := response.css("a.next::attr(href)").get():
            yield response.follow(nxt)

    async def parse_product(self, response: Response):
        yield {"title": response.css("h1::text").get("")}

result = Products(crawldir="crawl_data/products").start()   # sync entry; Ctrl+C pauses with checkpoint
result.items.to_jsonl("products.jsonl")
```

- **Per-request fetch tier by session id** (`sid="stealth"`), sessions declared once, optionally lazy: the cleanest tier-switch API found in this survey ([sessions.md](https://github.com/D4Vinci/Scrapling/blob/v0.4.15/docs/spiders/sessions.md)).
- **`start()` (sync, returns `CrawlResult` with `items`, `stats`, `completed`/`paused`) vs `stream()` (async iterator of items with live `spider.stats`)**.
- **Pause/resume**: `crawldir`; first Ctrl+C drains in-flight requests and checkpoints pending + seen fingerprints atomically, second Ctrl+C forces; periodic checkpoint every 5 min; checkpoint deleted on completion ([advanced.md](https://github.com/D4Vinci/Scrapling/blob/v0.4.15/docs/spiders/advanced.md)).
- **`development_mode`**: cache every response on first run, replay offline afterwards, bypassing rate limits and block retries, with `cache_hits`/`cache_misses` stats.
- Defaults (verified [spider.py](https://github.com/D4Vinci/Scrapling/blob/v0.4.15/scrapling/spiders/spider.py)): `robots_txt_obey=False`, `concurrent_requests=4`, `download_delay=0.0`, `max_blocked_retries=3`, `autothrottle_enabled=False`, `logging_level=DEBUG`.
- Keeps the name `start_requests()` that Scrapy removed in 2.16: naming divergence between look-alike APIs.
- Its download progress messages on stdout broke MCP stdio clients ([#50](https://github.com/D4Vinci/Scrapling/issues/50), 16 c); the owner's answer was to configure logging. Bare `pip install scrapling` imports failed until extras were used ([#343](https://github.com/D4Vinci/Scrapling/issues/343)). Crawlee Python declined a built-in Scrapling parser but documented custom parsers ([crawlee-python#1392](https://github.com/apify/crawlee-python/issues/1392), 5 r, 18 c).

### 2.9 Katana (ProjectDiscovery)

CLI-first reconnaissance crawler; the Go library reuses the **CLI options struct**: `types.Options{MaxDepth, FieldScope: "rdn", Concurrency, Parallelism, RateLimit: 150, Strategy: "depth-first", OnResult: func(output.Result){…}}` → `types.NewCrawlerOptions` → `standard.New` (or hybrid/headless engines) → `crawler.Crawl(url)` ([README](https://github.com/projectdiscovery/katana/blob/v1.8.0/README.md#katana-as-a-library)). One result callback, no per-page-type routing; scope by field (`rdn`, `fqdn`, `dn`) and regex; JS/XHR endpoint discovery and form filling. Users report hybrid concurrency "doesn't work the way we think it does" ([#1215](https://github.com/projectdiscovery/katana/issues/1215)) and hangs ([#416](https://github.com/projectdiscovery/katana/issues/416), [#1272](https://github.com/projectdiscovery/katana/issues/1272) "Error closing page: context deadline exceeded", 9 r). Interesting as a model for "library = typed CLI options + result callback", less as a crawl-framework API.

### 2.10 Apify SDK (JS 3.7.2, Python 4.1.0)

The runtime wrapper around Crawlee for the Apify platform: `Actor.init()`/`Actor.exit()` or `Actor.main(fn)` (JS), `async with Actor:` (Python); `Actor.getInput()`, `Actor.pushData()`, `Actor.openDataset/openKeyValueStore/openRequestQueue`, `Actor.setValue`, `Actor.createProxyConfiguration()`, charging events. It is where platform concerns (input schema, encrypted secrets, migrations, reboot, pay-per-event) live; Crawlee v4 moved `KeyValueStore.getInput()` out of Crawlee into the SDK "Reading the run input is an Apify platform concern". Pain is around migration/reboot and local-vs-platform drift: [apify-sdk-js#4](https://github.com/apify/apify-sdk-js/issues/4) "Apify.call should survive migration" (21 c, open since 2020), [#668](https://github.com/apify/apify-sdk-js/issues/668) "Crawler successfully finish with pending Requests in RequestQueue" (open), [#460](https://github.com/apify/apify-sdk-js/issues/460) `getRecordPublicUrl()` "works locally but not on the Apify Platform". The Python SDK also wraps **Scrapy** (an Apify–Scrapy integration with its own scheduler/queue bridge; [apify-sdk-python#392](https://github.com/apify/apify-sdk-python/issues/392) "Request queue stuck in infinite loop due to ID mismatch in Apify-Scrapy integration").

### 2.11 Heritrix and StormCrawler

Not consumer libraries in the same sense (Heritrix is a Java archival crawler configured by Spring XML "crawl jobs" with a web UI; StormCrawler is a set of Apache Storm spouts/bolts configured by YAML/Flux topologies). Their consumer API lessons are already captured in the capability matrix (`rnd/research/2026-10-07-crawl-capability-matrix.md`): URL status frontier with `nextFetchDate`, per-host partitioning, best-effort page actions. Not re-examined here; no evidence they are dead (StormCrawler 3.7.0 is an Apache TLP release), but they are not developer-facing SDKs comparable to Foxhound's target.

---

## 3. Cross-framework comparison

### 3.1 Consumer-API matrix

| Dimension | Crawlee JS 3.18 (→ v4 RC) | Crawlee Py 1.10 | Scrapy 2.19 | Spider 2.53 (+ bindings) | Colly v2.3 | Crawl4AI 0.9 | Firecrawl SDK v2 | Scrapling 0.4 | Katana 1.8 |
|---|---|---|---|---|---|---|---|---|---|
| Crawl defined by | Crawler object + options; `requestHandler` or router | Same; decorators `@crawler.router.handler(label)` | `Spider` subclass + generator callbacks; `CrawlSpider` rules | `Website` builder/fields + verb (`crawl`, `crawl_smart`, `scrape`) | `Collector` + `On*` callbacks | `AsyncWebCrawler` + `CrawlerRunConfig` + deep-crawl strategy object | Remote job: `crawl(url, **options)` | `Spider` subclass + async-generator callbacks | Options struct (CLI flags) + `OnResult` |
| Page-type routing | Labels → router handlers | Labels → router handlers | `callback=` per request | none (one subscriber) | separate collectors (`Clone`) | none (result list) | none (server-side) | `callback=` per request | none |
| Handler gets | Context object (shape varies by tier) | Context dataclass (varies by tier) | `Response` (+ `cb_kwargs`) | `Page` value from channel | `HTMLElement`/`Response` | `CrawlResult` | `Document` objects | `Response` | `output.Result` |
| Data to next page | `request.userData` (JSON) | `request.user_data` (JSON) | `cb_kwargs` / `meta` (pickled for JOBDIR) | n/a | `Request.Ctx` map | n/a | n/a | `response.follow(**kwargs)` | n/a |
| Discover/enqueue | `enqueueLinks({selector, strategy, include/exclude, label})` | `enqueue_links(...)`, `add_requests` | `response.follow(_all)`, `LinkExtractor`+`Rule` | declarative: domains, white/blacklist, budget, depth, glob | `e.Request.Visit`, `AllowedDomains`, `URLFilters` | `BFS/DFS/BestFirst` strategy + filter chain + scorer | `include_paths/exclude_paths`, depth, sitemap, NL `prompt` | `response.follow` | scope fields + regex |
| Default robots.txt | **ignored** (`respectRobotsTxtFile=false`) | **ignored** | ignored by library, **obeyed in project template** | **ignored** (`respect_robots_txt=false`) | **ignored** (`IgnoreRobotsTxt=true`) | **ignored** (`check_robots_txt=False`) | **obeyed** (`ignore_robots_txt=False`) | **ignored** | n/a (recon) |
| Default per-domain politeness | none (`sameDomainDelaySecs=0`), autoscaled concurrency | none, autoscaled | 8/domain, 0 s (library); 1/domain, 1 s (template) | delay 0, system-derived concurrency | none unless `LimitRule` | `mean_delay=0.1`, 5 sessions | server-managed | 4 concurrent, 0 s | 150 req/s rate limit |
| Results | `Dataset`/`KeyValueStore` (local `./storage`, purged on start) | `Dataset`/KVS; memory/FS/SQL/Redis clients | Items → pipelines → feed exports | broadcast `Page` stream, `get_pages()` | user code | `CrawlResult` (markdown, extracted JSON, links) | formats per page, job results | `CrawlResult.items` / `stream()` | JSONL / callback |
| Schema / typed items | none (v4: transactional writes) | none in core; `PydanticAiCrawler.extract(Model)` | `Item`, dataclass, attrs, loaders; web-poet page objects | none | none | JSON CSS schema, LLM schema | JSON schema / Zod / Pydantic for `json` format and `extract` | none | fixed fields |
| Config surface | ctor options + `Configuration` + `CRAWLEE_*` env (v3: env wins; v4: ctor wins) | ctor kwargs + `Configuration` (pydantic-settings) | layered settings, component dicts with integer order, add-ons | **Cargo features (132)** + builder + env (`CHROME_URL`) | functional options | config objects | kwargs / options objects | class attributes | struct fields |
| HTTP → browser | change class (+ rewrite selectors) or `AdaptivePlaywrightCrawler` | same; adaptive with `with_parsel_static_parser()` | plugin: settings + reactor + `meta={"playwright": True}` per request | recompile with `chrome`/`smart`, call `crawl_smart()` | not supported | browser is default; `HTTPCrawlerStrategy` alternative | server-side engine waterfall | **per request `sid=` to a declared session** | `-headless` / hybrid engine |
| Resume | persisted storage if purge disabled / named queue | storage clients (FS/SQL/Redis) | `JOBDIR` (clean stop only) | none for frontier (visited set on SQLite) | storage backends; no frontier ack | `resume_state` + `on_state_change` callback | job IDs server-side | `crawldir` checkpoints, Ctrl+C pause | root-input resume file |
| Failure surface | `errorHandler`, `failedRequestHandler`, error classes | `@error_handler`, `@failed_request_handler` | `errback`, retry middleware, `get_retry_request` | fields on `Page`, `RetryStrategy` | `OnError` | `result.success`, `error_message` | exceptions + `get_crawl_errors(id)` | `on_error` hook, `is_blocked()` retry | logged |
| Extension points | nav hooks, BrowserPool hooks, HTTP client, storage client, `router.use`, (v4) `extendContext`, injectable pools | same + `service_locator` | middlewares, pipelines, extensions, signals, download handlers, add-ons | link/admission callbacks, `RetryStrategy`, `RemoteFetcher`, features | callbacks, `Storage`, transport | strategies, hooks (`on_page_context_created`…) | none (API) | lifecycle hooks, sessions, `is_blocked` override | options |
| Testing aid | none official | none official | **contracts**, `scrapy shell`, `parse`, web-poet fixtures, HTTP cache, (2.19) MCP | none | none | none | n/a | **`development_mode` replay cache** | n/a |
| Async model | Node promises | asyncio | Twisted (+asyncio reactor default; no-reactor mode experimental) | Tokio; bindings wrap it | goroutines | asyncio | sync + async clients | anyio | goroutines |
| Typing | TS; context loosely typed in v3, strict in v4 | `py.typed`, pydantic | `py.typed` since 2.12 | Rust types, feature-dependent; Py binding untyped | Go | partial; return type varies by config until 0.9.1 | Pydantic / TS types | typed | Go |

Sources: sections 2.1–2.9 above; defaults confirmed in the cited source files at the pinned versions.

### 3.2 How painful is moving a crawler from HTTP to browser?

| Framework | What the developer changes | Code rewrite? | Evidence |
|---|---|---|---|
| Crawlee (classic) | crawler class, router factory, every `$()` → `page.locator()` (or wrap in `parseWithCheerio()`), browser install | Partial–full | context interfaces differ by tier (§2.1); [#2275](https://github.com/apify/crawlee/issues/2275) browsers not installed (21 c) |
| Crawlee adaptive | write the handler against the restricted context once | No, but must avoid `page` | [adaptive context](https://github.com/apify/crawlee/blob/v3.18.2/packages/playwright-crawler/src/internals/adaptive-playwright-crawler.ts); Python mirrors it |
| Scrapy | install plugin, swap download handler, require asyncio reactor, add `meta={"playwright": True}`, manage page lifecycle and contexts, parse JSON out of `<pre>` | No for selectors; yes for anything interactive | scrapy-playwright README; Windows thread workaround; [#343](https://github.com/scrapy-plugins/scrapy-playwright/issues/343) Scrapy 2.13 + Windows hang until 0.0.44 |
| Spider | add Cargo feature(s), rebuild, call `crawl_smart()`; discover interception defaults | No for code, yes for build | [#293](https://github.com/spider-rs/spider/issues/293), [#349](https://github.com/spider-rs/spider/issues/349), [#261 discussion](https://github.com/spider-rs/spider/discussions/261) "`chrome` feature not working" |
| Colly | not possible; switch library (chromedp/rod) | Full | [#4](https://github.com/gocolly/colly/issues/4) (49 r): one commenter moved to Ferret because Colly lacks JS |
| Scrapling | declare a browser session, route requests with `sid=` | No | [sessions.md](https://github.com/D4Vinci/Scrapling/blob/v0.4.15/docs/spiders/sessions.md) |
| Firecrawl / spider.cloud | a request option (`request: "chrome"`/engine choice server-side) | No | SDK READMEs |

---

## 4. Common patterns and divergences

### 4.1 Patterns that recur

1. **Three-part decomposition: discover → fetch → handle → emit.** Every framework separates request scheduling from page handling, and page handling from output, even when the API hides it (Spider's subscriber, Firecrawl's server). Where output has its own first-class stage (Scrapy pipelines/feeds, Crawlee `Dataset`), users rely on it heavily; where it does not (Spider, Colly, Katana), users build it.
2. **Labels or callbacks as the page-type router.** Crawlee labels + router and Scrapy `callback=` are equivalent; both pass per-request state (`userData`/`cb_kwargs`) that must be serializable to survive persistence. Both frameworks had a top issue about how to pass data between handlers (Scrapy [#1138](https://github.com/scrapy/scrapy/issues/1138), 18 r, 20 c; crawlee-python [#524](https://github.com/apify/crawlee-python/issues/524), 20 c).
3. **"Enqueue links" helper with a scope strategy.** Crawlee `enqueueLinks(strategy)`, Scrapy `LinkExtractor` + `allowed_domains`, Spider domain flags, Colly `AllowedDomains`, Firecrawl `allow_subdomains`/`crawl_entire_domain`. Defaults converge on **same host**; divergence is in how patterns combine with the strategy (Crawlee v3 replaced, v4 AND; Firecrawl regex on path unless `regex_on_full_url`).
4. **Default politeness is off, safety is opt-in.** Seven of eight libraries ignore robots.txt by default (only Firecrawl's server obeys; Scrapy obeys only in its generated template). Only Scrapy (`Scrapy/<v> (+https://scrapy.org)`) and Colly (`colly - https://github.com/gocolly/colly`, [colly.go L490](https://github.com/gocolly/colly/blob/v2.3.0/colly.go#L490)) identify the *tool* by default; none identifies the *operator*. Crawlee, Spider (`ua_generator` in default features) and the browser-based tools present browser-like user agents by design. Tool identification without operator contact is what drew the Scrapy abuse reports (§6.1); browser-like UAs are what drew the Firecrawl one.
5. **Global configuration + per-crawler overrides + env vars**, with precedence that surprises (Crawlee v3 env > constructor; Scrapy template vs library defaults; Spider README vs `Configuration::new()`).
6. **Tier choice migrating from "pick a class/plugin" to "per request" or "adaptive".** Crawlee adaptive (learned per label), Spider smart (heuristic per page), Scrapy-playwright (per-request meta), Scrapling (per-request session id), Firecrawl/spider.cloud (server-side option). The newest designs make tier a **per-request runtime decision**.
7. **AI extraction is moving into the handler context.** Crawlee Python `context.extract(Model)` with selector caching; Crawlee JS/Py `StagehandCrawler` (`page.extract`); Crawl4AI `LLMExtractionStrategy`; Firecrawl `json` format/`extract`; Spider `openai` feature and `spider_agent`. The cost-control pattern is the same everywhere: **learn selectors once, replay without the model** (Crawlee `PydanticAiSelectorExtractor`, prior audits of Stagehand caching).
8. **Transactions around handler side effects arrive late.** Crawlee v4 makes handler storage writes transactional and adds `afterStorageCommit`; adaptive crawling already buffered writes. Scrapy relies on pipelines after the callback returns. No framework offers exactly-once delivery to external sinks.
9. **Agents as first-class operators.** Scrapy 2.19 ships an MCP server against a live crawl and an agent plugin; Crawl4AI and Scrapling ship MCP servers; Firecrawl and spider.cloud market agent use. Two of these broke MCP stdio by logging to stdout (Crawl4AI [#1968](https://github.com/unclecode/crawl4ai/issues/1968), Scrapling [#50](https://github.com/D4Vinci/Scrapling/issues/50)).

### 4.2 Divergences that matter for an API designer

| Question | Option A | Option B | Who |
|---|---|---|---|
| Who owns the event loop? | Framework owns it (`scrapy crawl`, `CrawlerProcess`, Scrapling `start()`) | User's program owns it (`await crawler.run()`, `async with AsyncWebCrawler()`, Tokio main) | A: Scrapy, Scrapling (sync entry). B: Crawlee, Crawl4AI, Spider, Scrapling `stream()` |
| Results: push or pull? | Push into storage from the handler (`pushData`, `yield item`) | Pull a stream (`subscribe`, `stream()`, `async for result`) | Push: Crawlee, Scrapy. Pull: Spider, Scrapling, Crawl4AI |
| Output typing | Typed items + pipelines | Untyped dicts into a dataset | Scrapy vs Crawlee ("`scrapy.Item` → A plain `dict`") |
| Tier selection | Per crawler (class) | Per request (meta, session id, learned) | Crawlee classic vs Scrapy-playwright/Scrapling/adaptive |
| Capability gating | Runtime options | Compile-time features | Everyone vs Spider |
| Extension model | Ordered component registry (import paths + integers) | Composition of injected objects and hooks | Scrapy vs Crawlee v4 (`ISessionPool`, `IBrowserPool`, `extendContext`) |
| State between runs | Implicit purge by default | Implicit continue by default | Crawlee v3 / crawlee-python (purge) vs Crawlee v4 (continue, warn when nothing to do) |
| Remote vs local | Local library | Hosted job API (`start_crawl` → id → poll/webhook/WebSocket) | Crawlee/Scrapy/Spider vs Firecrawl/spider.cloud |
| Same-name methods, different semantics | Scrapling keeps `start_requests()`; Scrapy removed it in 2.16 and uses `start()`; Scrapling uses `start()` to run the spider | | Look-alike APIs diverge |

---

## 5. What users like (ranked)

Ranking = number of independent reports (GitHub issue authors with no maintainer association, HN commenters, SO askers, independent bloggers), then intensity. Item codes (CJ1, S2…) refer to the full evidence table in [forum-evidence.md](forum-evidence.md); every quote below was seen in the source, and a sample was re-checked against the Algolia and Stack Exchange APIs on 2026-10-14.

| # | What is liked | Independent reports | Representative evidence (quote under 15 words) | Frameworks |
|---|---|---|---|---|
| L1 | **One API for HTTP and browser fetching, with the browser only where needed** | 8 (CJ1, CJ2, CJ4, SP1, SP2, SP4, X1, L2) | "I'm especially excited about the unified API for browser and HTML scraping" ([HN 32561840](https://news.ycombinator.com/item?id=32561840), 2022-08-23); "Playwright only kicks in for pages that actually need JavaScript" ([dev.to](https://dev.to/unixnick/practical-web-scraping-in-python-playwright-scrapy-and-where-they-meet-44), 2026-07-29); headless "is in the order of 10-100x slower" ([HN 32561621](https://news.ycombinator.com/item?id=32561621)) | Crawlee, scrapy-playwright (per-request), Scrapling |
| L2 | **Testable callbacks, good selectors, and offline replay of saved responses** | 5 (S1, S2, S7, X3, X4) | "very well structured, testable, and has solved every problem" ([HN 32570837](https://news.ycombinator.com/item?id=32570837)); "Parsel (the selector library powering Scrapy) is A++" ([HN 32585288](https://news.ycombinator.com/item?id=32585288)); Scrapy's cache "enables recrawls to fix logic bugs" ([HN 40928731](https://news.ycombinator.com/item?id=40928731)) | Scrapy (contracts, HTTP cache, web-poet fixtures); Scrapling `development_mode` is the same idea |
| L3 | **Batteries included plus a deep middleware ecosystem and fine-grained control** | 5 (S3, S4, S5, S6, CP6) | "you need the flexibility middleware gives you" ([HN 40922731](https://news.ycombinator.com/item?id=40922731), 2024-07-10, in the Crawlee Python launch thread); "I don't even consider alternatives at this point." ([HN 15954482](https://news.ycombinator.com/item?id=15954482)) | Scrapy |
| L4 | **"It just works" / time to first result** | 5 (CJ4, CJ5, CJ6, CP1, L1) | "your library has worked better than all the others I've tried" ([HN 40915166](https://news.ycombinator.com/item?id=40915166)); "scrapling seems to pretty much just work." ([HN 49825368](https://news.ycombinator.com/item?id=49825368), 2026-09-24) | Crawlee JS/Py, Scrapling |
| L5 | **Raw speed and low CPU** | 5 (G4, G5, R3, S5, plus colly#840 comment) | "Immediately went from CPU bottleneck to network." ([HN 15955945](https://news.ycombinator.com/item?id=15955945), Scrapy→Colly); "Colly is insanely fast, efficient, and cost-effective" ([colly#840](https://github.com/gocolly/colly/issues/840#issuecomment-2922149457), 2025-05-30) | Colly, Spider/spider.cloud, Scrapy vs browsers |
| L6 | **A small, explicit SDK surface (one call, typed config objects)** | 4 (F1, F2, F3, A2) | "API key plus one SDK or HTTP call" ([Zack Proser](https://zackproser.com/blog/firecrawl-vs-crawl4ai), affiliate disclosure); "crawl4ai's workflow is easy to follow" ([HN 43478906](https://news.ycombinator.com/item?id=43478906)) | Firecrawl SDK, Crawl4AI |
| L7 | **Readable, pleasant API to build on at production scale** | 2 (CJ3, CJ4) | "It's extremely versatile and very pleasant to build on." ([HN 32562550](https://news.ycombinator.com/item?id=32562550), poster reports hundreds of millions of requests/month) | Crawlee |
| L8 | **Builder-style configuration and responsive maintainers** | 2 + observed | "`with_full_resources(True)` is fantastic!" ([spider-py#6](https://github.com/spider-rs/spider-py/issues/6)); regressions fixed within hours ([#232](https://github.com/spider-rs/spider/issues/232), [#268](https://github.com/spider-rs/spider/issues/268), [#349](https://github.com/spider-rs/spider/issues/349)) | Spider |
| L9 | **Optional, modular LLM use** | 1 (A1) | "it works very well without these dependencies and is modular" ([HN 42215031](https://news.ycombinator.com/item?id=42215031)) | Crawl4AI |
| L10 | **Data passing that "just works" in Scrapy** (seen from a Crawlee user) | 1 | "In Scrapy I don't have to think about this at all." (quoted in [crawlee-python#524](https://github.com/apify/crawlee-python/issues/524)) | Scrapy `cb_kwargs` |

Vendor-side positioning that matches L1 and L4: Apify staff, "It's not uncommon to switch from HTTP to headless back to HTTP" ([HN 32561720](https://news.ycombinator.com/item?id=32561720)); Crawlee Python pitch "Set up your scraper with fewer than 10 lines of code." ([HN 40914659](https://news.ycombinator.com/item?id=40914659)).

---

## 6. Pain points (ranked by evidence weight)

Weight combines: independent reports across sources, reactions/comments/votes, number of frameworks affected, and whether maintainers acknowledged it. "Status" is as of 2026-10-14. Each item ends with the **API-design lesson** for Foxhound/Collie (analysis).

### 6.1 Embedding and lifecycle: who owns the process and event loop
- **Evidence (12 independent + aggregates).** Stack Overflow: "How to run Scrapy from within a Python script" 88 votes / 90,594 views ([SO 13437402](https://stackoverflow.com/q/13437402)); "ReactorNotRestartable error in while loop" 41 votes ([SO 39946632](https://stackoverflow.com/q/39946632)); `ReactorNotRestartable` search: 54 questions, 228 votes; 100+ "script" title questions (top 100: 555 votes); 41 "celery" questions. GitHub: [scrapy#2473](https://github.com/scrapy/scrapy/issues/2473) (28 r, 42 c), [#6024](https://github.com/scrapy/scrapy/issues/6024) Twisted 23.8 broke `CrawlerProcess` (20 r; 2.10.1 pinned Twisted), [#2594](https://github.com/scrapy/scrapy/issues/2594) Jupyter, [#2941](https://github.com/scrapy/scrapy/issues/2941), [#5026](https://github.com/scrapy/scrapy/issues/5026), [#4826](https://github.com/scrapy/scrapy/issues/4826) "Improve Testability of Scrapy (ReactorNotRestartable)", [#4485](https://github.com/scrapy/scrapy/issues/4485) reactor setting ignored in `custom_settings`; scrapy-playwright [#131](https://github.com/scrapy-plugins/scrapy-playwright/issues/131) (reactor already installed, open since 2022), [#353](https://github.com/scrapy-plugins/scrapy-playwright/issues/353) "The future belongs to a different loop"; Apify SDK `main()` exiting the process ([SO 58543882](https://stackoverflow.com/q/58543882)); spider-py `stop()` does not stop ([#14](https://github.com/spider-rs/spider-py/issues/14)). Quote: "A limitation of its core component, the reactor, is that it cannot be restarted." ([Tryolabs, 2011](https://tryolabs.com/blog/2011/09/27/calling-scrapy-python-script)).
- **Status.** Scrapy: asyncio reactor default (2.13.0, 2025-05-08); experimental no-reactor mode (2.15.0) "becomes possible" to run several `AsyncCrawlerProcess`; Jupyter issue closed 2026-07 ("2.15.0+ can be used in Jupyter notebooks"). Crawlee and Crawl4AI never had the problem (user owns the loop); Crawlee v4 still adds a `SIGINT` listener per crawler and warns above 10 crawlers.
- **Lesson.** The SDK must be a library inside the user's program, never the owner of the process: no global singletons, constructible and runnable many times per process, usable from notebooks, web servers and task queues (Celery-style workers), cancellable with the host language's native mechanism. For Goodfetch's Attached engine this means the SDK-side lifecycle (spawn, reuse, restart, shutdown of the engine process) must be re-entrant and per-instance, and signal handling must be opt-in.

### 6.2 Unsafe defaults: politeness, robots.txt and identification
- **Evidence (5 independent issues, very high intensity, maintainers engaged).** [scrapy#6597](https://github.com/scrapy/scrapy/issues/6597) (17 r, 32 c, open): "Scrapy is actually a malicious denial-of-service tool in its default configuration" (site owner; 38,000 requests in 17 minutes; did not slow down after 429); comments report 109,106 and 243,656 hits. [scrapy#6755](https://github.com/scrapy/scrapy/issues/6755) (20 r, 24 c): "it opened like a thousand simultaneous HTTP connections"; a reply with 35 reactions: "if you are making a tool with very sharp edges, please add safety features". Maintainer: "Scrapy is a framework, not an app, and it's highly configurable". Discussion [#6848](https://github.com/scrapy/scrapy/discussions/6848) "Scrapy attacks my website" (10 reactions). [crawl4ai#107](https://github.com/unclecode/crawl4ai/issues/107) "Please respect robots.txt" (31 r, top issue). [colly#614](https://github.com/gocolly/colly/issues/614) (6 r, open). [firecrawl#1169](https://github.com/firecrawl/firecrawl/issues/1169) (7 r, 18 c, open): "Any legit bot crawler should set its own UA". HN: Firecrawl "doesn't even respect general robots.txt exclusions" at launch ([HN 40027403](https://news.ycombinator.com/item?id=40027403)).
- **Verified defaults.** robots.txt ignored by default in Crawlee JS and Python, Scrapy (library), Spider, Colly, Crawl4AI, Scrapling; obeyed by Firecrawl and the Scrapy project template; per-domain delay 0 everywhere except the Scrapy template (1 s, 1/domain since 2.13.3) and Crawl4AI (`mean_delay=0.1`). See §3.1.
- **Status.** Scrapy changed only the generated template (2.13.3); library defaults unchanged in 2.19.0; Zyte's hosted Scrapy Cloud enables AutoThrottle by default (maintainer comment). Crawlee v4 aligned robots error handling with RFC 9309 but did not flip the default.
- **Lesson.** Put safety in the **library defaults**, not in a scaffold: obey robots.txt (with an explicit, logged opt-out), per-host concurrency 1–2 and a non-zero delay, honor 429/503 and `Retry-After` with backoff, stop or pause a host on sustained error rates, and require an operator-identifying contact in the user agent for crawls (a setting with a clear error when absent, or at least a loud warning). The reputational cost lands on the tool: site owners now block `Scrapy` UAs wholesale.

### 6.3 Run state surprises: purge on start, second run does nothing, resume, re-running failures
- **Evidence (7 independent + vendor admission + redesign).** [crawlee#1602](https://github.com/apify/crawlee/issues/1602) (4 r, 14 c; "The second time the code is run it does not work."), [SO 74709844](https://stackoverflow.com/q/74709844) "How to reset crawlee URL cache?", [crawlee#3315](https://github.com/apify/crawlee/issues/3315) (3.15.3, restart with `purgeOnStart: false` does nothing), [#2196](https://github.com/apify/crawlee/issues/2196) storage dir ignored, [SO 76782655](https://stackoverflow.com/q/76782655) ENOENT at 64,000 pages, [crawlee-python#369](https://github.com/apify/crawlee-python/issues/369), [#1106](https://github.com/apify/crawlee-python/issues/1106) (12 c; handler never entered on rerun because of queue dedup), Discussions [#3254](https://github.com/apify/crawlee/discussions/3254) (14 c), crawlee-python [#1050](https://github.com/apify/crawlee-python/discussions/1050) "how to restart failed crawl jobs" (maintainer: "I don't think we have simple mechanisms for this"), [crawlee#1363](https://github.com/apify/crawlee/issues/1363) "put failed requests back to queue" (12 c, open since 2022). Vendor: "People are still asking about the storages, purging, resuming etc." ([#1782](https://github.com/apify/crawlee/issues/1782), open). Scrapy: `JOBDIR` resume is documented as supported only after a clean stop. Spider: no frontier resume; Katana resumes root inputs only.
- **Status.** Crawlee v4: per-instance default queues, repeated `run()` continues, **a run that processes nothing warns and says why**, new `purge()`, but `purgeOnStart` still wipes default storages once per process. crawlee-python opened the mirror issue [#2305](https://github.com/apify/crawlee-python/issues/2305) on 2026-10-13.
- **Lesson.** Make the **run** an explicit, named object with a lifecycle (`new` / `resume` / `restart` / `retry-failed`), never infer intent from a storage directory. Resume must survive an unclean stop (lease-based frontier, as the capability matrix proposes). When nothing is processed, say why. Provide a first-class "re-enqueue failed requests" operation.

### 6.4 Silent hangs and swallowed failures
- **Evidence (6 independent in forum notes + many GitHub threads).** Crawlee [#505](https://github.com/apify/crawlee/issues/505) "Crawler not finished / not resolving" (58 c), [#1829](https://github.com/apify/crawlee/issues/1829) "PlaywrightCrawler hangs infinitely when closing page" (15 c, 2023-03 → fixed 2026-09-24), [#2261](https://github.com/apify/crawlee/issues/2261) teardown never called; crawlee-python [#1694](https://github.com/apify/crawlee-python/issues/1694) rare deadlock (2026-01); scrapy-playwright [#266](https://github.com/scrapy-plugins/scrapy-playwright/issues/266) "Scrapy hangs with no exception raised", [#343](https://github.com/scrapy-plugins/scrapy-playwright/issues/343), [#371](https://github.com/scrapy-plugins/scrapy-playwright/issues/371) "`_get_response_and_download` hangs forever"; Spider [#268](https://github.com/spider-rs/spider/issues/268) `scrape()` never finishing with a subscriber; spider-nodejs [#2](https://github.com/spider-rs/spider-nodejs/issues/2) "crawlSmart just fails and doesn't throw an error"; Firecrawl [#2350](https://github.com/firecrawl/firecrawl/issues/2350) (10 r, 25 c) "stuck in an infinite loop", [#1679](https://github.com/firecrawl/firecrawl/issues/1679) (14 c), [#4212](https://github.com/firecrawl/firecrawl/issues/4212) failed and cancelled jobs returned "exactly like successful ones"; Katana [#416](https://github.com/projectdiscovery/katana/issues/416), [#1272](https://github.com/projectdiscovery/katana/issues/1272) (9 r). Browser driver errors leaking as raw exceptions: Crawlee Discussion [#2020](https://github.com/apify/crawlee/discussions/2020) "How would you troubleshoot Playwright 'Target page, context or browser has been closed'?", Crawl4AI [#842](https://github.com/unclecode/crawl4ai/issues/842) (36 c), [#895](https://github.com/unclecode/crawl4ai/issues/895), [#920](https://github.com/unclecode/crawl4ai/issues/920).
- **Status.** Crawlee v4 adds a whole-request internal timeout and bounded navigation hooks; Scrapy 2.19 adds `RemoteControl` + MCP server to inspect "why a crawl is slow, doesn't make requests, or makes requests without producing items".
- **Lesson.** Every phase needs a deadline (navigation, hooks, handler, sink write, shutdown), with the phase named in the error. Expose a live status that answers "what is in flight, what is waiting and why" (Foxhound run events; an introspection surface agents can use). Failures must never look like success: a job/result type with an explicit outcome enum, and errors that wrap driver exceptions in framework categories (Scrapy 2.15 did this for download handlers).

### 6.5 Switching fetch tier is a rewrite or a plugin integration
- **Evidence.** Likes L1 show the demand. Pains: "something I've had to hack on top of Scrapy in the past" ([HN 32561840](https://news.ycombinator.com/item?id=32561840)); Colly [#4](https://github.com/gocolly/colly/issues/4) (49 r, 30 c, top issue); SO "Can scrapy be used to scrape dynamic content…AJAX?" 168 votes, "selenium with scrapy" 101 votes, scrapy-splash 581 questions; scrapy-playwright page lifecycle ("crawls could freeze if the limit is reached and pages remain open"), meta-key confusion ([SO 75943441](https://stackoverflow.com/q/75943441), 4,594 views), Windows thread workaround, memory ("consumes gigabytes of memory" at 50 tabs, dev.to); Crawlee: competitor review "this was not a one-line class-only swap" (Thunderbit, author-of-competitor); `npx playwright install` friction ([#2275](https://github.com/apify/crawlee/issues/2275)); Spider: `chrome` feature not working ([Discussion #261](https://github.com/spider-rs/spider/discussions/261)), JS blocked by default interception ([#293](https://github.com/spider-rs/spider/issues/293)), smart mode semantics questioned ([#291](https://github.com/spider-rs/spider/issues/291), Discussion [#181](https://github.com/spider-rs/spider/discussions/181) "Programatically steer smart mode detection").
- **Status.** The field is converging on per-request or learned tier selection (Crawlee adaptive, Spider smart, Scrapling `sid=`, Firecrawl server waterfall).
- **Lesson.** Keep **one handler-context shape across tiers** (selectors, links, push, state), expose the browser `page` only as an optional capability that, when touched from HTTP, either escalates (Crawlee adaptive) or fails clearly. Let tier be chosen per request (route/label default, per-request override, learned escalation) at **runtime**, never at build time. Tie escalation to a result check, not only HTML heuristics (prior audit).

### 6.6 Documentation gaps, undocumented options, missing real-world examples
- **Evidence (12 independent).** Crawlee Python launch: "your cutesy coding style makes that a non-starter" (undocumented `tiered_proxy_urls`, [HN 40918032](https://news.ycombinator.com/item?id=40918032)); "I literally had to go through the entire codebase" ([HN 40919056](https://news.ycombinator.com/item?id=40919056)); "figuring out how to connect all the pieces for complicated use cases" ([crawlee-python#862](https://github.com/apify/crawlee-python/issues/862)); "it's all just magic" (architecture overview request, [#603](https://github.com/apify/crawlee-python/issues/603), vendor employee); Spider: "where does the data go?" ([#78](https://github.com/spider-rs/spider/issues/78)), Discussion [#142](https://github.com/spider-rs/spider/discussions/142) "Is there any documentation apart from docs.rs/spider?"; Crawl4AI: "LLMconfig barely has documentation" ([SO 79512095](https://stackoverflow.com/q/79512095)), docs example `fit_markdown` failing ([#453](https://github.com/unclecode/crawl4ai/issues/453), 8 r); Crawlee [#796](https://github.com/apify/crawlee/issues/796) "Improve Session Management guide." (21 r). Doc/code drift found in this audit: Spider README timeout 15 s vs code 120 s; Firecrawl JS README prose still names `FirecrawlApp`.
- **Lesson.** Generate the API reference from the protocol types (Goodfetch already plans `xtask codegen`), test every documented example in CI, ship an architecture page (Scrapy's data-flow diagram is what users ask Crawlee for), and structure upgrade guides like Crawlee v4's ("Changes most users will hit" then "Only if you…").

### 6.7 Install, packaging and runtime friction
- **Evidence (11 independent).** Browsers not installed with the package: Crawlee [#2275](https://github.com/apify/crawlee/issues/2275) (21 c; "It's not a great first experience to get a huge error on first run"), [SO 76225651](https://stackoverflow.com/q/76225651) Alpine Docker; bundling/serverless: [#2208](https://github.com/apify/crawlee/issues/2208) (10 r, open; "`npm install crawlee playwright` is not enough"); runtimes: Deno [#1913](https://github.com/apify/crawlee/issues/1913) (10 r), Bun [#2046](https://github.com/apify/crawlee/issues/2046) (7 r), HN 2025-09 "Deno doesn't work with crawlee"; Scrapy native build failures [#2115](https://github.com/scrapy/scrapy/issues/2115) (40 r, 41 c), [#2881](https://github.com/scrapy/scrapy/issues/2881) (9 r, 34 c); Scrapling extras [#343](https://github.com/D4Vinci/Scrapling/issues/343) and import-time crash [#396](https://github.com/D4Vinci/Scrapling/issues/396); spider-py install [#1](https://github.com/spider-rs/spider-py/issues/1), [#12](https://github.com/spider-rs/spider-py/issues/12), [#21](https://github.com/spider-rs/spider-py/issues/21), wheels [#15](https://github.com/spider-rs/spider-py/issues/15); crawlee-python dependency break on Pydantic ([#1464](https://github.com/apify/crawlee-python/issues/1464)) and browserforge ([#1718](https://github.com/apify/crawlee-python/issues/1718)). Crawlee v4 adds a new trap: `npm install --omit=optional` "breaks native dependencies".
- **Lesson.** Ship prebuilt engine binaries per platform through the language package managers (optionalDependencies/wheels/crates) with a tested matrix including Alpine/musl, containers, Bun/Deno where feasible; provision browsers with an explicit, idempotent command and a precise error when missing; keep the pure-HTTP path free of browser dependencies.

### 6.8 Breaking changes, churn and regressions
- **Evidence (9 independent + framework facts).** Firecrawl SDK: "can you please avoid breaking changes?" ([#2017](https://github.com/firecrawl/firecrawl/issues/2017), "Class doesn't exist anymore ScrapeOptions"), generic type break ([#675](https://github.com/firecrawl/firecrawl/issues/675)), v1→v2 method renames; Crawl4AI "silent API removals" ([#2091](https://github.com/unclecode/crawl4ai/issues/2091)), tests diverged ([#893](https://github.com/unclecode/crawl4ai/issues/893)); Crawlee patch break 3.13.8→3.13.9 ([#3063](https://github.com/apify/crawlee/issues/3063)); Scrapy performance regression 2.13 ([#7158](https://github.com/scrapy/scrapy/issues/7158), 13 r: 3.9 → 1.5 M items/min on the asyncio reactor, recovered to 3.7 in 2.14.0) and Discussion [#7280](https://github.com/scrapy/scrapy/discussions/7280) "Why is Scrapy 2.14.x so much slower than Scrapy 2.13.x?"; Scrapy `start_requests()` deprecated 2.13 (2025-05) and **removed 2.16** (2026-05, about 12 months), with confused reports ([#6818](https://github.com/scrapy/scrapy/issues/6818), [#7696](https://github.com/scrapy/scrapy/issues/7696)); Spider: 2,227 versions, regressions "again" ([#252](https://github.com/spider-rs/spider/issues/252)); Crawlee v4 "many" breaking changes; crawlee-python 1.0 storage and HTTP-client changes.
- **Lesson.** Version the protocol and the SDK surface separately, keep deprecations for a defined window with runtime warnings naming the replacement (Scrapy does this well), provide codemods or compatibility shims at the SDK edge only when product-promised (Goodfetch ADRs 0030/0041/0047), and benchmark releases to catch throughput regressions. Organize migration guides by blast radius.

### 6.9 Extensibility versus opinionated defaults
- **Evidence (9 pains + 4 Scrapy likes).** Crawlee Python launch: pain for "anyone who isn't on the happy path" ([HN 40917985](https://news.ycombinator.com/item?id=40917985)), "you need the flexibility middleware gives you" ([HN 40922731](https://news.ycombinator.com/item?id=40922731)), asks for proxy eviction and HTTP cache ([HN 40928731](https://news.ycombinator.com/item?id=40928731)); Crawlee "I didn't see any obvious way of decoupling" storage ([HN 32585288](https://news.ycombinator.com/item?id=32585288)); remote browser `connectOptions` ([#1822](https://github.com/apify/crawlee/issues/1822), 24 r, the most-reacted Crawlee issue); custom `enqueueLinks` ([SO 73477231](https://stackoverflow.com/q/73477231)); Spider custom HTTP client for TLS fingerprints ([#222](https://github.com/spider-rs/spider/issues/222)), access to reqwest internals ([#226](https://github.com/spider-rs/spider/issues/226)); Colly cache ([HN 40930346](https://news.ycombinator.com/item?id=40930346)); crawlee-python Discussion [#855](https://github.com/apify/crawlee-python/discussions/855) "Design Inspiration From Scrapy": crawler classes couple fetching with parsing. The cost of Scrapy's model is visible too: 100+ SO "settings" questions (257 votes), 100+ "pipeline" (496 votes), "difference between spider middleware and downloader middleware" (24 votes).
- **Status.** Crawlee v4 moved to injectable interfaces (`ISessionPool`, `IBrowserPool`, `IRenderingTypePredictor`, `IRequestManager`, `IStatistics`), `extendContext`, `router.use`; crawlee-python documents custom parsers rather than adding Scrapling.
- **Lesson.** Offer a small number of named, typed extension points at clear phases (before request, after response, before handler, after handler, on item, on error, on finish) plus injectable components for fetcher, parser, storage/sink, session/proxy, predictor; avoid import-path registries with magic integer ordering. Decouple fetch from parse (a handler should pick its parser, not its crawler class).

### 6.10 Passing data between handlers and the request payload model
- **Evidence.** SO: 4 questions, 168 votes (e.g. [SO 13910357](https://stackoverflow.com/q/13910357), 56 votes); [scrapy#1138](https://github.com/scrapy/scrapy/issues/1138) (18 r, 20 c; "understanding request.meta is a common struggle for beginners", maintainer); Scrapy 2.18 had to add `MetaCopyDetectionMiddleware`; [crawlee-python#524](https://github.com/apify/crawlee-python/issues/524) (20 c) and silent stringification of non-JSON values; [crawlee-python#1261](https://github.com/apify/crawlee-python/issues/1261) "The user_data doesn't belong exclusively to the user" (the framework stores `label` in it); Colly [#458](https://github.com/gocolly/colly/issues/458) "Issue with Context when using queue" (14 c, open).
- **Lesson.** Give each request a **typed, schema-validated, serializable payload** owned by the user, separate from framework-reserved fields (label, depth, retries); validate at enqueue time (fail loudly instead of stringifying), and generate the type in each SDK so the handler for label X receives payload type X.

### 6.11 Bindings and language ports lag the core
- **Evidence (7 independent).** spider-py last published 2025-01-28 while the crate advanced ~25 minor versions; "Build failure with current spider" ([spider-py#18](https://github.com/spider-rs/spider-py/issues/18)); "I can't figure out how to duplicate some of the current functionality" ([spider-py#6](https://github.com/spider-rs/spider-py/issues/6)); type hints requested twice; spider-nodejs "there haven't been any builds tagged since 0.0.70" ([#2](https://github.com/spider-rs/spider-nodejs/issues/2)); spider.cloud client "missing types for a lot of method parametesr" [sic] ([spider-clients#3](https://github.com/spider-rs/spider-clients/issues/3)); Crawlee Python homepage "That looks like Javascript?" ([HN 40915767](https://news.ycombinator.com/item?id=40915767)); vendor: adaptive crawling "isn't in Python yet" in 2024 ([HN 40919198](https://news.ycombinator.com/item?id=40919198)); parity maintained by mirror issues ([#2305](https://github.com/apify/crawlee-python/issues/2305), [#2133](https://github.com/apify/crawlee-python/issues/2133)).
- **Lesson.** This is the central risk of a Rust core with TS/Python/Rust SDKs. Generate SDK types from one protocol, release all SDKs in lockstep with the engine, run one conformance suite against every SDK, and make each SDK idiomatic (Crawlee Python was criticized for looking like JavaScript) while keeping semantics identical.

### 6.12 Compile-time capability gating (Cargo features)
- **Evidence (3 independent + source facts).** [spider#243](https://github.com/spider-rs/spider/issues/243) (default `io_uring` panics in Docker; "If I set container to run in privileged mode, then it works fine."), [spider-py#13](https://github.com/spider-rs/spider-py/issues/13) (fix: "Remove default features and \"io_uring\""), [spider#349](https://github.com/spider-rs/spider/issues/349) (feature combination fails to compile), Discussion [#261](https://github.com/spider-rs/spider/discussions/261); in source, features change field types and method signatures and turn methods into silent no-ops (§2.4); 132 features.
- **Lesson.** Confirms Goodfetch's rule "A capability is a setting, never a Cargo feature": capabilities negotiated at runtime with a clear "not available in this engine build" error, platform-sensitive I/O backends chosen at runtime with fallback, public types identical in every build.

### 6.13 Concurrency autoscaling and memory
- **Evidence.** crawlee-python [#1535](https://github.com/apify/crawlee-python/issues/1535) "Crawlee increasing concurrency until it dies" (Kubernetes OOM; log shows `cpu = 0.0; mem = 0.0`; resolved 2026-09-28), [#662](https://github.com/apify/crawlee-python/issues/662) memory average "seems wrong" (15 c), [#354](https://github.com/apify/crawlee-python/issues/354) slowdown after hours (15 c), [#1437](https://github.com/apify/crawlee-python/issues/1437) scaling on 429; Crawlee [#1306](https://github.com/apify/crawlee/issues/1306) "Autoscaled Pool doesn't properly downscale on overloaded client" (28 c); Crawl4AI [#1256](https://github.com/unclecode/crawl4ai/issues/1256) memory leak (10 r, open); Firecrawl [#722](https://github.com/firecrawl/firecrawl/issues/722) (6 r, 16 c); scrapy-playwright [#19](https://github.com/scrapy-plugins/scrapy-playwright/issues/19) heap OOM (15 c), [#183](https://github.com/scrapy-plugins/scrapy-playwright/issues/183); Katana [#1215](https://github.com/projectdiscovery/katana/issues/1215); Python GIL ceiling: crawlee-python Discussion [#1133](https://github.com/apify/crawlee-python/discussions/1133) (performance "saturates" around 2 cores).
- **Lesson.** Container-aware resource detection (cgroups), explicit ceilings that autoscaling never exceeds, a single budget shared across crawlers (Crawlee v4 `ConcurrencySystem`), and a visible "why concurrency is N" signal. A Rust engine removes the GIL ceiling for fetch/parse but handlers in Python still serialize: measure handler-bound throughput separately.

### 6.14 Duplicate or partial results on retry (commit boundary)
- **Evidence.** [crawlee#1216](https://github.com/apify/crawlee/issues/1216) "Requests are processed multiple times" (15 c; maintainer reproduction: request reclaimed and retried while the handler "still ran"); "It's Crawlee who's invoking handlers multiple times with the same request." ([SO 77358550](https://stackoverflow.com/q/77358550)); prior audit: `pushData` before `markRequestHandled` window.
- **Status.** Crawlee v4: handler storage writes transactional; commit at-least-once; `afterStorageCommit()`; queue additions write-through by default.
- **Lesson.** Define the handler as an attempt whose outputs commit only on success (Foxhound Sinks fed from a committed outbox), with idempotency keys so at-least-once delivery cannot duplicate items; document which effects are not rolled back.

### 6.15 Failure, retry and skip semantics
- **Evidence.** Retry from a callback: [scrapy#3590](https://github.com/scrapy/scrapy/issues/3590) (8 r, 21 c; "I miss everything the retry middleware currently does."); per-request delay: [scrapy#802](https://github.com/scrapy/scrapy/issues/802) (14 r, 38 c, open since 2014); 4xx retries not configurable: [crawlee-python#756](https://github.com/apify/crawlee-python/issues/756); Crawlee v3 skipping required "set `request.noRetry`, throw, then decrement `requestsFailed`" (v4 guide); Firecrawl failed jobs look successful ([#4212](https://github.com/firecrawl/firecrawl/issues/4212)); failed requests counted as "handled" in Crawlee (prior audit); Spider retries default 0 and errors are fields.
- **Lesson.** First-class outcomes per request: succeeded, failed (with category), skipped (with reason), blocked, retried; `retry(reason, delay)` and `skip(reason)` callable from the handler; per-request delay and backoff in the scheduler; status-code retry policy configurable per route.

### 6.16 Handler-context access and typing
- **Evidence.** Handler context gaps: Apify SDK `page.on('response')` not reachable in the handler ([SO 64355710](https://stackoverflow.com/q/64355710)); final URL after redirect ([spider#127](https://github.com/spider-rs/spider/issues/127)); `playwright_page` missing from meta ([SO 75943441](https://stackoverflow.com/q/75943441)); session cookies not in context ([crawlee-python#710](https://github.com/apify/crawlee-python/issues/710), 3 r, 9 c). Typing: [scrapy#4041](https://github.com/scrapy/scrapy/issues/4041) (15 r), Crawl4AI return type ([#1898](https://github.com/unclecode/crawl4ai/issues/1898)), spider-py hints ([#2](https://github.com/spider-rs/spider-py/issues/2), [#16](https://github.com/spider-rs/spider-py/issues/16)), Firecrawl generics ([#675](https://github.com/firecrawl/firecrawl/issues/675)), Crawlee v3 `Record` context (fixed in v4).
- **Lesson.** One method, one return type (Crawl4AI's config-dependent `arun()` return is the counter-example); strict, generated context types per tier; the final URL, redirect chain, response headers, cookies and session are always on the context.

### 6.17 Cookies and session API
- **Evidence.** [scrapy#1878](https://github.com/scrapy/scrapy/issues/1878) "Expose cookiejars" (25 r, open since 2016; "getting and setting cookies in Scrapy is really huge pain"), [#1448](https://github.com/scrapy/scrapy/issues/1448) (6 r); Crawlee Discussion [#2299](https://github.com/apify/crawlee/discussions/2299) "How to manually set session cookies" (5 up); crawlee-python Discussion [#2028](https://github.com/apify/crawlee-python/discussions/2028) "How to keep cookies on request retry?"; Crawlee v4: `sendRequest` "finally respects your Cookie header", browser cookies now persisted after the handler.
- **Lesson.** A first-class Session (identity) object with readable and writable cookies, proxy and fingerprint, shared consistently between HTTP and browser tiers (matches the capability matrix's `IdentityContext`).

### 6.18 Logging to stdout breaks protocol hosts
- **Evidence.** Crawl4AI [#1968](https://github.com/unclecode/crawl4ai/issues/1968) "AsyncLogger writes to stdout, breaking MCP stdio transport" (fixed 2026-05); Scrapling [#50](https://github.com/D4Vinci/Scrapling/issues/50) (16 c; owner suggested configuring logging; user kept `redirect_stdout`).
- **Lesson.** Already a Goodfetch trap rule (only transport, CLI writer and `tracing` touch stdout/stderr). The SDKs need the same discipline: no library prints, structured logs routed through the host's logger.

### 6.19 Self-hosted versus hosted parity
- **Evidence (7 independent).** Firecrawl: 15 of the top 25 issues by reactions tagged `[Self-Host]`; proxy support missing ([#1129](https://github.com/firecrawl/firecrawl/issues/1129), 18 r); "All scraping engines failed!" ([#1390](https://github.com/firecrawl/firecrawl/issues/1390), 14 r); closed-source anti-bot engine "the biggest deal breaker" ([HN 42056056](https://news.ycombinator.com/item?id=42056056)); Crawl4AI Docker memory leak. Apify SDK: KVS URL "works locally but not on the Apify Platform" ([apify-sdk-js#460](https://github.com/apify/apify-sdk-js/issues/460)).
- **Lesson.** Local engine and Goodfetch Cloud must expose the same contract; any cloud-only capability is negotiated and reported, never a silently different behaviour.

### 6.20 Maintenance and bus factor
- **Evidence.** Colly: no release 2020-06 → 2025-03, "is it Dead?" ([#833](https://github.com/gocolly/colly/issues/833)), "Looking for maintainers" ([#840](https://github.com/gocolly/colly/issues/840), open), sponsor withdrew. Spider bindings stale (6.11).
- **Lesson.** Release cadence is part of the API's perceived quality; visible changelogs and predictable releases matter as much as features.

### 6.21 Lessons condensed for Foxhound and Collie (analysis)

1. User's process owns the loop; SDK objects are re-entrant, restartable, notebook-safe (6.1).
2. Polite by default in the library: robots, per-host limits, delay, 429/Retry-After backoff, operator-identifying UA (6.2).
3. Runs are named objects with explicit resume/restart/retry-failed; "nothing to do" is explained (6.3).
4. Deadlines on every phase, live introspection, explicit outcome enums; never a silent hang or a failure that looks like success (6.4, 6.15).
5. One handler context across HTTP and browser; tier is a per-request runtime choice with learned escalation tied to result checks (6.5, 6.12).
6. Typed per-label payloads, validated at enqueue, separate from framework fields (6.10).
7. Handler outputs commit only on success into Datasets/Sinks with idempotency keys (6.14).
8. Generated, lockstep, conformance-tested SDKs; idiomatic per language (6.11, 6.6).
9. Few, typed extension points plus injectable components; decouple fetch and parse (6.9).
10. Offline replay of recorded responses and page-object style fixtures as the testing story (L2; Scrapy contracts, web-poet, Scrapling `development_mode`).
11. AI extraction in the handler compiles to cached selectors (Crawlee Python `PydanticAiSelectorExtractor`) so repeat pages cost no model call.
12. Prebuilt binaries, explicit browser provisioning, no stdout writes (6.7, 6.18).

---

## 7. Open questions and unverified items

**Coverage gaps**
- **Reddit (r/webscraping, r/Python, r/rust, r/node) could not be read in this pass**: login wall (302), 403, mirror 429, and the search tool refuses the domain; no attempt was made to bypass. The only Reddit material is the earlier [reddit-evidence follow-up](../../2026-10-07-reddit-evidence-followup.md) (October 7), which had no direct Crawlee/Scrapy/Spider API comparison. Lobsters and the Apify Discord mirror were also blocked. **The "likes" ranking therefore over-weights Hacker News and GitHub.**
- Hands-on long-form reviews of Crawlee, Crawl4AI, Firecrawl and Scrapling come mostly from Thunderbit, a competitor; they are cited only as supporting, never counted.
- No independent long-form review of Spider-rs or scrapy-poet exists in the sources reached; Spider's evidence is GitHub-only and its benchmark tables are vendor claims (unverified; prior audit found protocol problems).
- Download and usage numbers (npm, PyPI, crates.io downloads) were not compared; stars are not usage.

**Unverified or partially verified**
- **Crawlee v4**: the upgrade guide was read on `master` (`8cb2eb61`), not at `v4.0.0-rc.1`; some described behaviour may postdate rc.1. GA date unknown. Real-world reaction to v4 is not yet observable beyond RC issues filed 2026-10-09.
- `spider_rs` (Python) README calls `website.crawl()` without `await` inside an async function; whether that call blocks, schedules or does nothing was not tested.
- Crawlee Python `PydanticAiCrawler` and `PydanticAiSelectorExtractor`: read in source only, not executed; cache invalidation behaviour on markup drift not tested.
- Scrapy 2.19 `RemoteControl` extension and the official Scrapy MCP server: read in docs only; capabilities and security model not exercised.
- Crawl4AI 0.9.5 was released on 2026-10-14; its changes were not reviewed; claims use 0.9.x docs at the `v0.9.5` tag.
- Firecrawl SDK claims use `main` READMEs and `v2/types.py`; not matched line-by-line to the 4.50.0/4.46.0 published packages.
- CP13 (Crawlee Python purge leaves queue metadata, Discord) is **unverified (snippet only)**.
- Colly v2.3.0 is visible on the Go proxy (prior audit) but has no GitHub Release; the GitHub "latest release" is v2.2.0.
- Whether Scrapy's per-domain "8" default plus `DOWNLOAD_DELAY=0` alone explains the reported request rates is disputed in [#6597](https://github.com/scrapy/scrapy/issues/6597) (a maintainer asked how 2,200 RPM is possible with defaults; another pointed to [#6476](https://github.com/scrapy/scrapy/issues/6476)); the defaults themselves are verified.
- Heritrix and StormCrawler consumer APIs were not re-examined in this pass.

**Open design questions this raises for Goodfetch (for the owner, not decisions)**
1. Should Foxhound expose a **router with labels** (Crawlee) or **per-request callbacks** (Scrapy/Scrapling), given handlers run in the user's process behind JSON-RPC? Labels serialize naturally across the protocol; callbacks need a registry anyway.
2. Push (`push_data` into a Dataset) vs pull (`for await item of run.items()`): Scrapling offers both (`start()` and `stream()`); which is primary for Foxhound, and how does backpressure cross the JSON-RPC boundary?
3. Typed items: adopt a schema per Dataset (Scrapy items, web-poet, Pydantic/Zod) or Crawlee's "plain dict" default?
4. Should politeness defaults (robots obey, per-host 1–2, delay, identifying UA with contact) be hard defaults with explicit opt-out, given Scrapy's experience that template-only defaults do not reach script users?
5. Run identity and resume: is "continue by default, explain when nothing to do" (Crawlee v4) the right default, or must resume be explicit?
6. How is the browser `page` exposed in a cross-tier handler context over JSON-RPC (a remote handle with Collie primitives?), and what happens when an HTTP-tier handler touches it: escalate (Crawlee adaptive) or error?
7. Test story: record/replay of responses (Scrapy HTTP cache, Scrapling `development_mode`) plus page-object fixtures (web-poet): engine feature, SDK feature, or both?
