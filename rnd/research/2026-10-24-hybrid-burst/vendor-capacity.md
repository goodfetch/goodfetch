# Burst capacity: who can render 10M URLs in 1 hour (researched 2026-10-24)

Target: 10,000,000 / 3,600 s = 2,778 pages/s = 166,667 new pages/min. By Little's law, concurrency = rate x page duration: 3 s -> 8,333; 5 s -> 13,889; 10 s -> 27,778 concurrent pages. Browser-hours for the job = 10M x duration / 3,600 = 8,333 / 13,889 / 27,778.

Evidence rules: every figure was read from the cited page on 2026-10-24 unless tagged [unverified] (search snippet or third party only) or [computed]. Pages that carry no date are marked "page undated". Prices already in `pricing_hosted_browsers_and_agents.md` (2026-10-12) were not re-fetched unless noted.

## 1. Table

"Create rate" = new browser sessions (or requests) per unit time. "Batch?" = submit N URLs, get results, vs one CDP/WebSocket session per page.

| Vendor | Max concurrency (plan -> enterprise) | Session / request creation rate | Batch endpoint? | Price per browser-hour (or per-page unit) | Source |
|---|---|---|---|---|---|
| Browserbase | 3 / 25 / 100 / Scale "250+", "fully custom"; vendor claims customers run 6,000+ concurrent | 5 / 25 / 50 / "150+" per min; 429 above | No (CDP session per page; "Fetch" API $1 per 1k, $0.5 on Startup) | $0.12 Dev, $0.10 Startup; Scale custom | docs.browserbase.com/guides/plans-and-pricing; /optimizations/concurrency/overview.md; browserbase.com/blog/what-is-a-browserbase-browser (2026-06-16) |
| Bright Data Browser API | "Unlimited" concurrent sessions (PAYG and Scale) | none published | No for browser (CDP). Web Unlocker / Scraper API: sync request per URL; "Bulk Request Handling" named, undocumented on pricing page | No hourly fee; $8/GB PAYG, Scale $499/mo incl. 71 GB. Web Unlocker $1.5/1k PAYG, $1.3/1k Scale. Scraper API $1.5/1k records | brightdata.com/pricing/scraping-browser, /pricing/web-unlocker, /pricing/web-scraper |
| Zyte API (browserHtml) | Not stated per plan; RPM-limited: 3,000 RPM per API key (50 req/s) Standard, Enterprise custom; browser rendering lowers max concurrency | 3,000 RPM; temporary raise by ticket >=24 h ahead | Request-per-URL HTTP API, no bulk endpoint found | $1.01 / 2.01 / 4.02 / 8.04 / 16.08 per 1k browser responses by tier (PAYG); $0.48-7.68 at $500 tier | docs.zyte.com/zyte-api/usage/rate-limit.html; zyte.com/pricing |
| Cloudflare Browser Run | Paid: 200 concurrent per account (limits page, fetched today); raisable "Cloudflare can increase your account limits" via form. Changelog 2026-04-15 said 120; third party says 200 since 2026-08-20 [unverified] | 3 new browsers/s (Paid), Quick Actions 30 req/s; Free 1 per 20 s | `/crawl` (beta, async): single seed `url`, `limit` up to 100,000 pages per job, results kept 14 days, default 0.5 s delay per domain, `render` default true. Not a URL-list endpoint | $0.09/h after 10 h/mo; +$2/mo per concurrent browser above 10 (sessions only); Quick Actions: hours only | developers.cloudflare.com/browser-run/limits/, /pricing/, /quick-actions/crawl-endpoint/, /changelog/; blog.cloudflare.com/browser-run-containers (2026-05-13) |
| Kernel | 5 / 10 / 150 (Start-Up) / Enterprise custom, "raisable" | No numeric rate published ("create rate caps how fast you create browsers"); browser pools (Start-Up+) bypass create rate; pool fill rate default 10%/min, max 25%/min of pool size [unverified, from docs PR mirror] | Pools (pre-warmed, `acquire`), not URL batch | $0.06/h headless (1 GB), $0.48 headful; billed per second, idle free after 5 s | kernel.sh/docs/browsers/concurrency-and-limits.md; /browsers/pools/overview.md; kernel.sh/pricing |
| Steel | 10 / 100 / Enterprise "1,000+" | 60 / 600 requests per min / custom (API requests; no session rate stated) | No batch. `/scrape`, `/screenshot`, `/pdf` single-URL tools, $5 per 1k | $0.10 Launch, $0.08 Scale | docs.steel.dev/overview/pricinglimits |
| Hyperbrowser | Official plan numbers not retrievable (JS-rendered). Third-party: 25 (Startup), 100 (Scale), 1,000+ Enterprise [unverified]. Homepage: "1,000+" concurrent sessions | not published; docs say "hundreds of browser sessions in seconds" [unverified, search snippet] | Yes: `POST /api/scrape/batch`, `GET /api/scrape/batch/{id}` (API catalog, [unverified]; docs page 404 on fetch). Scrape 1 credit/page ($0.001), 10 with proxy | $0.10/h; scrape $1 per 1k pages | hyperbrowser.ai (home), hyperbrowser.ai/docs/pricing; API catalog via search |
| Browser Use Cloud | 10 ($0) / 50 ($100-200) / 250 ($1k) / 500 ($5k) / 1,000 ($25k lifetime spend); more via support | General request budget max(25, 2 x concurrency) req/s, capped 100 RPS; 1,000 req/s per IP at edge | No | $0.02/h; proxy $5/GB resi, $0.20/GB own | docs.browser-use.com/cloud/guides/concurrency.md; browser-use.com/pricing (via 10-12 notes) |
| Firecrawl | Concurrent browsers: Free 2, Hobby 5, Std 25, Growth 50, Scale/Enterprise "100+" | /scrape 10,000 RPM, /crawl and /batch 2,000 RPM at Scale. Queue up to 200,000+ jobs (2,000 x concurrency, cap 2M) | Yes: batch scrape, async job, uses team's full concurrency, no stated URL cap; results kept 24 h | 1 credit/page (+4 JSON). Scale $599/mo = 1M credits; overage $5 per 5,000 credits at Scale = $1.00/1k | docs.firecrawl.dev/rate-limits, /features/batch-scrape, /billing |
| Spider Cloud | Bought concurrency 2 ($40/mo) to 1,538 ($19,994/mo); more "talk to sales". One in-flight request per unit; excess gets immediate 429 (no server queue) | 10,000 core API requests/min per account on every plan | Docs say "bulk" objects per request; URL-list scrape not confirmed | Per-page: $1/GB bandwidth + $0.0001 per CPU-minute (no JS surcharge); unit $14/mo each at 25-unit tier | spider.cloud/pricing, /docs/api |
| ScrapingBee | 25 -> 900 (Enterprise 120 plan); "custom plans offer additional" | not published | No (one request per URL) | JS rendering = 5 credits [unverified, third-party; basic request 1 credit confirmed]. Enterprise 120: $5,799/mo, 120M credits | scrapingbee.com/#pricing; third-party snippets |
| ZenRows | 5 / 20 / 50 / 100 / Scale 200-400 / Enterprise 500-1,000+ | not published | No | JS rendering 5 credits; $0.33-0.55 per 1k JS requests at Scale rungs [computed by the page fetch, monthly billing] | zenrows.com/pricing |
| ScraperAPI | 20 -> 500 threads; Enterprise "500+" | not published | Async/batch exists in vendor docs but not checked | JS rendering "included"; 1 credit standard page; Advanced $1,975/mo = 21.5M credits | scraperapi.com/pricing |
| Apify | Per account: Free 25, Starter 32, Scale 128, Business 256 concurrent Actor runs; total memory 16 / 64 / 256 / 512 GB | Run Actor endpoint 400 req/s per resource; global 250,000 req/min per user | Yes (Actor with input list; Puppeteer/Playwright crawler many pages per run) | $0.13-0.20 per compute unit (1 GB-hour); Business $999/mo | docs.apify.com/platform/limits; /api/v2 rate limiting; apify.com/pricing |
| AWS Bedrock AgentCore Browser | 1,000 concurrent active sessions per account (adjustable by support ticket); 1 vCPU / 4 GB per session (fixed) | CreateBrowser 5 TPS; StartBrowserSession 30 TPS (adjustable); InvokeBrowser 5 TPS | No (one session per browser; `InvokeBrowser` is per-session) | $0.0895/vCPU-h + $0.00945/GB-h, per second, actual CPU: max about $0.127/h per session (1 vCPU, 4 GB) [computed] | docs.aws.amazon.com/bedrock-agentcore/latest/devguide/bedrock-agentcore-limits.html; aws.amazon.com/bedrock/agentcore/pricing/ |

