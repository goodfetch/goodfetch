# Needs by segment, new agent uses and buyer requirements

**Status:** v1, desk research. **Date:** 2026-10-08. **Corpus:** 421 items ([corpus.jsonl](corpus.jsonl)).
**Limitations:** the segment is **estimated** by the coder from the declared context; it is not verified firmographic data. The size of each segment in the corpus mostly reflects **who speaks in public** (OSS agent repositories, HN) and not the size of the market. 32 items are `unknown`.

## 1. Distribution

| Segment | Items | Share | Top three codes (n) | `D-segment` option |
| --- | ---: | ---: | --- | --- |
| `agent-developer` | 186 | 44% | `agent-token-latency-cost` (36), `integ-mcp-agent-tooling` (34), `access-antibot-ip` (28) | `agent-developers` |
| `data-team` | 86 | 20% | `access-antibot-ip` (22), `perf-memory-concurrency` (13), `ops-scale-ratelimit-scheduling` (10) | `data-teams` |
| `unknown` | 32 | 8% | `compliance-site-owner-politeness` (11), `market-adoption-stats` (7) | — |
| `b2b-portal-integration` | 29 | 7% | `usecase-portal-workflow` (14), `access-auth-session` (7), `wtp-price-points` (6) | `b2b-portals` |
| `enterprise` | 29 | 7% | `compliance-security-privacy` (15), `access-antibot-ip` (5), `build-vs-buy-switching` (4) | cross-cutting |
| `ecommerce-pricing` | 22 | 5% | `usecase-monitoring-leadgen` (11), `access-antibot-ip` (8), `wtp-price-points` (4) | `ecommerce-pricing` |
| `seo-marketing` | 16 | 4% | `usecase-monitoring-leadgen` (9), `wtp-price-points` (6), `compliance-security-privacy` (3) | — |
| `hobbyist` | 16 | 4% | `wtp-price-points` (4), `access-antibot-ip` (3), `agent-human-in-loop` (3) | — |
| `research-academic` | 5 | 1% | `usecase-qa-research-ingest` (3), `data-extraction-quality` (2) | — |

The `unknown` items are mostly site operators, who are not buyers but **a constraint** (§4).

## 2. Needs by segment

### 2.1 Agent developers (`agent-developer`, 186)

**What they ask for:**

1. **A token-efficient page representation.** Paginated snapshots (`github-107`), less verbose tool outputs (`github-106`), cleaned HTML provided by the platform (`github-105`), stable element handles that survive re-renders (`reddit-14` `[P]`, `reddit-29`).
2. **Compile or replay instead of reasoning on every run.** Export of runs as scripts (`github-15`, `hn-12`), reuse of XPaths (`github-40`), a versionable cache outside the vendor's cloud (`hn-87`).
3. **Use real sessions.** Attach to the real Chrome or to a persistent profile (`reddit-7`, `reddit-9`, `github-113`), with one profile per service and passwords invisible to the agent (`demand-use-cases-68` `[P]`).
4. **Verify the state reached.** A native assertion tool (`github-25`), each action verified by re-reading the page (`reddit-1`), a typed "blocked" result (`github-80`).
5. **Keep control of cost.** Tokens visible in logs (`github-21`), caps per execution and per tool, retry budgets per error class, circuit breakers (`reddit-58`), cost per accepted result tracked at p95 (`reddit-65`).
6. **Plug in the model and runtime of their choice.** Local models (`github-31`), compatibility with the AI SDK (`github-49`), Python SDK (`github-48`), edge runtime (`github-45`).
7. **An interface suited to the host.** Claude Code (65.5%), Copilot (58.7%), Codex (29.5%) and Cursor (25.5%) dominate coding-agent usage (`reviews-reports-54`, Stack Overflow 2026, 12,255 respondents). The choice between MCP and CLI remains open (`hn-50`, `hn-51`, `hn-42`).

**Willingness to pay:** low and polarized. Several prices are deemed excessive (`hn-60`, `hn-58`), the temptation of "Codex + OSS library" is strong (`hn-71`, `demand-use-cases-27`), and only 6.4% of developers who switched AI tools did so for cost, versus 25.1% for quality (`reviews-reports-53`). This is the segment that **talks the most** and that **least justifies** a paid offering.

