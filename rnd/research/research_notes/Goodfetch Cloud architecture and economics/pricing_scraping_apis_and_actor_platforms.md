# Pricing of scraping, crawling and extraction APIs and of actor platforms (October 2026)

> **Conventions.** USD, taxes and VAT excluded. Monthly billing unless marked "annual". Every official page cited was checked on **2026-10-12** unless another date is given. "Secondary" marks a third-party source. "Unverified" marks a figure I could not confirm on an official page. "Calc" marks my own arithmetic, run in Python (the script stays in the session scratchpad, not in the repo). The browser pane was used to read JavaScript-rendered pricing pages (ScraperAPI, Oxylabs, Decodo, Jina, Crawl4AI). **Search budget:** the shared WebSearch budget ran out midway through this session. After that, I only fetched official pages directly, and user reactions come from the repository's existing corpus (`rnd/research/market/corpus.jsonl`, items dated 2024-11 to 2026-10, not re-read in this session) instead of fresh Hacker News or Reddit searches.

## 1. Apify: plans, prepaid usage, compute units, proxies, storage, monetization rules, commission and limits

### Takeaway
On every Apify plan, the monthly fee is converted 1:1 into prepaid usage: Free $0 (with $5 of usage), Starter $19, Scale $199, Business $999. Compute costs $0.20, $0.20, $0.16 and $0.13 per compute unit (CU, 1 GB-hour). Residential proxy costs $8 → $7/GB, SERP proxy $2.5 → $1.7 per 1,000, and a new "Unblocker" $1.5 → $1 per 1,000 requests. Unused usage expires at the end of each cycle. In the Store, rentals have been fully retired since 2026-10-01, pay-per-result Actors have been migrated to pay-per-event (PPE), and creators keep 80% of PPE revenue minus the platform costs of their paying users. The commission is therefore 20%.

### Cited Findings

#### Plans and unit prices — [apify.com/pricing](https://apify.com/pricing)

| Plan | Monthly | Annual (per month) | Prepaid usage / month | CU price | Residential / GB | Datacenter IPs | SERP / 1,000 | Unblocker / 1,000 req. | Max RAM | Max concurrent runs | Support |
|---|---|---|---|---|---|---|---|---|---|---|---|
| Free | $0 | $0 | $5 | $0.2 | $8 | 5 included | $2.5 | $1.5 | 16 GB | 5 | Community |
| Starter | $19 | $17 | $19 | $0.2 | $8 | 30 included, then $1/IP | $2.5 | $1.5 | 64 GB | 32 | Chat |
| Scale | $199 | $179 | $199 | $0.16 | $7.5 | 200 included, then $0.8/IP | $2 | $1.25 | 256 GB | 128 | Priority chat |
| Business | $999 | $899 | $999 | $0.13 | $7 | 500 included, then $0.6/IP | $1.7 | $1 | 512 GB | 256 | Account manager |
| Enterprise | custom | custom | custom | — | — | — | — | — | — | — | "Dedicated team of experts" |

