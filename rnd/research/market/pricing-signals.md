# Price signals: budgets, accepted or rejected units, billing complaints

**Status:** v1, desk research. **Date:** 2026-10-08. **Corpus:** 421 items ([corpus.jsonl](corpus.jsonl)). Codes used: `wtp-price-points` (46 items), `cost-unpredictable-runaway` (28), `cost-infra-proxy-spend` (19), `build-vs-buy-switching` (31).

> **Warning: desk research does not measure willingness to pay.** This document lists what people **say** they paid, refused or found unfair, as well as the prices that **vendors display**. These signals are self-selected, since people complain more readily than they praise. They are often promotional (`[P]`) and say nothing about a price accepted for Automan. The budgets in job postings pay for **human labor, not software**, and the Upwork sample leans toward the low end of the market ($3-70/h, $35-700 fixed price). The `D-pricing` decision requires interviews and a price test (§5).

## 1. Observed price points, by segment

| Segment | Signal | Value | Nature | Source |
| --- | --- | --- | --- | --- |
| Hobbyists | Total budget for a personal project blocked by Turnstile | ~$50/month | User statement | `reddit-2` |
| Hobbyists | Residential pool judged over budget | $100/month | User statement | `reddit-12` |
| Hobbyists | Firecrawl replaced by about 2,700 lines of Elixir | ~$190/month paid before | User statement | `hn-58` |
| Hobbyists | Two simple scrapes and a Sheets update via an agent | ~$15 | User statement | `reddit-30` |
| Price monitoring (SMB) | Desired threshold compared with Prisync ($99) and Price2Spy ($83) | < $30/month | User statement | `reddit-43` |
| Price monitoring (SMB) | Refuses tools at $100/month and above; considers scripting it themselves | > $100/month refused | User statement | `reddit-47` |
| Price monitoring (SMB) | Daily scraper with anti-bot to Sheets | $35 fixed price | Job posting | `demand-use-cases-6` |
| Price monitoring (enterprise) | DIY stack replaced by an API | $530k/yr → $75k/yr | Vendor `[P]` | `demand-use-cases-57` |
| Agent developers | Cost of an autonomous agent stack on a VPS | $30-400/month | User statements | `reddit-64` |
| Agent developers | Price tested for a structured agentic scraper | $0.025-0.03/request (question) | User statement | `demand-use-cases-23` |
| Agent developers | Acceptable price for a product database API | ~$0.01/1k requests | User statement | `demand-use-cases-24` |
| Agent developers | Notte pricing after a complaint about credit opacity | 1¢/scrape, 2¢/step, 1¢/minute; ~21¢ for 10 steps and 1 min | Vendor `[P]` | `hn-75` |
| Agent developers | Market browser infrastructure according to Firecrawl | $0.005-0.05/browser minute; ~$0.005/page | Vendor `[P]` | `demand-use-cases-49` |
| Agent developers | Cost per task depending on the harness, same model | $1.50 vs ~$10/task | Vendor `[P]` | `hn-89` |
| Agent developers | Agent loop replaced by a replayed JSON plan | $0.50-3.00 → $0.01-0.05/task | Vendor `[P]` | `reddit-62` |
| Agent developers | Agent cost for three form submissions, vs ~1,000 human captchas for $2 | $2.27 | User statement | `hn-69` |
| Agent developers | Captcha solvers | ~$1-3/1,000 | User statements | `reddit-66` |
| B2B portals | Sales-led plan, signed by 5-person companies according to the vendor | $2,500/month | Vendor `[P]` | `hn-33` |
| B2B portals | Objection: direct jump from free trial to $2,500/month, with no pay-as-you-go | — | User statement | `hn-32` |
| B2B portals | Starter pack requested | ~$10 for 5k credits | User review | `reviews-reports-16` |
| B2B portals | Playwright robots from portals to GoHighLevel | $3/h | Job posting | `demand-use-cases-4` |
| Data teams | Daily monitoring run over 500 sites for agents | ~$310/month, including $180 of compute | User statement | `reddit-59` |
| Data teams | Oxylabs $49 plan | ~42.6k generic web results instead of the advertised 98k | Blog post | `reviews-reports-28` |
| Data teams | Public data scraping | $10/h; $15-35/h; MVP at $100 | Job postings | `demand-use-cases-2`, `demand-use-cases-9`, `demand-use-cases-0` |
| Enterprise | In-house team vs managed service | ~$260k/yr vs ~$10k/month | Vendor `[P]` | `demand-use-cases-58` |
| Enterprise | Consolidation of three providers (proxy, captcha, browsers) | $22.1k → $5.9k/month | Vendor `[P]` | `demand-use-cases-53` |
| Enterprise | Resale of public datasets | Raw $0.5-5; enriched $5-50 per 1k records | User statements | `demand-use-cases-59` |
| Agent agencies | Scoping, implementation, maintenance subscription | $1.5-3k; $5-25k; $1-5k/month, or $300-1,500/month per client | Vendor `[P]` | `demand-use-cases-70` |
| SEO and GEO | AI visibility tracking tools | $95-828/month | Review | `demand-use-cases-66` |
| SEO and leads | Google Maps leads: price per accepted lead, sample of 50 | per accepted lead | Job posting | `demand-use-cases-16` |
| SEO and leads | Enriched Google Maps data, quote for 2,000 and 5,000 records | per record | Job posting | `demand-use-cases-18` |
| SEO and leads | Google Maps at scale and emails | $700 fixed price (noted during the source check) | Job posting | `demand-use-cases-15` |
| SEO and leads | LinkedIn automation SaaS | ~$2k the first month, in lifetime deals | Vendor `[P]` | `reddit-78` |
| Cross-cutting | Interest in a price of about $2/1k pages for AI scrapers | ~$2/1k pages | Reddit thread aggregator | `reddit-81` |

