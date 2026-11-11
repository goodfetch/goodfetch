# Compiled-language crawlers: the work around one page (2026-11-06)

Research for Goodfetch issue #2159 (wayfinder research). This covers Spider 2.53.9 with Chromey 2.58.2 (Rust), Colly v2.3.0 (Go) and Katana v1.8.0 (Go), with Katana's standard, hybrid and headless engines.

**Versions and sources read**

- **Spider 2.53.9 and Chromey 2.58.2.** Read from the cargo registry: `~/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/spider-2.53.9/` and `chromey-2.58.2/`. The crates `spider_fingerprint-2.39.0`, `spider_transformations-2.39.13` and `sysinfo-0.38.4` were read from the same place. Paths below are relative to each crate root.
- **Colly v2.3.0.** Tag object `64dfa7d1`, commit `157009d952a88424e33afa396ef30ef700a0926e`, tagged 2025-12-04. It is the newest v2 tag. The newest GitHub *Release* entry is still v2.2.0 (2025-03-27). URLs use `https://github.com/gocolly/colly/blob/v2.3.0/<path>#L<n>`.
- **Katana v1.8.0.** Commit `35267ac5c8ff1db9694a319d0eb466ed97b0969f`, released 2026-10-05. URLs use `https://github.com/projectdiscovery/katana/blob/v1.8.0/<path>#L<n>`.
- **Libraries read at the versions Katana and Colly use:**
  - Go `net/http/transport.go` at `go1.24.0`, the version in Colly's `go.mod`. Katana's `go.mod` says 1.26.0, but the lines cited have the same logic there. That sameness is **unverified** at 1.26.
  - projectdiscovery `retryablehttp-go` v1.3.29.
  - projectdiscovery `hmap` v0.0.102.

All three were fetched through `gh api` at those tags.

The Spider gap report and the browser gap report (`research-spider-gap-20261106.md` and `research-browser-gap-20261106.md`, section 5 of each) already cover these, so this report does not repeat them:

- Spider's HTTP page path: reqwest, the `JoinSet` and semaphore, streaming lol_html, the in-memory `HashSet` that spills at 15,000 links.
- Spider's browser basics: one browser, one context, a new tab per page, `load` wait, and the content read with links taken in Rust.

For Spider, this report fills the gaps those two left.

Nothing was built or run. The counts are from reading source. Anything not read in source is marked **unverified**.

---

## 1. Summary table: the 11 points per crawler

