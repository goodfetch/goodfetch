# Index of competitive research and benchmarks

**Research of October 7, 2026.** This index brings together the market notes, technical analyses, audits of published benchmarks and conclusions used to design Goodfetch. The reports contain their sources, inspected versions/commits and limitations.

**Benchmark status:** the initial notes analyze public protocols and results. The following phase built the [local lab](../lab/README.md) and ran controlled campaigns on crawling, browser primitives and storage. These exploratory measurements qualify configurations on fixtures; they constitute neither a ranking of all the players listed nor proof of universal superiority.

## 1. Overview documents

- [Commercial mapping](2026-10-07-commercial-landscape.md): the market and service models.
- [Stagehand / Crawlee / Spider comparison](2026-10-07-stagehand-crawlee-spider-comparison.md): differences in contracts and overlaps.
- [Crawl, pools and workers matrix](2026-10-07-crawl-capability-matrix.md) and [agents, plans and repair matrix](2026-10-07-agent-capability-matrix.md): the consolidated capabilities.
- [Audit of public benchmarks](2026-10-07-benchmark-audit-followup.md): how existing figures were produced.
- [Performance and evidence protocol](2026-10-07-benchmark-and-runtime.md): the proposed comparison protocol.

The current product proposals are in the [consolidated report](../proposals/proposition-goodfetch.md) and the [R&D plan](../plan/rd-plan.md). They build on this research, without turning its conclusions into already demonstrated performance.

## 2. Market and competitor families

| Report | Content |
| --- | --- |
| [Commercial landscape](2026-10-07-commercial-landscape.md) | 64 players, products or families: hosted browsers, agents, collection/extraction, proxies, data platforms and RPA; Browserbase, Apify, Spider, Browser Use, ScrapingBee, Firecrawl and niche players; offerings, observed price units and OSS/cloud boundaries |
| [Browser and agent frameworks](2026-10-07-browser-frameworks.md) | Stagehand, Browser Use, Skyvern, Magnitude, AgentQL, agent-browser, recorder/replay and alternatives; versions, architectures, licenses and limitations |
| [Crawl engines](2026-10-07-crawl-engines.md) | Crawlee, Spider, Scrapy, Colly, Crawl4AI, Firecrawl, Scrapling and other engines; HTTP/browser, scheduling, workers, persistence and extraction |
| [Platforms and services](2026-10-07-product-platform-options.md) | React/shadcn console, Vite/Next, native macOS, daemon/SDK/MCP, local/cloud and operations/monetization; Apify, Browserbase, Spider and Firecrawl references |
| [Initially consolidated research and direction](2026-10-07-direction-goodfetch.md) | Synthesis of the market investigations and mechanisms adopted; extended by the current design dossier |

**Scope:** broad coverage, with no claim to worldwide exhaustiveness nor an audit of every option of every repository. Not all the players listed were examined to the same depth. The features of a commercial service are not presumed to be present in its public repository.

## 3. Detailed technical analyses

| Report | Questions addressed |
| --- | --- |
| [Stagehand / Crawlee / Spider: in-depth comparison](2026-10-07-stagehand-crawlee-spider-comparison.md) | Determinism/AI, adaptive HTTP, storage, sessions, concurrency, StagehandCrawler integration, limits of the comparisons and recommendations |
| [Crawl, resources and workers matrix](2026-10-07-crawl-capability-matrix.md) | Frontier, discovery, recrawl, queues, leases, retries, pools, autoscaling, workers, outputs and licenses; deep dives on Katana and StormCrawler |
| [Agents, plans and repair matrix](2026-10-07-agent-capability-matrix.md) | Browser primitives, perception/vision, references, agents, compilation, cache, validation, repair, CLI/MCP and license/cloud boundaries |
| [Stagehand contracts and alternatives](2026-10-07-stagehand-alternatives-followup.md) | Published Stagehand 4.1.0: calls, cache, self-heal and differences from Playwright; alternatives, replay and result quality |
| [Crawlee and Spider contracts](2026-10-07-crawlee-spider-contracts-followup.md) | TS and Python queues/retries/storage, Stagehand integration, Spider modes, visited versus frontier, workers and published Rust agent |
| [Engines, workers and browsers](2026-10-07-engines-workers-browsers.md) | Table of the languages, runtimes, processes, drivers and browsers actually used |
| [Language and architecture: level of consensus](2026-10-07-language-architecture-evidence.md) | Observed convergences, absence of universal consensus, SDK language versus engine and topology |
| [Runtime ADR](../adr/0001-runtime-language.md) | Rust/Tokio proposal with SDK and CDP; alternatives, consequences and required qualification |
| [ADR-002 access, identity and stealth](../adr/0002-access-identity-stealth.md) | `identified` (default), `standard` and full opt-in `stealth` profiles; open source/cloud split, CAPTCHA tiers, safeguards and qualification |
| [Audited Reddit testimonials](2026-10-07-reddit-evidence-followup.md) | Identified threads/comments, sessions/costs/reliability; testimonials distinguished from comparative evidence |

