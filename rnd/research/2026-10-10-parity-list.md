# Automan — V1 parity list: Crawlee (JS and Python), Spider-rs and Stagehand v4

Research of **October 10, 2026** for ticket #62 (map #61). It refreshes the [crawl capability matrix](2026-10-07-crawl-capability-matrix.md) and the [agent capability matrix](2026-10-07-agent-capability-matrix.md) against the current releases. The aim is a checkable list: a project built on any of the four references should be able to move to Automan without losing a user-facing capability it uses. Like the rest of `rnd/`, this list is input for inspiration; it adopts no term or decision.

## Summary

- **Current versions:** Crawlee JS **3.18.2** (stable, 2026-09-29; **4.0.0-rc.1** published 2026-10-06), Crawlee Python **1.10.4** (2026-10-06), Spider **2.53.9** (crates.io, 2026-09-05, still the latest release), Stagehand **4.2.0** (TypeScript, Python and Go, all released 2026-10-08). Sources: [cjs:npm], [cpy:pypi], [sp:crate], [sh:release].
- **About 140 rows** in nine sections, one per map area. Each row ID carries its owning area: `RT` Runtime & state, `SDK` SDK binding & distribution, `SA` Site access, `ST` Stealth & challenge solving, `CO` Collection, `AI` AI & agents, `SC` Security & compliance, `CN` Console, `CL` Cloud readiness.
- **Crawlee's biggest surfaces** are Collection (queue, sessions, datasets, storage backends) and Site access. Much of its browser surface comes from the raw Playwright or Puppeteer page it hands to user code (rows marked "driver"). Parity therefore implies either a Playwright-compatible page or an explicit equivalent for each row.
- **Spider** exposes about 130 Cargo features. Many rows are opt-in builds (`opt`), and its Node and Python bindings lag far behind the crate (npm 0.0.163, PyPI 0.0.57).
- **Stagehand v4** is a small Chromium driver plus AI primitives (`act`, `observe`, `extract`), WebMCP and agent integrations. Much of what users associate with it is **Browserbase-only** (`cloud`): action cache, Model Gateway, fetch, search, CAPTCHA solving, managed proxies, recordings, regions.
- **The Crawlee ↔ Stagehand integrations are still on Stagehand v3:** `@crawlee/stagehand` 3.18.2 targets `^3.0.0`, and Crawlee Python caps `stagehand<4`. A Crawlee+Stagehand project therefore uses v3 calls (including Python `execute`), not the v4 surface. Sources: [cjs:stagehand-pkg], [cpy:pyproject], [cpy:stagehand-types].
- **New since the 2026-10-07 matrices:**
  - **Stagehand 4.2.0:** page PDF export, snapshot and locator timeouts, WebMCP add/remove hooks, automatic extension discovery on connect, and telemetry that stays off until configured. Its action cache now defaults to **on** whenever a Browserbase session is present (`cache ?? true`); the 2026-10-07 note had read a `false` default in 4.1.0.
  - **Crawlee Python 1.7–1.10:** `ThrottlingRequestManager`, `PydanticAiCrawler`, `FileDownloadCrawler`, router middleware and the security guide.
  - **Crawlee JS 4.0 RC:** a large breaking change set (see the [deltas](#version-deltas-that-will-move-the-target)).

## Pinned references

| Code | Reference | Version and date | License | Base sources |
|---|---|---|---|---|
| **JS** | Crawlee for JavaScript/TypeScript | 3.18.2, 2026-09-29 (npm `latest`); `rc` 4.0.0-rc.1, 2026-10-06 | Apache-2.0 | [cjs:readme], [cjs:changelog], tag `v3.18.2` |
| **PY** | Crawlee for Python | 1.10.4, 2026-10-06 (PyPI); 1.10.5b3 pre-release | Apache-2.0 | [cpy:pyproject], [cpy:changelog], tag `v1.10.4`, [docs 1.10](https://crawlee.dev/python/docs/guides) |
| **SP** | Spider (`spider` crate and its workspace: `spider_agent`, `spider_cli`, `spider_mcp`, `spider_worker`, `spider_utils`) | 2.53.9, 2026-09-05; the crate's `.cargo_vcs_info.json` resolves to SHA `e4b53c5b582357e46433fa9ba3d91f930fb67cb4` | MIT | [sp:cargo], [sp:crate-readme], [sp:website] |
| **SH** | Stagehand v4 (`@browserbasehq/stagehand`, PyPI `stagehand`, Go `github.com/browserbase/stagehand/packages/sdk-go/v4`) | 4.2.0, 2026-10-08, release commit `2d605b099996e10c3e3ab0dcf293db8d482a2c97` | MIT | [sh:release], [sh:npm], [sh:pypi], [sh:go], [v4 docs](https://docs.stagehand.dev/v4/first-steps/introduction) |

**Method:** this note reads first-party sources directly: registry metadata (npm, PyPI, crates.io and the Go module proxy), tagged source trees, the published Stagehand 4.2.0 tarball, the Spider 2.53.9 `.crate` file, changelogs and the official docs. Exa and Context7 were not needed, because every fact here has a first-party page. No package was installed and no crawl or browser session was run.

## How to read the tables

Each cell gives the reference's status, plus the API or flag name where useful.

| Mark | Meaning |
|---|---|
| `Y` | Built in with the default install. |
| `Y (driver)` | Crawlee hands user code the raw Playwright/Puppeteer page; the capability belongs to the driver, not to Crawlee code. |
| `opt` | Needs an optional package, extra or Cargo feature (named in the cell). |
| `cloud` | Only through the vendor's paid cloud (Apify, Browserbase, Spider Cloud); needs an API key. |
| `part` | Partial (the cell says how). |
| `ext` | Documented recipe that relies on a third-party tool. |
| `v4` | Crawlee JS only in 4.0.0-rc.1 or on `master`, not in the 3.18.2 stable release. |
| `—` | Not found in the inspected sources. This does **not** prove absence. |
| `n/a` | Does not apply to that kind of tool. |

The list covers **user-facing capabilities**, not internals. Hosted-platform features are listed only where the SDK itself exposes them.

## Runtime & state (RT)

| ID | Capability | Crawlee JS 3.18.2 | Crawlee Py 1.10.4 | Spider 2.53.9 | Stagehand 4.2.0 | Sources |
|---|---|---|---|---|---|---|
| RT-01 | Concurrency that autoscales from CPU, memory, event-loop and client load | Y `AutoscaledPool` + `SystemStatus` | Y `AutoscaledPool`, `Snapshotter` | opt `balance`; `adaptive_concurrency` (AIMD) | — | [cjs:scaling], [cpy:g:scaling], [sp:cargo] |
| RT-02 | Concurrency bounds and request-rate cap (min/max/desired, max per minute) | Y `min/maxConcurrency`, `maxRequestsPerMinute` | Y `ConcurrencySettings` | Y `with_concurrency_limit`, `with_delay`; opt `rate_limit` | — | [cjs:basic], [cpy:basic], [sp:website] |
| RT-03 | Resource budget aware of container limits (cgroups, Kubernetes, systemd) | Y `systemInfoV2`, default since 3.15.2 | Y, reads Docker/K8s/systemd/Windows-job limits | — | — | [cjs:changelog], [cpy:g:scaling] |
| RT-04 | Retries: max count, error handler before retry, failed-request handler | Y `maxRequestRetries`, `errorHandler`, `failedRequestHandler` | Y | Y `with_retry`, `with_retry_strategy` (per attempt) | — (user code; docs show a backoff loop) | [cjs:basic], [cpy:g:errors], [sp:website], [sh:act] |
| RT-05 | Error types that steer retries (non-retryable, critical abort, session error) | Y | Y; also `abort_on_error` | part (`should_retry`, `error_status` on `Page`) | — | [cjs:core], [cpy:g:errors], [sp:page] |
| RT-06 | Timeouts: navigation, handler, whole crawl, first byte | Y `navigationTimeoutSecs`, `requestHandlerTimeoutSecs` | Y, both 60 s by default | Y `with_request_timeout`, `with_crawl_timeout`, HTTP/Chrome first-byte watchdogs | Y per-call `timeout`; locator and snapshot default 20 s (4.2.0); `domSettleTimeoutMs` | [cjs:http], [cpy:g:security], [sp:config], [sh:release] |
| RT-07 | Routing by label to handlers; typed labels; middleware | Y router; typed labels and `userData` schema validation (3.18) | Y router; `Router.use` middleware (1.7.0) | — (callbacks `with_on_should_crawl_callback`, `with_on_link_find_callback`) | — | [cjs:changelog], [cpy:g:router], [sp:website] |
| RT-08 | Pre- and post-navigation hooks | Y `preNavigationHooks`, `postNavigationHooks` | Y (post hooks 1.6.0) | part (per-URL `with_execution_scripts`, `with_automation_scripts`) | — | [cjs:http], [cpy:changelog], [sp:website] |
| RT-09 | User state persisted across restarts | Y `useState`, `RecoverableState` | Y `use_state` (1.6.0) | — | — | [cjs:core], [cpy:changelog] |
| RT-10 | Resume an interrupted crawl from persisted queue and state | Y (persisted storages, `purgeOnStart: false`) | Y ("resuming paused crawl" example) | part (`disk` keeps visited links in SQLite, not the pending frontier) | — | [cjs:configuration], [cpy:examples], [crawl matrix §5](2026-10-07-crawl-capability-matrix.md) |
| RT-11 | Stop, pause and resume a running crawl | Y `crawler.stop()`, `autoscaledPool.pause()` | Y `crawler.stop()` | opt `control`: pause/resume/shutdown by crawl id | n/a | [cjs:basic], [cpy:examples], [sp:crate-readme] |
| RT-12 | Keep-alive crawler that waits for new work | Y `keepAlive` | Y `keep_alive` | Y `website.queue()` sender (`sync`) | n/a | [cjs:basic], [cpy:basic], [sp:website] |
| RT-13 | In-process scheduled (cron) runs | — | — | opt `cron` (`with_cron`, `run_cron`) | — | [sp:crate-readme] |
| RT-14 | Lifecycle events (persist state, system info, migrating, aborting) | Y `EventManager` | Y | part (page broadcast via `subscribe`) | part (`logging.onLog` callback) | [cjs:core], [cpy:changelog], [sp:website], [sh:observability] |
| RT-15 | Run statistics, periodic status log, custom status message | Y `Statistics`, `statusMessageCallback` | Y `statistics_log_format`, `status_message_callback` | part (`get_status`; per-page duration with `time`) | part (`metrics()`: tokens and inference time only) | [cjs:basic], [cpy:basic], [sp:crate-readme], [sh:ref-stagehand] |
| RT-16 | Error grouping, with snapshots (HTML + screenshot) saved on error | Y `ErrorTracker`, `ErrorSnapshotter` | Y `ErrorTracker` + `ErrorSnapshotter` | — | — | [cjs:errsnap], [cpy:changelog] |
| RT-17 | Structured logging (levels, JSON, custom sink) | Y (`@apify/log`) | Y (JSON logging example) | Y (`log` / `tracing` crates) | Y (`logging` level/format/`onLog`; JSONL session log files) | [cjs:core], [cpy:examples], [sp:cargo], [sh:logging] |
| RT-18 | OpenTelemetry traces | v4 (`@crawlee/otel` on `master`, absent from 3.18.2) | opt `otel` extra | — (a `tracing` feature exists; no OTLP exporter found) | Y OTLP traces with W3C context propagation | [cjs:v4-otel], [cpy:g:otel], [sp:cargo], [sh:observability] |
| RT-19 | Several crawlers in one process | part (v3 crawlers share the default queue; v4 isolates them and adds a shared `ConcurrencySystem`) | Y ("run parallel crawlers" example) | Y (several `Website`s; `with_shared_queue`) | Y (several instances/browsers) | [cjs:v4], [cpy:examples], [sp:website], [sh:browser] |
| RT-20 | Hedged requests and coalescing of duplicate in-flight requests | — | — | opt `hedge`, `request_coalesce` | — | [sp:cargo] |
| RT-21 | Pluggable components (storage, HTTP client, event manager, browser plugin) | Y (`Configuration`, `StorageClient`, `BaseHttpClient`, browser plugins) | Y (`service_locator`) | Y (`with_fetch_engine`, `with_remote_fetcher`) | part (custom LLM client function) | [cjs:http-clients], [cpy:g:service-locator], [sp:website], [sh:models] |
| RT-22 | Configuration through a typed object and environment variables | Y (`Configuration`, `CRAWLEE_*`) | Y (`Configuration`, pydantic-settings) | Y (`Configuration`; env such as `CHROME_URL`, `SPIDER_WORKER`) | Y (`BROWSERBASE_*`, model keys) | [cjs:configuration], [cpy:pyproject], [sp:crate-readme], [sh:browser] |
| RT-23 | Run a script next to the browser to save round trips | n/a | n/a | n/a | Y `experimentalBatch` (a JS callback; Python and Go pass JS source) | [sh:ref-stagehand] |

## SDK binding & distribution (SDK)

| ID | Capability | Crawlee JS 3.18.2 | Crawlee Py 1.10.4 | Spider 2.53.9 | Stagehand 4.2.0 | Sources |
|---|---|---|---|---|---|---|
| SDK-01 | Primary language and package | TS/JS, npm `crawlee`, Node ≥16 | Python ≥3.10, PyPI `crawlee` | Rust/Tokio crate `spider` | TS (Node ≥22.18), Python ≥3.11, Go module `…/sdk-go/v4` | [cjs:pkg], [cpy:pyproject], [sp:cargo], [sh:npm], [sh:pypi], [sh:go] |
| SDK-02 | Same API in several languages | part (JS and Python are separate implementations whose features diverge) | part (same) | part (Node `@spider-rs/spider-rs` 0.0.163, Python `spider_rs` 0.0.57; feature set fixed at build) | Y (three SDKs generated from one protocol) | [sp:node], [sp:py], [sh:install] |
| SDK-03 | Modular install | Y (`@crawlee/*` packages) | Y (extras: `playwright`, `sql-*`, `redis`, `otel`, `stagehand`, `pydantic-ai`, `curl-impersonate`, `httpx`…) | Y (~130 Cargo features) | — (single package) | [cjs:readme], [cpy:pyproject], [sp:cargo] |
| SDK-04 | Project scaffolding CLI and templates | Y `crawlee create`/`run`/`install-playwright-browsers`; cheerio, playwright, puppeteer, camoufox templates | Y `crawlee create`; uv/poetry templates incl. adaptive and stagehand | — | — | [cjs:cli], [cjs:templates], [cpy:changelog] |
| SDK-05 | No-code CLI that crawls or drives a browser | — | — | Y `spider_cli` (crawl/scrape/download, `--return-format`) | Y `browse` CLI 0.11.1 (separately versioned; depends on Stagehand 4.1.0) | [sp:cli], [sh:browse] |
| SDK-06 | Typed public API and schemas | Y TS generics | Y typed, Pydantic | Y Rust types; `serde` opt | Y Zod / Pydantic / Go structs | [cjs:readme], [cpy:pyproject], [sp:cargo], [sh:extract] |
| SDK-07 | Docs for AI coding assistants (llms.txt, rules) | Y llms.txt generator (3.14.0) | Y llms.txt | — | Y "AI rules" page + llms.txt | [cjs:changelog], [crawlee.dev/llms.txt](https://crawlee.dev/llms.txt), [sh:ai-rules] |
| SDK-08 | Browser provisioning | Y (Playwright browser install command) | Y (`playwright install`) | part (uses installed Chrome or `CHROME_URL`) | part (uses installed Chrome; no download step; none needed with Browserbase) | [cjs:cli], [cpy:g:playwright], [sp:crate-readme], [sh:mig-pw] |

## Site access (SA)

| ID | Capability | Crawlee JS 3.18.2 | Crawlee Py 1.10.4 | Spider 2.53.9 | Stagehand 4.2.0 | Sources |
|---|---|---|---|---|---|---|
| SA-01 | Plain HTTP fetching, no browser | Y got-scraping; opt `@crawlee/impit-client` | Y impit by default; httpx, curl-impersonate opt | Y reqwest; opt `wreq` | cloud `browserbase.fetch` (no JS, 5 MB, 60 s) | [cjs:http-clients], [cpy:g:http-clients], [sp:cargo], [sh:fetch] |
| SA-02 | Replaceable HTTP transport | Y `BaseHttpClient` | Y custom client | Y `with_fetch_engine`, `with_remote_fetcher` | n/a | [cjs:http-clients], [cpy:g:http-clients], [sp:website] |
| SA-03 | HTTP/2 | Y (got-scraping) | Y (impit, httpx2 `http2`) | Y `with_http2_prior_knowledge`; opt `h2_multiplex` | n/a | [cjs:readme], [cpy:pyproject], [sp:config] |
| SA-04 | Extra HTTP request from a handler, sharing session and proxy (`sendRequest`) | Y | Y `send_request` (timeouts 1.10.0) | — | n/a | [cjs:basic], [cpy:changelog] |
| SA-05 | Chromium rendering, headless or headful | Y Playwright/Puppeteer | Y Playwright | opt `chrome`, `chrome_headed` | Y (Chromium only) | [cjs:readme], [cpy:g:playwright], [sp:crate-readme], [sh:mig-pw] |
| SA-06 | Firefox, WebKit, Edge | Y (Playwright Firefox/WebKit) | Y `browser_type`: chromium/firefox/webkit/chrome | opt `webdriver_firefox`, `webdriver_edge` (thirtyfour) | — | [cjs:examples], [cpy:pw], [sp:cargo], [sh:mig-pw] |
| SA-07 | Puppeteer backend | Y | — | — | — | [cjs:readme] |
| SA-08 | Attach to a running or remote browser (CDP or WebDriver); never close a borrowed one | v4 `remoteBrowser`; 3.18.2 only through the Stagehand plugin | part (Stagehand v3 controller only) | Y `CHROME_URL`, `with_chrome_connection(s)`, `with_webdriver` | Y `localBrowser.connect({cdpUrl})`, `browserbase.connect`; never closes a browser it did not launch | [cjs:v4-remote], [cpy:browsers], [sp:website], [sh:browser] |
| SA-09 | Browser pool: several browsers and pages, page caps, retirement, lifecycle hooks | Y `BrowserPool` (20 pages per browser, retire after 100 pages, launch and page hooks) | Y `BrowserPool` (launch and page hooks 1.6/1.7) | part (one Chrome reused per crawl; tab semaphores) | part (one context per browser, several pages; run more browsers for isolation) | [cjs:pool], [cpy:g:playwright], [crawl matrix §4](2026-10-07-crawl-capability-matrix.md), [sh:mig-pw] |
| SA-10 | Isolated incognito contexts per page or session | Y `useIncognitoPages` | Y `use_incognito_pages` | — | — (one context per browser) | [cjs:launch], [cpy:pw], [sh:mig-pw] |
| SA-11 | Persistent browser profile (user data directory) | Y `launchContext.userDataDir` | Y `user_data_dir`, `PlaywrightPersistentBrowser` | — | Y `userDataDir`, `preserveUserDataDir`; cloud contexts | [cjs:launch], [cpy:browsers], [sh:user-data] |
| SA-12 | Full native driver in the handler (frames, `route`, waits, assertions, `getBy*`) | Y (driver) | Y (driver) | part (`chrome_store_page` keeps the chromey page) | — (own, smaller driver; no `getBy*`, `expect`, `route`, `waitForResponse`; Playwright objects rejected) | [cjs:pwutils], [cpy:pw-ctx], [sp:crate-readme], [sh:mig-pw] |
| SA-13 | Element and coordinate input: click, fill, type, select, hover, scroll, drag, key press, file upload | Y (driver) | Y (driver) | part `WebAutomation` (Click/ClickAll/ClickPoint/ClickHold/ClickDrag/Fill/Type/ScrollX/ScrollY/Evaluate; no upload found) | Y locator `click/fill/type/selectOption/setInputFiles/hover/scrollTo`; page `click/hover/scroll/dragAndDrop/type/keyPress` | [sp:automation], [sh:ref-locator], [sh:ref-page] |
| SA-14 | Waits: selector, load state, network idle, delay | Y (driver auto-wait) | Y (driver) | Y `with_wait_for_selector`, idle network/DOM, delay; `WaitFor*` steps | part `waitForSelector`, `waitForLoadState`, `waitForTimeout`; no auto-wait | [sp:website], [sh:ref-page], [sh:mig-pw] |
| SA-15 | Targeting inside iframes and shadow DOM | Y (driver) | Y (driver) | — | Y (`>>` crosses iframes; CSS pierces open and closed shadow roots; refs valid across OOPIFs in 4.2.0) | [sh:ref-locator], [sh:release] |
| SA-16 | Evaluate JS in the page; init scripts that run before page scripts | Y (driver; `injectFile`, `injectJQuery`) | Y (driver) | Y `Evaluate`, `with_evaluate_on_new_document` | Y `evaluate`, `addInitScript` (page and context) | [cjs:pwutils], [sp:website], [sh:ref-page], [sh:ref-context] |
| SA-17 | Request interception and resource/ad blocking | Y `blockRequests` + driver `route` | Y `block_requests` | opt `chrome_intercept`, `with_block_assets`, `adblock`, first-party allow flags | part (`setDomainPolicy` blocks whole domains only); cloud `blockAds` | [cjs:pwutils], [cpy:pw-pre], [sp:website], [sh:ref-context], [sh:browser] |
| SA-18 | Network and console events | Y (driver) | Y (driver) | part (`with_event_tracker`; request/response maps on `Page`) | part (`page.on("console")` only) | [sp:page], [sh:ref-page] |
| SA-19 | Main-document response (status, headers, body) | Y | Y | Y `Page` (status, headers with `headers`, final redirect, remote address) | Y `Response` | [cjs:http], [sp:page], [sh:ref-response] |
| SA-20 | Custom headers, user agent, referer | Y | Y | Y `with_headers`, `with_user_agent`, `with_referer`, `with_modify_headers` | Y `setExtraHTTPHeaders` (page and context) | [cjs:http], [sp:website], [sh:ref-context] |
| SA-21 | Read and write cookies | Y (session + driver) | Y (cookie guide) | opt `cookies` (`with_cookies`) | Y `cookies`, `addCookies`, `clearCookies` | [cjs:session], [cpy:g:cookies], [sp:crate-readme], [sh:ref-context] |
| SA-22 | Proxy setup: list, custom function, auth, SOCKS, per-request choice | Y `ProxyConfiguration` (`proxyUrls`, `newUrlFunction`) | Y `ProxyConfiguration` | Y `with_proxies`, `with_proxy_strategy`, per-kind lists; opt `socks` | part (one proxy at local launch); cloud `proxies` | [cjs:proxy], [cpy:g:sessions], [sp:website], [sh:browser] |
| SA-23 | Device emulation: viewport, locale, timezone, geolocation, touch, scale | Y (fingerprint + driver context) | Y | Y `with_viewport`, `with_locale`, `with_timezone_id`, `with_auto_geolocation` | part (viewport, locale, `deviceScaleFactor`, `hasTouch` at launch; `setViewportSize`) | [cjs:blocking], [sp:website], [sh:browser], [sh:ref-page] |
| SA-24 | Redirect control and limits | part (driver/client defaults) | part | Y `with_redirect_limit`, `with_redirect_policy`, `with_max_main_frame_navigations` | — | [sp:website] |
| SA-25 | Bind to a local IP or network interface | — | — | Y `with_local_address`, `with_network_interface` | — | [sp:website] |
| SA-26 | Auto-dismiss JS dialogs | Y (driver) | Y (driver) | Y `with_dismiss_dialogs` | — | [sp:website] |
| SA-27 | HTTP auth challenge in the browser | Y (driver) | Y (driver) | Y `with_auth_challenge_response` | — | [sp:website] |
| SA-28 | Close cookie-consent modals | Y `closeCookieModals` | — | — | — | [cjs:pwutils] |
| SA-29 | Several tabs in one session, with an "active page" | Y (driver) | Y (driver) | n/a | Y `newPage`, `activePage`, `setActivePage`; new tabs are followed | [sh:tabs], [sh:ref-context] |
| SA-30 | Clipboard read, write, copy, cut, paste | part (driver code) | part (driver code) | — | Y `clipboard` API | [sh:ref-clipboard] |

## Stealth & challenge solving (ST)

| ID | Capability | Crawlee JS 3.18.2 | Crawlee Py 1.10.4 | Spider 2.53.9 | Stagehand 4.2.0 | Sources |
|---|---|---|---|---|---|---|
| ST-01 | Browser fingerprint generation and injection | Y fingerprint-suite, on by default (`useFingerprints`) | Y browserforge, on by default | opt `with_fingerprint(_advanced)`, `spider_fingerprint` | part (local "basic stealth"); cloud "advanced fingerprinting" | [cjs:blocking], [cpy:g:blocking], [sp:website], [sh:browser] |
| ST-02 | Headless-Chrome evasion patches | ext (playwright-extra / puppeteer-extra example) | — | opt `chrome_stealth`, `with_stealth(_advanced)` | cloud (Verified browsers, Scale plan) | [cjs:plugins], [sp:cargo], [sh:browser] |
| ST-03 | Browser-like header generation for HTTP | Y (got-scraping header generator) | Y (browserforge headers; impit) | opt `ua_generator`, `spoof` | n/a | [cjs:readme], [cpy:g:http-clients], [sp:crate-readme] |
| ST-04 | TLS/HTTP fingerprint impersonation for HTTP | part (got-scraping TLS; opt `impit`) | Y impit by default; opt curl-impersonate | opt `wreq` (`with_emulation`) | n/a | [cjs:http-clients], [cpy:g:http-clients], [sp:website] |
| ST-05 | Hardened browser builds (Camoufox, CloakBrowser) | ext Camoufox (template + guide) | ext Camoufox, CloakBrowser | — | — | [cjs:blocking], [cpy:g:blocking] |
| ST-06 | Cloudflare challenge handling | Y `handleCloudflareChallenge` (Playwright; markup updated in 3.18.1) | — | opt `real_browser` (Turnstile/"Just a moment" handler) | cloud `solveCaptchas` | [cjs:changelog], [sp:solvers], [sh:browser] |
| ST-07 | CAPTCHA solving (reCAPTCHA, hCaptcha, GeeTest, Imperva, Lemin) | — | — | opt `real_browser` (some solvers call Gemini) | cloud `solveCaptchas` | [sp:solvers], [sh:browser] |
| ST-08 | Block detection with automatic retry or rotation | Y `retryOnBlocked`, blocked status codes retire the session | Y `retry_on_blocked` | Y `anti_bot_tech`, `waf_check`, `should_retry`, `blocked_crawl` | — | [cjs:basic], [cjs:session], [cpy:basic], [sp:page] |
| ST-09 | Proxy rotation bound to sessions; tiered proxies that escalate on blocks | Y (`tieredProxyUrls`) | Y (`tiered_proxy_urls`) | part (rotation, `with_proxy_strategy`) | cloud (Browserbase residential proxies) | [cjs:proxy], [cpy:g:sessions], [sp:website], [sh:browser] |
| ST-10 | Hand-off to a managed unblocker | — (Apify Proxy sits outside Crawlee) | — | cloud `spider_cloud` modes: Proxy, Api, Unblocker, Fallback, Smart | cloud (Browserbase sessions) | [sp:crate-readme], [sh:browser] |

## Collection: frontier, sessions, extraction, datasets and sinks (CO)

| ID | Capability | Crawlee JS 3.18.2 | Crawlee Py 1.10.4 | Spider 2.53.9 | Stagehand 4.2.0 | Sources |
|---|---|---|---|---|---|---|
| CO-01 | Persistent request queue, dedup by unique key, forefront insertion | Y `RequestQueue` (v2, with locking) | Y `RequestQueue` | part (in-memory visited set; opt `bloom`; `disk` keeps visited only) | n/a | [cjs:storages], [cpy:g:loaders], [sp:cargo] |
| CO-02 | Static seed lists, list+queue tandem, iterable inputs | Y `RequestList`, `TandemRequestProvider`, (async) iterables | Y `RequestList`, `RequestManagerTandem` | part (seed URL + `set_extra_links`) | n/a | [cjs:storages], [cjs:changelog], [cpy:loaders] |
| CO-03 | Add URLs while the crawl runs | Y `addRequests` | Y `add_requests` | Y `queue()` | n/a | [cjs:basic], [cpy:basic], [sp:website] |
| CO-04 | Frontier ordered by priority score | part (forefront only) | part (forefront only) | opt `priority_frontier` | n/a | [sp:cargo] |
| CO-05 | Link discovery with scope strategy (same hostname/domain/origin/all), include/exclude patterns, selector, transform, limit | Y `enqueueLinks` (globs, regexps, pseudoUrls, exclude, `transformRequestFunction`) | Y `enqueue_links`, `extract_links`; non-`href` links (1.6.0) | Y subdomains/TLD/external domains, regex/glob allow/deny, `with_normalize` | n/a | [cjs:request-storage], [cpy:g:security], [sp:website] |
| CO-06 | Enqueue by clicking JS-driven elements | Y `enqueueLinksByClickingElements` | — | — | — | [cjs:pwutils] |
| CO-07 | Crawl-depth limit | Y `maxCrawlDepth` (3.14) | Y `max_crawl_depth` | Y `with_depth` | n/a | [cjs:changelog], [cpy:basic], [sp:website] |
| CO-08 | Page budgets, global and per path | part (`maxRequestsPerCrawl`) | part (`max_requests_per_crawl`) | Y `with_limit`, per-path `with_budget` | n/a | [cjs:basic], [cpy:basic], [sp:website] |
| CO-09 | Per-domain politeness: delay, 429/`Retry-After` backoff, robots crawl-delay, latency-based throttle, token bucket | part `sameDomainDelaySecs` | Y `ThrottlingRequestManager` (1.7.0) | Y `with_delay`; opt `auto_throttle`, `rate_limit` | n/a | [cjs:basic], [cpy:g:throttling], [sp:config] |
| CO-10 | Sitemap discovery and loading | Y `SitemapRequestList`, `discoverValidSitemaps` | Y `SitemapRequestLoader`, `discover_valid_sitemaps` | opt `sitemap` (`crawl_sitemap`) | n/a | [cjs:storages], [cjs:changelog], [cpy:loaders], [sp:website] |
| CO-11 | Callback for skipped or blocked links | Y `onSkippedRequest` | Y `on_skipped_request` | Y `with_on_link_blocked_callback` | n/a | [cjs:basic], [cpy:changelog], [sp:website] |
| CO-12 | Session pool: cookies + proxy + error score, rotation, retirement, persistence | Y `SessionPool` | Y `SessionPool`; pin a request to a session (`session_id`) | — (one cookie jar per crawl) | — (one context; cloud contexts) | [cjs:session-guide], [cpy:g:sessions], [cpy:changelog] |
| CO-13 | Authenticated crawling: log in once, reuse the session | part (handler + session cookies) | Y ("logging in" guide) | part (`with_cookies`, auth challenge) | Y `userDataDir`; cloud context cookbook | [cpy:g:login], [sp:website], [sh:persisted-login] |
| CO-14 | Built-in HTML parsers | Y Cheerio, JSDOM, LinkeDOM | Y BeautifulSoup, Parsel | part (streaming link extraction; `spider_utils` CSS query maps) | n/a (live-DOM locators, AI `extract`) | [cjs:readme], [cpy:crawlers], [sp:utils] |
| CO-15 | Per-page choice between HTTP and browser rendering | Y `AdaptivePlaywrightCrawler` | Y `AdaptivePlaywrightCrawler` | opt `smart` (`crawl_smart`) | — | [cjs:changelog], [cpy:crawlers], [sp:crate-readme] |
| CO-16 | Parallel backends raced with a quality score | — | — | opt `parallel_backends` + `with_quality_validator` | — | [sp:website] |
| CO-17 | Infinite-scroll helper | Y `infiniteScroll` | Y `infinite_scroll` | Y `InfiniteScroll` step | — | [cjs:pwutils], [cpy:pw-ctx], [sp:automation] |
| CO-18 | HTML → Markdown or text | part (`htmlToText`) | — | opt (`spider_transformations` markdown/commonmark/text/xml in CLI/MCP; `with_prefer_native_markdown`) | cloud `browserbase.fetch` (markdown, JSON) | [cjs:cheerio-utils], [sp:cli], [sp:website], [sh:fetch] |
| CO-19 | Contact, social and metadata extraction (emails, phones, social handles, Open Graph) | Y `social`, Open Graph parser | — | part (`Metadata`: title, description, image) | — | [cjs:utils], [sp:page] |
| CO-20 | Screenshots (full page, clip, mask) | Y `saveSnapshot` + driver | Y `get_snapshot` + driver | opt `chrome_screenshot` (`with_screenshot`) | Y `page.screenshot` | [cjs:pwutils], [cpy:pw-pre], [sp:crate-readme], [sh:ref-page] |
| CO-21 | Page to PDF | Y (driver `page.pdf`) | Y (driver) | — | Y `page.pdf` (4.2.0; TS/Python/Go) | [sh:release], [sh:ref-page] |
| CO-22 | File download and streaming | Y `FileDownload`, Playwright `Download`, `abortDownload` | Y `FileDownloadCrawler` (1.9.0) | part (`binary_file`; CLI `download`) | part (`acceptDownloads`/`downloadsPath` at launch; no download events) | [cjs:file-download], [cjs:changelog], [cpy:changelog], [sp:cli], [sh:mig-pw] |
| CO-23 | Collect every page resource (CSS, JS, images) | — | — | opt `full_resources` | — | [sp:crate-readme] |
| CO-24 | Character-encoding handling | Y `suggestResponseEncoding`, `forceResponseEncoding` | — (not checked) | opt `encoding` | n/a | [cjs:http], [sp:crate-readme] |
| CO-25 | Push records to a dataset and read them back (JS adds map/reduce) | Y `Dataset`, `pushData` | Y `push_data`, `get_data` | part (Node binding `pushData`) | — | [cjs:result-storage], [cpy:basic], [sp:node] |
| CO-26 | Key-value store for files, blobs and state | Y `KeyValueStore` | Y | — | — | [cjs:result-storage], [cpy:g:storage-clients] |
| CO-27 | Export to CSV/JSON (all keys) | Y `exportData`, `collectAllKeys` | Y `export_data`, `collect_all_keys` (1.9.0) | part (Node `exportJsonlData`) | — | [cjs:changelog], [cpy:changelog], [sp:node] |
| CO-28 | Storage backends: memory, filesystem, SQL, Redis, custom | part (memory + filesystem; custom `StorageClient`; Apify cloud client) | Y memory, filesystem, SQL (SQLite/Postgres/MySQL/MariaDB), Redis ≥8, custom | part (SQLite `disk` for links; HTTP cache on disk or in memory) | — | [cjs:storages], [cpy:storage-clients-src], [cpy:g:storage-clients], [sp:cargo] |
| CO-29 | Stream of pages as they arrive | part (one handler per request; async iterators over storages, 3.16) | part (one handler per request) | Y `subscribe`, `subscribe_guard` | n/a | [cjs:changelog], [sp:website] |
| CO-30 | WARC web-archive output | — | ext (pywb recording proxy or warcio) | opt `warc` (`with_warc`) | — | [cpy:g:warc], [sp:config] |
| CO-31 | HTTP response cache and conditional re-crawl (ETag) | — | — | opt `cache`, `cache_mem`, `cache_chrome_hybrid`, `etag_cache`; cache policies and namespaces | — | [sp:cargo], [sp:config] |

## AI & agents (AI)

| ID | Capability | Crawlee JS 3.18.2 | Crawlee Py 1.10.4 | Spider 2.53.9 | Stagehand 4.2.0 | Sources |
|---|---|---|---|---|---|---|
| AI-01 | Natural-language action on a page | opt `@crawlee/stagehand` (`page.act`, Stagehand ^3) | opt `stagehand` extra (`act`, Stagehand <4 REST) | opt `openai`/`gemini` generate page JS; `with_remote_multimodal` | Y `act()` | [cjs:stagehand], [cpy:stagehand-types], [sp:website], [sh:act] |
| AI-02 | Observe candidate actions, then replay one without inference | opt (Stagehand v3 `observe`) | opt (Stagehand v3 `observe`) | — | Y `observe()` → `act(Action)` | [cjs:stagehand], [cpy:stagehand-types], [sh:observe] |
| AI-03 | LLM extraction validated by a schema | opt (Stagehand v3 `extract` + Zod) | opt `PydanticAiCrawler` (1.8.0, experimental); Stagehand v3 | opt (`spider_agent` extraction; `openai` `extra_ai_data`) | Y `extract()` (Zod/Pydantic/Go; scope with `locator`) | [cjs:stagehand], [cpy:g:pydantic-ai], [sp:agent], [sh:extract] |
| AI-04 | Vision input (screenshot sent to the model) | — | — | opt `with_remote_multimodal` | Y `extract({ screenshot: true })` | [sp:website], [sh:extract] |
| AI-05 | Multi-step autonomous agent loop | part (Stagehand v3 agent, not bound to the crawler page) | opt (Stagehand v3 `execute`) | opt `spider_agent`; remote multimodal loop | — (`agent()` removed in v4: external loop or code mode) | [crawl matrix §4](2026-10-07-crawl-capability-matrix.md), [cpy:stagehand-types], [sp:agent], [sh:mig-v3] |
| AI-06 | Browser tools for external agent frameworks | — | — | part (`spider_mcp` tools) | Y `run`/`snapshot`/`screenshot` for Claude Agent SDK, Codex, Cursor, CrewAI, Mastra, Vercel AI SDK, Deep Agents, Eve, Pi, fx (built from the repo, not on npm) | [sp:mcp], [sh:agents], [sh:integrations] |
| AI-07 | MCP server | — | — | Y `spider_mcp` (`spider_scrape`, `spider_crawl`, `spider_links`, `spider_transform`) | Y stdio `stagehand-facade` (from the repo) | [sp:mcp], [sh:cli-agents], [sh:integrations] |
| AI-08 | Accessibility/DOM snapshot with element refs for models | — | — | part (`spider_agent_html` cleaning and diff) | Y `page.snapshot()` (`formattedTree`, `xpathMap`, `urlMap`, iframes) | [sp:agent], [sh:ref-page] |
| AI-09 | Page-declared tools (WebMCP): list, invoke, cancel, add/remove events | — | — | — | Y (`page.tools`, `invoke`, `onToolsAdded/Removed` in 4.2.0) | [sh:webmcp], [sh:ref-webmcp], [sh:release] |
| AI-10 | Model providers, bring-your-own model, per-call override | opt (Stagehand v3 providers) | opt (Pydantic AI providers) | opt OpenAI/OpenAI-compatible, Gemini; per-round model routing in the agent | Y OpenAI, Anthropic, Google, Groq, Cerebras; custom LLM function; per-call model and headers | [cjs:stagehand], [cpy:g:pydantic-ai], [sp:agent], [sp:changelog], [sh:models] |
| AI-11 | Managed model gateway (no provider keys) | — | — | — | cloud Model Gateway | [sh:models] |
| AI-12 | Cache of AI results (skip the model on repeats) | — | — | opt `cache_openai`, `cache_gemini` | cloud server-side cache for act/observe/extract (needs a Browserbase session; on by default in 4.2.0; threshold; `metadata.cache`) | [sp:cargo], [sh:caching], [sh:tarball] |
| AI-13 | Self-healing after a failed action | — | — | opt, documented for `spider_agent` (runtime effect not established by the 2026-10-07 audit) | Y `selfHeal` | [sp:agent], [sp:audit], [sh:ref-stagehand] |
| AI-14 | Token and cost accounting | — | — | Y (`openai_credits_used`, `remote_multimodal_usage` on `Page`) | Y `metadata.usage`, `metrics()` | [sp:page], [sh:ref-stagehand] |
| AI-15 | Web search API for agents | — | — | opt `search` (Serper, Brave, Bing, Tavily) | cloud `browserbase.search` | [sp:cargo], [sh:search] |
| AI-16 | Configure a crawl from a natural-language prompt | — | — | Y `configure_from_prompt` | — | [sp:website] |
| AI-17 | Research synthesis (search + extract + summarize) | — | — | opt `spider_agent` | — (AI SDK cookbook pattern) | [sp:agent] |
| AI-18 | Code tools for the agent (Python/JS interpreters) | — | — | opt `spider_agent` `scripting` | part (`run` executes model-written JS in the extension service worker) | [sp:agent], [sh:agents] |
| AI-19 | Custom system prompt | — | — | Y (remote multimodal config) | Y `systemPrompt` | [sp:website], [sh:ref-stagehand] |

## Security & compliance (SC)

| ID | Capability | Crawlee JS 3.18.2 | Crawlee Py 1.10.4 | Spider 2.53.9 | Stagehand 4.2.0 | Sources |
|---|---|---|---|---|---|---|
| SC-01 | Opt-in robots.txt compliance with a custom user agent | Y `respectRobotsTxtFile` | Y `respect_robots_txt_file` (5xx → disallow, 1.8.3) | Y `with_respect_robots_txt`; opt `robots_cache` | — | [cjs:changelog], [cpy:changelog], [sp:website] |
| SC-02 | Same-hostname scope by default; host re-checked after redirects; sitemap/robots URLs filtered by scope | Y (3.18.1 fixes) | Y (documented) | Y (subdomains/TLD off by default) | n/a | [cjs:changelog], [cpy:g:security], [sp:website] |
| SC-03 | Only `http`/`https` URL schemes allowed | — | Y (1.7.0) | — | n/a | [cpy:g:security] |
| SC-04 | Domain allow/deny policy on browser traffic | part (exclusions at enqueue time only) | part (same) | Y blacklist/whitelist; opt `firewall` | Y `setDomainPolicy` | [cjs:request-storage], [sp:website], [sp:cargo], [sh:ref-context] |
| SC-05 | Guards against resource exhaustion (crawl traps, page size, redirect loops, decompression bombs) | part (depth and request caps) | Y (caps, timeouts, bounded sitemap decompression 1.8.3) | Y `with_max_page_bytes`, `with_max_bytes_allowed`, redirect limit, main-frame navigation cap | part (cloud fetch capped at 5 MB) | [cpy:g:security], [sp:website], [sh:fetch] |
| SC-06 | TLS verification on by default, with explicit opt-out | Y (`ignoreSslErrors`) | Y | Y (`with_danger_accept_invalid_certs`) | Y (`ignoreHTTPSErrors`) | [cjs:http], [cpy:g:security], [sp:website], [sh:browser] |
| SC-07 | Browser sandbox control | part (launch args) | Y (`disable_browser_sandbox`; sandbox on by default) | part (headless defaults to `no_sandbox` since 2.45.19) | Y (`chromiumSandbox`) | [cpy:g:security], [sp:changelog], [sh:browser] |
| SC-08 | Secrets kept out of model prompts | n/a | n/a | — | Y `%var%` variables (the docs warn that values travel with cache requests) | [sh:act], [sh:caching] |
| SC-09 | Isolation of model-written code | n/a | n/a | opt (`scripting` sandbox: cap-std filesystem, network opt-in) | part (`run` code stays out of the host process; docs recommend Browserbase as the isolation boundary) | [sp:agent], [sh:agents] |
| SC-10 | Telemetry off unless explicitly configured | n/a | n/a | n/a | Y (4.2.0) | [sh:release] |

## Console (CN)

| ID | Capability | Crawlee JS 3.18.2 | Crawlee Py 1.10.4 | Spider 2.53.9 | Stagehand 4.2.0 | Sources |
|---|---|---|---|---|---|---|
| CN-01 | Watch a running browser live | part (headful mode) | part (headful mode) | part (`chrome_headed`) | Y local DevTools (`devtools: true`); cloud live view | [cpy:pw], [sp:crate-readme], [sh:browser], [sh:observability] |
| CN-02 | Session recording and replay | — | — | — | cloud (`recordSession`, Browserbase recordings) | [sh:browser], [sh:observability] |
| CN-03 | Run status and progress shown in a UI | part (`setStatusMessage`; displayed by Apify Console, cloud) | part (`status_message_callback`) | — | — | [cjs:basic], [cpy:basic] |
| CN-04 | Hosted dashboard of runs/sessions, filterable by metadata | cloud (Apify Console) | cloud (Apify Console) | cloud (spider.cloud) | cloud (Browserbase sessions, `userMetadata`) | [cjs:deploy], [cpy:deploy], [sp:readme], [sh:observability] |

## Cloud readiness (CL)

| ID | Capability | Crawlee JS 3.18.2 | Crawlee Py 1.10.4 | Spider 2.53.9 | Stagehand 4.2.0 | Sources |
|---|---|---|---|---|---|---|
| CL-01 | Docker images and Dockerfiles in templates | Y | Y (Playwright-pinned base images) | — | — | [cjs:docker], [cpy:changelog] |
| CL-02 | Deployment guides (Apify, AWS Lambda, GCP Cloud Run/Functions) | Y Apify, AWS (browser and Cheerio), GCP | Y Apify, AWS Lambda, Cloud Run, Cloud Run Functions | — | cloud Browserbase Functions (+ Vercel cron example) | [cjs:deploy], [cpy:deploy], [sh:deploy] |
| CL-03 | Crawler served as an HTTP service | Y (web-server guide) | Y (web-server guide) | Y `spider_worker` (HTTP worker/proxy, optional TLS) | cloud (Functions invocation) | [cjs:webserver], [cpy:g:webserver], [sp:worker], [sh:deploy] |
| CL-04 | Several processes or machines sharing one queue | Y (request locking, default since 3.10; parallel-scraping guide) | Y (SQL and Redis storage clients) | opt `decentralized` + `spider_worker` (IO offload; the shipped worker has no Chrome) | — | [cjs:locking], [cjs:parallel], [cpy:g:storage-clients], [sp:crate-readme] |
| CL-05 | Remote browser sessions with regions and keep-alive | v4 `remoteBrowser` | part (Stagehand v3 Browserbase env) | Y remote CDP; cloud Spider Browser (`with_spider_browser`) | Y `browserbase.launch/connect`; cloud: 4 regions, `keepAlive` | [cjs:v4-remote], [cpy:g:stagehand], [sp:website], [sh:browser] |
| CL-06 | Move from local to the vendor cloud by configuration | cloud (Apify SDK `Actor`, a separate package) | cloud (Apify SDK) | cloud `spider_cloud` / `with_spider_cloud_config` | cloud `browserbase.launch` | [cjs:deploy], [cpy:deploy], [sp:crate-readme], [sh:browser] |
| CL-07 | Persistent browser contexts stored in the cloud | — | — | — | cloud (Browserbase contexts) | [sh:user-data], [sh:persisted-login] |

## Version deltas that will move the target

- **Crawlee JS 4.0 (RC on 2026-10-06).** Notable changes in the [v4 upgrade guide][cjs:v4]:
  - **Handler and storage behavior:** storage writes in a request handler become transactional (a handler that throws leaves nothing behind), and repeated `run()` calls no longer empty the queue.
  - **Concurrency and timing:** one `ConcurrencySystem` can cap several crawlers, and timeouts can be set per route.
  - **Pluggable components:** `ISessionPool`, `IBrowserPool`, `IRequestManager` and `IStatistics` can be swapped.
  - **Sessions and proxies:** a session (proxy + cookies + score) becomes the rotation unit, and tiered proxies and rotation counters are removed.
  - **Removed or renamed:** got-scraping becomes opt-in; `globs`/`regexps`/`pseudoUrls` become `include`; `SitemapRequestList` becomes `SitemapRequestLoader`.
  - **Runtime requirements:** ESM only, Node ≥22.13, TypeScript ≥5.8.
  - **New capabilities on `master`:** remote browsers ([`remoteBrowser`][cjs:v4-remote]) and [`@crawlee/otel`][cjs:v4-otel].

  V1 must track 3.18.2 users, but the RC rows (`v4`) show where the reference is heading.
- **Stagehand 4.2.0 (2026-10-08).** It adds page PDF export, `clientOptions` on Browserbase launch/connect, snapshot and locator timeouts, WebMCP add/remove hooks and bounded CDP heartbeats. Telemetry is now off by default, and extension IDs passed to `connect()` are deprecated and ignored ([sh:release]).
- **Crawlee Python 1.10.5** is in beta (1.10.5b3, 2026-10-08), and Stagehand **4.2.1** alphas were published on 2026-10-09/10 ([cpy:pypi], [sh:npm]).
- **Spider `main` after 2.53.9** removed the Spider Cloud *unblocker* endpoint on 2026-10-01 ([sp:main-unblocker]). The next release may therefore drop the `Unblocker` mode listed in ST-10.

## Gaps and uncertainties

1. **"Not found" cells are not proof of absence.** They mean the inspected sources did not show the capability. Python encoding handling (CO-24) was not checked.
2. **Driver-delegated surface.** For Crawlee, every `Y (driver)` cell stands for the whole Playwright (or Puppeteer) page API. Parity for Crawlee browser projects therefore means either a Playwright-compatible page in Automan or a mapped equivalent per call. The Site access area must decide this explicitly; Stagehand v4 shows the cost of a smaller driver (its [Playwright migration page][sh:mig-pw] lists the missing calls).
3. **Cloud-only rows.** Many rows (cache, Model Gateway, fetch/search, CAPTCHA solving, proxies, recordings, contexts, Apify Console, Spider Cloud) exist only in a vendor's paid cloud. The map must decide whether V1 parity means equivalent OSS capability, an adapter to the same vendor, or an explicit exclusion.
4. **Stagehand v3 is still in use through Crawlee.** `@crawlee/stagehand` 3.18.2 targets Stagehand `^3.0.0` and Crawlee Python caps it below 4 (1.9.2 changelog). A Crawlee+Stagehand project may rely on v3-only calls (Python `execute`, v3 `agent`), which this list does not enumerate.
5. **Option exposed ≠ effect proven.** Spider exposes many options and agent modes (planning, self-healing, confidence) whose runtime effect the [2026-10-07 audit][sp:audit] did not establish. Rows quote the documentation, and acceptance tests must check behavior.
6. **Spider bindings lag the crate.** Their feature sets are fixed at build time (Node 0.0.163 from January 2026, Python 0.0.57). A Node or Python Spider user has a smaller surface than the Rust table suggests.
7. **Docs drift.** The Stagehand Playwright migration page still lists `page.pdf()` as having "no equivalent", while 4.2.0 ships it ([sh:mig-pw], [sh:ref-page]). On the earlier cache-default divergence, the 4.2.0 runtime now defaults `cache` to `true` when a Browserbase session exists ([sh:tarball]), which matches the docs.
8. **Unpublished pieces.** The Stagehand agent integrations and the `stagehand-facade` MCP server are built from the repository and are not on npm ([sh:integrations]). `browse` 0.11.1 pins Stagehand 4.1.0, not 4.2.0 ([sh:browse]).
9. **Scope.** Hosted-platform products (Apify Actors/Store, Browserbase platform APIs, the spider.cloud REST API) are listed only where the SDK exposes them. Nothing here was installed or run; it is a static reading of versioned sources and current docs as of 2026-10-10.

## Sources

Crawlee JS 3.18.2 (tag `v3.18.2` unless noted):

[cjs:npm]: https://www.npmjs.com/package/crawlee/v/3.18.2
[cjs:pkg]: https://github.com/apify/crawlee/blob/v3.18.2/packages/crawlee/package.json
[cjs:readme]: https://github.com/apify/crawlee/blob/v3.18.2/README.md
[cjs:changelog]: https://github.com/apify/crawlee/blob/v3.18.2/CHANGELOG.md
[cjs:basic]: https://github.com/apify/crawlee/blob/v3.18.2/packages/basic-crawler/src/internals/basic-crawler.ts
[cjs:http]: https://github.com/apify/crawlee/blob/v3.18.2/packages/http-crawler/src/internals/http-crawler.ts
[cjs:pool]: https://github.com/apify/crawlee/blob/v3.18.2/packages/browser-pool/src/browser-pool.ts
[cjs:launch]: https://github.com/apify/crawlee/blob/v3.18.2/packages/browser-pool/src/launch-context.ts
[cjs:pwutils]: https://github.com/apify/crawlee/blob/v3.18.2/packages/playwright-crawler/src/internals/utils/playwright-utils.ts
[cjs:session]: https://github.com/apify/crawlee/blob/v3.18.2/packages/core/src/session_pool/session_pool.ts
[cjs:proxy]: https://github.com/apify/crawlee/blob/v3.18.2/packages/core/src/proxy_configuration.ts
[cjs:core]: https://github.com/apify/crawlee/blob/v3.18.2/packages/core/src/index.ts
[cjs:storages]: https://github.com/apify/crawlee/blob/v3.18.2/packages/core/src/storages/index.ts
[cjs:utils]: https://github.com/apify/crawlee/blob/v3.18.2/packages/utils/src/index.ts
[cjs:cheerio-utils]: https://github.com/apify/crawlee/blob/v3.18.2/packages/utils/src/internals/cheerio.ts
[cjs:errsnap]: https://github.com/apify/crawlee/blob/v3.18.2/packages/core/src/crawlers/error_snapshotter.ts
[cjs:scaling]: https://github.com/apify/crawlee/blob/v3.18.2/docs/guides/scaling_crawlers.mdx
[cjs:blocking]: https://github.com/apify/crawlee/blob/v3.18.2/docs/guides/avoid_blocking.mdx
[cjs:http-clients]: https://github.com/apify/crawlee/blob/v3.18.2/docs/guides/http-clients.mdx
[cjs:session-guide]: https://github.com/apify/crawlee/blob/v3.18.2/docs/guides/session_management.mdx
[cjs:request-storage]: https://github.com/apify/crawlee/blob/v3.18.2/docs/guides/request_storage.mdx
[cjs:result-storage]: https://github.com/apify/crawlee/blob/v3.18.2/docs/guides/result_storage.mdx
[cjs:configuration]: https://github.com/apify/crawlee/blob/v3.18.2/docs/guides/configuration.mdx
[cjs:docker]: https://github.com/apify/crawlee/blob/v3.18.2/docs/guides/docker_images.mdx
[cjs:webserver]: https://github.com/apify/crawlee/blob/v3.18.2/docs/guides/running-in-web-server/running-in-web-server.mdx
[cjs:parallel]: https://github.com/apify/crawlee/blob/v3.18.2/docs/guides/parallel-scraping/parallel-scraping.mdx
[cjs:locking]: https://github.com/apify/crawlee/blob/v3.18.2/docs/experiments/request_locking.mdx
[cjs:examples]: https://github.com/apify/crawlee/tree/v3.18.2/docs/examples
[cjs:file-download]: https://github.com/apify/crawlee/blob/v3.18.2/docs/examples/file_download.mdx
[cjs:plugins]: https://github.com/apify/crawlee/tree/v3.18.2/docs/examples/crawler-plugins
[cjs:deploy]: https://github.com/apify/crawlee/tree/v3.18.2/docs/deployment
[cjs:cli]: https://github.com/apify/crawlee/tree/v3.18.2/packages/cli/src/commands
[cjs:templates]: https://github.com/apify/crawlee/tree/v3.18.2/packages/templates/templates
[cjs:stagehand]: https://github.com/apify/crawlee/blob/v3.18.2/packages/stagehand-crawler/src/internals/stagehand-crawler.ts
[cjs:stagehand-pkg]: https://github.com/apify/crawlee/blob/v3.18.2/packages/stagehand-crawler/package.json
[cjs:v4]: https://github.com/apify/crawlee/blob/ade1625438ebfb7bcde583d37b25469c1d139035/docs/upgrading/upgrading-v4/upgrading-v4.md
[cjs:v4-remote]: https://github.com/apify/crawlee/blob/master/docs/guides/remote-browser.mdx
[cjs:v4-otel]: https://github.com/apify/crawlee/tree/master/packages/otel

Crawlee Python 1.10.4 (tag `v1.10.4`; guides are the live 1.10 docs):

[cpy:pypi]: https://pypi.org/project/crawlee/1.10.4/
[cpy:pyproject]: https://github.com/apify/crawlee-python/blob/v1.10.4/pyproject.toml
[cpy:changelog]: https://github.com/apify/crawlee-python/blob/v1.10.4/CHANGELOG.md
[cpy:crawlers]: https://github.com/apify/crawlee-python/tree/v1.10.4/src/crawlee/crawlers
[cpy:basic]: https://github.com/apify/crawlee-python/blob/v1.10.4/src/crawlee/crawlers/_basic/_basic_crawler.py
[cpy:pw]: https://github.com/apify/crawlee-python/blob/v1.10.4/src/crawlee/crawlers/_playwright/_playwright_crawler.py
[cpy:pw-ctx]: https://github.com/apify/crawlee-python/blob/v1.10.4/src/crawlee/crawlers/_playwright/_playwright_crawling_context.py
[cpy:pw-pre]: https://github.com/apify/crawlee-python/blob/v1.10.4/src/crawlee/crawlers/_playwright/_playwright_pre_nav_crawling_context.py
[cpy:browsers]: https://github.com/apify/crawlee-python/blob/v1.10.4/src/crawlee/browsers/__init__.py
[cpy:stagehand-types]: https://github.com/apify/crawlee-python/blob/v1.10.4/src/crawlee/browsers/_stagehand_types.py
[cpy:loaders]: https://github.com/apify/crawlee-python/blob/v1.10.4/src/crawlee/request_loaders/__init__.py
[cpy:storage-clients-src]: https://github.com/apify/crawlee-python/tree/v1.10.4/src/crawlee/storage_clients
[cpy:g:throttling]: https://crawlee.dev/python/docs/guides/request-throttling
[cpy:g:pydantic-ai]: https://crawlee.dev/python/docs/guides/pydantic-ai-crawler
[cpy:g:security]: https://crawlee.dev/python/docs/guides/security-of-web-scraping
[cpy:g:warc]: https://crawlee.dev/python/docs/guides/creating-web-archive
[cpy:g:otel]: https://crawlee.dev/python/docs/guides/trace-and-monitor-crawlers
[cpy:g:webserver]: https://crawlee.dev/python/docs/guides/running-in-web-server
[cpy:g:storage-clients]: https://crawlee.dev/python/docs/guides/storage-clients
[cpy:g:http-clients]: https://crawlee.dev/python/docs/guides/http-clients
[cpy:g:blocking]: https://crawlee.dev/python/docs/guides/avoid-blocking
[cpy:g:sessions]: https://crawlee.dev/python/docs/guides/session-management
[cpy:g:errors]: https://crawlee.dev/python/docs/guides/error-handling
[cpy:g:cookies]: https://crawlee.dev/python/docs/guides/cookie-management
[cpy:g:login]: https://crawlee.dev/python/docs/guides/logging-in-with-a-crawler
[cpy:g:loaders]: https://crawlee.dev/python/docs/guides/request-loaders
[cpy:g:scaling]: https://crawlee.dev/python/docs/guides/scaling-crawlers
[cpy:g:router]: https://crawlee.dev/python/docs/guides/request-router
[cpy:g:playwright]: https://crawlee.dev/python/docs/guides/playwright-crawler
[cpy:g:stagehand]: https://crawlee.dev/python/docs/guides/stagehand-crawler
[cpy:g:service-locator]: https://crawlee.dev/python/docs/guides/service-locator
[cpy:examples]: https://crawlee.dev/python/docs/examples
[cpy:deploy]: https://crawlee.dev/python/docs/deployment

Spider 2.53.9 (SHA `e4b53c5b582357e46433fa9ba3d91f930fb67cb4`, verified from the published crate):

[sp:crate]: https://crates.io/crates/spider/2.53.9
[sp:readme]: https://github.com/spider-rs/spider/blob/e4b53c5b582357e46433fa9ba3d91f930fb67cb4/README.md
[sp:crate-readme]: https://github.com/spider-rs/spider/blob/e4b53c5b582357e46433fa9ba3d91f930fb67cb4/spider/README.md
[sp:cargo]: https://github.com/spider-rs/spider/blob/e4b53c5b582357e46433fa9ba3d91f930fb67cb4/spider/Cargo.toml
[sp:website]: https://github.com/spider-rs/spider/blob/e4b53c5b582357e46433fa9ba3d91f930fb67cb4/spider/src/website.rs
[sp:config]: https://github.com/spider-rs/spider/blob/e4b53c5b582357e46433fa9ba3d91f930fb67cb4/spider/src/configuration.rs
[sp:page]: https://github.com/spider-rs/spider/blob/e4b53c5b582357e46433fa9ba3d91f930fb67cb4/spider/src/page.rs
[sp:automation]: https://github.com/spider-rs/spider/blob/e4b53c5b582357e46433fa9ba3d91f930fb67cb4/spider/src/features/chrome_common.rs
[sp:solvers]: https://github.com/spider-rs/spider/blob/e4b53c5b582357e46433fa9ba3d91f930fb67cb4/spider/src/features/solvers.rs
[sp:changelog]: https://github.com/spider-rs/spider/blob/e4b53c5b582357e46433fa9ba3d91f930fb67cb4/CHANGELOG.md
[sp:agent]: https://github.com/spider-rs/spider/blob/e4b53c5b582357e46433fa9ba3d91f930fb67cb4/spider_agent/README.md
[sp:mcp]: https://github.com/spider-rs/spider/blob/e4b53c5b582357e46433fa9ba3d91f930fb67cb4/spider_mcp/README.md
[sp:cli]: https://github.com/spider-rs/spider/blob/e4b53c5b582357e46433fa9ba3d91f930fb67cb4/spider_cli/README.md
[sp:utils]: https://github.com/spider-rs/spider/blob/e4b53c5b582357e46433fa9ba3d91f930fb67cb4/spider_utils/README.md
[sp:worker]: https://github.com/spider-rs/spider/blob/e4b53c5b582357e46433fa9ba3d91f930fb67cb4/spider_worker/README.md
[sp:node]: https://github.com/spider-rs/spider-nodejs
[sp:py]: https://github.com/spider-rs/spider-py
[sp:main-unblocker]: https://github.com/spider-rs/spider/commit/fb0f270d77e19442de59350d9dbbacf52f1804b4
[sp:audit]: 2026-10-07-crawlee-spider-contracts-followup.md

Stagehand 4.2.0 (release commit `2d605b099996e10c3e3ab0dcf293db8d482a2c97`; docs are the live v4 docs as of 2026-10-10):

[sh:release]: https://github.com/browserbase/stagehand/pull/2969
[sh:npm]: https://www.npmjs.com/package/@browserbasehq/stagehand/v/4.2.0
[sh:pypi]: https://pypi.org/project/stagehand/4.2.0/
[sh:go]: https://pkg.go.dev/github.com/browserbase/stagehand/packages/sdk-go/v4@v4.2.0
[sh:tarball]: https://registry.npmjs.org/@browserbasehq/stagehand/-/stagehand-4.2.0.tgz
[sh:install]: https://docs.stagehand.dev/v4/first-steps/installation
[sh:ai-rules]: https://docs.stagehand.dev/v4/first-steps/ai-rules
[sh:act]: https://docs.stagehand.dev/v4/basics/act
[sh:extract]: https://docs.stagehand.dev/v4/basics/extract
[sh:observe]: https://docs.stagehand.dev/v4/basics/observe
[sh:webmcp]: https://docs.stagehand.dev/v4/basics/webmcp
[sh:browser]: https://docs.stagehand.dev/v4/configuration/browser
[sh:logging]: https://docs.stagehand.dev/v4/configuration/logging
[sh:models]: https://docs.stagehand.dev/v4/configuration/models
[sh:observability]: https://docs.stagehand.dev/v4/configuration/observability
[sh:search]: https://docs.stagehand.dev/v4/add-ons/search
[sh:fetch]: https://docs.stagehand.dev/v4/add-ons/fetch
[sh:agents]: https://docs.stagehand.dev/v4/integrations/agent-frameworks/overview
[sh:cli-agents]: https://docs.stagehand.dev/v4/integrations/cli-agents/overview
[sh:caching]: https://docs.stagehand.dev/v4/best-practices/caching
[sh:tabs]: https://docs.stagehand.dev/v4/best-practices/using-multiple-tabs
[sh:user-data]: https://docs.stagehand.dev/v4/best-practices/user-data
[sh:deploy]: https://docs.stagehand.dev/v4/best-practices/deployments
[sh:persisted-login]: https://docs.stagehand.dev/v4/cookbooks/persisted-login
[sh:mig-v3]: https://docs.stagehand.dev/v4/migrations/v3
[sh:mig-pw]: https://docs.stagehand.dev/v4/migrations/playwright
[sh:ref-stagehand]: https://docs.stagehand.dev/v4/reference/stagehand
[sh:ref-context]: https://docs.stagehand.dev/v4/reference/context
[sh:ref-page]: https://docs.stagehand.dev/v4/reference/page
[sh:ref-locator]: https://docs.stagehand.dev/v4/reference/locator
[sh:ref-response]: https://docs.stagehand.dev/v4/reference/response
[sh:ref-webmcp]: https://docs.stagehand.dev/v4/reference/webmcp
[sh:ref-clipboard]: https://docs.stagehand.dev/v4/reference/clipboard
[sh:integrations]: https://github.com/browserbase/stagehand/blob/2d605b099996e10c3e3ab0dcf293db8d482a2c97/packages/integrations/README.md
[sh:browse]: https://github.com/browserbase/stagehand/blob/2d605b099996e10c3e3ab0dcf293db8d482a2c97/packages/cli/README.md

Table cells cite these sources by label, for example `[cjs:basic]`. Registry facts were read on 2026-10-10: Crawlee JS dist-tags `latest` 3.18.2, `rc` 4.0.0-rc.1; PyPI `crawlee` 1.10.4; crates.io `spider` max stable 2.53.9; npm `@browserbasehq/stagehand` `latest` 4.2.0; PyPI `stagehand` 4.2.0; Go proxy `packages/sdk-go/v4` latest v4.2.0; npm `@spider-rs/spider-rs` 0.0.163; PyPI `spider_rs` 0.0.57; npm `browse` 0.11.1.