| # | Spider 2.53.9 + Chromey 2.58.2 | Colly v2.3.0 | Katana v1.8.0 |
|---|---|---|---|
| 1 Tasks, hand-offs, concurrency | **HTTP:** one Tokio task per link in a `JoinSet` under a semaphore (see the Spider gap report). **Browser:** the same per-link task, plus one Chromey **Handler task per browser** that owns every target. Each command goes page → mpsc → Handler → WebSocket; each event goes Handler → listener channel. Tab close is handed to a dedicated `spider-tab-closer` thread. Default permits: `calc_limits(1)` = max(logical×physical, 30) without SMT, 20 with SMT. Under CPU load ≥70% or RSS pressure, the crawl switches to the process-wide `SEM_SHARED`. | **Async:** one goroutine per `Visit`, with **no global cap**. The only bound is a `LimitRule.Parallelism` channel per rule. **Sync:** the callback recursion. **`queue`:** N `Threads` goroutines plus one loop goroutine, with an unbuffered channel hand-off. | **Standard:** one goroutine per page under a `sizedwaitgroup(Concurrency=10)` per target, plus one extra goroutine per page for the rate-limit `Take`. Up to `Parallelism=10` targets at once, each with its own queue, client and pool. **Hybrid and headless:** **sequential**, one page at a time per target. |
| 2 Fetch client, pooling, HTTP/2 | reqwest, as in the Spider gap report. Browser: Chrome's own network stack, with no `Fetch` interception by default. | `http.Client{Timeout: 10s}` with `DefaultTransport`: HTTP/2 on (`ForceAttemptHTTP2`), `MaxIdleConns` 100, **`MaxIdleConnsPerHost` 2**. Over HTTP/1.1, more than 2 parallel requests to one host reconnect. | `retryablehttp` over a custom `http.Transport` built **per target**: `MaxIdleConnsPerHost` 10, `MaxConnsPerHost` 100, fastdialer, TLS verification off. Because it sets `TLSClientConfig` and dialers, **HTTP/2 is off**. The second client (h2) is used only after an "HTTP/1.x transport connection broken" error. |
| 3 Frontier and dedup | In memory (see the Spider gap report). Depth is the **number of URL path segments**, not link hops. Domain and subdomain checks and the asset-extension filter run **inside the lol_html link handler**, so off-scope links are never inserted. | Dedup: FNV-64a of the WHATWG-normalized URL (and the POST body) in a `map[uint64]bool` behind a RWMutex. It checks, then sets, with no atomic test-and-set. Frontier: none in Async mode (parked goroutines). In the `queue` package, a linked list of JSON-serialized requests capped at 100,000. | Dedup: hmap with **LevelDB on disk** (`DefaultDiskOptions`) in a temp directory, one `Get` and one `Put` per new URL, no fsync. Also an MD5 of every body as a content-dedup key. Frontier: an in-memory stack (depth-first, the default) or a priority queue, unbounded. |
| 4 Politeness | Off by default (see the Spider gap report). | `LimitRule`: a buffered channel of size `Parallelism` per *rule*, not per host. The delay is a `time.Sleep` **after** the response while still holding the slot. Each request scans the rules for a regex or glob match. robots.txt is off by default. | A global leaky bucket at **150 requests per second by default**, one `Take` per page. Optional per-host limiter and `-delay`. An exponential back-off per host after 429 or 503 (LRU of 10,000 hosts). robots.txt is not enforced. |
| 5 Parsing and links | lol_html streaming on chunks (see the Spider gap report). In the browser: the same handlers run over the streamed `outerHTML` chunks. | Whole body read (`io.ReadAll`, 10 MiB cap). Charset conversion copies the body if it is not UTF-8. **goquery builds a full DOM tree** when any `OnHTML` exists, then one `Find` per callback. The XML tree is a separate parse. | Whole body read (4 MiB cap). Full goquery DOM. **About 26 separate `Find` passes** (one per tag kind) plus header parsers and an endpoint regex over the whole body. |
| 6 Main text | Not in the crawl path. `spider_transformations` is a separate crate run by the caller: lol_html tag removal, then `llm_readability` (a full DOM), `clean_html`, then html2md or html2text. Each step builds a new HTML string. | None. | None. Optional: a page-type classifier (`dit`), Wappalyzer, simhash similarity. |
| 7 Written per page | Nothing on disk unless `balance` sees pressure; then the HTML is spooled to a temp file. The page goes on a tokio `broadcast`; the body is `bytes::Bytes`, so a clone is a reference-count bump. With `disk`, visited URLs go to SQLite only above 15,000 or under memory pressure. | Nothing; callbacks decide. The optional `CacheDir` writes a gob file per GET. | One result line (JSON or screen) per page under a global mutex to stdout and a `bufio` file. Flush and fsync happen only at close. Optional: one file per page plus an index line. LevelDB writes for the URL and content keys. |
| 8 Browser | **1 browser, 1 context, a new tab per page.** No recycling for memory: relaunch happens only when the browser dies. About **21 CDP commands per page** with the defaults. The stealth script is built per tab from `spider_fingerprint`, tens of KB (estimate). A `TabPool` exists but **nothing calls it**. | None. | **Hybrid:** 1 browser, 1 incognito context, a new tab per page. `Fetch` interception at the response stage on `*`: each subresource costs `getResponseBody`, a goquery parse and a link parse. Then `DOM.getDocument(depth=-1, pierce)`, `HTML()` and onclick clicks with 1 s waits. **Headless:** a pool of **1 browser per target with 1 reused tab**. Stealth (≈186 KB of Go source) and 20 KB of helpers are registered **once per tab**. |
| 9 Memory | Visited set in memory, spilled at 15,000 or under pressure. Bodies are dropped after the broadcast. One background sampler task every 200 ms; each page reads an atomic. Spool to disk when RSS ≥ 90% of RAM, or ≥ 95% for critical. Chromey evicts its stale network maps on a tick. | The visited map grows forever, about 1 entry per URL. **In Async mode, the frontier is goroutines plus closures (≥ 2 to 8 KiB of stack each), unbounded.** The queue is capped at 100,000 serialized requests. | The seen set is on disk (LevelDB). The queue is in memory and unbounded. Each page holds the body ×3 to 4 (bytes, two strings, the raw dump) until output. **The headless engine's crawl graph keeps every page state, including its full DOM and stripped DOM.** |
| 10 Progress | No per-page progress. `emit_log` is a `log` call; there are no counters. | Two atomic counters (`requestCount`, `responseCount`). | One atomic result counter, and a summary at the end. Resume state lists the unfinished *targets* only. |
| 11 For Foxhound | See [section 5](#5-what-foxhound-could-adopt). | | |

---

## 2. Spider 2.53.9 and Chromey 2.58.2: the gaps the Spider gap report left

### 2.1 Chromey: the Handler and one page's life

**One task per browser owns all targets.** Spider spawns one Tokio task per browser that loops `handler.next()` until the stream ends (`spider-2.53.9/src/features/chrome.rs:1068-1092`). `Handler`'s `Stream::poll_next` drains `from_browser` (the commands, `CreatePage`, `AddEventListener` and so on) under budgets: 128 browser messages, 128 per target, 512 WebSocket messages per poll (`chromey-2.58.2/src/handler/mod.rs:1235-1330`).

A per-session parallel handler exists behind the `parallel-handler` feature, documented as "Phase 2 minimum scope" (`chromey/src/handler/parallel/mod.rs:1-30`). Spider does not enable it: it uses Chromey with features `bytes` and `stream` only (`spider-2.53.9/Cargo.toml:625-632`).

So each page command crosses two hops: page → mpsc → Handler → WebSocket. Each event crosses the reverse: Handler → per-listener channel. `Browser::new_page` sends `HandlerMessage::CreatePage` and waits on a oneshot (`chromey/src/browser.rs:528-540`).

**Target init chain** (`chromey/src/handler/target.rs:556-665`) is a state machine. Each step is a `CommandChain` sent in order:

1. `Target.attachToTarget` (flatten) (`target.rs:563-579`).
2. `Page.enable`, `Page.getFrameTree`, `Page.setLifecycleEventsEnabled` (`handler/frame.rs:282-318`).
3. The isolated world: one `Page.addScriptToEvaluateOnNewDocument` whose source is only a `//# sourceURL=` line, and one `Page.createIsolatedWorld` per known frame (`frame.rs:687-720`).
4. `Network.enable`, plus `Security.setIgnoreCertificateErrors` when HTTPS errors are ignored (`handler/network.rs:196-219`, `:592-599`).
5. `Target.setAutoAttach` (`waitForDebuggerOnStart: true`) and `Log.disable`, plus `Performance.enable` under `collect_metrics` (`target.rs:67-99`).
6. Only when a viewport is set: `Emulation.setDeviceMetricsOverride` and `Emulation.setTouchEmulationEnabled` (`handler/emulation.rs:38-68`). Spider's default viewport is `None` unless `real_browser` is on (`spider/src/features/chrome.rs:827-831`).
7. `Runtime.runIfWaitingForDebugger` is sent only for auto-attached child targets that wait (`target.rs:489-498`).

`Fetch.enable` is not sent unless interception is configured (`network.rs:221-226`).

**Spider's per-page additions**, all sent concurrently with `tokio::join!` under a 15 s budget (`spider/src/features/chrome.rs:1588-1690`):

- `Page.addScriptToEvaluateOnNewDocument` with the merged stealth script. Stealth is **on by default**: `Configuration` derives `Default` (`configuration.rs`, the derive above `pub struct Configuration`), and `Tier`'s `#[default]` is `LowWithNavigator`, so `stealth()` is true (`spider_fingerprint-2.39.0/src/configs.rs:4-51`).
- `Network.setUserAgentOverride` and `Emulation.setUserAgentOverride`. `set_user_agent` sends both (`chromey/src/page.rs:1444-1457`). A random user agent is set by default (`configuration.rs:1063`).
- `Emulation.setHardwareConcurrencyOverride` when stealth is on (`chromey/src/page.rs:2476`).
- Optionally: `Page.setAdBlockingEnabled`, `Log.disable`, `Page.setBypassCSP`, `Emulation.setScriptExecutionDisabled`, and the timezone, locale and geolocation overrides (`chrome.rs:1366-1517`). All are off by default.
- `setup_chrome_interception_base` does nothing unless `chrome_intercept` is on (`chrome.rs:1574-1586`).

**Navigation and content:**

- `Page.navigate` through `page.http_future`, which waits for `load` (`spider/src/utils/mod.rs:1457-1490`).
- Event listeners are subscribed per page for `loadingFinished`, `loadingFailed` and `responseReceived`, with `requestWillBeSent` and `dataReceived` only when needed (`utils/mod.rs:4008-4036`). One spawned task per page collects the network bytes (`utils/mod.rs:4128`). These are handler messages, not CDP commands.
- The content is read by `content_bytes_stream`. One `Runtime.evaluate` wraps `outerHTML` in a V8-only object. One `Runtime.callFunctionOn` gets the length. Then one `callFunctionOn` per 64 Ki UTF-16 units. The release is batched through a shared `runtime_release` worker (`chromey/src/content_stream.rs:1-60`, `:444-511`).
- **Under `balance` pressure only**, the chunks are written straight to a spool file with the lol_html extractor folded into the pump (`utils/mod.rs:4731-4800`).

**Close:** `TabCloseGuard` sends the page to a global unbounded channel served by a dedicated OS thread named `spider-tab-closer`, running its own current-thread runtime, which dedups by target ID. The page's task never waits for `Target.closeTarget` (`chrome.rs:1944-2000`, `:2110-2160`).

**Count with the defaults (no viewport, no interception), from source: about 21 commands per page.**

| Phase | Commands |
|---|---|
| Create and attach | 2 |
| Frame | 3 |
| Isolated world | 2 |
| Network | 1 |
| Auto-attach and log | 2 |
| Stealth script | 1 |
| User agent | 2 |
| Hardware concurrency | 1 |
| Navigate | 1 |
| Content (small page) | about 4 |
| Close | 1 |
| **Total** | **about 21** |

This fits the browser gap report's "about 25", which included mouse events. The count was not traced on the wire (**unverified by tap**).

**Why each is sent:**

- The frame and lifecycle commands drive `load` detection.
- The isolated world gives Chromey a clean evaluation context.
- `Network.enable` drives the response metadata, status and headers that Spider reports.
- Auto-attach lets it see iframes and workers.
- The stealth script, user-agent and hardware overrides are fingerprint consistency.
- The streamed content read caps the size of each message.

**Stealth script size.** It is assembled at run time by `spider_fingerprint::emulate_with_profile` from templates in `spoofs.rs` (36.9 KB of Rust source), `spoof_gpu.rs` and `spoof_user_agent.rs`, plus a random GPU profile (`spider_fingerprint-2.39.0/src/lib.rs:960-974`, `:722-833`). The emitted size was not measured: **an estimate of tens of KB, unverified**. That is one to two orders of magnitude below Foxhound's 583 KB Consent bundle. **Spider registers it per tab too**, so the per-target pattern is common; the size is the outlier.

**Tab reuse and browser pool.**

- `spider/src/utils/tab_pool.rs:1-130` implements a FIFO pool of tabs on a `crossbeam` `ArrayQueue`. Release navigates to `about:blank` with a 5 s cap, then pushes the tab back. **Nothing in the crate uses it**: `TabPool` appears only in its own file and the `pub mod` line (`utils/mod.rs:69`).
- The live path makes a new tab per page with `attempt_navigation("about:blank", …)` → `Target.createTarget` (`chrome.rs:1470-1517`; macro `chrome_page_fetch!`, `website.rs:79-140`). One browser is set up per crawl (`website.rs:9182-9210`, `setup_browser`, `:13304`). One context is created per browser with `disposeOnDetach` (`chrome.rs:1094-1131`).
- There is no browser pool. A browser is relaunched only when `browser_dead` is set by a WebSocket or launch error (`chrome.rs:1071-1091`, `relaunch_browser_at` at `website.rs:5371`). There is no memory-based recycling.

### 2.2 `smart` mode (HTTP first, browser only when needed)

`smart = ["chrome", "chrome_intercept"]` (`Cargo.toml:444-447`).

- `crawl_concurrent_smart` (`website.rs:11655`) fetches each page over HTTP (`Page::new_page_with_cache_engine`, `:11798`).
- It then calls `smart_links` → `links_stream_smart` (`page.rs:9744-9790`). That runs the **same lol_html pass** that extracts links, with extra handlers that score whether the page needs rendering (`page.rs:8395-8560`). The threshold is `SMART_UPGRADE_THRESHOLD = 10`. A strong signal sets it at once; four or more non-tracker `<script src>` add 10.
- The strong signals are:
  - `/_nuxt/` assets;
  - a script path matching Chromey's `ALLOWED_MATCHER` of framework files;
  - a `<noscript>` containing "enable javascript" or similar (`NO_SCRIPT_JS_REQUIRED`, `page.rs:1879-1886`);
  - an inline script with DOM-mutation or SPA-routing calls (`DOM_SCRIPT_WATCH_METHODS`, `page.rs:1888-1905`);
  - an anti-bot body (`page.rs:8628-8633`).
- The upgrade happens only when `CrawlEnhancement::RenderUpgrade` is enabled and the score is at least 10.
- The browser is launched **lazily, once**, through a `tokio::sync::OnceCell` (`OnceBrowser`, `website.rs:11663`; `page.rs:8638-8650`). The page is fetched again in a new tab, and the lol_html link extractor runs on the rendered chunks (`page.rs:8650-8750`).
- A transport-level HTTP failure with an empty body also falls back to Chrome (`website.rs:11917-11930`).

**Cost:** the signals cost no extra parse; they are handlers in the one streaming pass. An upgraded page is fetched twice: HTTP, then browser.

### 2.3 Transformations and extraction

Main text is **not** part of Spider's crawl. `spider_transformations` (a separate crate that depends on `spider`) exposes `transform_content` for callers, typically the broadcast subscriber (`spider_transformations-2.39.13/src/transformation/content.rs:655-760`). Its pipeline:

1. Decode to a `String` (`get_html_with_selector`).
2. Optional lol_html removal of the ignore list: `img`, `svg`, and for `main_content` `nav`, `header:first-of-type`, `footer`, `body > aside:not(:first-of-type)` (`content.rs:225-268`).
3. Optional `llm_readability::extractor::extract`, a full html5ever DOM (`:699-713`).
4. Optional `clean_html` (`:715-719`).
5. `fast_html2md` (lol_html based), `html2text` or text (`:721-758`).

There are three or four passes, each building a new `String`. All options are off by default, and `ReturnFormat` defaults to `Raw` (`content.rs:29-35`, `:100-115`).

### 2.4 `balance` and `detect_system`

- **The sampler.** One background task, started once (`detect_system.rs:174-180`), loops every `sysinfo::MINIMUM_CPU_UPDATE_INTERVAL`: 200 ms on Linux and macOS (`sysinfo-0.38.4/src/unix/linux/system.rs:115`). It refreshes the global CPU and memory and the process RSS (`detect_system.rs:119-171`), stores three `AtomicI8` states, and pushes the memory state into `html_spool`'s cache. **Per page, a reader pays one relaxed atomic load.**
- **CPU states:** 0 below 70%, 1 at 70% or more, 2 at 95% or more (`:52-60`).
- **System memory states** (`disk` + `balance`): 1 at 50% or more, 2 at 80% or more (`:67-77`).
- **Process RSS states,** as a percentage of RAM: pressure at 90% or more, critical at 95% or more. Both can be set by environment variable (`:79-117`).
- **Concurrency.** `get_semaphore` takes the worse of CPU and process memory. At 1 or more it returns the process-wide `SEM_SHARED` instead of the crawl's own semaphore. At 2 it sleeps 100 ms, at most once per time window, not per link (`utils/mod.rs:10038`, `:10100-10150`). An opt-in environment flag keeps the crawl's own limit (`RESPECT_CONCURRENCY_UNDER_LOAD`).
- **HTML spool.** `should_spool` is false at or below a minimum size. Otherwise it spools when the state is critical; when the state is pressure and the page is over a quarter of the per-page threshold or over the memory budget; and in the normal state, only for very large pages (`html_spool.rs:358-390`). It runs in `channel_send_page` before the broadcast (`website.rs:15089-15114`), and in the Chrome content path when RSS pressure is at 1 or more (`utils/mod.rs:4743-4750`).
- **Visited set spill.** With `disk`, `insert_link` writes to SQLite when the system memory state is 2, when the set holds 15,000 links or more, or under a shared disk. At state 1 it goes partly to disk (`website.rs:1883-1925`, `:1023-1030`).

### 2.5 Budget, depth and subdomain checks

- `is_allowed` → `is_allowed_budgetless`: in order, the visited-set probe, the whitelist and blacklist (compiled regex), robots.txt when enabled, then depth (`website.rs:2090-2226`).
- **Depth counts the segments of the URL's path**, not link hops (`is_over_inner_depth_budget`, `website.rs:2248-2269`). It is adjusted once for a start URL that is already deep (`:14632-14644`).
- **Budget:** per-path-prefix counters in a `HashMap`, plus a `*` wildcard. Each link walks its path segments and decrements the matching counters (`website.rs:2292-2365`). That is O(path segments) map probes per link.
- **Subdomain, TLD and external domains** are checked while extracting, inside `push_link` → `validate_link` in the lol_html handler (`page.rs:2878-2960`). An off-scope link is never allocated in the set. The same place:
  - skips ignored asset extensions (`push_link_check`, `page.rs:2964-2990`);
  - **rewrites every link's scheme to the parent's scheme** (`page.rs:2905-2908`).

### 2.6 Progress and broadcast

- There is no per-page progress object. `emit_log` is a `log` macro (`website.rs:9361`).
- `channel_send_page` does one `broadcast::Sender::send` per page, plus an atomic increment of `ChannelGuard` when a guard is set (`website.rs:15116-15120`).
- Each receiver clones the `Page` on `recv`. `html` is `Option<bytes::Bytes>` (`page.rs:2426`), so the body is shared, but the struct's other fields (URL, headers map, links) are cloned per subscriber.
- The channel capacity defaults to `DEFAULT_PERMITS` (`website.rs:14676-14690`).
- With no subscriber, the page is dropped.

---

## 3. Colly v2.3.0

**Tasks and hand-offs.**

- `Collector.scrape` builds the `http.Request`, runs `requestCheck`, then `wg.Add(1)` and, in Async mode, `go c.fetch(...)` (`colly.go#L632-L685`). In Sync mode it calls `fetch` inline.
- `fetch` runs, in order: `OnRequest`, the backend `Cache`/`Do`, `OnResponseHeaders`, the charset fix, `OnResponse`, `OnHTML`, `OnXML`, `OnScraped` (`colly.go#L687-L768`).
- `Request.Visit` re-enters `scrape` with `Depth+1` from inside the callback (`request.go#L122-L123`). In Async mode, each discovered link becomes a goroutine at once.
- **No collector-wide concurrency limit exists.** `LimitRule.Parallelism` is a buffered channel per rule (`http_backend.go#L51-L91`), acquired in `Do` before the request (`#L174-L185`).
- The `queue` package is the alternative. It runs `Threads` goroutines reading an unbuffered channel fed by one loop goroutine (`queue/queue.go#L133-L212`). Each request is JSON-marshalled into storage and unmarshalled back (`#L117-L123`, `#L214-L222`).

**Fetch client.**

- `httpBackend.Init` makes `&http.Client{Jar, Timeout: 10s}` with no `Transport` (`http_backend.go#L93-L100`), so Go's `DefaultTransport` is used: `ForceAttemptHTTP2: true`, `MaxIdleConns: 100`, and the default `MaxIdleConnsPerHost = 2` (`go1.24.0 net/http/transport.go#L45-L60`).
- Above 2 concurrent requests per host over HTTP/1.1, the extra connections close on release and reopen. HTTP/2 hosts multiplex on one connection.
- The body is read whole with `io.ReadAll`, capped at `MaxBodySize` (10 MiB default). gzip is decoded by hand when needed (`http_backend.go#L205-L225`, `colly.go#L495-L496`).

**Frontier and dedup.**

- `requestHash` is FNV-64a of the WHATWG-normalized URL, plus the body for POST (`colly.go#L1637-L1654`).
- `InMemoryStorage` holds a `map[uint64]bool` behind a RWMutex (`storage/storage.go#L48-L82`).
- `requestCheck` calls `IsVisited` and then `Visited`: two lock acquisitions, not atomic. Two goroutines can both pass for the same URL (`colly.go#L786-L812`). Redirect targets are checked the same way (`colly.go#L1451-L1490`).
- Depth is the hop depth, checked at `scrape` (`colly.go#L772-L774`).
- The queue is a linked list of serialized requests with `MaxSize` 100,000 (`queue/queue.go#L60-L72`, `#L231-L247`).
- The `storage.Storage` interface allows external stores; Redis and SQLite back-ends are separate modules that were not read (**unverified**).

**Politeness.** `LimitRule` with `Delay` and `RandomDelay`:

- The sleep runs in a `defer` **after** the response, while the slot is held. With `Parallelism: 1`, the spacing is response time plus delay (`http_backend.go#L176-L184`).
- Rules are matched by scanning them (regex or glob) under a read lock, once per request (`#L114-L126`).
- robots.txt is off by default (`IgnoreRobotsTxt = true`, `colly.go#L503`). When it is on, it is fetched synchronously by the first request to the host and cached in a map (`colly.go#L843-L905`).

**Parsing.** `handleOnHTML` returns early without HTML callbacks. Otherwise it builds a goquery document (a full `x/net/html` tree) from the body, resolves `<base href>`, and runs `doc.Find(selector)` for each callback (`colly.go#L1175-L1235`). `fixCharset` re-encodes the whole body when the charset is not UTF-8, with optional `chardet` sniffing (`response.go#L65-L108`).

**Main text:** none.

**Written per page:** nothing, except optional `CacheDir` (`http_backend.go#L128-L171`): a SHA-1-named gob file per GET, written as a temporary file and renamed.

**Browser:** none.

**Memory.**

- The visited map grows by one entry per URL, forever.
- In Async mode, the frontier is goroutines blocked on a `LimitRule` channel, each with its stack and captured request. There is no bound unless the user adds one.
- `Context` is a mutex-guarded map per request.

**Progress:** two atomics, `requestCount` and `responseCount` (`colly.go#L134-L135`), shown by `String()` (`#L906-L915`).

---

## 4. Katana v1.8.0

### 4.1 Standard engine (HTTP)

**Tasks.**

- The runner starts up to `Parallelism` (default 10) targets at once with `sizedwaitgroup` (`internal/runner/executer.go#L37-L54`).
- Each target gets a `CrawlSession` with its own queue and its own `retryablehttp` client (`pkg/engine/common/base.go#L318-L389`).
- `Shared.Do` reads `Queue.PopWithContext` and starts one goroutine per item under `sizedwaitgroup(Concurrency)`, default 10 (`base.go#L405-L442`). Inside it, **one more goroutine per page** does the rate-limit `Take` so cancellation is not blocked (`#L454-L467`).
- Then come back-off, delay and the per-domain cap, then `makeRequest`, `Output`, `ParseResponse` and `Enqueue`, all in the same goroutine (`#L468-L521`).
- **The queue's pop goroutine sleeps 1 s** whenever the queue is empty, and ends the crawl only after `Timeout` (10 s by default) with nothing to pop (`pkg/utils/queue/queue.go#L80-L131`). After the seed is popped, the first wave waits up to 1 s for the seed's links. Every crawl ends with up to 10 s of idle.

**Fetch client** (`base.go#L354`; `pkg/engine/common/http.go#L23-L75`).

- The transport has `MaxIdleConns` 100, `MaxIdleConnsPerHost` 10 and `MaxConnsPerHost` 100, with fastdialer for DNS, TLS verification off, and optional JA3 impersonation.
- It is **HTTP/1.1 only**. Setting `TLSClientConfig` and `DialContext` without `ForceAttemptHTTP2` turns off h2 (`go1.24.0 transport.go#L456-L481`).
- `retryablehttp` keeps a second client with `http2.ConfigureTransport` and uses it only after the error "HTTP/1.x transport connection broken: malformed HTTP" (`retryablehttp-go v1.3.29 client.go#L113-L117`, `do.go#L68-L72`).
- Retries default to 1 with a host-spray policy (`http.go#L73`).
- The redirect callback **parses every intermediate redirect response** (body, goquery, links) and enqueues its links (`base.go#L354-L375`).

**Per page in `makeRequest`** (`pkg/engine/standard/crawl.go#L22-L150`):

- `req.Dump()` (the raw request).
- `io.ReadAll` capped at 4 MiB.
- An MD5 content-dedup check. That is a LevelDB `Get` and `Put`, done before parsing, and it skips pages with identical bodies.
- An optional simhash similarity check and optional Wappalyzer.
- `BuildKnowledgeBase(string(data))`. That is nil unless the classifier or extractors are on (`pkg/types/crawler_options.go#L264-L280`).
- `string(data)` twice: once as the restored body, once as `response.Body`.
- goquery parse.
- `httputil.DumpResponse(resp, true)`, which copies the whole body again into `Raw`.

**Frontier and dedup** (`Shared.Enqueue`, `base.go#L107-L202`). For each discovered link, in order:

1. URL validity.
2. An optional query strip or similarity fingerprint.
3. Depth, where depth exceeded means output without visiting.
4. `MaxDomainPages`.
5. `UniqueFilter.UniqueURL`: an hmap with `DefaultDiskOptions`, LevelDB in a new temp directory. `Get`, then `Put` with no `Sync` (`pkg/utils/filters/simple.go#L28-L44`; `hmap v0.0.102 store/hybrid/hybrid.go#L59-L64`, `store/disk/leveldb.go#L86-L94`). This is check-then-set and not atomic.
6. The cycle check: the longest repeated substring of the URL (`simple.go#L65-L75`).
7. The scope check, which parses the URL again.
8. `queue.Push`.

So there is **one LevelDB write per new URL and one per fetched body**. The queue is in memory: a stack for depth-first (the default) or a priority queue keyed by depth (`queue.go#L20-L71`).

**Politeness.** A global `ratelimit.New(150/s)` by default (`crawler_options.go#L175-L183`; flag default at `cmd/katana/main.go#L224`). A per-host `AutoLimiter` when a host rate limit is set (`HostRateLimit`, `HostRateLimitMinute`). A fixed `Delay` in seconds. After a 429 or 503, a per-host exponential back-off: 1 s × 2^(n−1), up to 30 s, with jitter (`base.go#L37-L46`, `#L252-L294`). robots.txt is not enforced; `-kf robotstxt` only mines it for URLs.

**Parsing.** About 26 body parsers, each a separate `doc.Find(...)` over the full goquery tree: `a`, `link`, `img`, `script`, `form`, `meta`, htmx attributes and so on. Header parsers cover `Link`, `Content-Location` and `Refresh`, plus a custom regex. With `-jc`, there is also a regex endpoint scrape of textual bodies (`pkg/engine/parser/parser.go#L38-L91`, `#L173-L781`).

**Main text:** none.

**Written per page** (`pkg/output/output.go#L178-L285`):

- Extension, match and filter checks.
- An optional per-page response file plus an index update (`#L214-L232`).
- JSON (jsoniter) or a screen line, then under **one global mutex** `gologger.Silent()` to stdout and a `bufio.Writer` file (`#L269-L281`). Flush and fsync happen only in `Close` (`pkg/output/file_writer.go`).
- Errors go to an error file.

**Memory.** The seen set and content hashes are on disk; LevelDB's in-memory buffers use goleveldb defaults (**unverified**). The queue is unbounded in memory. Bodies are held about four times per page until output. `DomainPageCounter` is a `sync.Map` per domain. The host back-off is an LRU of 10,000.

**Progress:** `atomic.AddInt64(&resultCount)` per written result (`output.go#L267`) and a summary at the end (`executer.go#L57`). The resume file lists the targets still in flight, which are crawled again from scratch (`internal/runner/runner.go#L37-L56`, `#L158-L161`).

### 4.2 Hybrid engine (`-hh`, rod)

**Browsers, contexts and pages.**

- **One Chrome per run** (rod launcher with `Leakless`) and **one incognito browser context** created with `Target.createBrowserContext`, carrying the proxy and the `<-loopback>` bypass (`pkg/engine/hybrid/hybrid.go#L59-L175`).
- **A new tab per page**: `Browser.Page(TargetCreateTarget{})`, closed with a `defer` (`crawl.go#L37-L56`).
- No recycling.

**Concurrency: sequential per target.** `Crawler.Do` calls `doRequest` inline, one page at a time, and its comment says CDP on one browser is not safe concurrently (`hybrid.go#L244-L330`). Parallelism comes only from targets: `Parallelism` 10 targets share the one browser and context.

**CDP per page** (`crawl.go#L30-L448`):

- `Network.setUserAgentOverride` and `Network.setExtraHTTPHeaders` when custom headers are set (`#L450-L476`).
- `Fetch.enable` with pattern `*` at the **Response** stage (`#L64-L68`; `hijack.go#L37-L61`).
- `Page.navigate`, then the lifecycle wait. The `heuristic` strategy is the default (`main.go#L181`): `firstMeaningfulPaint`, then `WaitStable(1s)`. Then a fixed 200 ms sleep.
- `a[onclick]` elements, up to 10, each clicked with a 1 s wait, and navigated back if the URL changed.
- `DOM.getDocument(depth: -1, pierce: true)`, then `HTML()`.
- `Fetch.disable` and `Target.closeTarget`.
- **Every subresource** costs `Fetch.requestPaused` → `Fetch.getResponseBody` → `Fetch.continueRequest`. In Go, each one also gets two raw dumps, a goquery parse, optional Wappalyzer and the KB, and `ParseResponse` and `Enqueue` (`#L70-L216`).
- The shadow-DOM text is rebuilt from the `DOM.getDocument` tree and parsed again (`#L392-L404`). The final HTML is parsed again (`#L415`).

rod's own attach and domain-enable commands per tab were not read (**unverified**).

### 4.3 Headless engine (`-hl`, a state-graph explorer)

**Browsers and pages.**

- Per target: `MaxBrowsers: 1` (`pkg/engine/headless/headless.go#L123`). The `TODO` at `#L215` says concurrency is fixed at 1.
- The pool is `rod.Pool[BrowserPage]` (`browser/browser.go#L39`, `#L82`). Each slot is **its own Chrome process** with its own temp profile, launched with Playwright's flags. It prefers `chrome-headless-shell` on linux/amd64, with an incognito flag (`#L113-L207`, `#L371-L473`).
- **One tab reused for every navigation:**
  - `GetPageFromPool` creates it on first use.
  - `PutBrowserToPool` closes stray targets (`Target.getTargets` plus `closeTarget`) and calls `Page.bringToFront` (`#L654-L684`).
  - A page whose context was cancelled, or whose browser is disconnected (a `Browser.getVersion` probe), is discarded and its browser closed (`#L654-L716`).

**Init scripts once per tab:** `EvalOnNewDocument(stealth.JS)`, from go-rod/stealth, at 185,765 bytes of `assets.go` including the Go wrapper, plus `utils.js` (12,310 bytes) and `page-init.js` (8,168 bytes) (`browser.go#L460-L470`; `js/js.go#L19-L27`). Interception is `Fetch.enable` `*` at the response stage, with a body fetch per response and cookie-banner blocking (`browser.go#L495-L600`).

**Per state:**

- `page.HTML()` and `page.Info()`, a DOM normalizer, SHA-256 and simhash (`crawler/state.go#L54-L101`).
- `FindNavigations` does one `Runtime.evaluate` per element class (forms, buttons, links, listeners). It is a composed read in page JavaScript (`browser/element.go#L60-L100`, `#L199-L230`).

**Memory.** `crawlGraph.AddPageState` stores each `PageState` **with `DOM` and `StrippedDOM`** for the whole run (`graph/graph.go#L47-L62`, `state.go#L80-L96`).

---

## 5. What Foxhound could adopt

Each item below is a setting with an off state. The default should match or beat these crawlers. "Loss" marks what Foxhound should not copy.

**Adopt: the browser**

1. **Close tabs off the critical path.** Spider hands `Target.closeTarget` to a dedicated closer that dedups by target ID. The page's task returns at once (`spider/src/features/chrome.rs:1944-2160`). Foxhound's `closeTarget` then `detachFromTarget` is part of each page's command sequence, and Chrome refuses the detach once every 2.5 pages (browser gap report). Whether Foxhound awaits them on the page's path was not checked here (**unverified**). Setting: `browser.close_tabs: background | inline`.
2. **Stealth and init scripts: size first, then registration.** Both Spider (per tab, tens of KB, estimate) and Katana headless (once per reused tab, about 206 KB) register their fingerprint scripts with `addScriptToEvaluateOnNewDocument`. Foxhound sends 583 KB per target. The comparison favours **cutting the bundle and splitting it by feature**, loading only the parts a Session's settings use. Per-target registration is what Spider does too. This keeps ADR 0048's detectors and ADR 0138's reads.
3. **Read the document as one composed fetch.** Chromey's `content_bytes_stream` does one `Runtime.evaluate` to wrap `outerHTML`, `callFunctionOn` slices of 64 Ki units, and batched `releaseObject` (`chromey/src/content_stream.rs`). Katana's headless engine makes one evaluate per element class. Both support #1894's direction: one composed read per document, streamed into lol_html in Rust as chunks arrive.
4. **A tab pool as an opt-in setting.** Katana headless reuses one tab per browser. Spider has `TabPool` code with no caller (`tab_pool.rs`). Reuse saves `createTarget`, attach and the init chain, about 10 commands per page in Chromey's chain. But it carries Session state across pages (`sessionStorage`, service workers, history), and the Spider author left it unused. If adopted, only within one Session and one origin, off by default. Shipping it on by default would weaken Session isolation (ADR 0040), a **loss**.
5. **Sample machine pressure; do not measure it per page.** Spider runs one background sampler every 200 ms; a page pays one atomic load (`detect_system.rs:156-180`). Foxhound walks the browser processes' PSS at each page admission (`goodfetch-page/src/pool.rs:566-588`). Adopt the sampled state for admission and keep the per-browser footprint read for recycling decisions. This keeps ADR 0040's governor promise. Setting: the governor's sample interval.
6. **Escalate from HTTP to the browser inside the one HTML pass.** Spider's `smart` scores in the same lol_html pass that extracts links: framework script paths, the script count, `<noscript>` "enable javascript", DOM-mutation calls, anti-bot markers. Only scores of 10 or more launch, lazily, a browser (`page.rs:8395-8650`). If Foxhound's tier escalation decides in a separate pass, fold its signals into the extractor's handlers. The double fetch on upgrade is the price; Foxhound could keep the HTTP response for the first-paint fields.

**Adopt: the HTTP path**

7. **Check scope during link extraction.** Spider's `validate_link` runs inside the lol_html handler. Off-domain and asset-extension links never enter the batch (`page.rs:2878-2990`). Foxhound could filter scope and Filter-list matches before the batch reaches admission and the run file. That is fewer rows, with no change to what is admitted.
8. **Count progress; do not serialize it.** Colly (2 atomics), Katana (1 atomic) and Spider (none) have no progress object per page. This supports the Spider gap report's R7: atomics per page, and a snapshot only on read or change.
9. **Cheaper Politeness when it cannot bind.** Colly's `LimitRule` is one channel send and receive per request. Katana's limiter is one token `Take`. Neither touches storage. This matches the Spider gap report's run-private in-memory Permit store, which keeps ADR 0053 and 0086: any cap set still holds. One detail to copy: Katana's per-host 429/503 back-off. One detail **not** to copy: Colly applies its `Delay` *after* the response while holding the slot, so the spacing drifts with latency.
10. **A middle durability level for the seen set.** Katana keeps its seen URLs in an embedded on-disk KV with no fsync: one `Put` per new URL, and the memory stays flat. Between Foxhound's per-batch SQLite commit and Spider's pure memory, a `durability: off` seen set could be an in-memory hash that spills unsynced to disk past a size, as Spider does at 15,000. This needs ADR 0138's commit design to change for `off`, as the Spider gap report says.
11. **Buffered output with explicit flush points.** Katana writes one line per page into a `bufio.Writer` under one mutex and flushes at close. Where a Dataset's durability setting allows it, batch Record writes and flush on Checkpoint or a clean stop.

**Losses: do not copy these**

- **Spider's depth is URL path depth** (`website.rs:2248-2269`), not link hops. Colly and Katana count hops. Foxhound's depth should stay hop-based.
- **Spider rewrites every link's scheme to the parent's** (`page.rs:2905-2908`). That silently changes URLs.
- **Colly's unbounded goroutine per link** in Async mode: the frontier has no memory bound. Also Colly's `MaxIdleConnsPerHost = 2` connection churn, and its non-atomic visited check.
- **Katana's 1 s empty-queue poll** and its end-of-crawl after 10 s idle. Its HTTP/1.1-only client. Three to four body copies per page. Parsing every redirect hop and every subresource body. Its per-tag DOM passes.
- **Katana's sequential browser crawl** (hybrid and headless, one page at a time per target). And the headless graph that **keeps every page's full DOM** for the whole run.
- **Spider's `SEM_SHARED` switch under load:** at CPU load of 70% or more, a crawl's own concurrency limit is replaced by a process-wide pool unless an environment variable is set. Foxhound's limits must hold under load (ADR 0041).

---

## 6. Unverified, or not measured

- Spider's stealth script size in bytes. It is assembled at run time and was estimated from template source sizes, not emitted.
- Spider's per-page CDP count (about 21) was counted from source, not tapped on the wire. The count can rise with optional settings: viewport, interception, `real_browser` mouse events, ad blocking, locale and timezone, `disable_log`.
- rod v0.116.2's own commands per new tab for Katana hybrid and headless: attach, and the domain enables.
- goleveldb's default buffer and cache sizes behind Katana's hmap.
- That Go 1.26's `Transport.protocols()` keeps the go1.24.0 logic cited. Katana builds with 1.26.
- Colly's external storage back-ends (Redis, SQLite) and their per-page writes.
- Whether any Spider build path outside `crawl_concurrent` (chrome), `crawl_concurrent_smart` and the HTTP path uses `TabPool`. A search of `src/` found no caller.
