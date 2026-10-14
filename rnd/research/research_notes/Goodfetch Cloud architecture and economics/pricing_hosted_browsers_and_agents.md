# Pricing of hosted browsers and AI browser agents, October 2026

All prices were read on **2026-10-12** from the page linked next to each figure. They are in USD, exclude taxes, and are billed monthly unless marked "annual". "Computed" values were calculated with Python from the cited rates; the scripts are in the session scratchpad, not in the repo. This pass started from, and refreshes, section 7 of [the 2026-10-07 commercial landscape](../../2026-10-07-commercial-landscape.md).

Limits of this pass:
- The shared WebSearch budget ran out midway, so later checks used direct fetches of official pages only.
- The fetch tool could not reach the Wayback Machine (web.archive.org). Change history therefore comes from vendor changelogs, "last edited" dates, legacy-plan notes, and undated search-index snapshots, which are marked unverified.
- reddit.com was blocked for search. Reddit signals come from the repo's market corpus, which holds mirror URLs.
- Some pricing pages render with JavaScript (Hyperbrowser's plan cards, Scrapeless's plan cards) and returned no figures.

## 1. Plans, fees, included usage, overage and limits for each service

### Takeaway
On mainstream entry plans, bare browser time lists at $0.00–0.12 per hour, while residential proxy traffic lists at $1.80–12 per GB. ZenRows' Scale credits convert to $1.39–1.67 per GB, and GoLogin claims $1/GB once its allowance is used. Upper plans are sold on concurrency, session length, the strongest stealth tiers, SLAs and compliance; every service sampled bundles recording and live view, and captcha solving is either bundled or priced at about $1–20 per 1,000 solves.

### Cited Findings

