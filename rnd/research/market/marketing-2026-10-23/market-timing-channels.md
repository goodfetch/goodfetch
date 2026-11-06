# Market timing and launch channels: open-source crawling + browser automation (research as of 2026-10-23)

Scope: read-only web research. Every claim carries a source URL and a date. Flags: **[vendor]** = published by a company that sells in this market; **[secondary]** = reported by a third party, not the primary; **[unverified]** = I could not confirm it from a primary source; **[count 2026-10-23]** = measured by me today from a public API.

---

## Part 1. Timing

### 1.1 Demand driver: AI agents and LLM ingestion

| Fact | Source (date) | Flag |
|---|---|---|
| DataDome saw about 25 billion AI-agent requests in Q3 2026, up from 17.7 billion in Q2 (+41.5% QoQ). Agentic-browser traffic grew nearly 5x in one month. Meta's undeclared "Muse" agent made 29M requests in 12 days from cloud infrastructure. | https://datadome.co/press/datadome-report-finds-agentic-browser-traffic-surged-nearly-5x-in-one-month (2026-10-15) | **[vendor]**, measured on its own network |
| DataDome: 7.9 billion AI-agent requests in Jan-Feb 2026 alone, +5% over Q4 2025; known agents are being impersonated. | https://datadome.co/press/businesses-struggle-to-identify-ai-agent-traffic (2026-03-05) | **[vendor]** |
| Bright Data reports above $300M ARR, growth above 50% a year, "14 of the top 20 LLM labs" as customers, and a $400M target by mid-2026. I found no confirmation that the $400M was reached. | https://brightdata.com/blog/general/data-for-ai-is-fueling-massive-growth-at-bright-data and https://www.calcalistech.com/ctechnews/article/sjeyg2ezwe (2025-11-23) | **[vendor]**, self-reported |
| Extract Summit Austin talk "Who's actually buying web data now": AI companies are now the main data buyers. They rank coverage, freshness, provenance and reproducibility, and are very sensitive to parser changes. | https://proxyway.com/news/zyte-extract-summit-2026-austin-recap (2026-10-22) | **[secondary]** recap of a Zyte conference |
| Extract Summit main themes in 2026: AI for parsing and self-healing, agentic scraping, and more logging and observability for provenance. | same page (2026-10-22) | **[secondary]** |
| Zyte's 2026 report says production adoption of fully autonomous scraping agents is still limited. | https://www.scraping.club/p/a-preview-of-the-zyte-2026-web-scraping (2026-01-25) | **[vendor]** report, summarized by a newsletter |

**Market-size estimates**

