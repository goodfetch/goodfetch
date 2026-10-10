# Automan Lab — audit of the three requested engines

Snapshot taken on **October 7, 2026**. "Crawell" is interpreted as **Crawlee**; "Spider rs" designates the OSS Rust crate **`spider` from the spider-rs repository**, distinct from the commercial Spider Cloud API.

**Conclusion:** all three tools should enter the lab, with separate configurations. Spider is a reference for Rust HTTP/Chrome collection; Crawlee a reference for crawl orchestration; Stagehand a reference for browser driving and perception. Their common functions allow experiments; their different functions prevent an honest global ranking without defining the expected result.

This note complements the [Crawlee/Spider contracts](2026-10-07-crawlee-spider-contracts-followup.md), the [Stagehand audit](2026-10-07-stagehand-alternatives-followup.md) and the [CDP clarification](2026-10-07-stagehand-cdp-latency.md). It re-verifies the registries and official sources; **no dependency installed, no engine launched, no benchmark run**.

## 1. Re-verified stable versions and indispensable separation

| Component | Stable version found | Contract useful to the lab | Exact official source |
|---|---|---|---|
| Crawlee TypeScript | **3.18.2** | Node.js SDK, HTTP and browser crawlers | [npm registry](https://registry.npmjs.org/crawlee/3.18.2) |
| `@crawlee/stagehand` adapter | **3.18.2** | Peer Stagehand **`^3.0.0`**, Playwright `*` | [npm manifest](https://registry.npmjs.org/@crawlee%2Fstagehand/3.18.2) |
| Stagehand TypeScript | **4.1.0** | v4 driver/runtime, Node **`>=22.18.0`** | [npm manifest](https://registry.npmjs.org/@browserbasehq%2Fstagehand/4.1.0) |
| Stagehand v3 branch | **3.7.3**, tag `v3-latest` | Pinned candidate for testing the current Crawlee adapter | [global registry](https://registry.npmjs.org/@browserbasehq%2Fstagehand), [3.7.3 manifest](https://registry.npmjs.org/@browserbasehq%2Fstagehand/3.7.3) |
| Crawlee Python | **1.10.4** | Python **`>=3.10`**; Stagehand extra **`>=3.19.5,<4.0.0`** | [exact PyPI](https://pypi.org/pypi/crawlee/1.10.4/json) |
| Spider Rust | **2.53.9** | Native HTTP, browser/smart/disk/distribution features | [exact crates.io](https://crates.io/api/v1/crates/spider/2.53.9) |
| `spider_agent` | **2.53.9** | Optional Rust agent, per-capability features | [exact crates.io](https://crates.io/api/v1/crates/spider_agent/2.53.9) |
| Playwright | **1.63.0** | Independent baseline for the browser | [npm manifest](https://registry.npmjs.org/playwright/1.63.0) |
| Puppeteer | **25.12.0** | Other optional CDP baseline | [npm manifest](https://registry.npmjs.org/puppeteer/25.12.0) |

The Stagehand npm and Python numbers describe different packages: npm `3.7.3` is not Python `3.19.5`. npm Crawlee 3.18.2 was published on September 29; Stagehand 4.1.0 on September 9; Spider 2.53.9 on September 5, according to their registries.

**Do not force Stagehand v4 into `@crawlee/stagehand` 3.18.2.** This peer excludes v4. Create a standalone Stagehand v4 lane and, if useful, a Crawlee + Stagehand v3 integration lane. A dependency that installs with `--force` does not prove compatibility of the objects or of the runtime. [Versioned adapter source](https://github.com/apify/crawlee/blob/v3.18.2/packages/stagehand-crawler/package.json).

## 2. Stagehand: CDP, placement, guarantees and cost

### The figure that motivates the lab

The v4 blog post reports **a 50-action Wikipedia journey**, all performed in both variants: Stagehand batch **14,221.7 ms**, Playwright **22,650.3 ms**, i.e. **1.59× the throughput**, with a client–browser RTT of **42.2 ms**. The announced medians are `waitForSelector` 237.6 vs 493.2 ms, `click` 323.1 vs 628.1 ms and `goBack` 17.5 vs 139.5 ms. The authors explicitly ask that this be considered **one run, not a general benchmark**. [Official v4 blog post, August 10, 2026](https://www.browserbase.com/blog/stagehand-v4).

The announced gain concerns a deterministic/batch sequence and its remote transport. It does not demonstrate LLM extraction throughput, the total cost of an agent, crawler autoscaling or reliability on all sites. The blog post alone does not provide a reproducible corpus, a distribution of runs, p95s or a detailed protocol of identical postconditions.

Playwright **already uses CDP for Chromium**: `CRBrowser` and `CRConnection` are in its backend. The relevant opposition is therefore driver, placement of execution, number of RPCs and checks performed. Playwright's `connect()` and `connectOverCDP()` modes are themselves distinct; the fidelity caveat in the docs targets the latter attachment mode. [Official Chromium code](https://github.com/microsoft/playwright/blob/v1.63.0/packages/playwright-core/src/server/chromium/crBrowser.ts), [attachment reference](https://playwright.dev/docs/api/class-browsertype#browser-type-connect-over-cdp).

### What changed in v4

Stagehand v4 places target/frame tracking and CDP dispatch in a Chrome extension, notably its service worker. The SDK is an RPC client; the extension is close to the browser state. `experimentalBatch()` executes a JavaScript callback in this worker: a series thus avoids one SDK–browser trip per operation. The callback must be self-contained, with explicit JSON data; a Node.js closure is not transported. A batch is not a transaction and its timeout does not cancel an effect already produced. [`experimentalBatch` reference](https://docs.stagehand.dev/v4/reference/stagehand).

V4 no longer has a Playwright dependency and no longer receives a Playwright `Page`. The migration describes a single BrowserContext per browser, and the absence of Playwright auto-waiting, `getBy*` locators, web-first assertions and route interception. `goto()` waits for `domcontentloaded` by default; a selector is resolved once. A shorter duration can therefore buy fewer guarantees. [Official Playwright migration](https://docs.stagehand.dev/v4/migrations/playwright).

A Playwright click notably checks visibility, stability, event reception and enabled state; its assertions retry. Even these checks do not establish that the right business entity was modified. Two measurements are needed: **native operation as provided**, then **operation with the common contract and the lab's oracle**. [Playwright actionability](https://playwright.dev/docs/actionability).

### Deterministic paths and AI paths

| Stagehand v4 path | Inference on the normal path | Conditions/limits to measure |
|---|---|---|
| `page`, `locator`, snapshot, capture | No LLM resolution needed | Browser/extension required; establish postconditions independently |
| `observe(instruction)` | Model resolution, barring an eligible cache hit | Candidate actions returned, not goal executed |
| `act(instruction)` | Interpretation then action, barring cache | Technical success distinct from business truth; additional calls possible |
| `act(Action)` | Direct replay without initial inference | In stable 4.1.0, **model/gateway configuration checked before the direct branch**; repair may call the model |
| `extract(instruction,schema)` | In stable 4.1.0, generation then completeness call | The second call receives instruction and output, not the source snapshot; schema conformance distinct from truth |

These details were re-verified in the **[stable 4.1.0 tarball](https://registry.npmjs.org/@browserbasehq/stagehand/-/stagehand-4.1.0.tgz)** opened in memory, without installation: `dist/extension/service-worker.js`, functions `act$2`, `extract$1` and `buildCacheContext`, and types `dist/index.d.mts`. The controller throws `An LLM was not configured during Stagehand initialization` when `act` has neither model nor gateway, before replay. The deterministic Page/Locator primitives remain a distinct path.

A local browser can use a BYOK model configuration or a `generate()` callback, possibly connected to a self-hosted model. **The Browserbase gateway is not the only AI path**, but a local extension does not mean that inference is local. Publish the endpoint, model, revision, availability and latency of this dependency. [Official configuration](https://docs.stagehand.dev/v4/reference/stagehand).

`agent()` was removed in v4; the SDK exposes tools, and orchestration falls to the code or to an external agent. Compiling a script and then executing it without a model is therefore not exclusive to Automan. [Official v3 migration](https://docs.stagehand.dev/v4/migrations/v3).

### Cache and fresh data

The standard cache is a Browserbase service: Browserbase session and key required; the local browser alone does not benefit from it. The current docs announce it as enabled by default, but stable 4.1.0 `buildCacheContext` contains **`defaultCaching: initParams.cache ?? false`**. Set `cache` explicitly, check the metadata and separate local and cloud tests. [Official cache](https://docs.stagehand.dev/v4/best-practices/caching), [exact distributed source](https://registry.npmjs.org/@browserbasehq/stagehand/-/stagehand-4.1.0.tgz).

In the inspected runtime, `act` stores actions, `observe` a list of candidates, and `extract` a **structured output**. An extract HIT returns this value. The cache request contains URL, AX trees, instruction/options and schema; the model is omitted. This proves neither systematically stale data nor universal freshness: the server's non-public matching/TTL remain to be exercised. [Corresponding public module](https://github.com/browserbase/stagehand/blob/fdd179582b6b1f9e43c3d18ad27917cf2e66631e/packages/extension/services/cacheService.ts).

Decisive test: same URL, same layout, changed price; verify the returned value and HIT/MISS with a server oracle. Account for all captures, cache calls, repair inferences, failed attempts and tokens. "Zero tokens on HIT" is not "zero cost": browser, network, validations and service remain.

## 3. Crawlee: orchestration baseline, two runtimes

Crawlee TS is written in TypeScript and executed in Node.js; Python is a distinct implementation. Both offer async concurrency, pools and HTTP/browser crawlers. TS brings Playwright and Puppeteer; Python has its own Playwright integration and its own adapters. Do not extrapolate the surface or the measurements of one to the other. [TS introduction](https://crawlee.dev/js/docs/introduction), [Python architecture](https://crawlee.dev/python/docs/guides/architecture-overview).

`AutoscaledPool` adjusts async tasks according to resources, while `BrowserPool` manages browsers/pages. An async worker of this autoscaling is not by definition a distributed process. The `RequestQueue` deduplicates by `uniqueKey`, can add/reclaim/mark handled requests and plugs into the storage actually chosen. [Official pool](https://crawlee.dev/js/api/core/class/AutoscaledPool), [official queue](https://crawlee.dev/js/api/core/class/RequestQueue).

**Request persistence is not a transactional workflow.** Replaying a handler after a crash can repeat an external effect or an output already written before acknowledgment. The Automan proposal must define its leases, checkpoints, idempotency keys and state validation; the presence of Crawlee does not establish exactly-once execution. This consequence must be qualified with a kill test and a controlled sink. [Versioned BasicCrawler lifecycle](https://github.com/apify/crawlee/blob/v3.18.2/packages/basic-crawler/src/internals/basic-crawler.ts).

`AdaptivePlaywrightCrawler` is a particularly useful reference: it compares the outputs of the HTTP and browser handlers, with `resultChecker`/`resultComparator`, then learns a choice. The default comparator concerns dataset items; its validity depends on what the handler produces. Empty lists do not prove identical discovered links. [Adaptive source 3.18.2](https://github.com/apify/crawlee/blob/v3.18.2/packages/playwright-crawler/src/internals/adaptive-playwright-crawler.ts).

TS HTTP can use **Impit, a native Rust transport**: "TypeScript SDK" does not mean "all network operations in JavaScript". This point justifies a qualified Crawlee HTTP baseline, specifying the TLS/HTTP client, parser and compression. [Official Impit guide](https://crawlee.dev/js/docs/guides/impit-http-client).

The TS Stagehand adapter 3.18.2 starts Stagehand and attaches Playwright via CDP. The plugin warns that fingerprinting remains partial because Stagehand controls the launch. Python 1.10.4 uses `AsyncStagehand`, a session and its CDP URL; its limitations on contexts/pages/proxies must be handled separately. [Pinned TS plugin](https://github.com/apify/crawlee/blob/v3.18.2/packages/stagehand-crawler/src/internals/stagehand-plugin.ts), [pinned Python manifest](https://github.com/apify/crawlee-python/blob/v1.10.4/pyproject.toml), [Python guide](https://crawlee.dev/python/docs/guides/stagehand-crawler).

## 4. Spider-rs: what exists in the published crate

Source 2.53.9 tied to SHA **`e4b53c5b582357e46433fa9ba3d91f930fb67cb4`**. The inspected manifest uses Rust/Tokio, HTTP `reqwest 0.13`, and **optional Chromey** for CDP. Dependency versions are ranges: a future `Cargo.lock` must pin the transitive resolution. [Exact manifest](https://github.com/spider-rs/spider/blob/e4b53c5b582357e46433fa9ba3d91f930fb67cb4/spider/Cargo.toml).

| Proposed lane | Capabilities/activation | What it does not prove on its own |
|---|---|---|
| HTTP | `basic`, HTTP acquisition/parsing | JavaScript-executed content |
| Chrome | `chrome`, Chromey, local Chrome/Chromium or CDP | Full guarantees of Playwright locators |
| Smart | `smart` implies `chrome` and `chrome_intercept` | Always-optimal choice relative to the business result |
| Agent | `spider_agent`/`agent_chrome`, configured model | Success independent of the model |
| Disk | `disk`/SQLx, SQLite sets | Durable pending queue and transactional workflows |
| Distributed | `decentralized` + `spider_worker` | Browser-rendering worker or general distributed orchestration |

Do not enable `full` or all features for a minimal baseline. The default includes `basic`, `io_uring`, `tcp_fastopen`, `splice`, `numa`, `zero_copy`; several optimizations are platform-dependent. The docs.rs documentation aggregates many features: it does not automatically describe the binary installed on macOS. [Official features list](https://crates.io/api/v1/crates/spider/2.53.9).

Smart uses HTML signals to decide an upgrade, and can open Chrome lazily. Spider also offers optional parallel backends and a `QualityValidator`: validation must not be reduced to "presence of JS". Measure upgrade rate, doubled work and quality of the requested result. [Pinned smart path](https://github.com/spider-rs/spider/blob/e4b53c5b582357e46433fa9ba3d91f930fb67cb4/spider/src/page.rs#L8305), [quality hook](https://github.com/spider-rs/spider/blob/e4b53c5b582357e46433fa9ba3d91f930fb67cb4/spider/src/website.rs#L14248).

`setup_semaphore()` chooses a provided adaptive handle, a shared semaphore or a configured/default limit. This does not prove that each execution automatically adjusts to CPU/RAM like Crawlee. The normal browser is reused during the crawl: do not set against it a baseline that launches Chromium for each URL. [Semaphore](https://github.com/spider-rs/spider/blob/e4b53c5b582357e46433fa9ba3d91f930fb67cb4/spider/src/website.rs#L6956), [browser lifecycle](https://github.com/spider-rs/spider/blob/e4b53c5b582357e46433fa9ba3d91f930fb67cb4/spider/src/website.rs#L9180).

### Durability and distribution not to overestimate

The `disk` database contains `resources` and `signatures` tables, without pending/running/done states or step leases. `persist` defaults to false and Drop deletes the file when it is not kept. This persistence serves visit/signature sets; a frontier after a crash requires an additional contract. [Exact schema and Drop](https://github.com/spider-rs/spider/blob/e4b53c5b582357e46433fa9ba3d91f930fb67cb4/spider/src/features/disk.rs).

`Website::queue()` is an **in-memory broadcast** Tokio channel; `persist_links()` changes the object's status to continue passes. The page stream must be consumed into a sink and its lag observed. It is not a durable results journal. [Published queue](https://github.com/spider-rs/spider/blob/e4b53c5b582357e46433fa9ba3d91f930fb67cb4/spider/src/website.rs#L14692).

The OSS worker uses Warp 0.4/Hyper 1/Tokio and its HTTP fetch/parsing handlers; its manifest does not enable Chrome. The word worker is therefore not enough to promise distributed rendering, leases or job restoration. Spider Cloud's private infrastructure is another surface. [Worker manifest](https://github.com/spider-rs/spider/blob/e4b53c5b582357e46433fa9ba3d91f930fb67cb4/spider_worker/Cargo.toml), [worker handlers](https://github.com/spider-rs/spider/blob/e4b53c5b582357e46433fa9ba3d91f930fb67cb4/spider_worker/src/main.rs).

The public Spider benchmark pits Spider, Colly, the Node package **`crawler`** and wget against each other: **Crawlee and Stagehand are not included**. Startup/concurrency/outputs have asymmetries and correct volumes are not asserted. Do not use its multipliers to choose the Automan runtime. [Pinned source protocol](https://github.com/spider-rs/spider/blob/fb0f270d77e19442de59350d9dbbacf52f1804b4/benches/BENCHMARKS.md), [detailed local audit](2026-10-07-benchmark-audit-followup.md).

## 5. Proposed reproducible installation, to be carried out during implementation

**Do not install all three in a single dependency project.** Prepare isolated adapters under a `lab/` directory, each exposing the harness contract and its versions. Use the published packages; upstream clones only for an explicitly experimental profile.

1. **Crawlee TS 3.18.2 + Playwright 1.63.0**: isolated packages/workspaces, exact versions, `pnpm-lock.yaml`, installation via `pnpm install --frozen-lockfile` to reproduce. The npm alternative is `package-lock.json` + `npm ci`; choose one manager and its exact version for the lab. HTTP and browser explicitly selected; the HTTP backend and the parser are recorded.
2. **Standalone Stagehand TS 4.1.0**: separate lock, exact Node satisfying `>=22.18.0`, the package's extension bundle. Model-free lane via Page/Locator, then a separate model/cache lane. Set `cache` and `selfHeal` explicitly.
3. **Optional Crawlee + Stagehand v3**: separate lock, `@crawlee/stagehand@3.18.2` and `@browserbasehq/stagehand@3.7.3`, with resolved peers and a smoke test before qualification. No mixing of v3/v4.
4. **Spider `=2.53.9`**: `Cargo.lock`, exact Rust toolchain after qualification, `--release --locked` build, published features. HTTP, Chrome and Smart profiles compiled/checked separately; record `cargo tree -e features`.
5. **Optional Python**: isolated environment, Crawlee `==1.10.4`, lock with hashes and exact versions of the extras. The TS baseline remains distinct from Python.

Cargo can unify the features of a single dependency in a workspace build: an HTTP lane must not silently inherit Chrome/Smart because the harness compiles all the adapters together. Use controlled projects or invocations and verify the graph actually built. [Official Cargo rules](https://doc.rust-lang.org/cargo/reference/features.html#feature-unification).

### The browser must also be pinned

Pin **the Chrome/Chromium executable and its exact version**, platform/architecture, checksum and flags. Provide the same `executablePath` to the comparators of the common profile. Keep a second "tool-provided/recommended browser" profile if that choice influences capabilities; its results are not merged with the common profile.

Stable Stagehand looks for an existing Chrome and, on macOS, may prefer **Canary** over Stable; autodetection would undermine control of the browser. Its extension manifest announces a minimum of Chrome 116, but the SDK uses `Extensions.loadUnpacked` and reports an error if this command is missing. This minimum is therefore not proof of compatibility with all builds. Require a prequalification of extension/loadUnpacked, frames, screenshot and click; then pin the build. [Exact tarball, SDK and extension manifest](https://registry.npmjs.org/@browserbasehq/stagehand/-/stagehand-4.1.0.tgz).

Normalize isolation: Stagehand v4 provides **one context per browser**. In a session/tenant isolation experiment, use one browser per identity for the baselines too, or separately publish the cost of a multi-context strategy available elsewhere. A shared multi-tab browser and several isolated browsers do not provide the same service.

### Data to record in each result

Package version + lock hash + features; runtime/compiler; browser/extension/flags; machine/OS; client/driver/browser placement; model/endpoint; cache HIT/MISS; corpus URL/seed; concurrency and limits; number of commands/captures/inferences; verified final states; attempts and errors; CPU/RSS of the **process group**, bytes and total cost.

The projects are currently candidates. A lock is proof of reproducible resolution, not proof of compatibility: local qualification is indispensable, then common tests before any Rust/TypeScript or CDP/Playwright decision.

## 6. Experiments that will decide between these engines

- **Deterministic transport/browser**: Playwright, Stagehand Page/Locator, Stagehand batch, Spider/Chromey on the same primitives and the same oracle; local, then controlled RTTs, driver placement recorded.
- **Collection**: Spider HTTP/Chrome/Smart and Crawlee HTTP/Playwright/adaptive on a known corpus, same authorized URLs, content and resources; no mandatory AI.
- **Perception/AI**: Stagehand observe/act/extract and `spider_agent`, same model/budget if possible, exact extraction, false successes and real number of calls.
- **Repetition and drift**: first decision, replay, fresh data, semantically misleading DOM mutation and bounded repair; not just HIT rate.
- **Incident/resumption**: kill after output/before acknowledgment, slow sink, CDP loss, browser crash and expired identity; lost work/duplicates explicitly counted.

**Proposed decision:** Rust/Tokio remains a coherent hypothesis for a common and portable runtime. This lab must determine which layers benefit from it, which components to reuse and which guarantees to own. An extension placed close to the browser, a co-located Node worker and a remote Rust driver can reverse a ranking based only on language.