## 2. Per-vendor notes (what matters for a burst)

**Browserbase.** Published ceiling is Scale "250+" concurrent and "150+" new sessions per minute. Their own example: on Startup "it will take about 2 minutes to spin up all 100 sessions." Exceeding either limit returns 429. The Scale plan is "fully custom"; the enterprise page promises "higher burst rates for peak demand" with no number. The largest primary-source claim anywhere in this survey is theirs: "Customers run 6,000+ concurrent sessions on us today" (blog, 2026-06-16), plus 35M+ sessions per month (enterprise page) and a "<2 seconds" start. 6,000 concurrent still needs 2.3 h (5 s pages) for 10M [computed]. Creation at 150/min would take 1,111 h for 10M one-page sessions [computed], so session reuse is mandatory.

**Bright Data.** Only vendor that states "unlimited" concurrency (Browser API and Web Unlocker pricing pages). No numeric cap and no creation rate is published, so the claim is untested; an unlimited claim on a pricing page is not a capacity commitment. Billing is per GB, not per hour, which makes page weight the main cost driver. Web Unlocker bills per request ($1.3-1.5/1k) and does full browser rendering "included". Browser API FAQ page had no concurrency or session-limit statement.

**Zyte API.** Clearest published throttle in the set: 3,000 RPM (50 req/s) per key on Standard; the page says browser rendering affects maximum concurrency. 2,778 pages/s is 55x that. Temporary raises are by ticket at least 24 h ahead, naming key, RPM, target sites and dates, so Zyte is the only vendor with a documented procedure for a one-off burst; the ceiling for Enterprise is not published. Priced per response, tiered by site difficulty.

