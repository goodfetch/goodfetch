# How the Scrapy family organises the work around one page (2026-11-06)

Input to wayfinder research ticket #2159 ("How do the leading crawlers organise the work around one page?"). Source reading only; nothing was run, installed or built from the read sources.

## Versions and sources

| Crawler | Version read | Where |
|---|---|---|
| Scrapy | **2.19.0** (2026-09-10), tag commit `8026deeaac371a5d9a3edbe4886d58f61139d464` | `git clone --depth 1 --branch 2.19.0`; cited as **S** = `https://github.com/scrapy/scrapy/blob/2.19.0/` + path. The `just compare` venv holds the same 2.19.0 wheel. |
| scrapy-playwright | **0.0.48**, with Playwright **1.64.1**, Twisted 26.10.0, parsel 1.12.1, lxml 6.1.3, w3lib 2.5.0, queuelib 1.10.0 | The `xtask/compare-scrapy/.venv` of the `spec-1577` worktree, cited as **V** = `/Users/jb/Dev/goodfetch/.claude/worktrees/spec-1577/xtask/compare-scrapy/.venv/lib/python3.12/site-packages/` + path. The path in the brief (`/Users/jb/Dev/goodfetch/xtask/compare-scrapy/.venv`) does not exist on this machine (the Hetzner server, `ubuntu-64gb-fsn1-1`); the worktree venv pins the same versions. |
| Scrapling | **v0.5.1** (latest release, 2026-11-04), commit `3a4bb5b0a008233c85269a0a38a6d08edd4ce329` | `git clone --depth 1 --branch v0.5.1`; cited as **L** = `https://github.com/D4Vinci/Scrapling/blob/v0.5.1/` + path. Its HTTP client, curl_cffi, was read at the pinned minimum **v0.16.3** and the latest **v0.17.0** through `gh api` (cited as **C**). |