**Orders of magnitude.** **Individual** budgets, stated by users, range between $30 and $400/month. For simple access, the **unit** prices accepted or requested range from $0.00001 ($0.01 for 1k requests, `demand-use-cases-24`) to $0.03 per request or per page. Most fall between $0.002 and $0.01 (`reddit-81`, `demand-use-cases-49` `[P]`, `hn-75` `[P]`). An agent task costs from ~$0.20 to ~$10 depending on the harness (`hn-75` `[P]`, `reddit-62` `[P]`, `hn-89` `[P]`). Prices above $1,000/month all come from vendors `[P]`. In this high end, the only independent payment indicator is a sign-up for Browser Use's paid tier, with no known amount (`hn-13`).

## 2. Rejected or accepted units

### 2.1 Rejected or sources of complaints

| Unit or practice | Complaint | Sources |
| --- | --- | --- |
| **Credit multipliers** by option (JS, premium proxy, screenshot, extraction) | Entry tiers run out quickly; confusing breakdown; unannounced 5x multiplier on large contracts; extraction up to 10 times the base credit | `reviews-reports-13`, `reviews-reports-18`, `reviews-reports-19` `[P]`, `reviews-reports-32`, `reddit-35` |
| **Billing for failures** | Failed requests billed; agent endpoint billed even on failure; very short runs billed | `reviews-reports-10` `[P]`, `reviews-reports-31`, `reviews-reports-7`, `hn-76` |
| **No spending cap** | Bill about 40 times what was expected; $40 retry loop in one night; item limits not respected | `reviews-reports-23`, `reddit-57`, `reviews-reports-7` |
| **Bandwidth billing** | Monthly cost impossible to predict; 10 GB of proxy consumed in two days | `reviews-reports-12`, `hn-55` |
| **Credits that expire** with no rollover | Penalizes bursty usage; pay-as-you-go requested | `reviews-reports-30`, `reviews-reports-7` |
| **Compute units** at large volumes | Become expensive; proxy fees on top of Actor fees | `reviews-reports-3`, `reviews-reports-2` |
| **Gated access** (sales call, KYC, card required despite "no card") | Hinders self-service purchase; accounts blocked at sign-up | `reviews-reports-12`, `reviews-reports-22`, `reviews-reports-25` |
| **Price jump** (free → $2,500/month) | Nothing in between to test | `hn-32` |
| **Opaque agent bill** (every click and every retry billed) | Retries and failed branches often account for 30 to 40% of a run's cost | `reddit-30`, `reddit-65`, `reddit-64` |

### 2.2 Accepted or requested