**Cloudflare Browser Run.** Limits page (fetched today) shows 200 concurrent, 3 new browsers per second, Quick Actions 30 req/s on Paid; the changelog snapshot I fetched shows the earlier 120 concurrent and 1/s (2026-04-15), and the May 2026 blog said "up to 120 concurrently" and 60 browsers/min via the Workers binding, so the 200/3 figures are a later raise. Cloudflare can raise limits on request. `/crawl` is the only bulk-ish endpoint: async, one seed URL with sitemap/link discovery, up to 100,000 pages per job, per-domain delay (0.5 s default) so one domain cannot be hammered; it cannot take a list of 10M arbitrary URLs. 3 new browsers/s would take 926 h for 10M one-page sessions; with Quick Actions at 30 req/s it takes 92.6 h [computed].

**Kernel.** Start-Up caps at 150 concurrent; Enterprise "custom", "the limit is raisable". Reserved pools give pre-warmed browsers and avoid the create rate, but pool capacity counts against concurrency, pool size is capped by the org limit, and the docs advise "raise the size before a traffic peak rather than during it." The 25%/min max fill rate is from an unmerged docs PR mirror [unverified]; if true, filling a 10,000-browser pool from empty takes at least 4 minutes. Cheapest honest headless rate among CDP vendors ($0.06/h, 1 GB, per-second billing).

**Steel.** Enterprise "1,000+" concurrent; max session 24 h; no creation rate or batch published. `/scrape` tool is per URL at $5 per 1k (that is $50,000 for 10M).

**Hyperbrowser.** Headline claims are the largest on paper ("1,000+" on its homepage; third-party sites repeat "10,000+ instant" and "0 to 5,000 in 30 s" [unverified, not on their own site]). Only vendor besides Firecrawl with a documented batch scrape (`/api/scrape/batch`, [unverified] endpoint name from an API catalog; my fetch of the docs page 404ed). Official plan concurrency not retrievable (JS-rendered pricing page).