### 2.2 Data teams (`data-team`, 86)

**What they ask for:**

1. **Access without "full-time" proxying.** Switching to hosted APIs when every difficult site demands rendering, proxy and captcha (`reddit-6`, `reddit-3`). Proxy spending rising for 58.3% of respondents, and purchasing from 2 to 3 providers for 43.1% (`reviews-reports-36`, `reviews-reports-37`, `[P]` Apify).
2. **An engine that does not run away.** Real backoff on 429s (`github-85`, `github-96`), autoscaling that reads memory correctly (`github-84`), no more orphan Chrome processes (`github-70`), fewer memory leaks (`github-69`).
3. **Correctness hygiene.** No duplicates after retry or redirect (`github-7`), no item loss on shutdown (`github-8`), no silent partial crawl (`github-74`), recovery after a crash and not only after a clean stop (`github-5`).
4. **The same product, self-hosted.** Proxy (`github-52`), ready-made images (`github-56`), Helm (`github-58`), parity between server and library (`github-76`).
5. **HTTP first, browser as fallback.** Developers who rewrite this pattern themselves (`github-94`), JS pages that often pull from a JSON endpoint (`hn-22`).

**Willingness to pay:** real budgets go to **proxy and infrastructure**, not to the framework. 62.5% report an increase in infrastructure cost excluding proxies (`reviews-reports-39` `[P]`). Distributed rate limiting would be "a problem people pay for" (`hn-103`). But according to the Apify survey, 46.7% use only in-house code and only 5% rely entirely on external providers (`reviews-reports-38` `[P]`).

### 2.3 B2B portal integration (`b2b-portal-integration`, 29)

**What they ask for:**

1. **Long-lived sessions.** Sessions that last for months, reconnection alerts, human → agent handoff for 2FA (`hn-34`). Cookies that persist across workflows (`github-99`, `github-28`).
2. **Credentials kept out of the LLM** and separate identities per supplier (`demand-use-cases-38`, `demand-use-cases-52` `[P]`).
3. **Usable outputs.** File download and renaming, schema-compliant output to the ERP (`demand-use-cases-10`, `demand-use-cases-52` `[P]`), with recovery.
4. **Approval before side effects.** Human validation before submission or payment (`demand-use-cases-11`, `demand-use-cases-69`).
5. **Sites that change without warning.** Coupa injects DOM elements non-deterministically (`hn-38`); a hand-written script takes 10 to 20 h per site (`hn-81` `[P]`).

**Willingness to pay:** the highest declared prices in the corpus come from this segment, but from **vendor sources**. Simplex claims that five-person companies sign at $2,500/month (`hn-33` `[P]`), a price disputed for lack of pay-per-use pricing (`hn-32`). A user signed up for Browser Use's paid tier at launch (`hn-13`). Intuned cites govtech, insurtech and agencies as customers, some of which run more than 1,000 scrapers (`demand-use-cases-26` `[P]`). On the freelance-gig side, rates range from $3/h (`demand-use-cases-4`) to $20-30/h (`demand-use-cases-11`). **Independent evidence is thin.** Only 8 of the 29 items are promotional or affiliated, but they are the ones carrying the high prices and customer volumes. The other 21 are mostly low-rate freelance gigs and technical issues.

### 2.4 E-commerce price monitoring (`ecommerce-pricing`, 22)

**What they ask for:**

1. **Automatic product matching.** Pasting URLs for thousands of SKUs amounts to checking by hand (`reddit-46`).
2. **Diffs and alert thresholds.** Scraping is "the easy half"; stock and promotions must also be tracked (`reddit-44`). A bug shows what is at stake: a change from $12 to $21 is seen as "unchanged" (`demand-use-cases-40`).
3. **Volume despite anti-bot measures.** 20,000 OEM items per day with an alert at a 2% deviation (`reddit-41`). One team took 6 months to make its proxy, solvers, pipeline and monitoring stack reliable (`reddit-42`). Another targets more than 500M pages per day (`hn-105`).

