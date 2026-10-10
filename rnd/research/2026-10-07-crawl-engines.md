# Crawl, extraction and automation engines: source audit

State observed on **October 7, 2026**. Scope: Crawlee TS/Python, Spider/Chromey, Scrapy, Crawl4AI, Firecrawl, Scrapling, ScrapeGraphAI, Colly, Rod, chromedp, Ferret and more specialized alternatives. The goal is an open source framework usable as a product, with an ambitious architecture developed by several agents.

**Conclusion for the ambitious target: a canonical Rust engine behind a sidecar protocol, generated SDKs, native HTTP and CDP, and a separate workflow orchestration is a coherent direction.** It does not by itself prove better performance. The core must reuse proven building blocks and define a more rigorous action and result contract, with Playwright as the reference and a qualified compatibility backend. HTTP-first, autoscaling, pooling, selector recovery and replay already exist separately: recombining them without a measurable quality contract is not enough to establish superiority.

## Method and limits

- Exa search: **80 results requested** over 8 calls, i.e. 10 + 10 + 15 + 10 + 5 + 10 + 10 + 10. `sources_reviewed = 80` in the sense of the requested counter, which represents the volume of search results, not 80 fully audited sources or 80 unique sources.
- Primary inspection: README, LICENSE, manifest files and critical paths downloaded from GitHub; repository metadata; published versions verified on npm, PyPI, crates.io and the Go proxy. The published code of Crawlee 3.18.2 and the Spider 2.53.9 crate archive were examined, in order to distinguish an available feature from something new on the main branch.
- GitHub `pushed_at` dates are repository activity dates, **not necessarily the date of the last functional change**. An active repository and a published release are two distinct observations.
- No comparative local benchmark was run. Maintainers' figures are qualified according to their protocol; they do not prove a gain on a business journey, on a protected site, or on long sessions.
- Some GitHub Releases queries hit the API limit. The registries made it possible to verify the main versions; the latest stable Ferret v1 release and several niche versions were not established here.

## A map of layers, not a list of interchangeable products

