# Crawl-framework products: packaging, and how they plug in automation libraries

Research of **October 14, 2026** for ticket [#449](https://github.com/goodfetch/automan/issues/449). Question: how do crawl frameworks, Foxhound's direct competitors, package themselves, and how do they let users plug in browser-automation libraries, their own or third-party? It feeds the tickets that decide the Collie/Foxhound product boundary and how Foxhound works with Playwright, Puppeteer, Stagehand and Browser Use.

**Sources.** Package registries (npm, PyPI, crates.io, the Go module proxy) read through their JSON APIs on 2026-10-14; source code at release tags through the GitHub API, or from the published crate archives; official docs; GitHub issues and pull requests. It builds on the October 7 notes ([engines](2026-10-07-crawl-engines.md), [Crawlee/Spider contracts](2026-10-07-crawlee-spider-contracts-followup.md), [capability matrix](2026-10-07-crawl-capability-matrix.md)) and the October 10 notes on frontier and sessions and on storage and sinks (branches `research/frontier-sessions-competitors` and `research/storage-sinks-competitors`). Facts reused from them were re-checked against the registries and the tagged sources.

**Measured versus cited.** Install sizes were measured, not quoted (§5): npm trees resolved with `npm install --package-lock-only --ignore-scripts` and summed from each package's registry `dist.unpackedSize`; Python trees resolved with `uv pip compile` for CPython 3.12 on x86_64 manylinux and summed from the size of the wheel PyPI serves for that platform. Nothing was installed or run. Measured facts are marked **M**; everything else is cited and marked **C**. Like everything in `rnd/`, this note informs a decision; it does not make one.

## Versions inspected

| Product | Version (date) | Where read |
|---|---|---|
| Crawlee JS | `crawlee` **3.18.2** (2026-09-29), **4.0.0-rc.1** (2026-10-06); 15 `@crawlee/*` packages at each | [npm](https://registry.npmjs.org/crawlee), tags [v3.18.2](https://github.com/apify/crawlee/tree/v3.18.2), [v4.0.0-rc.1](https://github.com/apify/crawlee/tree/v4.0.0-rc.1) |
| Crawlee Python | **1.10.4** (2026-10-06) | [PyPI](https://pypi.org/project/crawlee/1.10.4/), tag [v1.10.4](https://github.com/apify/crawlee-python/tree/v1.10.4) |
| Apify SDK | JS `apify` **3.7.2** (2026-05-11; 4.0.0-rc.0 on `rc`), Python `apify` **4.1.0** (2026-10-08) | [npm](https://registry.npmjs.org/apify), [PyPI](https://pypi.org/project/apify/) |
| Spider-rs | `spider`, `spider_cli`, `spider_agent`, `spider_mcp`, `spider_utils`, `spider_worker` **2.53.9** (2026-09-05); `chromey` **2.58.2** (2026-09-12); `main` says 2.54.0, unpublished | [crates.io](https://crates.io/crates/spider), crate archives |
| Spider bindings | npm `@spider-rs/spider-rs` **0.0.163** (2026-01-24); PyPI `spider_rs` **0.0.57** (2025-01-28) | [npm](https://registry.npmjs.org/@spider-rs/spider-rs), [PyPI](https://pypi.org/project/spider_rs/) |
| Scrapy | **2.19.0** (2026-09-10); `scrapy-playwright` **0.0.48** (2026-07-10); `scrapy-splash` 0.11.1 (2025-02-11); `scrapy-zyte-api` 0.36.0 (2026-08-20) | [PyPI](https://pypi.org/project/Scrapy/), tag [scrapy-playwright v0.0.48](https://github.com/scrapy-plugins/scrapy-playwright/tree/v0.0.48) |
| Firecrawl | last GitHub release **v2.11.0** (2026-06-19); tags up to v2.11.518; `main` at [`f4fdf7f`](https://github.com/firecrawl/firecrawl/tree/f4fdf7f300415180f45272e893918dbfa3649073) (2026-10-13); `firecrawl-py` 4.50.0 | GitHub API, [PyPI](https://pypi.org/project/firecrawl-py/) |
| Crawl4AI | **0.9.5** (2026-10-14) | [PyPI](https://pypi.org/project/crawl4ai/0.9.5/), tag [v0.9.5](https://github.com/unclecode/crawl4ai/tree/v0.9.5) |
| Automation libraries | Playwright **1.64.1** (2026-10-14); Puppeteer **25.13.0** (2026-10-08); Stagehand npm **4.2.1** (2026-10-11), `v3-latest` 3.7.3; Stagehand PyPI 4.2.1 (2026-10-10); Browser Use **0.13.11** (2026-10-07) | npm, PyPI |
| Brief coverage | Katana **v1.8.0** (2026-10-05); Colly **v2.3.0** (Go proxy, 2025-12-04); Heritrix **3.18.0** (2026-10-06); StormCrawler 3.7.0 (2026-07-29); Scrapling 0.4.15 (2026-08-23); Browsertrix Crawler 1.15.0 (2026-10-02) | Go proxy, GitHub releases |

---

## 1. Summary

- **Every crawl framework with a browser tier treats the browser library as the user's choice, not its own dependency, except Crawl4AI.**
  - Crawlee JS declares `playwright` and `puppeteer` as optional peer dependencies with range `*`, on the `crawlee` meta-package and on every browser package, in 3.18.2 and in 4.0.0-rc.1 **C** ([npm `crawlee`](https://registry.npmjs.org/crawlee/latest), [npm `@crawlee/playwright`](https://registry.npmjs.org/@crawlee/playwright/4.0.0-rc.1)). Its own tests pin Playwright 1.63.0 and Puppeteer 25.12.0 ([pnpm catalog](https://github.com/apify/crawlee/blob/v4.0.0-rc.1/pnpm-workspace.yaml), [root `package.json`](https://github.com/apify/crawlee/blob/v4.0.0-rc.1/package.json)) **C**.
  - Crawlee Python and scrapy-playwright put Playwright behind an extra or a plugin with only a lower bound (`playwright>=1.27.0`, `playwright>=1.40`) **C**.
  - Spider-rs compiles its browser in through Cargo features: `chrome` pulls its own CDP driver, Chromey; `webdriver` pulls thirtyfour; neither is a default feature **C**.
  - Crawl4AI's core install requires Playwright **and** Patchright **and** playwright-stealth, plus an exact pin on its own LiteLLM fork (`unclecode-litellm==1.81.13`): 95 packages, 207 MB of wheels **M**.
- **The handler gets the crawler's page as a library object, never as a protocol endpoint.** Crawlee and Crawl4AI hand the handler a Playwright (or Puppeteer) `Page`; scrapy-playwright does when a request sets `playwright_include_page`; Spider hands a Chromey `Page` when built with `chrome_store_page` **C**. A third-party library that wants a CDP URL (Browser Use, Stagehand v4) gets nothing to attach to by default: Playwright launches Chromium over `--remote-debugging-pipe` and exposes a WebSocket only when the user adds `--remote-debugging-port` to the launch arguments ([chromium.ts @ v1.64.1](https://github.com/microsoft/playwright/blob/v1.64.1/packages/playwright-core/src/server/chromium/chromium.ts#L355-L372)) **C**.
- **The one first-party third-party integration, Crawlee's Stagehand crawler, inverts the relationship: Stagehand launches the browser and the crawler attaches to it.** `StagehandCrawler` (JS and Python) lets Stagehand launch Chromium and connects Playwright over CDP to it; the guide lists what is lost: Chromium only, "low-level browser properties cannot be modified", only `headless`, `args`, `executablePath`, `proxy` and `viewport` passed through, custom context options not fully supported ([guide @ v4.0.0-rc.1](https://github.com/apify/crawlee/blob/v4.0.0-rc.1/docs/guides/stagehand_crawler.mdx)) **C**. `StagehandCrawler` rejects `remoteBrowser` in v4 ([upgrading guide @ v4.0.0-rc.1](https://github.com/apify/crawlee/blob/v4.0.0-rc.1/docs/upgrading/upgrading_v4.md)) **C**.
- **Both Crawlee integrations are stuck on Stagehand v3 while Stagehand ships v4.** `@crawlee/stagehand` 4.0.0-rc.1 still peers `@browserbasehq/stagehand ^3.0.0`; `crawlee[stagehand]` 1.10.4 requires `stagehand<4.0.0,>=3.19.5`, which resolves to 3.23.0 while PyPI's latest is 4.2.1 **C/M**. Stagehand v4 states "there is no interop": a Playwright `Page` will not type-check as `act()`'s `page`, and attaching to a running browser (`localBrowser.connect({ cdpUrl })`) loads Stagehand's extension, which needs `--enable-unsafe-extension-debugging` on the browser's command line and a shared filesystem ([Playwright migration](https://docs.stagehand.dev/v4/migrations/playwright.md), [browser configuration](https://docs.stagehand.dev/v4/configuration/browser.md)) **C**.
- **Crawlee v4 adds the seam a third-party browser needs, on the browser-pool side.** Crawlers accept any `IBrowserPool` (`newPage`, `closePage`, `extractPageState`, `injectPageState`); a pool passed in is borrowed and never destroyed by the crawler; a `remoteBrowser` option or a `RemoteBrowserPool` connects to Browserbase, Browserless, Steel or any CDP WebSocket, with `maxOpenBrowsers` and a session-release callback ([browser management](https://github.com/apify/crawlee/blob/v4.0.0-rc.1/docs/upgrading/upgrading_v4.md), [remote browser guide](https://github.com/apify/crawlee/blob/v4.0.0-rc.1/docs/guides/remote_browser.mdx)) **C**. On a remote browser, `headless` and `launchOptions` do not apply, incognito pages are forced, and `userDataDir` has no effect (same guide) **C**. The 2023 request, [crawlee#1822](https://github.com/apify/crawlee/issues/1822), is the most-reacted issue found on the topic (24 reactions) **C**. Crawlee Python has the same request open ([crawlee-python#1743](https://github.com/apify/crawlee-python/issues/1743)) **C**.
- **scrapy-playwright 0.0.48 made the driver itself pluggable.** `PLAYWRIGHT_BROWSER_PROVIDER` is "an extension point for integrating third-party drivers that expose Playwright-compatible `Browser`/`BrowserContext`/`Page` objects (e.g. patchright, camoufox) without changing the handler" ([README @ v0.0.48](https://github.com/scrapy-plugins/scrapy-playwright/blob/v0.0.48/README.md), [PR #385](https://github.com/scrapy-plugins/scrapy-playwright/pull/385)) **C**. Its known issue: no per-request proxy, proxies are per browser or per context (same README) **C**.
- **Packaging splits three ways:** one meta-package over many lockstep packages (Crawlee JS: 15 `@crawlee/*` packages released together by Lerna in fixed mode, and v4 moves JSDOM and LinkeDOM out of the monorepo); one distribution with extras (Crawlee Python: 14 extras; Crawl4AI); one crate with feature flags (Spider: **129** Cargo features, 6 crates released together at 2.53.9, its CDP driver Chromey versioned apart at 2.58.2) **C/M**. Spider's Node and Python bindings are versioned apart and lag: the newest Node release, 0.0.163 (2026-01-24), declares no platform packages while 0.0.157 declared 14; the Python binding's last release is 0.0.57 (2025-01-28) **M**.
- **The open-source cores ship the crawl machinery; the clouds keep scheduling, the UI, durable storage and the hard fetch tier.** Crawlee, Spider and Scrapy ship a frontier, per-domain limits and local Datasets; none ships a Politeness store shared across machines (Crawlee Python's Redis and SQL storage clients share the queue, not the pacing). Schedules and a web UI are Apify-, Zyte- or Firecrawl-cloud features, except Spider's `cron` feature, Scrapyd's HTTP scheduling API and Heritrix's web UI (§8) **C**. Firecrawl's self-hosted stack has no browser actions: only its Fire-engine engines declare `actions: true`, and Fire-engine is "not included" ([engines/index.ts @ f4fdf7f](https://github.com/firecrawl/firecrawl/blob/f4fdf7f300415180f45272e893918dbfa3649073/apps/api/src/scraper/scrapeURL/engines/index.ts), [self-host guide](https://docs.firecrawl.dev/contributing/self-host)) **C**.
- **Clouds sell the raw browser too.** Spider Cloud (`WS /v1/browser`, "Cloud browser via CDP over WebSocket"), Firecrawl's Browser Sandbox (`POST /v2/interact`, every session returns a `cdpUrl`, Playwright and agent-browser preinstalled, 20 concurrent sessions) and Browser Use Cloud each expose a CDP endpoint for any library ([Spider API](https://spider.cloud/docs/api), [Firecrawl Browser](https://docs.firecrawl.dev/features/browser), [Browser Use](https://docs.browser-use.com/cloud/browser/playwright-puppeteer-selenium.md)) **C**. Spider's open-source crate routes its own fetches to Spider Cloud through the `spider_cloud` feature, on by default in `spider_cli` and `spider_mcp`, with a `Smart` mode that "falls back to `/unblocker` when it detects bot protection" ([configuration.rs, crate 2.53.9](https://docs.rs/crate/spider/2.53.9/source/src/configuration.rs)) **C**.

---

## 2. Packaging: what is core and what is optional

### Crawlee JS

| | 3.18.2 | 4.0.0-rc.1 |
|---|---|---|
| Meta-package `crawlee` depends on | `@crawlee/cli`, `core`, `http`, `basic`, `jsdom`, `utils`, `browser`, `cheerio`, `linkedom`, `puppeteer`, `playwright`, `browser-pool`, `memory-storage` | `cli`, `core`, `http`, `basic`, `utils`, `browser`, `cheerio`, `puppeteer`, `fs-storage`, `playwright`, `impit-client` |
| Optional peers of `crawlee` | `puppeteer: *`, `playwright: *`, `idcac-playwright: *` | `puppeteer: *`, `playwright: *` |
| Browser packages' peers | `@crawlee/browser`, `browser-pool`: `playwright: *`, `puppeteer: *` (optional); `@crawlee/playwright` adds `idcac-playwright ^0.2.0` | same, without `idcac-playwright` |
| `@crawlee/stagehand` peers | `@browserbasehq/stagehand ^3.0.0`, `playwright: *`, `zod ^3 \|\| ^4` | `@browserbasehq/stagehand ^3.0.0`, `playwright: *` |
| Default HTTP client | `got-scraping` (a hard dependency of `@crawlee/core`) | `impit` through `@crawlee/impit-client`; `got-scraping` moves to opt-in `@crawlee/got-scraping-client` |

Sources: npm manifests of each package at both versions ([`crawlee` 3.18.2](https://registry.npmjs.org/crawlee/3.18.2), [`crawlee` 4.0.0-rc.1](https://registry.npmjs.org/crawlee/4.0.0-rc.1), [`@crawlee/stagehand` 4.0.0-rc.1](https://registry.npmjs.org/@crawlee/stagehand/4.0.0-rc.1)), [upgrading guide @ v4.0.0-rc.1](https://github.com/apify/crawlee/blob/v4.0.0-rc.1/docs/upgrading/upgrading_v4.md) **C**.

What v4 moves out of the default install, and why:
- **JSDOM and LinkeDOM leave the monorepo.** "`jsdom` and `linkedom` are heavy dependencies that every workspace install and CI run has to pay for, and the two crawlers are rarely used yet tied to the release cadence of everything else"; they become separate repositories with `@crawlee/http` as a peer and keep their npm names ([crawlee#4113](https://github.com/apify/crawlee/issues/4113), [#4119](https://github.com/apify/crawlee/pull/4119)) **C**. The parser is now a `parser` option of a `DomCrawler` ([#4104](https://github.com/apify/crawlee/pull/4104)) **C**.
- **`idcac-playwright` stops being a dependency** ([#3977](https://github.com/apify/crawlee/pull/3977)), and the cookie-modal helper is replaced by a hook built on `@duckduckgo/autoconsent` ([#4044](https://github.com/apify/crawlee/pull/4044)) **C**.
- **`@crawlee/browser-pool` is no longer re-exported from `crawlee`**: code that builds its own pool imports it directly ([browser management](https://github.com/apify/crawlee/blob/v4.0.0-rc.1/docs/upgrading/upgrading_v4.md)) **C**.
- **Native binaries arrive.** `impit` and `@crawlee/fs-storage-native` ship one binary per platform as `optionalDependencies`, so `npm install --omit=optional` "breaks native dependencies" (same guide) **C**.
- **The Apify SDK is being decoupled.** `apify` depends on `@crawlee/core` "only because of the storage classes and a handful of runtime pieces around them"; the plan is a shared package both depend on ([crawlee#4101](https://github.com/apify/crawlee/issues/4101)) **C**. Today `apify` 3.7.2 depends on `@crawlee/core`, `@crawlee/types` and `@crawlee/utils` `^3.14.1` ([npm](https://registry.npmjs.org/apify/3.7.2)) **C**.

### Crawlee Python

One distribution, `crawlee` 1.10.4, with a small core (`impit`, `pydantic`, `protego`, `tldextract`, `psutil` and others) and 14 extras: `adaptive-crawler`, `beautifulsoup`, `cli`, `curl-impersonate`, `httpx`, `otel`, `parsel`, `playwright`, `pydantic-ai`, `redis`, `sql-mysql`, `sql-postgres`, `sql-sqlite`, `stagehand`, plus `all` ([PyPI metadata](https://pypi.org/pypi/crawlee/1.10.4/json)) **C**. The browser extras pin nothing above: `playwright>=1.27.0`, and `stagehand<4.0.0,>=3.19.5` **C**. 1.10 adds an experimental `PydanticAiCrawler`, an HTTP crawler whose `context.extract(Model)` asks a Pydantic AI model for a validated instance, defaulting to `openai:gpt-5.4-nano` ([`_pydantic_ai_crawler.py` @ v1.10.4](https://github.com/apify/crawlee-python/blob/v1.10.4/src/crawlee/crawlers/_pydantic_ai/_pydantic_ai_crawler.py)) **C**.

### Spider-rs

- **Crates.** `spider` (engine), `spider_cli`, `spider_agent` (LLM automation), `spider_mcp` (MCP server), `spider_utils`, `spider_worker` (HTTP worker for the `decentralized` mode), all 2.53.9 on 2026-09-05; `chromey`, the CDP driver forked from chromiumoxide, 2.58.2; satellites versioned apart: `spider_fingerprint` 2.39.0, `spider_firewall` 2.42.0, `spider_network_blocker` 0.0.86, `spider_transformations` 2.39.13 ([crates.io API](https://crates.io/api/v1/crates/spider)) **M**.
- **Features.** The engine crate declares **129** Cargo features; its default set is `basic`, `io_uring`, `tcp_fastopen`, `splice`, `numa`, `zero_copy`, so no browser, no disk, no cache ([Cargo.toml, crate 2.53.9](https://docs.rs/crate/spider/2.53.9/source/Cargo.toml.orig)) **M**. Browser features: `chrome` (Chromey), `smart` (`chrome` + `chrome_intercept`), `chrome_store_page`, `chrome_stealth`, `chrome_headed`, `adblock`, `firewall`, `webdriver` with `webdriver_chrome`, `webdriver_firefox`, `webdriver_edge` (thirtyfour) **C**. Other tiers: `wreq` (TLS-impersonating client), `cache`, `cache_mem`, `cache_chrome_hybrid`, `disk` (SQLite through sqlx), `cron`, `warc`, `openai`, `gemini`, `agent`, `spider_cloud` **C**.
- **CLI and MCP defaults.** `spider_cli` defaults to `status_code`, `chrome`, `warc`, `spider_cloud`; `spider_mcp` to `chrome`, `spider_cloud`; `spider_agent` has `default = []` ([crate archives 2.53.9](https://docs.rs/crate/spider_cli/2.53.9/source/Cargo.toml)) **M**.
- **Feature-off calls are silent no-ops.** `with_chrome_connection` "does nothing if the `chrome` is not enabled", and the same `#[cfg(not(feature))]` twin exists for `with_stealth`, `with_fingerprint`, `with_cron`, `with_webdriver`, `with_spider_cloud` ([configuration.rs, crate 2.53.9](https://docs.rs/crate/spider/2.53.9/source/src/configuration.rs)) **C**. The October 7 note found the same for `crawl_smart()` without `smart` ([contracts follow-up](2026-10-07-crawlee-spider-contracts-followup.md)).
- **Bindings.** `@spider-rs/spider-rs` (napi-rs) and `spider_rs` (PyO3) each depend on `spider = "2"` with their own feature list (the Python one enables `chrome`, `smart`, `cache`, `openai` and others) ([spider-nodejs Cargo.toml](https://github.com/spider-rs/spider-nodejs/blob/main/Cargo.toml), [spider-py Cargo.toml](https://github.com/spider-rs/spider-py/blob/main/Cargo.toml)) **C**. Version numbers are unrelated to the crate's, and releases lag (above) **M**. The cloud has its own clients, `spider-client` on PyPI (0.1.93) and `@spider-cloud/spider-client` on npm (0.2.5) **M**.

### Scrapy

- **Core.** `scrapy` 2.19.0: 23 runtime requirements around Twisted and lxml, no browser ([PyPI](https://pypi.org/pypi/scrapy/2.19.0/json)) **C**.
- **Browser layer.** Download-handler plugins in separate distributions, each versioned on its own:
  - `scrapy-playwright` (`playwright>=1.40`), maintained under `scrapy-plugins` **C**;
  - `scrapy-splash` 0.11.1 against the Splash rendering service, whose repository was last pushed on 2024-08-02 ([GitHub API](https://github.com/scrapinghub/splash)) **C**;
  - `scrapy-selenium` 0.0.7 from 2019 **C**;
  - `scrapy-camoufox` 0.0.12 (2025-01-16, `camoufox<0.5.0`) **C**;
  - Zyte's own `scrapy-zyte-api` 0.36.0, which sends requests to the paid Zyte API for rendering and actions **C**.
- **TLS impersonation** is another plugin, `scrapy-impersonate` 1.9.0 (curl-cffi) **C**.

### Firecrawl, Crawl4AI

- **Firecrawl** is a service, not a library: an AGPL-3.0 API server run with Docker Compose (API, workers, a Playwright microservice, Redis, RabbitMQ, NuQ PostgreSQL and an optional FoundationDB), plus thin SDKs (`firecrawl-py` 4.50.0: 26 packages, 4.4 MB of wheels) **C/M** ([SELF_HOST.md @ f4fdf7f](https://github.com/firecrawl/firecrawl/blob/f4fdf7f300415180f45272e893918dbfa3649073/SELF_HOST.md)).
- **Crawl4AI** is one distribution whose core already includes both browser drivers and LiteLLM. Extras add PDF, torch, transformers and Selenium (`sync`) ([PyPI](https://pypi.org/pypi/crawl4ai/0.9.5/json)) **C**.

---

## 3. How the browser layer is chosen and swapped

| Product | Unit of choice | First-party drivers | Swap mechanism |
|---|---|---|---|
| Crawlee JS | crawler class + browser plugin | Playwright (Chromium, Firefox, WebKit, Camoufox via `camoufox-js`), Puppeteer, Stagehand v3 | `PlaywrightCrawler` / `PuppeteerCrawler` / `StagehandCrawler`; `launchContext.launcher` takes any Playwright-compatible launcher; v4: any `IBrowserPool`, or `remoteBrowser` / `RemoteBrowserPool` for a CDP or Playwright WebSocket |
| Crawlee Python | crawler class + `BrowserPlugin` | Playwright, Stagehand v3 (REST SDK) | `PlaywrightCrawler(browser_type=…)`, `StagehandCrawler`; a custom `BaseBrowserPlugin`; no remote-browser option yet ([#1743](https://github.com/apify/crawlee-python/issues/1743)) |
| Spider | Cargo feature at compile time | Chromey (CDP), thirtyfour (WebDriver) | `chrome` / `smart` / `webdriver_*` features; `with_chrome_connection(url)` or `with_chrome_connections(urls)` for an existing Chrome |
| Scrapy | download handler per scheme | none in core; scrapy-playwright, scrapy-splash, scrapy-zyte-api | `DOWNLOAD_HANDLERS`; in scrapy-playwright, `PLAYWRIGHT_BROWSER_TYPE`, `PLAYWRIGHT_CDP_URL`, `PLAYWRIGHT_CONNECT_URL`, `PLAYWRIGHT_BROWSER_PROVIDER` |
| Crawl4AI | `BrowserConfig.browser_mode` + crawler strategy | Playwright, Patchright (`undetected`), playwright-stealth | `browser_mode` `dedicated` (default), `builtin`, `cdp` (with `cdp_url`), `docker`; a custom `AsyncCrawlerStrategy` |
| Firecrawl | engine registry | `fetch`, `playwright`, Fire-engine (`chrome-cdp`, `tlsclient`, stealth variants) | server-side waterfall by required features; Fire-engine only through a separate service |
| Katana | CLI mode | go-rod (headless hybrid, experimental) | `-headless`, `-system-chrome`, `-chrome-ws-url` |
| Heritrix | processor in the crawl chain | WebDriver BiDi to a local Chrome or Firefox | `BrowserProcessor` bean with pluggable `Behavior`s |

Sources: [Crawlee remote browsers @ v4.0.0-rc.1](https://github.com/apify/crawlee/blob/v4.0.0-rc.1/docs/guides/remote_browser.mdx); [Crawlee Python browsers @ v1.10.4](https://github.com/apify/crawlee-python/tree/v1.10.4/src/crawlee/browsers); Spider [Cargo.toml](https://docs.rs/crate/spider/2.53.9/source/Cargo.toml.orig) and [configuration.rs](https://docs.rs/crate/spider/2.53.9/source/src/configuration.rs); [scrapy-playwright README @ v0.0.48](https://github.com/scrapy-plugins/scrapy-playwright/blob/v0.0.48/README.md); [Crawl4AI async_configs.py @ v0.9.5](https://github.com/unclecode/crawl4ai/blob/v0.9.5/crawl4ai/async_configs.py#L942-L976) and [async_crawler_strategy.py](https://github.com/unclecode/crawl4ai/blob/v0.9.5/crawl4ai/async_crawler_strategy.py); [Firecrawl engines @ f4fdf7f](https://github.com/firecrawl/firecrawl/blob/f4fdf7f300415180f45272e893918dbfa3649073/apps/api/src/scraper/scrapeURL/engines/index.ts); [Katana README @ v1.8.0](https://github.com/projectdiscovery/katana/blob/v1.8.0/README.md); [Heritrix CHANGELOG](https://github.com/internetarchive/heritrix3/blob/master/CHANGELOG.md) **C**.

Two details matter for Goodfetch:
- **Version pinning is pushed to the user, and the vendor tests one version.** Crawlee pins Playwright 1.63.0 in its own catalog with a `minimumReleaseAge: 1440` (one day) for third-party packages, while users install any `playwright` ([pnpm-workspace.yaml @ v4.0.0-rc.1](https://github.com/apify/crawlee/blob/v4.0.0-rc.1/pnpm-workspace.yaml)) **C**. A pull request of 2026-10-05 had to "keep transitive playwright in sync with the pinned version" ([#4199](https://github.com/apify/crawlee/pull/4199)) **C**.
- **Heritrix records the browser's traffic through its own proxy.** Its `BrowserProcessor` drives the browser over WebDriver BiDi and "records all browser requests" through a recording proxy built on Jetty's ProxyHandler and its FetchHTTP2 module, so subresources stay under the crawler's network rules ([CHANGELOG 3.10.0](https://github.com/internetarchive/heritrix3/blob/master/CHANGELOG.md)) **C**. scrapy-playwright, by contrast, does not fire Scrapy's `headers_received` and `bytes_received` signals and routes proxies per browser or context, not through Scrapy's middlewares ([README known issues](https://github.com/scrapy-plugins/scrapy-playwright/blob/v0.0.48/README.md)) **C**.

---

## 4. Artifact A: a third-party library on the crawler's page

The question per product: can the handler hand the page it is given to any automation library, and when it does, what stays with the crawler?

| Product | What the handler receives | Path for a third-party library | Kept by the crawler | Lost or user-owned |
|---|---|---|---|---|
| Crawlee JS `PlaywrightCrawler` / `PuppeteerCrawler` | `page` (Playwright or Puppeteer object); v4 drops `browserController` from the context | In process: any library that accepts that `Page` object (Stagehand v3 did). Over CDP: only if the user adds `--remote-debugging-port` to `launchOptions.args`, then attaches by WebSocket and finds the tab | Proxy (set at launch or context), fingerprint injected at page creation, session cookies (v3 saved them after navigation and **before** the handler; v4 also after the handler), page closing, retries | Anything the other library opens itself (pages, contexts, its own proxy or stealth); no accounting of its LLM calls |
| Crawlee `StagehandCrawler` (JS and Python) | Playwright `page` with `act`/`extract`/`observe` attached | Stagehand v3 launches Chromium; Playwright attaches over CDP | Queue, retries, sessions, request lifecycle | Chromium only; "low-level browser properties cannot be modified"; only five launch options pass through; Python: no incognito pages, no canvas/WebGL/screen fingerprint; `remoteBrowser` rejected in v4; v3 only |
| Crawlee v4 remote pool | `page` from a remote browser | The crawler is the second client: `RemoteBrowserPool` attaches to a provider's CDP or Playwright WebSocket | Session rotation, request lifecycle, `maxOpenBrowsers`, release callback | `headless`, `launchOptions`, `userDataDir`; incognito forced |
| Crawlee Python `PlaywrightCrawler` | `context.page` (Playwright) | In process only; no CDP or remote option in 1.10.4 | As Crawlee JS | As Crawlee JS |
| Spider | `Page` from `subscribe()`; with `chrome_store_page`, `page.get_chrome_page()` returns the live Chromey page, closed by `close_page()` | Rust code can drive the Chromey page; another library must share a Chrome that Spider joined through `with_chrome_connection(ws_url)` | Spider's own stealth, fingerprint and interception, set on the page before navigation | Page lifetime moves to the subscriber; a bug report says `chrome_store_page` did not store the page ([spider#392](https://github.com/spider-rs/spider/issues/392), closed) |
| scrapy-playwright | `response.meta["playwright_page"]` when `playwright_include_page` is set | In process: any library that accepts a Playwright page; `PLAYWRIGHT_CDP_URL` to run on a browser started elsewhere; `PLAYWRIGHT_BROWSER_PROVIDER` for Patchright or Camoufox | Contexts and their proxy, `PLAYWRIGHT_MAX_PAGES_PER_CONTEXT` | The callback must close the page or the spider can stall at the page limit; no per-request proxy; Scrapy signals not fired |
| Crawl4AI | Playwright `page` and `context` in 9 hooks (`on_browser_created`, `on_page_context_created`, `before_goto`, `after_goto`, `on_execution_started`, `on_execution_ended`, `before_retrieve_html`, `before_return_html`, `on_user_agent_updated`) | `browser_mode="cdp"` with `cdp_url` attaches to a browser another tool launched; `create_isolated_context` forces a fresh context; `cdp_cleanup_on_close` decides whether to close it | Its stealth (playwright-stealth or Patchright) when it launched the browser | On a CDP attach, the existing contexts unless isolated |
| Firecrawl | No handler. `actions` (click, write, wait, screenshot, execute JavaScript) as JSON, only on Fire-engine | Cloud Browser Sandbox: a `cdpUrl` per session for Playwright or agent-browser | In the sandbox: proxies, profiles (one writer at a time) | Self-hosted: no actions, no sandbox |
| Katana | No handler (CLI) | `-chrome-ws-url` to use a browser "launched elsewhere with the debugger listening" | Scope, rate limits | Browser identity |
| Heritrix | No handler; Java `Behavior`s (scroll, extract links, paginate) | none documented | All browser traffic, through its recording proxy | — |

Sources: [Crawlee v4 browser management and cookies](https://github.com/apify/crawlee/blob/v4.0.0-rc.1/docs/upgrading/upgrading_v4.md); [Stagehand guide @ v4.0.0-rc.1](https://github.com/apify/crawlee/blob/v4.0.0-rc.1/docs/guides/stagehand_crawler.mdx); [Crawlee Python Stagehand guide](https://crawlee.dev/python/docs/guides/stagehand-crawler); [Stagehand v3 Playwright integration](https://docs.stagehand.dev/v3/integrations/playwright); [Spider page.rs, crate 2.53.9](https://docs.rs/crate/spider/2.53.9/source/src/page.rs); [scrapy-playwright README @ v0.0.48](https://github.com/scrapy-plugins/scrapy-playwright/blob/v0.0.48/README.md); [Crawl4AI hooks @ v0.9.5](https://github.com/unclecode/crawl4ai/blob/v0.9.5/crawl4ai/async_crawler_strategy.py#L100-L135); [Firecrawl Browser](https://docs.firecrawl.dev/features/browser); [Katana README](https://github.com/projectdiscovery/katana/blob/v1.8.0/README.md) **C**.

**What the third-party libraries expect.**
- **Stagehand v3** works on a Playwright, Puppeteer or Patchright page, provided that driver is connected over CDP to Stagehand's own browser ([v3 Playwright integration](https://docs.stagehand.dev/v3/integrations/playwright)) **C**. The October 7 note found that Crawlee's `page.agent()` wrapper does not bind the agent to the handler's page ([contracts follow-up §3](2026-10-07-crawlee-spider-contracts-followup.md)).
- **Stagehand v4** has "no Playwright interop"; its `page` option "only accepts a page from `browser.context`". It can attach to "any Chromium browser you are already running, by URL", loads its extension into it, needs `--enable-unsafe-extension-debugging` so the SDK can reach the extension's service worker, and closes only browsers it launched ([Playwright migration](https://docs.stagehand.dev/v4/migrations/playwright.md), [browser configuration](https://docs.stagehand.dev/v4/configuration/browser.md)) **C**.
- **Browser Use** (open source) attaches through `cdp_url` "for connecting to existing browser instance", with `keep_alive` to leave the browser running ([browser parameters](https://docs.browser-use.com/open-source/customize/browser/all-parameters.md)) **C**. Its core pins most dependencies exactly (`pydantic==2.13.5`, `aiohttp==3.14.3`, `anyio==4.12.1`, `openai==2.26.0` and others) ([PyPI 0.13.11](https://pypi.org/pypi/browser-use/0.13.11/json)), so installing it in the same environment as a crawler with lower-bound ranges is where conflicts appear (**inference**).

So for a library that wants a CDP URL, the crawler must launch its browser with a debugging port, which exposes cookies to any local process ([ADR 0017](../../docs/adr/0017-one-egress-policy-and-one-endpoint-rule.md) records why Goodfetch rejected an open debugging port), or the library launches and the crawler attaches, losing the crawler's launch-time stealth (Crawlee's Stagehand caveats).

---

## 5. Install size (measured)

npm sizes are the sum of every resolved package's `unpackedSize`, counting only the platform binary for linux-x64-glibc where a package ships one per platform. Python sizes are compressed wheels for CPython 3.12 on x86_64 manylinux. Browser binaries are not included (Playwright downloads them with `npx playwright install`; Puppeteer downloads Chrome at install time) **M**.

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
| PyPI `crawlee[playwright]` | 33 | 56.7 MB (Playwright wheel 46.0 MB, it bundles a Node driver) |
| PyPI `crawlee[stagehand]` | 40 | 92.7 MB (Stagehand 3.23.0 wheel 35.7 MB) |
| PyPI `crawlee[all]` | 129 | 202.5 MB |
| PyPI `scrapy==2.19.0` | 44 | 16.9 MB |
| PyPI `scrapy` + `scrapy-playwright` | 48 | 63.6 MB |
| PyPI `crawl4ai==0.9.5` (core) | 95 | 207.4 MB (Playwright 46.0, Patchright 46.0, SciPy 33.7, `unclecode-litellm` 17.2) |
| PyPI `firecrawl-py` (cloud client) | 26 | 4.4 MB |
| crates.io `spider` 2.53.9 crate archive | 1 | 0.78 MB compressed (the dependency tree depends on features; not measured) |

The scripts and lockfiles are in the session scratchpad, not in the repository.

---

## 6. Version lockstep

| Product | Lockstep? | Evidence |
|---|---|---|
| Crawlee JS | Yes: Lerna fixed mode, every `@crawlee/*` package at one version, published within minutes (3.18.2 on 2026-09-29 10:01 to 10:08 UTC) | [lerna.json @ v4.0.0-rc.1](https://github.com/apify/crawlee/blob/v4.0.0-rc.1/lerna.json), npm publish times **M** |
| Crawlee JS v4 extracted packages | `@crawlee/jsdom` and `@crawlee/linkedom` move to their own repositories and release processes | [#4233](https://github.com/apify/crawlee/issues/4233) **C** |
| Crawlee Python | One distribution; Apify SDK Python requires `crawlee>=1.8.0,<2.0.0,!=1.10.1` | [PyPI `apify` 4.1.0](https://pypi.org/pypi/apify/4.1.0/json) **C** |
| Spider | Engine, CLI, agent, MCP, utils and worker crates in lockstep at 2.53.9; Chromey and the fingerprint, firewall and blocker crates on their own numbers; bindings on 0.0.x | crates.io **M** |
| Scrapy | No: Scrapy, scrapy-playwright, scrapy-splash, scrapy-zyte-api release independently | PyPI **M** |
| Browser libraries | Never in lockstep with any crawler: peer `*` or lower bounds, except Crawlee's Stagehand peer capped at the previous major | npm, PyPI **M** |

---

## 7. Recurring complaints

- **"Install the browser" on first run.** [crawlee#2275](https://github.com/apify/crawlee/issues/2275) (open since 2024-01-08, 21 comments): a project with `crawlee` and `playwright` "fails on first start asking to `npx install playwright`" **C**.
- **Remote and third-party browsers.** [crawlee#1822](https://github.com/apify/crawlee/issues/1822) "Connect to remote browser services" (24 reactions, closed by the v4 remote pool); [crawlee#4196](https://github.com/apify/crawlee/issues/4196) (2026-10-05, open) asks to forward `cdpUrl` so `StagehandCrawler` can use a remote browser; [crawlee-python#1743](https://github.com/apify/crawlee-python/issues/1743) and [#1744](https://github.com/apify/crawlee-python/issues/1744) (redesign the browser pool) are open **C**.
- **Bundling and runtimes.** [crawlee#2208](https://github.com/apify/crawlee/issues/2208) "Support bundling" (open, 10 reactions); [#1913](https://github.com/apify/crawlee/issues/1913) Deno support (10 reactions) **C**.
- **Weight of rarely used parsers** paid by every install: the rationale of [crawlee#4113](https://github.com/apify/crawlee/issues/4113) **C**.
- **Stagehand major lag.** Both Crawlee integrations cap Stagehand below 4 while Stagehand 4.2.1 is current (§1) **M**.
- **Spider defaults that break environments.** [spider#243](https://github.com/spider-rs/spider/issues/243): "Operation not permitted from tokio-uring" in a Docker container (15 comments), from `io_uring` in the default features **C**. [spider#294](https://github.com/spider-rs/spider/issues/294): many errors from Chromiumoxide with a remote Chrome (10 comments) **C**. [spider-py#18](https://github.com/spider-rs/spider-py/issues/18): "Build failure with current spider" (2026-01-19) **C**.
- **scrapy-playwright hangs.** [#266](https://github.com/scrapy-plugins/scrapy-playwright/issues/266) "Scrapy hangs with no exception raised", [#343](https://github.com/scrapy-plugins/scrapy-playwright/issues/343) stuck on Windows with Scrapy 2.13 (18 comments), [#244](https://github.com/scrapy-plugins/scrapy-playwright/issues/244) hangs when a page method raises **C**.
- **Firecrawl self-host parity.** Actions, Agent, Browser and "specialized formats" need Firecrawl Cloud or an external service ([self-host guide](https://docs.firecrawl.dev/contributing/self-host)) **C**.

---

## 8. Artifact B: open-source core versus cloud, against Foxhound's capabilities

Cells: **OSS** (in the open-source framework), **Cloud** (only in the vendor's hosted service), **Both**, **Plugin** (a separately installed open-source package), **No**.

| Foxhound capability | Crawlee JS + Apify | Crawlee Python + Apify | Spider + Spider Cloud | Scrapy + Zyte | Firecrawl | Crawl4AI |
|---|---|---|---|---|---|---|
| Frontier (queue, visited, URL rules) | Both (RequestQueue local; Apify request queue) | Both (memory, file, SQL, Redis clients) | OSS (in memory; SQLite visited set with `disk`) | OSS (scheduler, dupefilter, `JOBDIR`) | Both (NuQ PostgreSQL self-hosted) | OSS (deep-crawl strategies) |
| Politeness, per domain | OSS (`sameDomainDelaySecs`, v4 `ThrottlingRequestManager`) | OSS (opt-in throttling) | OSS (delays; Crawl-delay and `Retry-After` honoured) | OSS (download slots, AutoThrottle) | Both | OSS (rate limiter) |
| Politeness store across machines | No | No (queue shared by SQL/Redis, not pacing) | No | Plugin (Scrapy Cluster, Redis) | No | No |
| Fetch tiers with escalation | OSS (`AdaptivePlaywrightCrawler`) | OSS (adaptive crawler) | Both (`smart` OSS; `spider_cloud` Smart mode escalates to the paid unblocker) | Plugin (scrapy-zyte-api escalates to the paid Zyte API) | Both (fetch, Playwright OSS; Fire-engine Cloud) | OSS (HTTP and browser strategies) |
| Datasets | Both (local files; Apify datasets) | Both | OSS (page stream; no dataset object) | Both (feeds; Scrapy Cloud items) | Cloud (no result store self-hosted) | No (results returned) |
| Sinks | Cloud (Apify integrations) | OSS SQL/Redis storage clients; Cloud integrations | Cloud (data connectors: S3, GCS, Sheets, Azure Blob, Supabase) | OSS (feed exports: S3, GCS, FTP) | Both (webhooks; retries Cloud) | OSS (Docker server webhooks) |
| Search over results | No | No | No | No | No | No |
| Response cache | No (v4 issue on Impit cache only) | No | OSS (`cache`, `cache_mem`, `cache_chrome_hybrid`) | OSS (`HttpCacheMiddleware`) | Cloud (`maxAge`) | OSS (SQLite cache) |
| Console (web UI) | Cloud (Apify Console) | Cloud | Cloud (dashboard) | Cloud (Scrapy Cloud); Scrapyd minimal UI | Cloud (dashboard); self-hosted queue admin UI off by default | OSS (Docker server playground and monitor) |
| Daemon (long-lived shared server) | No | No | Plugin (`spider_worker`, HTTP fetch worker) | Plugin (Scrapyd) | OSS (the server itself) | OSS (Docker API server) |
| Schedules | Cloud (Apify Schedules, cron) | Cloud | OSS (`cron` feature) | Plugin (Scrapyd API, no cron); Cloud periodic jobs (paid units only) | Cloud (`/v2/monitor`) | No |

Sources: earlier notes for frontier, politeness and sinks ([frontier and sessions](https://github.com/goodfetch/automan/blob/research/frontier-sessions-competitors/rnd/research/2026-10-10-frontier-sessions-competitors.md), [storage and sinks](https://github.com/goodfetch/automan/blob/research/storage-sinks-competitors/rnd/research/2026-10-10-storage-sinks-competitors.md)); ADRs [0036](../../docs/adr/0036-the-daemon-schedules-runs-above-the-engine.md), [0046](../../docs/adr/0046-a-per-user-response-cache-off-by-default-with-revalidate-and-replay.md), [0053](../../docs/adr/0053-politeness-is-shared-by-every-run-of-a-user-and-across-machines-through-a-store.md); [Apify Schedules](https://docs.apify.com/platform/schedules); [Spider Cloud API](https://spider.cloud/docs/api); [Scrapy Cloud pricing](https://docs.zyte.com/scrapy-cloud/pricing.md) (free plan: 7-day retention, 1-hour jobs; paid unit $9/month: 120 days, scheduling); [Firecrawl SELF_HOST.md](https://github.com/firecrawl/firecrawl/blob/f4fdf7f300415180f45272e893918dbfa3649073/SELF_HOST.md) **C**. Cells marked as draft below are still being checked.

---

## 9. Implications for Goodfetch

Considerations for the decision, not decisions.

- **Making Collie a separately installable product does not by itself match the market.** The comparable split is Crawlee and Playwright: two vendors, a peer dependency with range `*`, and a first-run failure when the browser is missing (crawlee#2275). Shipping Collie inside Foxhound's binary removes the peer-version problem that Crawlee pushes to its users, at the cost of Foxhound's install carrying a browser layer. Crawlee v4's own direction is to move rarely used parsers out of the default install (#4113), not the browser layer.
- **A handler that lends its page needs both a page handle and an endpoint.** In-process libraries take a page object (Stagehand v3, Playwright-compatible drivers); out-of-process ones take a CDP URL (Browser Use, Stagehand v4). ADR 0019 already serves a raw CDP WebSocket endpoint for Playwright and Puppeteer, which is the endpoint half; none of the surveyed crawlers serves one per page, so they leave the user to add `--remote-debugging-port` and find the tab.
- **Stagehand v4 is a moving target for "lend the page".** It needs its extension in the browser and `--enable-unsafe-extension-debugging` on the browser's command line, and it refuses foreign page objects; Crawlee has not followed it past v3 in either language. A Goodfetch integration test against Stagehand v4's `localBrowser.connect` would settle whether lending works without launching the browser for Stagehand.
- **What the crawler keeps when another library drives the page is the contract to write down.** Crawlee keeps launch-time proxy and fingerprint and syncs cookies at fixed points; it loses anything the other library opens itself. Heritrix keeps all subresource traffic by recording through its own proxy, the closest model to ADR 0017's engine-owned egress proxy for raw CDP Sessions.
- **Cloud lines are drawn at scheduling, UI, durable results and the hard fetch tier.** ADR 0036 (Schedules in the Daemon), ADR 0031 (a local Console) and ADR 0053 (a shared Politeness store) put in the open-source framework what every surveyed competitor keeps in its cloud or leaves out.

---

## 10. Unverified / open

- Whether Stagehand v4 can act on a tab another client opened (it attaches by URL, but its `page` option accepts only its own pages): **Unverified**; not run.
- Whether `@spider-rs/spider-rs` 0.0.163 installs a working native binary without the platform packages its predecessor declared: **Unverified**; registry metadata only.
- Fingerprint injection on Crawlee v4 remote browsers: the guide lists what does not apply but does not mention fingerprints: **Unverified**.
- Browser download sizes (Playwright's Chromium and headless shell, Puppeteer's Chrome): not measured.
- Draft cells in §8 to re-check: Crawl4AI Datasets, Firecrawl politeness, Spider Cloud dashboard and scheduling.

---

## Sources

Registries: [npm `crawlee`](https://registry.npmjs.org/crawlee), [npm `@crawlee/stagehand`](https://registry.npmjs.org/@crawlee/stagehand), [npm `@browserbasehq/stagehand`](https://registry.npmjs.org/@browserbasehq/stagehand), [npm `apify`](https://registry.npmjs.org/apify), [npm `@spider-rs/spider-rs`](https://registry.npmjs.org/@spider-rs/spider-rs), [PyPI `crawlee`](https://pypi.org/project/crawlee/), [PyPI `crawl4ai`](https://pypi.org/project/crawl4ai/), [PyPI `scrapy-playwright`](https://pypi.org/project/scrapy-playwright/), [PyPI `browser-use`](https://pypi.org/project/browser-use/), [crates.io `spider`](https://crates.io/crates/spider), [Go proxy Katana](https://proxy.golang.org/github.com/projectdiscovery/katana/@latest), [Go proxy Colly](https://proxy.golang.org/github.com/gocolly/colly/v2/@latest).

Code and docs: cited inline.
