# Stagehand, Crawlee, Spider-rs: in-depth comparison for Goodfetch

Research of **October 7, 2026**, extending the market dossier. The goal remains an ambitious open source framework and services around it; the former assumption of a small team over 3–6 months was withdrawn by the user. This report distinguishes published capabilities, inspected code, testimonials and proposals for Goodfetch. No comparative benchmark of the engines was run.

## Conclusion that governs the comparison

**Stagehand, Crawlee and Spider-rs mainly work at different layers.** Stagehand addresses perception and browser operations for a developer or an agent. Crawlee organizes request processing, resources, sessions, recovery and results. Spider is a Rust collection engine, extensible toward Chrome and AI processing. They overlap, but comparing them as three interchangeable agents produces misleading conclusions.

The provided text describes a real tension — semantic discovery versus known execution — but several of its oppositions are too absolute. Stagehand retains deterministic operations and selectors; Crawlee can incorporate Stagehand and adapt HTTP/browser rendering; Spider has optional browser and AI functions. Maintenance does not disappear with any of these mechanisms.

**For Goodfetch, the proposed direction is a common runtime that chooses the sufficient executor, produces reusable plans and verifies the requested result.** Mass collection and application actions must share transports and perception, while keeping distinct scheduling and recovery rules. This proposal does not constitute proof of a commercial lead: several competitors already have cache, replay, repair and partial validation.

## 1. Precisely correcting the provided text

| Provided claim | What the sources allow us to say |
| --- | --- |
| "Reddit consensus Stagehand vs Crawlee" | The references provided are home pages. Our follow-up research did not find a sufficiently documented direct comparison of the trio. Individual testimonials exist, sometimes contradictory or promotional. |
| "Stagehand does not use classic selectors" | False as a description of the framework. Its semantic primitives can produce `Action`s, and its page/locator APIs allow deterministic operations. The instruction provided by the developer must not be confused with the target finally executed. |
| "Crawlee always requires manual DOM maintenance" | Too broad: Stagehand integration, adaptive HTTP/Playwright crawler and possible extensions. Scrapling also offers structural recovery without an LLM. None of these tools guarantees business correctness after every change. |
| "Crawlee handles anti-bot automatically" | It provides sessions, proxies and fingerprinting mechanisms depending on the backend. They do not guarantee access to a site, and the Stagehand integrations have explicit fingerprint/isolation limits. |
| "Playwright in Crawlee will always be faster" | A known path can eliminate inferences, but the result depends on rendering, network, session state, batching, cache and the work actually done. The word "always" is not supported by any common protocol examined. |
| "Crawlee is free and needs no API keys" | The framework can run locally without a model API; browsers, machines, proxies and maintenance have a cost. Adding AI primitives adds the requirements of the model or its gateway. |
| "StagehandCrawler makes it possible to combine the two" | Yes, with an official integration. However, the TypeScript and Python contracts must be read separately and the versions checked; the presence of the adapter does not establish Stagehand v4 compatibility. |