The three Goodfetch measurements were taken as inputs and are not repeated: `research-spider-gap-20261106.md`, `research-browser-gap-20261106.md` (which already counts scrapy-playwright at about 20 CDP commands per page and 1 browser, 1 context) and `research-crawler-frontier-persistence-20261106.md` (which covers Scrapy's JOBDIR writes in detail).

## The 11 points at a glance

| # | Scrapy 2.19.0 (HTTP) | scrapy-playwright 0.0.48 (browser) | Scrapling v0.5.1 |
|---|---|---|---|
| 1. Tasks, hand-offs, concurrency | One thread, one asyncio-backed Twisted reactor. Per page: about 7 signals, 14 downloader and 6 spider middleware hops, 2 spawned coroutines (download, scrape), Deferred chains. `CONCURRENT_REQUESTS` 16 global, `CONCURRENT_REQUESTS_PER_DOMAIN` 8 per slot; AutoThrottle (off by default) changes the delay, never the concurrency | Scrapy's pipeline unchanged; the download handler becomes Playwright calls over a pipe to a Node driver. Per-context page semaphore (`PLAYWRIGHT_MAX_PAGES_PER_CONTEXT`, default `CONCURRENT_REQUESTS`) | One anyio task group; a poll loop (10–50 ms sleeps) spawns one task per request up to `concurrent_requests` (default **4**); optional per-domain `CapacityLimiter`; AutoThrottle with block back-off |
| 2. Fetch client, pooling, HTTP/2 | Twisted `HTTPConnectionPool`, persistent, `maxPersistentPerHost` = per-domain concurrency; HTTP/1.1 only by default. HTTP/2 via an optional `H2DownloadHandler` (TLS only) or the experimental httpx handler | Chrome's network stack; Scrapy's HTTP/1.1 handler still fetches robots.txt and non-`playwright` requests | curl_cffi `AsyncSession` with Chrome impersonation; **its pool holds at most 10 curl handles** (curl_cffi default, Scrapling passes none) |
| 3. Frontier, duplicate check | SHA-1 of a canonical JSON of method, canonical URL, body → 20-byte fingerprint in a Python `set` (≈95 B per URL). `DownloaderAwarePriorityQueue`: per-slot priority queues, pop takes the slot with the fewest active downloads. Memory LIFO by default; with `JOBDIR`, every request goes to a pickled disk queue and every fingerprint is appended to `requests.seen` | Same as Scrapy | `asyncio.PriorityQueue` (one global heap) + `set` of SHA-1 fingerprints (orjson, canonical URL) + a mirror dict for checkpoints. Optional checkpoint: whole queue and seen set pickled every 300 s, atomic rename, no fsync |
| 4. Politeness | Per-slot (host) concurrency + `DOWNLOAD_DELAY` spaced between dispatches, jitter ±50% by default (`DOWNLOAD_DELAY_JITTER` 0.5, new in 2.19). robots.txt (off by default) fetched once per netloc, kept forever, checked in a downloader middleware for every request; Crawl-delay **ignored** | Same as Scrapy | Delay slept **inside** the per-domain limiter before every request; robots.txt via Protego, cached per domain, Crawl-delay and Request-rate **honoured**; AutoThrottle doubles the delay on a block or follows `Retry-After` |
| 5. Parsing, links | Lazy: no parse until the callback touches `response.selector`/`.css`; then a full lxml tree from the decoded text (parsel). Links by XPath/CSS over the tree, `urljoin` against `<base href>` | `page.content()` serialised in the browser → Python `str` → re-encoded to bytes → decoded again → lxml tree | **Eager**: every `Response` builds an lxml tree in its constructor, used or not; `LinkExtractor` one XPath over the tree; `urljoin` ignores `<base href>` |
| 6. Main text | None (user's selectors; ecosystem add-ons only) | None | `Response.markdown(main_content_only=True)`: `<body>` only, two `deepcopy`s of the tree to strip noise tags and hidden/prompt-injection elements, then markdownify. No readability |
| 7. Written per page | Nothing by default. Items → pipelines (async chain) → `FeedExporter` writes each item to a temp file through the exporter's buffered file; storage upload at close or per batch | Same | Nothing by default; items kept **in memory** (`ItemList`) until the end unless the caller streams them; exports at the end (`to_jsonl` etc.). Dev-mode response cache: one file per response |
| 8. Browser | n/a | **1 browser, 1 context ("default")** for the crawl, created lazily; **a new page per request, closed after**; no recycling (restart only on disconnect); `page.route("**")` on every page, so every sub-request goes Chrome → Node → Python → Node → Chrome, and Playwright disables the HTTP cache once interception is on; no init script by default | 1 persistent context (patchright Chromium), **pages reused** from a pool (`max_pages` default **1**); per reuse: `set_extra_http_headers`, `unroute_all`, optional `route`; per page `context.cookies()` (whole jar), 3 header reads; proxy rotation = a new context per request. No recycling |
| 9. Memory | Fingerprints (≈95 B/URL), queued `Request` objects (or disk with JOBDIR), robots parsers per netloc, DNS LRU 10,000. Backpressure: the engine stops pulling when responses waiting for callbacks exceed 5 MB (`SCRAPER_SLOT_MAX_ACTIVE_SIZE`). `MEMUSAGE_LIMIT_MB` (0 = off) closes the spider; checked every 60 s | Adds `ScrapyPlaywrightMemoryUsageExtension`: Python RSS + the whole Playwright/Chrome process tree; on breach the spider closes (no recycling) | Seen set, pending dict, queue, items list, `stats.proxies` list (one entry per proxied request), per-domain limiter/delay dicts. No memory limit, no spill except the periodic checkpoint |
| 10. Stats, progress | In-memory dict, about 10 `inc_value` per page (one builds the request's raw HTTP bytes just to count them); a DEBUG "Crawled (200)" log line per page (default `LOG_LEVEL` DEBUG); `LogStats` once a minute | Adds 2–4 `inc_value` per sub-request and per sub-response (request handler, response handler) | Counters in a dataclass; an **INFO log line per fetched response**; an item log line formatted with `pprint.pformat` per item even when DEBUG is filtered; no periodic progress |
| 11. For Foxhound | see §11 | see §11 | see §11 |

## 1. Tasks and hand-offs one page crosses; concurrency

### Scrapy

Runtime: `TWISTED_REACTOR = "twisted.internet.asyncioreactor.AsyncioSelectorReactor"` by default (S `scrapy/settings/default_settings.py:609`), so everything runs on one asyncio event loop in one thread; DNS goes to a thread pool (`DNS_RESOLVER = CachingThreadedResolver`, `REACTOR_THREADPOOL_MAXSIZE = 10`, `DNSCACHE_SIZE = 10000`; `default_settings.py:285-288,510`; `scrapy/resolver.py:30`). 2.19 also runs without a reactor (`TWISTED_REACTOR_ENABLED`), where the aiohttp handler is the default (`docs/news.rst`, 2.19.0 "Backward-incompatible changes").

The loop that feeds the downloader is a "call later once" (`CallLaterOnce`, `scrapy/utils/reactor.py:50-82`) scheduled after every event that might free capacity, plus a 5 s heartbeat (`scrapy/core/engine.py:77-79,102,328-329`). Each run pulls from the scheduler until `needs_backout()` (`engine.py:351-375`): the downloader holds `CONCURRENT_REQUESTS` requests (`core/downloader/__init__.py:235-237`) or the scraper holds more than 5 MB of responses waiting for callbacks (`core/scraper.py:62-103`, `SCRAPER_SLOT_MAX_ACTIVE_SIZE = 5000000`, `default_settings.py:561`).

One page, from the link that names it to the links it yields:

1. **Callback output.** The spider's generator output passes back through the 6 base spider middlewares' `process_spider_output` (`default_settings.py:580-591`: Start, HttpError, Referer, UrlLength, Depth, MetaCopyDetection), up to `CONCURRENT_ITEMS` (100) outputs at a time (`core/scraper.py:399-438`).
2. **Scheduling.** `engine.crawl()` sends the `request_scheduled` signal, whose `OffsiteMiddleware` receiver drops off-site requests (`engine.py:449-469`; `downloadermiddlewares/offsite.py:66,79-90`); then `Scheduler.enqueue_request` runs the duplicate filter and pushes (`core/scheduler.py:364-385`), with 2 stat increments.
3. **Dequeue.** `_start_scheduled_request` pops, wraps the download in a Deferred chain with 3 `addBoth`/`addErrback` pairs (`engine.py:381-417`).
4. **Downloader middleware, request side.** 14 base middlewares (`default_settings.py:327-343`: Offsite, RobotsTxt, HttpAuth, DownloadTimeout, DefaultHeaders, UserAgent, Retry, MetaRefresh, HttpCompression, Redirect, Cookies, HttpProxy, DownloaderStats, HttpCache), each a no-op when its feature is off.
5. **Slot.** `_enqueue_request` finds or creates the slot, sends `request_reached_downloader`, queues `(request, Deferred)` and runs `_process_queue`, which spawns `_wait_for_download` as a new coroutine (`core/downloader/__init__.py:268-307`).
6. **Fetch.** The handler's Deferred chain (latency, body ready, body done, timeout; `core/downloader/handlers/http11.py:498-545`), with a `headers_received` signal and a `bytes_received` signal **per received chunk** (`http11.py:555,717-731`). Then `response_downloaded` (AutoThrottle listens here) and `request_left_downloader` (`core/downloader/__init__.py:313-342`).
7. **Downloader middleware, response side**, in reverse.
8. **Engine.** A DEBUG "Crawled (200)" log record and the `response_received` signal (`engine.py:519-532`; `logformatter.py:24,77-85`).
9. **Scraper.** `enqueue_scrape` queues the response and spawns `_wait_for_processing` as a coroutine (`core/scraper.py:219-246`), which runs `process_spider_input` and the callback (`scraper.py:247-349`).

So about 7 signal dispatches per page plus one per body chunk, 20 middleware hops, 2 spawned coroutines and a few Deferred-to-Future bridges. Signal dispatch walks pydispatcher's receivers and calls each through `robustApply` (`scrapy/utils/signal.py:36-63`).

**Concurrency settings** (`default_settings.py:262-265`): `CONCURRENT_REQUESTS = 16`, `CONCURRENT_REQUESTS_PER_DOMAIN = 8`, `CONCURRENT_REQUESTS_PER_IP = 0`, `CONCURRENT_ITEMS = 100`. The global cap counts requests waiting in slot queues as well as transferring ones (`downloader/__init__.py:224-237` adds to `active` before the slot), so one slow domain can hold global capacity; the priority queue counters this (§3).

**AutoThrottle** (`scrapy/extensions/throttle.py:61-127`, off by default; `default_settings.py:234-238`): on each `response_downloaded`, `target = latency / AUTOTHROTTLE_TARGET_CONCURRENCY` (default 1.0); `new = max(target, (old + target) / 2)`, clamped to `[DOWNLOAD_DELAY, AUTOTHROTTLE_MAX_DELAY = 60]`; a non-200 response may raise but never lower the delay; the first delay is `max(DOWNLOAD_DELAY, AUTOTHROTTLE_START_DELAY = 5)`. `latency` is `download_latency`, which the HTTP/1.1 handler measures **to the response headers**, not the end of the body (`http11.py:511-541`). AutoThrottle changes only the slot's delay; the per-slot concurrency stays `CONCURRENT_REQUESTS_PER_DOMAIN`, so the effective in-flight count converges to about the target concurrency through spacing.

### scrapy-playwright

The handler subclasses `HTTP11DownloadHandler` and takes requests whose `meta["playwright"]` is true (V `scrapy_playwright/handler.py:157,408-427`); others, robots.txt included, keep Scrapy's HTTP/1.1 path. It requires the asyncio reactor (`handler.py:161`). Per page, on top of Scrapy's hand-offs above: `_create_page` under a global context lock, the context's page semaphore, `context.new_page()`, 3–5 `page.on` listeners, `page.route("**")`, `page.goto`, `response.all_headers()`, `page.content()`, `response.security_details()`, `response.server_addr()`, `page.close()` (`handler.py:312-362,450-588`). Each is a JSON message over the pipe to Playwright's Node driver, which turns it into CDP. Concurrency: Scrapy's settings, then a per-context semaphore of `PLAYWRIGHT_MAX_PAGES_PER_CONTEXT`, default `CONCURRENT_REQUESTS` (`handler.py:150-151,306,326`), and an optional `PLAYWRIGHT_MAX_CONTEXTS` semaphore (`handler.py:185-186,266-268`).

### Scrapling

`CrawlerEngine.crawl` (L `scrapling/spiders/engine.py:356-453`) runs one anyio task group. The dispatcher is a **poll loop**: when the queue is empty it sleeps 50 ms; when `_active_tasks >= concurrent_requests` it sleeps 10 ms; otherwise it dequeues and `start_soon`s `_task_wrapper` (`engine.py:393-438`). A page's task: robots check and per-domain delay lookup (`engine.py:186-197`), optional dev cache read, the domain's `CapacityLimiter` (or the global one), the delay `sleep`, `session_manager.fetch` (`engine.py:214-236`), `is_blocked` (status in `{401, 403, 407, 429, 444, 500, 502, 503, 504}`, `spiders/spider.py:16,205-209`), AutoThrottle record, retry of blocked requests with priority − 1, then the callback's async generator: each `Request` goes through the allowed-domains check and `scheduler.enqueue`, each dict through `on_scraped_item` (`engine.py:155-184`). Defaults: `concurrent_requests = 4`, `concurrent_requests_per_domain = 0` (no per-domain limiter), `download_delay = 0` (`spiders/spider.py:83-85`).

AutoThrottle (`spiders/throttle.py:61-97`) uses Scrapy's formula, with `target_concurrency` defaulting to `concurrent_requests_per_domain` (or 1), plus a block rule: a non-2xx or blocked response sets the delay to `Retry-After` when present, otherwise doubles it, and a block can never shorten it. Latency is the whole `session_manager.fetch`, body and lxml parse included (`engine.py:226-228`, §5).

## 2. Fetch client, connection pooling, HTTP/2

- **Scrapy.** `HTTP11DownloadHandler` keeps one Twisted `HTTPConnectionPool(persistent=True)` with `maxPersistentPerHost = CONCURRENT_REQUESTS_PER_DOMAIN` (S `core/downloader/handlers/http11.py:104-108`). The body is accumulated in a `BytesIO` as chunks arrive (`http11.py:678,717-731`), with `DOWNLOAD_MAXSIZE` 1 GiB and `DOWNLOAD_WARNSIZE` 32 MiB (`default_settings.py:307-308`); certificates are not verified by default (`DOWNLOAD_VERIFY_CERTIFICATES = False`, `:317`). HTTP/2: not in the default handler; `H2DownloadHandler` (Twisted + h2, TLS only, no h2c, separate extra) or the experimental `HttpxDownloadHandler` with `HTTPX_HTTP2_ENABLED` (`docs/topics/download-handlers.rst:149-150,224,235-290,377-399`; `handlers/_httpx.py:80-102`). The experimental aiohttp handler's `TCPConnector` uses `limit = CONCURRENT_REQUESTS`, `limit_per_host = CONCURRENT_REQUESTS_PER_DOMAIN` and turns off its cookie jar and decompression, which middlewares own (`handlers/_aiohttp.py:55-70`; `_base_streaming.py:88-91`).
- **scrapy-playwright.** Chrome's network stack, one context so one connection pool and one cookie jar for the crawl. No Python-side pooling.
- **Scrapling.** `FetcherSession` → `_ASyncSessionLogic.__aenter__` creates `AsyncCurlSession()` with no arguments (L `scrapling/engines/static.py:434-441`). curl_cffi's `AsyncSession` defaults to `max_clients = 10` and hands handles out through an `asyncio.LifoQueue(max_clients)`; a request waits for a free handle (C `curl_cffi/requests/session.py:973-1083` at v0.16.3; `:1191-1294` at v0.17.0). So a spider set to 16 or 32 concurrent requests on the default session still has **at most 10 transfers in flight**. Impersonation defaults to `"chrome"`, stealth headers on, a Google referer added (`static.py:74-75,179-208`); retries 3 with a 1 s sleep (`static.py:81-82,457-507`). HTTP/3 is opt-in (`static.py:171`). Whether curl-impersonate's Chrome profile negotiates HTTP/2 by ALPN is **unverified** (not read in curl_cffi's C layer).

## 3. Frontier and duplicate check

**Scrapy.**
- **Fingerprint.** `RequestFingerprinter`: `canonicalize_url` (w3lib), then `json.dumps(sort_keys=True)` of method, URL, body hex (and optional headers), then SHA-1 → 20 bytes, cached per request in a `WeakKeyDictionary` (S `scrapy/utils/request.py:34-36,84-108`).
- **Seen set.** `RFPDupeFilter` keeps `set[bytes]` (`scrapy/dupefilters.py:103,140-147`); measured at **≈95 bytes per URL** in CPython (100,000 random 20-byte keys, `tracemalloc`). With `JOBDIR`, each new fingerprint is also appended, length-prefixed, to `requests.seen` through a buffered `a+b` file, never flushed per write (`dupefilters.py:117-120,145-146`).
- **Queue.** The default `SCHEDULER_PRIORITY_QUEUE` is `DownloaderAwarePriorityQueue`, over `LifoMemoryQueue` / `PickleLifoDiskQueue`, with FIFO queues for start requests (`default_settings.py:553-559`). It keeps one `ScrapyPriorityQueue` per download slot (`pqueues.py:358,421-426`); each of those is a dict of priority → downstream queue with a `curprio` pointer (`pqueues.py:173-228`). `pop()` asks the downloader how many requests are active in **every** slot that has queued requests and takes the one with the fewest, round-robin among ties (`pqueues.py:364-419`): O(slots) per pop, which buys fairness across hosts and keeps a full slot from blocking others.
- **Memory or disk.** Without `JOBDIR`, everything is in memory. With `JOBDIR`, the scheduler pushes every request to the disk queue first and uses memory only for requests that cannot be pickled (`core/scheduler.py:374-385,414-443`); per-slot subdirectories, `active.json` written at close (`scheduler.py:353-362,489-498`). Writes per page with `JOBDIR`: one fingerprint append per new link, one pickled request append per new link, one queue-file truncate per pop (queuelib `LifoDiskQueue`, V `queuelib/queue.py:224-256`). The frontier survey covers durability. 2.19 adds SQLite queues that commit each request in its own transaction (`docs/news.rst`, 2.19.0 "New features").
- **Where the check sits.** At scheduling, after the spider middlewares and the Offsite signal. Robots.txt is checked later, in the downloader middleware, so a disallowed URL is still fingerprinted, queued and dequeued before being dropped (§4).

**Scrapling.** `Scheduler` (L `scrapling/spiders/scheduler.py:12-100`): one `asyncio.PriorityQueue` of `(-priority, counter, request)` (BFS by counter, DFS with a negative counter), a `set[bytes]` of SHA-1 fingerprints over orjson-serialised canonical URL, method, body and optional kwargs and headers (`spiders/request.py:72-126`), a `_pending` dict mirroring the queue for snapshots and an `_inflight` dict. There is no per-domain queue, so a request for a domain whose limiter is full is dequeued anyway and its task waits on the limiter while holding one of `concurrent_requests` (`engine.py:214`, `432-438`): head-of-line blocking that Scrapy's queue avoids. Persistence is optional: `CheckpointManager.save` pickles the full pending list and seen set every `interval` (300 s) to a temp file and renames it, without fsync (`spiders/checkpoint.py:42-61`; `engine.py:299-304,416-417`); nothing is written per page.

## 4. Politeness

**Scrapy.**
- **Slot key and limits.** The key is the hostname (or the resolved IP with `CONCURRENT_REQUESTS_PER_IP`, or `meta["download_slot"]`). Per-slot settings come from `DOWNLOAD_SLOTS` (S `core/downloader/__init__.py:240-265`); idle slots are collected every 60 s (`:359-374`).
- **Delay.** `_process_queue` compares now with `slot.lastseen`, which is when the **previous request left the queue**, not when its response arrived. It sends one request and re-arms a timer when a delay is set, and drains up to the free transfer slots when not (`:285-311`).
- **Jitter.** `DOWNLOAD_DELAY_JITTER = 0.5` (±50%, new in 2.19, replacing `RANDOMIZE_DOWNLOAD_DELAY`), applied only when the delay is non-zero (`:115-119,136-178`; `default_settings.py:292-293`).
- **Cost per page with delay 0:** a dict lookup, a set add and remove, a deque push and pop.
- **robots.txt.** `ROBOTSTXT_OBEY = False` by default (`default_settings.py:549-551`). When on, the first request to a netloc downloads `/robots.txt` through the engine, and concurrent requests to that netloc wait on a Deferred. The Protego parser is cached in a dict for the whole crawl, with no expiry and no size bound. Every request then pays `rp.allowed(url, ua)` (`downloadermiddlewares/robotstxt.py:58-141`). The parser exposes `crawl_delay()` (`scrapy/robotstxt.py:70-121`), but nothing in the middleware or the downloader reads it: Scrapy **ignores Crawl-delay**.

**Scrapling.**
- **Delay.** The delay is `max(download_delay, robots Crawl-delay, period / Request-rate count)`, cached per domain (L `spiders/engine.py:107-137`). It is slept **inside** the domain limiter before every request (`engine.py:214-219`), so it is a pause per request per concurrent slot, not spacing between dispatches. With per-domain concurrency C, latency l and delay d, the rate is about C / (d + l).
- **No jitter.**
- **robots.txt.** `RobotsTxtManager`: Protego per netloc, a lock per domain for the first fetch, seeds prefetched from `start_urls`, and the cache is never evicted (`spiders/robotstxt.py:10-64`; `engine.py:337-354`).
- **Allowed-domains check.** A linear loop over `allowed_domains` per yielded request (`engine.py:96-105`).

## 5. Parsing and link extraction

**Scrapy.**
- **When.** Lazy: `TextResponse.selector` builds a parsel `Selector` on first access (S `scrapy/http/response/text.py:161-167`). parsel parses the whole document into an lxml tree, `HTMLParser(recover=True, huge_tree=True)` (V `parsel/selector.py:120-160`).
- **Decoding.** `response.text` decodes the body to a Python `str` once and caches it (`text.py:105-140`). 2.19 falls back to charset-normalizer when the body is neither ASCII nor UTF-8 (`text.py:142-150`; `docs/news.rst`).
- **Links.** `follow_all(css="a")` runs the CSS query on the tree and builds one `Request` per `href` (`text.py:238-316`). `urljoin` uses `get_base_url`, which honours `<base href>` and is cached per response in a `WeakKeyDictionary` (`text.py:116-119`; `utils/response.py:28-38`). `LxmlLinkExtractor` (CrawlSpider) walks the same tree, joins, `safe_url_string`s, optionally canonicalises, and de-duplicates per page (`linkextractors/lxmlhtml.py:116-168`).
- **No streaming parse.** The body is fully buffered before parsing.

**scrapy-playwright.** The body is `page.content()` (V `scrapy_playwright/_utils.py:62-89`): the browser serialises the DOM, the driver sends it as JSON, and Python re-encodes the `str` with the header or `<meta>` charset (`_utils.py:28-43`; `handler.py:577-588`). Scrapy then decodes it again before lxml parses it. So there are three copies of the document in Python before the tree.

**Scrapling.**
- **When.** **Eager**: `Response` subclasses `Selector`, whose constructor replaces NUL bytes (a copy) and parses the whole body with `HTMLParser(recover, remove_blank_text, remove_comments, compact, huge_tree)` (L `scrapling/parser.py:141-165`; `engines/toolbelt/custom.py:31-85`). Every fetched response is parsed, including robots.txt, blocked pages and pages whose callback never reads the DOM.
- **Links.** `LinkExtractor.extract` runs one XPath union over `tag/@attr` pairs, then joins, filters and canonicalises (`spiders/links.py:226-272`). `Selector.urljoin` joins against the response URL only, **ignoring `<base href>`** (`parser.py:337-339`), a correctness gap compared with Scrapy.

## 6. Main-text extraction or cleaning

- **Scrapy and scrapy-playwright:** none in the core; the user writes selectors. The `just compare` extraction scenario takes `main ::text` (`xtask/compare-scrapy/crawl.py`, the `parse` method).
- **Scrapling:** `Response.markdown(css_selector, main_content_only)` (L `engines/toolbelt/custom.py:92-106`). "Main content" means the `<body>` element. `_strip_noise_tags` and `_sanitize_for_ai` each `deepcopy` the lxml tree and remove noise tags, then CSS-hidden, `aria-hidden`, `<template>` and hidden-slot elements (`core/shell.py:73,103-108,639-660`). The result is serialised to HTML and converted by `markdownify`, which re-parses it with BeautifulSoup (`shell.py:627-636`). That is four tree passes per page and no readability-style scoring. The `site_to_markdown` template turns it on by default (`spiders/templates/site_to_markdown.py:40-47,72`).

## 7. What is written per page, and when

- **Scrapy.** Nothing by default. Items run through `ItemPipelineManager` (`core/scraper.py:496-560`), each pipeline awaited in order. `FeedExporter.item_scraped` calls `exporter.export_item` straight away (S `extensions/feedexport.py:667-685`); `JsonLinesItemExporter` encodes and writes to the slot's file, a `NamedTemporaryFile` for remote storages (`exporters.py:141-146`; `feedexport.py:145-156`). There is no fsync. The file is uploaded at close, or every `FEED_EXPORT_BATCH_ITEM_COUNT` items (`default_settings.py:371`), and remote stores upload in a thread (`feedexport.py:152-156`). Stats and the log are the only other per-page writes (§10).
- **scrapy-playwright.** Same as Scrapy; nothing of its own.
- **Scrapling.** Nothing by default. Items are appended to an in-memory `ItemList` (L `spiders/engine.py:167-175`), or sent through a 100-item memory stream when the caller iterates the spider (`engine.py:460-480`). They are written only when the caller calls `to_json`, `to_jsonl`, `to_csv` or `to_xml` after the crawl (`spiders/result.py:37-117`). In development mode, each response is written as one orjson file (`spiders/cache.py:55-80`).

## 8. Browser

### scrapy-playwright 0.0.48 with Playwright 1.64.1

- **Browsers per crawl:** one, launched lazily under `browser_launch_lock` when the first context is needed (V `handler.py:244-254`). It is never recycled. On `disconnected`, every context closes, and the next request launches a new browser when `PLAYWRIGHT_RESTART_DISCONNECTED_BROWSER` is true, its default (`handler.py:142-143,730-739`). Remote browsers attach through `PLAYWRIGHT_CDP_URL` or `PLAYWRIGHT_CONNECT_URL` (`provider.py:85-109`).
- **Contexts:** named by `meta["playwright_context"]`, default `"default"`, and created on first use under a global `context_launch_lock` (`handler.py:312-324`). `PLAYWRIGHT_CONTEXTS` pre-creates contexts at start (`:232-242`). `PLAYWRIGHT_MAX_CONTEXTS` (default unset, so unlimited) is a semaphore released when a context closes (`:185-186,266-268,748-768`). A context with `user_data_dir` is a persistent context (`:272-274`). A context lives until the user closes it, through `playwright_include_page`, or the crawl ends: there is no per-request context and no context recycling. Context options come per request from `meta["playwright_context_kwargs"]`, but only on the context's first request.
- **Pages:** a **new page per request** (`context.new_page()`, `handler.py:326-331`), **closed after the response is built** (`:559-561`) unless `playwright_include_page` hands it to the callback. A caller can pass a page back in through `meta["playwright_page"]` to reuse it; the handler then calls `unroute("**")` before routing again (`:451-455,469-484`). The page semaphore is released from the page's `close` or `crash` event (`:353-355,741-746`).
- **Init scripts:** none by default. A user hook, `playwright_page_init_callback`, runs per page (`_utils.py:156-182`), and `playwright_page_methods` run after `goto`, each followed by `wait_for_load_state` (`handler.py:675-714`).
- **Protocol calls per page** (Python → driver):
  - `newPage`, `route` (→ `setNetworkInterceptionPatterns`), `goto` with the default `load` wait, and `close`;
  - the response's `rawResponseHeaders` (`all_headers`), plus one `response()` per redirect hop in `_set_redirect_meta`;
  - `content`, `securityDetails` and `serverAddr` (`handler.py:519-561`; V `playwright/_impl/_network.py:862-900`; `_frame.py:500-501`);
  - per sub-request: one `rawRequestHeaders` from `use_scrapy_headers` (`headers.py:12-41`; `_network.py:347-357`) and one `continue`.

  The browser research's count of about 20 CDP commands per page stands for the driver-to-Chrome side.
- **Request interception:** `page.route("**", handler)` is installed on **every** page whatever the settings (`handler.py:472-484`), so every request the page makes, the document and each sub-resource, is paused and resumed:
  - **Path:** Chrome sends `Fetch.requestPaused` to the Node driver, which sends a `route` event to Python. Python's handler runs `abort_request` if set, then `process_request_headers` (by default `use_scrapy_headers`, one more driver round trip for the raw headers), then `route.continue_` back to the driver and `Fetch.continueRequest` to Chrome (`handler.py:770-884`).
  - **Cache:** when interception turns on, Playwright also sends `Network.setCacheDisabled {cacheDisabled: true}`, so **Chrome's HTTP cache is off for every intercepted page** and shared sub-resources are re-downloaded per page (V `playwright/driver/package/lib/coreBundle.js:37455-37475`; pause handling at `:37364,37535-37560`).
  - **Opt-outs:** `PLAYWRIGHT_PROCESS_REQUEST_HEADERS = None` removes the header-merge round trip but not the route itself. `PLAYWRIGHT_ABORT_REQUEST` adds an abort decision in Python per request.
- **Stats listeners:** `page.on("request")` and `page.on("response")` stat counters per sub-request (`handler.py:356-357,716-728`); with DEBUG logging, two more listeners that each await a header read (`:358-360`; `_utils.py:185-229`).
- **Download handling:** `download` and `response` listeners are added and removed per page (`handler.py:590-673`).

### Scrapling v0.5.1 (`StealthySession` / `AsyncStealthySession`, patchright)

- **Browser and context:** `launch_persistent_context` with a temporary user-data directory gives one browser and one context per session (L `engines/_browsers/_stealth.py:411-417`). With a `proxy_rotator`, the session launches a plain browser and opens a **fresh context for every request**, closed afterwards (`engines/_browsers/_base.py:447-474`). `RemoteSession` connects over CDP (`_remote.py:40-41,69`). Context options include `color_scheme: "dark"` and **`device_scale_factor: 2`** (`_base.py:502`), which doubles the pixels rendered per page; the CPU cost is **unverified**.
- **Pages: reused.** `PagePool(max_pages)`, default **`max_pages = 1`** (`_validators.py:62`; `_page.py:46-108`). A finished page is marked ready and reused (`_base.py:476-486`); a page in error is closed and dropped. When the pool is full, `_get_page` **holds the session's `asyncio.Lock` while polling every 50 ms, for up to 60 s**, for a page to become ready (`_base.py:263-273,346-372`). Page acquisition is serialised, and a waiting page pays up to 50 ms extra.
- **Per page (reuse), protocol calls:** `set_extra_http_headers` (always, `{}` when none), `unroute_all`, and `route("**/*")` only when `disable_resources` or `blocked_domains` is set (`_base.py:376-383`). Then `goto`, `all_headers` on the final response, `content` (or a shadow-DOM snapshot `evaluate`), `all_headers` on the first response and on its request, and **`context.cookies()` for the whole persistent context**, whose cost grows with the jar over the crawl. Redirect hops add header reads (`engines/toolbelt/convertor.py:332-430,259-299`). A `response` listener is added and removed per fetch (`_stealth.py:591,653`). `wait_for_load_state("load")` and then `"domcontentloaded"` (because `load_dom = True` by default) are resolved client-side when the event has already fired (V `playwright/_impl/_frame.py:301-315`; patchright is assumed to keep this, **unverified**).
- **Init scripts:** none unless `init_script` is set; stealth lives in launch flags (`engines/constants.py:15-84`) and in patchright itself (patchright's own CDP behaviour, for example avoiding `Runtime.enable`, is **unverified** here).
- **Interception:** off by default. With `disable_resources` or `blocked_domains`, every request goes to a Python handler that aborts or continues it (`engines/toolbelt/navigation.py:70-92`).
- **Recycling:** none; the context lives for the session.

## 9. Memory: what grows per URL, bounds, spill

- **Scrapy.**
  - **Per URL ever seen:** one fingerprint (≈95 B).
  - **Per URL queued:** a `Request` object (headers, meta dict, callback reference) in memory, or a pickled record on disk with `JOBDIR`.
  - **Per host:** a downloader slot, collected after 60 s idle; a robots parser, never collected; a DNS cache entry (LRU of 10,000).
  - **Per page in flight:** the buffered body, the decoded text, and the lxml tree if the callback touched it.
  - **Bounds:** the scraper's 5 MB backpressure limits responses waiting for callbacks (§1). `MEMUSAGE_ENABLED = True`, but `MEMUSAGE_LIMIT_MB = 0` and `MEMUSAGE_WARNING_MB = 0` mean the extension only records `memusage/max`. It reads `ru_maxrss` (peak, not current) every `MEMUSAGE_CHECK_INTERVAL_SECONDS = 60`, and when the limit is set it closes the spider with `memusage_exceeded` (S `extensions/memusage.py:39-125`; `default_settings.py:492-496`).
  - **Spill:** the only one is `JOBDIR`'s disk queue.
- **scrapy-playwright.** One browser and one context for the crawl; pages are closed per request, so the browser holds one renderer per in-flight page. `ScrapyPlaywrightMemoryUsageExtension` adds the RSS of the Playwright driver and all its descendants, Chrome included, by walking the process tree with psutil at each check (V `scrapy_playwright/memusage.py:14-62`). Exceeding the limit closes the spider; nothing recycles the browser.
- **Scrapling.**
  - **Per URL:** a seen fingerprint, a queued `Request` and its `_pending` entry.
  - **Per item:** the item itself, kept until the end unless streamed.
  - **Per proxied request:** one `stats.proxies` entry (`engine.py:221-224`), unbounded.
  - **Per domain:** a limiter, a delay, a robots parser and an AutoThrottle delay, never evicted.
  - **Bounds:** no memory limit setting and no spill; the checkpoint is a periodic copy, not a spill. The network recorder, opt-in, is bounded at 1,000 responses and a byte cap (`engines/_browsers/_network.py:16-80`).

## 10. Stats and progress: cost per page

- **Scrapy.** `MemoryStatsCollector` is a dict; `inc_value` is a `setdefault` plus an add (S `statscollectors.py:77-83`). Per page:
  - **Scheduler:** `scheduler/enqueued/memory`, `scheduler/enqueued`, `scheduler/dequeued/memory`, `scheduler/dequeued` (`core/scheduler.py:379-405`).
  - **`DownloaderStats` (`downloadermiddlewares/stats.py:49-72`):** request count, method count and request bytes, where request bytes **serialises the whole request to raw HTTP (`request_httprepr`) just to measure its length**; then response count, status count, and response bytes from the body length plus headers.
  - **`CoreStats`:** `response_received_count` and, per item, `item_scraped_count` (`extensions/corestats.py:51-60`).

  With the default `LOG_LEVEL = "DEBUG"` (`default_settings.py:464`), every page also emits a "Crawled (200) …" record and every item a "Scraped from" record (`logformatter.py:77-85`). `LogStats` prints pages and items per minute every `LOGSTATS_INTERVAL = 60` (`extensions/logstats.py:49-52`; `default_settings.py:480`). 2.19's `RemoteControl` extension, enabled by default, lets a client inspect the crawl over HTTP (`docs/news.rst`); its per-page cost was not read and is **unverified**.
- **scrapy-playwright.** Plus 2–4 `inc_value` per sub-request and per sub-response, keys built with f-strings (V `handler.py:716-728`), and page and context gauges per page (`:332-349,367-377`).
- **Scrapling.** `CrawlStats` counters per response: requests per session ID, bytes per domain, status counts (L `spiders/result.py:159-168`; `engine.py:229-231`). `Response.__init__` logs `Fetched (200) <GET url>` at **INFO for every response** (`engines/toolbelt/custom.py:78-79`). Each scraped item is formatted with `pprint.pformat` inside an f-string passed to `log.debug`, so the item is pretty-printed even when DEBUG is filtered (`spiders/engine.py:171`). The spider's default `logging_level` is DEBUG (`spiders/spider.py:101`). There is no periodic progress line, only the final stats dump (`engine.py:452`).

## 11. What Foxhound could adopt, and what would be a loss

Each item is framed as a setting with an off state, per the owner's rules, and checked against what the three Goodfetch measurements found.

**Adoptable.**

1. **Lazy parse.** Build a page's tree only when a consumer asks for it, as Scrapy does (§5). Scrapling's eager parse shows the cost of doing otherwise. Foxhound's raw cell already parses in one streaming pass; the rule is not to build a DOM for responses nobody reads (robots, blocked pages, non-HTML, redirects). Setting: none needed, as this is an implementation rule; off state n/a.
2. **Host-fair frontier pop.** Like `DownloaderAwarePriorityQueue`, take the next URL from the host with the fewest active fetches, so a saturated host never holds a global concurrency slot (§3). Scrapling's single heap shows the head-of-line blocking it avoids. Setting `crawl.frontier.order: host_fair | priority` (off = plain priority).
3. **Backpressure on parse work, not only on fetches.** Scrapy stops dequeuing when responses awaiting callbacks exceed 5 MB (`SCRAPER_SLOT_MAX_ACTIVE_SIZE`). Foxhound can bound its analysis queue the same way, so fetches never outrun extraction and memory stays flat. Setting: a byte budget with 0 = unbounded.
4. **Politeness off the hot path.** When delay is 0 and concurrency is unbounded, Scrapy's slot costs a dict lookup and a deque push and pop, with no actor and no transaction. The spider gap measurement found Foxhound consulting the Politeness actor per page even when it is non-binding. Short-circuit when the effective Politeness for a host is "no delay, concurrency ≥ global" (no feature lost; Politeness stays a setting).
5. **Dispatch-to-dispatch delay with jitter.** Scrapy measures the delay from the previous dispatch and jitters it ±50% by default; Scrapling sleeps per request inside the limiter, which wastes concurrency. If Foxhound's delay semantics differ, document them; a `politeness.jitter` setting (0 = off) matches Scrapy 2.19's `DOWNLOAD_DELAY_JITTER`.
6. **AutoThrottle with block back-off.** Scrapy's latency-driven delay plus Scrapling's rule (double on a block, follow `Retry-After`, never speed up on an error). Setting `politeness.adaptive: off | latency | latency+block`. Use latency to the response headers, as Scrapy does, not the whole fetch and parse.
7. **Browser: one browser, few contexts, reused pages.** scrapy-playwright's single browser and single context is what the browser research found cheap; Scrapling also reuses pages, saving target creation, attach and domain enables per page. For Foxhound: let a pool `browser.page_reuse: on | off` keep a target and navigate it again within one Session, resetting only what isolation needs. Combined with the ADR 0040 amendment the browser research proposes (no recycle at twice the warm-up), this attacks the 43-browser and 57-command figures. Contexts per Session remain a promise; a `sessions.isolation: context | shared` setting could let crawls that need no isolation share one context.
8. **Interception only when something needs it.** scrapy-playwright routes every request through Python, and that turns Chrome's HTTP cache off; Scrapling routes only when blocking is on. Foxhound should enable `Fetch` only when a feature needs it on that page (resource blocking, live hiding, body capture), and say in the docs that interception disables the browser cache. Setting: interception follows the features that need it; off when none do.
9. **Stats as plain counters; per-page logging at debug, formatted lazily.** Scrapy's dict counters and minute-level `LogStats` are the model. Avoid Scrapy's `request_httprepr`-for-length and Scrapling's INFO line per response and eager `pformat`. Setting: `progress.interval` (0 = off).
10. **Memory limit as a stop rule, measured on the whole process tree.** scrapy-playwright's extension adds Chrome's RSS to the budget and closes cleanly. Foxhound already recycles; a run-level `memory.limit` with a "stop" action, and a warning threshold, matches Scrapy at off-by-default.
11. **robots.txt Crawl-delay and Request-rate.** Scrapling honours them; Scrapy ignores Crawl-delay. If Foxhound honours them, keep it, and keep a setting to ignore them (Scrapy's behaviour) for owners who allow it.

**What would be a loss.**

- **Scrapy's in-memory default frontier with no per-page durability.** Adopting it as the only mode would drop Foxhound's resumable run file. Make durability a setting (Foxhound has `durability: off`); the loss is only in removing the durable mode.
- **scrapy-playwright's page-per-request with route-everything.** Simple, but it disables the HTTP cache and puts every sub-request through Python; copying it would cost CPU and bandwidth.
- **Scrapling's eager parse, poll loops (10–50 ms sleeps), lock held while polling for a page, curl handle cap of 10, `<base href>` ignored, in-memory items.** None of these should be copied.
- **Scrapling's "main content" = `<body>` + markdownify.** It is weaker than Foxhound's readability-based extraction; adopting it would regress extraction quality. Its cleaning of hidden and prompt-injection elements is a feature worth checking against Foxhound's (Consent and hiding already cover part of it).
- **Scrapy's lack of HTTP/2 in the default handler.** Not something to adopt: Foxhound's reqwest transport already pools and can negotiate HTTP/2.

## Unverified

- Whether curl-impersonate's Chrome profile negotiates HTTP/2 over ALPN in Scrapling's default session (not read in curl_cffi's C layer).
- The per-page CPU cost of Scrapling's `device_scale_factor: 2`.
- Patchright's internals: whether it matches Playwright 1.64's client-side `wait_for_load_state`, and how it avoids `Runtime.enable`.
- The per-page cost of Scrapy 2.19's `RemoteControl` extension (enabled by default; not read).
- Every CPU and time figure in this note. The only measured number is the ≈95 B per fingerprint (`tracemalloc`, 100,000 keys). Everything else is source reading, not measurement.