| Project | Main layer | HTTP | Browser / actions | LLM | Orchestration / reliability | Observed license |
|---|---|---|---|---|---|---|
| [Crawlee TS](https://github.com/apify/crawlee) | Crawl framework | Http/Cheerio/JSDOM/LinkeDOM, injectable client | Playwright/Puppeteer, pools; Stagehand separate | Stagehand integration | Queue, sessions, retries, autoscaling, storage | Apache-2.0 |
| [Crawlee Python](https://github.com/apify/crawlee-python) | Crawl framework | Parsel/BeautifulSoup, HTTP clients | Playwright, Stagehand | Optional integrations | Queue and memory/file/SQL/Redis clients, pools | Apache-2.0 |
| [Spider](https://github.com/spider-rs/spider) | Rust collection engine | reqwest, streaming, caches and hooks | Chrome via Chromey, smart mode | Optional features and agent crate | Concurrency, budgets, deduplication, page broadcasting, distributed mode | MIT |
| [Chromey](https://github.com/spider-rs/chromey) | Rust CDP driver | Transport/cache primitives around the browser | Chrome/Chromium via CDP | Not its main role | Lifecycle, events, network, cache, fingerprinting | MIT OR Apache-2.0 in Cargo |
| [spider-browser](https://github.com/spider-rs/spider-browser) | Remote browser SDK | Via the service | Cloud page API | act/observe/extract/agent | Retry and escalation in the cloud offering | MIT for the SDKs examined |
| [Scrapy](https://github.com/scrapy/scrapy) | Python crawl framework | Downloader, middlewares, scheduling | Via integrations, notably scrapy-playwright | Extensions | Scheduler, pipelines, AutoThrottle, resumable jobs | BSD-3-Clause |
| [Crawl4AI](https://github.com/unclecode/crawl4ai) | Web extraction into corpora for AI | HTTP strategy and collection tools | Playwright, remote CDP, sessions, scripts | Optional extraction and semantic strategy | Memory dispatcher, rate limiting, cache | Apache announced + attribution added to LICENSE |
| [Firecrawl](https://github.com/firecrawl/firecrawl) | Extraction/crawl service with API | Fetch and several engines | Browser service, actions depending on engine | Extract/agent and other features depending on plan | Queue and workers, engine selection, formats | AGPL-3.0 core; SDKs and some components MIT |
| [Scrapling](https://github.com/D4Vinci/Scrapling) | Adaptive parsing + fetchers + Python spiders | curl_cffi and sessions | DynamicFetcher, StealthyFetcher with Patchright | Optional; adaptive parsing without LLM | Deduplication, checkpoint, per-domain throttling | BSD-3-Clause |
| [ScrapeGraphAI](https://github.com/ScrapeGraphAI/Scrapegraph-ai) | Model-assisted extraction graphs | Document acquisition | Browser backend / integrations | Central in SmartScraperGraph | Graph of fetch, parsing, answer nodes | MIT for this repository |
| [Colly](https://github.com/gocolly/colly) | Go HTTP crawl framework | Yes, callbacks/transport | Not an integrated JS engine equivalent to Playwright | Not its role | Limits, cookies, cache, storage extensions | Apache-2.0 |
| [Rod](https://github.com/go-rod/rod) / [chromedp](https://github.com/chromedp/chromedp) | Go CDP drivers | Browser transport | Chrome/Chromium | Not their role | Contexts, events, waits, cancellation | MIT |
| [Ferret](https://github.com/MontFerret/ferret) | Declarative language/runtime | Integrations/capabilities | Drivers/capabilities | Not required | Compiled plans and sessions in v2 alpha | Apache-2.0 |

This reading avoids two mistakes: comparing a cloud SDK to a self-hostable engine, and comparing the speed of an HTML parser to that of a full browser agent.

## Crawlee: the architectural competitor to study first

### What is already solved

The code separates the crawler, sessions, storage, transports and the browser pool. `AutoscaledPool` adjusts concurrency according to available resources; `BrowserPool` handles creating and retiring browsers and exposes hooks. The queue has deduplication, recovery and locking logic. It is cheaper to compose these elements than to immediately rewrite a universal scheduler. [AutoscaledPool](https://github.com/apify/crawlee/blob/master/packages/basic-crawler/src/internals/autoscaling/autoscaled_pool.ts), [BrowserPool](https://github.com/apify/crawlee/blob/master/packages/browser-pool/src/browser-pool.ts), [RequestQueue](https://github.com/apify/crawlee/blob/master/packages/core/src/storages/request_queue.ts).

**HTTP-first with browser fallback is no longer a differentiator in itself.** `AdaptivePlaywrightCrawler` is present in the published 3.18.2 release. It uses a handler able to run on a limited HTTP context or a browser. Accessing `page` in HTTP mode raises an error that enables the fallback. A `resultChecker` validates the output; `resultComparator` compares the outputs of the two modes. [Published code](https://github.com/apify/crawlee/blob/v3.18.2/packages/playwright-crawler/src/internals/adaptive-playwright-crawler.ts), [release 3.18.2](https://github.com/apify/crawlee/releases/tag/v3.18.2).

The mechanism is more precise than simple detection of `<script>` tags:

1. The predictor uses the host, the URL path segments and the request label. Segment similarity serves as a feature for a logistic regression.
2. A URL with no history starts in browser mode and with a high detection probability.
3. For a detection, the browser handler is run, then the HTTP handler on a comparable state. The difference between outputs is used to learn which mode is sufficient.
4. By default, the detection ratio is 0.1; uncertainty and a low volume of observations increase the checks.
5. A "static" prediction allows subsequent requests to be handled by HTTP only, with a return to the browser in case of an invalid result or a relevant error. [Published predictor](https://github.com/apify/crawlee/blob/v3.18.2/packages/playwright-crawler/src/internals/utils/rendering-type-prediction.ts).

The most relevant limit for Automan: in release 3.18.2, the default comparison covers the `datasetItems`, their count and their deep equality. This is not a universal proof of equivalence between two journeys. A handler that produces no item may yield an uninformative comparison. A product must therefore express its expected results, rather than rely on this default. The code exposes callbacks for this; this observation does not demonstrate a Crawlee bug. [Comparator and execution](https://github.com/apify/crawlee/blob/v3.18.2/packages/playwright-crawler/src/internals/adaptive-playwright-crawler.ts#L336).

**Another major limit: the handler may be executed several times.** The in-code documentation forbids side effects other than those provided by the context, and captures writes so as to commit only the intended results. This strategy suits extraction; it does not guarantee that an external action, such as creating a record on the site, can be replayed. Automan must keep a separate contract for action steps, their preconditions, their postconditions and their resumption. [Handler contract](https://github.com/apify/crawlee/blob/v3.18.2/packages/playwright-crawler/src/internals/adaptive-playwright-crawler.ts#L167).

The Python version 1.10.4 also has an `AdaptivePlaywrightCrawler` and a dedicated predictor. It is therefore not a simple port limited to Playwright. [Python implementation](https://github.com/apify/crawlee-python/blob/master/src/crawlee/crawlers/_adaptive_playwright/_adaptive_playwright_crawler.py), [Python predictor](https://github.com/apify/crawlee-python/blob/master/src/crawlee/crawlers/_adaptive_playwright/_rendering_type_predictor.py).

### Watch out for Stagehand and versions

`@crawlee/stagehand` is published at **3.18.2**, with a peer dependency `@browserbasehq/stagehand: ^3.0.0`. The integration is therefore **not a validation of compatibility with Stagehand 4**. The Stagehand package does not appear among the dependencies of the main `crawlee` bundle: it is chosen separately. The local launch code uses Stagehand options and plugs into the Crawlee pool ecosystem. [Published package/tag](https://github.com/apify/crawlee/blob/v3.18.2/packages/stagehand-crawler/package.json), [npm manifest](https://registry.npmjs.org/@crawlee/stagehand/latest), [launcher](https://github.com/apify/crawlee/blob/master/packages/stagehand-crawler/src/internals/stagehand-launcher.ts).

The main branch also includes `remote-browser-pool.ts` and remote provider abstractions. Their presence confirms the orientation toward interchangeable infrastructures; their exact status in a given release must be verified before making them an MVP dependency. [Source](https://github.com/apify/crawlee/blob/master/packages/browser-pool/src/remote-browser-pool.ts).

### Impit: Rust without rewriting the product in Rust

Apify now recommends `ImpitHttpClient` for new projects, and states that got-scraping is no longer actively maintained. Impit performs HTTP and TLS impersonation, offers HTTP/3 as an option, and provides native Node binaries for Windows/macOS/Linux. The license of the repository and of the npm package is Apache-2.0; npm observed: `impit` 0.14.5, `@crawlee/impit-client` 3.18.2. [Official guide](https://crawlee.dev/js/docs/guides/impit-http-client), [Impit code](https://github.com/apify/impit), [license](https://github.com/apify/impit/blob/master/LICENSE.md).

This weakens the argument "the TS controller would necessarily be slow". A TS SDK can already delegate its transport to a native engine. The useful question is the CPU/memory/latency share of each step, with a real browser and a real network.

## Spider and Chromey: low-level references, integration cost to be measured

Spider published on crates.io: **2.53.9**, September 5, 2026. The main branch announces 2.54.0. Chromey published: **2.58.2**, September 12, 2026; the branch announces 2.58.3. The contents of the published archives were examined, including smart mode and the concurrency controller. [Spider crate](https://crates.io/crates/spider/2.53.9), [published source](https://docs.rs/crate/spider/2.53.9/source/), [Chromey crate](https://crates.io/crates/chromey/2.58.2).

The engine does not boil down to "reqwest + a parser". Its features include Chrome, request interception, hybrid caches, streaming, budgets, distributed mode, execution controls, page borrowing, WebDriver and several collection paths. `RemoteFetcher` makes it possible to replace the HTTP round-trip while keeping the scheduling; its contract specifies that the hook applies to the HTTP path and that the provider remains responsible for its cancellation. [Manifest](https://github.com/spider-rs/spider/blob/main/spider/Cargo.toml), [RemoteFetcher contract](https://github.com/spider-rs/spider/blob/main/spider/src/fetcher.rs).

Its `smart` mode notably uses heuristic signals: `noscript` text, scripts that mutate the DOM, data requests, Next/Nuxt/Gatsby markers and framework attributes. These signals feed an upgrade-to-Chrome score. This differs from the Crawlee model, which compares the extraction result. A server-rendered React page can contain hydration markers without the browser being necessary for the requested result; conversely, a seemingly plausible HTTP extraction can miss a piece of data. These are cases to measure, not failures established here. [Detection in page.rs](https://github.com/spider-rs/spider/blob/main/spider/src/page.rs#L1876), [smart processing](https://github.com/spider-rs/spider/blob/main/spider/src/page.rs#L8305).

The required Cargo feature is **`smart`**, which enables Chrome and interception. Calling `crawl_smart()` without this feature follows another compiled path that falls back to `crawl()`. The main-branch README says in one passage to enable only `chrome`; the README included in the published crate correctly documents `smart`. For an integration, the manifest and the compiled code take precedence over this simplified wording. [Feature flags](https://github.com/spider-rs/spider/blob/main/spider/Cargo.toml#L312), [conditional variants](https://github.com/spider-rs/spider/blob/main/spider/src/website.rs#L7625).

Adaptive concurrency is not just a configurable integer: an **AIMD** controller and an `AdaptiveSemaphore` are present. The contract exposes a progressive increase and a multiplicative decrease, as well as an atomic bridge to the live semaphore. The application can drive this mechanism from observed results; the mere existence of the types does not prove optimal autoscaling on every workload. [Source](https://github.com/spider-rs/spider/blob/main/spider/src/utils/adaptive_concurrency.rs).

Chromey is a fork of chromiumoxide. It keeps the library name `chromiumoxide` while publishing the `chromey` package, and adds fingerprint, network, blocking, cache and browser control components. Its Cargo license is `MIT OR Apache-2.0`, even though GitHub displays Apache in its metadata. The Spider fingerprint/firewall/network blocker crates examined declare MIT. [README](https://github.com/spider-rs/chromey/blob/main/README.md), [Cargo](https://github.com/spider-rs/chromey/blob/main/Cargo.toml), [upstream](https://github.com/mattsse/chromiumoxide).

The hidden cost of a deep fork is the compilation matrix and synchronization with CDP and Chrome. Spider's main code is large and has many feature branches: the `website.rs` examined approaches 18,500 lines. This size does not establish poor quality, but taking over this entire system means tracking a technical surface broader than Automan's need. A team of agents speeds up implementation; it removes neither races between events, nor validation on real Chrome, nor fingerprint changes. Choosing a narrow adapter or contributing upstream preserves a responsibility boundary.

### Choosing a Rust base without taking over the whole Spider stack

**Chromiumoxide 0.9.1** has been published since February 25, 2026 and is also `MIT OR Apache-2.0`. **rust-headless-chrome 1.0.22**, published on June 11, 2026, is MIT and constitutes another reference, with an ergonomics and execution model to compare against Tokio needs. [Chromiumoxide](https://crates.io/crates/chromiumoxide/0.9.1), [rust-headless-chrome](https://crates.io/crates/headless_chrome/1.0.22).

The initial choice can be: **Chromiumoxide to establish a clean stock CDP baseline; Chromey as a candidate if its network, cache and streaming functions bring a measured gain; Spider as a crawl adapter rather than as a global fork of the runtime.** The first two descend from the same family and require testing their actually published versions. The Chromey fork appears to be published more frequently at this date; the number of stars or the frequency of tags does not measure the success rate of actions.

In both drivers, the inspected `Element::click()` path performs `scroll_into_view`, looks for a clickable point, then sends the click. This path alone does not constitute a contract equivalent to Playwright's locators and actionability. **The important work for Automan is above the CDP driver:** resolving the locator at action time, visibility/enablement/stability, hit testing, frames and out-of-process targets, shadow DOM, arming waits before the click, handling detached elements, then checking the resulting state. The full coverage of the drivers must be verified before asserting that all these mechanisms are absent; the inspection only establishes that the cited click method does not guarantee all of them. [Chromey Element](https://github.com/spider-rs/chromey/blob/main/src/element.rs#L257), [Chromiumoxide Element](https://github.com/mattsse/chromiumoxide/blob/main/src/element.rs#L269).

**Another concrete boundary: Chromey's vendor methods are not guaranteed functions of stock Chrome.** `Content.getMarkdown` and `WebMCP.listTools/callTool` explicitly belong to non-standard CDP domains exposed by certain remote engines. The library documents the unsupported case and the fallback. The sidecar must negotiate capabilities; neither the presence of the bindings nor the WebMCP name proves standard local support. [Content domain](https://github.com/spider-rs/chromey/blob/main/src/content_markdown.rs), [WebMCP domain](https://github.com/spider-rs/chromey/blob/main/src/webmcp.rs).

**spider-browser is something else**: the TypeScript SDK connects by default to `wss://browser.spider.cloud`, and the Rust README explicitly describes a cloud fleet client. Page and AI methods, retry and proxy escalation are exposed in the offering; the MIT SDK repository does not by itself constitute a self-hostable fleet engine. The term "Rust-powered browsers" does not mean that Chromium is reimplemented in Rust. [TypeScript SDK](https://github.com/spider-rs/spider-browser/blob/main/typescript/spider-browser.ts), [Rust README](https://github.com/spider-rs/spider-browser/blob/main/rust/README.md).

## The other major references

### Scrapy

PyPI version observed: **2.19.0**, published on September 10, 2026. The sources show the engine, the scheduler/downloader/scraper boundaries and backpressure. The scheduler combines a memory/disk queue and a duplicate filter; AutoThrottle adjusts the delay based on latency and target concurrency. The recent branch also includes async APIs: summarizing it only as "old Twisted" would miss its evolution. [Engine](https://github.com/scrapy/scrapy/blob/master/scrapy/core/engine.py), [scheduler](https://github.com/scrapy/scrapy/blob/master/scrapy/core/scheduler.py), [throttle](https://github.com/scrapy/scrapy/blob/master/scrapy/extensions/throttle.py), [package](https://pypi.org/project/Scrapy/).

Its value for Automan is the discipline of boundaries, hooks and persistent queues. Browser rendering requires a suitable integration; it is not equivalent to an agent loop or a compiled chain of actions.

### Crawl4AI

Published version observed: **0.9.4**, September 23, 2026. Its strength is acquiring and transforming the Web into clean content for models, with possible deterministic extraction, sessions and CDP. The code handles reuse of Playwright/CDP connections, contexts and sessions. The dispatcher protects memory with thresholds, hysteresis and a session limit. These elements should be studied before proposing "pooling" or "memory-aware concurrency" as a novelty oneself. [BrowserManager](https://github.com/unclecode/crawl4ai/blob/main/crawl4ai/browser_manager.py), [dispatcher](https://github.com/unclecode/crawl4ai/blob/main/crawl4ai/async_dispatcher.py), [strategy](https://github.com/unclecode/crawl4ai/blob/main/crawl4ai/async_crawler_strategy.py).

Its "adaptive crawling" includes a search guided by coverage and relevance, distinct from Crawlee's HTTP/browser choice. The project also provides **C4A-Script**, an interaction DSL with visual tools and recording: a DSL/replay is therefore not an empty space. [Adaptive code](https://github.com/unclecode/crawl4ai/blob/main/crawl4ai/adaptive_crawler.py), [C4A-Script documentation](https://github.com/unclecode/crawl4ai/blob/main/docs/md_v2/core/c4a-script.md).

**Concrete license point:** the LICENSE file starts with Apache, then adds an attribution requirement for distributions, publications and public uses, with required visibility. The README says attribution is both recommended and mandatory. Avoid treating this repository as standard Apache-2.0 without examining this addition when reusing it. [LICENSE](https://github.com/unclecode/crawl4ai/blob/main/LICENSE#L54), [README](https://github.com/unclecode/crawl4ai/blob/main/README.md#L372).

### Firecrawl

The latest GitHub Release returned by the API was **v2.11.0**, June 19, 2026; the main-branch sources are more recent. Firecrawl is a product/API and multi-format extraction reference, but its core is AGPL-3.0, while the SDKs and some components are MIT. Licenses cannot be inferred from the npm/Python client alone. [README/licenses](https://github.com/firecrawl/firecrawl/blob/main/README.md#L886), [release](https://github.com/firecrawl/firecrawl/releases/tag/v2.11.0).

The examined branch chooses engines according to their capabilities and several priorities: fetch, Playwright, fire-engine/CDP/TLS engines and specialized parsers. The fire-engine integration uses a remote service; its engine is not implemented in the wrapper files examined. The README explicitly distinguishes cloud and open source. A self-hosted reproduction must therefore not be deemed equivalent to the cloud without proof. [Engine registry](https://github.com/firecrawl/firecrawl/blob/main/apps/api/src/scraper/scrapeURL/engines/index.ts), [fire-engine wrapper](https://github.com/firecrawl/firecrawl/blob/main/apps/api/src/scraper/scrapeURL/engines/fire-engine/index.ts).

The main-branch SELF_HOST announces NuQ PostgreSQL for the queue and scraper-microservice as the default browser engine, with Redis/RabbitMQ and other Compose services. Systematically describing the current architecture as BullMQ would be outdated. These observations concern `main`, not an installation validation of release 2.11.0. [SELF_HOST](https://github.com/firecrawl/firecrawl/blob/main/SELF_HOST.md).

### Scrapling

PyPI version: **0.4.15**, August 23, 2026; BSD-3-Clause repository. Three concerns are separable: parsing and element recovery, HTTP transport, browser transport. `StealthyFetcher` imports Patchright and adds challenge logic; `Fetcher` uses transport sessions. Success against changing protections cannot be inferred from the presence of these functions. [Fetchers](https://github.com/D4Vinci/Scrapling/tree/main/scrapling/fetchers), [stealth engine](https://github.com/D4Vinci/Scrapling/blob/main/scrapling/engines/_browsers/_stealth.py).

Adaptivity here means **finding an element again after an HTML change**, from stored signatures, without requiring an LLM. It is a direct competitor to the "selector auto-healing" part. One must verify that the recovered element keeps the expected role and value; structural similarity and business correctness are two properties. [Adaptive documentation](https://github.com/D4Vinci/Scrapling/blob/main/docs/parsing/adaptive.md).

The spiders framework has deduplication, checkpoints and resumption, per-domain quotas and AutoThrottle. The latter adjusts delays and takes blocks and Retry-After into account. The README benchmarks are **parsing / element search** benchmarks, not results of complete agents. [Scheduler](https://github.com/D4Vinci/Scrapling/blob/main/scrapling/spiders/scheduler.py), [engine](https://github.com/D4Vinci/Scrapling/blob/main/scrapling/spiders/engine.py), [benchmarks](https://github.com/D4Vinci/Scrapling/blob/main/benchmarks.py).

### ScrapeGraphAI

PyPI version: **2.3.1**, October 6, 2026. `SmartScraperGraph` assembles FetchNode, ParseNode, GenerateAnswerNode and optional reasoning/regeneration branches. The value is going from an instruction to an extraction, with models at the center; it is not automatically the best path for known, repeated work whose extraction can be compiled. The OSS repository is MIT; the associated ScrapeGraphAI services are not to be confused with the entirety of this repository's code. [Inspected graph](https://github.com/ScrapeGraphAI/Scrapegraph-ai/blob/main/scrapegraphai/graphs/smart_scraper_graph.py), [README](https://github.com/ScrapeGraphAI/Scrapegraph-ai), [LICENSE](https://github.com/ScrapeGraphAI/Scrapegraph-ai/blob/main/LICENSE).

## Niche players and building blocks it would be a shame to ignore

| Tool | Why study it | Caveat / boundary |
|---|---|---|
| [Colly](https://github.com/gocolly/colly) | Simple Go callbacks, per-domain limits, cookies, cache and customizable transport | HTTP-centric. Public call for maintainers in March 2025; recent activity ≠ no maintenance risk. [Issue](https://github.com/gocolly/colly/issues/840) |
| [Rod](https://github.com/go-rod/rod) | Go CDP, pages/events/waits, direct interface | Latest tag returned by the Go proxy: v0.116.2, July 2024, despite repository activity in August 2026 |
| [chromedp](https://github.com/chromedp/chromedp) | Go contexts and CDP tasks; useful to compare simplicity, lifecycle and cost | Chrome-centric, not a universal orchestration. Go proxy v0.20.1 dated October 5, 2026 |
| [Geziyor](https://github.com/geziyor/geziyor) | Go HTTP + JS rendering, extraction, export and concurrency | MPL-2.0; latest Go proxy version observed is a pseudo-version from August 2024. Examine version and modified files before reuse |
| [Ferret](https://github.com/MontFerret/ferret) | DSL, reusable plans, bytecode VM, isolated executions, capabilities exposed by the host | `main` = v2 alpha; stable v1 on branch `v1`. An inspiration for compilation/replay, not an acquired stable v2 dependency |
| [Katana](https://github.com/projectdiscovery/katana) | Hybrid HTTP and headless, discovery of JS/XHR endpoints, scope and form filling | Product oriented toward reconnaissance pipelines; headless announced as experimental. MIT. [Docs](https://docs.projectdiscovery.io/opensource/katana/usage) |
| [Apache StormCrawler](https://github.com/apache/stormcrawler) | Distributed frontier, continuous stream, per-host partitioning, status feedback, Playwright/Tika/WARC | Excellent model for large scale, Storm operational dependency too heavy for a first action engine. Apache-2.0. [Architecture](https://github.com/apache/stormcrawler/blob/main/docs/src/main/asciidoc/architecture.adoc) |
| [Heritrix](https://github.com/internetarchive/heritrix3) | Large-scale archival crawling, archive quality | Different purpose from application automation, Apache-2.0 |
| [Botasaurus](https://github.com/omkarcloud/botasaurus) / [driver](https://github.com/omkarcloud/botasaurus-driver) | browser/request/task decorators, sessions, rendering and fetch from a browser, packaging a scraper | MIT. The promises of access to all sites and massive cost reduction are vendor claims, not an independent benchmark |
| [MechanicalSoup](https://github.com/MechanicalSoup/MechanicalSoup) | Shows that a form journey can work with Requests + parsing, without Chrome | Does no JavaScript rendering; MIT. Useful as a minimal baseline |
| [DrissionPage](https://github.com/g1879/DrissionPage) | CDP and hybrid HTTP/browser approach; ergonomic API | **Current LICENSE forbids commercial use without authorization.** Source-available does not mean permissive OSS. [License](https://github.com/g1879/DrissionPage/blob/master/LICENSE) |
| [nodriver](https://github.com/ultrafunkamsterdam/nodriver) | Async Python driver directly over CDP, without Selenium/Chromedriver | AGPL-3.0. PyPI version 0.50.5 of October 5, 2026; stealth promises to be measured |
| [Zendriver](https://github.com/cdpdriver/zendriver) | nodriver fork focused on fixes and community participation | AGPL-3.0, PyPI 0.17.1 of October 2, 2026; README documents the reason for the fork and the need to handle bugs/PRs |
| [Selenium-Driverless](https://github.com/kaliiiiiiiiii/Selenium-Driverless) | CDP, interception, request fetch, familiar API | **CC BY-NC-SA with commercial addition**, commercial license required for the intended goal. [License](https://github.com/kaliiiiiiiiii/Selenium-Driverless/blob/master/LICENSE.md) |
| [BrowserForge](https://github.com/daijro/browserforge) | Consistent header/fingerprint generation, Python reimplementation of Apify's fingerprint-suite | Identity building block; fingerprint generation alone does not guarantee an undetectable session |
| [wreq](https://github.com/0x676e67/wreq), [curl_cffi](https://github.com/lexiforest/curl_cffi), [Impit](https://github.com/apify/impit) | Transport and impersonation without a browser | Compare by platforms, profiles and maintenance; not rendering engines |
| [scrapling-rs](https://github.com/chrisabruce/scrapling-rs) | Rust port of adaptive parsing and wreq transport, very recent competitor | Primary documentation discovered; implementation, conformance to Python behaviors, maintenance and full license not audited here |
| [ScrapeForge](https://scrapeforge.io/docs/guides/anti-bot-bypass) / [StealthPlex](https://github.com/SSujitX/StealthPlex) | Engine escalation and fingerprint management already offered by niche players | Discovery-level documentation coverage; no execution evaluation nor audit of all dependencies/licenses |

Claims such as "undetectable", "all websites" or "Rust = fastest" are recurrent. Treat them as commercial positioning to be tested, never as established properties of the software.

## What existing benchmarks actually allow us to say

The Impit example is useful because the protocol is explicit: median of 11 runs, 2,000 sequential requests to a local HTTP/2 server, 1 KiB JSON response, one warm connection. It compares transport libraries, including Python and Node. This experiment can inform a client optimization; it measures neither cold TCP/TLS over the Internet, nor residential proxies, nor JS execution, nor model costs, nor extraction accuracy. [Methodology](https://github.com/apify/impit/blob/master/README.md#L37).

The Scrapling README compares the parser and element location over repeated loops. The closeness of Parsel and Scrapling on a given task does not predict the duration of a protected crawl. [Benchmarks](https://github.com/D4Vinci/Scrapling/blob/main/README.md#L479).

The controller's language can reduce its memory and CPU consumption. On the other hand, the browser runs the same Chromium engine, sites have variable latencies and LLM calls can dominate the critical path. These shares must be profiled before turning a microbenchmark optimization into a rewrite project.

## Concrete direction for the target architecture

1. **Task contract at the center.** A task receives an explicit expected result: data schema, completeness constraints, element role, observed state before/after an action. Engines return facts and artifacts, not just "success".
2. **Two categories of work.** Acquisition/extraction allows parallel attempts or comparative double executions. Actions on an application require a journal and resumption rules adapted to their effects. Keep this distinction in the model, without multiplying products.
3. **Canonical Rust engine; TS/Python SDKs from a common protocol.** A sidecar carries sessions, transports, observations and actions; the SDKs do not reimplement its invariants. Typed, versioned commands expose cancellation, deadlines, progress, result and artifacts. Large streams remain streamed, and transport/protocol/task errors are distinct. The first local and remote transport must be the same contract, with the overhead measured before adding a different FFI path.
4. **Narrow adapters and separate orchestration.** Native HTTP and native CDP constitute the target path; qualified Playwright serves as compatibility backend and behavioral reference. A workflow API orchestrates the steps and their journal without imposing a crawl queue on every journey. A Rust Spider adapter and, on the ecosystem side, a Crawlee adapter must remain optional. Reuse the ideas of their queues/sessions/backpressure; avoid embedding two schedulers that both own the same task.
5. **Contextualized execution policy.** Learn per site, route and requested result; HTTP-only when quality is validated, direct browser when history justifies it. A systematic HTTP-then-browser fallback penalizes pages known to be JS-only.
6. **LLM for discovery and repair.** Once a journey is established, run the deterministic recipes with postcondition checks. Trigger a repair on diagnosed failure; keep the recipe version, the model used and the justification for the change. Compilation/replay already exists, so verified quality and the maintenance experience must carry the differentiation.
7. **Pool and budgets from the start.** Keep a consistent cookies/proxy/profile identity within a session, limit concurrency and resources per site, provide a total task deadline and a retry budget. A reusable pool does not mean mixing customer sessions.
8. **Diagnostic artifacts.** Typed events, fallback reasons, time per step, parsed outputs, screenshot or DOM needed for diagnosis. Hide what is not necessary for the end user's understanding.

The business model can sell managed sessions, guaranteed capacity, observability, recipe maintenance, proxies and support. The free framework must be able to run a real workflow locally so that the service offering has an observable advantage, without confusing a free SDK with a complete open source product.

### Implementation order and validation gates

| Step | Deliverable | Verification that decides what comes next |
|---|---|---|
| Baseline and contract | Representative corpus + instrumentation, Playwright/Crawlee/Spider baseline, sidecar protocol | Result quality, p50/p95 latency, cold/warm time, success rate and cost per actually successful task |
| Native executors | Rust HTTP/CDP, generated SDKs, typed results, sessions and trace, first recipes | Parity of critical cases with Playwright; CPU/memory profile; equivalent quality on the same workloads |
| Workflow reliability | Resumption journal, targeted repair, crawl adapter, local packaging | Resumption after crashes/timeouts, no observed duplications on representative cases, actionable diagnostics |
| Managed services | Remote sessions, hosting, observability, recipe maintenance | Total cost of operation and resolution time compared with direct use of existing tools |

**Promotion gate for the native path:** Rust is the target choice, but each replacement of Playwright behavior must demonstrate its effects on a corpus at equal quality. Keep the fallback when a case is incomplete. Profiling must distinguish control, parsing, transport, Chromium engine, network and LLM; optimize the dominant share, with packaging and maintenance costs included. No magic percentage or "10×" factor is justified before this experiment. This architecture does not imply rewriting Chromium, which remains an external browser process.

A reasonable corpus must mix static HTTP, server HTML with hydration, JS-only, navigation with authentication, forms, pagination, downloads, rate limits and long sessions. Testing a DOM modification and a stop between action and validation reveals differences invisible in a simple pages-per-second table. Separate quality, network, protection, driver and resumption failures.

## Remaining uncertainties

- No audited library was tested here on the Automan corpus: speed and reliability ranking not established.
- The Spider/Firecrawl/Crawl4AI cloud features were not exercised; OSS/cloud parity not established.
- Crawlee Stagehand 3 compatibility with Stagehand 4 is absent from the examined npm declaration; a product-specific adapter could be necessary.
- The right copyleft scope will depend on what is actually combined, distributed and offered over a network; this document records the licenses, without resolving a code combination that does not yet exist.
- Unpinned imports and versions, and in particular driver forks, must be validated on a real Chrome/OS matrix. Update frequency does not replace compatibility testing.
