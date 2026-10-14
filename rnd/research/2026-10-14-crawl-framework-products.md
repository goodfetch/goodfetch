# Crawl-framework products: packaging, and how they plug in automation libraries

Research of **October 14, 2026** for ticket [#449](https://github.com/goodfetch/automan/issues/449). Question: how do crawl frameworks, Foxhound's direct competitors, package themselves, and how do they let users plug in browser-automation libraries, their own or third-party? It feeds the tickets that decide the Collie/Foxhound product boundary and how Foxhound works with Playwright, Puppeteer, Stagehand and Browser Use.

**Sources.** Package registries (npm, PyPI, crates.io, the Go module proxy) read through their JSON APIs on 2026-10-14; source code at release tags through the GitHub API, or from the published crate archives; official docs; GitHub issues and pull requests, with comment and reaction counts read on 2026-10-14. It builds on the October 7 notes ([engines](2026-10-07-crawl-engines.md), [Crawlee/Spider contracts](2026-10-07-crawlee-spider-contracts-followup.md), [capability matrix](2026-10-07-crawl-capability-matrix.md)) and the October 10 notes on [frontier and sessions](https://github.com/goodfetch/automan/blob/research/frontier-sessions-competitors/rnd/research/2026-10-10-frontier-sessions-competitors.md) and on [storage and sinks](https://github.com/goodfetch/automan/blob/research/storage-sinks-competitors/rnd/research/2026-10-10-storage-sinks-competitors.md). Facts reused from them were re-checked against the registries and the tagged sources where this note relies on them.

**Measured versus cited.** Install sizes and package counts were measured, not quoted (§5): npm trees resolved with `npm install --package-lock-only --ignore-scripts` and summed from each package's registry `dist.unpackedSize`; Python trees resolved with `uv pip compile` for CPython 3.12 on x86_64 manylinux and summed from the size of the wheel PyPI serves for that platform. Nothing was installed or run. Measured facts are marked **M**; everything else is cited and marked **C**; a reading of code or docs that no source states is marked **inference**. Like everything in `rnd/`, this note informs a decision; it does not make one.

## Versions inspected

| Product | Version (date) | Where read |
|---|---|---|
| Crawlee JS | `crawlee` **3.18.2** (2026-09-29) with 17 `@crawlee/*` packages; **4.0.0-rc.1** (2026-10-06) with 18, plus `@crawlee/fs-storage-native` 0.2.2 | [npm](https://registry.npmjs.org/crawlee), tags [v3.18.2](https://github.com/apify/crawlee/tree/v3.18.2), [v4.0.0-rc.1](https://github.com/apify/crawlee/tree/v4.0.0-rc.1) |
| Crawlee Python | **1.10.4** (2026-10-06) | [PyPI](https://pypi.org/project/crawlee/1.10.4/), tag [v1.10.4](https://github.com/apify/crawlee-python/tree/v1.10.4) |
| Apify SDK | JS `apify` **3.7.2** (2026-05-11; 4.0.0-rc.0 on the `rc` tag), Python `apify` **4.1.0** (2026-10-08) | [npm](https://registry.npmjs.org/apify), [PyPI](https://pypi.org/project/apify/) |
| Spider-rs | `spider`, `spider_cli`, `spider_agent`, `spider_mcp`, `spider_utils`, `spider_worker` **2.53.9** (2026-09-05); `chromey` **2.58.2** (2026-09-12); `main` says 2.54.0, unpublished | [crates.io](https://crates.io/crates/spider), crate archives |
| Spider bindings | npm `@spider-rs/spider-rs` **0.0.163** (2026-01-24); PyPI `spider_rs` **0.0.57** (2025-01-28) | [npm](https://registry.npmjs.org/@spider-rs/spider-rs), [PyPI](https://pypi.org/project/spider_rs/) |
| Scrapy | **2.19.0** (2026-09-10); `scrapy-playwright` **0.0.48** (2026-07-10); `scrapy-splash` 0.11.1 (2025-02-11); `scrapy-zyte-api` 0.36.0 (2026-08-20) | [PyPI](https://pypi.org/project/Scrapy/), tag [scrapy-playwright v0.0.48](https://github.com/scrapy-plugins/scrapy-playwright/tree/v0.0.48) |
| Firecrawl | last GitHub release **v2.11.0** (2026-06-19); tags up to v2.11.518; `main` at [`f4fdf7f`](https://github.com/firecrawl/firecrawl/tree/f4fdf7f300415180f45272e893918dbfa3649073) (2026-10-13); `firecrawl-py` 4.50.0 (2026-10-08) | GitHub API, [PyPI](https://pypi.org/project/firecrawl-py/) |
| Crawl4AI | **0.9.5** (2026-10-14) | [PyPI](https://pypi.org/project/crawl4ai/0.9.5/), tag [v0.9.5](https://github.com/unclecode/crawl4ai/tree/v0.9.5) |
| Automation libraries | Playwright **1.64.1** (2026-10-14); Puppeteer **25.13.0** (2026-10-08); Stagehand npm **4.2.1** (2026-10-11), `v3-latest` 3.7.3; Stagehand PyPI 4.2.1 (2026-10-10); Browser Use **0.13.11** (2026-10-07) | npm, PyPI |
| Brief coverage | Katana **v1.8.0** (2026-10-05); Colly **v2.3.0** (Go proxy, 2025-12-04); Heritrix **3.18.0** (2026-10-06); StormCrawler 3.7.0 (2026-07-29); Scrapling 0.4.15 (2026-08-23); Browsertrix Crawler 1.15.0 (2026-10-02) | Go proxy, GitHub releases |

---

## 1. Summary

- **Every crawl framework with a browser tier leaves the browser library's version to the user, except Crawl4AI.**
  - Crawlee JS declares `playwright` and `puppeteer` as optional peer dependencies with range `*`, on the `crawlee` meta-package and on every browser package, in 3.18.2 and 4.0.0-rc.1 ([npm `crawlee`](https://registry.npmjs.org/crawlee/latest), [npm `@crawlee/playwright`](https://registry.npmjs.org/@crawlee/playwright/4.0.0-rc.1)) **C**. Its own CI pins Playwright 1.63.0 and Puppeteer 25.12.0 ([pnpm catalog](https://github.com/apify/crawlee/blob/v4.0.0-rc.1/pnpm-workspace.yaml), [root `package.json`](https://github.com/apify/crawlee/blob/v4.0.0-rc.1/package.json)) **C**.
  - The `*` range is a reaction: Crawlee 3.1.4 capped the peer at `>= 1.21.x <= 1.27.x`, which produced `ERESOLVE` errors and the issue "Crawlee should not dictate playwright version" ([crawlee#1723](https://github.com/apify/crawlee/issues/1723), 2022; [package.json at that commit](https://github.com/apify/crawlee/blob/6781f121ccb413958d09c1288a1e1e9463783c3c/packages/playwright-crawler/package.json)) **C**.
  - Crawlee Python and scrapy-playwright give Playwright only a lower bound (`playwright>=1.27.0`, `playwright>=1.40`) **C**. Spider-rs compiles its browser in through Cargo features (`chrome` for its own CDP driver Chromey, `webdriver` for thirtyfour), neither on by default **C**.
  - Crawl4AI's core install requires Playwright **and** Patchright **and** playwright-stealth, plus an exact pin on its own LiteLLM fork (`unclecode-litellm==1.81.13`): 95 packages, 207 MB of wheels **M**.
- **The handler receives the crawler's page as a library object, never as a protocol endpoint.**
  - Crawlee and Crawl4AI hand the handler a Playwright (or Puppeteer) `Page`; scrapy-playwright does when a request sets `playwright_include_page`; Spider hands a Chromey `Page` when built with `chrome_store_page` **C**.
  - A library that wants a CDP URL (Browser Use, Stagehand v4) has nothing to attach to by default: Playwright launches Chromium over `--remote-debugging-pipe` and reports a WebSocket only when the user adds `--remote-debugging-port` to the launch arguments ([chromium.ts @ v1.64.1](https://github.com/microsoft/playwright/blob/v1.64.1/packages/playwright-core/src/server/chromium/chromium.ts#L355-L372)) **C**.
  - Crawl4AI is the exception in both directions: its managed browser is launched with a debugging port, and `cdp_url` with `browser_context_id` and `target_id` makes it reuse a page another tool opened ([async_configs.py @ v0.9.5](https://github.com/unclecode/crawl4ai/blob/v0.9.5/crawl4ai/async_configs.py#L942-L976)) **C**.
- **The one first-party third-party integration, Crawlee's Stagehand crawler, inverts the relationship: Stagehand launches the browser and the crawler attaches.**
  - `StagehandCrawler` (JS and Python) lets Stagehand launch Chromium and connects Playwright to it over CDP **C**.
  - The guide lists the cost: Chromium only, "low-level browser properties cannot be modified", only `headless`, `args`, `executablePath`, `proxy` and `viewport` pass through, custom context options not fully supported ([guide @ v4.0.0-rc.1](https://github.com/apify/crawlee/blob/v4.0.0-rc.1/docs/guides/stagehand_crawler.mdx)) **C**. Python adds no incognito pages and no canvas, WebGL or screen fingerprint ([Python guide](https://crawlee.dev/python/docs/guides/stagehand-crawler)) **C**. In v4, `StagehandCrawler` rejects `remoteBrowser` ([upgrading guide @ v4.0.0-rc.1](https://github.com/apify/crawlee/blob/v4.0.0-rc.1/docs/upgrading/upgrading_v4.md)) **C**.
- **Both Crawlee integrations are held at Stagehand v3 while Stagehand ships v4, and v4 refuses foreign pages.**
  - `@crawlee/stagehand` 4.0.0-rc.1 still peers `@browserbasehq/stagehand ^3.0.0`; `crawlee[stagehand]` 1.10.4 requires `stagehand<4.0.0,>=3.19.5`, which resolves to 3.23.0 while PyPI's latest is 4.2.1 **C/M**.
  - Stagehand v4: "there is no interop"; a Playwright page "won't type-check as `act()`'s `page` option" ([Playwright migration](https://docs.stagehand.dev/v4/migrations/playwright.md)) **C**.
  - Attaching to a running browser (`localBrowser.connect({ cdpUrl })`) loads Stagehand's extension, which needs `--enable-unsafe-extension-debugging` on the browser's command line and a shared filesystem ([browser configuration](https://docs.stagehand.dev/v4/configuration/browser.md)) **C**.
- **Crawlee v4 adds the seam a third-party browser needs, on the browser-pool side.**
  - Crawlers accept any `IBrowserPool` (`newPage`, `closePage`, `extractPageState`, `injectPageState`); a pool passed in is borrowed and never destroyed by the crawler ([upgrading guide](https://github.com/apify/crawlee/blob/v4.0.0-rc.1/docs/upgrading/upgrading_v4.md)) **C**.
  - `remoteBrowser` or a `RemoteBrowserPool` connects to Browserbase, Browserless, Steel "or any service that exposes a WebSocket/CDP endpoint", with `maxOpenBrowsers` and a session-release callback; there, `headless` and `launchOptions` do not apply, incognito is forced and `userDataDir` has no effect ([remote browser guide](https://github.com/apify/crawlee/blob/v4.0.0-rc.1/docs/guides/remote_browser.mdx)) **C**.
  - The 2023 request, [crawlee#1822](https://github.com/apify/crawlee/issues/1822), is the most-reacted issue found on browser integration (24 reactions) **C**. Crawlee Python has the same request open ([crawlee-python#1743](https://github.com/apify/crawlee-python/issues/1743)) **C**.
- **scrapy-playwright 0.0.48 made the driver pluggable.** `PLAYWRIGHT_BROWSER_PROVIDER` is "an extension point for integrating third-party drivers that expose Playwright-compatible `Browser`/`BrowserContext`/`Page` objects (e.g. patchright, camoufox) without changing the handler" ([README @ v0.0.48](https://github.com/scrapy-plugins/scrapy-playwright/blob/v0.0.48/README.md), [PR #385](https://github.com/scrapy-plugins/scrapy-playwright/pull/385)) **C**. Its known issues: no per-request proxy, and Scrapy's `headers_received` and `bytes_received` signals do not fire (same README) **C**.
- **Packaging takes three shapes.**
  - **A meta-package over lockstep packages** (Crawlee JS): Lerna in fixed mode publishes every `@crawlee/*` package at one version within minutes **M**; v4 moves JSDOM and LinkeDOM to their own repositories ([crawlee#4113](https://github.com/apify/crawlee/issues/4113)) **C**.
  - **One distribution with extras**: Crawlee Python (14 extras), Crawl4AI, Scrapling **C**.
  - **One crate with features**: Spider declares **129** Cargo features across 6 crates released together at 2.53.9, with its CDP driver Chromey on its own numbers **M**. Its Node and Python bindings are versioned apart and lag: Node 0.0.163 (2026-01-24) declares no platform packages where 0.0.157 declared 14; Python's last release is 0.0.57 (2025-01-28) **M**.
- **The open-source cores ship the crawl machinery; the clouds keep scheduling, the UI, durable results and the hard fetch tier** (§8).
  - Crawlee, Spider and Scrapy ship a Frontier, per-domain limits and local result storage. None ships a Politeness store shared across machines; Crawlee Python's Redis and SQL clients share the queue, not the pacing ([ADR 0053](../../docs/adr/0053-politeness-is-shared-by-every-run-of-a-user-and-across-machines-through-a-store.md)) **C**.
  - Schedules and a web UI are Apify, Zyte, Spider Cloud, Firecrawl or Crawl4AI Cloud features. The exceptions are Spider's `cron` feature, Scrapyd's scheduling API, Crawl4AI's Docker playground and Heritrix's web UI **C**.
  - Firecrawl's self-hosted stack has no browser actions: only its Fire-engine engines declare `actions: true`, and Fire-engine is "not included" ([engines/index.ts @ f4fdf7f](https://github.com/firecrawl/firecrawl/blob/f4fdf7f300415180f45272e893918dbfa3649073/apps/api/src/scraper/scrapeURL/engines/index.ts), [self-host guide](https://docs.firecrawl.dev/contributing/self-host)) **C**.
- **The open-source libraries now funnel into their clouds from inside.**
  - Spider's `spider_cloud` feature, on by default in `spider_cli` and `spider_mcp`, routes fetches to Spider Cloud; its `Smart` mode "falls back to `/unblocker` when it detects bot protection" ([configuration.rs, crate 2.53.9](https://docs.rs/crate/spider/2.53.9/source/src/configuration.rs)) **C**.
  - Crawl4AI 0.9.5 prints a "Crawl4AI Cloud is live" banner at setup, when the first crawler starts (daily until 2027-01-01, then once per version) and when a crawl is blocked by anti-bot protection; `CRAWL4AI_NO_CLOUD_NOTICE=1` turns it off ([cloud_notice.py @ v0.9.5](https://github.com/unclecode/crawl4ai/blob/v0.9.5/crawl4ai/cloud_notice.py)) **C**.
- **Clouds also sell the raw browser for any library.** Spider Cloud (`WS /v1/browser`, "Cloud browser via CDP over WebSocket"), Firecrawl's Browser Sandbox (`POST /v2/interact`; each session returns a `cdpUrl`; Playwright and agent-browser preinstalled; 20 concurrent sessions) and Browser Use Cloud each expose a CDP endpoint ([Spider API](https://spider.cloud/docs/api), [Firecrawl Browser](https://docs.firecrawl.dev/features/browser), [Browser Use](https://docs.browser-use.com/cloud/browser/playwright-puppeteer-selenium.md)) **C**.

---

## 2. Packaging: what is core and what is optional

### Crawlee JS

| | 3.18.2 | 4.0.0-rc.1 |
|---|---|---|
| Meta-package `crawlee` depends on | `@crawlee/cli`, `core`, `http`, `basic`, `jsdom`, `utils`, `browser`, `cheerio`, `linkedom`, `puppeteer`, `playwright`, `browser-pool`, `memory-storage` | `cli`, `core`, `http`, `basic`, `utils`, `browser`, `cheerio`, `puppeteer`, `fs-storage`, `playwright`, `impit-client` |
| Optional peers of `crawlee` | `puppeteer: *`, `playwright: *`, `idcac-playwright: *` | `puppeteer: *`, `playwright: *` |
| Browser packages' peers | `@crawlee/browser`, `browser-pool`: `playwright: *`, `puppeteer: *` (optional); `@crawlee/playwright` adds `idcac-playwright ^0.2.0` | same, without `idcac-playwright` |
| `@crawlee/stagehand` peers | `@browserbasehq/stagehand ^3.0.0`, `playwright: *`, `zod ^3 \|\| ^4` | `@browserbasehq/stagehand ^3.0.0`, `playwright: *` |
| Default HTTP client | `got-scraping` (a hard dependency of `@crawlee/core`) | `impit` through `@crawlee/impit-client`; `got-scraping` moves to opt-in `@crawlee/got-scraping-client`; without `impit`, a plain `fetch` client "with a logged warning (no proxy support or impersonation)" |

Sources: npm manifests of each package at both versions ([`crawlee` 3.18.2](https://registry.npmjs.org/crawlee/3.18.2), [`crawlee` 4.0.0-rc.1](https://registry.npmjs.org/crawlee/4.0.0-rc.1), [`@crawlee/stagehand` 4.0.0-rc.1](https://registry.npmjs.org/@crawlee/stagehand/4.0.0-rc.1)), [upgrading guide @ v4.0.0-rc.1](https://github.com/apify/crawlee/blob/v4.0.0-rc.1/docs/upgrading/upgrading_v4.md) **C**.

What v4 moves out of the default install, and why:
- **JSDOM and LinkeDOM leave the monorepo.** "`jsdom` and `linkedom` are heavy dependencies that every workspace install and CI run has to pay for, and the two crawlers are rarely used yet tied to the release cadence of everything else"; they become separate repositories with `@crawlee/http` as a peer and keep their npm names ([crawlee#4113](https://github.com/apify/crawlee/issues/4113), [#4119](https://github.com/apify/crawlee/pull/4119)) **C**. The parser becomes a `parser` option of a `DomCrawler` ([#4104](https://github.com/apify/crawlee/pull/4104)) **C**.
- **`idcac-playwright` stops being a dependency** ([#3977](https://github.com/apify/crawlee/pull/3977)); the cookie-modal helper is replaced by a hook built on `@duckduckgo/autoconsent` ([#4044](https://github.com/apify/crawlee/pull/4044)) **C**.
- **`@crawlee/browser-pool` is no longer re-exported from `crawlee`**: code that builds its own pool imports it directly; the pool factories (`playwrightBrowserPool()`, `puppeteerBrowserPool()`, `stagehandBrowserPool()`) stay in the crawler packages ([upgrading guide](https://github.com/apify/crawlee/blob/v4.0.0-rc.1/docs/upgrading/upgrading_v4.md)) **C**.
- **Native binaries arrive.** `impit` and `@crawlee/fs-storage-native` ship one binary per platform as `optionalDependencies`, so `npm install --omit=optional` "breaks native dependencies" (same guide) **C**.
- **The Apify SDK is being decoupled.** `apify` depends on `@crawlee/core` "only because of the storage classes and a handful of runtime pieces around them"; the plan is a shared package that both depend on ([crawlee#4101](https://github.com/apify/crawlee/issues/4101)) **C**. Today `apify` 3.7.2 depends on `@crawlee/core`, `@crawlee/types` and `@crawlee/utils` `^3.14.1` ([npm](https://registry.npmjs.org/apify/3.7.2)) **C**.

### Crawlee Python

One distribution, `crawlee` 1.10.4, with a small core (`impit`, `pydantic`, `protego`, `tldextract`, `psutil` and others) and 14 extras: `adaptive-crawler`, `beautifulsoup`, `cli`, `curl-impersonate`, `httpx`, `otel`, `parsel`, `playwright`, `pydantic-ai`, `redis`, `sql-mysql`, `sql-postgres`, `sql-sqlite`, `stagehand`, plus `all` ([PyPI metadata](https://pypi.org/pypi/crawlee/1.10.4/json)) **C**. The browser extras set only lower bounds (`playwright>=1.27.0`), except Stagehand (`stagehand<4.0.0,>=3.19.5`) **C**. 1.10 adds an experimental `PydanticAiCrawler`: an HTTP crawler whose `context.extract(Model)` asks a Pydantic AI model for a validated instance, defaulting to `openai:gpt-5.4-nano`, with an alternative extractor that caches CSS selectors ([`_pydantic_ai_crawler.py` @ v1.10.4](https://github.com/apify/crawlee-python/blob/v1.10.4/src/crawlee/crawlers/_pydantic_ai/_pydantic_ai_crawler.py)) **C**. The Apify SDK for Python requires `crawlee>=1.8.0,<2.0.0,!=1.10.1` ([PyPI `apify` 4.1.0](https://pypi.org/pypi/apify/4.1.0/json)) **C**.

### Apify platform

Crawlee runs anywhere; the Apify SDK (`Actor.init()`, `Actor.pushData()` and the like) binds a Crawlee program to the platform. The platform's parts, per its docs: **Actors** ("serverless cloud programs", Docker images), **Storage** (datasets, key-value stores, request queues), **Proxy**, **Schedules** (cron expressions, time zones, up to 10 Actors and 10 tasks per schedule, 10-second minimum interval), **Console** (the web UI), **Integrations**, **Store** (a marketplace of Actors) and **Monitoring** ([platform overview](https://docs.apify.com/platform), [Schedules](https://docs.apify.com/platform/schedules)) **C**. Export formats, webhook retries and the missing database sinks are in the [storage and sinks note](https://github.com/goodfetch/automan/blob/research/storage-sinks-competitors/rnd/research/2026-10-10-storage-sinks-competitors.md).

### Spider-rs

- **Crates.** `spider` (engine), `spider_cli`, `spider_agent` (LLM automation), `spider_mcp` (MCP server), `spider_utils`, `spider_worker` (HTTP worker for the `decentralized` mode), all 2.53.9 on 2026-09-05; `chromey`, the CDP driver forked from chromiumoxide, at 2.58.2; satellites on their own numbers: `spider_fingerprint` 2.39.0, `spider_firewall` 2.42.0, `spider_network_blocker` 0.0.86, `spider_transformations` 2.39.13 ([crates.io API](https://crates.io/api/v1/crates/spider)) **M**.
- **Features.** The engine crate declares **129** Cargo features; its default set is `basic`, `io_uring`, `tcp_fastopen`, `splice`, `numa`, `zero_copy`: no browser, no disk, no cache ([Cargo.toml, crate 2.53.9](https://docs.rs/crate/spider/2.53.9/source/Cargo.toml.orig)) **M**.
  - Browser: `chrome` (Chromey), `smart` (`chrome` + `chrome_intercept`), `chrome_store_page`, `chrome_stealth`, `chrome_headed`, `adblock`, `firewall`, and `webdriver` with `webdriver_chrome`, `webdriver_firefox`, `webdriver_edge` (thirtyfour) **C**.
  - Other tiers: `wreq` (a TLS-impersonating client), `cache`, `cache_mem`, `cache_chrome_hybrid`, `disk` (SQLite through sqlx), `cron`, `warc`, `openai`, `gemini`, `agent`, `spider_cloud` **C**.
  - Chrome is found on the machine (`which`, `home`); Chromey's download `fetcher` is a non-default feature (Chromey Cargo.toml, crate 2.58.2) **C**.
- **CLI and MCP defaults.** `spider_cli` defaults to `status_code`, `chrome`, `warc`, `spider_cloud`; `spider_mcp` to `chrome`, `spider_cloud`; `spider_agent` to nothing ([crate archives 2.53.9](https://docs.rs/crate/spider_cli/2.53.9/source/Cargo.toml)) **M**.
- **Calls made without their feature are silent no-ops.** `with_chrome_connection` "does nothing if the `chrome` is not enabled"; the same `#[cfg(not(feature))]` twin exists for `with_automation_scripts`, `with_stealth`, `with_fingerprint`, `with_cron`, `with_webdriver` and `with_spider_cloud` ([configuration.rs, crate 2.53.9](https://docs.rs/crate/spider/2.53.9/source/src/configuration.rs)) **C**. The October 7 note found the same for `crawl_smart()` without `smart` ([contracts follow-up §6](2026-10-07-crawlee-spider-contracts-followup.md)).
- **Bindings.** `@spider-rs/spider-rs` (napi-rs) and `spider_rs` (PyO3) each depend on `spider = "2"` with their own feature list; the Python one enables `chrome`, `smart`, `cache` and `openai` among others ([spider-nodejs Cargo.toml](https://github.com/spider-rs/spider-nodejs/blob/main/Cargo.toml), [spider-py Cargo.toml](https://github.com/spider-rs/spider-py/blob/main/Cargo.toml)) **C**. Their version numbers are unrelated to the crate's, and they lag (above) **M**. The cloud has its own clients, `spider-client` on PyPI (0.1.93) and `@spider-cloud/spider-client` on npm (0.2.5) **M**.

### Scrapy

- **Core.** `scrapy` 2.19.0: 23 runtime requirements around Twisted and lxml, no browser ([PyPI](https://pypi.org/pypi/scrapy/2.19.0/json)) **C**.
- **Browser layer.** Download-handler plugins in separate distributions, each versioned on its own **C**:
  - `scrapy-playwright` 0.0.48 (`playwright>=1.40`), under the `scrapy-plugins` organization;
  - `scrapy-splash` 0.11.1, a client of the Splash rendering service, whose repository was last pushed on 2024-08-02 ([GitHub](https://github.com/scrapinghub/splash));
  - `scrapy-selenium` 0.0.7, from 2019;
  - `scrapy-camoufox` 0.0.12 (2025-01-16, `camoufox<0.5.0`);
  - Zyte's `scrapy-zyte-api` 0.36.0, which sends requests to the paid Zyte API for rendering and actions.
- **TLS impersonation** is another plugin, `scrapy-impersonate` 1.9.0 (curl-cffi) **C**.
- **Running it.** Scrapyd 1.6.0 (2025-07-22) is the self-hosted job server; Zyte's Scrapy Cloud runs jobs with 7-day data retention and 1-hour jobs on the free plan, and 120 days, unlimited run time and scheduling for paid units at $9 a month ([Scrapy Cloud pricing](https://docs.zyte.com/scrapy-cloud/pricing.md)) **C**.

### Firecrawl

A service, not a library: an AGPL-3.0 API server run with Docker Compose (API and workers, a Playwright service, Redis, RabbitMQ, NuQ PostgreSQL and an optional FoundationDB queue backend), plus thin SDKs (`firecrawl-py` 4.50.0: 26 packages, 4.4 MB of wheels) **C/M**. The self-host baseline is "bundled Playwright with basic fetch fallback", no model provider and no authentication ([SELF_HOST.md @ f4fdf7f](https://github.com/firecrawl/firecrawl/blob/f4fdf7f300415180f45272e893918dbfa3649073/SELF_HOST.md)) **C**. Fire-engine, Agent, Browser and "specialized formats" need Firecrawl Cloud or an external service ([self-host guide](https://docs.firecrawl.dev/contributing/self-host)) **C**.

### Crawl4AI

One distribution whose core already includes both browser drivers and LiteLLM (above); extras add PDF, torch, transformers and Selenium (`sync`) ([PyPI](https://pypi.org/pypi/crawl4ai/0.9.5/json)) **C**. The Docker server adds a `/playground` UI, an MCP endpoint over SSE and WebSocket, a job queue with webhooks and a monitor ([deploy/docker README @ v0.9.5](https://github.com/unclecode/crawl4ai/blob/v0.9.5/deploy/docker/README.md)) **C**. Crawl4AI Cloud (soft launch) sells `/scrape`, `/extract`, `/search`, an experimental `/answer` and a beta `/serp`, with "queues, jobs, retries, and proxies" handled, a hosted MCP endpoint and a key console, priced in credits ([crawl4ai.com](https://crawl4ai.com/)); the library's `crwl cloud` command uploads browser profiles to it ([cloud/cli.py @ v0.9.5](https://github.com/unclecode/crawl4ai/blob/v0.9.5/crawl4ai/cloud/cli.py)) **C**.

### Others, briefly

- **Scrapling** 0.4.15: a parser-only core; the `fetchers` extra adds curl_cffi, Playwright, Patchright and browserforge, the `ai` extra an MCP server ([PyPI](https://pypi.org/pypi/scrapling/0.4.15/json)) **C**.
- **Katana** v1.8.0: a Go CLI and library; headless crawling through go-rod is "experimental", and `-chrome-ws-url` uses a browser "launched elsewhere with the debugger listening at this URL" ([README @ v1.8.0](https://github.com/projectdiscovery/katana/blob/v1.8.0/README.md)) **C**.
- **Colly** v2.3.0: HTTP only, no browser layer ([October 7 matrix](2026-10-07-crawl-capability-matrix.md)) **C**.
- **Heritrix** 3.18.0: `BrowserProcessor`, added in 3.10.0, drives a local Chrome or Firefox over WebDriver BiDi and runs pluggable Java `Behavior`s (scroll, extract links, paginate since 3.16.0); it was "working for small crawls but needs more robust error handling" at introduction, and 3.17.0 added browser restarts ([CHANGELOG](https://github.com/internetarchive/heritrix3/blob/master/CHANGELOG.md)) **C**.
- **Browsertrix Crawler** 1.15.0: a Docker-packaged archival crawler on `puppeteer-core` ^25.11.0, `@puppeteer/replay` and `browsertrix-behaviors` ([package.json @ v1.15.0](https://github.com/webrecorder/browsertrix-crawler/blob/v1.15.0/package.json)) **C**.
- **StormCrawler** 3.7.0: an optional Playwright module, reaching a local, CDP or remote Playwright browser ([October 7 matrix](2026-10-07-crawl-capability-matrix.md)) **C**.

---

## 3. How the browser layer is chosen and swapped

| Product | Unit of choice | First-party drivers | Swap mechanism |
|---|---|---|---|
| Crawlee JS | crawler class + browser plugin | Playwright (Chromium, Firefox, WebKit, Camoufox through `camoufox-js`), Puppeteer, Stagehand v3 | `PlaywrightCrawler` / `PuppeteerCrawler` / `StagehandCrawler`; `launchContext.launcher` takes a Playwright-compatible launcher; v4: any `IBrowserPool`, or `remoteBrowser` / `RemoteBrowserPool` over CDP or Playwright's WebSocket protocol |
| Crawlee Python | crawler class + `BrowserPlugin` | Playwright, Stagehand v3 (REST SDK) | `PlaywrightCrawler(browser_type=…)`, `StagehandCrawler`, a custom browser plugin; no remote-browser option ([#1743](https://github.com/apify/crawlee-python/issues/1743)) |
| Spider | Cargo feature at compile time | Chromey (CDP), thirtyfour (WebDriver) | `chrome` / `smart` / `webdriver_*` features; `with_chrome_connection(url)` or `with_chrome_connections(urls)` (failover) for a running Chrome |
| Scrapy | download handler per scheme | none in core; scrapy-playwright, scrapy-splash, scrapy-zyte-api | `DOWNLOAD_HANDLERS`; in scrapy-playwright, `PLAYWRIGHT_BROWSER_TYPE`, `PLAYWRIGHT_CDP_URL`, `PLAYWRIGHT_CONNECT_URL`, `PLAYWRIGHT_BROWSER_PROVIDER` |
| Crawl4AI | `BrowserConfig` + crawler strategy + browser adapter | Playwright, Patchright (`UndetectedAdapter`), playwright-stealth (`enable_stealth`) | `browser_mode`: `dedicated` (default), `builtin`, `cdp` with `cdp_url`, `docker`; a custom `AsyncCrawlerStrategy` |
| Firecrawl | engine registry, server side | `fetch`, `playwright`, Fire-engine (`chrome-cdp`, `tlsclient`, stealth variants) | engines chosen by the features a request needs; Fire-engine only as a separate service |
| Katana | CLI mode | go-rod | `-headless`, `-system-chrome`, `-chrome-ws-url` |
| Heritrix | processor in the crawl chain | WebDriver BiDi | `BrowserProcessor` bean with pluggable `Behavior`s |

Sources: [Crawlee remote browsers @ v4.0.0-rc.1](https://github.com/apify/crawlee/blob/v4.0.0-rc.1/docs/guides/remote_browser.mdx); [Crawlee Python browsers @ v1.10.4](https://github.com/apify/crawlee-python/tree/v1.10.4/src/crawlee/browsers); Spider [Cargo.toml](https://docs.rs/crate/spider/2.53.9/source/Cargo.toml.orig) and [configuration.rs](https://docs.rs/crate/spider/2.53.9/source/src/configuration.rs); [scrapy-playwright README @ v0.0.48](https://github.com/scrapy-plugins/scrapy-playwright/blob/v0.0.48/README.md); Crawl4AI [async_configs.py](https://github.com/unclecode/crawl4ai/blob/v0.9.5/crawl4ai/async_configs.py#L942-L976), [browser_adapter.py](https://github.com/unclecode/crawl4ai/blob/v0.9.5/crawl4ai/browser_adapter.py) and [async_crawler_strategy.py](https://github.com/unclecode/crawl4ai/blob/v0.9.5/crawl4ai/async_crawler_strategy.py) @ v0.9.5; [Firecrawl engines @ f4fdf7f](https://github.com/firecrawl/firecrawl/blob/f4fdf7f300415180f45272e893918dbfa3649073/apps/api/src/scraper/scrapeURL/engines/index.ts); [Katana README @ v1.8.0](https://github.com/projectdiscovery/katana/blob/v1.8.0/README.md); [Heritrix CHANGELOG](https://github.com/internetarchive/heritrix3/blob/master/CHANGELOG.md) **C**.

Details that matter for Goodfetch:
- **The vendor tests one browser-library version; the user installs another.** Crawlee pins Playwright 1.63.0 in its own catalog, while its peer range accepts any version, including the 1.64.1 released today ([pnpm-workspace.yaml @ v4.0.0-rc.1](https://github.com/apify/crawlee/blob/v4.0.0-rc.1/pnpm-workspace.yaml)) **C**. A pull request of 2026-10-05 had to "keep transitive playwright in sync with the pinned version" ([#4199](https://github.com/apify/crawlee/pull/4199)) **C**. In 2022 a maintainer explained the old cap: "We depend on the package, and we need to test our extension code works" ([crawlee#1723](https://github.com/apify/crawlee/issues/1723)) **C**.
- **Swapping in a stealth browser means switching off the crawler's own stealth.** Crawlee's Camoufox example sets `useFingerprints: false` "to avoid conflicts with Camoufox" ([avoid_blocking_camoufox.ts @ v4.0.0-rc.1](https://github.com/apify/crawlee/blob/v4.0.0-rc.1/docs/guides/avoid_blocking_camoufox.ts)) **C**. Crawl4AI's `enable_stealth` "cannot be used with" its undetected mode ([async_configs.py](https://github.com/unclecode/crawl4ai/blob/v0.9.5/crawl4ai/async_configs.py)) **C**.
- **Heritrix records the browser's traffic through its own proxy.** Its `BrowserProcessor` "records all browser requests" through a recording proxy built on Jetty's ProxyHandler and its FetchHTTP2 module, so subresources stay under the crawler's network handling ([CHANGELOG 3.10.0](https://github.com/internetarchive/heritrix3/blob/master/CHANGELOG.md)) **C**. scrapy-playwright, by contrast, routes proxies per browser or context rather than through Scrapy's middlewares ([README](https://github.com/scrapy-plugins/scrapy-playwright/blob/v0.0.48/README.md)) **C**.
- **Spider's declarative actions are built in.** `with_automation_scripts` maps URL patterns to `WebAutomation` steps, 25 variants from `Click`, `Fill` and `Type` to `InfiniteScroll`, `WaitForNetworkIdle`, `Screenshot` and `ValidateChain` ([chrome_common.rs, crate 2.53.9](https://docs.rs/crate/spider/2.53.9/source/src/features/chrome_common.rs)) **C**. Firecrawl's equivalent, `actions`, runs only on Fire-engine (§1) **C**.

---

## 4. Artifact A: a third-party library on the crawler's page

The question per product: can the handler hand the page it is given to any automation library, and when it does, what stays with the crawler?

| Product | What the handler receives | Path for a third-party library | Kept by the crawler | Lost or left to the user |
|---|---|---|---|---|
| Crawlee JS `PlaywrightCrawler` / `PuppeteerCrawler` | `page` (Playwright or Puppeteer object); v4 removes `browserController` from the context | In process: a library that accepts that `Page` object. Over CDP: only after the user adds `--remote-debugging-port` to the launch arguments, then attaches by WebSocket and finds the tab | Launch- or context-level proxy, the fingerprint injected at page creation, session cookies (v3 saves them after navigation and **before** the handler; v4 also after it), page closing, retries | Pages, contexts, proxies or stealth that the other library sets up itself (**inference**); no accounting of its model calls |
| Crawlee `StagehandCrawler` (JS and Python) | Playwright `page` with `act`/`extract`/`observe` attached | Stagehand v3 launches Chromium; Playwright attaches over CDP | Queue, retries, Sessions, request lifecycle | Chromium only; "low-level browser properties cannot be modified"; five launch options pass through; Python: no incognito pages, no canvas/WebGL/screen fingerprint; `remoteBrowser` rejected in v4; Stagehand v3 only |
| Crawlee v4 remote pool | `page` of a remote browser | The crawler is the second client: `RemoteBrowserPool` attaches to a provider's CDP or Playwright WebSocket | Session rotation, request lifecycle, `maxOpenBrowsers`, release callback | `headless`, `launchOptions`, `userDataDir`; incognito forced |
| Crawlee Python `PlaywrightCrawler` | `context.page` (Playwright) | In process only; no CDP or remote option in 1.10.4 | As Crawlee JS | As Crawlee JS |
| Spider | `Page` from `subscribe()`; with `chrome_store_page`, `page.get_chrome_page()` returns the live Chromey page, closed with `close_page()` | Rust code drives the Chromey page. Another library must share a Chrome that Spider joined through `with_chrome_connection(ws_url)` | Stealth and fingerprint scripts that Spider adds to each page through CDP before navigation (`setup_chrome_events`), interception, its automation steps | Page lifetime moves to the subscriber; a 2026 report that `chrome_store_page` did not store the page ([spider#392](https://github.com/spider-rs/spider/issues/392), closed) |
| scrapy-playwright | `response.meta["playwright_page"]` when `playwright_include_page` is set | In process: a library that accepts a Playwright page; `PLAYWRIGHT_CDP_URL` (Chromium only) to run on a browser started elsewhere; `PLAYWRIGHT_BROWSER_PROVIDER` for Patchright or Camoufox | Contexts and their proxy, `PLAYWRIGHT_MAX_PAGES_PER_CONTEXT` | The callback must close the page or "the spider job could get stuck" at the page limit; no per-request proxy |
| Crawl4AI | Playwright `page` and `context` in 9 hooks: `on_browser_created`, `on_page_context_created`, `on_user_agent_updated`, `before_goto`, `after_goto`, `on_execution_started`, `on_execution_ended`, `before_retrieve_html`, `before_return_html` | Its managed browser listens on a debugging port (default 9222), so another library can attach. In reverse, `browser_mode="cdp"` with `cdp_url` attaches to a browser another tool launched, and `browser_context_id` + `target_id` reuse that tool's page | Stealth (playwright-stealth or Patchright) when it launched the browser; `create_isolated_context` for concurrent crawls on a shared browser | On a CDP attach, the existing context unless isolated; it finds a target through Playwright's private `_impl_obj._target_id` ([browser_manager.py](https://github.com/unclecode/crawl4ai/blob/v0.9.5/crawl4ai/browser_manager.py#L1537-L1560)) |
| Firecrawl | No handler; `actions` as JSON, only on Fire-engine | Cloud Browser Sandbox: one `cdpUrl` per session for Playwright or agent-browser | In the sandbox: proxies; profiles with one writer at a time | Self-hosted: no actions, no sandbox |
| Katana | No handler (CLI) | `-chrome-ws-url` | Scope, rate limits | Browser identity |
| Heritrix | No handler; Java `Behavior`s | none documented | All browser traffic, through its recording proxy | — |

Sources: [Crawlee v4 upgrading guide, browser management and cookies](https://github.com/apify/crawlee/blob/v4.0.0-rc.1/docs/upgrading/upgrading_v4.md); [Stagehand guide @ v4.0.0-rc.1](https://github.com/apify/crawlee/blob/v4.0.0-rc.1/docs/guides/stagehand_crawler.mdx); [Crawlee Python Stagehand guide](https://crawlee.dev/python/docs/guides/stagehand-crawler); [Spider page.rs](https://docs.rs/crate/spider/2.53.9/source/src/page.rs) and [chrome.rs](https://docs.rs/crate/spider/2.53.9/source/src/features/chrome.rs), crate 2.53.9; [scrapy-playwright README @ v0.0.48](https://github.com/scrapy-plugins/scrapy-playwright/blob/v0.0.48/README.md); [Crawl4AI hooks @ v0.9.5](https://github.com/unclecode/crawl4ai/blob/v0.9.5/crawl4ai/async_crawler_strategy.py#L100-L135); [Firecrawl Browser](https://docs.firecrawl.dev/features/browser); [Katana README](https://github.com/projectdiscovery/katana/blob/v1.8.0/README.md) **C**.

**What the third-party libraries expect.**
- **Stagehand v3** works on a Playwright, Puppeteer or Patchright page, provided that driver is connected over CDP to Stagehand's own browser; the page goes in the options, `stagehand.act("…", { page })` ([v3 Playwright integration](https://docs.stagehand.dev/v3/integrations/playwright)) **C**. The October 7 note found that Crawlee's `page.agent()` wrapper does not bind the agent to the handler's page ([contracts follow-up §3](2026-10-07-crawlee-spider-contracts-followup.md)).
- **Stagehand v4** has "no Playwright interop"; its `page` option "only accepts a page from `browser.context`". It attaches to "any Chromium browser you are already running, by URL", discovers or loads its extension there, needs `--enable-unsafe-extension-debugging` so the SDK can reach the extension's service worker, and closes only browsers it launched. Its SDK "exposes no CDP escape hatch" ([Playwright migration](https://docs.stagehand.dev/v4/migrations/playwright.md), [browser configuration](https://docs.stagehand.dev/v4/configuration/browser.md)) **C**.
- **Browser Use** (open source) attaches through `cdp_url` "for connecting to existing browser instance", with `keep_alive` to leave the browser running ([browser parameters](https://docs.browser-use.com/open-source/customize/browser/all-parameters.md)) **C**. Its core pins most dependencies exactly (`pydantic==2.13.5`, `aiohttp==3.14.3`, `anyio==4.12.1`, `openai==2.26.0` and others) ([PyPI 0.13.11](https://pypi.org/pypi/browser-use/0.13.11/json)) **C**, so sharing a Python environment with a crawler that sets lower bounds is where version conflicts would appear (**inference**). No first-party Browser Use integration exists in Crawlee JS, Crawlee Python or scrapy-playwright (repository search, 2026-10-14) **M**.

**The pattern.** A library that wants a CDP URL gets one either because the crawler launched its browser with a debugging port, which lets any local process read its cookies ([ADR 0017](../../docs/adr/0017-one-egress-policy-and-one-endpoint-rule.md) records why Goodfetch refused an open debugging port), or because the library launched the browser and the crawler attached, losing the crawler's launch-time stealth (Crawlee's Stagehand caveats). Clouds sidestep the choice by selling a remote browser with a CDP URL.

---

## 5. Install size (measured)

npm sizes are the sum of every resolved package's `unpackedSize`, counting only the linux-x64-glibc binary where a package ships one per platform. Python sizes are compressed wheels for CPython 3.12 on x86_64 manylinux. Browser binaries are not included: Playwright downloads them with `npx playwright install`, a first-run step that Crawlee users trip on ([crawlee#2275](https://github.com/apify/crawlee/issues/2275)) **M/C**.

| Install | Packages | Size |
|---|---|---|
| npm `playwright@1.64.1` alone | 2 | 17.8 MB |
| npm `puppeteer@25.13.0` alone | 26 | 25.6 MB |
| npm `@crawlee/cheerio@3.18.2` | 135 | 23.7 MB |
| npm `crawlee@3.18.2` (no browser library) | 290 | 72.3 MB (the CLI's `figlet` alone is 18.9 MB) |
| npm `crawlee@3.18.2` + `playwright` | 292 | 90.1 MB |
| npm `crawlee@4.0.0-rc.1` (no browser library) | 175 | 49.3 MB |
| npm `@crawlee/playwright@4.0.0-rc.1` + `playwright` | 126 | 62.3 MB |
| npm `@crawlee/stagehand@3.18.2` + Stagehand 3.7.3 + `playwright` | 358 | 127.7 MB (`@google/genai` 14.4 MB, Stagehand 10.2 MB) |
| PyPI `crawlee==1.10.4` | 27 | 9.2 MB |
| PyPI `crawlee[playwright]` | 33 | 56.7 MB (the Playwright wheel, 46.0 MB, bundles a Node driver) |
| PyPI `crawlee[stagehand]` | 40 | 92.7 MB (the Stagehand 3.23.0 wheel is 35.7 MB) |
| PyPI `crawlee[all]` | 129 | 202.5 MB |
| PyPI `scrapy==2.19.0` | 44 | 16.9 MB |
| PyPI `scrapy` + `scrapy-playwright` | 48 | 63.6 MB |
| PyPI `crawl4ai==0.9.5` (core) | 95 | 207.4 MB (Playwright 46.0, Patchright 46.0, SciPy 33.7, `unclecode-litellm` 17.2) |
| PyPI `firecrawl-py` (cloud client) | 26 | 4.4 MB |
| crates.io `spider` 2.53.9 crate archive | 1 | 0.78 MB compressed; the build's dependency tree depends on the features chosen (not measured) |

Two readings: the browser driver dominates every Python install (46 MB per Playwright-family wheel, twice in Crawl4AI); and Crawlee v4's default install is a third smaller than v3's (49.3 MB against 72.3 MB) after moving parsers and `got-scraping` out **M**.

---

## 6. Version lockstep

| Product | Lockstep? | Evidence |
|---|---|---|
| Crawlee JS | Yes. Lerna fixed mode: every `@crawlee/*` package at one version, published within minutes (3.18.2: 10:01 to 10:08 UTC on 2026-09-29) | [lerna.json @ v4.0.0-rc.1](https://github.com/apify/crawlee/blob/v4.0.0-rc.1/lerna.json), npm publish times **M** |
| Crawlee JS exceptions | `@crawlee/fs-storage-native` 0.2.2 and its 8 platform packages on their own numbers; `@crawlee/jsdom` and `@crawlee/linkedom` leave for their own repositories and release process | npm **M**, [#4233](https://github.com/apify/crawlee/issues/4233) **C** |
| Crawlee Python and Apify SDK | One distribution; the SDK takes a range (`>=1.8.0,<2.0.0`) | PyPI **C** |
| Spider | The 6 engine-side crates at 2.53.9; Chromey and the fingerprint, firewall and blocker crates on their own numbers; bindings on 0.0.x | crates.io, npm, PyPI **M** |
| Scrapy | No: Scrapy and each plugin release independently | PyPI **M** |
| Browser libraries | Never in lockstep with any crawler: a `*` peer or a lower bound, except the Stagehand peer capped one major behind | npm, PyPI **M** |

Goodfetch's [ADR 0030](../../docs/adr/0030-one-product-version-monthly-minors-and-versioned-contracts.md) already releases every package at one version, which is Crawlee's model.

---

## 7. Recurring complaints

- **Installing the browser.** [crawlee#2275](https://github.com/apify/crawlee/issues/2275) (open since 2024-01-08, 21 comments): a project with `crawlee` and `playwright` "fails on first start asking to `npx install playwright`" **C**. [crawlee#2715](https://github.com/apify/crawlee/issues/2715): browser version mismatch between a Windows machine and an AWS deployment **C**.
- **The browser library's version.** [crawlee#1723](https://github.com/apify/crawlee/issues/1723) "Crawlee should not dictate playwright version" (2022) led to today's `*` peer **C**. Both Crawlee integrations cap Stagehand below 4 while 4.2.1 is current **M**.
- **Remote and third-party browsers.** [crawlee#1822](https://github.com/apify/crawlee/issues/1822) "Connect to remote browser services" (24 reactions, closed by the v4 remote pool); [crawlee#4196](https://github.com/apify/crawlee/issues/4196) (2026-10-05, open) asks to forward `cdpUrl` so `StagehandCrawler` can use a remote browser; [crawlee-python#1743](https://github.com/apify/crawlee-python/issues/1743) and [#1744](https://github.com/apify/crawlee-python/issues/1744) (redesign the browser pool) are open **C**. [crawl4ai#1563](https://github.com/unclecode/crawl4ai/issues/1563): "Unable to execute arun_many with managed browsers and cdp" (5 reactions) **C**.
- **Bundling, runtimes and weight.** [crawlee#2208](https://github.com/apify/crawlee/issues/2208) "Support bundling" (open, 10 reactions); [#1913](https://github.com/apify/crawlee/issues/1913) Deno support (10 reactions); rarely used parsers paid for by every install ([#4113](https://github.com/apify/crawlee/issues/4113)) **C**.
- **Environment breakage from defaults and pins.**
  - [spider#243](https://github.com/spider-rs/spider/issues/243): "Operation not permitted from tokio-uring" in a Docker container (15 comments), from `io_uring` in the default features **C**.
  - [spider#294](https://github.com/spider-rs/spider/issues/294): many errors from Chromiumoxide with a remote Chrome **C**; [spider-py#18](https://github.com/spider-rs/spider-py/issues/18): "Build failure with current spider" (2026-01-19) **C**.
  - [crawl4ai#1903](https://github.com/unclecode/crawl4ai/issues/1903): not compatible with Python 3.14 through its lxml constraint (open); [#2020](https://github.com/unclecode/crawl4ai/issues/2020): the Docker build fails on `nvidia-cuda-toolkit`; [#1206](https://github.com/unclecode/crawl4ai/issues/1206): Playwright errors in AWS Lambda (6 reactions) **C**.
- **Hangs in the browser plugin.** scrapy-playwright [#266](https://github.com/scrapy-plugins/scrapy-playwright/issues/266) "Scrapy hangs with no exception raised", [#343](https://github.com/scrapy-plugins/scrapy-playwright/issues/343) stuck on Windows with Scrapy 2.13 (18 comments), [#244](https://github.com/scrapy-plugins/scrapy-playwright/issues/244) hangs when a page method raises **C**.
- **Self-host parity.** Firecrawl [#1129](https://github.com/firecrawl/firecrawl/issues/1129) "we need proxy on self-hosted version" (18 reactions), [#1028](https://github.com/firecrawl/firecrawl/issues/1028) "[Self-Host] Screenshots are not supported" (18 comments), [#1390](https://github.com/firecrawl/firecrawl/issues/1390) "All scraping engines failed" on self-host (14 reactions) **C**.
- **AI integration** complaints are about the model layer rather than the browser: Crawl4AI's LiteLLM path ([#979](https://github.com/unclecode/crawl4ai/issues/979), 22 comments; [#1291](https://github.com/unclecode/crawl4ai/issues/1291), defaults to OpenAI instead of the configured provider) **C**.

---

## 8. Artifact B: open-source core versus cloud, against Foxhound's capabilities

Cells: **OSS** (in the open-source framework), **Cloud** (only in the vendor's hosted service), **Both**, **Plugin** (a separately installed open-source package), **No** (neither).

| Foxhound capability | Crawlee JS + Apify | Crawlee Python + Apify | Spider + Spider Cloud | Scrapy + Zyte | Firecrawl | Crawl4AI + Cloud |
|---|---|---|---|---|---|---|
| Frontier (queue, visited, URL rules) | Both: local RequestQueue; platform request queue | Both: memory, file, SQL and Redis clients | OSS: in memory; visited set in SQLite with `disk` | OSS: scheduler, dupefilter, `JOBDIR` | Both: NuQ PostgreSQL when self-hosted | OSS: deep-crawl strategies; Cloud "queues, jobs" |
| Politeness per domain | OSS: `sameDomainDelaySecs`, v4 `ThrottlingRequestManager` | OSS: opt-in throttling | OSS: delays; Crawl-delay and `Retry-After` honoured | OSS: download slots, AutoThrottle | Partial: robots.txt on by default; Crawl-delay parsed, not applied | OSS: rate limiter |
| Politeness store across machines | No | No (the SQL and Redis clients share the queue, not the pacing) | No | Plugin: Scrapy Cluster on Redis | No | No |
| Fetch tiers with escalation | OSS: `AdaptivePlaywrightCrawler` | OSS: adaptive crawler | Both: `smart` (OSS); `spider_cloud` Smart mode escalates to the paid unblocker | Plugin: scrapy-zyte-api escalates to the paid Zyte API | Both: fetch and Playwright OSS; Fire-engine Cloud | OSS: anti-bot retries across a proxy list, then a user `fallback_fetch_function`; Cloud "hard site" tier |
| Datasets | Both: local files; platform datasets | Both: plus SQL and Redis clients | No dataset object: a stream of pages | Both: feeds; Scrapy Cloud items | No dataset: job results paged through the API | No dataset: results returned |
| Sinks | Cloud: platform integrations | OSS: SQL and Redis storage clients; Cloud integrations | Cloud: data connectors (S3, GCS, Google Sheets, Azure Blob, Supabase) | OSS: feed exports (S3, GCS, FTP, local) | Both: webhooks; retries in Cloud | OSS: Docker server webhooks |
| Search over results | No | No | No | No | No | No |
| Response cache | No | No | OSS: `cache`, `cache_mem`, `cache_chrome_hybrid` | OSS: `HttpCacheMiddleware` | Cloud: `maxAge` | OSS: SQLite cache |
| Console (web UI) | Cloud: Apify Console | Cloud | Cloud: dashboard | Cloud: Scrapy Cloud; Plugin: Scrapyd's minimal UI | Cloud: dashboard; self-hosted queue admin UI off by default | Both: Docker server playground and monitor; Cloud key console |
| Daemon (long-lived shared server) | No | No | Plugin: `spider_worker`, an HTTP fetch worker | Plugin: Scrapyd | OSS: the API server itself | OSS: the Docker API server |
| Schedules | Cloud: Apify Schedules (cron) | Cloud | OSS: `cron` feature; Cloud not found | Plugin: Scrapyd API (no cron); Cloud: periodic jobs on paid units | Cloud: `/v2/monitor` | No |

Sources: [frontier and sessions note](https://github.com/goodfetch/automan/blob/research/frontier-sessions-competitors/rnd/research/2026-10-10-frontier-sessions-competitors.md) and [storage and sinks note](https://github.com/goodfetch/automan/blob/research/storage-sinks-competitors/rnd/research/2026-10-10-storage-sinks-competitors.md); ADRs [0036](../../docs/adr/0036-the-daemon-schedules-runs-above-the-engine.md), [0046](../../docs/adr/0046-a-per-user-response-cache-off-by-default-with-revalidate-and-replay.md), [0053](../../docs/adr/0053-politeness-is-shared-by-every-run-of-a-user-and-across-machines-through-a-store.md); [Apify platform](https://docs.apify.com/platform); [Spider Cloud API](https://spider.cloud/docs/api); [Scrapy Cloud pricing](https://docs.zyte.com/scrapy-cloud/pricing.md); [Firecrawl SELF_HOST.md](https://github.com/firecrawl/firecrawl/blob/f4fdf7f300415180f45272e893918dbfa3649073/SELF_HOST.md); [Crawl4AI async_webcrawler.py @ v0.9.5](https://github.com/unclecode/crawl4ai/blob/v0.9.5/crawl4ai/async_webcrawler.py) and [Docker README](https://github.com/unclecode/crawl4ai/blob/v0.9.5/deploy/docker/README.md); [crawl4ai.com](https://crawl4ai.com/) **C**.

Reading the table: **no competitor offers search over a run's results** (the nearest is Apify's dataset `fields`/`unwind` filters on export), **only Scrapy has a shared-pacing add-on**, and **schedules plus a UI sit in a cloud everywhere except Spider's in-process `cron` and the Scrapyd/Crawl4AI servers**.

---

## 9. Implications for Goodfetch

Considerations for the decision, not decisions.

- **A separately installable Collie matches the market only if Foxhound pins it.** The comparable split, Crawlee and Playwright, is two vendors joined by a `*` peer range: Crawlee tests one Playwright while users run another, and a capped range produced the opposite complaint (#1723). A Foxhound binary that contains Collie removes that version problem, as ADR 0030's single product version already does for the SDKs; Crawlee v4 thins its default install by moving rarely used parsers out (#4113), not the browser layer.
- **Lending a page needs two things: a page handle and an endpoint.** In-process libraries take a page object (Stagehand v3, Playwright-compatible drivers); out-of-process ones take a CDP URL (Browser Use, Stagehand v4, agent-browser). ADR 0019 already serves a raw CDP WebSocket endpoint for Playwright and Puppeteer, which is the endpoint half. None of the surveyed crawlers hands the handler an endpoint bound to its own page: users add `--remote-debugging-port` and find the tab, or point the library at a cloud browser. Crawl4AI's `target_id` is the only first-party "use this exact tab" setting found, and it relies on a Playwright private field.
- **Stagehand v4 is the hard case.** It refuses foreign page objects, needs its extension and `--enable-unsafe-extension-debugging` in the browser, and has no CDP escape hatch; Crawlee has not followed it past v3 in either language. An integration test against `localBrowser.connect` on an engine-served endpoint would settle whether Goodfetch can lend it a page without letting it launch the browser.
- **What the crawler keeps while another library drives the page is the contract to write down.** Crawlee keeps launch-time proxy and fingerprint and syncs cookies at fixed points, and must switch its own fingerprinting off for a stealth browser. Heritrix keeps every subresource request by recording through its own proxy, the closest model to ADR 0017's engine-owned egress proxy for raw CDP Sessions. The Session, the Politeness counters and the Usage of the other library's model calls are what a lent page could escape.
- **Cloud lines are drawn at scheduling, UI, durable results, the hard fetch tier and the raw remote browser.** ADR 0036 (Schedules in the Daemon), ADR 0031 (a local Console over a Run index) and ADR 0053 (a shared Politeness store) put in the open-source framework what every surveyed competitor keeps in its cloud or leaves out. Spider and Crawl4AI also show the opposite move, a cloud funnel compiled or printed into the open-source library.

---

## 10. Unverified / open

- Whether Stagehand v4 can act on a tab another client opened. It attaches by URL, but its `page` option accepts only pages of its own `browser.context`: **Unverified**, not run.
- Whether `@spider-rs/spider-rs` 0.0.163 installs a working native binary without the platform packages its predecessor declared: **Unverified**, registry metadata only.
- Fingerprint injection on Crawlee v4 remote browsers: the guide lists what does not apply and does not mention fingerprints: **Unverified**.
- Whether a library that opens its own page in a Crawlee-owned context inherits Crawlee's fingerprint (injected per context or per page): **Unverified** (**inference** in §4).
- Spider Cloud scheduling and its dashboard's features: not found in the API docs read; **Unverified**.
- Browser download sizes (Playwright's Chromium and headless shell, Puppeteer's Chrome): not measured.
- Firecrawl politeness beyond robots.txt (per-domain concurrency): taken from the October 10 note, not re-read at `f4fdf7f`.

---

## Sources

Registries (read 2026-10-14): [npm `crawlee`](https://registry.npmjs.org/crawlee), [npm `@crawlee/stagehand`](https://registry.npmjs.org/@crawlee/stagehand), [npm `@browserbasehq/stagehand`](https://registry.npmjs.org/@browserbasehq/stagehand), [npm `apify`](https://registry.npmjs.org/apify), [npm `@spider-rs/spider-rs`](https://registry.npmjs.org/@spider-rs/spider-rs), [npm `playwright`](https://registry.npmjs.org/playwright), [npm `puppeteer`](https://registry.npmjs.org/puppeteer), [PyPI `crawlee`](https://pypi.org/project/crawlee/), [PyPI `apify`](https://pypi.org/project/apify/), [PyPI `crawl4ai`](https://pypi.org/project/crawl4ai/), [PyPI `scrapy`](https://pypi.org/project/Scrapy/), [PyPI `scrapy-playwright`](https://pypi.org/project/scrapy-playwright/), [PyPI `scrapling`](https://pypi.org/project/scrapling/), [PyPI `browser-use`](https://pypi.org/project/browser-use/), [PyPI `stagehand`](https://pypi.org/project/stagehand/), [PyPI `spider_rs`](https://pypi.org/project/spider_rs/), [crates.io `spider`](https://crates.io/crates/spider), [crates.io `chromey`](https://crates.io/crates/chromey), [Go proxy Katana](https://proxy.golang.org/github.com/projectdiscovery/katana/@latest), [Go proxy Colly](https://proxy.golang.org/github.com/gocolly/colly/v2/@latest).

Code at tags: [Crawlee v4.0.0-rc.1](https://github.com/apify/crawlee/tree/v4.0.0-rc.1) (upgrading guide, remote browser, Stagehand and Camoufox guides, `pnpm-workspace.yaml`, `lerna.json`); [Crawlee Python v1.10.4](https://github.com/apify/crawlee-python/tree/v1.10.4); [Spider 2.53.9 crate source](https://docs.rs/crate/spider/2.53.9/source/) (`Cargo.toml.orig`, `configuration.rs`, `page.rs`, `features/chrome.rs`, `features/chrome_common.rs`); [scrapy-playwright v0.0.48](https://github.com/scrapy-plugins/scrapy-playwright/tree/v0.0.48); [Crawl4AI v0.9.5](https://github.com/unclecode/crawl4ai/tree/v0.9.5) (`async_configs.py`, `async_crawler_strategy.py`, `async_webcrawler.py`, `browser_manager.py`, `browser_adapter.py`, `cloud_notice.py`, `cloud/cli.py`, `deploy/docker/README.md`); [Firecrawl `f4fdf7f`](https://github.com/firecrawl/firecrawl/tree/f4fdf7f300415180f45272e893918dbfa3649073) (`SELF_HOST.md`, `engines/index.ts`); [Playwright v1.64.1 `chromium.ts`](https://github.com/microsoft/playwright/blob/v1.64.1/packages/playwright-core/src/server/chromium/chromium.ts); [Katana v1.8.0 README](https://github.com/projectdiscovery/katana/blob/v1.8.0/README.md); [Heritrix CHANGELOG](https://github.com/internetarchive/heritrix3/blob/master/CHANGELOG.md); [Browsertrix Crawler v1.15.0 package.json](https://github.com/webrecorder/browsertrix-crawler/blob/v1.15.0/package.json).

Docs (read 2026-10-14): [Stagehand v4 Playwright migration](https://docs.stagehand.dev/v4/migrations/playwright.md), [Stagehand v4 browser configuration](https://docs.stagehand.dev/v4/configuration/browser.md), [Stagehand v3 Playwright integration](https://docs.stagehand.dev/v3/integrations/playwright), [Crawlee Python Stagehand guide](https://crawlee.dev/python/docs/guides/stagehand-crawler), [Browser Use browser parameters](https://docs.browser-use.com/open-source/customize/browser/all-parameters.md), [Browser Use cloud browser over CDP](https://docs.browser-use.com/cloud/browser/playwright-puppeteer-selenium.md), [Apify platform](https://docs.apify.com/platform), [Apify Schedules](https://docs.apify.com/platform/schedules), [Spider Cloud API](https://spider.cloud/docs/api), [Firecrawl self-host guide](https://docs.firecrawl.dev/contributing/self-host), [Firecrawl Browser](https://docs.firecrawl.dev/features/browser), [Scrapy Cloud pricing](https://docs.zyte.com/scrapy-cloud/pricing.md), [Crawl4AI Cloud](https://crawl4ai.com/).

Issues and pull requests: cited inline (Crawlee #1723, #1822, #1913, #2208, #2275, #2715, #3977, #4044, #4101, #4104, #4113, #4119, #4196, #4199, #4233; Crawlee Python #1743, #1744; Spider #243, #294, #392; spider-py #18; scrapy-playwright #244, #266, #343, #385; Crawl4AI #979, #1206, #1291, #1563, #1903, #2020; Firecrawl #1028, #1129, #1390).