#### Browserbase
- **Plans:** Free $0, Developer $20/mo, Startup $99/mo ("most popular"), Scale custom. No annual price is shown. — [Pricing](https://www.browserbase.com/pricing)
- **Hours and proxies:** Free 1 browser-hour. Developer 100 h, then $0.12/h. Startup 500 h, then $0.10/h. Proxy data: 0 GB on Free, 1 GB then $12/GB on Developer, 5 GB then $10/GB on Startup. Scale is usage-based. — [Plans doc](https://docs.browserbase.com/account/billing/plans.md)
- **Billing granularity:** browser time is billed by the minute and proxy bandwidth by the MB, rounded, "with a one-minute and one-MB minimum per session". — [Plans doc](https://docs.browserbase.com/account/billing/plans.md)
- **Capacity:**
  - Concurrency 3 / 25 / 100 / 250+.
  - Session creation 5 / 25 / 50 / 150+ per minute.
  - Maximum session 15 min on Free, 6 h on Developer and Startup, 6+ h on Scale.
  - Projects 1 / 2 / 5 / 5+.
  - — [Plans and pricing guide](https://docs.browserbase.com/guides/plans-and-pricing)
- **Keep-alive:** "Session keep alive is only available on paid plans". Unreleased sessions keep accruing browser minutes. — [Keep-alive doc](https://docs.browserbase.com/platform/browser/long-sessions/keep-alive.md)
- **Identity and stealth:**
  - No captcha solving on Free; "Auto captcha solving" on Developer and Startup.
  - Stealth Mode is "Basic" on Developer and Startup and "Advanced" on Scale, with "Verified Agents" on Scale. — [Pricing](https://www.browserbase.com/pricing)
  - The docs call the same tiers "Verified: Basic / Verified". — [Plans doc](https://docs.browserbase.com/account/billing/plans.md)
- **Retention conflict:** the pricing page lists 7 days on Free, 30 days on Developer and Startup, and 30+ days on Scale. The plans doc lists 7 days for Developer. — [Pricing](https://www.browserbase.com/pricing); [Plans doc](https://docs.browserbase.com/account/billing/plans.md)
- **Bundled platform services:**
  - Agent runs included: 3 / 15 / 50 / custom.
  - Search: 1,000 calls included, then $7 per 1k.
  - Fetch: 1,000 calls on Developer, 10,000 on Startup, then $1 per 1k on Developer and $0.5 per 1k on Startup; $4 per 1k with proxies.
  - Extract: $4 per 1k, $7 per 1k with proxies.
  - Functions are free on every plan.
  - Model Gateway: "$5 in tokens" on Free, "Market Price: Pay as you go" on paid plans. — [Pricing](https://www.browserbase.com/pricing); [Plans doc](https://docs.browserbase.com/account/billing/plans.md)
- **Recording:** session recording is described in the FAQ with no separate fee. — [Pricing](https://www.browserbase.com/pricing)
- **Enterprise:** Scale offers HIPAA (BAA), DPA and SSO. No SLA figures are published. — [Pricing](https://www.browserbase.com/pricing)
- **Vendor claim:** 100 browser-hours is "roughly 3,000 page-level tasks". — [Pricing](https://www.browserbase.com/pricing)
- **Director:** the pricing page describes Stagehand but does not price it, and no separate Director price was found. — [Pricing](https://www.browserbase.com/pricing)

#### Browser Use Cloud (browser sessions)
- **Billing model:** no subscription plans, "Pay as you go". Minimum top-up is $5; credits never expire. Existing subscribers can still manage their legacy billing. — [Pricing](https://browser-use.com/pricing)
- **Rates:**
  - Browser $0.02/h, "Rounded up to whole minutes. 1-minute minimum."
  - Residential proxy $5/GB, on by default.
  - Direct traffic or your own proxy $0.20/GB. — [Pricing](https://browser-use.com/pricing)
- **Concurrency by lifetime spend:** 10 sessions at $0, 50, 250 at $1,000, 500 at $5,000, 1,000 at $25,000. "Your limit never goes down." — [Pricing](https://browser-use.com/pricing)
  - The threshold for 50 sessions is $200 on the pricing page but $100 in the docs. — [Concurrency doc](https://docs.browser-use.com/cloud/guides/concurrency.md)
- **Session lifetime:** V4 agent browsers have "a four-hour hard timeout". Idle browsers become eligible for cleanup after about 20 minutes without run activity. — [Concurrency doc](https://docs.browser-use.com/cloud/guides/concurrency.md)
- **Captcha, stealth, recording, residency:** supported captchas are solved automatically. Chromium is managed with stealth protections. "API browser recording defaults to off", and Zero Data Retention projects never record. An EU deployment runs in Frankfurt. — [Docs index](https://docs.browser-use.com/llms.txt)
- **Free tier:** "$1 once per person" for eligible Google, GitHub or Microsoft signups, with no card. — [Pricing](https://browser-use.com/pricing)
- **Enterprise:** the page links to an enterprise contact but lists no terms or SLA. — [Pricing](https://browser-use.com/pricing)

#### Steel.dev
- **Plans** (no annual prices):
  - Launch: $0 plus usage, with $30 of credit once, valid 90 days.
  - Scale: $250/mo plus usage, with $100 of usage credit every month.
  - Enterprise: custom.
  - — [Pricing](https://steel.dev/pricing); [Pricing and limits doc, "Last edited: June 30th, 2026"](https://docs.steel.dev/overview/pricinglimits)
- **Usage rates, Launch / Scale:**
  - Browser hours $0.10 / $0.08, "billed by the minute, rounded up".
  - Proxy $10/GB / $6/GB.
  - Captcha $3 / $1 per 1k solves.
  - Browser tools (`/scrape`, `/screenshot`, `/pdf`) $5 per 1k.
  - — [Pricing and limits doc](https://docs.steel.dev/overview/pricinglimits)
- **Limits, Launch / Scale / Enterprise:**
  - Concurrency 10 / 100 / 1,000+.
  - Maximum session 15 min / 1 h / up to 24 h.
  - Requests per minute 60 / 600 / custom.
  - Retention 7 / 14 days / custom.
  - — [Pricing and limits doc](https://docs.steel.dev/overview/pricinglimits)
- **Feature gates:**
  - The stealth browser and reserved browser pools are Enterprise only.
  - Dedicated IPs cost $5 per IP per month on Scale.
  - On Launch, a $10 paid deposit unlocks captcha solving and Steel-provided proxies.
  - Support and uptime SLAs are Enterprise only. SSO and a HIPAA BAA come with Scale.
  - — [Pricing](https://steel.dev/pricing); [Pricing and limits doc](https://docs.steel.dev/overview/pricinglimits)
- **Legacy plans:** "Existing Starter, Developer, and Startup customers can keep their legacy plan." — [Pricing and limits doc](https://docs.steel.dev/overview/pricinglimits)
- **Payments:** pay-per-use billing over x402 (USDC) is documented. — [Docs index](https://docs.steel.dev/llms.txt)

#### Hyperbrowser
- **Credits:**
  - 1 credit = $0.001.
  - Browser time is 100 credits per hour ($0.10/h). Proxy data is 10,000 credits per GB ($10/GB).
  - Plan credits refresh at renewal. Directly purchased credits "expire after 12 months".
  - — [Docs pricing](https://hyperbrowser.ai/docs/pricing)
- **Other services:**
  - Sandboxes: $0.045 per vCPU-hour and $0.015 per GiB-hour.
  - Fetch: 1 credit per page, +9 with a proxy, +5 for JSON.
  - Search: 5 credits, or 10 with a location.
  - Scrape: 1 credit, or 10 with a proxy.
  - AI Extract: $30 per million output tokens.
  - — [Docs pricing](https://hyperbrowser.ai/docs/pricing)
- **Stealth:** a standard stealth mode, plus "Ultra stealth", which is "only available on enterprise plans". — [Stealth doc](https://hyperbrowser.ai/docs/sessions/stealth.md)
- **Session timeout:** set per team. `keepAlive=true` holds a session until it times out, and a session stops when all its pages close. No maximum is published. — [Lifecycle doc](https://hyperbrowser.ai/docs/sessions/lifecycle.md)
- **Plan fees (UNVERIFIED):** the official pricing page returned no text, so these figures come from third-party aggregators only, one of which says it last verified them on September 6, 2026.
  - Startup $30/mo: 30,000 credits, 25 concurrent browsers, 30-day retention.
  - Scale $100/mo: 100,000 credits, 100 concurrent.
  - Free: 1,000 or 5,000 credits, 1 concurrent (the sources disagree).
  - Enterprise: 1,000+ concurrent.
  - — [costbench](https://costbench.com/software/browser-automation/hyperbrowser/); [aicoolies](https://aicoolies.com/pricing/hyperbrowser); [freetier.co](https://freetier.co/directory/products/hyperbrowser); [EveryDev](https://www.everydev.ai/tools/hyperbrowser)

#### Kernel
- **Plans** (each plus usage):
  - Free / Developer $0: $5 of credit per month, 5 concurrent.
  - Hobbyist $30/mo: $10 of credit, 10 concurrent.
  - Start-Up $200/mo: $50 of credit, 150 concurrent.
  - Enterprise: custom.
  - — [Pricing](https://www.kernel.sh/pricing); [Concurrency and limits](https://kernel.sh/docs/browsers/concurrency-and-limits.md)
- **Usage per second:**
  - Headless $0.0000166667, headful $0.0001333336, headful + GPU $0.0008000016. GPU needs Start-Up or Enterprise. — [Pricing doc](https://www.kernel.sh/docs/info/pricing)
  - Computed: $0.06, $0.48 and $2.88 per hour.
  - Default memory is 1 GB headless and 8 GB headful (up to 16 GB). — [Concurrency and limits](https://kernel.sh/docs/browsers/concurrency-and-limits.md)
  - This matches the "$0.0000166667 per GB-second" footnote: 1 GB × 3,600 s = $0.06/h; 8 GB = $0.48/h. — [Pricing](https://www.kernel.sh/pricing)
- **Idle time:** usage stops 5 seconds after the last CDP or Live View activity (Standby). "You don't pay for idle time." — [Pricing doc](https://www.kernel.sh/docs/info/pricing)
- **Proxies:**
  - Kernel-provided ISP, residential and mobile proxies are "unmetered and not billed", subject to fair-use rules. — [Proxies doc](https://kernel.sh/docs/proxies/overview.md)
  - Hobbyist lists "Unlimited proxy usage"; Start-Up adds custom (bring-your-own) proxies. — [Pricing](https://www.kernel.sh/pricing)
- **Session timeout:** `timeout_seconds` defaults to 60 with a maximum of 259,200 (72 h). Reserved pool capacity and standby browsers count toward concurrency. — [Concurrency and limits](https://kernel.sh/docs/browsers/concurrency-and-limits.md)
- **Included features:** live view; stealth mode (captcha solver and proxy); managed auth with no per-connection fee; session replays kept 1 / 7 / 30 days by plan. — [Pricing](https://www.kernel.sh/pricing); [Pricing doc](https://www.kernel.sh/docs/info/pricing)
- **Enterprise:** 99.9% SLA, SSO, SOC2 and HIPAA, zero data retention, white-labeling. — [Pricing](https://www.kernel.sh/pricing)
- **Other billing details:**
  - Through MPP, "one stealth, headful browser for 30 minutes costs $0.50", more than the $0.24 the usage rate gives for 30 minutes.
  - App invocations are billed for active compute, failed and canceled runs included.
  - — [Pricing doc](https://www.kernel.sh/docs/info/pricing)
- **Vendor claim:** customers "often see 50–98% lower costs". — [Pricing](https://www.kernel.sh/pricing)

#### Anchor Browser
- **Plans (docs):**
  - Free $0: $5 of credit per month, 5 concurrent, 5 new sessions per minute.
  - Starter $50/mo: 25 concurrent, 25 per minute.
  - Growth $2,000/mo: 200 concurrent, 100 per minute.
  - Enterprise: custom, 500+ concurrent, 200+ per minute.
  - Overage is $1.00 per credit. — [Docs pricing](https://docs.anchorbrowser.io/pricing.md)
  - The marketing page also lists Team at $500/mo (500 credits, 50 concurrent, 30-day retention). — [Pricing page](https://anchorbrowser.io/pricing)
- **Infrastructure rates (docs):**
  - $0.01 per browser created.
  - $0.05 per browser-hour, billed to the nearest full minute of active time.
  - Anchor proxy $8/GB. Your own proxy, or no proxy, $0.20/GB of egress.
  - Worked example: 10 browsers × 5 h + 2 GB = $18.60. — [Docs pricing](https://docs.anchorbrowser.io/pricing.md)
- **Conflict:** the marketing page's calculator uses "$0.09/hour" for browser duration, and its two plan estimates are internally inconsistent. — [Pricing page](https://anchorbrowser.io/pricing)
- **Feature gates:**
  - Captcha bypass, authenticated browsers and geolocation from Starter.
  - "Full stealth solution", Cloudflare verified browser agents and bring-your-own proxy from Growth. Unique fingerprinting on Enterprise.
  - Retention 7 days on Free and Starter, custom on Growth, zero data retention on Enterprise.
  - SOC2 Type 2 and ISO27001 on Growth; BAA, HIPAA, SSO, and bring-your-own-cloud or on-premise on Enterprise.
  - — [Docs pricing](https://docs.anchorbrowser.io/pricing.md)
- AI step pricing is in section 2.

#### Browserless
- **Plans** (prices billed annually; "yearly billing saves 30%"; monthly-billing prices are not displayed):
  - Free $0: 1k units.
  - Prototyping $25/mo: 20k units, overage $0.0020 per unit.
  - Starter $140/mo: 180k units, overage $0.0017.
  - Scale $350/mo: 500k units, overage $0.0015.
  - Enterprise: custom.
  - — [Pricing](https://www.browserless.io/pricing)
- **Unit:** "a block of browser time of up to 30 seconds per browser connection". A reconnect counts as a new connection and costs a fresh unit. — [Pricing](https://www.browserless.io/pricing)
  - Idle time counts. Several tabs in one session do not multiply units. — [Unit consumption](https://docs.browserless.io/overview/unit-consumption)
- **Proxies and captcha:** residential 6 units per MB, datacenter 2 units per MB. Third-party proxies are not metered. Captcha costs 10 units per successful solve; failed attempts are free. — [Unit consumption](https://docs.browserless.io/overview/unit-consumption)
- **Limits by plan, Free / Prototyping / Starter / Scale:**
  - Concurrency 2 / 10 (+5) / 40 (+10) / 100 (+20).
  - Maximum session 2 / 15 / 30 / 60 min.
  - Replays kept 1 / 7 / 30 / 90 days.
  - — [Pricing](https://www.browserless.io/pricing)
- **Feature gates:** screen recording from Prototyping. Enterprise adds SOC 2 Type II, a HIPAA BAA, SSO, EU datacenters, GPU, and a commercial self-hosting license. — [Pricing](https://www.browserless.io/pricing)
- **Computed:**
  - Captcha costs $20 per 1k at Prototyping overage and $15 per 1k at Scale overage, or $12.50 and $7 per 1k at the included-unit price.
  - Residential traffic costs $12/GB at Prototyping overage and $9/GB at Scale, with 1 GB = 1,000 MB.
  - If the 30% saving is applied exactly, monthly billing would be about $35.71, $200 and $500 (UNVERIFIED inference).

#### Cloudflare Browser Run (formerly Browser Rendering)
- **Plans:** Workers Free includes 10 minutes per day and 3 concurrent browsers. Workers Paid includes 10 hours per month, then $0.09 per hour, and 10 concurrent browsers (the monthly average of daily peaks), then $2.00 per extra browser. — [Pricing](https://developers.cloudflare.com/browser-run/pricing/) (same figures at [the old URL](https://developers.cloudflare.com/browser-rendering/pricing/))
- **Billing:**
  - Daily seconds are summed per billing cycle and rounded to the nearest whole hour.
  - Quick Actions are billed on browser hours only. Browser Sessions are billed on hours plus concurrency.
  - Quick Actions that fail with `waitForTimeout` are not charged.
  - Each response reports browser time in an `X-Browser-Ms-Used` header.
  - — [Pricing](https://developers.cloudflare.com/browser-run/pricing/)
- **Limits:**
  - Paid: 200 concurrent browsers per account, 3 new browsers per second.
  - Free: 1 new browser every 20 s; `/crawl` limited to 5 jobs a day and 100 pages per crawl.
  - Both: 60 s inactivity timeout, `keep_alive` up to 10 minutes, no fixed maximum lifetime while active.
  - — [Limits](https://developers.cloudflare.com/browser-run/limits/)
- **Not documented:** no managed residential proxy offer appears on these pages. The Workers Paid base fee was not re-checked in this pass.

#### Airtop
- **Plans:**
  - Free $0: 1,000 credits, 3 sessions, 1 deployed agent.
  - Starter $29/mo: credit options from 30k to 150k, 3 sessions, 10 agents, integrated proxy.
  - Professional $189/mo: 225k–500k credits, 30 sessions, custom proxy.
  - Enterprise $558/mo: 775k–1.5M credits, 100 sessions, SOC 2 Type 2.
  - Custom: volume pricing.
  - Annual billing gives a "10% discount & all credits upfront".
  - — [Pricing](https://www.airtop.ai/pricing)
- **Credit rules:**
  - Credits renew monthly and unused monthly credits do not roll over.
  - "Airtop does not offer one-off credit purchases."
  - Free gets 1,000 credits a month plus a one-time 30,000-credit signup grant. The pricing page says 10,000, a conflict.
  - — [Billing help](https://www.airtop.ai/help/billing/billing.md)
- **Missing:** the credit cost per browser minute, AI action or GB is not published on the pricing, billing or FAQ pages. — [FAQ](https://www.airtop.ai/help/guides/FAQ.md)

#### Notte
- **Plans:**
  - Free $0: $10 of credit once, 5 concurrent, 15-minute sessions.
  - Developer $20/mo: $20 of credit per cycle, expiring; 25 concurrent; 120-minute sessions.
  - Startup $100/mo: $100 of credit; 100 concurrent; 300-minute sessions; bring-your-own keys and proxies.
  - Enterprise: custom, with SSO, SLAs, HIPAA and on-premise.
  - — [Pricing](https://www.notte.cc/pricing)
- **Usage:**
  - Browser sessions $0.00/hour, charged per minute and rounded up.
  - Browser functions $0.05/h.
  - Residential proxies $10/GB.
  - LLM tokens at the token price, with "no markup".
  - — [Pricing](https://www.notte.cc/pricing); [Docs pricing](https://docs.notte.cc/intro/pricing)
- **Captcha and stealth:** included on Free, Developer and Startup; "Targeted" on Enterprise. — [Pricing](https://www.notte.cc/pricing)

#### Skyvern Cloud (agent only, no raw browser product)
- **Plans:**
  - Free $0: 5,000 credits once (about 170 actions), 1 concurrent run.
  - Hobby $29/mo: 30,000 credits (about 1,200 actions), 10 concurrent.
  - Pro $149/mo: 150,000 credits (about 6,200 actions), 25 concurrent, residential proxy, advanced captcha solver.
  - Enterprise: custom, 100 concurrent.
  - — [Pricing (md)](https://www.skyvern.com/md/pricing.md)
- **Billing and Enterprise extras:** credits do not roll over. Skyvern SMS numbers cost $10 per month on Pro. Enterprise adds self-hosting, BAA, SSO and custom SLAs (FAQ). — [Billing doc](https://skyvern.com/docs/cloud/account-settings/billing-usage); [Pricing (md)](https://www.skyvern.com/md/pricing.md)

#### Lightpanda Cloud
- **Plans:**
  - Explorer $0: 10 h per month, 5 concurrent.
  - Builder $19/mo: 300 h, then $0.08/h, 30 concurrent.
  - Proxies are bring-your-own.
  - Enterprise: SLA, on-premise or private cloud, no figures.
  - — [Pricing](https://lightpanda.io/pricing)
- **Vendor claim:** "Lightpanda is more efficient. So we give you more hours at the same price point." The engine is not Chromium. — [Pricing](https://lightpanda.io/pricing)

#### Bright Data Browser API (Scraping Browser)
- **Plans:**
  - Free tier: 5K credits and 1 GB per month.
  - Pay as you go: $8/GB.
  - Scale: $499 per month for 71 GB.
  - Enterprise: custom, with a premium SLA, SSO and audit logs.
  - — [Pricing](https://brightdata.com/pricing/scraping-browser)
- **Terms:**
  - There is no per-hour charge.
  - Concurrent sessions are unlimited.
  - "CAPTCHA Solving & JS Rendering" are included.
  - The first deposit is matched up to $500.
  - USD pricing is binding.
  - — [Pricing](https://brightdata.com/pricing/scraping-browser)
- **Computed:** Scale works out to about $7.03/GB.

#### ZenRows (Browser Sessions inside the unified credit plans)
- **Browser Sessions cost** "(GB used × 25,000) + (session minutes × 5)" credits. Free allows at most 0.2 GB of browser use. — [Docs pricing](https://docs.zenrows.com/first-steps/pricing)
- **Plans**, monthly / annual price per month:
  - Build: $19 / $16 for 45k credits, $29 / $24 for 80k, $39 / $32 for 120k.
  - Launch: $69 / $57 for 250k, $99 / $82 for 500k, $129 / $107 for 700k.
  - Growth: $199 / $165 for 1.2M, $279 / $232 for 2M, $399 / $331 for 3M.
  - Scale: $549 / $456 for 5M, $749 / $622 for 8M, $999 / $829 for 12.5M, $1,999 / $1,666 for 28M, $2,999 / $2,499 for 45M.
  - Annual billing gives two months free.
  - Concurrency: Free 5, Build 20, Launch 50, Growth 100, Scale 200–400, Enterprise 500–1,000+.
  - — [Pricing](https://www.zenrows.com/pricing)
- **Computed per GB:** $10.56 on Build B1, $1.67 on Scale S5 billed monthly, $1.39 on S5 billed annually.

#### Scrapeless Scraping Browser
- **Rates:** "$0.09 per browser hour" and "$1.80 per GB of residential proxy". — [Product page](https://www.scrapeless.com/en/product/scraping-browser)
- **Terms:**
  - Captcha solving is "Included free — no metering".
  - The free tier has 1 concurrent session and 1 hour per month.
  - Paid plans offer "Thousands of concurrent sessions".
  - Yearly billing saves 10%.
  - — [Product page](https://www.scrapeless.com/en/product/scraping-browser)
- **Billing:** prepaid compute units with no carry-over, and KYC to unlock higher thread limits. The plan fees did not render. — [Pricing](https://www.scrapeless.com/en/pricing)
- **Entry fee (UNVERIFIED, third-party):** $49/mo. — [Prospeo](https://prospeo.io/s/scrapeless-pricing-reviews-pros-and-cons)

#### Oxylabs "Agent Browser" (its unblocking-browser URL now shows this name)
- **Monthly plans:** Starter $30, Basic $100, Flexible $500, Enterprise "starting at $2,500/month". — [Pricing](https://oxylabs.io/pricing/unblocking-browser)
- **Per-GB tiers:** $8 up to 3 GB, $7.5 up to 13 GB, $5.5 up to 90 GB, $4 from 625 GB. A $5 welcome credit is offered, and VAT may apply. — [Pricing](https://oxylabs.io/pricing/unblocking-browser)

#### Decodo
- **No browser product:** the pricing page shows no hosted, scraping or remote browser. — [Pricing](https://decodo.com/pricing)
- **Residential proxies:** $4/GB pay as you go; the smallest plan is 3 GB at $3.75/GB ($11.25 + VAT). — [Pricing](https://decodo.com/pricing)
- **Site Unblocker (third-party, conflicting, UNVERIFIED):** prices between $0.95 and $1.60 per 1K requests. — [Proxidize](https://proxidize.com/blog/decodo-pricing/)

#### BrowserCat
- **Plans:**
  - Hobby $0: 1,000 credits, overage $0.0025 per credit.
  - Business $50/mo: 10k–1M credits, selectable; overage $0.00175.
  - Credit packs, paid in USDC on Base through Stripe: $20 for 10k, $87.50 for 50k, $300 for 200k.
  - — [Pricing](https://www.browsercat.com/pricing)
- **Credit:** 1 credit is 30 seconds of WebSocket activity. — [Pricing](https://www.browsercat.com/pricing)
- **Proxies:** bringing your own is free. Managed proxies are "on the way", with no price yet. — [Pricing](https://www.browsercat.com/pricing)

#### Rebrowser
- **Plans:**
  - Starter $49/mo: $0.2–0.5 per browser-hour, $0–10/GB, 1 concurrent browser, 5 persistent profiles, captcha "Detection".
  - Professional $149: 5 concurrent.
  - Business $499: 20 concurrent, captcha "Solver".
  - Enterprise $999: $0.17–0.425/h, $0–8.5/GB, 100 concurrent.
  - — [Pricing](https://rebrowser.net/pricing)
- **Billing:** sessions are "rounded up to the nearest 5 seconds". Monthly funds do not roll over. The free trial is worth $49. — [Pricing](https://rebrowser.net/pricing)

#### Cloud browsers from anti-detect vendors (brief)
- **Multilogin:** priced per profile, not per browser-hour.
  - Pro 10: $11/mo billed monthly, or $7.08/mo billed annually ($85 a year), with 1 GB of proxy traffic.
  - Business 100: $40/mo billed monthly.
  - Proxy traffic from $2.275/GB when bought in bulk.
  - No browser-hour price is given for cloud profiles, and the page contradicts itself on API access for the Free plan.
  - — [Pricing](https://multilogin.com/pricing/)
- **GoLogin:**
  - Profile plans are discounted "Annually 50% off", for example Professional 100 at $19.50/mo against a $39 list price. — [Pricing](https://gologin.com/pricing/)
  - **Separate Cloud Browser plans:**
    - Free: 15 minutes.
    - Professional $4.50/mo: 50 h, 1 parallel session, $0.09/h overage.
    - Business $8/mo: 100 h, 2 parallel, $0.08/h.
    - Enterprise $14/mo: 200 h, 3 parallel, $0.07/h.
    - Each extra parallel session costs $2 a month.
    - Residential IPs are included, and traffic beyond the allowance costs $1/GB. The allowance is not stated.
    - — [Cloud Browser](https://gologin.com/cloud-browser/)

#### Sandboxes sold for browser or computer use (brief)
- **E2B:** none of these sandboxes has browser-specific pricing. — [Pricing](https://e2b.dev/pricing)
  - Hobby $0: $100 of credit once, 20 concurrent, 1-hour sessions.
  - Pro $150/mo + usage: 100 concurrent, 24-hour sessions.
  - Concurrency add-ons: Pro+ at $650/mo (600 concurrent) and Pro++ at $1,150/mo (1,100).
  - Enterprise: a $3,000/mo minimum.
  - Rates: $0.000014 per vCPU-second and $0.0000045 per GiB-second.
  - The default sandbox (2 vCPU, 4 GiB) costs about $0.1656/h.
- **Vercel Sandbox (Pro):** — [Pricing, last_updated 2026-09-10](https://vercel.com/docs/vercel-sandbox/pricing)
  - Active CPU $0.128/h; I/O waits are not billed.
  - Memory $0.0212 per GB-hour, with 2 GB per vCPU and a one-minute minimum.
  - Creations $0.60 per million.
  - Downloads are free. Outbound traffic and exposed-port traffic are billable: inside Flat Rate CDN on Pro, $0.15/GB on Enterprise.
  - Maximum session 45 minutes on Hobby and 24 hours on Pro.
  - Concurrency 10 on Hobby and 10,000 on Pro.
- **Daytona:** — [Pricing](https://www.daytona.io/pricing)
  - $0.0504 per vCPU-hour and $0.0162 per GiB-hour of memory, billed per second.
  - Storage $0.000108 per GiB-hour after 5 free GiB.
  - $200 of free compute; startups get up to $50k of credit.

#### Hosted computer use from OpenAI, Anthropic and Google
- **OpenAI:**
  - `computer-use-preview` costs $3 per million input tokens and $12 per million output tokens (snapshot `computer-use-preview-2025-03-11`). — [Model page](https://developers.openai.com/api/docs/models/computer-use-preview)
  - For the computer tool, "You provide the environment". — [Guide](https://developers.openai.com/api/docs/guides/tools-computer-use)
  - Hosted containers (for code, not browsers) cost $0.03 (1 GB), $0.12 (4 GB), $0.48 (16 GB) or $1.92 (64 GB) per 20-minute session, billed by the minute with a 5-minute minimum. — [Pricing](https://developers.openai.com/api/docs/pricing)
- **Anthropic:**
  - Declaring `computer_toolset_20260801` adds about 4,500 input tokens to a request; `browser_toolset_20260801` adds about 6,600. — [Pricing](https://platform.claude.com/docs/en/about-claude/pricing)
  - Screenshots are "roughly 1,000–1,800 input tokens each", and the developer runs the environment. — [Computer use doc](https://platform.claude.com/docs/en/agents-and-tools/tool-use/computer-use-tool)
  - The browser toolset is generally available, and "nothing runs on Anthropic's side"; it "isn't currently available in Claude Managed Agents". — [Browser use doc](https://platform.claude.com/docs/en/agents-and-tools/tool-use/browser-use-tool)
  - Managed Agents bill tokens plus "$0.08 per session-hour" of runtime. Code execution costs $0.05 per container-hour after 1,550 free hours a month. — [Pricing](https://platform.claude.com/docs/en/about-claude/pricing)
- **Google:**
  - Computer use is "Charged as regular tokens per model pricing". Gemini 3.8 Flash costs $0.75 / $3.75 per million tokens through Dec 31, 2026, then $1.50 / $7.50 from Jan 1, 2027.
  - Computer use has no free tier.
  - For managed agents, environment compute is "not billed during the preview period".
  - — [Pricing](https://ai.google.dev/gemini-api/docs/pricing)

#### Adjacent services with a browser-time unit
- **Firecrawl:**
  - Hobby $19/mo ($16 annual): 5,000 credits, 5 concurrent browsers.
  - Standard $83/mo (annual): 100k credits, 25 concurrent.
  - Growth $333 (annual): 500k credits, 50 concurrent.
  - Scale $599 (annual): 1M credits, 100 concurrent.
  - "Interact" (browser sessions) costs 2 credits per browser-minute.
  - — [Pricing](https://www.firecrawl.dev/pricing)
- **TinyFish:**
  - Pay as you go only: a $10 minimum deposit and $8 of starting funds.
  - Browser $0.002 per minute; agent $0.016 per step with the LLM included ("No key to bring").
  - "Residential proxies are included in Agent and Browser runs".
  - Concurrency: 2 agent runs and 5 browser sessions.
  - Enterprise: a 99.99% uptime SLA.
  - — [Pricing](https://www.tinyfish.ai/pricing)
- **Scrapfly:** credit plans run from Discovery at $30 for 200k credits to Enterprise at $500 for 5.5M. Cloud Browser rates were not shown in the captured page. — [Pricing](https://scrapfly.io/pricing)

### Inferences
- **The headline browser-hour is no longer where vendors compete.** On entry plans it sits at $0.00–0.12 for most vendors. Upgrades are driven instead by concurrency (3–25 on entry plans), session length (15 min to 6 h on entry plans, 24 h only at Steel Enterprise, E2B Pro and Vercel Pro, 72 h at Kernel), and stealth or identity tiers.
- **The strongest stealth is gated to enterprise or upper plans** at Steel, Hyperbrowser (ultra stealth), Anchor (full stealth from Growth, unique fingerprinting on Enterprise) and Browserbase (Advanced or Verified at Scale). A stealth Chromium offered on self-serve plans would be a visible difference for Goodfetch Cloud.
- **Bot identity is becoming a priced tier:** Browserbase Verified, Anchor's Cloudflare-verified agents, and Kernel's Web Bot Auth token on Startup and Enterprise plans ([Kernel docs index](https://www.kernel.sh/docs/llms.txt)).
- **No service in the sample charges separately for recording or live view.** Retention (1–90 days) and zero data retention are the levers instead.
- **Only two vendors publish an SLA figure,** both for enterprise: Kernel 99.9% and TinyFish 99.99%.

### Gaps
- **Hyperbrowser:** official plan fees, concurrency and data retention; the page renders with JavaScript, so only third-party figures exist.
- **Browserless:** monthly-billing plan prices; only annual prices are shown.
- **Airtop:** credit cost per browser minute, AI action, proxy GB and captcha.
- **Scrapeless:** plan fees and per-plan concurrency.
- **Browserbase:** price of agent runs beyond those included; Director pricing; whether a paid-plan session can exceed 6 h.
- **Missing limits:**
  - Bright Data Browser API: session duration and idle limits.
  - Steel: idle timeout.
  - Kernel: creation rate per plan.
  - Hyperbrowser: maximum session.
  - Firecrawl: whether "interact" includes proxies.
  - Lightpanda: whether Chromium is offered next to its engine.
  - Scrapfly: Cloud Browser rates.
- **Billing periods and mappings:**
  - GoLogin: whether its Cloud Browser prices are monthly or annual-equivalent.
  - Cloudflare: the Workers Paid base fee was not re-checked.
  - Oxylabs: the mapping of plans to per-GB tiers is an inference (see section 3).

## 2. Agent offerings: price per task, step or minute; LLM pass-through or markup; included models

### Takeaway
Three pricing models coexist:
- **Token pass-through with a disclosed markup:** 0% at Notte and Browserbase's Model Gateway, about 5% at Hyperbrowser (inferred), and 20% at Browser Use, which also applies when you bring your own key.
- **A flat price per step that includes the LLM:** $0.016 at TinyFish, $0.02 for Hyperbrowser's HyperAgent, about $0.024 per action at Skyvern, and $0.01–0.10 at Anchor depending on the model class.
- **Included runs:** Browserbase includes 3, 15 or 50 agent runs per plan and does not publish the price of more.

OpenAI, Anthropic and Google bill computer use as plain tokens and leave the browser to the developer.

### Cited Findings
- **Browser Use:**
  - Agents cost "Model cost + 20%", plus browser time and traffic. The listed token prices already include the 20%. — [Pricing](https://browser-use.com/pricing)
  - Prices per million tokens (input / output): Claude Sonnet 5 $2.40 / $12.00, Claude Opus 5 $6.00 / $30.00, GPT-5.6 $6.00 / $36.00, GPT-5.6 Luna $0.24 / $1.44, Gemini 3.6 Flash $1.80 / $9.00, Grok 4.5 $2.40 / $7.20, GPT-6 Astra $12.00 / $60.00, MiniMax M3 $0.36 / $1.44.
  - Its own BU 2.0 model costs $0.60 input, $0.06 cached input and $3.50 output per million.
  - With your own key, "Browser Use charges 20% of published token rates (including cached rates)".
  - Published benchmark costs per task include GPT-5.6 Luna (default) at $0.056 and Claude Opus 5.5 at $4.215 (a lower bound). The page says these are benchmark-recorded costs, not Cloud prices.
- **Hyperbrowser:**
  - "HyperAgent: 20 credits per step ($0.02/step)" and "Browser Use: 20 credits per step". Browser sessions used by agents are billed at the normal session rates. — [Docs pricing](https://hyperbrowser.ai/docs/pricing)
  - Computer-use agents from Claude, OpenAI, Gemini, Meta, Grok and Jev are "billed per token". Examples per million tokens: computer-use-preview $3.15 / $12.60, Claude Haiku 4.5 $1.05 / $5.25, Sonnet 5.5 $2.10 / $10.50, Opus 5.5 $4.20 / $21.00, "Sonnet 3.7–5" $3.15 / $15.75, Gemini 3.6–3.8 Flash $1.575 / $7.875, grok-4.7 $2.10 / $6.30. — [Docs pricing](https://hyperbrowser.ai/docs/pricing)
- **Anchor:**
  - AI steps, effective August 21, 2026: Light $0.01 per step (for example GPT-4.1 Mini, Gemini 2.5 Flash Lite); Standard $0.05 (Claude Haiku 4.5, GPT-5, Gemini 3 Flash Preview); High Performance $0.10 (Claude Sonnet 4.6, GPT-5.4, Gemini 3.1 Pro).
  - With your own key, a flat $0.01 per step.
  - 0.1 credits per completed task.
  - On Free and Starter: at most 120 AI steps and 10M tokens per task, and 1B AI tokens a month.
  - — [Docs pricing](https://docs.anchorbrowser.io/pricing.md)
- **TinyFish:** the agent costs $0.016 per step, with the LLM included and residential proxies included. — [Pricing](https://www.tinyfish.ai/pricing)
- **Skyvern:**
  - The billing doc says each AI browser action, navigation included, consumes credits, "one discrete browser action" per credit. Examples: a login takes about 3, a 10-field form about 11, a 5-page extraction about 15–25. — [Billing doc](https://skyvern.com/docs/cloud/account-settings/billing-usage)
  - The plan table equates 30,000 credits with about 1,200 actions and 150,000 with about 6,200. The FAQ says Skyvern "does not charge per page or per step — only per workflow execution". — [Pricing (md)](https://www.skyvern.com/md/pricing.md)
- **Notte:** LLM usage is billed at the token price with "no markup".
  - Worked example: 1 minute, 10 steps, Chrome with US proxies. LLM: Gemini 2.0 Flash, 112k tokens, about $0.015. Browser: $0.00. Proxy: 10 MB, $0.10. Total about $0.115. — [Docs pricing](https://docs.notte.cc/intro/pricing)
  - The same page compares itself with "HyperAgent: $0.02 per step", "Browser Use: $0.02 per step", OpenAI CUA at $3.15 / $12.60 and Claude computer use at $3.15 / $15.75. — [Docs pricing](https://docs.notte.cc/intro/pricing)
- **Browserbase:**
  - Agents are asynchronous natural-language runs built on Stagehand. The overview page gives no billing model, model names or maximum duration. — [Agents overview](https://docs.browserbase.com/platform/agents/overview.md)
  - The plans include 3 / 15 / 50 agent calls. — [Plans doc](https://docs.browserbase.com/account/billing/plans.md)
  - Model Gateway "tokens are billed at market price, the same rate you'd pay going direct to the provider". Examples: openai/gpt-5, anthropic/claude-sonnet-4-6, google/gemini-2.5-flash. — [Model Gateway](https://docs.browserbase.com/platform/model-gateway/overview.md)
  - Model Router (`model: "auto"`, July 28, 2026) claims "30–40% lower inference cost". — [Changelog](https://www.browserbase.com/changelog)
- **Airtop:** credits are consumed by agents and sessions, but no rate per action is published. — [Billing help](https://www.airtop.ai/help/billing/billing.md)
- **Anthropic list prices** per million tokens, input / output:
  - Haiku 5.5: $0.10 / $0.50 for prompts up to 100k tokens.
  - Sonnet 5: $2 / $10. The introductory price became standard, and the increase to $3 / $15 planned for Sept 1, 2026 "will not occur".
  - Sonnet 5.5: $2 / $10. Opus 5.5: $4 / $20. Opus 5: $5 / $25.
  - — [Pricing](https://platform.claude.com/docs/en/about-claude/pricing)
- **OpenAI:** computer-use-preview at $3 / $12. — [Model page](https://developers.openai.com/api/docs/models/computer-use-preview)
- **Google:** Gemini 3.8 Flash at $0.75 / $3.75. — [Pricing](https://ai.google.dev/gemini-api/docs/pricing)

### Inferences
- **Hyperbrowser marks up model tokens by about 5%.** The computed ratios are exact: $3.15 / $3.00, $12.60 / $12.00, $1.05 / $1.00, $4.20 / $4.00, $2.10 / $2.00. Its "Sonnet 3.7–5" row ($3.15 / $15.75) prices Sonnet 5 57.5% above Anthropic's current $2 / $10 (computed), which suggests a stale row.
- **Flat per-step prices of $0.016–0.02 that include the LLM** are below the computed Sonnet 5 token cost of a step at 10k input and 1k output tokens ($0.03). Those vendors probably rely on cheaper models, caching or short contexts, and they carry the model-cost risk. Anchor's three-tier per-step price makes that risk explicit.
- **Bringing your own key is not free on every platform.** Browser Use charges 20% of the token rates, Anchor $0.01 per step, while Notte and Browserbase's Model Gateway add no markup. Goodfetch's engine meters tokens per model, which supports any of these models, including a disclosed markup.
- **None of the three model providers hosts a browser for API customers.** Hosted-browser vendors remain the execution layer for OpenAI, Anthropic and Google computer use. Anthropic's $0.08 per session-hour Managed Agents runtime, which has no browser, is a reference price for agent runtime alone.

### Gaps
- Browserbase's price per agent run beyond the included ones, and its agent model.
- Airtop's credit cost per AI action or agent run.
- Whether HyperAgent's $0.02 per step includes the LLM. Inferred yes, because Hyperbrowser bills only the computer-use agents per token.
- Whether TinyFish's agent price also includes browser minutes.
- Skyvern's real credits per action: about 24–25 by the plan table, 1 by the billing doc.
- Which models Skyvern and TinyFish use.
- Current OpenAI computer-use models (the computer-use guide uses gpt-6.1-sol in its examples) and their official prices; the OpenAI pricing page did not list computer-use-preview.

## 3. Normalized comparison: $/browser-hour, with 1 GB residential per hour, and per 10-step agent task

### Takeaway
On entry plans, a bare browser-hour costs $0.00–0.48 at the margin, but adding 1 GB of residential traffic per hour puts most bills at $5–12 an hour, with proxies 92–100% of the total. The exceptions bundle proxies (Kernel $0.06–0.48, TinyFish $0.12) or price them low (GoLogin $1.07–1.09, Scrapeless $1.89, ZenRows Scale $1.41–1.69). A 10-step agent task costs about $0.02–0.77 on token-priced offers with a light profile, $0.16–1.19 on flat per-step offers, and up to about $2.21 with heavy token use. The model dominates the task cost, residential traffic comes second, and browser time is under 1%.

### Cited Findings

**Method (computed from the rates cited in section 1):**
- **"Marginal"** is the price of one more hour on that plan: the usage or overage rate, or for credit plans the plan's price per credit.
- **"At allowance"** is the plan fee divided by the included hours.
- **Scenario B** adds 1 GB of the vendor's residential proxy to each hour.
- Browserless MB rates use 1 GB = 1,000 MB.
- The largest self-serve plan is the biggest plan bought without sales.

**Table N1/N2: $/browser-hour (A) and with 1 GB residential per hour (B)**

| Service | Entry paid plan | A marginal | A at allowance | B (+1 GB) | Largest self-serve plan | A marginal | B (+1 GB) | Rate sources |
|---|---|---|---|---|---|---|---|---|
| Browserbase | Developer $20/mo | $0.12 | $0.20 | $12.12 | Startup $99/mo | $0.10 ($0.198 at allowance) | $10.10 | [plans](https://docs.browserbase.com/account/billing/plans.md) |
| Browser Use | Pay as you go | $0.02 | n/a | $5.02 | same rates | $0.02 | $5.02 | [pricing](https://browser-use.com/pricing) |
| Steel | Launch $0 + usage | $0.10 | n/a | $10.10 | Scale $250/mo | $0.08 | $6.08 | [limits](https://docs.steel.dev/overview/pricinglimits) |
| Hyperbrowser | Startup $30/mo (fee unverified) | $0.10 | $0.10 | $10.10 | Scale $100/mo (fee unverified) | $0.10 | $10.10 | [docs](https://hyperbrowser.ai/docs/pricing) |
| Kernel headless / headful | Hobbyist $30/mo | $0.06 / $0.48 | n/a | same as A (proxies unmetered) | Start-Up $200/mo | $0.06 / $0.48 | same as A | [pricing doc](https://www.kernel.sh/docs/info/pricing) |
| Anchor (docs rates) | Starter $50/mo | $0.06 with one session per hour; $0.65 with 60 one-minute sessions | n/a | $8.06 | Growth $2,000/mo | same | same | [docs](https://docs.anchorbrowser.io/pricing.md) |
| Browserless | Prototyping $25/mo (annual) | $0.24 | $0.15 | $12.24 | Scale $350/mo (annual) | $0.18 ($0.084 at allowance) | $9.18 | [pricing](https://www.browserless.io/pricing) |
| Cloudflare Browser Run | Workers Paid | $0.09 (+$2/mo per average concurrent browser above 10) | n/a | n/a (no proxy offer) | same | $0.09 | n/a | [pricing](https://developers.cloudflare.com/browser-run/pricing/) |
| Notte (sessions) | Developer $20/mo | $0.00 | n/a | $10.00 | Startup $100/mo | $0.00 | $10.00 | [docs](https://docs.notte.cc/intro/pricing) |
| Lightpanda | Builder $19/mo | $0.08 | $0.063 | proxy is BYO | Builder | $0.08 | BYO | [pricing](https://lightpanda.io/pricing) |
| Scrapeless | fees not published | $0.09 | n/a | $1.89 | n/a | $0.09 | $1.89 | [product](https://www.scrapeless.com/en/product/scraping-browser) |
| ZenRows | Build B1 $19/mo | $0.127 | n/a | $10.68 | Scale S5 $2,999/mo | $0.020 (annual $0.017) | $1.69 (annual $1.41) | [docs](https://docs.zenrows.com/first-steps/pricing), [plans](https://www.zenrows.com/pricing) |
| Bright Data Browser API | Pay as you go | none (billed per GB) | n/a | $8.00 | Scale $499/mo (71 GB) | none | $7.03 | [pricing](https://brightdata.com/pricing/scraping-browser) |
| Oxylabs Agent Browser | Starter $30/mo | none (billed per GB) | n/a | $8.00 | Flexible $500/mo | none | $5.50 | [pricing](https://oxylabs.io/pricing/unblocking-browser) |
| TinyFish | Pay as you go | $0.12 | n/a | $0.12 (residential included) | same | $0.12 | $0.12 | [pricing](https://www.tinyfish.ai/pricing) |
| GoLogin Cloud Browser | Professional $4.50/mo | $0.09 | $0.09 | $1.09 once the allowance is used | Enterprise $14/mo | $0.07 ($0.07 at allowance) | $1.07 | [cloud browser](https://gologin.com/cloud-browser/) |
| BrowserCat | Business $50/mo | $0.21 (Scale pack $0.18) | n/a | BYO proxy only | packs | $0.18 | BYO | [pricing](https://www.browsercat.com/pricing) |
| Rebrowser | Starter $49/mo | $0.20–0.50 | n/a | $0.20–10.50 | Enterprise $999/mo | $0.17–0.425 | $0.17–8.93 | [pricing](https://rebrowser.net/pricing) |
| Firecrawl (interact) | Hobby $19/mo | $0.456 | $0.456 | proxy not documented | Scale $599/mo (annual) | $0.072 | n/a | [pricing](https://www.firecrawl.dev/pricing) |
| E2B or Daytona sandbox, 2 vCPU / 4 GiB | usage | $0.166 | n/a | no proxy | same | $0.166 | n/a | [E2B](https://e2b.dev/pricing), [Daytona](https://www.daytona.io/pricing) |
| Vercel Sandbox, 2 vCPU / 4 GB | Pro | $0.11 at 10% CPU, $0.16 at 30%, $0.34 at 100% | n/a | no proxy | same | same | n/a | [pricing](https://vercel.com/docs/vercel-sandbox/pricing) |

Airtop and Skyvern cannot be normalized per hour: Airtop publishes no credit cost per minute, and Skyvern sells only agent actions.

**Plan-to-tier inference for Oxylabs:** each plan fee divided by its per-GB rate lands on a tier boundary. $30 / $8 = 3.75 GB, $100 / $7.5 = 13.3 GB, $500 / $5.5 = 90.9 GB, $2,500 / $4 = 625 GB. — computed from [Oxylabs pricing](https://oxylabs.io/pricing/unblocking-browser)

**Table N3: a standardized monthly workload of 1,000 browser-hours, with no proxy (A) and with 1,000 GB of residential traffic (B)**

The table assumes the plan's concurrency covers about 1.4 browsers on average. Inputs are the section 1 rates.

| Plan | Total A | $/h A | Total B | $/h B |
|---|---|---|---|---|
| Browser Use (pay as you go) | $20 | $0.02 | $5,020 | $5.02 |
| Notte Developer | $20 | $0.02 | $10,000 | $10.00 |
| Anchor Starter, one session per hour | $60 | $0.06 | $8,060 | $8.06 |
| GoLogin Cloud Browser Enterprise | $70 | $0.07 | traffic allowance unknown | n/a |
| Lightpanda Builder | $75 | $0.075 | BYO proxy | n/a |
| Kernel Hobbyist, headless | $80 | $0.08 | $80 | $0.08 |
| Scrapeless (usage only) | $90 | $0.09 | $1,890 | $1.89 |
| Cloudflare (+$5 Workers Paid fee, assumed; average concurrency 10 or less) | $94.10 | $0.094 | n/a | n/a |
| ZenRows (cheapest rung that covers the credits: L2 for A, S4 for B) | $99 | $0.099 | $1,999 | $2.00 |
| Steel Launch | $100 | $0.10 | $10,100 | $10.10 |
| Hyperbrowser | $100 | $0.10 | $10,100 | $10.10 |
| TinyFish | $120 | $0.12 | $120 | $0.12 |
| Browserbase Developer | $128 | $0.128 | $12,116 | $12.12 |
| Browserless Starter (annual price) | $140 | $0.14 | $10,238 | $10.24 |
| Browserbase Startup | $149 | $0.149 | $10,099 | $10.10 |
| Cloudflare, average daily peak of 50 browsers | $174.10 | $0.174 | n/a | n/a |
| Kernel Start-Up, headless | $210 | $0.21 | $210 | $0.21 |
| Browserless Prototyping (annual price) | $225 | $0.225 | $12,225 | $12.23 |
| Steel Scale | $250 | $0.25 | $6,230 | $6.23 |
| Firecrawl Growth (annual; 120k credits needed) | $333 | $0.333 | n/a | n/a |
| Browserless Scale (annual price) | $350 | $0.35 | $8,780 | $8.78 |
| Kernel Hobbyist, headful | $500 | $0.50 | $500 | $0.50 |
| Kernel Start-Up, headful | $630 | $0.63 | $630 | $0.63 |
| Bright Data, pay as you go | n/a | n/a | $8,000 | $8.00 |
| Oxylabs at the $4/GB tier (Enterprise, $2,500/mo minimum) | n/a | n/a | $4,000 | $4.00 |

**Break-even points (computed):**
- **Steel Scale vs Launch:** for time-only use, Scale is cheaper only above 7,500 h a month. When each hour carries 1 GB, Scale is cheaper above about 37 h a month.
- **Browserbase:** at 1,000 h a month without proxy, Developer ($128) is cheaper than Startup ($149). Startup buys concurrency (100 instead of 25), not lower cost.

**Table N4: section 7's example refreshed, 10,000 tasks × 1 browser-minute (166.67 h) with 5 GB of residential traffic in total**

| Plan | Cost |
|---|---|
| GoLogin Cloud Browser Enterprise (traffic allowance unknown) | $14.00 |
| Lightpanda Builder (plus BYO proxy) | $19.00 |
| Cloudflare (+$5 assumed; no proxy offer) | $19.13 |
| TinyFish (residential included) | $20.00 |
| Scrapeless (plan fee unknown) | $24.00 |
| **Browser Use** | **$28.33**, unchanged since Oct 7 |
| Bright Data, pay as you go | $40.00 |
| Notte Developer | $50.00 |
| Steel Launch | $66.67 |
| Hyperbrowser | $66.67 |
| ZenRows L1 (175k credits) | $69.00 |
| Kernel free plan, headful | $75.00 (headless $5.00) |
| **Browserbase Developer** | **$76.00**, unchanged |
| Browserless Prototyping (50k units) | $85.00 |
| **Browserbase Startup** | **$99.00**, unchanged |
| Kernel Hobbyist, headful | $100.00 |
| Browserless Starter | $140.00 |
| Anchor ($100 of it is the $0.01 per-browser fee) | $148.33 |
| Steel Scale | $250.00 |

**Table N5: a 10-step agent task with 1 browser-minute and 10 MB of residential traffic** (Notte's own worked-example values)

Two token profiles:
- **Light:** 100k input and 10k output tokens, about 10k input tokens per step. This is in line with Notte's 112k-token example ([Notte docs](https://docs.notte.cc/intro/pricing)) and with a corpus report of about 495k tokens over 47 vision steps ([HN, from the repo corpus](https://news.ycombinator.com/item?id=47965066)).
- **Heavy:** three times the tokens, 300k input and 30k output.

Token costs alone, light / heavy, at list prices:
- Claude Haiku 5.5: $0.015 / $0.045.
- Gemini 3.8 Flash: $0.1125 / $0.3375.
- Claude Sonnet 5: $0.30 / $0.90.
- OpenAI computer-use-preview: $0.42 / $1.26.
- Claude Opus 5.5: $0.60 / $1.80.
- Sources: [Anthropic](https://platform.claude.com/docs/en/about-claude/pricing), [Google](https://ai.google.dev/gemini-api/docs/pricing), [OpenAI](https://developers.openai.com/api/docs/models/computer-use-preview).

| Offer (pricing rule) | Light, Sonnet 5 | Other light cases | Heavy, Sonnet 5 |
|---|---|---|---|
| Browser Use (tokens + 20%, $0.02/h, $5/GB) | $0.41 | Haiku 5.5 $0.068; BU 2.0 model $0.145; Opus 5.5 $0.77 | $1.13 (Opus 5.5: $2.21) |
| Browserbase with your own Stagehand loop (Model Gateway at cost, Developer overage) | $0.42 | Haiku $0.137; Opus $0.72 | $1.02 |
| Steel with your own loop (Launch) | $0.40 | Haiku $0.117 | $1.00 |
| Kernel headful with your own loop (proxies unmetered) | $0.31 | Haiku $0.023 | $0.91 |
| Notte (tokens at cost, $0/h, $10/GB) | $0.40 | Haiku $0.115; Notte's own Gemini 2.0 Flash example $0.115 | $1.00 |
| Hyperbrowser computer-use agent (tokens +5%) | $0.42 (at its listed Sonnet 5 rate, $0.57) | Haiku $0.117 | $1.05 |
| Hyperbrowser HyperAgent ($0.02 per step) | $0.30, any model | | $0.30 |
| TinyFish ($0.016 per step, all-in) | $0.16, any model | | $0.16 |
| Skyvern (about 24–25 credits per action) | about $0.24 (Hobby or Pro); $0.01 if 1 credit is 1 action | | same |
| Anchor (per-step tier + $0.01 per browser + $0.05/h + $8/GB) | Standard $0.59 | Light $0.19; High Performance $1.09; own key + Sonnet 5 $0.49; add $0.10 if the per-task credit applies | own key: $1.09 |
| Direct OpenAI, Anthropic or Google API | token cost only, plus the browser you host | | |

### Inferences
- **Proxies dominate any workload that moves real traffic.** At 1 GB per hour, proxies are 92–100% of the bill wherever they are charged (computed).
  - The real comparison is the residential price: $1.80 (Scrapeless), $1.39–1.69 (ZenRows Scale, credit-converted), $5 (Browser Use), $6 (Steel Scale), $8 (Anchor, Bright Data), $10 (Steel Launch, Hyperbrowser, Notte, Browserbase Startup), $12 (Browserbase Developer, Browserless overage).
  - Kernel and TinyFish bundle proxies, with fair-use limits, which is a bet that most sessions carry little traffic.
  - For Goodfetch Cloud, billing proxy bytes separately at a competitive $/GB, with a cheap bring-your-own-proxy egress rate (Browser Use and Anchor charge $0.20/GB), matters more than the browser-second price.
- **The units are not like-for-like:**
  - Kernel bills only active seconds and stops after 5 seconds of idle. Browserbase, Browser Use, Steel, Notte and Anchor bill wall-clock minutes. Browserless and BrowserCat bill 30-second blocks; Rebrowser rounds to 5 seconds.
  - Cloudflare sums seconds and rounds monthly to the hour, but charges for averaged peak concurrency.
  - Bright Data and Oxylabs bill only GB.
  - An agent session that spends most of its time waiting for the LLM is therefore much cheaper per wall-clock hour at Kernel than its $0.48/h headful rate suggests.
  - Goodfetch's browser-second meter can support active-time billing.
- **Fixed per-session fees and minimums distort short tasks.** Anchor's $0.01 per browser raises one-minute sessions to an equivalent $0.65/h, against $0.06/h for one session an hour. One-minute minimums (Browserbase, Browser Use) matter in dollars only for sub-minute jobs at very large volume.
- **Plan fees matter mainly at low volume.** Browserbase Developer's effective rate at its allowance ($0.20/h) is about twice its marginal rate ($0.12/h). Steel Scale's $150 net platform fee pays off only for proxy-heavy use.
- **For agent tasks, browser time is negligible** (under $0.01 per task). The model and tokens per step drive the cost: compare Haiku 5.5 with Opus 5.5, or the light with the heavy profile. Ten MB of residential traffic adds $0.05–0.12, which is 10–40% of a cheap-model task.
  - Flat per-step prices (TinyFish $0.016, HyperAgent $0.02) undercut token pass-through on mid and frontier models and lose to it on very cheap models.

### Gaps
- **Assumptions:** the 1 GB-per-hour and 10 MB-per-task traffic figures are assumptions, not measurements. Tokens per step vary a lot with screenshots, accessibility trees and caching. The input side could fall by about 90% with caching, which was not modelled.
- **Not modelled:** regional pricing, taxes, annual discounts beyond those shown, retries, and failed-run billing. Kernel bills failed app invocations, Cloudflare does not bill `waitForTimeout` failures, and Browserless does not bill failed captcha solves.
- **Unconfirmed inputs:** Hyperbrowser's plan fees, Browserless's monthly prices, Scrapeless's plan fees, the Workers Paid base fee, and GoLogin's traffic allowance.
- **Airtop:** could not be normalized.

## 4. What changed in 2025–2026, and how users react

### Takeaway
2025–2026 brought three kinds of change:
- **Price cuts on time and proxies:** Browser Use cut proxies from $10 to $5/GB on April 3, 2026, and its browser-hour apparently fell from $0.06 to $0.02. Steel's rate card was rebuilt as Launch and Scale with $0.08/h at the top. Notte's browser time fell to $0.00/h.
- **New units:**
  - Cloudflare started billing on August 20, 2025, with averaged concurrency.
  - Kernel bills per second of active time and includes proxies.
  - ZenRows folded its browser into unified credits.
  - Anchor tiers its per-step price by model class from August 21, 2026.
  - Skyvern moved from $0.05 per step to monthly credits.
- **Agent-native payment rails:** x402, MPP and USDC packs.

Users argue about per-minute versus per-millisecond billing, about unpredictable proxy bandwidth, and about agent costs that come from tokens and the harness, not from the browser.

### Cited Findings

**Section 7 of the 2026-10-07 landscape, rechecked:**
- **Browserbase:** unchanged: $20 with 100 h, 25 concurrent and 1 GB; $0.12/h; $12/GB. $99 with 500 h, 100 concurrent and 5 GB; $0.10/h; $10/GB. — [Pricing](https://www.browserbase.com/pricing)
- **Browser Use:** unchanged: $0.02/h, $5/GB residential, $0.20/GB for your own proxy, a one-minute minimum, and agents at model cost + 20%. — [Pricing](https://browser-use.com/pricing)
- **Browserless:** the unit rules are confirmed, and the plan prices missing on Oct 7 are now recorded: $25 / $140 / $350, billed annually. — [Pricing](https://www.browserless.io/pricing)
- **Firecrawl:** Hobby $19 ($16 annual) with 5,000 credits, and "interact" at 2 credits per browser-minute, confirmed. — [Pricing](https://www.firecrawl.dev/pricing)
- The section 7 example still holds: $28.33 / $76 / $99. Table N4 extends it.
- The rows for Apify, Spider, ScrapingBee, Zyte and Scraping Fish are outside this note's scope and were not rechecked.

**Dated changes:**
- **Cloudflare:**
  - 2025-01-31: concurrency raised from 2 to 10.
  - 2025-07-16: billing announced. 2025-08-20: "Billing for Browser Rendering begins today".
  - 2025-09-25: paid limits raised.
  - 2026-03-04: REST rate raised from 3 to 10 requests per second.
  - 2026-03-10: `/crawl` beta.
  - 2026-04-15: the default for Workers Paid raised from 30 to 120 concurrent browsers, and from 30 per minute to 1 per second for new browsers.
  - — [Changelog](https://developers.cloudflare.com/browser-run/changelog/)
  - The limits page now shows 200 concurrent and 3 new browsers per second, under the "Browser Run" name. — [Limits](https://developers.cloudflare.com/browser-run/limits/)
  - The pricing page was "Last updated Apr 21, 2026". — [Pricing](https://developers.cloudflare.com/browser-run/pricing/)
- **Browser Use:**
  - 2026-04-03: "Browser Use Cloud is now free"; "Proxy data cost cut from $10/GB to $5/GB"; custom proxies on all subscription plans. — [Changelog](https://browser-use.com/changelog/3-4-2026)
  - The current model is pay as you go, with legacy subscriptions still manageable. — [Pricing](https://browser-use.com/pricing)
  - UNVERIFIED: undated search-index snapshots of the former docs and pricing pages showed $0.06/h on pay as you go and Starter, and $0.03/h on Business. — [docs URL, now redirecting](https://docs.cloud.browser-use.com/pricing)
  - The edge rate-limit ceiling was raised on September 9, 2026. — [Concurrency doc](https://docs.browser-use.com/cloud/guides/concurrency.md)
- **Steel:** a new Launch / Scale / Enterprise rate card, "Last edited: June 30th, 2026". The legacy Starter, Developer and Startup plans are kept. — [Pricing and limits doc](https://docs.steel.dev/overview/pricinglimits)
- **Anchor:** AI step tiers took effect on August 21, 2026. — [Docs pricing](https://docs.anchorbrowser.io/pricing.md)
- **Notte:**
  - August 2025: after complaints about opaque credits, the vendor said it charged about 1¢ per scrape, 2¢ per agent step and 1¢ per browsing minute, about 21¢ for a 10-step, one-minute task. — [HN, from the repo corpus](https://news.ycombinator.com/item?id=44834998)
  - Now: browser time $0.00/h and tokens at cost. — [Docs pricing](https://docs.notte.cc/intro/pricing)
  - UNVERIFIED: an undated search-index snapshot of Notte's docs showed $0.05/h.
- **Skyvern:** moved from $0.05 per step to monthly credits. This comes from a search summary of the post "Launch Week – Day 5 – Simpler Pricing Model"; the date was not captured. — [Blog](https://www.skyvern.com/blog/launch-week-day-5-simpler-pricing-model/)
- **ZenRows:** Browser Sessions are now 25,000 credits per GB plus 5 credits per minute. — [Docs pricing](https://docs.zenrows.com/first-steps/pricing)
  - UNVERIFIED: an undated search-index snapshot of the same FAQ URL described "$69 per month", "$5.50 per GB" and "$0.09 per hour". — [FAQ URL](https://docs.zenrows.com/scraping-browser/faq/pricing)
- **Browserbase:**
  - 2026 changelog entries: Model Router on July 28 (claims "30–40% lower inference cost"), Stagehand v4 on August 10 ("~80% more token efficient", a vendor claim), configurable caching on August 21 (a cache hit "skips inference entirely"). — [Changelog](https://www.browserbase.com/changelog)
  - UNVERIFIED, undated: Sacra describes an older structure with a 60-minute free tier and plans at $29, $49 and $99. — [Sacra](https://sacra.com/c/browserbase)
- **Kernel:** per-second rates, Standby after 5 seconds idle, unmetered proxies, and MPP payments. — [Pricing doc](https://www.kernel.sh/docs/info/pricing); [Proxies doc](https://kernel.sh/docs/proxies/overview.md)
- **Agent-native payment rails:** Steel x402 — [Docs index](https://docs.steel.dev/llms.txt); BrowserCat USDC packs — [Pricing](https://www.browsercat.com/pricing).
- **Anthropic:** Sonnet 5's introductory price became standard, cancelling the planned Sept 1, 2026 increase. The Managed Agents runtime costs $0.08 per session-hour. The computer and browser toolsets are dated 2026-08-01. — [Pricing](https://platform.claude.com/docs/en/about-claude/pricing)
- **Google:** Gemini 3.8 Flash doubles to $1.50 / $7.50 on January 1, 2027. Managed-agent compute is free during the preview. — [Pricing](https://ai.google.dev/gemini-api/docs/pricing)
- **Airtop:** current Starter / Professional / Enterprise fees are $29 / $189 / $558. — [Pricing](https://www.airtop.ai/pricing)
  - UNVERIFIED: a search-index snapshot showed $26 / $80 / $342. $26 equals $29 less the 10% annual discount (computed); $80 and $342 match no current price.

**User reactions:**
- **HN, Browser Use's article "How we run Firecracker VMs inside EC2 and start browsers in less than 1s" (about June 2026).** — [Thread](https://news.ycombinator.com/item?id=48556561); [Margin comment](https://news.ycombinator.com/item?id=48596203)
  - A screenshot-API operator says Browser Use "bills per minute, not per millisecond like lambda does", and estimates it at about twice Lambda's cost for 5.7-second jobs.
  - Browser Use replies that it bills by the minute because its main use case is web automation, and that per minute "Lambdas are 4-6x more expensive".
  - Shellbox claims to be 2–5 times cheaper.
  - Another commenter asks whether dedicating a physical core per browser on an m8i.large ($0.043/h spot) means the vendor loses money on every browser-hour.
- **HN, Cloudflare Sandbox SDK (about November 2025):** "more than $58/month for just 1vCpu and 1GiB RAM when used continuously"; Hetzner and Contabo are "35+ times cheaper"; serverless suits only sporadic use. — [Thread](https://news.ycombinator.com/item?id=45613653)
- **HN, Cloudflare crawl endpoint (about March 2026):** the free plan gives 10 minutes a day; a commenter computes about 500 pages a day from the `/crawl` limit of 5 jobs × 100 pages. — [Thread](https://news.ycombinator.com/item?id=47330415)
- **Repo corpus items** (`rnd/research/market/corpus.jsonl`, not re-read in this pass):
  - August 2026: Cloudflare's browser API used as a fallback is cheaper than Firecrawl, and search APIs are the bigger expense. — [HN](https://news.ycombinator.com/item?id=49165072)
  - August 2026: the choice is framed as paying for "expensive APIs like Firecrawl or Browserbase" or running RAM-hungry Chrome fleets yourself. — [HN](https://news.ycombinator.com/item?id=49148163)
  - October 2025: Browserbase is "far cheaper than Simplex". — [HN](https://news.ycombinator.com/item?id=45457133)
  - September 2026: the same model costs $1.50 per task in one harness and about $10 in another (vendor benchmark). — [HN](https://news.ycombinator.com/item?id=49833761)
  - January 2025: the LLM costs far more than the browser. — [HN](https://news.ycombinator.com/item?id=42814724)
  - July 2026: residential IPs blow a $50 budget. — [Reddit mirror](https://reddit.sentinel-team.org/posts/1v58c6c/snapshots/2026-07-30T05%3A30%3A58.663945Z)
  - June 2026: "Fewer steps did not mean lower cost." — [Reddit mirror](https://reddit.sentinel-team.org/posts/1uaeb9s/snapshots/2026-06-20T03%3A20%3A10.744219Z)
  - May 2026: replaying a JSON plan cuts the cost of a task from $0.50–3.00 to $0.01–0.05 (vendor). — [Reddit mirror](https://reddit.sentinel-team.org/posts/1tnfgel/snapshots/2026-05-29T19%3A16%3A10.134322Z)
  - April 2026: Bright Data's bandwidth billing makes monthly costs hard to forecast, and trials need a sales call or KYC. — [Review](https://ecommerceparadise.com/bright-data-review-2026/)
- **The repo's synthesis of complaints and requests:**
  - Complaints: credit multipliers, billing for failures, no spending caps, bandwidth billing, expiring credits.
  - Requests: pay as you go, hard caps, success-based units.
  - — [pricing-signals.md](../../market/pricing-signals.md)

### Inferences
- **Browser time is in a price war,** with headline rates of $0.02–0.10 an hour and $0 at Notte. Competition has moved to proxy $/GB, stealth and verified identity, session length and concurrency, and agent bundles. Goodfetch Cloud is unlikely to win on the browser-second price alone.
- **The market keeps moving toward finer, active-time units.** Kernel bills per second with Standby, Cloudflare per second with hourly rounding, Rebrowser in 5-second steps. Per-minute vendors are being challenged on short jobs.
- **Payment rails for agents** (x402, MPP, USDC packs) appeared at Steel, Kernel and BrowserCat in 2026. This is a nascent channel for agents that buy browsers themselves.
- **Users react to unpredictability more than to unit price:** bandwidth, credits and multipliers. Spending caps and per-run cost breakdowns are differentiators the cited complaints ask for.

### Gaps
- **Wayback Machine unreachable:** the fetch tool refused web.archive.org, so the exact dates of these changes are unknown:
  - Browser Use's browser-hour cut ($0.06 → $0.02);
  - Notte's $0.05 → $0.00;
  - ZenRows' move to credits;
  - Skyvern's move to credits;
  - Browserbase's earlier plan structure;
  - Steel's pre-June-2026 rate card.
- **Sources not searched:**
  - Reddit could not be searched directly; only corpus items via mirror URLs were used.
  - X/Twitter was not searched, because the WebSearch budget was exhausted.
  - Community reactions specific to Kernel, Steel's new plans, Anchor's step tiers and Cloudflare's 2025 billing start were not found in the sources reached.