The published numbers disagree by roughly 4x. TheDataHQ lists seven firms giving seven answers for 2025, from $502M to $3.2B, and notes that some reports list companies renamed five years ago (https://thedatahq.com/blog/web-scraping-market-size, 2026-09-10). Treat every figure below as low-reliability syndicated research.

| Estimate | Publisher | Source (date) | Flag |
|---|---|---|---|
| Web scraping: $1.34B (2025), $1.56B (2026), $3.49B (2031), CAGR 17.39% | Mordor Intelligence | https://www.giiresearch.com/report/moi2100261-web-scraping-market-share-analysis-industry-trends.html (2026-07-08) | syndicated research firm |
| Web scraping: $0.99B (2025), $1.17B (2026), $2.28B (2030) | The Business Research Company | https://www.giiresearch.com/report/tbrc2014250-web-scraping-global-market-report.html (2026-04-10) | syndicated |
| Web scraper software: $814M (2025), $2.5B (2034), CAGR 13.3% | Straits Research | https://straitsresearch.com/report/web-scraper-software-market (undated, 2026 edition) | syndicated |
| Web scraping software: $3.2B (2025), $12.5B (2034) | Verified Market Reports | https://www.verifiedmarketreports.com/product/web-scraping-software-market (2026-04-24) | outlier; its company list is stale per TheDataHQ |
| "AI-driven web scraping" for 2025: $7.5B, $7.8B, $8.2B | Market Research Future, SNS Insider, Research and Markets | https://thedatahq.com/blog/web-scraping-market-size (2026-09-10) | outliers; they define a different segment |
| Web scraping reached $1.03B in 2025, $2B by 2030. AI-based web scraping $3.16B by 2029 (Technavio, 39.4% CAGR). | Zyte 2026 Web Scraping Industry Report | https://www.scraping.club/p/a-preview-of-the-zyte-2026-web-scraping (2026-01-25) | **[vendor]** |
| Autonomous agents: $4.35B (2025) to $103.28B (2034) | Precedence Research, cited by Zyte | https://www.zyte.com/blog/scale-web-data-without-scaling-headcount (2026-01-22) | **[vendor]** citing research |
| AI browser market: $4.5B (2024) to $76.8B (2034) | market.us, cited by Firecrawl | https://www.firecrawl.dev/blog/best-browser-agents (2026-02-13) | **[vendor]** citing research; definitions differ |
| AI browser (consumer browsers with assistants): $1.42B (2025), $1.55B (2026) | Intel Market Research | https://www.intelmarketresearch.com/ai-browser-market-46945 (2026-10-16) | a different category: consumer browsers |

I found no credible standalone market-size figure for "browser infrastructure for agents". The best proxies are company data points: Browserbase reported more than $3M revenue and 1,000+ customers at its Series B (https://www.taskade.com/blog/history-of-browserbase, 2026, **[secondary]**, **[unverified]**), and Bright Data reported $300M ARR (above).

### 1.2 Cloudflare and other edge/bot vendors

**Cloudflare**

| Date | Event | Source |
|---|---|---|
| 2025-06-26 | Web Bot Auth added to Cloudflare Bots: HTTP-message-signature verification of bots. | https://developers.cloudflare.com/bots/changelog/ |
| 2025-07-01 | "Content Independence Day": new domains block AI crawlers by default. Pay per crawl introduced (HTTP 402 plus payment headers, using Web Bot Auth). | https://blog.cloudflare.com/introducing-pay-per-crawl (2025-07-01). The docs still call pay per crawl "closed beta": https://developers.cloudflare.com/ai-crawl-control/features/manage-ai-crawlers/ |
| 2025-08-04 | Cloudflare delisted Perplexity as a verified bot for undeclared "stealth" crawling and added blocking heuristics. It said 2.5M+ sites disallow AI training through its managed robots.txt. | https://blog.cloudflare.com/perplexity-is-using-stealth-undeclared-crawlers-to-evade-website-no-crawl-directives and https://www.searchenginejournal.com/cloudflare-delists-and-blocks-perplexity-from-crawling-websites/552899 (2025-08-04) |
| 2026-07-01 | Second "Content Independence Day". AI traffic is split into **Search / Agent / Training**. "Agent" is defined as "automated activity acting in real time on a person's behalf, such as chat fetch bots and **browser-use agents**." "Verified" no longer means "default allowed". Mixed-purpose crawlers (Googlebot, Bingbot, Applebot) are blocked wherever Training is blocked. New robots.txt Content-Signal field `use=`. New BotBase database. | https://blog.cloudflare.com/content-independence-day-ai-options and https://developers.cloudflare.com/bots/additional-configurations/block-ai-bots (2026-07-01) |
| 2026-09-15 | New defaults take effect: Training and Agent bots are blocked **on pages that display ads**; Search stays allowed. The Cloudflare blog and docs say this applies to new domains. TechCrunch also says it applies to existing free customers, so the scope is unclear. | same two pages, plus https://techcrunch.com/2026/07/01/cloudflares-new-policy-pushes-ai-companies-to-pay-for-publishers-content (2026-07-01) |
| 2026-07-01 | Pay per crawl is evolving into "Pay Per Use" (pay when content creates value). Launch partners are Ceramic.ai and You.com. Cloudflare data: over 50% of AI-crawler traffic is re-fetching unchanged pages. | TechCrunch, same URL |
| undated (2026) | "Monetization Gateway": charge any caller for any resource via the x402 payment protocol, optionally requiring Web Bot Auth. | https://blog.cloudflare.com/monetization-gateway |

Implication: user-driven browser agents now have their own class on the largest bot-management network. A tool that signs requests with Web Bot Auth and declares a purpose is the compliant path. A tool that stays undeclared is in the default-blocked, non-verified bucket.

**Akamai, Fastly, DataDome, HUMAN**

| Vendor | Move | Source (date) | Flag |
|---|---|---|---|
| Akamai | "Agentic Security Framework" with a "Know Your Agent" protocol linking agents to authorized human users (2026-06-15). Blog on charging AI agents at the edge with Akamai Functions (2026-10-12). | https://www.akamai.com/newsroom/press-release/akamai-unveils-agentic-security-framework-to-power-trusted-ai-driven-interactions-and-commerce and https://www.akamai.com/blog/ai/monetizing-machines-akamai-customers-charge-ai-agents-edge | **[vendor]** |
| DataDome | "Agent Trust": a 100-point score per agent. Named a Leader in the Forrester Wave: Bot and Agent Trust Management, Q2 2026. Priority Protect, a virtual waiting room for agentic commerce (2026-05-20). | https://datadome.co/changelog/datadome-product-roundup-q2-2026 (2026-07-14), https://datadome.co/press/datadome-announces-priority-protect/ | **[vendor]** |
| HUMAN Security | AgenticTrust (2025-07-30). It open-sourced a "Verified AI Agent" demo using HTTP Message Signatures. Its 2026 report warns that user-agent allowlists are unreliable because of spoofing (2026-03-26). | https://www.humansecurity.com/newsroom/first-adaptive-trust-layer-for-agentic-ai-era and https://www.humansecurity.com/learn/resources/2026-state-of-ai-traffic-cyberthreat-benchmarks | **[vendor]** |
| Fastly | "AI Bot Management" product: identify, classify, block/allow/monetize AI crawlers. I found no dated 2026 announcement. | https://www.fastly.com/products/fastly-ai-bot-management (undated) | **[vendor]**, **[unverified]** date |
| Google | Terms of Service effective 2026-07-30 reportedly prohibit automated access that violates machine-readable instructions such as robots.txt, including those that disallow training. | summarized at https://parallel.ai/articles/is-scraping-google-legal (2026-09-26); I did not read Google's page | **[secondary]**, and Parallel is a search-API competitor |

### 1.3 Standards (status in October 2026)

| Standard | Status | Source (date) |
|---|---|---|
| IETF **aipref** (AI Preferences) | WG active. Two Internet-Drafts, neither an RFC yet: `draft-ietf-aipref-vocab-08` (2026-09-13) and `draft-ietf-aipref-attach-05` (2026-08-18). The charter milestone to send both to the IESG was 2026-08-31 and was **missed**. Draft -07 states its contents "do not reflect consensus." The charter lists "technical enforcement" as out of scope. It extends robots.txt (RFC 9309) and HTTP headers. | https://datatracker.ietf.org/wg/aipref/documents, https://datatracker.ietf.org/wg/aipref/about/, https://datatracker.ietf.org/doc/html/draft-ietf-aipref-vocab-07 (2026-08-19) |
| IETF **Web Bot Auth** | WG active. First WG draft `draft-ietf-webbotauth-httpsig-protocol-00` (2026-09-01), authors from Cloudflare and Google, expires 2027-03-05. The milestone to send the BCP spec to the IESG was 2026-08-31 and was **missed**. The Anthropic-authored `draft-kelley-webbotauth-sail-00` (source address identity list) was posted 2026-10-22. Registry, hosted-directory and IP-range (Google) drafts exist. The WG excludes "defining a vocabulary for the intents of bots" from its scope. | https://datatracker.ietf.org/wg/webbotauth/documents, https://www.ietf.org/archive/id/draft-ietf-webbotauth-httpsig-protocol-00.html, https://datatracker.ietf.org/doc/html/draft-kelley-webbotauth-sail-00 |
| Cloudflare **Content Signals** | Informal robots.txt extension (`Content-Signal: search=yes,ai-train=no`), extended on 2026-07-01 with `use=`. Not an IETF standard. | https://blog.cloudflare.com/content-independence-day-ai-options (2026-07-01) |
| **llms.txt** | Not honored by major AI platforms. Ahrefs studied 137,210 domains in May 2026: 28% publish it, 97% of files got zero requests in the month, and AI retrieval bots made about 1.1% of the requests that did arrive. HasData (July 2026): OpenAI, Anthropic, Google and Perplexity read robots.txt and none has committed to llms.txt. | https://ahrefs.com/blog/llmstxt-study (2026-06-15, **[vendor]** SEO tool) and https://hasdata.com/blog/ai-crawler-block-index (2026-07-24, **[vendor]**) |
| EU GPAI Code of Practice (copyright chapter) | Signatories must detect and respect machine-readable rights reservations (robots.txt and other protocols). Measure 1.3(1)(a) rules out evading with spoofed identifiers or mimicking human traffic. Signatories must publish information on their crawlers. Taskforce meeting 2026-08-03. | https://www.aoshearman.com/en/insights/ao-shearman-on-data/eu-artificial-intelligence-office-publishes-the-final-version-of-the-gpai-code-of-practice (2025-07-30), https://link.springer.com/article/10.1007/s40319-025-01664-8 (2025-11-01), https://ec.europa.eu/en/news/fourth-gpai-signatory-taskforce-meeting (2026-08-03) |

Implication: the only robots.txt-class signals with real adoption are the old robots.txt, Cloudflare's Content Signals and Web Bot Auth signatures. Supporting `Content-Signal` and RFC 9309 by default, and signing requests with Web Bot Auth, are cheap and standards-aligned. Making llms.txt a headline feature is not worth it.

### 1.4 Legal (2025-2026)

| Case or rule | What happened | Source (date) |
|---|---|---|
| **Reddit v. SerpApi, Oxylabs, AWMProxy, Perplexity** (S.D.N.Y. 1:25-cv-08736) | Filed Oct 2025. On 2026-07-31 Judge Engelmayer largely denied the motions to dismiss. Surviving claims: DMCA §1201(a)(1)(A) against both SerpApi and Perplexity, §1201(a)(2) trafficking against SerpApi, and civil conspiracy. He held that Google's SearchGuard CAPTCHA/bot detection can be a "technological access control." The §1201(b) claim and the unfair-competition and unjust-enrichment claims were dismissed. A pleading-stage ruling only. | https://law.justia.com/cases/federal/district-courts/new-york/nysdce/1:2025cv08736/651592/104/ (2026-07-31), https://www.loeb.com/en/insights/publications/2026/08/reddit-v-serpapi-llc (2026-08-17), https://parallel.ai/articles/is-scraping-google-legal (2026-09-26) |
| **Google v. SerpApi** (N.D. Cal. 4:25-cv-10826) | Filed 2025-12-19. On 2026-07-20 Chief Judge Gonzalez Rogers dismissed the DMCA claims: with no leave to amend for results with no copyrighted content, and with leave for results containing licensed content. Google refiled 2026-08-10. SerpApi moved to dismiss again 2026-08-25. A hearing was set for **2026-10-13**; I found no outcome. | https://ailawsuittracker.com/cases/google-v-serpapi-4-25-cv-10826/ (reviewed 2026-09-27), https://parallel.ai/articles/is-scraping-google-legal (2026-09-26), https://arstechnica.com:8080/tech-policy/2026/07/reddit-keeps-weird-dmca-lawsuit-against-web-scraper-alive-despite-googles-loss (2026-07-31) |
| **Amazon v. Perplexity** (9th Cir. No. 26-1444) | Amazon sued Nov 2025 under the CFAA and California's CDAFA over Perplexity's Comet agentic browser. A preliminary injunction was granted in March 2026. The 9th Circuit **vacated** it (opinion in Aug 2026): the individual users, not Perplexity, "access" Amazon. It left open a different result if a developer controls its agent more. It noted Amazon can still enforce private terms of service. | https://cdn.ca9.uscourts.gov/datastore/opinions/2026/08/04/26-1444.pdf and https://www.zwillgen.com/artificial-intelligence/web-scraping-digest-9th-circuit-addresses-agentic-ai/ (2026-09-10) |
| **Meta v. Bright Data** and **X Corp. v. Bright Data** | 2024-01-23 (Meta): logged-off scraping is not "use" under Meta's terms, so no breach. 2024-05 (X Corp.): X's scraping claims preempted by copyright law. 2025-04-18 (X Corp.): Bright Data's antitrust counterclaims over X's terms were sustained. I did not find a 2026 merits ruling. | https://techcrunch.com/2024/01/24/court-rules-in-favor-of-a-web-scraper-bright-data-which-meta-had-used-and-then-sued (2024-01-24), https://www.skadden.com/insights/publications/2024/05/district-court-adopts-broad-view (2024-05-21), https://www.govinfo.gov/content/pkg/USCOURTS-cand-3_23-cv-03698/pdf/USCOURTS-cand-3_23-cv-03698-5.pdf (2025-04-18) |
| **Kneschke v. LAION** (Germany) | OLG Hamburg, 2025-12-10: downloading images to build a training set fell under the text-and-data-mining exceptions. A natural-language rights reservation is not machine-readable. Appeal to the Federal Court of Justice allowed. One tracker says the BGH heard it on 2026-09-03, **[unverified]** by a primary source. | https://www.vossius.eu/en/news/detail/decision-of-the-higher-regional-court-of-hamburg-on-ai-training-datasets (2025-12-11), https://ailawsuittracker.com/international-ai-lawsuits/germany/ |
| **GEMA v. OpenAI** (Munich) | 2025-11-11: models memorizing lyrics infringe. OpenAI appealed. About outputs, not crawling. | same tracker page |
| **Like Company v. Google Ireland** (CJEU C-250/25) | First AI-copyright reference to the CJEU, pending. | https://committees.parliament.uk/writtenevidence/162681/pdf (undated) |

Legal summary for a tool vendor: the CFAA mostly does not reach public pages (hiQ, Van Buren). The live risk is DMCA §1201 anti-circumvention, because stealth, fingerprint spoofing and CAPTCHA solving are what Google and Reddit plead as circumvention. Customers who send the queries can be named too (Perplexity in Reddit). In the EU, machine-readable opt-outs (robots.txt) carry real weight. Neither Google v. SerpApi nor Reddit v. SerpApi has a merits ruling.

**EU AI Act, obligations from August 2026 relevant to web data collection**

| Fact | Source (date) |
|---|---|
| GPAI provider obligations have applied since 2025-08-02: technical documentation, a copyright policy, and a public summary of training content. The **Commission's enforcement powers, including fines, apply from 2026-08-02**. Models placed on the market before 2025-08-02 must comply by 2027-08-02. | https://digital-strategy.ec.europa.eu/en/policies/guidelines-gpai-providers, https://digital-strategy.ec.europa.eu/en/policies/enforcement-ai-act (2026-10-20), https://www.taylorwessing.com/de/insights-and-events/insights/2026/08/gpai-obligations-under-the-eu-ai-act (2026-08-19) |
| Article 50 transparency obligations (AI-generated content marking) apply from 2026-08-02. | https://digital-strategy.ec.europa.eu/en/policies/code-practice-ai-generated-content (2026-10-19) |
| The Digital Omnibus on AI reached a trilogue deal on 2026-05-07. Stand-alone high-risk obligations moved to 2027-12-02 and embedded ones to 2028-08-02. GPAI dates did not change. | https://www.europarl.europa.eu/legislative-train/theme-internal-market-and-consumer-protection-imco/file-digital-omnibus-on-ai (as of 2026-09-20), https://ai-act-service-desk.ec.europa.eu/en/ai-act/faq/when-does-enforcement-start |
| The Act binds model providers, not scraping-tool vendors. The practical effect is downstream: AI-company customers must prove they honored rights reservations, so they will ask data suppliers for provenance, crawl logs and robots.txt/opt-out handling. This is my inference. It matches the Extract Summit talk on provenance and the legal panel's advice to "document data acquisition diligently". | https://proxyway.com/news/zyte-extract-summit-2026-austin-recap (2026-10-22) |

### 1.5 Funding (investor heat)

| Company | Round | Date | Source | Flag |
|---|---|---|---|---|
| Firecrawl | $75M Series B, led by Smash Capital; Altos, Nexus, YC, Freestyle, Offline Ventures. Launched "Alexandria". | 2026-09-22 | https://www.firecrawl.dev/blog/introducing-alexandria-series-b | primary |
| Firecrawl | $14.5M Series A, led by Nexus; total then $16.2M. | 2025-08-19 | https://www.firecrawl.dev/press/firecrawl-series-a-press-release.pdf | primary |
| Parallel Web Systems | $100M Series A at $740M (Kleiner, Index); then $100M Series B at $2B (Sequoia). | 2025-11-12; 2026-04-29 | https://www.reuters.com/business/ex-twitter-ceo-agrawals-ai-search-startup-parallel-raises-100-million-2025-11-12, https://www.prnewswire.com/news-releases/parallel-raises-at-2-billion-valuation-to-scale-web-infrastructure-for-agents-302756350.html | |
| Exa | $85M Series B at $700M (Benchmark); then $250M Series C at $2.2B (a16z). | 2025-09-03; 2026-05-20 | https://exa.ai/blog/announcing-series-b, https://exa.ai/blog/announcing-series-c | primary |
| Tavily | $25M Series A (Insight, Alpha Wave). | 2025-08-06 | https://www.tavily.com/blog/big-news-from-tavily-announcing-25m-series-a-to-power-the-internet-of-agents | primary |
| Oxylabs | $130M from Warburg Pincus at $3.6B; its first outside investment since 2015. | 2026-07-09 | https://www.globenewswire.com/news-release/2026/07/09/3324811/0/en/Oxylabs-Announces-130-Million-Investment-from-Warburg-Pincus.html | primary |
| Browser Use | $17M seed (Felicis; YC, Paul Graham). No later round found. | 2025-03-22 | https://browser-use.com/posts/seed-round | primary |
| Browserbase | $40M Series B at $300M post-money (Notable, CRV, Kleiner); about $67.5M total. Sources disagree on the date (Apr 22, May 2 and June 2025). **No 2026 round found.** | 2025 | https://sacra.com/c/browserbase, https://pitchbook.com/profiles/company/593111-35 | secondary |
| Kernel | $22M seed + Series A, led by Accel. | 2025-10-15 | https://www.kernel.sh/blog/series-a-announcement | primary |
| Anchor Browser | $6M seed, led by Blumberg. | 2025-10-22 | https://blumbergcapital.com/news-insights/anchor-launches-reliable-browser-automation | primary |
| Skyvern | $2.7M seed. | 2025-12-18 | https://platform.tracxn.com/a/d/company/59d4da4ee4b03c8ff2f646a2/skyvern | Tracxn only, **[unverified]** |
| Apify | No large venture round found. Third-party trackers show only small rounds (a $3M Series A in 2024). Apify runs a $1M developer challenge and says it has paid out over $4M to Store developers (2025-11-03). | | https://pitchbook.com/profiles/company/168029-47, https://natlawreview.com/press-releases/apify-bets-1m-independent-developers-building-ais-missing-tools | **[vendor]** for the payout figure |
| Hyperbrowser, Steel | No disclosed 2025-26 round found. Steel's browser is open source (https://steel.dev/). | | | **[unverified]** |
| Bright Data | No new round found; $300M ARR is self-reported (see 1.1). | | | |

Read: capital is flowing to the **search/context-API** layer (Parallel, Exa, Firecrawl, Tavily) and to a PE-backed incumbent (Oxylabs). The cloud-browser layer (Browserbase, Kernel, Anchor, Browser Use) raised $6-40M rounds in 2025 and I found nothing in 2026. That is a signal of crowding, or of investor attention moving up the stack.

### 1.6 Direct competitors for "AI actions that replay without a model"

| Project | Facts | Source (date) |
|---|---|---|
| **Libretto** (Saffron Health) | MIT. Records user actions and replays them as scripts; coding-agent-driven. Created 2026-03-02, 894 stars. Show HN "Making AI browser automations deterministic" got 134 points on 2026-04-15. | https://github.com/saffron-health/libretto, https://news.ycombinator.com/item?id=47780971 |
| **Stagehand v4** (Browserbase) | Released 2026-08-10: moves the framework into a browser extension, "self-healing actions", claims 2x faster than Playwright and about 80% more token-efficient **[vendor]**. 25.8k stars. TypeScript/Python/Go. | https://www.browserbase.com/blog/stagehand-v4 (2026-08-10), https://api.github.com/repos/browserbase/stagehand **[count 2026-10-23]** |
| **Browser Use** | MIT, 119.0k stars **[count 2026-10-23]**. Cloud browsers from $0.02 per hour. | https://github.com/browser-use/browser-use |
| **Skyvern** | AGPL-3.0, 23.3k stars **[count 2026-10-23]**. | https://github.com/Skyvern-AI/skyvern |
| **Firecrawl** | AGPL-3.0, 194.7k stars **[count 2026-10-23]**. "/interact" endpoint for clicks, pagination and logins (2026-05-21). Rust single-binary self-hosted alternative "Draco" (Show HN 2026-08-02, 15 points). | https://github.com/firecrawl/firecrawl, https://x.com/firecrawl/status/2057495269785358384 |

---

## Part 2. Channels

### 2.1 Subreddits

Reddit blocked my direct reads, so member counts come from third-party trackers **[secondary]**.

| Subreddit | Size | Source (date) |
|---|---|---|
| r/webscraping | about 102k members (101,746 on 2026-10-17; 105k at gummysearch, 104K at Prowlo; roughly +28k/yr). Created 2014. | https://www.replyhey.com/subreddits/webscraping (2026-10-17), https://reddapi.dev/subreddits/webscraping/insights, https://gummysearch.com/r/webscraping |
| r/dataengineering | about 240-256k | https://datadriven.partners/reach/channels/r-dataengineering-for-data-tool-launches (2026-05-17), https://www.deadsubs.com/r/dataengineering (Feb 2025 count) |
| r/scrapy | about 5.5k (low confidence). The same article lists r/webscraping at 12,329, which is far below every other source, so the page is stale or wrong. | https://www.scrapingbee.com/blog/11-best-subreddits-for-webscraping (2026-01-12) |
| r/scrapingtheweb | about 8.6k | https://prowlo.com/tools/subreddit-stats/scrapingtheweb (crawl 2026-09-13) |

### 2.2 Discord servers **[count 2026-10-23]**

Counts are `approximate_member_count` from the public invite endpoint (`discord.com/api/v9/invites/<code>?with_counts=true`). Invite codes came from each project's README or site.

| Server | Members | Online |
|---|---|---|
| Firecrawl | 31,159 | 2,785 |
| Apify & Crawlee | 22,970 | 1,614 |
| Browser Use | 22,672 | n/a |
| Scraping Enthusiasts (linked from the r/webscraping bookmarks) | 11,378 | 1,572 |
| Crawl4AI Community | 7,128 | 379 |
| Scrapy | 3,234 | 208 |
| Stagehand (by Browserbase) | 1,086 | 114 |

Other servers listed in 2022 guides, not counted: Extract Data Community (Zyte-run), Scraping In Prod, Oxylabs and ProxyWay (https://scrapeops.io/web-scraping-playbook/best-web-scraping-discord-servers, page dated 2022-07-20, **[stale]**).

### 2.3 Newsletters, media and conferences

| Channel | Facts | Source (date) |
|---|---|---|
| The Web Scraping Club (Pierluigi Vinciguerra) | Substack since mid-2022. Over 7,000 subscribers. Free posts weekly, with partner guest posts on Tuesdays. Vinciguerra spoke on a panel at OxyCon 2026. | https://www.scraping.club/ and https://www.scraping.club/about (undated); https://proxyway.com/news/oxycon-2026-recap (2026-10-08) |
| Proxyway | Industry news site. It writes the conference recaps (Extract Summit, OxyCon). It uses affiliate links. | https://proxyway.com/news/zyte-extract-summit-2026-austin-recap (2026-10-22) |
| Zyte "Extract Data" Substack | Zyte's blog and newsletter. | https://extractdata.substack.com/ (linked from Zyte's blog, 2026-07-16) |
| **Extract Summit 2026** (Zyte) | 8th year; Austin 2026-10-07/08 (held); **Dublin 2026-11-10/11 (upcoming)**. Tickets up to several hundred dollars. 10+ talks per city, workshops, a legal panel, with a live stream. Zyte calls it "the world's largest web scraping conference" **[vendor claim]**. Organizers say they reject vendor product pitches. Dublin lineup includes Lightpanda ("AI-native browser built from scratch"). Austin themes: AI parsing, antiban, TCP/IP fingerprinting, provenance, legal. | https://www.zyte.com/blog/five-reasons-to-attend-this-year-s-extract-summit (2026-07-16), https://www.extractsummit.io/, https://proxyway.com/news/zyte-extract-summit-2026-austin-recap (2026-10-22) |
| **OxyCon 2026** (Oxylabs) | Online, free, 2026-09-16. Seven presentations and two panels, including "finding the best open-source web scraping repositories." | https://proxyway.com/news/oxylabs-reveals-2026-oxycons-agenda (2026-07-30), https://proxyway.com/news/oxycon-2026-recap (2026-10-08) |

I did not research agent-focused events (for example AI Engineer conferences) or other scraping newsletters.

### 2.4 YouTube and awesome-lists

| Item | Facts | Source (date) |
|---|---|---|
| John Watson Rooney | About 104-105k subscribers, 314-317 videos. Python scraping, anti-bot, Cloudflare. Page snapshot date unknown. | https://www.youtube.com/c/JohnWatsonRooney, https://www.youtube.com/channel/UC8tgRQ7DOzAbn9L7zDL8mLg/videos |
| Others | Feedspot lists 20 scraping channels, most with 1-14K subscribers (the sample I saw). Tool vendors (Apify, Firecrawl) run their own channels. I did not verify other large creators. | https://videos.feedspot.com/web_scraping_youtube_channels (updated 2026-07-02) |
| lorien/awesome-web-scraping | 8,187 stars **[count 2026-10-23]**, last push 2026-09-19. The canonical list. | https://github.com/lorien/awesome-web-scraping, https://www.gitgenius.co/repos/lorien/awesome-web-scraping |
| ScrapeRouter/awesome-scraping | 18 stars **[count 2026-10-23]**. Auto-updated, run by a vendor. Low reach. | https://github.com/ScrapeRouter/awesome-scraping |
| gittrend.io "web scraping" trending | Daily ranking by star velocity. Shows how new repos surface. | https://gittrend.io/trending/web-scraping (2026-10-22) |

### 2.5 Which venues produced breakout launches (2024-2026)

Star counts are from the GitHub API **[count 2026-10-23]**. HN points are from the HN Algolia API **[count 2026-10-23]**.

| Project | Stars | Launch path | Evidence |
|---|---|---|---|
| **Firecrawl** | 194.7k (created 2024-04-15) | Launch YC post in July 2024 ("8,000+ stars in 3 months"). I found **no high-scoring HN post**. Growth came via GitHub, X and YC. A May 2026 X post got 4.7M views. | https://www.ycombinator.com/launches/LTf-firecrawl-open-source-crawling-and-scraping-for-ai-ready-web-data, https://www.firecrawl.dev/blog/firecrawl-july-2024-updates (2024-07-31), https://x.com/firecrawl/status/2057495269785358384 |
| **Crawl4AI** | 86.1k (created 2024-05-09) | Solo developer, "released it without ceremony" and it "went viral." HN posts about it all scored 7 points or less. Sustained by sponsorships ($5-$2,000 a month) plus a cloud API. | https://heatdrop.ai/editorial/unclecode/crawl4ai/2026-06-11 (**[secondary]**), HN Algolia query |
| **Browser Use** | 119.0k (created 2024-10-31) | Show HN 2024-11-05 (180 points), then Launch HN 2025-02-25 (259 points), then a $17M seed 2025-03-22. The founders say they built the prototype in four days and launched on HN. | https://news.ycombinator.com/item?id=42052432, https://news.ycombinator.com/item?id=43173378, https://browser-use.com/posts/seed-round |
| **Stagehand** | 25.8k | Show HN 2025-01-08, **326 points / 86 comments**. | https://news.ycombinator.com/item?id=42635942 |
| **Skyvern** | 23.3k | Show HN 2024-03-14 (422 points); Launch HN 2024-10-24 (327 points). | https://news.ycombinator.com/item?id=39706004, https://news.ycombinator.com/item?id=41936745 |
| **Crawlee** | 26.2k | Show HN 2022-08-23 (282 points); Crawlee for Python 2024-07-09 (254 points). | https://news.ycombinator.com/item?id=32561127, https://news.ycombinator.com/item?id=40913736 |
| **Libretto** | 894 | Show HN 2026-04-15, 134 points. Small star count even after a good HN result. | https://news.ycombinator.com/item?id=47780971 |
| **Hyperbrowser** | n/a | Show HN 2024-12-10 (57 points); MCP server 2025-03-20 (63 points). | https://news.ycombinator.com/item?id=42381712 |

Pattern: for AI-flavored browser automation, Hacker News (Show HN with a runnable repo and a plain explanation) is the proven venue (Stagehand, Skyvern, Browser Use). Scraping-for-LLM tools (Firecrawl, Crawl4AI) broke out through GitHub trending, X and YC. Most of these were launched when "LLM browser agent" was new; Libretto's 2026 result shows the same format still reaches the HN front page, with less star lift.

### 2.6 Community norms and self-promotion rules

| Venue | Rule | Source (date) |
|---|---|---|
| r/webscraping | Wiki: advertise commercial products only in the monthly thread and the weekly discussion. Do not promote paid services or products elsewhere, **including products with a free tier**. **Open-source tooling may be shared in any post "as long as they will continue to remain free in the future."** Links to branded guides must be relevant to the question and not require a subscription or API key. Product Hunt launches may be shared as a Link post within 10 days. Sidebar rules: keep discussions open, follow the wiki on promotion, "refrain from referring to commercial products or services, including proxy vendors." Third-party checkers advise asking the mods first. | https://www.reddit.com/r/webscraping/wiki/index/ and https://www.reddit.com/r/webscraping/about/ (read 2026-10-23); https://www.threadotter.com/subreddit-rules/webscraping (checked July 2026, **[secondary]**) |
| Hacker News | Show HN is for things people can run now; no landing pages, sign-up pages or newsletters; the project should be non-trivial. "Don't use HN primarily for promotion." Do not solicit upvotes. | https://news.ycombinator.com/showhn.html, https://news.ycombinator.com/newsguidelines.html (read 2026-10-23) |
| Extract Summit | Organizers say they reject recycled talks and vendor product pitches in favor of candid technical accounts. | https://www.zyte.com/blog/five-reasons-to-attend-this-year-s-extract-summit (2026-07-16) |
| r/dataengineering | Explicit vendor self-promotion rules. Transparent vendor participation is allowed. | https://datadriven.partners/reach/channels/r-dataengineering-for-data-tool-launches (2026-05-17, **[secondary]**) |

---

## Gaps and caveats

- I could not read Reddit directly (blocked), so subreddit sizes come from trackers. Check r/webscraping's live rules before posting.
- Outcome of the 2026-10-13 Google v. SerpApi hearing: not found.
- No standalone market figure for "browser infrastructure for agents".
- No 2026 funding found for Browserbase, Hyperbrowser, Steel, Apify or Browser Use. Absence from my searches is not proof they have not raised.
- Fastly's dated 2026 moves: not found.
- The BGH hearing date for Kneschke v. LAION rests on one tracker.
- Cloudflare's Sep 15 default scope (new domains only, or also existing free customers) differs between sources.
- Firecrawl's own HN launch: I found none above 40 points with the queries I ran. A different query may find one.
- I did not look for agent-oriented newsletters, podcasts or events beyond those above.