Sources: [Stagehand migration from Playwright](https://docs.stagehand.dev/v4/migrations/playwright), [published TS adapter](https://github.com/apify/crawlee/blob/v3.18.2/packages/stagehand-crawler/package.json), [Python StagehandCrawler](https://crawlee.dev/python/docs/guides/stagehand-crawler), [published adaptive crawler](https://github.com/apify/crawlee/blob/v3.18.2/packages/playwright-crawler/src/internals/adaptive-playwright-crawler.ts), [verified Reddit evidence](2026-10-07-reddit-evidence-followup.md).

## 2. The useful comparison: contract, resources and result

| Axis | Stagehand | Crawlee | Spider-rs |
| --- | --- | --- | --- |
| Main object | Page and semantic/deterministic operations | Request and handler within a crawl cycle | Website, pages, links and collection streams |
| Target discovery | Instructions, observation, locators, resolved actions | Handler chosen by the developer; AI via integrations | Parsing and hooks; optional browser/AI |
| Rendering | Browser; v4 runtime in a Chrome extension | HTTP and several browser backends | Native HTTP, Chrome and smart mode depending on features |
| Mass work | Page/tab concurrency does not constitute a crawl frontier | Queue, deduplication, autoscaling, pools, sessions, retries, storage | Concurrency, budgets, deduplication and page broadcasting; additional capabilities depending on compilation |
| Adaptation | Semantic resolution and cache/replay mechanisms | HTTP/Playwright choice based on observation/comparison of outputs | Smart mode based notably on rendering-necessity signals |
| Recovery | Local functions to be distinguished from resuming a complete job | Reclaim/retry of a request and configured storage | Crawl and storage to be examined per feature; not to be equated with a transactional business workflow |
| Truth of the result | Structured extraction and internal judgment are not enough to prove the business result | Handler and validation callbacks can express it | The application must validate the collected content and its completeness |
| Question to benchmark | How many decisions, calls and false successes for a task? | How many valid results with what resources and retries? | What throughput/footprint for equivalent acquisition, and what rate of unnecessary upgrades? |

Detailed contracts and versions are in [the Stagehand alternatives study](2026-10-07-stagehand-alternatives-followup.md) and [the Crawlee/Spider inspection](2026-10-07-crawlee-spider-contracts-followup.md). This grid is an analysis, not a matrix of measured performance.

## 3. Stagehand: the subject is now v3 versus v4 as much as AI versus code

### What to compare today

The study of the stable **Stagehand 4.1.0** package shows an extension runtime and a protocol shared between clients. The v4 migration removes the old autonomous `agent()` API in favor of primitives that can be integrated into external orchestration. The `page.act(...)` example from an old discussion therefore does not necessarily describe the current surface. [v4 architecture](https://www.browserbase.com/blog/stagehand-v4), [v3 migration](https://docs.stagehand.dev/v4/migrations/v3), [protocol](https://github.com/browserbase/stagehand/blob/main/packages/protocol/README.md).

Three paths must be separated:

1. **Semantic instruction:** asking it to find or do something requires interpretation and depends on the model, the context and the observation.
2. **Already resolved action:** executing an `Action` object can avoid inference; in the inspected 4.1.0 package, however, `act(Action)` retains a prior check of model/gateway configuration. "No inference for this branch" therefore does not imply "this whole API is usable without model config".
3. **Page/locator primitives:** a deterministic path, distinct from semantic calls, that must be qualified as a driver in its own right.

Above all, it must not be automatically credited with Playwright's guarantees because some methods have similar names. The [official migration](https://docs.stagehand.dev/v4/migrations/playwright) notably describes the absence of equivalent auto-waiting, of retrying Playwright assertions and of several network waits. The click must therefore be studied with its actual resolution and availability conditions.

### Extraction: conformance, completeness and truth are three questions

In the inspected stable package, the non-cached extraction path involves a structured generation followed by a completeness judgment. This second call receives the instruction and the output, not the source snapshot. This judgment is useful, but depends on the model; it does not consult an oracle independent of the task. The internal `completed` field is not exposed to the consumer as a guarantee of business truth. The inspected code can return the output and offer it for cache writing even when this judgment is negative; acceptance by the non-public cache server was not verified.

A schema that requires a number and a currency does not guarantee that the number is the current price of the right product. A plausible results page does not guarantee that the entire pagination was traversed. This limit concerns the result contract, beyond any single library. The exact source paths and the limits of this static reading are in [the Stagehand 4.1.0 report](2026-10-07-stagehand-alternatives-followup.md).

### Cache: do not confuse action and data

The three cache families examined do not store the same object: `observe` can reuse resolved actions, `act` can replay actions, and `extract` can directly return a **structured value**. This last case requires a freshness policy if the data changes. The client key notably transmits URL, accessibility trees, instruction and schema, but the matching server is not public; this reading does not prove its TTL or semantic invalidation rules.

A divergence exists between the v4 documentation presenting the cache as enabled by default and the default values of the inspected 4.1.0 package. A comparative experiment must **configure the cache explicitly and check the metadata**, then publish cache HIT/MISS and freshness. We did not exercise the cloud service; no assumption about its hidden implementation is converted into fact here.

For Goodfetch, separating perception cache, plan cache and data cache becomes a contract requirement: their invalidation rules cannot be interchangeable.

## 4. StagehandCrawler: a real composition, with two distinct contracts

### Published TypeScript

`@crawlee/stagehand` **3.18.2** declares `@browserbasehq/stagehand: ^3.0.0`. This constraint does not cover v4. The package is separate from the main `crawlee` bundle. [Versioned manifest](https://github.com/apify/crawlee/blob/v3.18.2/packages/stagehand-crawler/package.json).

Launching and connecting combine Stagehand and Playwright on **the same browser via CDP**. This is not simply two independent APIs dropped into a handler: browser ownership, page attachment and launch options matter. The `act/extract/observe` wrappers inject the page being processed. The v3 wrapper's autonomous `agent` interface must be studied separately before assuming that it always targets this page. The plugin code warns that the fingerprint is only partially applied since Stagehand controls the launch.

This yields real Crawlee orchestration around Stagehand v3 primitives. It demonstrates neither all of Playwright's guarantees, nor all of BrowserPool's fingerprint guarantees, nor a transparent v4 migration.

### Published Python

In Crawlee Python **1.10.4**, the Stagehand extra depends on the Python package `stagehand>=3.19.5,<4.0.0`. These numbers belong to the Python client; they are not interchangeable with those of the npm package. The plugin uses `AsyncStagehand`, starts a session, then connects Playwright to its CDP URL; local mode can select the Chromium provided by Playwright. [Python manifest](https://github.com/apify/crawlee-python/blob/v1.10.4/pyproject.toml), [guide](https://crawlee.dev/python/docs/guides/stagehand-crawler).

The inspection confirms a shared context and limits regarding new contexts/proxies, incognito pages and complete canvas/WebGL/screen fingerprinting; headers consistent with a fingerprint do not constitute a complete fingerprint. `StagehandPage` injects the page for its operations, including `execute`. The [contracts report](2026-10-07-crawlee-spider-contracts-followup.md) provides the sources and the detailed isolation consequences.

**Practical consequence:** the existence of an official integration is confirmed; compatibility and isolation must be verified for the chosen language, version and backend.

## 5. Crawlee: more interesting than "selectors with retries"

The important architectural competitor is `AdaptivePlaywrightCrawler`. In the TS 3.18.2 release, a predictor learns from the host, the path and the request label. Detection runs compare what the handler produces in the browser and over HTTP; `resultChecker` and `resultComparator` callbacks make it possible to decide whether the output is sufficient. [Implementation](https://github.com/apify/crawlee/blob/v3.18.2/packages/playwright-crawler/src/internals/adaptive-playwright-crawler.ts), [predictor](https://github.com/apify/crawlee/blob/v3.18.2/packages/playwright-crawler/src/internals/utils/rendering-type-prediction.ts).

This is closer to Goodfetch than a rule "if there is JavaScript, launch Chrome". The right question is: **does the cheaper means produce the requested result?** The default comparator examines only the `datasetItems`, their number and their equality by index; it does not define, on the user's behalf, product identity, data period or business completeness. A handler that only emits links can produce two empty item lists judged equal even though the discovered links differ. A `resultComparator` suited to the collection contract must therefore be chosen. [Published comparator](https://github.com/apify/crawlee/blob/v3.18.2/packages/playwright-crawler/src/internals/adaptive-playwright-crawler.ts#L341).

This detection can run the handler several times and manages its result writes. Their commit groups storage operations, without constituting a general transaction over all these operations. It suits acquisition with the intended contract; it does not make it safe to repeat a purchase, a submission or a creation on a site. Likewise, a request retry does not ensure a single execution of its external effects. Goodfetch must journal attempts and verify the resulting state before replaying a step that has effects.

The [Impit](https://crawlee.dev/js/docs/guides/impit-http-client) client also shows that a TS API can already use a native Rust transport. The SDK's language is not enough to infer the cost of the critical path. The choice of a Rust runtime for Goodfetch is justified by the unity of the engine, its distribution and resource control, and must then be validated by measurement.

## 6. Spider-rs: a serious engine reference, not a shortcut to a universal agent

The dossier inspects the published crates **Spider 2.53.9** and **Chromey 2.58.2**, and distinguishes their published source from what is new on `main`. Spider offers HTTP, fetch hooks, budgets, page broadcasting, Chrome and several paths conditioned by Cargo features. The code is not just a Rust parser. [Published crate](https://docs.rs/crate/spider/2.53.9/source/), [feature configuration](https://github.com/spider-rs/spider/blob/main/spider/Cargo.toml).

Its smart mode uses browser-necessity signals, notably scripts, mutations, data and framework markers. **This is not the same contract as comparing the expected extraction over HTTP and in the browser.** A page that is hydrated but already complete server-side, and an apparently complete page that omits a piece of data, must both enter the validation corpus. The `smart` feature is important: calling the method without the right compiled path does not guarantee the Chrome upgrade. Spider also has, with `parallel_backends`, response selection by HTML quality score and a customizable `QualityValidator`. This capability must not be ignored; it remains distinct from a comparison of handler outputs suited to the requested result. [Audited details](2026-10-07-crawlee-spider-contracts-followup.md), [published quality hook](https://github.com/spider-rs/spider/blob/e4b53c5b582357e46433fa9ba3d91f930fb67cb4/spider/src/website.rs#L14248).

Persistence and load control also have precise scopes. The SQLite `disk` storage keeps visits and signatures; this schema is not a durable pending/running/done frontier with acknowledgment. `Website::queue()` injects links via an in-memory Tokio broadcast. A retained visits database is therefore not enough to reconstruct all interrupted work. The adaptive semaphore can be provided explicitly; its presence does not guarantee automatic CPU/RAM autoscaling of every crawl. [Published schema](https://github.com/spider-rs/spider/blob/e4b53c5b582357e46433fa9ba3d91f930fb67cb4/spider/src/features/disk.rs#L96), [actual queue](https://github.com/spider-rs/spider/blob/e4b53c5b582357e46433fa9ba3d91f930fb67cb4/spider/src/website.rs#L14692), [semaphore resolution](https://github.com/spider-rs/spider/blob/e4b53c5b582357e46433fa9ba3d91f930fb67cb4/spider/src/website.rs#L6956).

**Spider also has a real local open source agent**, not just a cloud client. The MIT crate `spider_agent` **2.53.9** notably offers `run`, `act`, `observe`, execution memory and HTML/screenshot extraction, with a configurable LLM API and control of in-progress work; its CDP engine relies on Chromey depending on features. The published archive was inspected and identifies commit `e4b53c5b582357e46433fa9ba3d91f930fb67cb4`. This component therefore deserves a separate comparator against the Stagehand primitives and the Browser Use loops. [Published source of spider_agent](https://docs.rs/crate/spider_agent/2.53.9/source/), [detailed contracts](2026-10-07-crawlee-spider-contracts-followup.md).

One caveat, however, prevents turning the configuration list into a list of guarantees: in the inspected published Rust sources of `spider_agent` and `spider_agent_html`, the fields `html_diff_mode`, `planning_mode`, `self_healing` and `confidence_strategy` appear in the types/builders, with no operational read found. This does not demonstrate the absence of these capabilities in the whole ecosystem or in a later release; it means that **the associated savings and behaviors are not established by these fields in this published code**. Test the traces and the actual differences before repeating the token promises.

**The `spider-browser` cloud SDK, the `spider` crawl crate, the `spider_agent` crate and the Chromey driver do not constitute a single product with all the same capabilities.** The cloud SDK exposes primitives close to Stagehand's; this does not prove that they all have the same backend and the same guarantees locally. [Spider](https://github.com/spider-rs/spider), [spider-browser](https://github.com/spider-rs/spider-browser), [Chromey](https://github.com/spider-rs/chromey).

Chromey is a CDP driver candidate and has network/cache/fingerprint extensions. An `Element::click()` method does not, however, establish equivalence with all of Playwright's actionability: resolution at action time, visibility, stability, frames, hit testing and armed waiting must be qualified. Some vendor commands, such as Markdown/WebMCP domains, require a compatible remote engine and are not guaranteed commands of stock Chrome.

**Recommendation:** adapt Spider for collection and compare Chromey/Chromiumoxide for the driver; keep Goodfetch the owner of its result, plan and recovery contracts. Do not fork the whole of Spider before identifying what a narrow adapter does not allow. The protocol, the behavior and the validation must remain Goodfetch's.

The speed multipliers in the Spider benchmark do not rank this trio: Node there is the `crawler` package, not Crawlee, and the current code has asymmetries and stubs to fix. The [reproducible source audit](2026-10-07-benchmark-audit-followup.md) pins the commits and explains the limits precisely, without accusing the old results of having used these same defects.

## 7. Alternatives to follow, classified by mechanism

### Close to Stagehand or complementary

| Project | Why it matters | Useful comparison for Goodfetch |
| --- | --- | --- |
| [Browser Use](https://github.com/browser-use/browser-use) | Agent loop and browser tools; evolving toward native Rust components | Autonomous tasks, retained context, sessions and decision budget; separate SDK, native beta and cloud |
| [Skyvern](https://github.com/Skyvern-AI/skyvern) | Workflows, visual perception and script generation/reuse | Block repair, recovery and amortized cost of a known workflow |
| [AgentQL](https://docs.agentql.com/) | Semantic queries and element resolution on top of the browser | Extraction/targeting with schema; distinguish public SDK and service backend |
| [Magnitude](https://github.com/magnitudedev/browser-agent) | Agent automation based notably on vision | Interfaces poor in semantic DOM, visual changes, cost of screenshots and inference |
| [Imprint](https://github.com/ashaychangwani/imprint) | Discovery and compilation of reusable behaviors | Generated artifact, validity over repetitions and repair conditions; maturity to be read per version |
| [agent-browser](https://github.com/vercel-labs/agent-browser) | CLI, native daemon, snapshots and refs; AI chat now optional | Tool surface and observation volume of an external agent; the deterministic core remains distinct from the chat |
| [Playwright MCP](https://github.com/microsoft/playwright-mcp) / [CLI](https://github.com/microsoft/playwright-cli) | Expose a proven driver to an agent | Baseline for actionability, observation and ergonomics; the model and durability remain elsewhere |

These projects are not all fully self-hostable frameworks with the same scope. The [detailed report](2026-10-07-stagehand-alternatives-followup.md) establishes their mechanisms, observed licenses and SDK/backend boundaries. The [general landscape](2026-10-07-commercial-landscape.md) covers additional services, including Anchor and Airtop.

### Close to Crawlee or Spider

| Project | Mechanism to retain | Comparison limit |
| --- | --- | --- |
| [Scrapy](https://docs.scrapy.org/) | Scheduler, middlewares, pipelines, throttling and resumable jobs | Browser via integration; semantic agent not implied |
| [Scrapling](https://github.com/D4Vinci/Scrapling) | HTTP/browser, spiders and structural selector relocation | Structural similarity does not guarantee business identity |
| [Crawl4AI](https://docs.crawl4ai.com/) | Rendering, Markdown, structured extraction, sessions and dispatcher | Corpus pipeline different from an autonomous action loop |
| [Firecrawl](https://github.com/firecrawl/firecrawl) | Acquisition/extraction API, formats and engine routing | Separate OSS and cloud deployments; AGPL core to examine before reusing code |
| [Colly](https://github.com/gocolly/colly) | Configurable Go HTTP crawl, async, limits and storage | No built-in JS rendering comparable to Chromium |
| [Ferret](https://github.com/MontFerret/ferret) | DSL and plan execution, v2 with VM | v2 alpha; distinguish inspiration from a stable dependency |

Their value is complementary: orchestration, extraction, transport and planning must not be put into a single "speed" column.

### Two concrete nuances often missed

**Scrapling can adapt a selector without an LLM.** In the [parser inspected at commit `43dee004`](https://github.com/D4Vinci/Scrapling/blob/43dee004866e1c46843a9ac293cc1494aa7915d6/scrapling/parser.py), the adaptive CSS/XPath fallback kicks in when no element is found, if the option is enabled and a stored representation is available. It ranks structural resemblances and can return several tied best candidates. The percentage is a similarity score, not a calibrated probability of correctness. If a selector keeps finding a wrong element, this fallback path is not triggered on its own. A data validator remains necessary. [Documentation](https://scrapling.readthedocs.io/en/latest/parsing/adaptive/).

**Crawl4AI offers deterministic scripts.** C4A commands such as GO, WAIT, CLICK, TYPE and REPEAT are a DSL, even though its syntax is presented as close to English. The fact that a command is easy to read does not turn it into semantic resolution by a model. This provides another reference for an executable plan; the LLM extraction strategies remain a separate path. [C4A Script](https://docs.crawl4ai.com/core/c4a-script/), [LLM extraction](https://docs.crawl4ai.com/extraction/llm-strategies/).

## 8. Reddit: scenarios to test, no proof of consensus

The follow-up survey opened **10 threads and 6 comment permalinks**. Authors, dates, metrics and biases are documented in [the evidence dossier](2026-10-07-reddit-evidence-followup.md). No thread found in this sweep provides a common Stagehand/Crawlee/Spider protocol; this does not prove that no discussion exists elsewhere.

Three useful observations:

- A [Stagehand versus Browser Use report from April 15, 2026](https://www.reddit.com/r/AI_Agents/comments/1slvuta/stagehand_vs_browser_use_which_one_actually_works/) reports more completed sessions with Stagehand after erroneous Browser Use clicks. Models, versions and success rates are missing; the author says it may not generalize.
- A [June 2026 comment](https://www.reddit.com/r/AI_Agents/comments/1uhvp85/comment/oubl84a/), on the contrary, describes too much supervision being needed with Stagehand. It does not provide a paired metric either, but it prevents claiming unanimity.
- The [account of rebuilding a stack](https://www.reddit.com/r/WebScrapingInsider/comments/1tkqi5e/we_rebuilt_our_scraping_stack_4_times_in_18/) announces 400 sites, 2 million pages/day, 1.2% failures and -45% cost with Browserbase + Stagehand. No corpus, trace or invoice is given. Hosting, SDK and operations change together: this is not an isolated measurement of Stagehand.

The testimonials point toward authenticated sessions, observability, inference cost and false successes. They do not decide the runtime language or a universal winner. The so-called Rust benchmark discovered on Reddit concerns **Crawlyx, not Spider-rs**, and comes from its author.

## 9. Concrete direction for Goodfetch

Keep the [autonomous Rust runtime + shared protocol + SDK](2026-10-07-direction-goodfetch.md) target, but specify what this comparison imposes on the product:

1. **Task and ResultContract at the center.** Define expected data, identity, completeness, freshness and observable final state. The model proposes an action; an appropriate check decides whether the objective has been reached.
2. **Interchangeable executors.** Documented API/HTTP, parsing, deterministic browser and semantic discovery. Choose according to the result and the history, with the browser used directly on routes known to require it. Avoid a double HTTP-then-browser attempt on every request.
3. **Durable, versioned plans.** The first discovery produces a plan with preconditions, targets, waits and postconditions. Execute this plan without new inference while it remains valid; repair the diagnosed step and record the change.
4. **Three explicit caches.** Perception, plan and data have distinct keys, invalidations and freshness rules. A latency gain is acceptable only if the output remains compliant with the contract.
5. **Crawl frontier separate from action recovery.** Queue, per-site quotas, deduplication, leases and backpressure for collection. Journal, idempotence when it exists and state reconciliation after an incident for actions. Do not announce a universal exactly-once guarantee on third-party sites.
6. **Session as a first-class resource.** Cookies, identity, proxy, geography and isolated context must remain consistent. Pooling and multiplexing must not mix accounts or clients.
7. **Qualified driver.** Native CDP as the target; Playwright serves as reference and compatibility backend. Test waits, frames, detached elements, downloads and recovery before promoting each native path.
8. **Observable economics.** Count all attempts, model calls, rendering, proxies and resources; provide the cost per actually validated result and the time needed to diagnose an incident.

The differentiation to demonstrate is therefore: **the same workflow runs locally or remotely, chooses an economical access method, is reused, survives anticipated changes and makes incomplete results visible.** The engine must remain useful without a cloud service; billable services can cover managed browsers/sessions, capacity, recipe maintenance, observability and support. This positioning is a product hypothesis to be tested on pilots, not an already validated market promise.

## 10. Experiments that must decide what comes next

The [benchmark audit](2026-10-07-benchmark-audit-followup.md) explains why the current tables are not enough. The next comparison must isolate:

- HTTP with equivalent outputs: Spider, Crawlee HTTP, Scrapy/Colly async and Foxhound.
- JS rendering: Spider Chrome, Crawlee Playwright, Crawl4AI and Foxhound CDP.
- Known actions: Playwright, Stagehand primitives, agent-browser CLI and Collie plans.
- Semantic discovery: Stagehand 4, Browser Use, Skyvern, `spider_agent` and Collie, same models/budgets when possible.
- Repetitions and drift: 1/10/100/1,000 runs, controlled DOM mutations, changed data, expired session, interruption between action and validation.

Speed must be reported with coverage, false-success rate, total memory, p50/p95, warm/cold start, costs of all attempts and recovery results. The business cases of authenticated export, paginated catalog and differential collection proposed earlier provide a first corpus; they do not limit the general APIs.

## Provenance and limits of the dossier

The root of this follow-up requested **32 Exa results** in four complementary searches, then opened sources, registries and GitHub trees. The Reddit follow-up requested 55 results, Stagehand 32 and Crawlee/Spider 20, i.e. **139 results requested in this supplement**. The counters are volumes of requested results and may include duplicates or failures; they do not designate as many validated independent sources.

Reading the published code, checking version constraints and the static audit do not prove the performance or success of the cloud in real conditions. Testimonials remain declarative; the benchmark scripts examined were not run. A small Node expression and the aggregation of a public JSON verified, respectively, a stub's semantics and the published totals. The Rust decision remains a design choice to be qualified, with no reduction of ambition tied to the team or the schedule.