**Browser Use Cloud.** Concurrency is bought by lifetime spend: 1,000 sessions after $25,000 cumulative payments. Creation falls under a request budget of max(25, 2 x concurrency) req/s, capped at 100 RPS, so 100 sessions/s max; 2,778/s is out of reach without an override (docs say overrides exist). Cheapest hour ($0.02) but proxy at $5/GB by default.

**Firecrawl.** Best-documented batch semantics: a batch job "uses your team's full concurrent browser limit," jobs queue (up to 200k at Scale, 2,000 x concurrency, cap 2M), queued jobs expire after 48 h. So 10M jobs queue fine but drain at concurrency/duration: 100 concurrent at 5 s = 20 pages/s = 139 h [computed]. Enterprise concurrency is "100+", no ceiling published. Price is simple: 1 credit/page, $1/1k at Scale overage = $10,000 for 10M.

**Spider Cloud.** Two independent ceilings: bought concurrency (max listed 1,538 units, $19,994/mo; one request in flight per unit, instant 429 otherwise) and 10,000 requests/min per account (167/s) on every plan. Pricing is the only usage-based (bandwidth + CPU-minutes) model: "most pages land at a fraction of a cent." Vendor-run benchmark (2026-02-11 snapshot) reported 74 pages/s average vs Firecrawl 16 on their own infra [unverified, self-published]; their About page says "thousands of pages a second on a single host" is a design goal for the open-source crawler.

**ScrapingBee / ZenRows / ScraperAPI.** All sell request concurrency, not browsers, and all top out at published 500-1,000 concurrent (ScrapingBee 900, ZenRows "500 to 1000+", ScraperAPI "500+") before "custom". At 900 and 5 s, 10M takes 15.4 h [computed]. JS rendering is 5x credit cost at ScrapingBee and ZenRows. None publishes a creation rate or a burst commitment.

**Apify.** Hard per-account caps by plan: Business 256 concurrent runs and 512 GB total memory. A single Actor run can drive many pages (Crawlee), so the memory cap, not run count, is the binding limit: 512 GB at roughly 0.5-1 GB per concurrent Chrome page is about 500-1,000 concurrent pages [assumption]. Enterprise terms are not published. Platform API is generous (400 req/s run-start per resource).

**AWS Bedrock AgentCore Browser.** The only hyperscaler-managed option. Default 1,000 concurrent sessions per account and region, adjustable via support ticket; 1 vCPU / 4 GB fixed per session; CreateBrowser 5 TPS but StartBrowserSession 30 TPS (the call that actually creates a session), both adjustable. At 30 TPS, ramping to 13,889 sessions takes 463 s (7.7 min) and 10M one-page sessions take 92.6 h [computed], so session reuse (about 5 pages per session minute) is still needed. No batch endpoint. The quotas page is undated. Multi-region (account x region) multiplies the quota.

## 3. Raw infrastructure

**AWS Lambda.** Documented scaling rate: "for each function, 1,000 execution environments every 10 seconds" per Region (also "10,000 requests per second every 10 seconds" for the request form); it is per function and does not accrue (Lambda scaling-behavior page, fetched 2026-10-24, undated). Account concurrency default: 1,000, "can be increased up to tens of thousands" (Lambda quotas page). New accounts have "reduced concurrency and memory quotas" that AWS raises automatically with usage. Max 10,240 MB memory, 900 s timeout, 1,769 MB = 1 vCPU. Ramp arithmetic: 13,889 environments from one function at 1,000 / 10 s = 139 s; 27,778 = 278 s [computed]. Sharding across N identical functions multiplies the rate (functions scale independently), but the account quota still binds. Cost: x86 $0.0000166667 per GB-s + $0.20 per 1M requests (pricing page fetch). 2 GB x 5 s per page = $1,669 for 10M; 4 GB x 10 s = $6,669 [computed]. Case studies: I found none with numbers. Searches returned only tutorials (adieuadieu/chrome-aws-lambda, @sparticuz/chromium) and an AWS 2021 Puppeteer fan-out pattern; no published "X million pages/day on Lambda Chromium". Treat Lambda-Chromium as a plausible but unproven-at-scale option. Chromium packaging needs a 50 MB+ compressed build and 625 Mbps per-environment network.