**Willingness to pay:** highly bimodal.

- **Small sellers:** under $30/month wanted, versus Prisync ($99) and Price2Spy ($83) (`reddit-43`), refusal of $100/month and more (`reddit-47`), a $35 freelance gig (`demand-use-cases-6`).
- **High end:** a vendor case going from a DIY stack at $530k/year to an API at $75k/year (`demand-use-cases-57` `[P]`). Price and competitive monitoring is the fastest-growing use case (18.34% CAGR) according to Mordor (`demand-use-cases-45`).

### 2.5 SEO, marketing and lead generation (`seo-marketing`, 16)

**What they ask for:**

- **Google Maps at scale**, enriched with valid emails and phone numbers, paid per accepted lead or per record (`demand-use-cases-15`, `demand-use-cases-16`, `demand-use-cases-18`).
- **AI visibility tracking (GEO).** Tools at $95-828/month, several of which scrape the interface of real sessions (`demand-use-cases-66`). Google AI Overviews is protected by anti-bot measures, and the cost per query is volatile (`demand-use-cases-67` `[P]`).

**Risk:** LinkedIn automation exposes users to bans and lawsuits (`reddit-77` `[P]`, `reddit-78` `[P]`). For these buyers, account safety is the purchasing criterion.

### 2.6 Research, academic and hobbyists (21)

- **Research:**
  - Legal certainty above all (`reddit-76`).
  - Measuring a crawler's recall (`github-83`) and the quality of tables extracted for a financial RAG (`github-66`).
  - Per-site Scrapy configurations written by an agent for $1-3 of tokens per site, without an LLM per page (`demand-use-cases-33` `[P]`).
- **Hobbyists:** a budget of around $50/month, without wanting to "become a scraping expert" (`reddit-2`). Reluctance about agent bills (2 tasks for $15, `reddit-30`). Manual captcha solving (`reddit-66`).

## 3. New LLM and agent uses

| Use | What is requested | Sources |
| --- | --- | --- |
| **QA agents and regression tests** | Natural-language cases, real run, verdict with traces, then a **deterministic Playwright test exported** for CI; filter out false positives | `demand-use-cases-19`, `demand-use-cases-20`, `demand-use-cases-21`, `demand-use-cases-22`, `github-25`, `github-110`, `hn-0`, `hn-94` |
| **Research agents (search then scraping)** | Semantic search first, a rerank that discards 60 to 80% of URLs, Playwright as a last resort, early stopping; sourced report | `reddit-31`, `reddit-32`, `demand-use-cases-5`, `hn-18` |
| **Site ingestion for RAG** | Multi-tenant knowledge bases kept up to date, measured structural quality | `demand-use-cases-13`, `demand-use-cases-14`, `github-66`, `reddit-59` |
| **Agents in already-connected SaaS** | Stripe, Search Console, internal tools; isolated profiles; 14% of agent requests target account or authentication pages (HUMAN) | `demand-use-cases-68` `[P]`, `demand-use-cases-41`, `reddit-25`, `demand-use-cases-34` `[P]` |
| **Supplier and government portals** | Invoices from hundreds of energy supplier portals, purchasing, insurance eligibility, NFS-e, administrative forms | `demand-use-cases-50` `[P]`, `demand-use-cases-51` `[P]`, `demand-use-cases-12`, `reddit-68`, `demand-use-cases-32` |
| **Human-approved operations** | Clinic: invoices to the ERP with validation before submission or payment, local models (HIPAA); booking where payment marks an authorization boundary | `demand-use-cases-69`, `reddit-67` |
| **GEO and AI visibility tracking** | Brand citations in ChatGPT, Perplexity and AI Overviews | `demand-use-cases-66`, `demand-use-cases-67` `[P]` |
| **Agents as a skill or MCP server** | Scraping packaged as an agent skill; 2,265 MCP Actors in the Apify Store | `github-114`, `reviews-reports-59`, `hn-108` |

A constant across these uses: **the result must be verifiable**. Only 48% of developers trust AI, and only when they can verify easily. 78.7% consider source attribution important or very important (`reviews-reports-51`).

