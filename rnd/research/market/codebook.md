# "Market demand" track: codebook and coding protocol

**Status:** v1, frozen for the corpus of October 8, 2026 (`corpus.jsonl`, 421 items). **Date:** 2026-10-08.
**Scope:** this document defines the codes applied to the corpus items, how to apply them, the agreement and source checks, and the exclusion rules. It contains no analysis results; the rankings are in [pains-ranking.md](pains-ranking.md).

## 1. Unit of analysis

A **canonical item** is a single statement by an identifiable actor on a topic: a GitHub issue opening, an HN comment, a Reddit post or comment (via snapshot), a review or review summary, a report, a blog post, a freelance job offer. A single thread can produce several items if they carry distinct statements, for example `reddit-0` and `reddit-1`, or `hn-32` and `hn-33`. URL and content duplicates were merged upstream by the workflow. The deduplication cannot be replayed from the files in this folder alone.

Fields of each item (`corpus.jsonl`):

| Field | Meaning |
| --- | --- |
| `id` | Collector prefix (`github`, `hn`, `reddit`, `reviews-reports`, `demand-use-cases`) followed by a rank; gaps in the numbering come from upstream filtering |
| `url`, `date` | Source and date of the statement; `YYYY-MM` when only the month is known. For Reddit, the date is that of the snapshot (§5) |
| `source_type` | `github-issue`, `github-discussion`, `hn`, `reddit`, `review`, `report`, `blog`, `job-post`, `other` |
| `product` | Targeted product, as named by the source (`general` if none) |
| `kind` | `pain`, `feature-request`, `use-case`, `pricing-signal`, `statistic`, `switching`, `praise` |
| `segment` | Estimated segment: `agent-developer`, `data-team`, `b2b-portal-integration`, `ecommerce-pricing`, `seo-marketing`, `enterprise`, `research-academic`, `hobbyist`, `unknown` |
| `codes` | 1 to 3 leaf codes from the §3 table; **the first is the primary code** |
| `engagement` | Raw measure specific to the source (§4), not comparable from one source to another |
| `promo`, `vendor_affiliated` | Promotional content; author linked to a vendor (repository member or collaborator, founder, employee, competitor) |
| `summary` | English paraphrase written during coding; it is not a quotation |
| `quote` | Verbatim quotation of fewer than 15 words, present only if it respects this limit |
| `source_checked`, `check_note` | Item successfully checked in §5, with any reservation |

## 2. Coding protocol

