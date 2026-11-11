# How Crawlee organises the work around one page

Research for goodfetch/goodfetch#2159, 2026-11-06. Read-only source reading; nothing was built, installed or run.

**Versions read**

- **Crawlee JS v3.18.2**, tag commit `78da0622762e1b569b5cb0b56e8eb06a0a010d93` (npm `latest` is 3.18.2). Links below point at the tag.
- **Crawlee JS master**, commit `9b310c77d23ab01bc56f3c28351612529d82da21` (2026-11-06, packages at 4.0.0; npm `rc` is 4.0.0-rc.1). Only differences that matter are noted, and master was read less thoroughly than v3.18.2.
- **Crawlee Python v1.11.1**, tag commit `6ffb48fe414a6c1d819a6bfad870a20ab73dfbec` (latest release on PyPI).
- Dependencies that do the work and live outside the Crawlee repositories, read from their npm archives: `got-scraping` 4.2.2 (the v3 default HTTP client, `^4.2.1` in `packages/http-crawler/package.json`), `fingerprint-injector` 2.1.89 (`^2.1.68`), `proper-lockfile` 4.1.2 (`^4.1.2`). The lockfile in the repository may pin other patch versions: **unverified** which exact patch a v3.18.2 install resolves.

**Inputs not repeated:** the Spider gap measurement, the browser gap measurement and the frontier persistence survey (`.claude/handoff/wiki/research-*-20261106.md`). The persistence survey already covers Crawlee JS's request queue durability; this note adds the per-page writes and the code path, not the crash analysis.

**Citations:** linked references point at the tag (JS v3.18.2, Python v1.11.1) or the master commit. A short reference such as `` `#L1605` `` or `` `request-queue.ts#L64` `` refers to the last linked file of that name in the same paragraph or list, at the same version. Dependency references (`got-scraping@4.2.2 dist/index.js:1071`) are to the npm archive of that version.

**No measurement was made.** Every "cost per page" here is a count of operations read from the source (calls, round trips, file writes), not a timing.

## Table: the 11 points