## 4. Buyer requirements and compliance constraints

| Requirement | Who states it | Sources |
| --- | --- | --- |
| SOC 2 Type II, ISO 27001, DPA, list of subprocessors, right to audit | Enterprise buyers of data services | `demand-use-cases-62` `[P]`, `demand-use-cases-64` `[P]`, `hn-25` |
| Zero data retention, retention and deletion policy, security documentation | Cloud browser evaluators | `hn-91`, `hn-31`, `demand-use-cases-62` `[P]` |
| **Per-record provenance** (URL, timestamp, robots status) and the entity performing the collection | Data vendor due diligence, EU AI Act (2026-08-02) | `demand-use-cases-63` `[P]`, `reviews-reports-47` `[P]` |
| Human oversight, logging, per-action audit | Agent deployments in the EU, teams under SOC 2 | `demand-use-cases-39`, `demand-use-cases-50` `[P]`, `reddit-29` |
| Credentials never seen by the LLM, nor stored at the provider; 2FA not automated | Portal integrators, security-conscious customers | `demand-use-cases-38`, `hn-35`, `reddit-50` `[P]` |
| Local or self-hosted execution, license for one's own environment | Healthcare, professional practices, privacy-sensitive teams | `demand-use-cases-10`, `hn-40`, `hn-92`, `hn-44` |
| Written definition of accuracy, anomaly detection, SLA on delivery and not only on availability, PoC on 20 to 50 of the buyer's domains | Price monitoring RFPs | `demand-use-cases-61` `[P]`, `demand-use-cases-64` `[P]` |
| Consented origin of residential IPs, KYC, abuse reporting | Proxy purchasing | `demand-use-cases-65` `[P]` |
| Controlled supply chain | Agent CLI users | `github-33` |
| Lawful collection of public data; unauthorized access excluded | Lead buyers; criminal case in Australia | `demand-use-cases-16`, `reddit-75` |

**Site-side constraint:**

- **Polite defaults.** Respect for robots.txt and throttling (`github-0`, `github-1`, `hn-16`).
- **Crawler identity.** Identifiable user agent (`github-60`).
- **IP reputation.** Shared cloud browser IPs are blocked wholesale (`demand-use-cases-31`, `hn-110`).
- **Cloudflare default settings.** Agents and training bots blocked by default, pay-per-crawl (`reddit-70`, `reddit-72`, `hn-112`).
- **Robots.txt of the large platforms.** LinkedIn, Instagram and TikTok exclude GPTBot and ClaudeBot (`reddit-74` `[P]`).
- **AI directives in robots.txt.** 13% of sites have explicit rules for AI bots, 69% of news sites (`reviews-reports-44` `[P]`).

For the `enterprise` segment, compliance is the top code (15 items out of 29). A buyer who has budget talks about compliance **before** data quality (`demand-use-cases-59`).

## 5. Reading for `D-segment` (provisional, to be settled in interviews)

| Option | Voice volume | Pain aligned with the lab's mechanisms | Evidence of willingness to pay | Main risk |
| --- | --- | --- | --- | --- |
| `agent-developers` | High (186) | Strong: tokens, compiled replay, sessions, false successes | Weak, DIY pressure | Doing OSS without revenue |
| `b2b-portals` | Low (29) | Strong: long sessions, credentials out of the LLM, approval, audit, recovery | Highest displayed, but carried by vendors | Thin independent evidence; long sales cycle |
| `data-teams` | Medium (86) | Medium: engine robustness, rate limiting, recovery | Real proxy and infra budgets, preference for in-house | Head-on competition from access APIs |
| `ecommerce-pricing` | Low (22) | Medium: diffs, product matching, anti-bot volume | Bimodal: SMBs under $30, large accounts in services | Product matching outside the lab's scope |

What desk research does not settle, and what the interviews must settle:

1. Do B2B portals really pay at the level of the prices displayed by vendors?
2. Does an agent developer pay for a **verified result** when they can generate a script themselves?
3. Is enterprise compliance (§4) a prerequisite from the first segment, or can it wait?