| Unit or practice | Signal | Sources |
| --- | --- | --- |
| **Payment per successful result** | Oxylabs only bills successes, which makes spending predictable (the advertised volume, however, is misleading); SerpApi counts 1 credit per successful non-cached search; enterprise checklist guide | `reviews-reports-28`, `reddit-34` `[P]`, `demand-use-cases-64` `[P]` |
| **Payment per accepted lead or record** | Quote per valid lead, per record | `demand-use-cases-16`, `demand-use-cases-18`, `demand-use-cases-59` |
| **Pay-per-event** | 97.1% of the 64,812 Actors in the Apify Store | `reviews-reports-59` |
| **1 credit = 1 scrape, no multiplier** | Positioning argument against Firecrawl | `hn-64` `[P]` |
| **Pay-as-you-go and small starter packs** | Requested in response to expiring credits, price jumps and overly thin trials | `reviews-reports-30`, `hn-32`, `reviews-reports-16` |
| **Caps per task, per run, per tool** and retry budgets | Spending broker external to the agent | `reddit-58`, `reddit-57` |
| **Scope-based, predictable pricing** (managed services) | Promoted by a services vendor | `demand-use-cases-58` `[P]` |
| **Cost per accepted result** as an internal indicator | Tracked at p95 | `reddit-65` |

## 3. Where the money actually goes

1. **Access costs more than code.**
   - 58.3% of respondents to the Apify survey increased their proxy spending, and 62.5% their infrastructure costs (`reviews-reports-36`, `reviews-reports-39` `[P]`).
   - The residential proxy bill grows faster than compute (`reddit-15`). Bandwidth costs orders of magnitude more than Anubis's proof of work (`hn-118`).
   - An Instagram project goes from $40 to $2,000/month at 10k profiles (`demand-use-cases-71` `[P]`).
2. **The LLM costs more than the browser in agent workflows** (`hn-14`). Cost differences come from the harness more than from the model (`hn-89` `[P]`, `reddit-61`).
3. **Retries are a hidden cost.** Failed branches account for 30 to 40% of a run's cost (`reddit-65`); the extreme example is a loop of about 200 calls (`reddit-57`).
4. **Human time** remains the most cited item: 6 months to make a price monitoring stack reliable (`reddit-42`), "babysitting" agents (`reddit-0`, `reddit-64`).

Contrary signal: **cost is not the primary reason for switching tools.** Among developers who switched AI tools, 6.4% did so for cost, versus 25.1% for quality (`reviews-reports-53`, Stack Overflow 2026). On the scraping side, switches to hosted solutions are explained by reliability and workload (`reviews-reports-14`, `reddit-3`, `reddit-6`). Switches to DIY are explained by price (`hn-58`, `hn-56`).

## 4. Reading for `D-pricing`

| Option | What the signals say | Condition |
| --- | --- | --- |
| `verified-result` | The most aligned with the complaints: no billing for failures, predictable spending, a unit close to what lead and data buyers already ask for (per lead, per record, per successful result) | Define "verified" in an auditable way; oracle contracts and zero false success, see `C-zero-false-success` and `C-cost-per-1k-verified` in the lab |
| `compute-units` | The most criticized pattern when it is opaque: multipliers, bandwidth, expiring credits | Acceptable only with a per-run breakdown, hard caps and no billing for platform failures |
| `subscription-tiers` | Accepted for small monitoring needs, but willingness to pay there is low (under $30 to $100/month); price jumps are rejected | Fine-grained tiers, pay-as-you-go beyond, caps |

The unit that most often comes back as the ideal is a **successful result billed at actual usage, with a hard cap**. The signals do **not** say how much. Nor do they say whether buyers would agree to pay more per verified unit to avoid billed failures.

## 5. What to measure next (outside desk research)

1. **Structured interviews** per candidate segment, at least five per segment:
   - current budget broken down (proxy, captcha, LLM, infrastructure, human time);
   - last billing incident;
   - preferred unit;
   - price threshold questions (too cheap, cheap, expensive, too expensive) on a defined verified result.
2. **Price test** on a landing page. Compare two units (verified result vs capped subscription) and measure the waitlist sign-up rate.
3. **Paid PoC** on 20 to 50 of the customer's domains, on the model of the enterprise checklist (`demand-use-cases-64` `[P]`). This is the only real payment signal.