## 3a. Market demand

| Document | Content |
| --- | --- |
| [Market demand synthesis](market/README.md) | 421 coded public signals (GitHub, Reddit, HN, reviews, 2025–2026 reports), ranking of pain points, needs by segment, price signals, competitors' strengths and misses; linked to the decisions D-segment, D-pricing and D-feature-priority. Willingness to pay not measured: interviews to be conducted |

## 4. Benchmarks: documents and level of evidence

| Document | Benchmarks or measurements examined | Status in this research |
| --- | --- | --- |
| [Audit of public benchmarks](2026-10-07-benchmark-audit-followup.md) | Spider versus Colly, the Node `crawler` package and wget; Stagehand 3.2.1 / Browser Use / Sentinel agent benchmark; Markdown/RAG crawler benchmark; Stagehand evaluations | Inspection of public methods, code and results, pinned commits; Node expression verified and JSON totals recalculated; benchmark tasks not run |
| [Stagehand, CDP and latency](2026-10-07-stagehand-cdp-latency.md) | Published 50-action flow, Stagehand v4 batch versus Playwright, RTT and placement | Reading of the maintainers' result; one published run, not reproduced; no generalization to all tasks |
| [Performance, runtime and evidence protocol](2026-10-07-benchmark-and-runtime.md) | Lightpanda benchmark; publications on memory/compilation/repair; BrowserGym/WebArena/WorkArena; cost, reliability and metrics | Source analysis and protocol proposal; illustrative figures explicitly identified; no Goodfetch benchmark run |
| [Crawl engines](2026-10-07-crawl-engines.md) | Arguments and microbenchmarks of transports/engines, notably Impit and Spider | Figures and configurations from the sources analyzed; no proof of an overall Goodfetch gain |

**Central interpretation:** the experiments do not all provide the same service. An HTTP crawl, a Markdown conversion and a multi-step agent cannot be ranked from a single pages-per-second table. The dossier seeks to identify asymmetries and missing evidence, rather than to declare a universal winner.

## 5. Validation and context notes

| Document | Role |
| --- | --- |
| [Stagehand usage](stagehand-usage.md) | Complementary context note on usage modes |

## 6. Currency of the conclusions

The first market and framework notes kept a small-team/release-in-3–6-months hypothesis. Their opening notice specifies that this constraint has been removed. The [consolidated dossier](../proposals/proposition-goodfetch.md), the [architecture](../proposals/architecture.md) and the [R&D plan](../plan/rd-plan.md) are more recent; the initial documents remain useful for sources and historical reasoning.

Price data, versions and offerings are dated as of the survey; a later decision must revalidate items likely to have changed. The `sources_reviewed` counters refer to requested search results, including duplicates/discarded items, and are not a number of independent, experimentally validated sources.

## 7. Comparative lab proposal

The following request favors a **measurement lab before the console**: isolated installation of competitors, primitives, stress tests and evidence-based choice of engine/driver/placement. The initial report was followed by an [executable implementation](../lab/README.md). The version/competitor notes below describe their prior research phase; current measurements and limitations are distinguished in the lab.