- Add-ons: extra concurrent run $5/run, Actor RAM $1/GB, personal training $150/hour. "Save 10% on paid plans when billed annually" — [Apify pricing](https://apify.com/pricing). Calc: the annual discount is actually 10.5% on Starter, 10.1% on Scale and 10.0% on Business.
- The [markdown version of the pricing page](https://apify.com/pricing.md) repeats the CU prices ($0.2 / $0.2 / $0.16 / $0.13) and defines 1 CU as "1 GB of RAM running for 1 hour" — [apify.com/pricing.md](https://apify.com/pricing.md).
- Overage: on paid plans, usage beyond the included amount is added to the next invoice "until you reach the usage limit configured in the billing". On Free, access "will be blocked until the beginning of the next monthly cycle". API tokens issued to agents have a "fixed spend cap" — [Apify pricing](https://apify.com/pricing).
- Expiry: "unused usage credits are not rolled over to the next billing cycle, and they expire at the end of the billing cycle" — [Apify pricing](https://apify.com/pricing).

#### Compute unit definition — [Usage and resources docs](https://docs.apify.com/platform/actors/running/usage-and-resources)
- CU = memory × duration; "1024MB × 1 hour = 1 CU". Billing granularity is one second.
- "For every 4096MB of memory, the Actor receives one full CPU core" (512 MB = 1/8 core). Run memory ranges from 128 MB to 32,768 MB in powers of 2. Disk is 2× memory. There is a free CPU boost at start.
- Billed usage categories: compute units, data transfer, proxy and storage operations. This docs page gives no prices for them.
- Calc: 1 vCPU-hour (4 GB for 1 h) = 4 CU = $0.80 (Free/Starter), $0.64 (Scale), $0.52 (Business). The prepaid usage equals 95 CU (Starter), 1,243.8 CU (Scale) and 7,684.6 CU (Business), i.e. 23.8, 310.9 and 1,921.2 vCPU-hours.

#### Limits — [Platform limits docs](https://docs.apify.com/platform/limits)
- Max concurrent runs: Free 25, Starter 32, Scale 128, Business 256. Max total memory of running jobs: 16,384 MB, 65,536 MB, 262,144 MB and 524,288 MB. Max memory per run: Free 16,384 MB, Starter 32,768 MB. Per user: 500 Actors, 5,000 tasks, 100 schedules, 100 webhooks. 4,096 MB of memory per CPU core on all plans.
- **Conflict:** the Free concurrency limit is **5** on the [pricing page](https://apify.com/pricing) and **25** on the [limits page](https://docs.apify.com/platform/limits). PricingSaaS (secondary) records a cut from 25 to 5 on 2026-08-31, so the docs page is probably stale — [PricingSaaS Apify history](https://pricingsaas.com/companies/apify).

#### Storage and data transfer (platform usage cost table) — [PPE "pricing and costs" docs](https://docs.apify.com/platform/actors/publishing/monetize/pricing-and-costs)

| Item | FREE / BRONZE | SILVER | GOLD |
|---|---|---|---|
| Compute unit | $0.2 | $0.16 | $0.13 |
| Residential proxy / GB | $8 | $7.5 | $7 |
| SERP proxy / 1,000 | $2.5 | $2 | $1.7 |
| Data transfer, external / GB | $0.2 | $0.19 | $0.18 |
| Data transfer, internal / GB | $0.05 | $0.045 | $0.04 |
| Dataset reads / 1,000 | $0.0004 | $0.00036 | $0.00032 |
| Dataset writes / 1,000 | $0.005 | $0.0045 | $0.004 |
| Key-value store reads / 1,000 | $0.005 | $0.0045 | $0.004 |
| Key-value store writes / 1,000 | $0.05 | $0.045 | $0.04 |
| Key-value store lists / 1,000 | $0.05 | $0.045 | $0.04 |
| Request queue reads / 1,000 | $0.004 | $0.0036 | $0.0032 |
| Request queue writes / 1,000 | $0.01 | $0.009 | $0.008 |

- The table has tiers FREE, BRONZE, SILVER and GOLD, plus PLATINUM and DIAMOND for enterprise only. Creators can detect a user's tier with `APIFY_USER_IS_PAYING` — [PPE pricing and costs](https://docs.apify.com/platform/actors/publishing/monetize/pricing-and-costs).
- PricingSaaS (secondary) records that a "Silver Apify Store discount tier [was] added to Scale" in 2025 Q2 and that request-queue writes fell from $0.02 to $0.01 (Free/Starter) in April 2026 — [PricingSaaS](https://pricingsaas.com/companies/apify).

#### Monetization models, commission, rental retirement
- The [monetization overview](https://docs.apify.com/platform/actors/publishing/monetize) lists two models: **pay per event (PPE)** and **pay per usage** (the user pays only the platform usage). Rental appears only as a child page. Per-request agentic payments (e.g. Skyfire) are limited to PPE Actors that charge only for events, do not use Standby, and whose developer has completed KYC — [Apify monetize](https://docs.apify.com/platform/actors/publishing/monetize).
- PPE: "profit = (0.8 * revenue) - platform costs". Only paid-plan users count: Apify covers the platform costs of Free-plan users. For PPE-only Actors in Standby, the creator is "not responsible for covering platform usage costs". Worked example from the docs: $31 of revenue and $4 of costs give a profit of $20.80 — [PPE pricing and costs](https://docs.apify.com/platform/actors/publishing/monetize/pricing-and-costs).
- Rental: the developer receives "80% of the total rental fees". "April 1 [2026] - You can no longer publish new rental Actors or change pricing on existing ones. October 1 [2026] - Rental Actors are fully retired. All remaining Actors are migrated to pay-per-usage pricing." — [Rental docs](https://docs.apify.com/platform/actors/publishing/monetize/rental).
- Pay per result: the PPR docs URL returns **HTTP 404** — [PPR docs URL](https://docs.apify.com/platform/actors/publishing/monetize/pay-per-result). Apify's blog (2026-04-14) says it has "already migrated 2,000 Actors from a pay-per-result model to a granular pay-per-event (PPE) system". It states that "All Actors will migrate to a pay-per-event pricing model" and that those not migrated by their creators move automatically to pay-per-usage. Customers get a 14-day "cooling period" to review a migrated price — [Apify blog, Standardizing Actor pricing](https://blog.apify.com/standardizing-actor-pricing).
- Apify's stated rationale: "73% of our customers stated they preferred the pay-per-event model over fixed rentals" (vendor survey). It also cites unpredictable rental-plus-usage costs and AI agents that "pay-as-it-goes" — [Apify blog](https://blog.apify.com/standardizing-actor-pricing).
- A third-party guide claims pay-per-result "remain[s] available" (secondary, seen only through a search summary). This contradicts the 404 and the blog, so I treat it as unverified — [use-apify.com](https://use-apify.com/docs/apify-for-developers/monetize-actors).
- Store size (third-party daily census, via corpus item `reviews-reports-59`): 64,812 Actors from 3,471 publishers, **97.1% priced pay-per-event**. New Actors per month went from 1,836 (December 2025) to 14,400 (September 2026), and 2,265 are MCP-server Actors — [apifystats.com](https://apifystats.com/stats.html).
- Creators who limit features for Free-plan users must say so in the README and input schema and exit gracefully — [Apify monetize](https://docs.apify.com/platform/actors/publishing/monetize).

#### Reference per-page cost of a first-party Actor — [Website Content Crawler](https://apify.com/apify/website-content-crawler)
- Pay per usage. "$0.2 per 1,000 web pages with raw HTTP crawler" and "$0.5 - $5 per 1,000 web pages with a headless browser, depending on the website", with a stated baseline of "$0.25/CU". Optional AI summaries cost "about $2-3 per 1,000 summarized pages"; Free-plan users pay 10× the OpenRouter cost. 167,594 total users; 99.7% of runs succeed — [Website Content Crawler](https://apify.com/apify/website-content-crawler).

### Inferences
- The Apify plan fee is a **minimum monthly commitment returned as usage**, not a feature fee. The plans differ by unit price (CU −35% from Starter to Business), RAM and concurrency caps, and support. Expiry of unused usage is the hidden cost (see Q4).
- Calc: rescaled from the $0.25/CU baseline of the Website Content Crawler page to current CU prices, an HTTP page costs ~$0.16 (Starter) → $0.104 (Business) per 1,000. A browser page costs ~$0.40–$4.00 → $0.26–$2.60 per 1,000, before proxy. Starter's $19 of prepaid usage therefore buys about 118,750 HTTP pages, or 4,750–47,500 browser pages.
- Goodfetch Cloud's actor marketplace could use the market leader's terms as a reference. Those terms are PPE as the default, a 20% take rate, platform costs charged to the creator for paying users, Free-user costs absorbed by the platform, and a migration path away from rentals.

### Gaps
- Timed storage prices (dataset, key-value store and request-queue GB-hours or GB-months): the pricing page's details section is interactive and the docs table has no such rows.
- Payout thresholds and timing, the notice period for PPE price changes, and Enterprise limits (the payouts page was not fetched).
- What the new "Unblocker" line is (only its price row was seen), and whether any pay-per-result Actors remain live.

## 2. Price lists of the crawl and extraction APIs (Firecrawl through Import.io)

### Takeaway
Three billing families dominate. (1) **Flat price per page with internal escalation**: Firecrawl, Olostep, Bright Data, Oxylabs, Decodo, Import.io, Crawl4AI Cloud (with visible effort multipliers) and Scraping Fish. (2) **Option credits** that multiply 1× (HTTP), ~5–10× (JS) and ~25–125× (premium proxy + JS): ScrapingBee, ScraperAPI, ZenRows, Scrapfly, Scrape.do, ScrapingAnt and Crawlbase. (3) **Raw resource metering**: Spider ($/GB + CPU-minutes), Zyte (price per response by site tier), Jina (tokens) and Apify (CU + GB). Almost all of them bill successful requests only and now offer spend caps.

### Cited Findings

#### Firecrawl — [pricing](https://www.firecrawl.dev/pricing), [pricing.md](https://www.firecrawl.dev/pricing.md), [billing docs](https://docs.firecrawl.dev/billing)
- Plans (monthly / annual per month): Free 1,000 credits; **Hobby $19 / $16** ($190/year), 5,000 credits, 5 concurrent; **Standard $99 / $83** ($990/year), 100,000 credits, 25 concurrent; **Growth $399 / $333** ($3,990/year), 500,000 credits, 50 concurrent; **Scale $749 / $599** ($7,190/year), 1,000,000 credits, 100 concurrent; Enterprise custom. The annual discount is 16.7% (Hobby to Growth) and 20% (Scale) — [pricing.md](https://www.firecrawl.dev/pricing.md).
- Rate limits per minute: /scrape, /map and /search 10 / 100 / 500 / 5,000 / 10,000; /crawl and /agent 2 / 20 / 100 / 1,000 / 2,000 — [pricing](https://www.firecrawl.dev/pricing).
- Credits: scrape 1/page; crawl 1/page; map "1 credit per page" on the pricing page vs "1 credit per call" in the docs (**conflict**); search 2 per 10 results plus scrape costs; Monitor 1 per page per check (7 with deterministic extraction); Agent: 5 free runs per day, then "dynamic pricing in credits based on agent tokens" — [pricing](https://www.firecrawl.dev/pricing), [billing docs](https://docs.firecrawl.dev/billing).
- Interact (browser sandbox): **2–7 credits per browser-minute** (7 with a prompt, 2 code-only, +2 with Zero Data Retention), one-minute minimum — [billing docs](https://docs.firecrawl.dev/billing).
- Option surcharges per page: JSON, question, highlights, audio, video and PII redaction each +4; PDF +1 per page; ZDR +1; prompt-injection check +4; **enhanced proxy +0** ("escalated retries are not charged separately") — [billing docs](https://docs.firecrawl.dev/billing).
- Extract has no separate price on the pricing page; it "shares the agent limit" — [pricing.md](https://www.firecrawl.dev/pricing.md).
- Extra credits in $5 increments: Hobby 1,000, Standard 2,000, Growth 2,500 and Scale 5,000 credits per $5. Calc: $5.00 → $1.00 per 1,000 credits — [billing docs](https://docs.firecrawl.dev/billing).

#### Spider (spider.cloud) — [pricing](https://spider.cloud/pricing), [pricing guide, 2026-03-08](https://spider.cloud/guides/pricing-and-plans/), [1M-page case study, 2026-02-11](https://spider.cloud/blog/scraping-one-million-pages-what-actually-happens/)
- Pay-as-you-go: **$1 per GB of pre-transformation content + $0.0001 per CPU-minute**; "Failed requests cost $0". Top-ups from $5; balances never expire. Bonus +5% at $500 and +12% at $2K on the pricing page; the guide lists +5% / +8% / +12% / +15% at $500 / $1,000 / $2,000 / $5,000 — [pricing](https://spider.cloud/pricing), [guide](https://spider.cloud/guides/pricing-and-plans/).
- **Unlimited** plan, billed per concurrency: $350/month for 25 concurrent requests ($14 each). Self-serve range from 2 concurrent ($40) to 1,538 concurrent ($19,994/month). Requests over the limit get a 429 and are not billed; no geotargeting — [pricing](https://spider.cloud/pricing).
- JavaScript rendering and standard proxy rotation carry no multiplier. **Zero data retention is billed 2.5×** (included on Enterprise). Residential, ISP and AI extraction have their own prices, not listed — [pricing](https://spider.cloud/pricing).
- Guide: 10,000 credits = $1. Average costs per request: /crawl ~$0.0003, /scrape ~$0.0001, /screenshot ~$0.0006, /search ~$0.001. Compute is capped at 30 credits ($0.003) per request, or 50 ($0.005) with a browser. Proxied bandwidth costs 1.5× to 4× the $1/GB. Session recording ×1.15. "AI endpoints … billed by your model provider, not by Spider" — [guide](https://spider.cloud/guides/pricing-and-plans/).
- Average cost per 100 pages, by mode: HTTP $0.002–$0.01, Smart $0.0028–$0.0123, Browser $0.0035–$0.0155 — [guide](https://spider.cloud/guides/pricing-and-plans/).
- Case study (vendor-authored): 1,000,000 pages for $546.63 (546,629 credits), 98.98% success. Components: HTTP $204.36, Chrome $191.28, datacenter proxy $38.42, residential escalation $24.12, bandwidth $47.30 (47.3 GB), compute $22.71, retries $18.44. HTTP-only pages ≈ $0.30/1,000; Chrome + residential ≈ $1.20/1,000. Average page 49.2 KB of Markdown (median 31.7 KB) — [case study](https://spider.cloud/blog/scraping-one-million-pages-what-actually-happens/). Calc: the components do add up to $546.63, but that is **$0.547/1,000** requested pages ($0.552 per usable page), not the "$0.48 per 1,000" the article states.

#### Crawl4AI Cloud (launched as a "soft launch") — [Crawl4AI docs](https://docs.crawl4ai.com/), [cloud docs](https://crawl4ai.com/docs), [live price table](https://crawl4ai.com/v1/prices)
- The official docs (v0.9.x) advertise "Crawl4AI Cloud — the same crawler, hosted", pay-as-you-go with "Soft launch: prices can change, what you buy stays yours" — [docs.crawl4ai.com](https://docs.crawl4ai.com/).
- Live table: **1 credit = $0.001**. Scrape, extract, scan, site, jobs and recipes cost 0.50 credit each; search and SERP 1.00; answer 1.00 (2.00 when an LLM writes it); vendor record 3.00. Effort multipliers: Instant 0.5 (archive), Light 1.0, Standard 1.0, **Heavy (browser) 2.0, Hard (protected) 4.0**. Packs: 10,000 credits $10; 27,500 credits $25; 125,000 credits $100. Signup bonus 2,000 credits. Allowances: Community 2,500, Supporter 50,000, Builder 500,000 per month — [crawl4ai.com/v1/prices](https://crawl4ai.com/v1/prices).
- Not billed: refused requests, 5xx, a blocked 403, "no-answer" 502, "fleet-busy" 503 and early cancellations. Credits bought never expire — [cloud docs](https://crawl4ai.com/docs).
- Secondary claims of a "closed beta" are now outdated — [stork.ai](https://www.stork.ai/en/crawl4ai) (secondary).

#### Jina Reader — [jina.ai/reader](https://jina.ai/reader/) (price table read in the browser pane)
- "API pricing is based on the token usage." Reader counts output tokens. Search (s.jina.ai) costs a fixed amount per request, "starting from 10,000 tokens". "Tokens are not deducted for failed requests". jina-ocr-v1 "costs 40× more tokens" — [Jina Reader](https://jina.ai/reader/).
- Packs: free key 10 million tokens (non-commercial, CC-BY-NC); **1 billion tokens for $50 = $0.050/1M** (standard key); **11 billion for $500 = $0.045/1M** (premium key, higher rate limits) — [Jina Reader](https://jina.ai/reader/).
- Reader RPM: 20 without a key; 500 with a free or paid key; 5,000 with a premium key. The FAQ says instead Free 100 RPM / 100K TPM, Paid 500 RPM / 2M TPM, Premium 5,000 RPM / 50M TPM (**conflict on the free tier**) — [Jina Reader](https://jina.ai/reader/).
- "We introduced a new pricing model on May 6th, 2025". Users already on auto-recharge before that date keep the old price — [Jina Reader](https://jina.ai/reader/).

#### Exa — [pricing](https://exa.ai/pricing), [livecrawl docs](https://exa.ai/docs/reference/livecrawling-contents)
- Pay-as-you-go only: "no subscription and no minimum spend". $10/month of free credit that resets — [Exa pricing](https://exa.ai/pricing).
- **Contents: $1 per 1,000 pages per content type** (text and highlights billed separately; summary $1/1,000). Search: instant $4, fast/auto $7 per 1,000 (up to 10 results); deep $12–$15; Answer $5/1,000; Monitors $15/1,000. Agent per request: $0.012 (minimal) to $1.00 (xhigh) — [Exa pricing](https://exa.ai/pricing).
- `livecrawl` is deprecated in favour of `maxAgeHours` (0 = always fresh). The page does not say whether a fresh fetch costs more. Contents are free up to 10 results per search, then $1/1,000 pages — [Exa livecrawl docs](https://exa.ai/docs/reference/livecrawling-contents).

#### Tavily — [credits docs](https://docs.tavily.com/documentation/api-credits), [pricing](https://www.tavily.com/pricing)
- Extract: basic 1 credit per 5 successful URLs; advanced 2 credits per 5. Map: 1 credit per 10 pages (2 with instructions). Crawl = map + extraction (10 pages = 3 credits basic, 5 advanced). Research: 4–250 credits — [Tavily credits](https://docs.tavily.com/documentation/api-credits).
- Plans: Researcher free (1,000/month); **Project $30 (4,000)**; Bootstrap $100 (15,000); Startup $220 (38,000); **Growth $500 (100,000)**; PAYG $0.008/credit; Enterprise custom. Failed extractions and maps are not charged — [Tavily credits](https://docs.tavily.com/documentation/api-credits).
- "Your API credits reset on the first day of each month, regardless of the billing date". The plan price is garbled on the live page — [Tavily pricing](https://www.tavily.com/pricing).

#### Parallel — [pricing](https://parallel.ai/pricing)
- Search: Turbo/Fast $1, Basic/Advanced $5 per 1,000 (10 results). **Extract $1 per 1,000 results.** Task API $5 (Lite) to $2,400 (Ultra8x) per 1,000. Monitor $3–$10 per 1,000. FindAll from $0.25 + $0.03/match. Responses API $10 / $50 / $250 per 1,000. Free: 5,000 requests/month, up to $80 at signup plus $5/month. Billing of failed requests not stated — [Parallel pricing](https://parallel.ai/pricing).

#### Olostep — [pricing](https://www.olostep.com/pricing)
- Trial $0 (500 successful requests); **Starter $9 (5,000, 150 concurrent, $1.80/1,000)**; Standard $99 (200K, 500 concurrent, $0.495/1,000); **Scale $399 (1M, $0.399/1,000)**. "All requests are JS rendered and use residential IP addresses." Top-ups valid 6 months: 10k $20, 250k $200, 2M $1,000. Plan changes are pro-rated — [Olostep pricing](https://www.olostep.com/pricing).
- **Unverified:** `llm_extract` costs 20 credits per request and failures are not charged, per an Italian-language Olostep page seen only through a search excerpt — [olostep.com/it/url-to-json](https://www.olostep.com/it/url-to-json).

#### ScrapeGraphAI — [pricing](https://scrapegraphai.com/pricing)
- Free 500 credits one-time; **Starter $20 (10,000/month)**; Growth $100 (100,000); **Pro $500 (750,000)**; Enterprise custom.
- Credits: Scrape 1 (Markdown), 2 (screenshot), 25 (branding); Extract 5; Search 2 per result (5 with a prompt); Crawl 2 + per-page cost; PDF 1/page. **Stealth +5**; render mode (auto/fast/js) +0. Never-expiring top-ups: $5 / 1,000, $40 / 10,000, $150 / 50,000. Rollover and failed-request billing not stated — [ScrapeGraphAI pricing](https://scrapegraphai.com/pricing).

#### Diffbot — [pricing](https://www.diffbot.com/pricing/)
- Free $0 (10,000 credits, 5 req/min); **Startup $299 (250,000; overage $0.001/credit)**; **Plus $899 (1,000,000; $0.0009/credit; crawl 25 active)**; Enterprise custom.
- Extract 1 page = 1 credit (2 via datacenter proxy); crawl spidering 0; Knowledge Graph 25 per entity export. On Free, exceeding the quota returns `429` — [Diffbot pricing](https://www.diffbot.com/pricing/).

#### Zyte API and Scrapy Cloud — [pricing](https://www.zyte.com/pricing/), [pricing docs](https://docs.zyte.com/zyte-api/pricing.html), [Scrapy Cloud](https://www.zyte.com/scrapy-cloud/)

| Per 1,000 successful responses | HTTP T1 / T2 / T3 / T4 / T5 | Browser T1 / T2 / T3 / T4 / T5 |
|---|---|---|
| Pay-as-you-go | $0.13 / $0.23 / $0.44 / $0.70 / $1.27 | $1.01 / $2.01 / $4.02 / $8.04 / $16.08 |
| $100/month commitment | $0.10 / $0.17 / $0.33 / $0.53 / $0.95 | $0.75 / $1.50 / $3.00 / $6.00 / $12.00 |
| $200/month | $0.08 / $0.14 / $0.26 / $0.42 / $0.76 | $0.60 / $1.20 / $2.40 / $4.80 / $9.60 |
| $500/month | $0.06 / $0.11 / $0.21 / $0.34 / $0.61 | $0.48 / $0.96 / $1.92 / $3.84 / $7.68 |

- Source of the table: [zyte.com/pricing](https://www.zyte.com/pricing/). The docs list commitments of $100 (25% off), $200 (40%), $350 (48%) and $500 (52%), with spending limits of $200 / $400 / $700 / $2,500 respectively. PAYG has a $100 spending limit. Standard plans include $5 of free credit, Enterprise $200 — [Zyte docs](https://docs.zyte.com/zyte-api/pricing.html). Calc: the page's columns match the 25 / 40 / 52% discounts to within rounding.
- "You are only charged for successful responses". Extras: screenshots $1.34/1,000; automatic extraction $0.27–$1.07/1,000 per data type (SERP free). **Custom attributes (LLM):** text $1.34/1M input tokens + $6.67/1M output tokens; HTML $4.02/1M + $20.0/1M. Actions are billed by CPU and network. Tiers are assigned automatically, reviewed quarterly, with 2 weeks' notice — [Zyte docs](https://docs.zyte.com/zyte-api/pricing.html).
- Scrapy Cloud: Starter free (1 hour of crawl time, 1 concurrent crawl, 7-day retention); Professional "from $9 / unit per month"; "1 Scrapy Unit = 1 GB of RAM and 1 concurrent crawl"; 120-day retention — [Scrapy Cloud](https://www.zyte.com/scrapy-cloud/).

#### ScrapingBee — [pricing](https://www.scrapingbee.com/pricing/), [HTML API contract](https://www.scrapingbee.com/llms/html-api.txt)
- Freelance **$49** (250,000 credits, 50 concurrent); Startup $99 (1M, 100); Business $249 (3M, 200); Business+ $599 (8M, 400); Enterprise 14 $999 (14M, 500); Enterprise 24 $1,599 (24M); Enterprise 41 $2,399 (41M); **Enterprise 69 $3,599 (69M, 800)**. 1,000 free credits, no card. VAT excluded. Annual prices not shown — [ScrapingBee pricing](https://www.scrapingbee.com/pricing/).
- Credits: rotating proxy without JS 1; with JS 5 (JS on by default); premium without JS 10; premium with JS 25; **stealth with JS 75** (stealth without JS "coming soon"); `ai_query`/`ai_extract_rules` +5 — [contract](https://www.scrapingbee.com/llms/html-api.txt).
- Billed: 200, 404, 410. Not billed: 400, 401, 413, 429, 500. With `transparent_status_code=true`, every request is billed. `mode=auto` escalates 1 → 5 → 10 → 25 → 75, charges only the configuration that succeeds (0 if everything fails), and accepts a `max_cost` cap — [contract](https://www.scrapingbee.com/llms/html-api.txt).

#### ScraperAPI — [pricing (rendered in the browser pane)](https://www.scraperapi.com/pricing/), [credits docs](https://docs.scraperapi.com/credits-and-requests)
- **Hobby $49** (100,000 credits, 20 threads, US/EU geotargeting only); Startup $149 (1M, 50); Business $299 (3M, 100, global); Scaling $475 (5M, 200); Professional $975 (10.5M, 300; "+250K bonus" limited offer); **Advanced $1,975** (21.5M, 500; +500K bonus); Enterprise custom (22M+). "Yearly: 10% off". 7-day trial with 5,000 credits; free plan 1,000 credits / 5 concurrent. Banner: "ScraperAPI has acquired Traject Data" — [ScraperAPI pricing](https://www.scraperapi.com/pricing/).
- Credits: normal 1; ecommerce (Amazon, Walmart, eBay) 5; SERP (Google, Bing) 25; LinkedIn 30; `render=true` 10; `premium=true` 10; screenshot 10; `ultra_premium` 30; **premium + render 25**; **ultra_premium + render 75**. Bypasses for protected domains (Cloudflare, Turnstile, DataDome, PerimeterX, Akamai) are listed at "10 per scrape" — [ScraperAPI docs](https://docs.scraperapi.com/credits-and-requests). Interpretation: the docs list `render=true` as "10" in a column labelled "extra". I treat a rendered request as 10 credits in total, consistent with premium + render = 25.
- Only 200 and 404 are billed, plus requests cancelled after 70 seconds. Accounts on custom discounted pricing pay more credits on Ultra Premium domains, to keep a floor of "$3 per thousand requests without rendering and $7 per thousand with rendering" — [ScraperAPI docs](https://docs.scraperapi.com/credits-and-requests).

#### ZenRows — [pricing](https://www.zenrows.com/pricing)
- Free 5,000 credits. **Build** 45k $19 / 80k $29 / 120k $39 (annual per month $16 / $24 / $32; 20 concurrent). Launch 250k $69 / 500k $99 / 700k $129 (50 concurrent). Growth 1.2M $199 / 2M $279 / 3M $399 (100 concurrent). Scale 5M $549 / 8M $749 / 12.5M $999 / 28M $1,999 / **45M $2,999** (annual $2,499; 200–400 concurrent). Enterprise custom — [ZenRows pricing](https://www.zenrows.com/pricing).
- Credits per **successful** request: standard 1, JS 5, premium proxy 10, JS + premium 25. Residential bandwidth 25,000 credits/GB; Browser = bandwidth + 5 credits/minute. Extract "adds no cost beyond the Fetch request"; Batch at the same rate. 404/410 count as successful. Top-ups up to 10× per period roll over once. Annual billing: "pay for 10, get 12" — [ZenRows pricing](https://www.zenrows.com/pricing). Calc: residential costs $10.56/GB at Build and $1.67/GB at Scale 45M; Browser $0.0021/min (Build) and $0.00033/min (Scale) before bandwidth.

#### Scrapfly — [pricing](https://scrapfly.io/pricing), [Scrape API billing](https://scrapfly.io/docs/scrape-api/billing), [Extraction API billing](https://scrapfly.io/docs/extraction-api/billing)
- Free 1,000; **Discovery $30** (200,000, 5 concurrent, $5/10k extra, hard cap at quota); Pro $100 (1M, 20, $3.50/10k); Startup $250 (2.5M, 50, $2.00/10k); **Enterprise $500** (5.5M, 100, $1.20/10k); Custom $1,200–$30,000+. "2 months free (16% off) when paid annually" — [Scrapfly pricing](https://scrapfly.io/pricing).
- Credits: datacenter 1; residential 25; `render_js` +5 (DC + JS = 6, residential + JS = 30). ASP "may dynamically upgrade the proxy pool … which can affect the final cost". Bandwidth: text beyond 1 MB and binary beyond the first free MB at 3 (DC) or 10 (residential) credits per 100 KB; browser gets 3 MB free. Cache hit 1 credit. `cost_budget` per request. Pay-as-you-go capped at 50% of the monthly quota — [Scrapfly billing](https://scrapfly.io/docs/scrape-api/billing).
- Extraction API: template 1, **prompt (LLM) 5**, model 5, plus the base price for each extra 500 KB — [Extraction billing](https://scrapfly.io/docs/extraction-api/billing).

#### Crawlbase — [pricing](https://crawlbase.com/pricing)
- Graduated pay-as-you-go per 1,000 successful requests: first 1K $3.00; next 10K $2.00; next 100K $0.60; next 1M $0.50; next 10M $0.10; next 100M $0.05; next 1B $0.04; beyond $0.02. Vendor examples: 100K = $76.40, 1M = $527.50, 10M = $1,471.90 (Calc: reproduced exactly) — [Crawlbase pricing](https://crawlbase.com/pricing).
- Multipliers: site difficulty Standard ×1, Moderate ×1.5, Complex ×2, Complex I ×3, Complex II ×4.5, Extreme ×9, Extreme I ×13.5, Extreme II ×20, applied automatically; **JavaScript ×2**. Only successful requests are billed. Storage 0.5 credit/page/month; screenshots 2 credits — [Crawlbase pricing](https://crawlbase.com/pricing).
- Subscriptions (monthly / annual): **Starter 200K $99** / $1,089; Scale 500K $199; Scale 1M $349; **Growth 2M $599** / $6,589. Beyond the quota, billing continues pay-as-you-go — [Crawlbase pricing](https://crawlbase.com/pricing).

#### ScrapingAnt — [homepage pricing](https://scrapingant.com/), [credits docs](https://docs.scrapingant.com/credits-cost)
- **Enthusiast $19** (100,000); Startup $49 (500,000); Business $249 (3M); **Business Pro $599** (8M); Custom $699+ (10M+). 10,000 free credits per month. "No concurrency cap on any plan". "Pay only for successful scrapes — failed requests cost 0" — [ScrapingAnt](https://scrapingant.com/).
- Credits: no browser + datacenter 1; browser without JS 2; **browser JS + datacenter 10** (the default); no browser + residential 25; browser without JS + residential 50; **browser JS + residential 125**; Google + datacenter 10. **AI extractor: 1 credit per 30 characters of Markdown and output**, plus the scrape. Unused credits are not carried over — [ScrapingAnt docs](https://docs.scrapingant.com/credits-cost).
- **Conflict:** the homepage says "Proxy usage is billed separately by GB, not in credits", while the docs bill residential in credits — [ScrapingAnt](https://scrapingant.com/) vs [docs](https://docs.scrapingant.com/credits-cost).

#### Scrape.do — [pricing](https://scrape.do/pricing/), [documentation](https://scrape.do/documentation/), [request costs](https://scrape.do/documentation/request-costs/)
- Free 1,000 (5 concurrent); **Hobby $29** (250,000, 10); Pro $99 (1.25M, 50); Business $249 (3.5M, 100); **Advanced $699** (10M, 200). Annual = 10× monthly ($290 / $990 / $2,490 / $6,990). The vendor lists $0.11 / $0.08 / $0.07 / $0.06 per 1,000. Only successful responses "with valid content" are charged. At the limit, requests stop — [Scrape.do pricing](https://scrape.do/pricing/).
- Costs: datacenter 1; datacenter + render 5; super (residential/mobile) 10; super + render 25. Some domains are forced to a higher cost (e.g. jd.com 75). Billable: 2xx, 400, 404, 410 — [request costs](https://scrape.do/documentation/request-costs/). The 5 and 10 values were seen through a search excerpt of this page; the main documentation page directly confirms 1 credit (datacenter) and 25 (residential + JS) — [documentation](https://scrape.do/documentation/).

#### Bright Data — [Web Unlocker](https://brightdata.com/pricing/web-unlocker), [SERP API](https://brightdata.com/pricing/serp), [Web Scraper API](https://brightdata.com/pricing/web-scraper), [Crawl API](https://brightdata.com/products/crawl-api), [Datasets](https://brightdata.com/pricing/datasets), [Browser API](https://brightdata.com/pricing/scraping-browser)
- **Web Unlocker:** PAYG **$1.5/1,000** ("Pay only for success", monthly spend limits, unlimited concurrency, browser rendering included); Scale $499/month for 383K requests, then $1.3/1,000; free tier 5K/month — [Web Unlocker](https://brightdata.com/pricing/web-unlocker).
- **SERP API:** PAYG $1.5/1,000; Scale $499 for 380K; in asynchronous mode, only the "send request" is counted — [SERP](https://brightdata.com/pricing/serp).
- **Web Scraper API** (pre-built per-site scrapers, structured records): PAYG $1.5/1,000 records; Scale $499 for 384,000 records, then $1.3; "no charges for failed deliveries" — [Web Scraper API](https://brightdata.com/pricing/web-scraper).
- **Crawl API** (Markdown, HTML, JSON): PAYG $1.5/1,000 records. Plans $499 (510K), $999 (1M) and $1,999 (2.5M) at $0.98 / $0.83 / $0.75 with the coupon "APIS25" (list prices $1.3 / $1.1 / $1). Billing of failures not stated — [Crawl API](https://brightdata.com/products/crawl-api). Calc: at list price, $499 buys 383,846 records, consistent with the Web Scraper API.
- **Datasets:** "starting at $2.5/1K records" ($250 for 100K, one-time). Refresh subscriptions save 25% (biannual), 50% (quarterly), 80% (monthly) — [Datasets](https://brightdata.com/pricing/datasets).
- **Browser API:** PAYG $8/GB; Scale $499 for 71 GB (Calc: $7.03/GB); first deposit matched up to $500 — [Browser API](https://brightdata.com/pricing/scraping-browser).

#### Oxylabs — [Web Scraper API pricing](https://oxylabs.io/products/scraper-api/web/pricing) (redirects to [/products/web-api/scrape/pricing](https://oxylabs.io/products/web-api/scrape/pricing))
- "Single subscription. All product access": welcome credit $5; **Starter $30/month**; Basic $100; **Flexible $500**; Enterprise "starting at $2,500/month" — [Oxylabs](https://oxylabs.io/products/web-api/scrape/pricing).
- Per 1,000 results across five volume tiers (≤10K / ≤60K / ≤222K / ≤1.43M / 10M+): **/Scrape (Web API) $0.50 / $0.50 / $0.45 / $0.35 / $0.25**; custom domain without JS $1.00 / $1.00 / $0.90 / $0.80 / $0.75; **custom domain with JS $1.50 / $1.50 / $1.40 / $1.30 / $1.22**; Google with JS $1.35 → $0.90; Google without JS $1.00 → $0.50; LLM scrapers (ChatGPT, Perplexity, Gemini) $1.50 → $0.90–$1.30; **AI parsing $3.00 per 1,000 parsed pages**; media $2.00/GB — [Oxylabs pricing](https://oxylabs.io/products/scraper-api/web/pricing). Calc: the tiers match the plans ($30 / $0.50 = 60K; $100 / $0.45 = 222K; $500 / $0.35 = 1.43M; $2,500 / $0.25 = 10M).
- Billing: per an excerpt of the help-center article, results with 2xx **and 4xx** codes are billable and 5xx/6xx are not. That URL now redirects to the help-center index, so this is not directly verified — [Oxylabs help center](https://developers.oxylabs.io/help-center/billing-and-payments/how-does-web-scraper-api-pricing-work). The page's meta description still cites "$49/month" (stale).

#### Decodo (formerly Smartproxy) — [Web Scraping API pricing](https://decodo.com/scraping/web/pricing)
- "Regular" plans: $0 (up to 5K crawls or 10K pages scraped, 10 req/s); **$19** (19K crawls or 38K pages); $49 (65K / 122K, 25 req/s); $99 (141K / 330K, 50 req/s); **$249** (383K / 1.2M, 100 req/s); Enterprise via sales. JS rendering, Markdown and /scrape, /search, /map, /crawl included; 14-day money-back — [Decodo](https://decodo.com/scraping/web/pricing). Calc: $0.50 → $0.207 per 1,000 pages; $1.00 → $0.65 per 1,000 crawls.
- The page says the API "lets you enable JS rendering and premium proxies only when needed" and points to "full pricing details" in the documentation, which I could not open — [Decodo](https://decodo.com/scraping/web/pricing). **Unverified:** a search excerpt cites standard $0.50, standard + JS $0.75, premium $1.00, premium + JS $1.50 per 1,000 on the $19 plan. The fetched "variant" page does not show these figures — [pricing_variant_aa](https://decodo.com/scraping/web/pricing_variant_aa).

#### No-code tools — [Browse AI](https://www.browse.ai/pricing), [Octoparse](https://www.octoparse.com/pricing), [Import.io](https://www.import.io/pricing)
- Browse AI: Free 50 credits/month; Personal $19/month annual (12,000 credits/year); Professional $69/month annual (60,000 to 360,000/year); Premium from $500/month annual. "With each credit, you can extract 10 rows of data from a page or capture a screenshot". Premium sites cost a minimum of 2 to 10 credits per task. Annual overage $0.024 (Personal) and $0.017–$0.013 (Professional) per credit. Credits do not roll over. The page **contradicts itself** on monthly prices ($19 vs $48 Personal, $69 vs $87 Professional) — [Browse AI](https://www.browse.ai/pricing).
- Octoparse: Standard $69/month and Professional $249/month (annual; 16% saving vs monthly). Residential proxies $3/GB; CAPTCHA $1–1.5 per 1,000; pay-per-result templates $0.001–3 per 1,000 results; no per-page charge — [Octoparse](https://www.octoparse.com/pricing).
- Import.io (page dated 16 September 2026): Standard $199 annual / $249 monthly (50,000 successful queries); Professional $399 / $499 (200,000); Advanced $699 / $899 (500,000). Overage $7 / $3.50 / $2.50 per 1,000. 1 page = 1 query, unlimited rows. Web Scraper MCP at $0.0002 per successful call with a monthly spend limit — [Import.io](https://www.import.io/pricing).

#### Refresh of the hosted-browser rows of §7 — [Browserbase](https://www.browserbase.com/pricing), [Browser Use](https://browser-use.com/pricing), [Browserless](https://www.browserless.io/pricing), [Scraping Fish](https://scrapingfish.com/)
- Browserbase (unchanged since §7): Developer $20 (100 h, 25 concurrent, 1 GB; $0.12/h, $12/GB), Startup $99 (500 h, 100 concurrent, 5 GB; $0.10/h, $10/GB). New lines: Fetch $1/1,000 calls (Developer; $0.5 Startup; $4 with proxies; $4–$7 with Extract), Search $7/1,000, 15–50 agent runs included, captcha solving from Developer — [Browserbase](https://www.browserbase.com/pricing).
- Browser Use: $0.02/browser-hour (minute rounding, 1-minute minimum), residential $5/GB, BYOP $0.20/GB. Agents: "Model cost + 20%". "No subscription. Credits never expire". Concurrency depends on lifetime spend (10 sessions at $0 → 1,000 at $25,000) — [Browser Use](https://browser-use.com/pricing).
- Browserless (annual): Prototyping $25 (20k units), Starter $140 (180k), Scale $350 (500k). Unit = up to 30 s per connection; a reconnection = a new unit; residential 6 units/MB, datacenter 2 units/MB, captcha 10 units — [Browserless](https://www.browserless.io/pricing).
- Scraping Fish: "$0.002 Per Request", JS and proxies included, only 200/404 billed, 25 concurrent requests per domain — [Scraping Fish](https://scrapingfish.com/).

### Inferences
- The "option credits" vendors converge on the same multiplier grid (1 / 5 / 10 / 25, plus 75 for stealth or ultra) and differ mostly in the dollar price of a credit. Flat-rate vendors bundle escalation into the per-page price (Firecrawl now charges +0 for enhanced proxy; Olostep and Bright Data charge one price for every class).
- AI-native players price the page (Firecrawl, Olostep, Crawl4AI) or the token (Jina, Zyte custom attributes, Crawl4AI extract), while unblocking specialists price the anti-bot route. Goodfetch's meters (requests, bytes, browser-seconds, tokens per model, solver attempts, proxy bytes) correspond to Spider's and Apify's raw model, but the market sells mostly a per-page price.

### Gaps
- Decodo's per-class rates; Oxylabs's exact billing rule; Spider's residential, ISP and AI extraction rates; ScrapingBee annual prices; the LLM nature of ZenRows's Extract; Crawl4AI's LLM extraction unit (millicredits per block, ambiguous unit); Firecrawl's agent and extract rates (dynamic).
- Whether ScrapingBee's "Enterprise" tiers (up to $3,599) can be bought self-serve.

## 3. Normalization to $ per 1,000 pages (static, JS, anti-bot) and per 1,000 LLM-extracted records, from the entry paid plan to the largest self-serve plan

### Takeaway
At the largest self-serve plan, a static page costs **$0.05–$0.10/1,000** with option-credit vendors (ScrapingBee, ZenRows, Scrape.do, ScrapingAnt, Scrapfly, ScraperAPI) and Zyte, versus **$0.21–$1.30** with flat-rate vendors (Decodo, Jina, Oxylabs, Olostep, Crawl4AI, Firecrawl, Bright Data). A JS page costs **$0.26–$0.96** with option credits and Zyte, and **$0.23–$1.30** with flat-rate vendors (Jina $0.23, which uses a headless engine by default, up to Oxylabs and Bright Data at $1.30). An anti-bot page costs **$1.20–$2.73** with option credits and unblockers. That is more than at the flat-rate vendors that bundle escalation (Olostep $0.40, Firecrawl $0.60–$0.75), and it reaches $3.84–$7.68 with Zyte's hardest tiers and $6.89 with ScraperAPI ultra_premium. An LLM-extracted record costs **$0.52–$4.30** (ScrapingBee AI to Oxylabs AI parsing). Entry plans cost 1.15× to 6.3× more per unit than the largest plan.

### Cited Findings

#### Method and assumptions (Calc)
- One page = one billed successful request, no billed retries (most vendors bill success only, see Q4). (a) static = cheapest mode without JS, standard/datacenter proxy; (b) JS = rendering with standard proxy; (c) anti-bot = the vendor's premium/residential tier + JS (or stealth/unblocker); (d) LLM-extracted record = one record per page with the vendor's LLM or AI option. Token-metered products assume **5,000 input tokens (~20,000 Markdown characters) and 200 output tokens (~800 characters) per page**. Separate bandwidth fees are excluded (Scrapfly beyond 1 MB, ZenRows browser bandwidth, Apify proxy).
- "Entry" = cheapest paid plan (or PAYG when there is no plan); "largest" = the most expensive plan with a published price, where "Custom/Enterprise contact sales" is excluded. Monthly billing unless noted. Credit prices and multipliers come from the sources cited in Q2.

#### A. Option-credit and flat-rate per-page vendors, $ per 1,000 pages (entry → largest)

| Vendor (basis) | $ per 1,000 credits | (a) static | (b) JS | (c) anti-bot | (d) LLM record | Source |
|---|---|---|---|---|---|---|
| Firecrawl, monthly (Hobby $19/5k → Scale $749/1M) | $3.80 → $0.749 | $3.80 → $0.749 | $3.80 → $0.749 | $3.80 → $0.749 (enhanced proxy +0) | $19.00 → $3.75 (JSON +4) | [pricing.md](https://www.firecrawl.dev/pricing.md), [docs](https://docs.firecrawl.dev/billing) |
| Firecrawl, annual ($190/60k → $7,190/12M) | $3.17 → $0.599 | $3.17 → $0.599 | $3.17 → $0.599 | $3.17 → $0.599 | $15.83 → $3.00 | same |
| ScrapingBee (Freelance $49/250k → Enterprise 69 $3,599/69M) | $0.196 → $0.052 | $0.196 → $0.052 | $0.980 → $0.261 | $4.90 → $1.30 (premium + JS); stealth $14.70 → $3.91 | $1.96 → $0.522 (JS + AI) | [pricing](https://www.scrapingbee.com/pricing/), [contract](https://www.scrapingbee.com/llms/html-api.txt) |
| ScrapingBee, Business+ $599/8M as the top | → $0.075 | → $0.075 | → $0.374 | → $1.87 | → $0.749 | same |
| ScraperAPI (Hobby $49/100k → Advanced $1,975/21.5M) | $0.490 → $0.092 | $0.490 → $0.092 | $4.90 → $0.919 | $12.25 → $2.30 (premium + render); ultra $36.75 → $6.89 | n/a | [pricing](https://www.scraperapi.com/pricing/), [docs](https://docs.scraperapi.com/credits-and-requests) |
| ZenRows, monthly (Build $19/45k → Scale $2,999/45M) | $0.422 → $0.067 | $0.422 → $0.067 | $2.11 → $0.333 | $10.56 → $1.67 | n/a (Extract +0, nature unverified) | [pricing](https://www.zenrows.com/pricing) |
| ZenRows, annual ($16 → $2,499) | $0.356 → $0.056 | $0.356 → $0.056 | $1.78 → $0.278 | $8.89 → $1.39 | n/a | same |
| Scrapfly (Discovery $30/200k → Enterprise $500/5.5M) | $0.150 → $0.091 | $0.150 → $0.091 | $0.900 → $0.545 | $4.50 → $2.73 (residential + JS, before ASP upgrades) | $1.65 → $1.00 (JS + prompt) | [pricing](https://scrapfly.io/pricing), [billing](https://scrapfly.io/docs/scrape-api/billing), [extraction](https://scrapfly.io/docs/extraction-api/billing) |
| Scrape.do (Hobby $29/250k → Advanced $699/10M) | $0.116 → $0.070 | $0.116 → $0.070 | $0.580 → $0.350 | $2.90 → $1.75 (super + render) | n/a | [pricing](https://scrape.do/pricing/), [request costs](https://scrape.do/documentation/request-costs/) |
| ScrapingAnt (Enthusiast $19/100k → Business Pro $599/8M) | $0.190 → $0.075 | $0.190 → $0.075 | $1.90 → $0.749 | $23.75 → $9.36 (JS + residential, 125 cr) | $133.63 → $52.66 (1 cr/30 characters) | [ScrapingAnt](https://scrapingant.com/), [docs](https://docs.scrapingant.com/credits-cost) |
| Crawlbase subscription (Starter $99/200k → Growth $599/2M) | $0.495 → $0.299 | $0.495 → $0.299 | $0.990 → $0.599 | $1.98 → $1.20 (Complex ×2 + JS) up to $19.80 → $11.98 (Extreme II + JS) | n/a | [Crawlbase](https://crawlbase.com/pricing) |
| ScrapeGraphAI (Starter $20/10k → Pro $500/750k) | $2.00 → $0.667 | $2.00 → $0.667 | $2.00 → $0.667 | $12.00 → $4.00 (stealth +5) | $10.00 → $3.33 (Extract 5; $20.00 → $6.67 with stealth) | [ScrapeGraphAI](https://scrapegraphai.com/pricing) |
| Diffbot (Startup $299/250k → Plus $899/1M) | $1.20 → $0.899 | $1.20 → $0.899 | $1.20 → $0.899 | $2.39 → $1.80 (via proxy) | $1.20 → $0.899 (structured ML record, not LLM) | [Diffbot](https://www.diffbot.com/pricing/) |
| Olostep (Starter $9/5k → Scale $399/1M) | $1.80 → $0.399 | $1.80 → $0.399 | $1.80 → $0.399 | $1.80 → $0.399 (JS + residential included) | $36.00 → $7.98 (if `llm_extract` = 20 cr, **unverified**) | [Olostep](https://www.olostep.com/pricing) |
| Crawl4AI Cloud, soft launch ($10/10k pack → $100/125k) | $1.00 → $0.800 | $0.500 → $0.400 | $1.00 → $0.800 | $2.00 → $1.60 | not computable (ambiguous token unit) | [v1/prices](https://crawl4ai.com/v1/prices) |
| Browse AI, annual (Personal $228/12k → Professional $828/60k) | $19.00 → $13.80 | $19.00 → $13.80 | $19.00 → $13.80 | $38–$190 → $27.60–$138 (2–10 cr) | $19.00 → $13.80 (1 cr for ≤10 rows) | [Browse AI](https://www.browse.ai/pricing) |

#### B. Vendors priced per request, per result or by tier, $ per 1,000 pages

| Vendor | (a) static | (b) JS | (c) anti-bot | (d) structured / LLM record | Source |
|---|---|---|---|---|---|
| Bright Data Web Unlocker (PAYG → Scale $499/383K) | $1.50 → $1.30 | $1.50 → $1.30 | $1.50 → $1.30 | Web Scraper API (pre-built scrapers): $1.50 → $1.30 per 1,000 records; Datasets $2.50 | [Unlocker](https://brightdata.com/pricing/web-unlocker), [Scraper API](https://brightdata.com/pricing/web-scraper), [Datasets](https://brightdata.com/pricing/datasets) |
| Bright Data Crawl API (PAYG → $1,999/2.5M) | $1.50 → $0.80 (coupon) / $1.00 (list) | same | same | — | [Crawl API](https://brightdata.com/products/crawl-api) |
| Oxylabs (Starter $30 → Flexible $500) | $0.50 → $0.35 (/Scrape); custom domain without JS $1.00 → $0.80 | $1.50 → $1.30 (custom domain JS) | $1.50 → $1.30 (unblocking included; my interpretation) | AI parsing +$3.00: $3.50–$4.50 → $3.35–$4.30 | [Oxylabs](https://oxylabs.io/products/scraper-api/web/pricing) |
| Decodo ($19 → $249) | $0.50 → $0.207 ("pages scraped") | unverified ($0.75 at $19 per an excerpt) | unverified ($1.50 at $19 per an excerpt) | — | [Decodo](https://decodo.com/scraping/web/pricing) |
| Zyte API (PAYG → $500 commitment) | $0.13–$0.23 → $0.06–$0.11 (HTTP T1–T2) | $1.01–$2.01 → $0.48–$0.96 (browser T1–T2) | $8.04–$16.08 → $3.84–$7.68 (browser T4–T5); HTTP T4–T5 $0.70–$1.27 → $0.34–$0.61 | custom attributes ≈ $8.03 → $3.86 + response (Calc; discount on extras assumed); automatic extraction +$0.27–$1.07 | [Zyte](https://www.zyte.com/pricing/), [docs](https://docs.zyte.com/zyte-api/pricing.html) |
| Tavily Extract (Project $30 → Growth $500; PAYG) | $1.50 → $1.00 (basic); PAYG $1.60 | $3.00 → $2.00 (advanced); PAYG $3.20 | same as (b), no anti-bot class | n/a | [Tavily](https://docs.tavily.com/documentation/api-credits) |
| Tavily Crawl (map + extract) | $2.25 → $1.50 | $3.75 → $2.50 | — | n/a | same |
| Exa contents (PAYG) | $1.00 | $1.00 | n/a | + summary $1.00 → $2.00 total (LLM summary, not a verified schema) | [Exa](https://exa.ai/pricing) |
| Parallel Extract (PAYG) | $1.00 | $1.00 | n/a | Task API Lite $5 / Base $10 per 1,000 tasks (structured research, not URL extraction) | [Parallel](https://parallel.ai/pricing) |
| Jina Reader (pack $50/1B → $500/11B) | $0.250 → $0.227 (5,000 output tokens/page) | same (headless engine by default) | n/a | n/a | [Jina](https://jina.ai/reader/) |
| Import.io (Standard → Advanced) | monthly $4.98 → $1.80; annual $3.98 → $1.40 | same (JS coverage unverified) | unverified | rows included (not LLM) | [Import.io](https://www.import.io/pricing) |
| Crawlbase PAYG (graduated, standard) | $3.00 (1K/month) · $0.764 (100K) · $0.527 (1M) · $0.147 (10M) | ×2 | ×1.5–×20 | n/a | [Crawlbase](https://crawlbase.com/pricing) |
| Scraping Fish | $2.00 | $2.00 | $2.00 | n/a | [Scraping Fish](https://scrapingfish.com/) |

#### C. Resource-metered platforms (Calc, explicit assumptions)
- **Spider:** the vendor's per-mode ranges give $0.02–$0.10 (HTTP), $0.028–$0.123 (Smart) and $0.035–$0.155 (browser) per 1,000 — [guide](https://spider.cloud/guides/pricing-and-plans/). Its own 1M-page study measured ~$0.30 (HTTP), ~$1.20 (Chrome + residential) and $0.547 on average — [case study](https://spider.cloud/blog/scraping-one-million-pages-what-actually-happens/). Bandwidth alone at 100 KB/page = $0.10/1,000, or $0.15–$0.40 with a proxy (×1.5–×4). LLM extraction: model billed by the provider.
- **Apify** (Website Content Crawler rescaled to current CU prices, Starter → Business): HTTP $0.16 → $0.104; browser $0.40–$4.00 → $0.26–$2.60; anti-bot = browser + residential proxy ($8 → $7/GB adds $4.00 → $3.50 per 1,000 at 0.5 MB/page, or $16 → $14 at 2 MB/page), or Unblocker $1.5 → $1.0/1,000 requests — [WCC](https://apify.com/apify/website-content-crawler), [pricing](https://apify.com/pricing).
- **Hosted browsers**, at 10 s of browser time per page: Browserbase $0.333 (Developer overage) → $0.278 (Startup overage); Browser Use $0.056 (shared sessions) or $0.333 (one 1-minute-minimum session per page); Browserless $1.25 → $0.70 (1 unit per page), all before proxy — [Browserbase](https://www.browserbase.com/pricing), [Browser Use](https://browser-use.com/pricing), [Browserless](https://www.browserless.io/pricing).

#### D. Pages included in each entry plan (Calc: static / JS / anti-bot / LLM)
- Firecrawl Hobby $19: 5,000 / 5,000 / 5,000 / 1,000. ScrapingBee $49: 250,000 / 50,000 / 10,000 / 25,000. ScraperAPI $49: 100,000 / 10,000 / 4,000 / n/a. ZenRows $19: 45,000 / 9,000 / 1,800. Scrapfly $30: 200,000 / 33,333 / 6,666 / 18,181. ScrapingAnt $19: 100,000 / 10,000 / 800. Scrape.do $29: 250,000 / 50,000 / 10,000. ScrapeGraphAI $20: 10,000 / 10,000 / 1,666 / 2,000. Crawl4AI $10: 20,000 / 10,000 / 5,000. Bright Data PAYG $19 buys 12,666 pages of any class. Zyte PAYG $19 buys 146,153 (HTTP T1), 18,811 (browser T1) or 1,181 (browser T5).

### Inferences
- **Price bands by class at the largest self-serve plan:** see the Takeaway. Bright Data's single $1.30–$1.50 price is a central reference: it equals ScrapingBee's premium + JS price (0.999×, Calc). Outliers for the LLM record: ScrapingAnt $52.66 and Olostep $7.98 (unverified).
- **The multiplier grid is the main hidden cost:** on the same plan, the number of pages can drop by a factor of up to 75 (ScrapingBee 1 → 25 → 75) or 125 (ScrapingAnt) depending on the class. Flat-rate vendors cost more on static pages and less on protected pages. Calc: Firecrawl Scale costs 14.4× (11.5× annual) ScrapingBee Enterprise 69 on a static page, but 0.57× (0.46×) on a premium + JS page. Olostep Scale costs 7.65× on static and 0.31× on protected. Bright Data costs 25× on static and the same price on protected. These comparisons hold only if the bundled escalation succeeds on the target.
- **Volume discount ladder:** entry/largest unit price ratio of 6.34× (ZenRows), 5.33× (ScraperAPI), 5.07× (Firecrawl), 4.51× (Olostep), 3.76× (ScrapingBee), 3.00× (ScrapeGraphAI), 2.54× (ScrapingAnt), 2.17× (Zyte HTTP T1), 1.65–1.66× (Scrapfly, Scrape.do, Crawlbase), 1.33× (Diffbot), 1.15× (Bright Data Unlocker) (Calc).
- For Goodfetch Cloud: a per-page price by class with automatic escalation and a ceiling (ScrapingBee's `mode=auto` + `max_cost`, Scrapfly's `cost_budget`), billed on success, would sit in the market bands above. An LLM record priced in tokens of the chosen model plus a per-page fee would also be in line with the market (Zyte, Crawl4AI, Apify for free users).

### Gaps
- Real costs of anti-bot pages depend on the target. Zyte, Crawlbase, Oxylabs and Bright Data assign the tier automatically, so the bands above do not predict the cost of a given site without a test.
- No credible LLM-record price for Crawl4AI, ZenRows, Decodo, Spider (BYO model) or Tavily. Page weight assumptions (5,000 tokens; 0.5–2 MB) change Jina, ScrapingAnt, Zyte, Spider and Apify linearly; Spider measured 49.2 KB of Markdown per page on average, i.e. more than the 20,000 characters assumed.

## 4. Multipliers and hidden costs: credit multipliers, billing of failures, minimums, expiry, spend caps, overage

### Takeaway
In 2026, success-only billing and spend caps are the norm, but the definition of "success" includes 404 (and often 4xx), and several vendors bill without a document (Firecrawl's agent, browser and threat checks). Expiry of monthly credits is common (Apify, Firecrawl except Scale, ScraperAPI per secondary sources, ScrapingAnt, Browse AI, Tavily), while the PAYG and usage-based players advertise that credits never expire (Spider, Crawl4AI, Browser Use, ScrapeGraphAI top-ups).

### Cited Findings

| Vendor | Multipliers | Billing of failures | Minimum / commitment | Expiry / rollover | Spend cap | Overage | Source |
|---|---|---|---|---|---|---|---|
| Apify | CU = GB × h (RAM chosen by the Actor); proxy, storage, transfer extra | Short runs are billed per second (complaint, corpus `reviews-reports-7`) | Plan fee = prepaid usage | Unused usage expires every cycle | Usage limit configurable; fixed cap on agent tokens | At the plan's CU rate on the next invoice; Free blocked | [pricing](https://apify.com/pricing), [javisreview](https://javisreview.com/review/apify-review-2026/) |
| Firecrawl | +4 per option (JSON, PII, etc.), +1 PDF/page, browser 2–7/min | 0 if no document; 403/404 = 1 credit; agent, browser, threat checks billed without a document | Hobby $19 | No rollover except Scale (1 month) / Enterprise (2 months, annual); PAYG credits until cancellation | Monthly PAYG limit (multiple of $5, 0 = off) | $5 per 1,000 → 5,000 credits | [docs](https://docs.firecrawl.dev/billing) |
| Spider | Proxy bandwidth ×1.5–×4; ZDR ×2.5; recording ×1.15 | "Failed requests cost $0" | $1 minimum | Never expires | Compute capped per request (30/50 credits) | PAYG | [pricing](https://spider.cloud/pricing), [guide](https://spider.cloud/guides/pricing-and-plans/) |
| Zyte API | 5 tiers × HTTP/browser (1× to 123.7× between HTTP T1 and browser T5, Calc) | Successful responses only | Commitment $100–$500/month for the discounts | — | Spending limit per level | At the discounted rate, "No overage penalties" | [pricing](https://www.zyte.com/pricing/), [docs](https://docs.zyte.com/zyte-api/pricing.html) |
| ScrapingBee | 1 / 5 / 10 / 25 / 75, AI +5 | 404/410 billed; 5xx no; everything billed with `transparent_status_code` | $49 | Not stated | `max_cost` with `mode=auto` | Upgrade | [contract](https://www.scrapingbee.com/llms/html-api.txt) |
| ScraperAPI | 1 / 5 / 10 / 25 / 30 / 75 + per-domain costs | 200 and 404 billed; cancellation after 70 s billed | $49 | Not carried over (FAQ, secondary) | Not stated | PAYG from Scaling | [docs](https://docs.scraperapi.com/credits-and-requests), [pricing](https://www.scraperapi.com/pricing/) |
| ZenRows | 1 / 5 / 10 / 25; residential 25,000/GB | Successful only (404/410 = success) | $19 | Top-ups roll over once | Not stated | Top-ups ≤10×/period | [pricing](https://www.zenrows.com/pricing) |
| Scrapfly | DC 1, residential 25, JS +5, ASP dynamic, bandwidth | Not billed except listed 4xx codes; protection removed above 30% failures/hour | $30 | Not stated | `cost_budget`; PAG ≤50% of quota; Discovery hard cap | $5 → $1.20 per 10k | [billing](https://scrapfly.io/docs/scrape-api/billing), [pricing](https://scrapfly.io/pricing) |
| Crawlbase | Difficulty ×1–×20 automatic, JS ×2 | Successful only | $0 (PAYG) | — | Not stated | Graduated PAYG | [Crawlbase](https://crawlbase.com/pricing) |
| ScrapingAnt | 1 → 125; AI 1/30 characters | "Failed requests cost 0" | $19 | Not carried over | Not stated | Not stated | [ScrapingAnt](https://scrapingant.com/), [docs](https://docs.scrapingant.com/credits-cost) |
| Scrape.do | 1 / 5 / 10 / 25; forced costs per domain (jd.com 75) | Successful only (2xx, 400, 404, 410) | $29 | Not stated | Requests stop at the limit | Buy credits or upgrade | [pricing](https://scrape.do/pricing/), [costs](https://scrape.do/documentation/request-costs/) |
| Bright Data | One price per product | "Pay only for success"; secondary: everything billed once "custom" features are enabled | $0 PAYG; $499 Scale | — | "Set monthly spend limits" | $1.3/1,000 beyond Scale | [Unlocker](https://brightdata.com/pricing/web-unlocker), [Thunderbit (competitor)](https://thunderbit.com/blog/brightdata-review-costs-alternatives) |
| Oxylabs | Per target and JS | 2xx and 4xx billed, 5xx/6xx not (unverified) | $30 | Not stated | "Real-time spending & usage data" | Tiers | [Oxylabs](https://oxylabs.io/products/web-api/scrape/pricing) |
| Tavily | Basic/advanced ×2 | Failed extractions and maps not charged | $30 | Reset on the 1st of the month | — | PAYG $0.008 | [credits](https://docs.tavily.com/documentation/api-credits), [pricing](https://www.tavily.com/pricing) |
| Jina | OCR ×40; search ≥10,000 tokens | Not deducted | $50 pack | Not stated | Auto top-up threshold | Packs | [Jina](https://jina.ai/reader/) |
| Exa | Per content type | — | $0 | Free $10 resets monthly | Agent caps $5 (auto) / $20 (ultra) | PAYG | [Exa](https://exa.ai/pricing) |
| Crawl4AI Cloud | Effort ×0.5–×4 | 5xx, blocked 403, 502/503, cancellations free | $10 pack | Never expires (soft launch) | Overdraft 50 credits | Packs | [docs](https://crawl4ai.com/docs), [prices](https://crawl4ai.com/v1/prices) |
| Olostep | None published | Successful only (cards); unverified for LLM | $9 | Top-ups valid 6 months | — | Top-ups $2.00 → $0.50 per 1,000 | [Olostep](https://www.olostep.com/pricing) |
| Diffbot | Proxy ×2 | Not stated | $299 | — | Free: `429` at quota | $0.001 → $0.0009/credit | [Diffbot](https://www.diffbot.com/pricing/) |
| Browse AI | Premium sites 2–10× | — | $19 (annual) | No rollover | — | $0.024 → $0.013/credit | [Browse AI](https://www.browse.ai/pricing) |
| Import.io | None (1 page = 1 query) | Successful queries | $199 (annual) | — | MCP spend limit | $7 → $2.50 per 1,000 | [Import.io](https://www.import.io/pricing) |
| Browser Use | Rounding to the minute, 1-minute minimum | — | $5 top-up | "Credits never expire" | Concurrency by lifetime spend | — | [Browser Use](https://browser-use.com/pricing) |
| Browserless | Unit = 30 s; reconnection = new unit; proxy in units/MB | — | $25 (annual) | — | — | $0.0020 → $0.0015/unit | [Browserless](https://www.browserless.io/pricing) |

- User complaints from the corpus, by theme: multipliers that drain entry plans (ScrapingBee Capterra 2026-08 — [Capterra](https://www.capterra.co.uk/software/195060/scrapingbee); ScraperAPI, with "undisclosed 5x multipliers on large deals" per a competitor — [Thunderbit](https://thunderbit.com/blog/scraper-api-review); Firecrawl extract "up to 10x base credits" — [skiln.co](https://skiln.co/blog/firecrawl-review-2026)); failures billed (Firecrawl /agent — [runwhatworks](https://runwhatworks.com/blog/firecrawl-review); Bright Data custom features — [Thunderbit](https://thunderbit.com/blog/brightdata-review-costs-alternatives)); no cap ("it cost me $40 the first night", Reddit 2026-06, archived — [snapshot](https://reddit.sentinel-team.org/posts/1u9t2m9/snapshots/2026-06-19T20%3A07%3A29.810032Z); Zyte "no way to control your spending", 2024 — [Capterra](https://www.capterra.co.nz/software/180165/zyte)); credits that expire silently (Apify — [javisreview](https://javisreview.com/review/apify-review-2026/)).

### Inferences
- Zyte (spending limits), Firecrawl (PAYG limit), ScrapingBee (`max_cost`), Scrapfly (`cost_budget`), Exa (agent caps) and Import.io (MCP limit) now offer explicit caps, which answers the "no cap" complaints of 2024–2026. The remaining hidden costs are the definition of billable success (4xx, 404, block pages served as 200), expiry, and bandwidth billed separately.
- For Goodfetch: billing only on verified success (and not on 4xx block pages), never-expiring prepaid credits, and a per-request and per-run ceiling would differentiate the product on documented pain points.

### Gaps
- Rollover rules for ScrapingBee, Scrape.do, Scrapfly and Diffbot are not published. The exact billing of Oxylabs's 4xx results is unconfirmed. Whether Bright Data still bills failures when custom features are enabled is unconfirmed on an official page.

## 5. Changes in 2025–2026 and user reactions

### Takeaway
2025–2026 brought price cuts and restructuring: Apify (Scale −60% in 2025, CU −33 to −36% in April 2026, Starter $39 → $29 → $19 in 2026), Bright Data (−50% on Unlocker and SERP in January 2025), rentals retired at Apify, and new players with flat per-page prices (Crawl4AI Cloud, Olostep, Parallel). Consolidation also accelerated (ScraperAPI acquired Traject Data; Jina AI acquired by Elastic, secondary). User complaints about multipliers, failure billing and expiring credits persist.

### Cited Findings
- **Apify** (PricingSaaS, secondary): Scale $499 → $199 (2025 Q1, −60%); Starter CU $0.4 → $0.3 and Scale $0.3 → $0.25 (2025 Q3); Free prepaid credit $0 → $5 (2025 Q2); Starter $39 → $29 (April 2026); CU $0.3 → $0.2 (Free/Starter), $0.25 → $0.16 (Scale), $0.2 → $0.13 (Business) and RAM add-on $2 → $1/GB (2026-04-27); RAM limits doubled (2026-06-29); Free concurrency 25 → 5 (2026-08-31); Starter $29 → $19 (2026-09-07) — [PricingSaaS](https://pricingsaas.com/companies/apify). Calc: the April 2026 CU cuts are −33.3%, −36.0% and −35.0%; Starter −34.5%; Scale −60.1%. The current values match the [official pricing](https://apify.com/pricing).
- **Apify Store:** no new rentals since 2026-04-01; rentals retired 2026-10-01 — [rental docs](https://docs.apify.com/platform/actors/publishing/monetize/rental). 2,000 pay-per-result Actors migrated to PPE (blog 2026-04-14) — [Apify blog](https://blog.apify.com/standardizing-actor-pricing).
- **Bright Data:** Proxyway (2025-01-09) reports Web Unlocker and SERP API cut by 50% (PAYG $3 → $1.5) and Scraper APIs raised by 50% ($1 → $1.5) — [Proxyway](https://proxyway.com/news/bright-data-scraper-prices-unlocker-api) (secondary, industry press). The current $1.5 prices are confirmed — [Unlocker](https://brightdata.com/pricing/web-unlocker), [Scraper API](https://brightdata.com/pricing/web-scraper).
- **Jina:** "new pricing model on May 6th, 2025" with grandfathering — [Jina](https://jina.ai/reader/). Secondary sources date the acquisition by Elastic to 2025-10-09 — [usagepricing](https://usagepricing.com/blueprint/jina-ai). The Jina page itself says Elastic has sold the on-prem model licence separately since 2026-08-10 — [Jina](https://jina.ai/reader/).
- **Spider:** the §7 divergence (on 2026-10-07, $0.001 per CPU-minute on the pricing page vs $0.0001 on the homepage) is resolved. On 2026-10-12, the pricing page and the guide both show $0.0001 — [pricing](https://spider.cloud/pricing), [guide](https://spider.cloud/guides/pricing-and-plans/). A concurrency plan ("Unlimited", from $40/month) is now listed — [pricing](https://spider.cloud/pricing).
- **ScrapingBee:** §7 cited the help article of 2025-10-17 at $49.99 / $99.99 / $249.99 / $599.99; the page now shows $49 / $99 / $249 / $599, plus four Enterprise tiers up to $3,599 — [pricing](https://www.scrapingbee.com/pricing/).
- **Firecrawl:** §7 recorded "interact 2 credits per browser-minute"; the docs now say 2–7 credits per minute (7 with a prompt). Enhanced proxy is +0 — [docs](https://docs.firecrawl.dev/billing). Secondary sources describe an earlier separate token-based Extract plan from $89/month and enhanced mode at 5 credits/page (unverified, history) — [eesel.ai](https://www.eesel.ai/blog/firecrawl-pricing).
- **Oxylabs:** the Web Scraper API page now redirects to a "Web API /Scrape" product with $30 / $100 / $500 plans — [Oxylabs](https://oxylabs.io/products/web-api/scrape/pricing). A secondary review cited a $49 plan in July 2026 — [webscraping.cc](https://webscraping.cc/blog/oxylabs-scraper-api/).
- **ScraperAPI:** "ScraperAPI has acquired Traject Data" banner; seven published plans up to $1,975 — [pricing](https://www.scraperapi.com/pricing/).
- **Import.io:** self-serve plans "bought directly with no sales call", page dated 16 September 2026 — [Import.io](https://www.import.io/pricing). **Crawl4AI:** cloud soft launch — [docs](https://crawl4ai.com/docs).
- **Reactions (corpus):** "egregiously expensive" and Firecrawl replaced by ~2,700 lines of Elixir at ~$190/month (HN 2026-03-30) — [HN](https://news.ycombinator.com/item?id=47578663). A competitor's launch positioned on "one credit per scrape, no hidden multipliers" (HN 2026-07-09) — [HN](https://news.ycombinator.com/item?id=48848110). Objection to a jump from free to $2,500/month with no pay-as-you-go (HN 2025-10-02) — [HN](https://news.ycombinator.com/item?id=45452081). 10 GB of proxy used up in two days on the browser tier (HN 2026-08-03) — [HN](https://news.ycombinator.com/item?id=49153128). Apify: compute-unit pricing gets expensive at volume (G2, 611 reviews) — [G2](https://ai.g2.com/product/apify); proxy costs (Trustpilot) — [Trustpilot](https://www.trustpilot.com/review/apify.com). Oxylabs: Trustpilot 3.7 over 724 reviews, billing discrepancies and accounts blocked at KYC — [Trustpilot](https://www.trustpilot.com/review/oxylabs.io). Spending: 58.3% of respondents increased their proxy budget (Apify survey, December 2025) — [Apify report](https://blog.apify.com/web-scraping-report-2026/). Interest in ~$2/1,000 pages for AI scrapers (Reddit aggregator) — [discury.io](https://discury.io/report/best-ai-web-scrapers-2025-reddit).

### Inferences
- The low end is being squeezed (Apify Starter at $19, Bright Data at $1.5, Crawl4AI at $0.50/1,000, Browser Use at $0.02/h), while value moves to agentic endpoints (Exa agent, Parallel Task, Firecrawl agent, Apify PPE/MCP) billed per event or per token.
- The migration of the Apify Store to PPE (97.1% of Actors) indicates the unit buyers accept in an actor marketplace: price per event or result, with platform costs explicit. Complaints show that multipliers and expiry, more than the base price, drive switches to DIY.

### Gaps
- The search budget ran out, so I ran no fresh HN or Reddit searches on the 2026 changes (Apify rentals, Starter at $19, Crawl4AI Cloud). The reactions come from the existing corpus and predate some changes.
- The exact effective dates of several changes (Bright Data January 2025, ScraperAPI–Traject, Jina–Elastic) are not confirmed by official sources. Neither are the history of Firecrawl plans and Tavily changes in 2025–2026: I found no changes on Tavily's page.