**EC2 / Spot.** Default vCPU quotas per the EC2 instance-quota page: Running On-Demand Standard (A,C,D,H,I,M,R,T,Z) = 5 vCPU and All Standard Spot Requests = 5 vCPU; "EC2 automatically increases your quotas based on your usage"; adjustable via Service Quotas. A fleet of 2,800-8,400 vCPUs therefore needs a quota raise first (days of lead time in practice [assumption, no AWS SLA found]). API launch rate: `RunInstances` resource token bucket is 1,000 tokens refilled at 2/s: "you can immediately launch 1000 instances, using any number of API requests... After the resource token bucket is empty, you can launch up to two instances every second" (EC2 API throttling page). So 200-500 instances are API-feasible in one burst; 1,000 more would take 500 s. EC2 Fleet/Spot Fleet defaults (via search snippet of fleet-quotas page [unverified]): target capacity 10,000 units per fleet, 100,000 per Region; instant-type fleets exempt from capacity-pool limits. Spot capacity availability for 200-500 large instances is not documented anywhere I could find; there is no guarantee, diversify across instance types and AZs.

**Hetzner Cloud.** Official default found: 20 projects per account ("default limit of 20 projects per account", Hetzner cloud FAQ, undated). Server, volume, IP and load-balancer limits are not published on the FAQ; they are shown on the "Limits" tab in the Console. Increases: "Request change" -> "Limit increase", "processed manually, case by case, during business hours". One secondhand forum report: a new account was limited to 10 instances and Hetzner raised limits "step by step and on the actual requirement" [unverified, anecdotal]. No documented default for dedicated-vCPU (CCX) cores. Repo context: the owner already rents a CCX43 server by the hour (AGENTS.md, EUR 0.531/h); that account's current limit was not checked.

## 4. Could any vendor do 10M browser pages in 1 h, and at what cost?

**Short answer: no vendor publishes a number that covers it, and none can be shown to absorb it from a standing start on self-serve terms. The closest are Browserbase (6,000+ concurrent claimed), Bright Data ("unlimited", unverified in practice), and AWS (Lambda/EC2 via quota raises). Two to five large enterprise negotiations or a self-built fleet are needed.**

Arithmetic (all [computed], N = 10,000,000 pages):
- Rate: N / 3,600 s = 2,778 pages/s = 166,667/min.
- Concurrency: 5 s/page -> 13,889; 10 s/page -> 27,778 (3 s -> 8,333).
- Browser-hours: 13,889 (5 s) or 27,778 (10 s).

Hours to finish 10M at each vendor's highest published self-serve or enterprise concurrency (hours = N / (conc / duration) / 3,600):

| Vendor, concurrency used | 5 s pages | 10 s pages |
|---|---|---|
| Browserbase Scale default 250 | 55.6 h | 111 h |
| Browserbase claimed 6,000 | 2.3 h | 4.6 h |
| Cloudflare 200 | 69 h | 139 h |
| Kernel Start-Up 150 | 93 h | 185 h |
| Steel / Browser Use / AgentCore at 1,000 | 13.9 h | 27.8 h |
| Firecrawl 100 (Scale) | 139 h | 278 h |
| ScrapingBee 900 | 15.4 h | 30.9 h |
| ZenRows 1,000 | 13.9 h | 27.8 h |
| Spider 1,538 (and also capped at 10,000 req/min = 16.7 h) | 9.0 h | 18.1 h |
| Apify Business 256 runs | 54 h | 109 h |
| Zyte 3,000 RPM | 55.6 h regardless | 55.6 h |

Creation-rate wall if each page is its own session: Browserbase 150/min -> 1,111 h; Cloudflare 3/s -> 926 h; AgentCore StartBrowserSession 30 TPS -> 92.6 h; Browser Use 100 RPS -> 27.8 h. With session reuse (about 10,000-14,000 long-lived sessions each handling pages sequentially) the rate wall falls to the ramp time: AgentCore 463 s at 30 TPS; Browserbase 14k / 150 per min = 93 min (more than the whole job window) at the Scale floor.