| # | Crawlee JS v3.18.2 (HTTP: `CheerioCrawler`/`HttpCrawler`; browser: `PlaywrightCrawler`) | Crawlee Python 1.11.1 (HTTP: `BeautifulSoupCrawler`/`ParselCrawler`; browser: `PlaywrightCrawler`) |
|---|---|---|
| 1. Tasks and hand-offs; concurrency | One Node event loop, no worker threads. `AutoscaledPool` starts one promise per page (`_runTaskFunction`): fetch next request → get Session → (browser: `BrowserPool.newPage`, serialised pool-wide by `pLimit(1)`) → navigate/fetch → parse → user handler → `markRequestHandled`. No actor, no queue between steps; storage mutations go through a per-file async queue and a lock file. Concurrency starts at `minConcurrency` (1) and grows by `ceil(5%)` (so +1 below 20) every 10 s while the system is idle and ≥ 90% of the desired slots are busy; max 200. Overload = CPU > 95% (whole machine), memory > 25% of RAM (own process tree, `ps` every 1 s), event loop blocked > 50 ms, or storage client errors, over 5 s windows. | One asyncio loop. An orchestrator coroutine spawns one task per page while `current < desired`, calling `is_finished` and `is_task_ready` before each spawn. Same pipeline as JS, but written as a composed "context pipeline" of async generators; `add_requests`/`push_data` are buffered and committed after the handler succeeds. Parsing (`BeautifulSoup`/`Parsel`), every JSON dump and every file write go to the default thread pool (`asyncio.to_thread`). Concurrency: `min` 1, `desired` 10 at start, `max` 100; same 10 s, 5% autoscaling and the same overload rules. |
| 2. Fetch client, pooling, HTTP/2 | `got-scraping` on `got`: `http2: true` with ALPN probe (`http2-wrapper` `auto`), Node global agents for HTTP/1.1 (one pool per process), a fresh agent with no free sockets kept per request when a proxy is set. Browser-like headers generated per Session (`header-generator`), cached on the Session. TLS verification off by default. Master: default client is `impit` (Rust), lazily loaded, with `fetch` as fallback. | `impit` (Rust) `AsyncClient`, one per proxy URL, kept for the run; impersonates Firefox by default; HTTP/3 off. Redirects are followed in Python, one hop at a time, to attach Session cookies. HTTP/2 negotiation inside `impit`: **unverified**. |
| 3. Frontier and duplicate check | Request queue v2 (`RequestQueue`) over `MemoryStorage`: an insertion-ordered `Map` of all requests in memory, and, with `persistStorage` (default on), one JSON file per request. In front of it: a 2 M-entry LRU of known requests, a 1 M-slot FNV-hashed dedup array, and a 1,000-entry "recently handled" LRU, so a known link costs two SHA-256 hashes and a map lookup and never reaches storage. Head: up to 25 requests listed and locked at once into a local head, refilled when ≤ 1 remain. **Writes per page with persistence:** lock = read + write of the request file; handled = one more write; plus one write per new link. Each write is wrapped in a `proper-lockfile` lock (`mkdir`, `stat`, `rmdir`) and goes to the OS cache with no `fsync`. One shared queue; no per-domain queues. Master: storage is a native Rust module (`@crawlee/fs-storage-native`), and per-domain queues exist in the opt-in `ThrottlingRequestManager`. | `FileSystemStorageClient` by default: in-memory state (`handled_requests` set, `in_progress_requests` set, ordered pending dicts with sequence numbers) plus one JSON file per request and a metadata file, written atomically (temp file and `rename`, no `fsync`). A cache of up to 100,000 pending requests is rebuilt from files when it runs out. **Writes per page:** "handled" = `exists` check + request file write + metadata write (two atomic writes, six thread-pool hops); each new link = one file write; each add batch = one metadata write. The whole state (including every handled key) is dumped to the key-value store every 60 s. Per-domain queues only in the opt-in `ThrottlingRequestManager`. |
| 4. Politeness | None by default. `sameDomainDelaySecs` (default 0) is checked per page with one `Map` lookup by domain; a request that is too early is put back with a timer (`setTimeout` → `reclaimRequest`), after having taken a slot. `maxRequestsPerMinute` is pool-wide, not per domain. robots.txt off by default; when on, cached per origin (1,000-entry LRU). Master: `sameDomainDelaySecs > 0` builds a `ThrottlingRequestManager` (per-domain queues, crawl delay, 429 back-off, robots.txt `Crawl-delay`) that returns `null` instead of holding a slot. | None by default. `ThrottlingRequestManager` (opt-in, explicit domains) adds per-domain sub-queues, 429 back-off and robots.txt crawl delay. No per-domain concurrency cap anywhere. |
| 5. Parsing and link extraction | Body buffered whole, then parsed into trees, eagerly, before the handler: **twice** in v3.18.2 (`htmlparser2.parseDocument` for `dom`, then `cheerio.load(body)` again for `$`). Master parses once (`cheerio.load(dom)`). Links are extracted only when the handler calls `enqueueLinks`, with a Cheerio selector (`a` by default), default strategy same hostname. Browser: two `$$eval` round trips (`a[href]`, `base`). | Body parsed into a tree before the handler, in a worker thread (`lxml` via BeautifulSoup by default, or Parsel). Links on `enqueue_links`. Browser: `query_selector_all('a')` then **one `get_attribute` round trip per link**, plus one `evaluate('document.baseURI')`. |
| 6. Main-text extraction | None in the crawl path. `htmlToText` exists as a utility the user can call. | None in the crawl path (an HTML-to-text helper exists in `_utils/html.py`). |
| 7. What is written per page | Nothing unless the handler calls `pushData`; each item then becomes one JSON file (lock file around it), awaited by the handler. Statistics and SessionPool state are persisted every 60 s. | Same model: `push_data` is buffered and committed after the handler; one JSON file per item plus a metadata write. Statistics persisted every 60 s. |
| 8. Browser | One `BrowserPool`; browser launched when no running one has a free slot (20 pages per browser). **One persistent context per browser** (`useIncognitoPages: false`), so one context and one Session per browser. A new page (target) per request, closed after. Browser retired after 100 pages, after 10 s with no page, or when its Session is retired; retired browsers close when empty. **No memory-based recycling**: memory only throttles concurrency. Session pool: up to 1,000 Sessions; a new Session per `getSession` call until full, then a random one; Session expires after 50 uses or 3,000 s. Fingerprint: generated per Session (cached), injected once per context (`addInitScript` of a ~27 KB utility script plus the fingerprint, and `setExtraHTTPHeaders`). Per page: new page, `emulateMedia` (dark), `addCookies`, `goto` (`load`), `cookies()`, two `$$eval` for links, `close`. | One `BrowserPool`; 20 pages per browser; one shared context per browser (incognito off), created lazily with the fingerprint (`browserforge`) and generated headers. Retired after 100 pages; inactive browsers found every 20 s and closed every 30 s. No memory-based recycling. Sessions are **not** mapped to contexts: all Sessions share the browser's single context, and each page copies the Session's cookies into it before `goto` and back after. Session pool fills to 1,000 at first use, then picks at random. Per page: new page, `add_cookies`, `goto`, `cookies()`, up to 3 `query_selector` (blocked check, on by default), N+2 round trips for links, close. |
| 9. Memory per URL | Every request ever added stays in the queue's `Map` (its JSON in memory without persistence; a stub after 60 s idle with persistence), plus up to 2 M LRU entries and the 1 M-slot dedup arrays (allocated up front). Dataset items keep an entry each. No spill beyond the per-file storage; no limit on the number of requests other than `maxRequestsPerCrawl`. | `handled_requests` (one key per URL), pending dicts, a cache of up to 100,000 `Request` objects; with the memory storage client, a full `Request` per handled URL. Persisted state grows with every handled URL and is dumped whole every 60 s. |
| 10. Progress and statistics | Per page: `startJob`/`finishJob` (a `Map` insert and delete, a few counters). Status line every 10 s, statistics log every 60 s, persistence every 60 s. | Per page: the same counters (`record_request_processing_start/finish`). Status message every 10 s, statistics log every 60 s, persistence every 60 s. |
| 11. Adopt / lose | See [section 11](#11-what-foxhound-could-adopt-and-what-would-be-a-loss). | |

## 1. Tasks and hand-offs; how concurrency is set

### Crawlee JS v3.18.2

**One page, in order** (all on the main event loop):

1. `AutoscaledPool._maybeRunTask` starts a task if the pool is running, no other start is being decided, `current < desired`, the system is not overloaded (or `current < minConcurrency`), `isTaskReadyFunction` says yes, and the per-minute cap is not reached. It then schedules itself again with `setImmediate`, so concurrency builds one task per tick ([`autoscaled_pool.ts:486-590`](https://github.com/apify/crawlee/blob/v3.18.2/packages/core/src/autoscaling/autoscaled_pool.ts#L486-L590)).
2. `BasicCrawler._runTaskFunction` ([`basic-crawler.ts:1601-1747`](https://github.com/apify/crawlee/blob/v3.18.2/packages/basic-crawler/src/internals/basic-crawler.ts#L1601-L1747)):
   - `fetchNextRequest()` from the queue (`#L1605`), with a timeout and retries;
   - `sessionPool.getSession()` for **every** request (`#L1613-L1619`), even in the browser crawler, which then replaces it with the browser's Session;
   - `delayRequest()` for `sameDomainDelaySecs` (`#L1555-L1597`), robots.txt check if enabled (`#L1625`);
   - `stats.startJob`, build the crawling context, run the handler under a timeout (`#L1680`), `markRequestHandled` (`#L1685`), `stats.finishJob`, `session.markGood()`;
   - `_cleanupContext` (browser: `page.close()`), then a safety-net `deleteRequestLock`.
3. The HTTP crawler's handler wrapper fetches and parses ([`http-crawler.ts:511-600`](https://github.com/apify/crawlee/blob/v3.18.2/packages/http-crawler/src/internals/http-crawler.ts#L511-L600)); the browser crawler's opens a page, navigates, saves cookies ([`browser-crawler.ts:520-610`](https://github.com/apify/crawlee/blob/v3.18.2/packages/browser-crawler/src/internals/browser-crawler.ts#L520-L610)).

**Locks and queues a page meets:**

- `SessionPool.getSession` waits on an `AsyncQueue` (a mutex) shared by all tasks ([`session_pool.ts:305-331`](https://github.com/apify/crawlee/blob/v3.18.2/packages/core/src/session_pool/session_pool.ts#L305-L331)).
- `BrowserPool.newPage` runs inside `pLimit(1)`: page creation, and a browser launch when one is needed, are serialised across the whole pool ([`browser-pool.ts:339`](https://github.com/apify/crawlee/blob/v3.18.2/packages/browser-pool/src/browser-pool.ts#L339), [`440-475`](https://github.com/apify/crawlee/blob/v3.18.2/packages/browser-pool/src/browser-pool.ts#L440-L475)).
- `MemoryStorage`'s `listAndLockHead` holds the queue mutex while it scans ([`request-queue.ts:228-298`](https://github.com/apify/crawlee/blob/v3.18.2/packages/memory-storage/src/resource-clients/request-queue.ts#L228-L298)); each request file has its own `AsyncQueue` plus an on-disk lock (see section 3).

**Concurrency rules** (all defaults):

- Pool: `maxConcurrency` 200, `minConcurrency` 1, desired starts at `minConcurrency`, `desiredConcurrencyRatio` 0.9, steps of 5% up or down, autoscale every 10 s, `maybeRunInterval` 0.5 s ([`autoscaled_pool.ts:245-263`](https://github.com/apify/crawlee/blob/v3.18.2/packages/core/src/autoscaling/autoscaled_pool.ts#L245-L263), [`284`](https://github.com/apify/crawlee/blob/v3.18.2/packages/core/src/autoscaling/autoscaled_pool.ts#L284)). Scale up only when the last 30 s were not overloaded and current ≥ floor(0.9 × desired); scale down whenever overloaded (`#L597-L640`, step `#L651-L676`). Starting at 1, `ceil(5%)` is 1, so reaching 16 takes about 150 s unless the user sets `minConcurrency` or `desiredConcurrency`.
- Signals ([`system_status.ts:167-171`](https://github.com/apify/crawlee/blob/v3.18.2/packages/core/src/autoscaling/system_status.ts#L167-L171), [`snapshotter.ts:154-159`](https://github.com/apify/crawlee/blob/v3.18.2/packages/core/src/autoscaling/snapshotter.ts#L154-L159), [`configuration.ts:292-299`](https://github.com/apify/crawlee/blob/v3.18.2/packages/core/src/configuration.ts#L292-L299)):
  - CPU: overloaded when machine CPU use > `maxUsedCpuRatio` 0.95 in more than 40% of the samples of the last 5 s;
  - memory: own process plus children above 0.9 × 25% of total RAM (`availableMemoryRatio` 0.25) in more than 20% of samples. Children are measured with `ps-tree`, which spawns `ps` once a second ([`memory-info.ts:78-92`](https://github.com/apify/crawlee/blob/v3.18.2/packages/utils/src/internals/memory-info.ts#L78-L92); interval `systemInfoIntervalMillis` 1,000);
  - event loop: a sample every 0.5 s, blocked > 50 ms, in more than 60% of samples;
  - storage client: errors.

**Master (v4):** the autoscaling moved to `packages/basic-crawler/src/internals/autoscaling/`, and a `ConcurrencySystem` now owns the budget with the same defaults (max 200, min 1, ratio 0.9; [`concurrency-system.ts:19-22` (master)](https://github.com/apify/crawlee/blob/9b310c77d23ab01bc56f3c28351612529d82da21/packages/basic-crawler/src/internals/autoscaling/concurrency-system.ts#L19-L22), [`280`](https://github.com/apify/crawlee/blob/9b310c77d23ab01bc56f3c28351612529d82da21/packages/basic-crawler/src/internals/autoscaling/concurrency-system.ts#L280)). Not read further.

### Crawlee Python 1.11.1

- **Pool**: `ConcurrencySettings(min_concurrency=1, max_concurrency=100, desired_concurrency=10)` ([`_types.py:106-134`](https://github.com/apify/crawlee-python/blob/v1.11.1/src/crawlee/_types.py#L106-L134)). Same 10 s, 5%, 0.9 rules ([`autoscaled_pool.py:47-62`](https://github.com/apify/crawlee-python/blob/v1.11.1/src/crawlee/_autoscaling/autoscaled_pool.py#L47-L62), [`189-203`](https://github.com/apify/crawlee-python/blob/v1.11.1/src/crawlee/_autoscaling/autoscaled_pool.py#L189-L203)). Thresholds: CPU 0.4, memory 0.2, event loop 0.6, client 0.3 of a 5 s window ([`system_status.py:43-47`](https://github.com/apify/crawlee-python/blob/v1.11.1/src/crawlee/_autoscaling/system_status.py#L43-L47)); `max_used_cpu_ratio` 0.95, `available_memory_ratio` 0.25, `max_event_loop_delay` 50 ms, `system_info_interval` 1 s ([`configuration.py:90-188`](https://github.com/apify/crawlee-python/blob/v1.11.1/src/crawlee/configuration.py#L90-L188)).
- **Orchestrator** ([`autoscaled_pool.py:214-245`](https://github.com/apify/crawlee-python/blob/v1.11.1/src/crawlee/_autoscaling/autoscaled_pool.py#L214-L245)): a loop that calls `is_finished_function()` and then `is_task_ready_function()` (the queue's `is_empty`) **before each task it spawns**, then `asyncio.create_task(worker)`; it waits up to 0.5 s for a task to finish when it cannot spawn.
- **One page** ([`_basic_crawler.py:1482-1560`](https://github.com/apify/crawlee-python/blob/v1.11.1/src/crawlee/crawlers/_basic/_basic_crawler.py#L1482-L1560)): `fetch_next_request` → robots.txt check → `_get_session()` → proxy → build context → `record_request_processing_start` → run the context pipeline and the handler → `_commit_request_handler_result` (`#L1419-L1430`: replays the buffered `add_requests` and `push_data` calls) → `_mark_request_as_handled` → `session.mark_good()` → `record_request_processing_finish`.
- **HTTP pipeline** ([`_abstract_http_crawler.py:121-129`](https://github.com/apify/crawlee-python/blob/v1.11.1/src/crawlee/crawlers/_abstract_http/_abstract_http_crawler.py#L121-L129)): navigation timeout → pre-navigation hooks → HTTP request → post-navigation hooks → status check → parse (thread pool) → blocked check (`retry_on_blocked` defaults to **True** in Python, false in JS; [`_basic_crawler.py:296`](https://github.com/apify/crawlee-python/blob/v1.11.1/src/crawlee/crawlers/_basic/_basic_crawler.py#L296), [`basic-crawler.ts:661`](https://github.com/apify/crawlee/blob/v3.18.2/packages/basic-crawler/src/internals/basic-crawler.ts#L661)).
- **Browser pipeline** ([`_playwright_crawler.py:208-214`](https://github.com/apify/crawlee-python/blob/v1.11.1/src/crawlee/crawlers/_playwright/_playwright_crawler.py#L208-L214)): timeout → open page → navigate → post hooks → status → blocked check → context.
- **Thread-pool hops**: `json_dumps` runs `json.dumps` in a thread ([`file.py:124-133`](https://github.com/apify/crawlee-python/blob/v1.11.1/src/crawlee/_utils/file.py#L124-L133)) and `atomic_write` runs the write in a thread (`#L39-L62`, `#L176`); parsers run in a thread ([`_beautifulsoup_parser.py:27-30`](https://github.com/apify/crawlee-python/blob/v1.11.1/src/crawlee/crawlers/_beautifulsoup/_beautifulsoup_parser.py#L27-L30), [`_parsel_parser.py:24-27`](https://github.com/apify/crawlee-python/blob/v1.11.1/src/crawlee/crawlers/_parsel/_parsel_parser.py#L24-L27)).

## 2. Fetch client, connection pooling, HTTP/2

### JS v3.18.2: `got-scraping`

- Default client: `new GotScrapingHttpClient()` ([`basic-crawler.ts:701`](https://github.com/apify/crawlee/blob/v3.18.2/packages/basic-crawler/src/internals/basic-crawler.ts#L701)); the HTTP crawler streams the response through it ([`http-crawler.ts:935-955`](https://github.com/apify/crawlee/blob/v3.18.2/packages/http-crawler/src/internals/http-crawler.ts#L935-L955)).
- `got-scraping` defaults (`got-scraping@4.2.2`, `dist/index.js:1071-1102`): `http2: true`, `rejectUnauthorized: false`, no retries, 60 s timeout, `useHeaderGenerator: true`, `insecureHTTPParser: true`, agents `TransformHeadersAgent(http.globalAgent)` and `TransformHeadersAgent(https.globalAgent)`. The crawler also defaults `ignoreSslErrors` to true (`http-crawler.ts#L389`).
- **HTTP/2 and ALPN**: for `https:` URLs, the header hook first resolves the protocol by ALPN (`h2` or `http/1.1`), with `http2-wrapper`'s process-wide cache when there is no proxy, and a cache **per Session** when there is one (`dist/index.js:504-535`, `550-560`, `563-612`). With a per-Session cache, each new Session pays one extra ALPN probe connection per origin.
- **Pooling without a proxy**: Node's global agents, so one connection pool per process. Whether they keep connections alive depends on the Node version (`http.globalAgent` defaults to keep-alive from Node 19, per Node's docs): **unverified** for a given deployment.
- **Pooling with a proxy**: a new agent per request with `maxFreeSockets: 0` and the comment "The sockets won't be reused" (`dist/index.js:398-450`), so every request opens a new connection to the proxy.
- **Headers**: two header sets (HTTP/1 and HTTP/2) generated per Session and kept on it (`dist/index.js:574-603`). With the default 1,000-Session pool and a new Session per request until it is full (section 8), the first 1,000 requests each generate headers.
- **Body**: the HTTP crawler aborts non-HTML MIME types before downloading the body, then buffers the whole body (`http-crawler.ts#L748-L800`, `#L914-L931`).

### JS master (v4)

`LazyDefaultHttpClient` imports `@crawlee/impit-client` and falls back to native `fetch` when it is missing ([`basic-crawler.ts:119-135` (master)](https://github.com/apify/crawlee/blob/9b310c77d23ab01bc56f3c28351612529d82da21/packages/basic-crawler/src/internals/basic-crawler.ts#L119-L135), [`1143`](https://github.com/apify/crawlee/blob/9b310c77d23ab01bc56f3c28351612529d82da21/packages/basic-crawler/src/internals/basic-crawler.ts#L1143)). `got-scraping` moved to its own optional package (`packages/got-scraping-client`).

### Python 1.11.1: `impit`

- Default `ImpitHttpClient()` ([`_basic_crawler.py:410`](https://github.com/apify/crawlee-python/blob/v1.11.1/src/crawlee/crawlers/_basic/_basic_crawler.py#L410)), `browser='firefox'`, `http3=False`, cookies per Session ([`_impit.py:133-163`](https://github.com/apify/crawlee-python/blob/v1.11.1/src/crawlee/http_clients/_impit.py#L133-L163)).
- One `impit.AsyncClient` per proxy URL, reused for the run (`#L367-L389`), so pooling is per proxy, inside `impit`.
- Redirects are followed by Crawlee, one request per hop, so the Session's cookies apply per hop (`#L286-L340`).
- HTTP/2 inside `impit`, and its connection pool size: **unverified** (the Rust source was not read).

## 3. Frontier and duplicate check

### JS v3.18.2

**Structure** (front to back):

1. `RequestProvider.addRequests` ([`request_provider.ts:291-405`](https://github.com/apify/crawlee/blob/v3.18.2/packages/core/src/storages/request_provider.ts#L291-L405)):
   - builds a `Request` per link (URL normalisation in `Request.computeUniqueKey`, [`request.ts:204`](https://github.com/apify/crawlee/blob/v3.18.2/packages/core/src/request.ts#L204), [`418`](https://github.com/apify/crawlee/blob/v3.18.2/packages/core/src/request.ts#L418));
   - computes `getRequestId(uniqueKey)`, a SHA-256 cut to 15 characters ([`utils.ts:117-121`](https://github.com/apify/crawlee/blob/v3.18.2/packages/core/src/storages/utils.ts#L117-L121));
   - looks it up in `requestCache` (LRU, 2,000,000 entries for v2, [`request_queue_v2.ts:17`](https://github.com/apify/crawlee/blob/v3.18.2/packages/core/src/storages/request_queue_v2.ts#L17), [`76`](https://github.com/apify/crawlee/blob/v3.18.2/packages/core/src/storages/request_queue_v2.ts#L76)) and `requestSeenCache` (`RequestDeduplicationCache`: two preallocated 1,000,000-slot arrays indexed by an FNV-1a hash, overwrite on collision; [`request_dedup_cache.ts:14-50`](https://github.com/apify/crawlee/blob/v3.18.2/packages/core/src/storages/request_dedup_cache.ts#L14-L50));
   - sends only unknown links to the storage client in one `batchAddRequests` call, and pushes new ones straight into the local head when it is short (`_maybeAddRequestToQueueHead`, `#L787`).
2. `addRequestsBatched` (`#L415`): the first 1,000 at once, the rest in the background, 1,000 a second.
3. `MemoryStorage` request-queue client ([`request-queue.ts`](https://github.com/apify/crawlee/blob/v3.18.2/packages/memory-storage/src/resource-clients/request-queue.ts)):
   - an insertion-ordered `Map` of every request (`#L64`); `orderNo` is the add timestamp (negative for forefront), and a lock is encoded as a future `orderNo` (`#L228-L298`, `#L641-L667`);
   - the ID is a second SHA-256 of the unique key ([`utils.ts:47-54`](https://github.com/apify/crawlee/blob/v3.18.2/packages/memory-storage/src/utils.ts#L47-L54)); a duplicate found here returns `wasAlreadyPresent` (`#L384-L398`).
4. Head handling in `RequestQueue` v2: `listAndLockHead` with limit 25 and a lock of `max(handler timeout + 5 s, 60 s)`, refilled when ≤ 1 request is left locally ([`request_queue_v2.ts:24-28`](https://github.com/apify/crawlee/blob/v3.18.2/packages/core/src/storages/request_queue_v2.ts#L24-L28), [`309-420`](https://github.com/apify/crawlee/blob/v3.18.2/packages/core/src/storages/request_queue_v2.ts#L309-L420); lock duration [`basic-crawler.ts:769`](https://github.com/apify/crawlee/blob/v3.18.2/packages/basic-crawler/src/internals/basic-crawler.ts#L769)).

**Scan cost:** `listAndLockHead` iterates the `Map` from the start, skipping handled entries (`orderNo === null`) without touching disk (`request-queue.ts#L154-L162`, `#L252-L270`). Each refill of 25 therefore walks every handled request first: O(handled) per refill, O(n²/25) over a crawl. This is inferred from the code; not measured.

**Memory and disk:** with `persistStorage` (default true, `CRAWLEE_PERSIST_STORAGE`; [`memory-storage.ts:74-83`](https://github.com/apify/crawlee/blob/v3.18.2/packages/memory-storage/src/memory-storage.ts#L74-L83)), each entry is a `RequestQueueFileSystemEntry` whose data is dropped from memory after 60 s idle ([`fs.ts:12-90`](https://github.com/apify/crawlee/blob/v3.18.2/packages/memory-storage/src/fs/request-queue/fs.ts#L12-L90)). Without it, the full JSON stays in memory.

**Writes per page** (persistence on), from the code above:

| Step | Disk operations |
|---|---|
| Lock in the head (`listAndLockHead`) | read the request file (forced, `get(true)`), write it back with the new `orderNo` |
| Mark handled (`updateRequest`) | write the request file |
| Each new link (`batchAddRequests` → `update`) | write one new file |
| Each mutation (`updateTimestamps`) | schedule a metadata task; it writes `__metadata__.json` only when `writeMetadata` is on (off unless `DEBUG` matches) ([`request-queue.ts:612-631`](https://github.com/apify/crawlee/blob/v3.18.2/packages/memory-storage/src/resource-clients/request-queue.ts#L612-L631), [`fs-utils.ts:29-42`](https://github.com/apify/crawlee/blob/v3.18.2/packages/memory-storage/src/background-handler/fs-utils.ts#L29-L42)). It still allocates a UUID and a promise per mutation. |

Every read or write is wrapped by `lockAndCallback`, which takes a `proper-lockfile` lock ([`fs-utils.ts:44-90`](https://github.com/apify/crawlee/blob/v3.18.2/packages/memory-storage/src/background-handler/fs-utils.ts#L44-L90)). In `proper-lockfile@4.1.2`, a lock is an `mkdir` of `<file>.lock`, an mtime probe (`stat`), and an `rmdir` on release (`lib/lockfile.js:26-50`, `90`). The write is `fs.writeFile` of pretty-printed JSON (`JSON.stringify(data, null, '\t')`), with no `fsync`.

**Per-domain queues:** none in v3.18.2.

**Master (v4):** with `persistStorage`, the backend is `FileSystemStorageBackend`, which forwards every queue operation to the native Rust module `@crawlee/fs-storage-native` ("Request ordering, in-progress locking and state persistence are all owned by the native client"; [`service-locator.ts:249-256` (master)](https://github.com/apify/crawlee/blob/9b310c77d23ab01bc56f3c28351612529d82da21/packages/core/src/service-locator.ts#L249-L256), [`request-queue.ts:1-60` (master)](https://github.com/apify/crawlee/blob/9b310c77d23ab01bc56f3c28351612529d82da21/packages/fs-storage/src/resource-clients/request-queue.ts#L1-L60)). Requests are round-tripped through `JSON.parse(JSON.stringify())` before crossing into it (`#L20-L22`). Its write policy: **unverified** (the native source was not read).

### Python 1.11.1

- **Default storage**: `FileSystemStorageClient()` ([`_service_locator.py:102`](https://github.com/apify/crawlee-python/blob/v1.11.1/src/crawlee/_service_locator.py#L102)).
- **State** (`RequestQueueState`, [`_request_queue_client.py:54-57`](https://github.com/apify/crawlee-python/blob/v1.11.1/src/crawlee/storage_clients/_file_system/_request_queue_client.py#L54-L57)): `handled_requests` set, `in_progress_requests` set, `regular_requests` and `forefront_requests` dicts of sequence numbers. It is a `RecoverableState` persisted to the key-value store on each persist-state event (`#L137-L150`; [`recoverable_state.py:175-196`](https://github.com/apify/crawlee-python/blob/v1.11.1/src/crawlee/_utils/recoverable_state.py#L175-L196)), every 60 s by default. That dumps the whole handled set each time.
- **Duplicate check** (`add_batch_of_requests`, `#L321-L449`): set and dict membership on the unique key under the queue's `asyncio.Lock`.
- **Cache**: a `deque` of up to 100,000 pending requests (`#L87`), rebuilt by sorting the pending keys and reading their files when empty or invalidated (`#L712-L760`).
- **Writes per page**:

| Step | Disk operations |
|---|---|
| `fetch_next_request` (`#L470-L493`) | none (memory only) |
| `mark_request_as_handled` (`#L495-L540`) | `exists` check, request file write, metadata file write (each write = `json_dumps` in a thread + `mkstemp`/write/`rename` in a thread; metadata also does a `mkdir` in a thread: about six thread-pool hops) |
| Each new link | one request file write (two hops) |
| Each add batch | one metadata write |

`atomic_write` is temp file plus `Path.replace`, no `fsync` ([`file.py:39-62`](https://github.com/apify/crawlee-python/blob/v1.11.1/src/crawlee/_utils/file.py#L39-L62)).

- **Memory client** (`MemoryStorageClient`): dicts of full `Request` objects, including every handled one ([`_request_queue_client.py:54-57`](https://github.com/apify/crawlee-python/blob/v1.11.1/src/crawlee/storage_clients/_memory/_request_queue_client.py#L54-L57)).
- **Per-domain queues**: only through `ThrottlingRequestManager` (section 4).

## 4. Politeness and rate limiting

### JS v3.18.2

- Defaults: `sameDomainDelaySecs` 0 ([`basic-crawler.ts:647`](https://github.com/apify/crawlee/blob/v3.18.2/packages/basic-crawler/src/internals/basic-crawler.ts#L647), [`774`](https://github.com/apify/crawlee/blob/v3.18.2/packages/basic-crawler/src/internals/basic-crawler.ts#L774)), `respectRobotsTxtFile` false (`#L662`), `maxRequestsPerMinute` unset (pool-wide cap, `#L825`).
- **Cost per page when off:** `delayRequest` computes `getDomain(url)` and does one `Map` get and set (`#L1561-L1572`); nothing else.
- **When on:** a request that comes too early has already been fetched and has already taken a pool slot; it is reclaimed after a `setTimeout` (`#L1578-L1597`). The slot is released only when the task returns, so a crawl of one domain with a long delay spins through fetch and reclaim.
- No per-domain concurrency cap exists.

### JS master (v4)

`sameDomainDelaySecs > 0` wraps the queue in a `ThrottlingRequestManager` with `domains: 'all'`, `throttleBy: 'registrableDomain'` ([`basic-crawler.ts:1105-1131` (master)](https://github.com/apify/crawlee/blob/9b310c77d23ab01bc56f3c28351612529d82da21/packages/basic-crawler/src/internals/basic-crawler.ts#L1105-L1131)). It routes throttled domains into their own queues, serves the domain that has waited longest, applies two clocks (429 back-off honouring `Retry-After`; crawl delay floored by the setting and raised by robots.txt `Crawl-delay`), and returns `null` instead of blocking, so no slot is held ([`throttling-request-manager.ts:230-300` (master)](https://github.com/apify/crawlee/blob/9b310c77d23ab01bc56f3c28351612529d82da21/packages/basic-crawler/src/internals/throttling-request-manager.ts#L230-L300)). With the delay at 0 nothing is built, so the default path has no politeness code at all.

### Python 1.11.1

- No politeness by default.
- `ThrottlingRequestManager` (opt-in, explicit domain list): per-domain sub-managers, 429 back-off, robots.txt crawl delay; `fetch_next_request` returns `None` rather than wait ([`_throttling_request_manager.py:1-80`](https://github.com/apify/crawlee-python/blob/v1.11.1/src/crawlee/request_loaders/_throttling_request_manager.py#L1-L80)).
- The crawler warns that `respect_robots_txt_file` without it does not enforce `Crawl-delay` ([`_basic_crawler.py:720-726`](https://github.com/apify/crawlee-python/blob/v1.11.1/src/crawlee/crawlers/_basic/_basic_crawler.py#L720-L726)).

## 5. Parsing and link extraction

### JS v3.18.2

- **HTTP**: `CheerioCrawler._parseHTML` reads the whole body into a string, then builds **two** trees: `parseDocument(body)` for `context.dom`, then `cheerio.load(body, { _useHtmlParser2: true })` for `context.$` ([`cheerio-crawler.ts:176-213`](https://github.com/apify/crawlee/blob/v3.18.2/packages/cheerio-crawler/src/internals/cheerio-crawler.ts#L176-L213)). Both are built for every HTML page before the handler runs, whether or not it uses them. `HttpCrawler` itself only buffers (`http-crawler.ts#L794-L800`); its `parseWithCheerio` builds a tree on demand (`#L529-L542`).
- **Links**: only when the handler calls `enqueueLinks`. `extractUrlsFromCheerio` selects `base` and `a` (default) and resolves each `href` with `new URL` ([`cheerio.ts:98-122`](https://github.com/apify/crawlee/blob/v3.18.2/packages/utils/src/internals/cheerio.ts#L98-L122)). Default strategy: same hostname ([`enqueue_links.ts:323`](https://github.com/apify/crawlee/blob/v3.18.2/packages/core/src/enqueue_links/enqueue_links.ts#L323)). Then globs, regexps, pseudo-URLs, `transformRequestFunction`, a `Request` per link, and `addRequests` (section 3).
- **Browser**: `extractUrlsFromPage` runs two `$$eval` calls, one for `a[href]` values and one for `base` ([`browser-crawler.ts:876-890`](https://github.com/apify/crawlee/blob/v3.18.2/packages/browser-crawler/src/internals/browser-crawler.ts#L876-L890)): two round trips per page.
- **Master**: one tree, `cheerio.load(dom)` reusing the `htmlparser2` document ([`cheerio-parser.ts:26-27` (master)](https://github.com/apify/crawlee/blob/9b310c77d23ab01bc56f3c28351612529d82da21/packages/cheerio-crawler/src/internals/cheerio-parser.ts#L26-L27)).

### Python 1.11.1

- **HTTP**: the parser builds a tree in a worker thread before the handler ([`_abstract_http_crawler.py:165-183`](https://github.com/apify/crawlee-python/blob/v1.11.1/src/crawlee/crawlers/_abstract_http/_abstract_http_crawler.py#L165-L183)): BeautifulSoup with `lxml` by default ([`_beautifulsoup_parser.py:23-58`](https://github.com/apify/crawlee-python/blob/v1.11.1/src/crawlee/crawlers/_beautifulsoup/_beautifulsoup_parser.py#L23-L58)), or a Parsel `Selector`. With `retry_on_blocked` on (the default), every page is checked against three CSS selectors (`#L338`; [`blocked.py:9-13`](https://github.com/apify/crawlee-python/blob/v1.11.1/src/crawlee/_utils/blocked.py#L9-L13)).
- **Browser**: `query_selector_all(selector)` returns element handles, then `await element.get_attribute('href')` **for each element**, and one `evaluate('document.baseURI')` ([`_playwright_crawler.py:447-457`](https://github.com/apify/crawlee-python/blob/v1.11.1/src/crawlee/crawlers/_playwright/_playwright_crawler.py#L447-L457)). A page with 200 links costs about 202 protocol round trips. The handles are not disposed before the page closes.
- With `retry_on_blocked`, the browser path also runs one `query_selector` per blocked selector, three per page (`#L527-L554`).

## 6. Main-text extraction or cleaning

Neither crawler extracts main text or cleans the page in the crawl path. JS has `htmlToText` ([`cheerio.ts:27`](https://github.com/apify/crawlee/blob/v3.18.2/packages/utils/src/internals/cheerio.ts#L27)), used by its `social` helpers; Python has helpers in `_utils/html.py`. Both are user-called. A Crawlee benchmark that writes "text" therefore runs whatever the user's handler does.

## 7. What is written per page

- **JS**: nothing unless the handler calls `pushData`. Each item becomes one JSON file through the same lock-file write, and the handler awaits it ([`dataset.ts:209-245`](https://github.com/apify/crawlee/blob/v3.18.2/packages/memory-storage/src/resource-clients/dataset.ts#L209-L245)). Statistics ([`statistics.ts:325`](https://github.com/apify/crawlee/blob/v3.18.2/packages/core/src/crawlers/statistics.ts#L325)) and the SessionPool (persistence on by default, [`session_pool.ts:186-187`](https://github.com/apify/crawlee/blob/v3.18.2/packages/core/src/session_pool/session_pool.ts#L186-L187), [`361`](https://github.com/apify/crawlee/blob/v3.18.2/packages/core/src/session_pool/session_pool.ts#L361)) are written on the 60 s persist-state event (`persistStateIntervalMillis`, [`configuration.ts:232`](https://github.com/apify/crawlee/blob/v3.18.2/packages/core/src/configuration.ts#L232)).
- **Python**: `push_data` is recorded and committed after the handler succeeds ([`_basic_crawler.py:1419-1430`](https://github.com/apify/crawlee-python/blob/v1.11.1/src/crawlee/crawlers/_basic/_basic_crawler.py#L1419-L1430)). The file-system dataset writes one file per item and the metadata under its lock ([`_dataset_client.py:225-235`](https://github.com/apify/crawlee-python/blob/v1.11.1/src/crawlee/storage_clients/_file_system/_dataset_client.py#L225-L235)). The SessionPool is not persisted by default (`persistence_enabled=False`, [`_session_pool.py:43`](https://github.com/apify/crawlee-python/blob/v1.11.1/src/crawlee/sessions/_session_pool.py#L43)).
- Request-queue writes per page: section 3.

## 8. Browser

### JS v3.18.2: BrowserPool with PlaywrightCrawler

**Defaults** ([`browser-pool.ts:368-380`](https://github.com/apify/crawlee/blob/v3.18.2/packages/browser-pool/src/browser-pool.ts#L368-L380)):

| Setting | Default |
|---|---|
| `maxOpenPagesPerBrowser` | 20 |
| `retireBrowserAfterPageCount` | 100 |
| `operationTimeoutSecs` | 15 |
| `closeInactiveBrowserAfterSecs` | 300 |
| `retireInactiveBrowserAfterSecs` | 10 |
| `useFingerprints` | true |

Master keeps the same values and adds `maxOpenBrowsers` (∞) ([`browser-pool.ts:32-45` (master)](https://github.com/apify/crawlee/blob/9b310c77d23ab01bc56f3c28351612529d82da21/packages/browser-pool/src/browser-pool.ts#L32-L45)).

**Browsers, contexts, pages:**

- `newPage` picks the first active browser with a free slot (same plugin and proxy) or launches one, inside `pLimit(1)` (`#L440-L475`, `#L764-L778`). At concurrency 16 this means **one browser**.
- `useIncognitoPages` defaults to false ([`launch-context.ts:109`](https://github.com/apify/crawlee/blob/v3.18.2/packages/browser-pool/src/launch-context.ts#L109)), so the Playwright plugin launches with `launchPersistentContext(userDataDir, …)` ([`playwright-plugin.ts:73-125`](https://github.com/apify/crawlee/blob/v3.18.2/packages/browser-pool/src/playwright/playwright-plugin.ts#L73-L125)): **one context per browser**, and every page is `browserContext.newPage()` ([`playwright-controller.ts:38-80`](https://github.com/apify/crawlee/blob/v3.18.2/packages/browser-pool/src/playwright/playwright-controller.ts#L38-L80)). With `userDataDir` empty, Playwright uses a temporary profile (per Playwright's docs; **unverified** here).
- On the first launch per plugin, the plugin also launches a second, unused browser to read its version (`playwright-plugin.ts#L170-L175`).
- Page reuse: none. A new page per request; `_cleanupContext` closes it ([`browser-crawler.ts:460-468`](https://github.com/apify/crawlee/blob/v3.18.2/packages/browser-crawler/src/internals/browser-crawler.ts#L460-L468)). The pool bounds `close` to 5 s and retires a browser whose page will not close ([`browser-pool.ts:28-29`](https://github.com/apify/crawlee/blob/v3.18.2/packages/browser-pool/src/browser-pool.ts#L28-L29), [`807-881`](https://github.com/apify/crawlee/blob/v3.18.2/packages/browser-pool/src/browser-pool.ts#L807-L881)).

**Recycling rules:**

- After 100 pages a browser is retired at page creation (`#L600-L602`); a retired browser takes no new pages and is closed once its pages close (`#L884-L893`), or by the killer every 10 s when idle for 300 s (`#L30`, `#L333-L336`, `#L782-L801`).
- An active browser with no page for 10 s is retired (`#L406-L417`).
- A browser is retired when its Session is retired ([`browser-crawler.ts:779-797`](https://github.com/apify/crawlee/blob/v3.18.2/packages/browser-crawler/src/internals/browser-crawler.ts#L779-L797)).
- **No memory rule** in BrowserPool: memory only lowers the AutoscaledPool's desired concurrency (section 1). A 1,000-page crawl of one site at concurrency 16 therefore launches about 10 browsers, one after another (from the 100-page rule; not measured).

**Sessions and contexts:**

- SessionPool defaults: `maxPoolSize` 1,000 ([`consts.ts`](https://github.com/apify/crawlee/blob/v3.18.2/packages/core/src/session_pool/consts.ts)); Session `maxUsageCount` 50, `maxAgeSecs` 3,000, `maxErrorScore` 3 ([`session.ts:170-182`](https://github.com/apify/crawlee/blob/v3.18.2/packages/core/src/session_pool/session.ts#L170-L182)).
- **Growth rule**: while the pool has fewer than 1,000 Sessions, `getSession()` creates a new one; once full, it picks one at random, and if that one is unusable it removes all retired Sessions and creates a new one ([`session_pool.ts:129-134`](https://github.com/apify/crawlee/blob/v3.18.2/packages/core/src/session_pool/session_pool.ts#L129-L134), [`305-331`](https://github.com/apify/crawlee/blob/v3.18.2/packages/core/src/session_pool/session_pool.ts#L305-L331), [`468-478`](https://github.com/apify/crawlee/blob/v3.18.2/packages/core/src/session_pool/session_pool.ts#L468-L478)).
- **Mapping**: in the browser crawler, a Session is bound to a **browser**, not to a context. `_extendLaunchContext` takes one Session at launch (`browser-crawler.ts#L751-L777`), and `_enhanceCrawlingContextWithPageInfo` replaces the request's Session with the browser's one unless incognito pages are on (`#L612-L631`). One browser = one persistent context = one Session = one fingerprint. The Session taken by `_runTaskFunction` for each request is discarded, but it still counts towards the 1,000.
- Cookies: before `goto`, `context.addCookies(Session cookies + request cookies)`; after it, `context.cookies()` saved into the Session (`browser-crawler.ts#L566-L574`, `#L682-L697`; [`playwright-controller.ts:149-192`](https://github.com/apify/crawlee/blob/v3.18.2/packages/browser-pool/src/playwright/playwright-controller.ts#L149-L192)).

**Fingerprints and init scripts:**

- Generated per Session or proxy URL and cached (`QuickLRU`, 10,000 entries) ([`hooks.ts:14-57`](https://github.com/apify/crawlee/blob/v3.18.2/packages/browser-pool/src/fingerprinting/hooks.ts#L14-L57), [`browser-pool.ts:896-917`](https://github.com/apify/crawlee/blob/v3.18.2/packages/browser-pool/src/browser-pool.ts#L896-L917)).
- Injected **once per context**: `attachFingerprintToPlaywright` runs `context.setExtraHTTPHeaders`, `context.addInitScript({ content })`, and registers `page.emulateMedia({ colorScheme: 'dark' })` on every new page (`fingerprint-injector@2.1.89`, `fingerprint-injector.js:57-77`); the hook marks the context so it is not done twice (`hooks.ts#L80-L107`). The injected content is the 27 KB `utils.js` plus the fingerprint JSON (size of `utils.js` read from the archive).
- How Playwright applies a context-level init script to each new page at the protocol level (likely one `Page.addScriptToEvaluateOnNewDocument` per target): **unverified**.

**Protocol work per page** (default options, read from Crawlee's side; Playwright's own commands per call are not counted):

1. `context.newPage()`, plus the dark-scheme `emulateMedia`;
2. `context.addCookies`;
3. `page.goto(url)` with Playwright's default `waitUntil: 'load'` ([`playwright-utils.ts:188-246`](https://github.com/apify/crawlee/blob/v3.18.2/packages/playwright-crawler/src/internals/utils/playwright-utils.ts#L188-L246); a `page.route` is added only for non-GET or payload requests, extra headers use `setExtraHTTPHeaders`);
4. `context.cookies()`;
5. two `$$eval` if the handler enqueues links;
6. `page.close()`.

With `retryOnBlocked` (off by default) add three `page.$` calls.

### Python 1.11.1

**Defaults:**

- `max_open_pages_per_browser` 20 ([`_playwright_browser_plugin.py:51`](https://github.com/apify/crawlee-python/blob/v1.11.1/src/crawlee/browsers/_playwright_browser_plugin.py#L51));
- `retire_browser_after_page_count` 100, `browser_inactive_threshold` 10 s, inactive check every 20 s, close every 30 s, `operation_timeout` 60 s ([`_browser_pool.py:52-56`](https://github.com/apify/crawlee-python/blob/v1.11.1/src/crawlee/browsers/_browser_pool.py#L52-L56));
- incognito off (`#L125`).

**Contexts:**

- One shared context per browser, created lazily under a lock on the first page, with the fingerprint (`browserforge`'s `AsyncNewContext`) or generated headers ([`_playwright_browser_controller.py:158-209`](https://github.com/apify/crawlee-python/blob/v1.11.1/src/crawlee/browsers/_playwright_browser_controller.py#L158-L209), [`283-307`](https://github.com/apify/crawlee-python/blob/v1.11.1/src/crawlee/browsers/_playwright_browser_controller.py#L283-L307)). A user data directory, when given, uses a persistent context (`_playwright_browser_plugin.py#L190-L193`).
- The default fingerprint generator is on (`fingerprint_generator='default'`, [`_playwright_crawler.py:122`](https://github.com/apify/crawlee-python/blob/v1.11.1/src/crawlee/crawlers/_playwright/_playwright_crawler.py#L122), [`184-191`](https://github.com/apify/crawlee-python/blob/v1.11.1/src/crawlee/crawlers/_playwright/_playwright_crawler.py#L184-L191)).
- Injection cost per page: **unverified** (`browserforge`'s injector was not read).

**Pages and launches:**

- `_get_new_page` picks a browser with free capacity or launches one, then creates the page; it retires the browser at 100 pages ([`_browser_pool.py:297-346`](https://github.com/apify/crawlee-python/blob/v1.11.1/src/crawlee/browsers/_browser_pool.py#L297-L346)).
- There is no pool-wide lock around the pick-or-launch step. Several pages requested at once with no free browser could each launch one: an inference from the code, not observed.

**Sessions:**

- The pool fills itself to `max_pool_size` 1,000 on first use and then picks at random (`random.choice(list(...))`, a 1,000-item list built per call) ([`_session_pool.py:39`](https://github.com/apify/crawlee-python/blob/v1.11.1/src/crawlee/sessions/_session_pool.py#L39), [`180-197`](https://github.com/apify/crawlee-python/blob/v1.11.1/src/crawlee/sessions/_session_pool.py#L180-L197), [`238-248`](https://github.com/apify/crawlee-python/blob/v1.11.1/src/crawlee/sessions/_session_pool.py#L238-L248)). Session `max_usage_count` 50, `max_age` 50 min, `max_error_score` 3 ([`_session.py:40-46`](https://github.com/apify/crawlee-python/blob/v1.11.1/src/crawlee/sessions/_session.py#L40-L46)).
- **Sessions do not map to contexts.** Every Session uses the browser's one shared context. Before `goto`, the Session's cookies are added to that context; after navigation, all the context's cookies are copied back into the current Session ([`_playwright_crawler.py:373-375`](https://github.com/apify/crawlee-python/blob/v1.11.1/src/crawlee/crawlers/_playwright/_playwright_crawler.py#L373-L375), [`565-595`](https://github.com/apify/crawlee-python/blob/v1.11.1/src/crawlee/crawlers/_playwright/_playwright_crawler.py#L565-L595)). Read literally, cookies set for one Session are visible to the other Sessions using the same browser, and are copied into them: an inference from the code, not tested.

**Protocol work per page**: new page, `add_cookies`, `goto` (Playwright's default `load`), `context.cookies()`, three `query_selector` (blocked check on by default), N + 2 round trips for N links, close.

## 9. Memory: what grows per URL

### JS v3.18.2

| Structure | Bound |
|---|---|
| Queue `Map` entry per request ever added (`request-queue.ts#L64`) | none; data dropped after 60 s idle when persisted |
| `requestCache` LRU (`request_queue_v2.ts#L17`) | 2,000,000 entries |
| `requestSeenCache` (two arrays) | 1,000,000 slots, allocated at queue creation |
| `recentlyHandledRequestsCache` | 1,000 |
| `domainAccessedTime` `Map` (`basic-crawler.ts#L554`) | one per domain, unbounded |
| robots.txt cache (`#L708`) | 1,000 |
| fingerprint cache | 10,000 |
| SessionPool | 1,000 Sessions with cookie jars |
| Dataset entry per item | none |

No spill beyond the per-request files. `maxRequestsPerCrawl` is the only crawl-size limit.

### Python 1.11.1

| Structure | Bound |
|---|---|
| `handled_requests` (one key per URL) | none; dumped whole every 60 s |
| pending dicts | the backlog |
| request cache | 100,000 `Request` objects |
| SessionPool | 1,000 Sessions, created up front |
| memory client: full `Request` per handled URL | none |

## 10. Progress and statistics

- **JS**: per page, `startJob` (a `Map` set plus a `Job` object) and `finishJob` (a few counters, a histogram slot, a `Map` delete) ([`statistics.ts:217-256`](https://github.com/apify/crawlee/blob/v3.18.2/packages/core/src/crawlers/statistics.ts#L217-L256)). The status line is built every 10 s by the crawler ([`basic-crawler.ts:904-970`](https://github.com/apify/crawlee/blob/v3.18.2/packages/basic-crawler/src/internals/basic-crawler.ts#L904-L970)); statistics are logged every 60 s (`statistics.ts#L123`) and persisted every 60 s. Nothing is serialised per page.
- **Python**: the same counters per page ([`_statistics.py:226-245`](https://github.com/apify/crawlee-python/blob/v1.11.1/src/crawlee/statistics/_statistics.py#L226-L245)), a 60 s log (`#L84`, `#L112`), and a 10 s status message ([`_basic_crawler.py:307`](https://github.com/apify/crawlee-python/blob/v1.11.1/src/crawlee/crawlers/_basic/_basic_crawler.py#L307), [`507`](https://github.com/apify/crawlee-python/blob/v1.11.1/src/crawlee/crawlers/_basic/_basic_crawler.py#L507)).

## 11. What Foxhound could adopt, and what would be a loss

Each item is framed as a setting with an off state, per the owner's rules. Cross-references point to the measured causes in the gap notes.

### Could adopt

1. **A seen-cache in front of the frontier store.** Crawlee checks every discovered link against in-memory caches (an LRU of known requests and a fixed-size hashed array) before any storage call. Links already known never reach storage (`request_provider.ts#L347-L365`). Most links on a page are already known, and Foxhound's R2 (frontier admission, 173 k instructions a page) and part of R3 run on the run file for them.
   - Setting: `frontier.seen_cache.entries` (0 = off). The store stays the authority, and a cache miss falls through to it, so no feature changes.
2. **Batch claim of the queue head.** Crawlee v2 lists and locks 25 requests at once and serves them locally, refilling at 1 (`request_queue_v2.ts#L24-L28`, `#L309-L420`). Foxhound's claim and lease run once per page on the single writer (R3, R8).
   - Setting: `frontier.claim_batch` (1 = today's behaviour).
   - Cost: up to N leased requests go back to the queue on a crash, the same at-least-once trade-off every crawler in the persistence survey accepts.
3. **No Politeness work when it cannot bind.** With no delay configured, Crawlee runs no politeness code at all (JS master builds the `ThrottlingRequestManager` only when the delay is above 0; v3 does one `Map` lookup). Foxhound's permit costs 373 k instructions a page on a single actor even when the cap equals the handler concurrency and the delay is 0 (R4).
   - Setting: `politeness.skip_when_non_binding` (default on; off = always consult).
   - The results are identical by construction: the permit is skipped only when no answer could differ.
4. **Throttled requests do not hold a slot.** Crawlee master's `ThrottlingRequestManager` returns "nothing ready, next due at T" instead of blocking a slot, and keeps per-domain queues. v3's "fetch, then reclaim after a timer" is the version to avoid. This applies if Foxhound's handler slots ever wait on a permit; the gap notes do not say that they do.
5. **Time-driven progress.** Crawlee updates O(1) counters per page and builds the status every 10 s and the log every 60 s. Foxhound serialises in-flight progress to JSON 32 times a page (R7).
   - Setting: `progress.interval` (0 = per event, today's behaviour).
6. **Session bound to a browser, one context per browser.** This is Crawlee JS's default: a Session is taken at browser launch, the browser has one persistent context, and the Session is rotated by retiring the browser (100 pages, Session retirement, idle). Foxhound's default of 100 Sessions = 100 contexts in 13 browsers is the browser gap's first cause.
   - Setting: `session.isolation: context | browser` (or `session.contexts_per_browser`).
   - With `browser`, cookie isolation between Sessions is kept (one Session per browser), but fewer Sessions are live at once: one per browser instead of one per context.
7. **Page-count recycling instead of a memory cap.** Crawlee retires a browser after `retireBrowserAfterPageCount` (100) pages and when idle, and uses memory only to slow the autoscaler (25% of RAM by default). Foxhound's "twice the warm-up" memory cap launched 43 browsers instead of 13 (browser gap, cause 2).
   - Settings: `browser.retire_after_pages` (0 = off) next to the memory cap, with the memory cap's threshold also settable and able to be turned off.
8. **Fingerprint and init scripts once per context, not per page.** Crawlee injects its fingerprint script (about 27 KB plus data) once per context and marks it injected (`hooks.ts#L80-L107`). Foxhound sends a 583 KB Consent bundle with `Page.addScriptToEvaluateOnNewDocument` on every page.
   - Adopt: register context-level scripts once per context where the protocol allows it, and keep the per-page payload small (load the full runner only when a consent banner is detected).
   - Setting: `consent.injection: eager | on_detect`. Whether Chrome needs the command per target anyway when scripts are registered per context is **unverified**.
9. **One round trip for links.** Crawlee JS reads all `href`s with one `$$eval` (plus one for `base`); Crawlee Python's per-element `get_attribute` is the anti-pattern (N + 2 round trips). This is a check for Foxhound's CDP count (57–60 commands a page): links should come back in a single `Runtime.evaluate`. Foxhound already reads links in the live page (ADR 0138); whether it does so in one call was not checked here.
10. **Buffered handler results, committed once.** Crawlee Python records `add_requests`, `push_data` and key-value changes during the handler and commits them in one step after success (`_basic_crawler.py#L1419-L1430`). That is the shape of a single per-page commit on Foxhound's writer (fewer SQLite transactions a page), with retries leaving no partial writes.

### Would be a loss

1. **Crawlee's durability model.** JS writes a file per request without `fsync`; Python writes atomically without `fsync` and snapshots the handled set every 60 s. Copying it would weaken Foxhound's `durability` promises. Only Foxhound's `durability: off` should behave like this.
2. **No Politeness by default.** Crawlee has no per-domain concurrency cap and no delay unless configured. Foxhound's default Politeness is a feature; only its cost when non-binding should go.
3. **Python's shared context for all Sessions.** It mixes Session cookies within a browser, which defeats Session isolation. Foxhound should not copy it.
4. **Eager tree parsing, twice in JS v3.** Foxhound's one streaming pass is already better; nothing to adopt.
5. **No main-text extraction.** Foxhound's extraction is a product feature that Crawlee does not have. A like-for-like comparison has to give Crawlee a handler that does the same work, or compare raw only.
6. **AutoscaledPool's slow start.** Starting at concurrency 1 and adding 1 every 10 s is slower than a fixed concurrency for short crawls. Autoscaling could be an opt-in setting (`concurrency.autoscale`, off = fixed), never the default for benchmarks.
7. **Pool-wide `pLimit(1)` page creation in JS.** It serialises page opens, and browser launches block every page open. Avoid it.
8. **got-scraping's no-reuse proxy agents and per-Session ALPN caches.** These cost a new connection per request through a proxy, and an extra probe connection per new Session. Avoid them.

## Unverified

- Exact dependency patch versions that a v3.18.2 install resolves (read: got-scraping 4.2.2, fingerprint-injector 2.1.89, proper-lockfile 4.1.2).
- Node's HTTP keep-alive default for the global agents in the Node version a user runs (Node docs say on from Node 19).
- Playwright internals: the temporary profile for an empty `userDataDir`, and how a context-level `addInitScript` reaches each new target at the CDP level.
- `impit`'s HTTP/2 negotiation and pool sizing (Python default and JS master default).
- `@crawlee/fs-storage-native`'s write policy (JS master).
- `browserforge`'s injection mechanism and per-page cost (Python).
- The O(handled) scan in JS `listAndLockHead`, the Python concurrent-launch race, and the Python cross-Session cookie mixing are read from the code, not observed.
- No timing or instruction count was measured for Crawlee; every per-page figure here is an operation count from the source.
