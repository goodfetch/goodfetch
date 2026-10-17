# Commercial web automation market — October 7, 2026

> **Scope update:** the user withdrew the small-team and 3–6-month release constraint after the first draft. The planning and TS-first passages below relate to that initial assumption. The current target is a Rust engine shared by the SDKs, the CLI and the cloud; see [the updated synthesis](2026-10-07-direction-goodfetch.md). The mapping, the recorded prices and the caveats remain applicable.

## Conclusion for a small team, MVP in 3–6 months

**The defensible niche is not "a faster AI browser". It is an open source framework that turns three recurring B2B workflows into portable, checked and economical automations, followed by paid services to put them into production and maintain them.** The first proposed segment is developers, small agencies and SaaS integrators who collect data from authenticated supplier/distributor portals.

This conclusion is a **strategic inference**, not proven market demand. It must be tested with customers. Browser infrastructure, AI primitives, Markdown, proxies, MCP and selector repair are already widely available. The advantage must appear on repeated, verified tasks, with lower total cost and maintenance time.

The market is already converging on AI that builds the workflow, deterministic code that replays it, and AI that steps in when things change. [Airtop explicitly describes this compilation](https://www.airtop.ai/); [Anchor describes deterministic tasks and an action cache](https://anchorbrowser.io/). This pattern is useful, but is therefore not enough to establish commercial novelty.

## Method and scope

- Official commercial sources and vendor repositories were favored. Vendors' performance claims are claims, not comparative results validated by this research.
- `sources_reviewed = 155`: sum of the `numResults` requested in **13 Exa searches**, in line with the skill's convention; this number is neither that of independent sources nor that of fully read pages.
- **93 unique URLs requested for direct reading, 88 returned and inspected, 5 read failures.** The searches cover five initial angles, six deep dives and two gap passes.
- **64 players, products or families listed** below; a withdrawn product is isolated as a historical reference. Apify/Crawlee and Browserbase/Stagehand are deliberately grouped to avoid commercial double counting. Robocorp/Sema4.ai and TestMu AI/LambdaTest are also grouped.
- This mapping seeks broad coverage, from infrastructure provider to vertical product and local alternatives. It does not claim absolute exhaustiveness: the market moves, and individual projects and niche services are numerous.
- "Service" means that the documented commercial product is hosted/proprietary; it is not a claim that no client code or ancillary tool is public. A public SDK does not make the hosted engine open source. Uninspected licenses are indicated as such.
- Prices in USD, observed in public sources on October 7, 2026, without negotiation, taxes or billing tests. Prices are not comparable without fixing the same resources, network, models and success criteria.

## 1. Remote browsers and hosted agents

| Player / official source | Documented product and differentiation | Open source / service boundary |
|---|---|---|
| [Browserbase / Stagehand](https://docs.browserbase.com/welcome/introduction) | Platform that bundles browsers, search, fetch, runtime and model access; Stagehand adds the action, observation and extraction primitives. | Stagehand and CLI public; fleet service and commercial platform separate. |
| [Browser Use](https://browser-use.com/) | Two products: an agent that performs the task and a remote browser for one's own agent. | MIT framework; the [README distinguishes the OSS agent and the cloud agent](https://github.com/browser-use/browser-use), without promising their equivalence. |
| [Steel](https://steel.dev/) | Browser runtime, persistent sessions/profiles, credentials, files, extensions, observation and connections to the usual frameworks. | [steel-browser Apache-2.0](https://github.com/steel-dev/steel-browser), self-hostable; paid cloud operation. |
| [Browserless](https://www.browserless.io/) | WebSocket connections, BrowserQL, REST and MCP; dedicated or on-customer deployment. | **Source available, SSPL or commercial license**, according to the [LICENSE](https://raw.githubusercontent.com/browserless/browserless/main/LICENSE). Not to be treated as a permissive component freely reusable in a proprietary SaaS. |
| [Hyperbrowser](https://hyperbrowser.ai/docs/introduction) | Cloud Chrome browsers, SDK and Puppeteer/Playwright compatibility; [hosted agents](https://hyperbrowser.ai/docs/agents/overview) and Stagehand integration. | HyperAgent announced as open source; commercial managed fleet. Framework license not inspected here. |
| [Anchor](https://anchorbrowser.io/) | Identities, authentication, computer use environments, deterministic execution and deployment in the customer's cloud. | Service; do not confuse public SDK and infrastructure core. |
| [Airtop](https://www.airtop.ai/) | Builds and tests workflows, compiles them into reusable code and runs them on demand or on schedule; positioned on finance, marketing and sales functions. | Service and integrations; OSS core not established in this corpus. |
| [KERNEL](https://www.kernel.sh/) | Isolated Chromium sessions, profiles, managed auth, warm pools, CDP/BiDi, replays and process/file tools. | Compute/browser service; SDK and CLI distinct from the hosted engine. |
| [Notte](https://www.notte.cc/) | Sessions, agents and server functions, personas, secrets vault and replay; turns a browser workflow into an API. | [Public components](https://github.com/nottelabs/notte), hosted service; boundary/license to inspect before reuse. |
| [TinyFish](https://www.tinyfish.ai/) | Search, fetch, browser and agent; authentication via vault and profiles, recordings, enrichment and portal automation. | Service; open benchmarks or SDKs do not make the whole offering OSS. |

## 2. Collection, extraction, proxies and unblocking

| Player / official source | Documented value proposition | Open source / service boundary |
|---|---|---|
| [Apify / Crawlee](https://apify.com/pricing) | Actor execution, storage, proxies and marketplace; connector monetization and distribution. | [Crawlee Apache-2.0](https://github.com/apify/crawlee), library usable outside the platform; Actors and operations are not all public. |
| [Spider Cloud / spider-rs](https://spider.cloud/open-source-vs-cloud/) | Same Rust crawl engine locally and in the cloud; the service handles fleet, network, anti-bot and extraction. | [MIT ecosystem](https://spider.cloud/open-source/); managed services and their parity cannot be inferred from the license. |
| [Firecrawl](https://www.firecrawl.dev/pricing) | Search, scrape, crawl, map, extraction and interaction, LLM-ready outputs and volume plans. | [AGPL-3.0 engine](https://github.com/mendableai/firecrawl), commercial service; self-hosting parity must be tested endpoint by endpoint. |
| [Bright Data](https://brightdata.com/products) | Proxy/network, Web Unlocker, Browser API, per-target scrapers, datasets and managed services. | Proprietary offering; data distribution and access capacity are the major assets. |
| [Oxylabs](https://oxylabs.io/products) | Proxies, Web Unblocker, Scraper API, browser, search and video data; OxyCopilot tooling. | Proprietary offering; integration possible with OSS frameworks. |
| [Zyte](https://www.zyte.com/zyte-api/) | Automatic routing between HTTP and browser, ban management, sessions and structured extraction. | Scrapy constitutes a separate OSS asset; Zyte API and operated services are commercial. |
| [ZenRows](https://www.zenrows.com/) | Fetch, Extract, Browser Sessions and Batch platform with CLI/MCP/SDK; managed anti-bot. | Service; integration tools distinct from the fleet. |
| [Scrapfly](https://scrapfly.io/) | Scrape API, browser, screenshot, extraction, crawler, agent and MCP; observability and operated network/browser engines. | Curlium and Scrapium presented as proprietary; SDK and integrations distinct. |
| [ScraperAPI](https://www.scraperapi.com/) | General API and structured Amazon, Google and Walmart endpoints; collection pipeline and asynchronous processing. | Infrastructure and parsing service. |
| [ScrapingBee](https://www.scrapingbee.com/llms/html-api.txt) | URL API with JS, scenarios, screenshots, rules and extraction; automatic cost escalation with a cap. | Service; the public clients do not constitute a complete self-hostable engine. |
| [ScrapingAnt](https://scrapingant.com/) | Chrome API, Markdown and AI extraction, MCP, standalone residential/datacenter proxies. | Service; API credits and proxy bandwidth distinct. |
| [Nimble](https://www.nimbleway.com/) | Search and collection for agents, auditable search plans, memory/index and monitoring; specialized business scenarios. | Retrieval and data service. |
| [Crawlbase](https://crawlbase.com/docs) | Crawling API, per-target parsing, browser rendering and MCP; multi-language clients. | Service; the standalone Scraper API has been closed to new accounts since 2024, [the function is carried by Crawling API](https://crawlbase.com/docs/scraper-api). |
| [HasData](https://hasdata.com/apis/web-scraping-api) | General and SERP API, HTML/Markdown/JSON, deterministic or AI extraction, screenshots and network options. | Service; catalog of specialized endpoints. |
| [Scrapingdog](https://www.scrapingdog.com/) | General scraping and targeted SERP, ecommerce and social platform APIs. | Service; value of very targeted connectors. |
| [Scrape.do](https://scrape.do/) | General API and ready-to-use scrapers, anti-bot and JS interactions. | Service; the homepage shows standalone browser/proxies as "Coming Soon", so do not count these two products as available on this evidence alone. |
| [Scrapeless](https://www.scrapeless.com/en) | Agent Browser, Web Unlocker, Crawl, network and extraction of AI engine answers. | Service; distinct niche of observing what ChatGPT, Perplexity, etc. answer. |
| [Scraping Fish](https://scrapingfish.com/) | Browsers and mobile proxies at an identical price per successful request, with no announced feature multipliers. | Service; budget simplicity is its positioning. |
| [ScrapeNinja](https://scrapeninja.net/) | HTTP engine with Chrome fingerprint, browser engine, retries, playgrounds and JS extractors. | Service; shows that two HTTP/browser paths are already a common product. |

## 3. Semantic data, retrieval and vertical platforms

| Player / official source | What the customer buys | Open source / service boundary |
|---|---|---|
| [Diffbot](https://www.diffbot.com/) | Structured extraction, crawl, entities, knowledge graph and search, including an announced self-hostable index offering. | Service and data product; not to be reduced to a Markdown converter. |
| [Kadoa](https://www.kadoa.com/) | Pipelines for finance, automated maintenance, quality rules and per-value provenance; operations team for uncertain repairs. | Vertical product and services; on-prem/private cloud offered. |
| [AgentQL](https://www.agentql.com/) | Semantic query language, selection/extraction via Playwright, debugger and API, including PDF parsing. | Public SDK/query language do not imply a fully OSS resolution service. |
| [ScrapeGraphAI](https://github.com/ScrapeGraphAI/Scrapegraph-ai) | Extraction pipelines from prompts and graphs; separate cloud offering announced in the README. | MIT Python library; separate paid enhanced platform. |
| [DumplingAI](https://www.dumplingai.com/) | A single API/CLI/MCP surface over several providers, routing, normalized formats and pinning of the provider endpoint. | Commercial aggregator; direct competition for a simple "multi-vendor router". |
| [Exa](https://exa.ai/docs/reference/search-api-guide) | Search, index, latency/depth modes and [content retrieval](https://exa.ai/docs/reference/contents-api-guide). | Commercial API, clients and integrations; discovery/retrieval layer. |
| [Linkup](https://docs.linkup.so/pages/documentation/tutorials/migrating-from-tavily) | Search with sourced answer or JSON schema; page fetch as Markdown. | Commercial API; retrieval alternative, not an action framework. |
| [Tavily](https://docs.tavily.com/welcome) | Search, Extract, Crawl, Map and research; agent integration. | Commercial API; [credits per operation](https://docs.tavily.com/documentation/api-credits). |
| [Jina Reader](https://jina.ai/reader/) | Converting pages into usable content and retrieval tooling. | Reader product and public ecosystem; precise license/non-parity not studied here. |

## 4. Local agents, workflows and alternatives for business users

| Player / official source | Positioning / competitive relevance | Open source / service boundary |
|---|---|---|
| [Skyvern](https://www.skyvern.com/) | Visual workflows, auth/2FA, forms, document collection and APIs; self-hostable containers. | [AGPL-3.0](https://github.com/Skyvern-AI/skyvern) and commercial offering. |
| [Magnitude](https://magnitude.run/) | Vision-oriented agent, actions and extraction; visual assertions and test runner. | [Apache-2.0](https://github.com/magnitudedev/browser-agent); a library rather than an Apify platform clone. |
| [LaVague](https://docs.lavague.ai/en/latest/) | World Model, Action Engine that generates code, Selenium/Playwright drivers and measurement/debug tools. | [Apache-2.0](https://github.com/lavague-ai/LaVague/), enterprise support announced. |
| [Nanobrowser](https://www.github.com/nanobrowser/nanobrowser) | Local extension, already logged-in accounts, BYOK, multi-agent approach. | Apache-2.0; free alternative for individual uses. |
| [BrowserOS neo](https://www.browseros.com/) | Local browser for agents, import of logged-in accounts, compact snapshots, batched actions and replay. | Announced as open source; license of the neo version not inspected here. |
| [Lightpanda](https://lightpanda.io/) | Zig headless engine with JavaScript, without graphical rendering; memory/CPU focus rather than a business product. | Public engine; any cost reduction must be weighed against its compatibility on the target tasks. |
| [Browserflow](https://browserflow.app/) | Workflow builder in the browser or cloud, commands and custom JavaScript. | Commercial business product. |
| [Axiom](https://axiom.ai/) | No-code extension, steps SDK/API, orchestration and desktop/cloud execution. | Commercial product that already covers the move from builder to APIs. |
| [Bardeen](https://www.bardeen.ai/) | Scraping and lead qualification/enrichment; vertical GTM commercial positioning. | Commercial product; avoid head-on generalist competition on lead generation. |
| [Browse AI](https://www.browse.ai/) | No-code extraction/monitoring, columns proposed then approved, exports and integrations. | Commercial business service. |
| [Octoparse](https://www.octoparse.com/) | No-code application, auto-detection/flowchart, logins, pagination, cloud, exports and services. | Application and cloud; not an OSS framework. |
| [ParseHub](https://www.parsehub.com/) | Visual scraping tool. | Commercial product; poor text readout, current details not verified in this corpus. |
| [ScrapeStorm](https://www.scrapestorm.com/) | Data identification, visual builder, desktop execution, cloud and database/file destinations. | Commercial product; enterprise services offered. |
| [Roborabbit, formerly Browserbear](https://www.browserbear.com/) | No-code builder, REST APIs, cloud tasks and no-code connections. | Commercial product; the historical domain displays the Roborabbit brand. |
| [changedetection.io](https://changedetection.io/) | Change monitoring, browser steps, filters, alerts and managed browser. | Self-hostable project and service; precise license not inspected here. |
| [Visualping](https://visualping.io/) | Visual/text/code monitoring, importance assessment and history for business monitoring. | Specialized service. |
| [Distill](https://distill.io/) | Monitoring by element selection, conditions, versions, private pages/PDF and cloud. | Service and extensions; adjacent need for recurring collection. |

## 5. QA, RPA and adjacent distribution channels

| Player / official source | Importance for the project | Boundary |
|---|---|---|
| [BrowserStack Automate](https://www.browserstack.com/automate) | Multi-browser/device infrastructure, Playwright and test reliability/self-healing tools. | Commercial cloud; comparing a QA-oriented fleet to a collection fleet requires different workloads. |
| [Sauce Labs](https://www.saucelabs.com/) | Web/mobile testing, creation/diagnostic agents and enterprise reporting. | Enterprise QA service. |
| [TestMu AI, ex-LambdaTest](https://www.testmu.ai/) | Test cloud and QA agents, CLI and parallel execution. | The [current brand presents itself as formerly LambdaTest](https://www.testmu.ai/). |
| [Checkly](https://www.checklyhq.com/) | Monitors as code, Playwright tests, alerts and observability management; good comparison point for DX and operations. | Service; tooling and SDK distinct from the infrastructure. |
| [UiPath](https://www.uipath.com/) | Orchestration of agents, robots, APIs and humans; enterprise governance/processes. | Commercial platform; competing with all of RPA exceeds a 3–6-month MVP. |
| [Robocorp / Sema4.ai](https://robocorp.com/) | The Robocorp domain now displays Sema4.ai, back-office agents; the [Robocorp browser libraries](https://robocorp.com/portal/tutorial/browser-and-web-automation) remain a technical reference. | Distinguish libraries and the current enterprise offering. |
| [n8n](https://n8n.io/) | Integration channel and workflows; code, human in the loop and local/cloud choice. | [Sustainable Use License and enterprise license](https://docs.n8n.io/sustainable-use-license/); building a node or selling consulting is allowed, offering billed hosted n8n is restricted. |
| [Dify](https://github.com/langgenius/dify) | LLM application/workflow platform and plugins; destination for a future connector. | [Modified Apache-2.0 license](https://raw.githubusercontent.com/langgenius/dify/main/LICENSE) with multitenancy and frontend conditions; not to be treated as standard Apache. |

**Historical reference, outside the active market considered: [Reworkd](https://reworkd.ai/) announced the shutdown of its product on February 6, 2025.** Its ideas of schema-based extraction remain interesting; this page does not establish a provider usable today.

## 6. Deep dive on the six most structuring players

### Browserbase / Stagehand

The model combines a public framework with a broad managed platform. Distribution through the framework leads to execution, identity, search and model services. Your framework's architecture must therefore remain usable independently of the fleet provider. The rival is no longer just "remote Chrome", but the reduction of the number of integrations to assemble. [Official documentation](https://docs.browserbase.com/welcome/introduction).

### Browser Use

The public framework attracts developers; the service sells both the browser and the agent that completes a task. The README explains that the OSS and cloud paths have different capabilities. Do not measure the cloud agent against a simple local framework and attribute the whole gap to the language, the browser or the license. [Product boundary](https://github.com/browser-use/browser-use).

### Apify / Crawlee

Crawlee is the library; Apify provides the runtime, billing, destinations and above all distribution via Actors. The marketplace can become an initial channel for your connectors, without forcing your framework to depend on the Apify runtime. A commercial advantage must cover maintenance and the delivered data. Apify documents dedicated pages, billing, documentation and infrastructure for creators. [Publishing](https://docs.apify.com/platform/actors/publishing).

### Spider

The product claims the same Rust engine locally and in the cloud, with Python/Node bindings. The paid value lies in operations, network, output quality and anti-bot. It is a good example of services around a permissive core. But the marketing pages demonstrate neither the parity of the cloud browser with Chromium, nor the fidelity of a complex authenticated workflow. [OSS/cloud comparison](https://spider.cloud/open-source-vs-cloud/).

### ScrapingBee

The customer buys a readable API, with HTTP → JS → premium → stealth escalation and a cost cap. This policy is already documented: an economical router must therefore bring additional evidence, for example on persistent identity, workflow resumption and the quality of recurring data. Auto-mode does not change the customer's waits or scenarios. [HTML API contract](https://www.scrapingbee.com/llms/html-api.txt).

### Firecrawl

The offering is designed to find, read and structure the web from a single API. It is strong competition if your product boils down to Markdown/crawl/extraction for RAG. Its billing units distinguish page, search and browser-minute. The page specifies that a returned HTTP 403/404 can be billed: the billed technical unit and the useful business data are different concepts. [Pricing](https://www.firecrawl.dev/pricing).

## 7. Verified pricing: reason in units, not in headline price

| Player | Verified public unit and price | Consequence for your model |
|---|---|---|
| [Browserbase](https://www.browserbase.com/pricing) | Developer $20/month: 100 browser-hours, 25 concurrent, 1 GB proxy; overage $0.12/h and $12/GB. Startup $99: 500 h, 100 concurrent, 5 GB; then $0.10/h and $10/GB. | The hour is distinct from network and models; the offering bundles several products. |
| [Browser Use](https://browser-use.com/pricing.md) | $0.02/browser-hour, residential proxy $5/GB, direct egress/BYOP $0.20/GB; started minute and one-minute minimum. Agent V4: model cost and 20% service fee, browser/network extra. | Selling a browser-hour alone opens a price war; BYOK does not necessarily remove platform fees. |
| [Apify](https://apify.com/pricing) | Starter $19/month and $19 of usage included; CU $0.20. 1 CU = 1 GB RAM for one hour. Scale $199, CU $0.16; Business $999, CU $0.13. | Cost depends on RAM and time, with proxies, storage and any Actor price to add. |
| [Spider](https://spider.cloud/pricing) | $1/GB of content before transformation + $0.001/CPU minute on the pricing page; errors $0. No subscription; premium network and AI extraction separate. | A small Markdown output does not mean a low bill: measure input bytes and compute. See divergence below. |
| [ScrapingBee plans](https://help.scrapingbee.com/en/article/available-plans-explained-kbinm/) and [contract](https://www.scrapingbee.com/llms/html-api.txt) | Official help dated October 17, 2025: $49.99/250k credits, $99.99/1M, $249.99/3M, $599.99/8M. HTML: 1 credit HTTP, 5 JS, 10 premium HTTP, 25 premium JS, 75 stealth JS. | Plan prices to reconfirm in the dashboard: dynamic pricing page hard to extract. The same 1,000 URLs can consume 1k to 75k credits. |
| [Firecrawl](https://www.firecrawl.dev/pricing) | Hobby $19/month, 5,000 credits; $16/month if annual. Basic scrape/crawl/map 1 credit/page; search 2/10 results; interact 2/browser-minute; JSON/question/highlight add 4/page on scrape/crawl. | Compare the features actually needed; a credit is not a "universal request". |
| [Zyte API](https://www.zyte.com/zyte-api/) | PAYG HTTP $0.13–1.27/1,000 responses; rendered browser $1.01–16.08/1,000 depending on domain tier. Reduced prices with commitment. | A more expensive per-domain cost can be better if useful responses require much less maintenance. |
| [Browserless](https://www.browserless.io/pricing) | One unit covers up to 30 s per connection; reconnection is a new unit. 6 units/MB residential, 2 units/MB datacenter and 10/successful captcha. | Duration and network converted into the same credit; compare caps and reconnections. Plan price not recorded, incomplete rendering. |
| [Scraping Fish](https://scrapingfish.com/) | $0.002/successful request announced, browser/mobile proxy options bundled. | The readability of the offering can compete with sophisticated pricing. |
| [Apify creators PPE](https://docs.apify.com/actors/publishing/monetize/pay-per-event.md) | Creator profit = 80% of billed event revenue − platform costs; specific free/standby rules. | Marketplace = distribution and billing, not an automatic 80% gross margin. |

### Divergences and comparison limits

Spider displays **$0.0001/CPU minute** on the [homepage](https://spider.cloud/) in the inspected results, but **$0.001** on the [pricing page](https://spider.cloud/pricing), i.e. a factor of ten. The [OSS/cloud](https://spider.cloud/open-source-vs-cloud/) page also uses "Pay per page". Take the pricing page as the working reference, but ask for confirmation before budgeting a significant production; this investigation does not resolve the actual billing contract.

The success percentages and marketing multipliers displayed by Steel, Browser Use, Anchor, Scrapfly, Zyte or Lightpanda do not necessarily cover the same corpus, protocol, model, date or unit. One can learn from their methodologies; one cannot establish a universal ranking from their homepages.

### Calculated example, not a benchmark

Deliberately identical assumption: **10,000 tasks, one browser minute each and 5 GB of total measured residential traffic**. The tasks here require neither a hosted agent nor paid extraction. This represents 166.67 browser-hours; choosing one minute here avoids the bias of Browser Use's minimum billing for shorter sessions.

- Browser Use: 166.67 × 0.02 + 5 × 5 = **$28.33**.
- Browserbase Developer: $20 including 100 hours and 1 GB, then 66.67 × 0.12 + 4 × 12 = **$76**.
- Browserbase Startup: **$99**, with the hours and 5 GB covered.

The calculation does not account for differences in availability, speed, quotas, subscription cycles, new accounts, taxes, retries or bytes actually billed. It illustrates one point: **network can far exceed browser compute**. Reducing bytes, reconnections and model calls can pay off more than shaving a few hundred milliseconds off browser launch. The values are derived from [Browser Use pricing](https://browser-use.com/pricing.md) and [Browserbase](https://www.browserbase.com/pricing), not from measurements.

## 8. Where a small team can sell services

The following propositions are hypotheses to test. "Underserved" here means a combination for which the open source developer offering is less evident in the corpus, not the absence of competitors.

| Segment / need | Competition that already exists | Plausible initial offering | Evidence required |
|---|---|---|---|
| Small agencies and B2B SaaS: recurring collection of catalogs, prices, availability or documents from authenticated portals | Skyvern, TinyFish, Anchor, Airtop, Axiom and RPA cover pieces or complete offerings. | 3 maintained connectors with persistent sessions, pagination, dates/versions and destinations; exportable framework. | 3 paying partners, same workflows run for several weeks, result reconciled against customer expectation. |
| Execution in the customer's cloud or locally, with the option of a managed provider | Steel, Browserless, Kadoa, Skyvern and RPA platforms already offer self-hosting or private deployment. | Installation/support/updates, secrets and provider adapters; demonstrated ease of migration. | Reproducible installation, same workflow and output locally + on at least one provider. |
| Measurable maintenance, recovery after breakage and diagnostics readable by a developer | Kadoa does quality/provenance/self-repair; Stagehand, Airtop and Anchor optimize execution. | Workflow contract, acceptance tests, traces and repair ticket with diff; targeted maintenance service. | Controlled breakage cases, repair cost and validation without false success. |
| Cost reduction at moderate volume without rewriting | Adaptive HTTP/browser at ScrapingBee, Zyte, Spider, ScrapeNinja; DumplingAI aggregation. | Optimization audit and shadow mode that recommends HTTP/API/replay without immediately changing execution. | Before/after bills, bytes, models, total cost and maintained quality. |
| Destinations and integration into existing tooling | Apify, n8n, Dify and established no-code tools. | n8n/MCP connector + S3/webhook/CSV exports; pipeline deployment and operation. | Complete pipeline the customer can take over; not just a CLI demo. |

### Recommended packaging, without an anticipated universal SLA

1. **OSS framework genuinely useful on its own**: local creation/execution, versioned workflow, developer-defined validation, traces, resumption and provider interfaces. Do not artificially limit basic functions necessary for reliability.
2. **Paid onboarding per connector or workflow**: authorized access to portals, customer schema, auth, validation, destinations and acceptance. Give a concrete scope: sites, accounts, frequency and limits.
3. **Recurring maintenance**: covered site changes, updates, triage and repair. Define a response time for the pilot scope; a quantified availability commitment will come after measurement.
4. **Optional managed cloud**: transparent technical usage, spending caps, logs/replay and version management. Buy network/browser downstream; do not build a global proxy fleet or a browser engine during the MVP.
5. **Support and private deployment**: installation and operation in the customer's cloud for partners who justify it. Add enterprise features only with real commercial demand.

The simplest initial pricing to test combines **setup fee + maintenance subscription + documented cloud consumption**. Do not universally bill a "business success": a catalog, a downloaded PDF and an ERP write require different definitions and responsibilities. Technical credits or run quotas must not hide model and network consumption.

Apify can serve as a channel to sell a connector early; n8n/MCP can serve for acquisition and integration. Keep workflows exportable and the framework usable outside these platforms. The full marketplace platform, the universal business builder and the proxy network must remain out of the MVP.

## 9. Concrete product decision and stopping criteria

**Recommended choice: TypeScript/Playwright and composable Crawlee building blocks, plus a portable workflow format and a connector maintenance service.** Buy browsers/proxies if necessary, and allow local execution. The Rust language or a new engine can be studied later if measured profiles establish that runtime CPU/RAM dominate customer cost; the commercial catalog does not prove that this condition is met.

The three pilot workflows must test different difficulties: a paginated catalog with customized prices, a document download with explicitly handled auth/MFA, and an incremental collection with resumption after interruption. Give each workflow a result oracle, a cost limit and a procedure for expired auth and site changes. Use the same scenarios against the relevant alternatives, and measure cold start, end-to-end p50/p95, accuracy/completeness, bytes, model calls and maintenance.

Two risks can invalidate the project: customers only agree to pay the low price of raw infrastructure; or portals require human intervention too frequently for the envisaged margins. The MVP must therefore sell a first integration before broadening the framework, then show that maintaining the second and third workflows costs less. No reliability percentage or cost advantage must become a commercial promise before this test.

## Appendix — Failed direct reads and additional sources inspected

Failed URLs: `https://apify.com/platform`, `https://spider.cloud/docs`, `https://docs.zyte.com/zyte-api/usage/price.html`, `https://saucelabs.com/products/browser-testing`, `https://docs.browserbase.com/features/browserbase-functions`. Alternative official pages were used; these failures do not mean the corresponding product is unavailable.

Useful additional sources: [Apify Actors usage and resources](https://docs.apify.com/platform/actors/running/usage-and-resources), [Scrapfly getting started](https://scrapfly.io/docs/scrape-api/getting-started), [Browserless BaaS](https://docs.browserless.io/baas/start), [Spider ecosystem](https://spider.cloud/guides/spider-oss-ecosystem/), [ScrapingBee credits](https://help.scrapingbee.com/en/article/credit-system-explained-1h2ackp/), [Tavily Crawl](https://docs.tavily.com/documentation/api-reference/endpoint/crawl), [DataForSEO SERP](https://dataforseo.com/apis/serp-api), [SerpApi](https://serpapi.com/), [Scrapling](https://scrapling.readthedocs.io/en/latest/). The last three are additional players or technical projects to include in the benchmark/framework corpus, outside the 64 consolidated commercial rows above.