| Document | Role |
| --- | --- |
| [Trio versions and evidence](2026-10-07-lab-engine-evidence.md) | Crawlee, Spider-rs and Stagehand re-verified; v3/v4 compatibility, features, placement and limits of the published figures |
| [Competitor shortlist](2026-10-07-lab-competitor-shortlist.md) | Playwright/Puppeteer/CDP Rust/Go baselines, crawlers, agents, alternative browsers and services; local/cloud and licenses distinguished |
| [Lab test engineering](2026-10-07-lab-testing-engineering.md) | 26 primary sources: oracles, conformance, isolation, open/closed load, statistics, faults, reproducibility and CI |
| [Storage engineering](2026-10-07-lab-storage-engineering.md) | 16 primary sources: JSON/JSONL, SQLite WAL FULL/NORMAL, batching, producers versus writers, sync, integrity and crashes |
| [Operational lab](../lab/README.md) | Installation, backends, commands, protocol, campaigns, outputs and limits of evidence |
| [Local results and architecture to prototype](../lab/reports/qualification-2026-10-07.md) | Campaigns run, verified throughput, batching/contention, explicit failures and limits of the language conclusions |
| [Experimental coverage](../lab/docs/coverage-matrix.md) | Operational code versus backlog; gates before adopting an architecture |

## 8. Integrated qualification — October 8, 2026

- [Contract and method: workflow and shared SQLite](2026-10-08-workflow-and-shared-sqlite.md).
- [Executed results: 153 qualified cells](../lab/reports/workflow-2026-10-08.md), with Node/Rust sync/Go, idempotent recovery and comparison of the Crawlee pool wake-up.

## 9. Later research — October 2026

| Report | Content |
| --- | --- |
| [Goodfetch Cloud architecture and economics](reports/Goodfetch%20Cloud%20architecture%20and%20economics.md) | How comparable clouds are built and priced, the open-source layers around frameworks, AWS runtime, isolation, networking and billing, input costs (proxies, CAPTCHAs, models), workload profiles, unit economics and a revenue model; notes and a runnable model in [research_notes](research_notes/Goodfetch%20Cloud%20architecture%20and%20economics/) |
| [Tabbed Claude Code interfaces](reports/Tabbed%20Claude%20Code%20interfaces.md) | Whether any plugin, mod, GUI, IDE integration or terminal gives each Claude Code sub-agent its own interactive tab, as of October 13, 2026; tooling for the project's agents, not product research; notes in [research_notes](research_notes/Tabbed%20Claude%20Code%20interfaces/) |
| [Competitor codebase design](reports/Competitor%20codebase%20design.md) | How Crawlee (JS and Python), Scrapy, Playwright, Stagehand, Browser Use and Spider organize their code at their heads of October 15, 2026 (Spider's of October 1): paradigm, layering, design patterns, duplication, contracts, errors and quality machinery, with a cross-project scorecard, what Goodfetch's rules already cover and seven candidate gaps; one note per project in [research_notes](research_notes/Competitor%20codebase%20design/) |
| [Spider and Foxhound in `just compare`'s HTTP cells](2026-11-06-spider-gap.md) | Why Foxhound ran 12–14 times slower than Spider 2.53.9 on 2026-11-06: instructions, system calls and threads per page, the run-state and Politeness actors, and what each spec #1838 ticket recovers; behind ADR 0142 |
| [How crawlers keep their frontier and crawl state](2026-11-06-crawler-frontier-persistence.md) | Where Scrapy, Crawlee, Heritrix, Nutch, StormCrawler, Colly, Spider and Firecrawl keep their seen set and queue, when they write it and what a crash loses; behind ADR 0142 |
| [Filter lists blocking with no pause per request](2026-11-12-filter-lists-blocking-without-pause.md) | Whether `Network.setBlockedURLs`, Chromium's subresource filter, an extension's declarative rules or host matching in the egress relay can give the engine's Filter list verdicts with no pause per request, read in Chromium 156.0.8078.4, `adblock` 0.13.3 and uBO Lite; none is exact, the closest is a declarative-rules extension; for #2436 |