1. **Collection.** Five independent collectors (GitHub, Reddit, HN, reviews and reports, demand and use cases) worked read-only, without bypassing any access block (Reddit went through a third-party snapshot site, for lack of direct access). The Reddit and HN collectors recorded no usernames, and the corpus contains none. Items that could not be dated were discarded at collection. The coverage limits of each collector are given in the [README](README.md#5-coverage-and-gaps).
2. **First coder (LLM).** Each item receives 1 to 3 leaf codes. The primary code is the main need or pain of the statement; the following codes describe explicitly present secondary needs. Parent codes (without `parent`) are never assigned directly; in the final corpus, all codes used are leaves, and all 30 leaves are used. Average: 1.79 codes per item (755 assignments for 421 items).
3. **Decision rules.**
   - We code what the statement **says**, not what the cited product does.
   - `kind = statistic` and the codes `market-adoption-stats`, `wtp-price-points`, `build-vs-buy-switching` and `usecase-*` are **context codes** (market, price, use cases). They are ranked separately from pains in `pains-ranking.md`.
   - A block presented as a crash or a 500 falls under `silent-false-success` in addition to `access-antibot-ip`. A loop that bills falls under `cost-unpredictable-runaway` in addition to `rel-hang-crash`.
   - `agent-deterministic-replay` covers the **request** to compile, cache or replay. Complaints about the cost of an agent without this request fall under `agent-token-latency-cost`.
   - The segment is estimated from the stated context. Lead-generation offers are attached to `seo-marketing`, for lack of a "sales" segment.
4. **Independent second coder.** They recoded a **stratified sample of 120 items** without looking at the first coder's codes. Results:
   - **Agreement on the primary code: 83.3%** (100/120).
   - **Mean Jaccard over the full set of codes: 0.838.**

   These rates are raw agreements, without chance correction. A kappa was not computed: the coding is multi-label and the detail of the disagreements was not passed on to this synthesis.

   Interpretation: the agreement is sufficient to rank pain families. It does not allow separating codes whose scores differ by less than about 15%.
5. **Source check** (§5). On 60 sampled items, the URL, the date and the faithfulness of the summary were verified.

## 3. Codes

**n** counts the items carrying the code, whatever its position, and **primary n** those where it comes first. The definitions are reproduced as they were used for coding.

| Code | Label | Parent | Definition (coding text, original English) | n | primary n |
| --- | --- | --- | --- | ---: | ---: |
| `cost-billing` | Cost and billing | — | Parent: complaints and signals about what scraping/automation costs and how it is billed. | — | — |
| `cost-unpredictable-runaway` | Unpredictable or runaway spend | `cost-billing` | Opaque credits, credit multipliers, surcharges, bandwidth billing, expiring credits, charges on failure, and missing hard spend caps or budgets (including retry loops burning money). | 28 | 24 |
| `cost-infra-proxy-spend` | Proxy/captcha/infra spend growth | `cost-billing` | Residential proxy, captcha-solving, compute and hosting costs that grow faster than value; TCO of the access stack. | 19 | 11 |
| `access-blocking` | Access and blocking | — | Parent: getting to the content at all. | — | — |
| `access-antibot-ip` | Anti-bot, captcha, fingerprint and IP blocks | `access-blocking` | Cloudflare/WAF/captcha/fingerprint detection, stealth regressions, IP bans, proxy support, geo/locale mismatch. | 76 | 37 |
| `access-auth-session` | Authenticated sessions and logins | `access-blocking` | Login persistence, cookies/storage state, real logged-in profile reuse, 2FA/MFA, long-lived sessions, credential handling for portals. | 32 | 15 |
| `reliability-correctness` | Reliability and correctness | — | Parent: runs that hang, crash or produce wrong/incomplete results. | — | — |
| `rel-hang-crash` | Hangs, loops and crashes | `reliability-correctness` | Infinite loops, deadlocks, stuck jobs, closed targets, page/browser crashes, hosted-browser outages. | 14 | 11 |
| `rel-incorrect-incomplete` | Duplicate, missing or incomplete data | `reliability-correctness` | Duplicate requests/processing, lost items, partial crawls, nondeterministic output, content missing from rendered pages. | 11 | 9 |
| `silent-failure` | Silent failure | — | Parent: failures that are not surfaced as failures. | — | — |
| `silent-false-success` | Silent empty results or false success | `silent-failure` | Empty strings/datasets returned as success, false success toasts or 'done' claims, block reported as crash or 500, opaque errors, clicks reported ok but ignored, need for verify/assert of expected state. | 17 | 6 |
| `resume-durability` | Resume and durability | — | Parent: surviving interruption. | — | — |
| `durability-resume` | Crash-tolerant resume and checkpointing | `resume-durability` | Resumable runs after crash/interruption, checkpoint per page/step, idempotent steps so retries do not duplicate side effects, state recovery. | 5 | 3 |
| `performance-resources` | Performance and resources | — | Parent: speed and resource use of the runtime. | — | — |
| `perf-memory-concurrency` | Memory, process and concurrency limits | `performance-resources` | Memory leaks, OOM, orphaned Chrome processes, low sessions per instance, autoscaler misjudging memory, avoiding the browser (HTTP-first/JSON endpoints) for efficiency. | 15 | 10 |
| `maintenance-drift` | Maintenance and drift | — | Parent: things breaking over time. | — | — |
| `maint-site-drift` | Target-site change and selector drift | `maintenance-drift` | Redesigns breaking selectors, schema drift, constant scraper maintenance, self-healing/adaptive parsing, keeping data current. | 17 | 4 |
| `maint-upgrade-regression` | Framework upgrade regressions | `maintenance-drift` | Behaviour, performance, token or stealth regressions between tool versions; breaking API changes; dependency pin conflicts. | 12 | 3 |
| `llm-agent-cost-latency` | LLM agent cost and latency | — | Parent: cost and speed of LLM-driven browsing. | — | — |
| `agent-token-latency-cost` | Token bloat and slow per-step loops | `llm-agent-cost-latency` | Oversized snapshots/tool definitions, context exhaustion, token growth, per-step LLM latency and cost of agent runs, harness-driven cost differences. | 44 | 28 |
| `agent-deterministic-replay` | Compile/cache agent runs to deterministic code | `llm-agent-cost-latency` | Export runs as scripts, cache selectors/actions, plan once then execute, AI at dev time only, LLM called only on breakage. | 30 | 19 |
| `llm-agent-reliability` | LLM agent reliability | — | Parent: trustworthiness of agent behaviour. | — | — |
| `agent-unreliable-drift` | Agent nondeterminism and task drift | `llm-agent-reliability` | Agents drifting off-task, wrong clicks, early 'done', inaccuracy with local models, inconsistent success on real tasks. | 22 | 10 |
| `agent-human-in-loop` | Human-in-the-loop and approval gates | `llm-agent-reliability` | Pause/resume, human takeover for login/2FA/captcha, per-action or side-effect approvals (submit, pay). | 16 | 5 |
| `developer-experience` | Developer experience | — | Parent: setup, docs and debugging. | — | — |
| `dx-setup-docs` | Setup, configuration and docs friction | `developer-experience` | Install failures, first-run breakage, brittle config, unclear docs/positioning, support gaps, learning curve. | 18 | 7 |
| `dx-observability` | Observability, logs and replay | `developer-experience` | Tracing, per-step logs, token/cost logging, screenshots/recordings, audit events, health monitoring, actionable diagnostics. | 18 | 7 |
| `self-host-licensing` | Self-host and licensing | — | Parent: running it yourself. | — | — |
| `selfhost-parity-lockin` | Self-host parity and cloud lock-in | `self-host-licensing` | Self-hosted version missing features (anti-bot, live view, cache), OSS as funnel to paid cloud, deployment artifacts (Docker, Helm), licensing (AGPL). | 20 | 9 |
| `compliance-legal` | Compliance and legal | — | Parent: politeness, legal, security and privacy obligations. | — | — |
| `compliance-site-owner-politeness` | Site-owner backlash and polite crawling | `compliance-legal` | robots.txt, throttling defaults, identifiable UA, AI-crawler blocking defaults, pay-per-crawl, vendor blocklists, bot-traffic hostility. | 25 | 17 |
| `compliance-security-privacy` | Security, privacy and regulatory requirements | `compliance-legal` | Credential exposure to LLMs, supply-chain risk, PHI/HIPAA, SOC 2/GDPR/EU AI Act, ToS/account bans, unauthorised access, data provenance and retention. | 35 | 18 |
| `integration-interfaces` | Integration and interfaces | — | Parent: how the tool plugs into other systems. | — | — |
| `integ-mcp-agent-tooling` | MCP, CLI and agent-host integration | `integration-interfaces` | MCP servers, CLI vs MCP, agent skills, Claude Code/Cursor usage, stdio/context-corruption issues. | 37 | 19 |
| `integ-sdk-llm-runtime` | SDK, runtime and bring-your-own LLM/browser | `integration-interfaces` | Language SDKs, edge/serverless runtimes, attaching to existing Playwright/browser, custom or local LLM providers, job APIs. | 21 | 15 |
| `data-quality-extraction` | Data quality and extraction | — | Parent: quality of what is extracted. | — | — |
| `data-extraction-quality` | Extraction accuracy and quality measurement | `data-quality-extraction` | LLM hallucinated fields/URLs, structure fidelity, quality evals/recall, verifiable benchmarks, correct change detection, network/XHR capture. | 19 | 11 |
| `scale-ops` | Scale and operations | — | Parent: running at volume. | — | — |
| `ops-scale-ratelimit-scheduling` | Rate limiting, scheduling and orchestration | `scale-ops` | 429 backoff, distributed rate limiting, autoscaling, scheduled/recurring jobs, Kubernetes/orchestration, large-crawl operations. | 20 | 12 |
| `new-use-case` | Use cases | — | Parent: jobs customers want done. | — | — |
| `usecase-portal-workflow` | B2B portal and form workflows | `new-use-case` | Logged-in supplier/insurance/government/ERP portals, invoice and PDF retrieval, form filling, procurement. | 22 | 15 |
| `usecase-monitoring-leadgen` | Price/change monitoring and lead generation | `new-use-case` | Competitor price and listing monitoring, change alerts, Google Maps/LinkedIn lead lists, GEO/AI-visibility tracking. | 26 | 15 |
| `usecase-qa-research-ingest` | QA testing, research agents and RAG ingestion | `new-use-case` | AI-driven QA/regression tests, research agents with search+scrape, website/knowledge ingestion for RAG. | 24 | 13 |
| `pricing-wtp` | Pricing, willingness to pay and market | — | Parent: what buyers pay, how they buy, and market size. | — | — |
| `wtp-price-points` | Stated budgets and price points | `pricing-wtp` | Explicit budgets, per-unit price expectations, plan prices, willingness or refusal to pay. | 46 | 26 |
| `build-vs-buy-switching` | Build vs buy and switching | `pricing-wtp` | Moves between DIY, OSS, hosted APIs and platforms, and the reasons given. | 31 | 19 |
| `market-adoption-stats` | Market size and adoption statistics | `pricing-wtp` | Survey figures, funding, usage volumes, market forecasts, web-access barrier prevalence. | 35 | 23 |

## 4. Engagement measure and weighting

`engagement` is a raw value specific to each source:

| Source | Measure |
| --- | --- |
| GitHub issues and discussions | Reactions + comments (discussions: votes + comments) |
| HN | Points for a story; number of direct replies for a comment, because the API does not expose comment points |
| Reddit | Number of comments captured in the snapshot (votes not visible) |
| Reviews | Number of reviews on the aggregated page when the item summarizes a page (for example `reviews-reports-8`: 969). An individual review receives a small value or 0 |
| Reports, blog posts, offers | 0 in most cases |

These values are not comparable across sources: 969 Trustpilot reviews and 969 GitHub reactions do not measure the same thing. The weighted score in `pains-ranking.md` therefore remains **ordinal**:

`W(code) = Σ_items (1 + ln(1 + engagement)) × 0.5^(age_in_months / 6)`

Age is counted in days since 2026-10-08, divided by 30.4375. A `YYYY-MM` date is taken as the 1st of the month, which makes it slightly older and therefore less heavily weighted. The logarithm limits the effect of review aggregates without eliminating it. Two sensitivity variants are published alongside: without the `promo` or `vendor_affiliated` items, and with engagement capped at 50.

## 5. Source check and exclusion rules

**Check:** 60 items verified, one by one, on three criteria: accessible URL, date of the statement, faithfulness of the summary.

- 52 items passed the check (`source_checked: true`). Eight of them carry a `check_note`, most often a minor reservation: generalization from a single author (`github-94`), unestablished qualifier (`demand-use-cases-4`, `demand-use-cases-15`), cause assumed by the author (`github-115`), among others.
- **8 failures, i.e. 13.3%, all excluded from the corpus:**

| Excluded item | Reason |
| --- | --- |
| `github-4` | Unfaithful summary: "no fix" is false. The author later identified a race with the new default priority queue and worked around it |
| `reddit-5`, `reddit-10`, `reddit-23`, `reddit-69`, `reddit-71` | Date: the snapshot date was taken as the post date, whereas the post is 4 to about 102 days older |
| `reviews-reports-4`, `reviews-reports-6` | Date: the month of the review cannot be confirmed on the page |

**Exclusion rules applied:**

1. A failed source check leads to exclusion, even if the content is faithful.
2. An item without a reliable date is discarded at collection. The collectors' notes list the items discarded this way: an undated Browserbase failure rate, undated provider rates, etc.
3. A quotation of 15 words or more is removed, without excluding the item. Two quotations were removed (`reddit-20`, `reddit-76`).
4. Promotional or vendor-affiliated content is **kept and flagged**. The ranking is published with and without it. Out of 421 items, 98 are `promo` or `vendor_affiliated` (75 `promo`, 95 `vendor_affiliated`).
5. No username or personal data is kept.

**Identified residual bias:** for Reddit, 5 of the 12 checked items had a snapshot date later than the actual post date. The 68 retained Reddit items carry the snapshot date and may therefore be **older than indicated**, by up to about three months. Their recency weight may be overestimated by a factor of up to about 1.5. Sensitivity test: dividing the weight of all Reddit items by 1.47 yields the same ten pain codes at the top; only ranks 3 to 5 swap (see `pains-ranking.md`, §3).

## 6. Codebook evolution

Any change to a code (addition, merge or new definition) triggers a new version: full recoding by the first coder, a new sample of 120 items for the second, a new check of 60 sources. Rankings cannot compare two versions of the codebook.