Cost if a vendor did it (page cost only; 5 s and 10 s pages; excludes proxy):
- Per browser-hour: Browser Use $278-556; Kernel headless $833-1,667; Cloudflare $1,250-2,500; Steel Scale/Launch $1,111-2,778; Browserbase $1,389-3,333; Hyperbrowser $1,389-2,778; AgentCore at max profile $1,768-3,536. All in the low thousands of dollars: compute is not the problem.
- Per request: Firecrawl $10,000 (1 credit, $1/1k overage); Bright Data Web Unlocker $13,000-15,000; Scraper API $13,000-15,000; ScrapingBee about $2,400 on Enterprise-120 credit rate for 50M credits, ZenRows $3,300-5,500; Zyte browserHtml $4,800-160,800 depending on site tier and discount; Steel `/scrape` $50,000.
- Bandwidth dominates when billed per GB: at 0.5 MB/page = 5,000 GB; at 2 MB/page = 20,000 GB. Bright Data Browser API $8/GB -> $40,000-160,000. Browser Use own-proxy $0.20/GB -> $1,000-4,000, residential $5/GB -> $25,000-100,000. Browserbase proxy $10-12/GB -> $50,000-240,000 if proxied.
- Anchor Browser also charges $0.01 per browser created (not in the ask, shown for the reuse point): 10M one-page sessions = $100,000 vs 14k sessions = $140.
- Lambda: $1,669 (2 GB x 5 s) to $6,669 (4 GB x 10 s) plus egress. Assumes a quota of about 14k-28k concurrent environments; default 1,000, "tens of thousands" max by request.
- Self-hosted on rented servers: assume 1-3 vCPU-seconds per page (assumption). 2,778-8,333 vCPUs = 174-521 CCX43 servers (16 dedicated vCPU, 64 GB each; corrected by the report writer, the first draft said 48 vCPU) = EUR 92-277 for the hour at EUR 0.531/h (repo AGENTS.md rate), before bandwidth, IPv4s and boot time. Needs a Hetzner limit raise (manual, business hours) and 58-174 servers launched; AWS equivalent needs a vCPU quota raise from 5 to thousands and capacity.

What the arithmetic says: compute at mass scale is cheap ($300-$7,000); what is scarce is (a) concurrency allowances (all published ceilings are 100-1,538, one vendor claims 6,000), (b) session creation rates (3/s to 100/s, vs 2,778/s needed without reuse), (c) per-request pricing for managed unblockers ($10-15k for 10M) and per-GB pricing ($40k-240k).

Ramp speed (documented, [computed] where noted):
- AWS Lambda: 1,000 environments / 10 s / function = 100/s; sharding multiplies it; reaching 13,889 takes 139 s from one function.
- EC2: 1,000 instances instantly via RunInstances bucket, then 2/s; 200-500 large instances are inside the burst bucket, but vCPU quota (default 5) and Spot capacity are the gates.
- Browserbase: 150+/min (Scale), i.e. 2.5/s; Cloudflare 3/s; AgentCore 30 TPS; Browser Use up to 100 RPS; Kernel pools 25%/min of pool [unverified].
- Everything else: unpublished.

## 5. What I did not find

- No vendor publishes a "largest burst" claim in pages per hour or per second. Browserbase's "6,000+ concurrent" (blog 2026-06-16) and 35M sessions/month (enterprise page) are the strongest primary-source scale statements; Hyperbrowser's homepage says "1,000+" concurrent and "100M+" sessions; Bright Data says "unlimited"; Cloudflare says "several thousand containers" platform-wide (blog 2026-05-13).
- No documented Lambda-Chromium mass-rendering case study with numbers; no documented Spot launch-rate for 200-500 large instances; no Hetzner server-count defaults.
- Hyperbrowser official plan concurrency, Bright Data Browser API session limits, ScrapingBee official JS-credit cost, ScraperAPI JS-credit cost and Zyte per-plan concurrency were not retrievable from primary pages.
- Unverified items: Hyperbrowser batch endpoint name and third-party concurrency figures; Kernel pool fill rate; EC2 Fleet 10,000/100,000 capacity (search snippet); Cloudflare 200-per-account raise date (third-party); ScrapingBee 5-credit JS cost; Spider benchmark numbers (self-published).
