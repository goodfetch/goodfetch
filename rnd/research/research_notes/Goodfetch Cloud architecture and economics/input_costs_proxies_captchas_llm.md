# Goodfetch Cloud input costs (COGS): proxies, captcha and challenge solving, LLM tokens, search queries — October 2026

Conventions: prices are USD list prices excluding VAT or sales tax unless stated. Every vendor page was checked on 2026-10-12, and each link's text carries that date. Third-party articles carry their own publication or update date. "Unverified" marks a figure reported only second-hand. Success-rate and speed figures are the vendor's own claims unless a third-party benchmark is named. The shared web-search budget ran out partway through the session, so the remaining pages were fetched directly from known URLs; items that needed a search are listed under Gaps.

## 1. Proxies: list, enterprise and wholesale prices (residential, ISP, mobile, datacenter)

### Takeaway
At about 1 TB a month, rotating residential bandwidth costs $0.49 to $2.60/GB on published self-serve plans: Evomi Core $0.49, Rayobyte $0.70, DataImpulse $0.80, Webshare $1.50, Decodo $2.00 (enterprise), Oxylabs $2.50, and Bright Data about $2.50 with a 3-month coupon ($5 list). Published enterprise floors go down to $0.35–$0.85/GB (SOAX, by geography tier) and from $0.50/GB (Rayobyte). Proxyway's 2026 cross-vendor average is $4.16/GB at 5 GB and $1.94/GB at 1,000 GB, and residential prices stopped falling in early 2026. ISP IPs cost about $0.23 to $5 per IP per month, mobile about $0.95 to $8/GB, and datacenter about $0.02 to $2.20 per IP per month or $0.23 to $0.60/GB. NetNut was seized by the FBI on 2 July 2026 and is no longer an option.

### Cited Findings

#### Market level (Proxyway, *Proxy Market Research 2026*, published 2026-05-28)
- Standardized residential price per GB across 13 tested providers; average $4.16 (5 GB), $3.00 (50 GB), $2.84 (100 GB), $2.60 (250 GB), $2.28 (500 GB), $1.94 (1,000 GB). Medians: entry point $4; 5 GB $3.75/GB; 50 GB $3.00/GB; 500 GB $2.25/GB — [Proxyway 2026](https://proxyway.com/research/proxy-market-research-2026)
- Per-provider rows at 5 / 50 / 100 / 250 / 500 / 1,000 GB: Byteful 3.25/3.00/2.75/2.50/2.25/1.95; Decodo 3.75/3.00/2.75/2.50/2.25/2.00; Evomi 4.00/3.40/3.20/3.00/2.80/2.60; Infatica 4.00/3.84/3.60/2.90/2.70/2.60; IPRoyal 5.95/4.90/4.55/4.20/3.50/3.15; Oxylabs 6.00/5.00/4.50/4.00/3.00/2.50; ProxyEmpire 3.50/2.67/2.50/2.22/1.75/1.50; Rayobyte 3.50/2.00/2.00/1.50/1.50/0.70; Webshare 3.50/2.60/2.25/2.00/1.75/1.50. Rows with only five values (so the column alignment is unclear): DataImpulse 1.00 to 0.80; DataImpulse Premium 5.00 to 4.00; Evomi Core 0.49 flat; NetNut 3.53 to 2.49; Proxyrack 1.20 to 0.80; SOAX 3.40 to 2.00 — [Proxyway 2026](https://proxyway.com/research/proxy-market-research-2026)
- Trend: residential rates "contracted by up to 75%" between 2023 and 2025. In early 2026 prices "stabilized and began to revert". Oxylabs, Massive and SOAX hid or discontinued pay-as-you-go plans, which raised entry points "up to twenty-five times". ProxyEmpire and Webshare halved their rates against the previous March — [Proxyway 2026](https://proxyway.com/research/proxy-market-research-2026)
- Mobile medians: entry point $30; $4.38/GB at 5 GB; $3.50/GB at 50 GB; $2.48/GB at 500 GB. Mobile prices fell by as much as 98% (Rayobyte) — [Proxyway 2026](https://proxyway.com/research/proxy-market-research-2026)
- Grey-market storefronts sell residential bandwidth that "often runs under $0.50, reaching $0.15 at its lowest" per GB (**unverified**: second-hand, no named vendors) — [Proxyway 2026](https://proxyway.com/research/proxy-market-research-2026)
- Infrastructure success / average response time, global residential pool: Oxylabs 99.93% / 0.60 s; Decodo 99.92% / 0.53 s; DataImpulse Premium 99.82% / 0.66 s; SOAX 99.78% / 0.89 s; DataImpulse 99.74% / 0.97 s; Byteful 99.69% / 0.41 s; Evomi Premium 99.58% / 0.79 s; Webshare 99.28% / 0.86 s; ProxyEmpire 98.17% / 1.32 s; Rayobyte 98.02% / 1.86 s; Infatica 97.57% / 1.64 s; Proxyrack 96.73% / 1.88 s; IPRoyal 96.11% / 1.24 s; NetNut 95.28% / 1.66 s. Against protected targets (Amazon, Google, Instagram) the median success rate drops to 74.43% (best 81.23%, Byteful), with a median response of 3.21 s — [Proxyway 2026](https://proxyway.com/research/proxy-market-research-2026)

#### Rotating residential, per vendor (official pages)

| Vendor | Small volume | Mid volume | Large volume or enterprise | Notes | Source |
|---|---|---|---|---|---|
| Bright Data | PAYG $4/GB (struck-through $8) | $499/mo for 141 GB (about $3.54/GB, list $7); $999/mo for 332 GB (about $3.01/GB, list $6) | $1,999/mo for 798 GB (about $2.50/GB, list $5); custom above 1 TB | The shown prices apply coupon RESIGB50, 50% off "for 3 months". The meta description says "from $5.88/GB", which the table doesn't explain. KYC is required for residential and mobile | [Bright Data residential, 2026-10-12](https://brightdata.com/pricing/proxy-network/residential-proxies); [Bright Data mobile page (same tiers), 2026-10-12](https://brightdata.com/pricing/proxy-network/mobile-proxies) |
| Oxylabs | $6/GB up to 5 GB | $5/GB (up to 20 GB); $4/GB (up to 125 GB) | $2.5/GB (1K+ GB); Enterprise "starting at $2,500/month" | Plans $30 / $100 / $500 per month | [Oxylabs residential, 2026-10-12](https://oxylabs.io/pricing/residential-proxy-pool); [Oxylabs pricing, 2026-10-12](https://oxylabs.io/pricing) |
| Decodo (formerly Smartproxy) | PAYG $4/GB; 3 GB at $3.75 | 25 GB at $3.25; 100 GB at $2.75 | Enterprise "$2.50/GB (250GB) to $2/GB (1TB)" | Monthly plans + VAT | [Decodo residential, 2026-10-12](https://decodo.com/proxies/residential-proxies/pricing); [Decodo pricing, 2026-10-12](https://decodo.com/pricing) |
| SOAX (one rate for residential and mobile) | Sandbox (PAYG top-ups) $5.00 / $4.00 / $2.00 per GB, by geo tier 1/2/3 | Builder $200/mo: $3.00/$2.25/$1.20; Team $500/mo: $2.20/$1.65/$0.90 | Scale $1,500/mo: $1.50/$1.15/$0.60; Enterprise $3,000/mo: "from $0.85 / $0.55 / $0.35" | Enterprise annual through support only | [SOAX pricing, 2026-10-12](https://soax.com/pricing) |
| IPRoyal | 1 GB $7.35 PAYG ($7.00 subscription) | 10 GB $5.51 ($5.25) | From 10 TB $1.84 ($1.75) | Traffic does not expire | [IPRoyal residential, 2026-10-12](https://iproyal.com/pricing/residential-proxies/) |
| Webshare | 1 GB $3.50 | 100 GB $2.25; 250 GB $2.00; 500 GB $1.75 | 1,000 GB $1.50; 3,000 GB $1.40 | "$7.00/GB base rate struck through" | [Webshare residential, 2026-10-12](https://www.webshare.io/residential-proxy) |
| Evomi | Core: PAYG $0.99/GB from 15 GB; Premium PAYG $3.98/GB | Core: 100 GB plan $49/mo ($0.49/GB) | Core PAYG $0.79/GB from 1,000 GB; Premium "as low as $2.20/GB" | No expiry on PAYG | [Evomi pricing, 2026-10-12](https://evomi.com/pricing) |
| DataImpulse | $5 for 5 GB ($1/GB) | $50 for 50 GB ($1/GB) | $800 for 1 TB ($0.80/GB); custom 5 TB+ "from $4,000"; Premium residential $5/GB | "Traffic never expires" | [DataImpulse, 2026-10-12](https://dataimpulse.com/) |
| Rayobyte, Premium Residential | 1–49 GB $3.50 | 50–249 GB $2; 250–999 GB $1.50 | 1,000–4,999 GB $0.70; Enterprise "from $0.50/GB" | Unlimited concurrency; country, city, ZIP and ASN targeting | [Rayobyte pricing, 2026-10-12](https://rayobyte.com/pricing/); [Rayobyte residential, 2026-10-12](https://rayobyte.com/products/residential-proxies/) |
| Rayobyte, Scale Residential | $100/mo for 6,600 GB ($0.0150/GB) | $500/mo for 40,000 GB ($0.0125/GB); $1,500/mo for 136,000 GB ($0.0110/GB) | $5,000/mo for 526,000 GB ($0.0095/GB) | Concurrency "1 per $1 of plan"; targeting "None: the network picks the IP"; "Sold to verified businesses after a compliance review"; not available for resale | [Rayobyte pricing, 2026-10-12](https://rayobyte.com/pricing/); [Rayobyte residential, 2026-10-12](https://rayobyte.com/products/residential-proxies/) |
| Infatica | PAYG $4; 25 GB $3.84 | 100 GB $3.60; 241 GB $2.90; 500 GB $2.70 | 1,000 GB $2.60; a second, unlabeled table goes from 300 GB at $3.07 to 12,000 GB at $2.08 | The navigation menu says "from $0.3 per GB", which the table contradicts | [Infatica pricing, 2026-10-12](https://infatica.io/pricing/) |
| Massive | Starter $80 for 10 GB ($8/GB) | Builder $225 / 30 GB ($7.5); Scaler $385 / 70 GB ($5.5) | Pro $490 / 100 GB ($4.9); Enterprise "From $3.2/GB" | Unused bandwidth rolls over | [Massive pricing, 2026-10-12](https://www.joinmassive.com/pricing) |
| PacketStream | $1.00/GB, $50 minimum | same | Resellers: $500 minimum, no published wholesale rate | Peer-to-peer "Packeters" | [PacketStream, 2026-10-12](https://packetstream.io/) |
| Proxy-Cheap | "Starts at $0.78/GB", marked -70% from $2.60 | No tiers published; top-ups of 1 to 50 GB per month | — | — | [Proxy-Cheap pricing, 2026-10-12](https://www.proxy-cheap.com/pricing) |
| Geonode | "from $0.27/GB" (starting price only) | — | Unlimited residential "from $1,800/mo" | Tiers not published | [Geonode pricing, 2026-10-12](https://geonode.com/pricing) |
| Live Proxies (US rotating) | $70 for 200 IPs + 4 GB / 30 days (about $17.50/GB, computed) | $140 for 16 GB / 30 days (about $8.75/GB) | $500 for 50 GB / 60 days ($10/GB) | Priced per IP allocation plus GB; bulk B2B pricing is custom | [Live Proxies pricing, 2026-10-12](https://liveproxies.io/pricing) |
| NetNut | Pricing page now shows "Seized by the Federal Bureau of Investigation" | Proxyway's pre-seizure figures: $3.53 (5 GB) to $2.49 (500 GB) | — | See section 3 | [netnut.io/pricing, 2026-10-12](https://netnut.io/pricing/); [Proxyway 2026](https://proxyway.com/research/proxy-market-research-2026) |

#### ISP (static residential), per IP per month unless noted
- Bright Data: shared $1.8 (10 IPs) → $1.45 (100) → $1.4 (500) → $1.3 (1,000); dedicated $3.5 → $2.75 → $2.6 → $2.5; custom pricing above 1,000 IPs. "Each IP includes a 100 GB fair usage allowance per month." Pay-per-GB ISP: PAYG $8/GB, then $499 for 71 GB ($7), $999 for 166 GB ($6), $1,999 for 399 GB ($5) — [Bright Data ISP, 2026-10-12](https://brightdata.com/pricing/proxy-network/isp-proxies)
- Oxylabs: ISP $1.5/IP (up to 20 IPs) → $1.4 (up to 71) → $1.2 (up to 416) → $1.1 (2.27K+), "unlimited bandwidth with fair usage" (concurrency limited past 50 GB per IP per month); Dedicated ISP $3 → $2.2/IP (1.14K+ IPs) — [Oxylabs ISP, 2026-10-12](https://oxylabs.io/pricing/isp-proxies); [Oxylabs pricing, 2026-10-12](https://oxylabs.io/pricing)
- Decodo: dedicated ISP $3.33/IP (3 IPs) → $2.9 (10) → $2.7 (50) → $2.6 (100) → $2.5 (200); shared ISP "From $1.30/GB" pay-per-GB or "From $0.27/IP" — [Decodo ISP, 2026-10-12](https://decodo.com/proxies/isp-proxies/pricing)
- Webshare static residential: $0.30/proxy (20–250 proxies) → $0.27 (1,000) → $0.24 (3,000–5,000) → $0.225 (10,000), bandwidth "Up to Unlimited", subject to fair use — [Webshare static residential, 2026-10-12](https://www.webshare.io/static-residential-proxy)
- Evomi: shared $1.00/IP, private $2.50/IP, virgin $4.50/IP — [Evomi pricing, 2026-10-12](https://evomi.com/pricing)
- IPRoyal: from $1.80 (24 hours), $2.70 (30 days), $2.55 (60 days), $2.40 (90 days) per proxy, with unlimited sessions and threads — [IPRoyal ISP, 2026-10-12](https://iproyal.com/pricing/isp-proxies/)
- Rayobyte static ISP: $5/IP (5–99) → $4.80 (100–999) → $4.60 (1,000–4,999); enterprise "from $0.95/IP" with a commitment. Rotating ISP PAYG: $3.75/GB (1–15 GB) → $2.50/GB (501–1,000 GB). Monthly rotating ISP plans: $0.0140 → $0.0085/GB — [Rayobyte pricing, 2026-10-12](https://rayobyte.com/pricing/)
- Infatica $1.95–$3.00/IP with unlimited traffic; Massive "from $1.8/IP"; Geonode "from $1.25 per IP"; Proxy-Cheap $2.71/IP on 1- and 12-month plans (the homepage says "from $2.44") — [Infatica, 2026-10-12](https://infatica.io/pricing/); [Massive, 2026-10-12](https://www.joinmassive.com/pricing); [Geonode, 2026-10-12](https://geonode.com/pricing); [Proxy-Cheap, 2026-10-12](https://www.proxy-cheap.com/pricing)
- Proxyway gives no ISP price per IP; it says ISP pricing has "even less leeway" and that providers are changing "the optics" by advertising shared models — [Proxyway 2026](https://proxyway.com/research/proxy-market-research-2026)

#### Mobile
- Oxylabs $7.5/GB (up to 4 GB) → $6.67 (up to 14 GB) → $5 (up to 100 GB) → $3.5/GB (714+ GB) — [Oxylabs pricing, 2026-10-12](https://oxylabs.io/pricing)
- Bright Data: the mobile page shows the residential per-GB tiers ($4 PAYG down to about $2.50 at 798 GB with the coupon), and KYC is required — [Bright Data mobile, 2026-10-12](https://brightdata.com/pricing/proxy-network/mobile-proxies)
- SOAX: same rate card as residential (from $5/GB at Sandbox to "from $0.85/$0.55/$0.35" at Enterprise) — [SOAX, 2026-10-12](https://soax.com/pricing)
- Rayobyte (monthly only; bandwidth expires after 30 days): $250 for 83 GB ($3/GB); $1,000 for 571 GB ($1.75); $2,500 for 2,632 GB ($0.95) — [Rayobyte pricing, 2026-10-12](https://rayobyte.com/pricing/)
- DataImpulse $2/GB, falling to $1.6/GB at 1 TB; Evomi $3.98/GB PAYG, "as low as $2.20/GB"; Infatica $8/GB PAYG → $3.20/GB (2,400 GB); IPRoyal $6.80/GB (2 GB) → $5.20/GB (100 GB) or from $130 per 30-day port; Proxy-Cheap "Starts at $5.99/GB" — [DataImpulse](https://dataimpulse.com/), [Evomi](https://evomi.com/pricing), [Infatica](https://infatica.io/pricing/), [IPRoyal mobile](https://iproyal.com/pricing/mobile-proxies/), [Proxy-Cheap](https://www.proxy-cheap.com/pricing) (all 2026-10-12)

#### Datacenter
- Bright Data: shared $1.4/IP (10) → $0.9 (1,000); dedicated $2.2 → $1.3; per GB PAYG $0.6 → $0.51 (1 TB for $499) → $0.45 (2 TB for $999) → $0.42 (5 TB for $1,999). 100 GB fair use per IP — [Bright Data datacenter, 2026-10-12](https://brightdata.com/pricing/proxy-network/datacenter-proxies)
- Oxylabs: dedicated $2 → $1.2/IP (2.08K+); shared per-IP $1 → $0.6 (4.17K+); per GB $0.6 → $0.40/GB (6.25K+ GB) — [Oxylabs datacenter, 2026-10-12](https://oxylabs.io/pricing/datacenter-proxies)
- Webshare shared datacenter list: 10 proxies free; $0.0299/proxy/month at 100 proxies → $0.0224 (10,000) → $0.0179 (60,000–100,000); bandwidth "250GB - Unlimited", subject to fair use — [Webshare pricing, 2026-10-12](https://www.webshare.io/pricing)
- Rayobyte rotating datacenter PAYG $0.30/GB (3–50 GB) → $0.23/GB (351–1,000 GB); monthly $0.0130 → $0.0080/GB; static datacenter enterprise "from $0.85/IP"; semi-dedicated US $1 → $0.75/IP, enterprise from $0.50/IP — [Rayobyte pricing, 2026-10-12](https://rayobyte.com/pricing/)
- Evomi $0.45/GB PAYG → $0.30/GB (1,000 GB, $300/mo); DataImpulse $0.50 → $0.45/GB (1 TB); Infatica shared $0.60 → $0.30/GB (2,000 GB), dedicated $1–$1.50/IP; Geonode "from $0.14/GB"; Proxy-Cheap IPv4 $1.49/IP/month, IPv6 $0.15 — [Evomi](https://evomi.com/pricing), [DataImpulse](https://dataimpulse.com/), [Infatica](https://infatica.io/pricing/), [Geonode](https://geonode.com/pricing), [Proxy-Cheap](https://www.proxy-cheap.com/pricing) (all 2026-10-12)

#### Reseller, wholesale and enterprise terms disclosed
- Rayobyte: "Rotating ISP and Rotating Datacenter pay-as-you-go can be resold at published prices." Scale Residential "is not available for resale". Monthly ISP and datacenter plans "are priced for internal use", and reselling at monthly volume needs a custom quote. Sales "can offer rates below list for higher volume or commitments" — [Rayobyte pricing, 2026-10-12](https://rayobyte.com/pricing/)
- PacketStream: resellers need a $500 minimum purchase and set their own retail prices; no wholesale per-GB rate is published — [PacketStream, 2026-10-12](https://packetstream.io/)
- Infatica, Proxy-Cheap ("White Label Reseller"), Geonode and DataImpulse link to reseller programs, but their pricing pages show no reseller terms. Infatica only mentions a referral program "up to 30%" — [Infatica](https://infatica.io/pricing/), [Proxy-Cheap](https://www.proxy-cheap.com/pricing), [Geonode](https://geonode.com/pricing), [DataImpulse](https://dataimpulse.com/) (all 2026-10-12)
- Bright Data: "Contact sales for usage above 1 TB", with "custom price per GB" — [Bright Data residential, 2026-10-12](https://brightdata.com/pricing/proxy-network/residential-proxies)

### Inferences
- A defensible residential COGS assumption for 1–10 TB a month from EWDCI-listed vendors (see section 3) is $2.00–$2.50/GB for premium pools (Decodo, Oxylabs) and $0.70–$1.00/GB for budget pools (Rayobyte Premium, DataImpulse). Negotiated contracts below $1/GB are plausible, because SOAX ($0.35–$0.85) and Rayobyte (from $0.50) publish enterprise floors at that level.
- After the 2023–2025 price war, prices are flat to slightly rising (Proxyway). The removal of IPIDEA and NetNut supply (section 3) is a further upward pressure, so a margin model should not assume continued declines.
- Rayobyte's Scale Residential (about $0.01/GB) is priced by concurrency rather than bandwidth, has no targeting, and cannot be resold, so it is not a valid wholesale benchmark for a reseller.
- ISP and dedicated datacenter costs are fixed per IP per month with fair-use caps (100 GB per IP at Bright Data, 50 GB at Oxylabs). Goodfetch Cloud would carry utilization risk on them, unlike per-GB residential.

### Gaps
- Per-vendor reseller or white-label wholesale rates (IPRoyal's reseller page returned 404 and Infatica's returned 502). Hacker News and Reddit threads quoting wholesale rates could not be searched once the search budget ran out, so the only second-hand wholesale figure here is Proxyway's grey-market range (unverified).
- Decodo mobile and datacenter pages (404), SOAX ISP and datacenter (not on its pricing page), and Bright Data's own mobile-specific tiers if they exist.
- SOAX's geo-tier definitions (which countries fall in tiers 1, 2 and 3) were not captured.
- Bright Data's "from $5.88/GB" meta description does not match its plan table, and the post-coupon rate after 3 months is not stated explicitly (the struck-through list prices suggest $5–$8/GB).

## 2. Proxies: what scraping and browser platforms charge per GB, and the implied markup

### Takeaway
Platforms that resell proxies charge customers $6–$12/GB for residential bandwidth: Apify $7–$8, Browserbase $10–$12, Steel $6–$10, Bright Data's own Browser API $7–$8, and Browserless about $4–$12 in unit terms. That is about 3–6× a premium wholesale-scale rate ($2.00–$2.50/GB) and 8–17× a budget rate ($0.70–$0.80/GB). ZenRows is the outlier: its flat 25,000 credits/GB works out to $10.56/GB on its smallest plan but only $1.67/GB on its largest. Firecrawl doesn't sell bandwidth at all; it charges per page.

### Cited Findings
| Platform | Customer price for residential proxy bandwidth | Other relevant unit prices | Source |
|---|---|---|---|
| Apify | $8/GB (Free, Starter), $7.5/GB (Scale), $7/GB (Business) | SERP proxy $2.5 / $2 / $1.7 per 1,000; datacenter IPs included 5 / 30 / 200 / 500, extra $1 / $0.8 / $0.6 per IP; compute $0.2 → $0.13 per CU; plans $0 / $19 / $199 / $999 per month | [Apify pricing, 2026-10-12](https://apify.com/pricing) |
| Browserbase | $12/GB (Developer, $20/mo, 1 GB included); $10/GB (Startup, $99/mo, 5 GB included); Scale "usage-based" | Browser hour $0.12 / $0.10; captcha solving "Yes" on paid plans | [Browserbase pricing, 2026-10-12](https://www.browserbase.com/pricing) |
| Steel | $10/GB (Launch, $0 + usage); $6/GB (Scale, $250/mo + usage) | Browser hour $0.10 / $0.08; CAPTCHA $3 / $1 per 1,000 | [Steel pricing, 2026-10-12](https://steel.dev/pricing) |
| Browserless | "6 Units per MB" of residential proxy; units cost $0.0020 / $0.0017 / $0.0015 on overage (Prototyping $25 for 20k, Starter $140 for 180k, Scale $350 for 500k, billed annually) | Captcha "10 Units per successful solve"; a unit is up to 30 s of browser time | [Browserless pricing, 2026-10-12](https://www.browserless.io/pricing) |
| Bright Data Browser API | $8/GB PAYG; $499/mo for 71 GB | Proxies, unlocking and CAPTCHA solving included | [Bright Data Browser API, 2026-10-12](https://brightdata.com/pricing/scraping-browser) |
| Bright Data Web Unlocker | $1.5 per 1,000 requests PAYG ("Pay only for success"); $499/mo for 383K requests, then $1.3 per 1,000 | CAPTCHA solving included | [Bright Data Web Unlocker, 2026-10-12](https://brightdata.com/pricing/web-unlocker) |
| Oxylabs Web Unblocker | $7.2/GB (up to 4 GB) → $3/GB (833+ GB) | — | [Oxylabs pricing, 2026-10-12](https://oxylabs.io/pricing) |
| ZenRows | Residential bandwidth "25,000 credits per GB", described as a "Flat rate; roughly 1,000 hard-protected pages". Plans: $19 for 45,000 credits up to $2,999 for 45,000,000 credits | Basic request 1 credit; JS rendering 5; premium proxy 10; JS + premium 25; failed requests not charged. Scraping Browser: bandwidth plus 5 credits per minute | [ZenRows pricing, 2026-10-12](https://www.zenrows.com/pricing) |
| Firecrawl | No per-GB proxy price is listed | 1 credit per page scraped or crawled; +4 credits per page for JSON formats; search 2 credits per 10 results; browser "Interact" 2 credits per minute; plans $16 (5,000 credits) to $599 (1M credits) per month on annual billing, i.e. about $0.60–$3.80 per 1,000 credits | [Firecrawl pricing, 2026-10-12](https://www.firecrawl.dev/pricing) |

### Inferences
- Credit-to-dollar arithmetic from the published plans (mine, not stated by the vendors):
  - ZenRows' 25,000 credits/GB costs $10.56/GB on the $19 plan ($0.000422 per credit), $6.90 on $69/250k, $4.15 on $199/1.2M, $2.75 on $549/5M and $1.67 on $2,999/45M.
  - Browserless' 6 units/MB is 6,000–6,144 units/GB, which costs about $9.0–$12.3/GB at overage rates and about $4.2–$7.7/GB at each plan's included-unit rate. Its captcha price works out to $7–$20 per 1,000 solves.
- Implied markup over cost: Apify $7–$8/GB is 2.8–4.0× Decodo's or Oxylabs' 1 TB rates ($2.00–$2.50) and 8.8–11.4× Rayobyte's or DataImpulse's ($0.70–$0.80). Browserbase $10–$12/GB is 4–6× and 12.5–17× respectively. Steel $6–$10/GB is 2.4–5× premium.
- ZenRows' top tier ($1.67/GB) sits below Oxylabs' and Decodo's retail volume rates, which suggests either negotiated wholesale under $1.67/GB or cross-subsidy from per-request revenue.
- ZenRows' "roughly 1,000 hard-protected pages" per GB means about 1 MB per protected page. At $0.70–$2.50/GB that is $0.0007–$0.0025 of bandwidth COGS per page, against about $0.007–$0.008 per page billed at Apify's $7–$8/GB. This is a vendor estimate, and page weight varies widely.

### Gaps
- Hyperbrowser's pricing page didn't render, and Kernel, Anchor Browser, Spider and Zyte were not fetched.
- Firecrawl's cost for stealth or enhanced-proxy mode isn't listed on its pricing page.
- No source states which upstream proxy vendor any of these platforms uses, or what they pay.

## 3. Proxies: KYC, acceptable use and ethical sourcing for a reseller

### Takeaway
Mainstream vendors gate residential and mobile access behind KYC: Bright Data requires a live video identity check, a compliance officer's review and a declared use case before granting any access. They also block categories of targets, such as banks, government sites and login pages at Rayobyte, and adult content, gambling and crypto at Bright Data. Resale is allowed only on stated terms (Rayobyte, PacketStream). 2026 showed that sourcing claims and industry membership are weak evidence: Google disrupted IPIDEA in January, and the FBI seized NetNut on 2 July after it was tied to a botnet of about 2 million devices. Google says many popular brands were white-labeling NetNut.

### Cited Findings
- Bright Data: before using the residential or mobile network "a Bright Data representative will ask you to go through a short compliance process", which "may include a quick intro video call" — [Bright Data residential, 2026-10-12](https://brightdata.com/pricing/proxy-network/residential-proxies). Its KYC page describes "live video identity verification and a personal review by an experienced compliance officer". Every potential customer must give "a detailed account of their intended use case before any access is granted", activity is monitored against the declared use case, and prohibited uses include "adult content, gambling, cryptocurrency, and more" — [Bright Data KYC, 2026-10-12](https://brightdata.com/trustcenter/kyc)
- Bright Data's trust center says it uses Bright SDK to "ethically acquire Residential and Mobile IPs". It lists CSA STAR, ISO 27001/27017/27018/27701, SOC 2 and SOC 3, a PwC-audited compliance report, and whitelisting by AppEsteem, Avast, AVG, Microsoft Defender and McAfee. EWDCI isn't mentioned, and nothing is said about resellers — [Bright Data trust center, 2026-10-12](https://brightdata.com/trustcenter)
- Rayobyte: "Banks, government sites, login pages and sites that report abuse are blocked", for every customer. Scale Residential is sold only to verified businesses after a compliance review. Sourcing: "Every residential IP comes from a device whose owner opted in", through a consent screen with a visible opt-out — [Rayobyte residential, 2026-10-12](https://rayobyte.com/products/residential-proxies/)
- Proxyway 2026: some vendors now require KYC "to access ASN targeting" and others removed the feature; resellers such as Maskify advertise "no-KYC crypto payments". Sourcing methods include SDKs, partner apps (Oxylabs via Honeygain and JumpTask, IPRoyal via Pawns, Evomi via Earn.fm) and proxyware. Advertising "ethical IP sourcing" does not by itself make it real — [Proxyway 2026](https://proxyway.com/research/proxy-market-research-2026)
- Massive: "Every IP is opted in through the Massive SDK"; "AppEsteem certified" (independent review of SDK consent); "SOC 2 Type 1 audited" — [Massive pricing, 2026-10-12](https://www.joinmassive.com/pricing)
- DataImpulse: "First-party residential IP pools. No third-party resellers involved", "sourced directly and legally from people with full user consent" — [DataImpulse, 2026-10-12](https://dataimpulse.com/). Webshare: residential pool "ethically sourced from carefully selected partners", mostly "Tier A model proxies" — [Webshare residential, 2026-10-12](https://www.webshare.io/residential-proxy). PacketStream pays its peer-to-peer "Packeters" "$0.10 for every GB" — [PacketStream, 2026-10-12](https://packetstream.io/)
- EWDCI (Ethical Web Data Collection Initiative, part of the i2Coalition) has four principles: legality, social responsibility, ethics and ecosystem management. It awards an "EWDCI Certified" trust seal. Its participating members are Coresignal, Zyte, Proxyrack, Oxylabs, Decodo, Kadoa, Evomi, DataImpulse, Rayobyte, Byteful, ProxyEmpire, Proxy-Seller and SerpApi. The page doesn't address residential IP consent directly — [EWDCI, 2026-10-12](https://www.ethicalwebdata.com/). Bright Data, SOAX, IPRoyal, Webshare, Massive and Infatica are not in that list.
- NetNut: on 2 July 2026 "the FBI said it worked with industry partners to seize hundreds of domains associated with NetNut", which is operated by Alarum Technologies (NASDAQ: ALAR). Security firms tied it to the Popa botnet of "at least two million devices that have been compromised by malicious software" (smart TVs and streaming boxes). Google's Threat Intelligence Group saw 316 threat-actor clusters using suspected NetNut exit nodes in one week of June 2026 and said "many popular residential proxy brands are in fact whitelabeling the NetNut botnet". Spur found proxy SDKs in 42% of LG webOS apps — [KrebsOnSecurity, July 2026](https://krebsonsecurity.com/2026/07/fbi-seizes-netnut-proxy-platform-popa-botnet/). On 2026-10-12, netnut.io/pricing still showed the FBI seizure notice — [netnut.io/pricing](https://netnut.io/pricing/)
- IPIDEA: in late January 2026 Google's Threat Intelligence Group disrupted IPIDEA, which it described as one of the largest residential proxy networks. At least 13 proxy brands were taken offline, over 600 Android apps and 3,075 Windows files were tied to it, and Play Protect now removes apps with IPIDEA SDKs. Google expected knock-on effects on affiliated services "through reseller agreements" — [The Hacker News, Jan 2026](https://thehackernews.com/2026/01/google-disrupts-ipidea-one-of-worlds.html); [Security Affairs](https://securityaffairs.com/187463/security/google-targets-ipidea-in-crackdown-on-global-residential-proxy-networks)

### Inferences
- Reselling makes Goodfetch Cloud the customer of record for the upstream vendor's KYC and acceptable-use policy. It would need its own customer KYC and per-tenant target blocking at least as strict as its upstream: no banks, government sites or login pages (Rayobyte), and no adult, gambling or crypto uses (Bright Data). It would also need to declare reselling as its use case, and contract only where resale is permitted.
- EWDCI membership is weak evidence on its own; one search snippet (unverified) said NetNut joined the coalition in 2024. Stronger signals are first-party SDK consent with independent review (AppEsteem: Massive; whitelisting: Bright Data), third-party audits (Bright Data's PwC report, SOC 2), a contractual right to audit sourcing, and indemnity.
- Supply concentration is a real risk. Google says many brands white-labeled NetNut, so a reseller of a reseller can lose its pool overnight. Goodfetch Cloud should keep at least two upstream vendors behind its proxy interface.

### Gaps
- No vendor's specific KYC terms for resellers were retrieved: Bright Data's trust center doesn't mention resellers, and Oxylabs' and Decodo's KYC pages were not located before the search budget ran out.
- EWDCI's member list is a logo list with no certification dates. Whether NetNut was ever a member is unverified.
- Alarum's status after July 2026, and any charges, are unknown.

## 4. Captcha and challenge solving: price per 1,000, success rates and latency

### Takeaway
Token-based solving of reCAPTCHA v2/v3 and Cloudflare Turnstile lists at $0.50–$3.00 per 1,000. NextCaptcha, CapMonster Cloud and SolveCaptcha are cheapest at $0.50–$1.30, CapSolver is at $0.80–$1.20, and 2Captcha, Anti-Captcha and DeathByCaptcha are at $1–$2.89 (2Captcha quotes euros on its site). reCAPTCHA Enterprise v3 costs $1.50–$5. hCaptcha is not listed by any of the mainstream solvers fetched. In a September 2026 third-party test, the token types were solved 86–100% of the time, but average latency ranged from 6 to 93 s, and accuracy on hard image captchas was only 14–64%.

### Cited Findings

#### Price per 1,000 (vendor pages)
| Type | 2Captcha (EUR on site) | Anti-Captcha | CapSolver | CapMonster Cloud | NextCaptcha | DeathByCaptcha |
|---|---|---|---|---|---|---|
| Image / text | €0.5–€1 | $0.5–$0.7 | ImageToText $0.4 | Text $0.30 | — | $0.99–$2 |
| reCAPTCHA v2 | €0.99–€2.8 | $0.95–$2 | $0.8 (image recognition $0.4 per 1,000 images) | $0.60 (tokens); $0.04 (images, proxy included) | $0.5 (<6 s) | $2.89 |
| reCAPTCHA v3 | €1.4 (score ≤0.3); €2.8 (score >0.3) | $1–$2 (by score) | $1.0 | $0.90 | $0.6–$1.0 (<3 s) | $2.89 |
| reCAPTCHA Enterprise | €0.99–€2.8 | $5 (v2 and v3) | v2 Ent $1.0; v3 Ent $3.0 | v2 Ent $1.00; v3 Ent $1.50 | v2 Ent $0.6–$1.0 | — |
| Cloudflare Turnstile | €1.4 | $2 | $1.2 | $1.30 | $0.6 (<3 s) | $2.89 |
| Cloudflare challenge page | not listed | not listed | $1.2 (<10 s) | "Cloudflare Bot Challenge" $1.30 | — | — |
| GeeTest | €2.8 | $1.8 | v3/v4 $1.2 | $1.20 | — | v3/v4 $2.89 |
| AWS WAF / Amazon | €1.4 | $2 | $2.0 | $1.40 | — | — |
| Arkose / FunCaptcha | €1.4–€50 | $3 | — | — | — | — |
| DataDome | €1.4 | — | task listed, no price shown | listed, no price shown | — | — |
| hCaptcha | not listed | not listed | not listed | not listed | not listed | not listed |
| Source (2026-10-12) | [2Captcha pricing](https://2captcha.com/pricing); [2Captcha home](https://2captcha.com/) | [Anti-Captcha pricing](https://anti-captcha.com/pricing) | [CapSolver home](https://www.capsolver.com/); [CapSolver docs](https://docs.capsolver.com/en/guide/getting-started/) | [CapMonster prices](https://capmonster.cloud/en/prices) | [NextCaptcha](https://nextcaptcha.com/) | [DeathByCaptcha](https://deathbycaptcha.com/) |

- 2Captcha also lists €0.45 for audio and rotate captchas, €1.1 for click, slider and bounding-box captchas, €2.8 for Imperva, Tencent, Alibaba, TikTok and others, and €0 for Akamai, Kasada and PerimeterX (meaning unclear). "Solving speed" shows "0 sec." for every type, and hCaptcha is absent from its supported-captcha list — [2Captcha pricing, 2026-10-12](https://2captcha.com/pricing); [2Captcha home, 2026-10-12](https://2captcha.com/)
- 2Captcha's USD rates as cited in a September 2026 article: reCAPTCHA v2 $1–$2.99, Turnstile $1.45, image $0.5–$1, GeeTest v4 $2.99, audio $0.50; minimum top-up $3 — [HasData, updated 2026-09-25](https://hasdata.com/blog/captcha-solving)
- Anti-Captcha applies "automatic discounts based on your daily captcha volumes" and gives "5 s" as its speed for every type — [Anti-Captcha, 2026-10-12](https://anti-captcha.com/pricing)
- CapMonster Cloud claims success rates of 95–99% per type, and mentions a "Bonus program for large volumes" and "Personalized terms for large businesses" without figures — [CapMonster, 2026-10-12](https://capmonster.cloud/en/prices)
- NopeCHA is a subscription with daily caps: $4.99/mo for 2,000 a day, $19.99 for 20,000, $49.99 for 80,000, and $99.99 for 200,000 a day. Credits refill every 23 hours, and "One credit processes up to 16 images". It covers reCAPTCHA v2/v3, Turnstile, FunCaptcha, AWS WAF, GeeTest, PerimeterX, Lemin and text captchas (no hCaptcha), via an extension or a Token API — [NopeCHA pricing, 2026-10-12](https://nopecha.com/pricing); [NopeCHA home, 2026-10-12](https://nopecha.com/)
- DeathByCaptcha claims average solve times of 1 s (normal), 4 s (reCAPTCHA) and 10 s (others), "Over 90% Correctness", and a 10% discount for crypto payments — [DeathByCaptcha, 2026-10-12](https://deathbycaptcha.com/)
- SolveCaptcha's published rates as cited: reCAPTCHA v2 $0.55, Invisible $0.55–$0.8, Turnstile $0.80, image $0.35, GeeTest v4 $0.80 — [HasData, updated 2026-09-25](https://hasdata.com/blog/captcha-solving)
- Reseller prices for comparison: Steel charges $3 per 1,000 CAPTCHAs (Launch) or $1 (Scale) — [Steel, 2026-10-12](https://steel.dev/pricing). Browserless charges 10 units per successful solve — [Browserless, 2026-10-12](https://www.browserless.io/pricing). Bright Data bundles solving into Web Unlocker at $1.3–$1.5 per 1,000 requests — [Bright Data Web Unlocker, 2026-10-12](https://brightdata.com/pricing/web-unlocker)

#### Success rates and latency (third-party)
- HasData benchmark: 50 challenges per type, Python 3.13.7, last updated 2026-09-25. Average time to solve in seconds (success rate):

| Service | reCAPTCHA v2 | Invisible reCAPTCHA | Turnstile | Image | GeeTest v4 |
|---|---|---|---|---|---|
| 2Captcha | 50.71 (100%) | 19.07 (100%) | 16.96 (100%) | 2.33 (58%) | 14.38 (100%) |
| CapMonster | 32.23 (100%) | 13.77 (100%) | 6.24 (100%) | 3.28 (14%) | 16.2 (100%) |
| DeathByCaptcha | 34.54 (100%) | 33.09 (100%) | 13.07 (100%) | 2.13 (64%) | 36.25 (100%) |
| SolveCaptcha | 84.8 (100%) | 71.2 (100%) | 17.24 (100%) | 8.46 (48%) | 20.5 (86%) |
| Anti-Captcha | 92.86 (86%) | 49.87 (100%) | 20.33 (100%) | 13.82 (22%) | 42.34 (84%) |

  2Captcha alone handled audio captchas (27% success, 14.27 s). reCAPTCHA v3 and hCaptcha were not tested, and the article contradicts itself on SolveCaptcha's Invisible score (94% in the summary, 100% in the table) — [HasData](https://hasdata.com/blog/captcha-solving)
- AIMultiple tested 100 requests per vendor against Cloudflare "Under Attack" mode, which forces Turnstile on every request. Bright Data, Decodo and Oxylabs unblockers each succeeded 67% of the time ("roughly interchangeable"). The article was updated 2026-08-13 and last updated 2026-10-11 — [AIMultiple](https://aimultiple.com/captcha-solving-services)
- AIMultiple's price table still quotes hCaptcha prices for 2Captcha (~$2.99), CapSolver ($2.00–$3.00) and YesCaptcha ($2.00–$2.50), which conflicts with the vendors' current pages; treat them as stale and **unverified** — [AIMultiple](https://aimultiple.com/captcha-solving-services)

### Inferences
- A reasonable list-price COGS for managed token solving is about $0.60–$1.30 per 1,000 for reCAPTCHA v2/v3 and Turnstile (CapMonster, CapSolver, NextCaptcha), $1.20–$1.30 for Cloudflare challenge pages, and $1.50–$3.00 for reCAPTCHA v3 Enterprise. Steel resells at $1–$3 per 1,000 and Browserless at $7–$20 equivalent, so solving supports a markup like proxies.
- Per protected page, a Turnstile solve ($1.2 per 1,000 = $0.0012) costs about the same as the residential bandwidth for that page (about 1 MB at $0.70–$2.50/GB = $0.0007–$0.0025). Latency, at 6–93 s per token, is likely the bigger cost for hosted browsers billed per browser-hour.
- Because no mainstream solver lists hCaptcha, a managed hCaptcha capability would have to come from Goodfetch's own solver, which ADR 0018 places in a Private component, or from vision-model solving. The "buy from a solver" COGS model doesn't cover it.
- Solve rate is not pass rate: unblockers reached only 67% on Cloudflare "Under Attack", and residential proxies only 74% against protected targets (section 1). A margin model should price retries.

### Gaps
- No dated notice explaining hCaptcha's absence was retrieved: 2Captcha's hCaptcha doc URL returned 404 and CapMonster's returned 403, and search was unavailable.
- 2Captcha's USD pricing was not seen directly (its site showed euros), and the EUR to USD rate is not given here.
- No independent benchmark of reCAPTCHA v3 score quality, Enterprise variants, or AWS WAF or DataDome solving was found.
- Volume discount schedules for 2Captcha, CapSolver and CapMonster are not published.

## 5. LLM tokens: Anthropic, OpenAI and Google (closed models)

### Takeaway
As of 12 October 2026 the current lineups are:
- **Anthropic:** Claude Opus 5.5 ($4/$20 per million input/output tokens), Sonnet 5.5 ($2/$10), Haiku 5.5 ($0.10/$0.50 for prompts up to 100k tokens) and Fable 5.1 ($10/$50). Cache reads are 0.05× input on Opus and Sonnet 5.5.
- **OpenAI:** GPT-6 Astra ($10/$50), GPT-6.1 Sol ($2/$10) and GPT-6 Luna ($0.10/$0.50), all supporting computer use, with GPT-5.x still sold.
- **Google:** Gemini 3.8 Flash at $0.75/$3.75 through 31 December 2026, doubling to $1.50/$7.50 on 1 January 2027; Gemini 3.1 Flash-Lite at $0.25/$1.50; Gemini 3.1 Pro Preview at $2/$12.

Batch processing halves prices at all three. Claude's tokenizer since 4.7 produces about 30% more tokens for the same text, so per-token prices are not directly comparable across vendors.

### Cited Findings

#### Anthropic (Claude API, first party)
| Model (API ID) | Input | 5-min cache write | 1-h cache write | Cache hit | Output | Batch input / output |
|---|---|---|---|---|---|---|
| Claude Fable 5.1 (`claude-fable-5-1`) | $10 | $12.50 | $20 | $0.25 (0.025×) | $50 | $5 / $25 |
| Claude Mythos 5.1 (limited availability) | $10 | $12.50 | $20 | $0.25 | $50 | $5 / $25 |
| Claude Opus 5.5 (`claude-opus-5-5`) | $4 | $5 | $8 | $0.20 (0.05×) | $20 | $2 / $10 |
| Claude Opus 5 / 4.8 / 4.7 / 4.6 / 4.5 | $5 | $6.25 | $10 | $0.50 | $25 | $2.50 / $12.50 |
| Claude Sonnet 5.5 (`claude-sonnet-5-5`) | $2 | $2.50 | $4 | $0.10 (0.05×) | $10 | $1 / $5 |
| Claude Sonnet 5 | $2 | $2.50 | $4 | $0.20 | $10 | $1 / $5 |
| Claude Sonnet 4.6 / 4.5 | $3 | $3.75 | $6 | $0.30 | $15 | $1.50 / $7.50 |
| Claude Haiku 5.5 (`claude-haiku-5-5`), prompts ≤100k tokens | $0.10 | $0.125 | $0.20 | $0.01 | $0.50 | $0.05 / $0.25 |
| Claude Haiku 5.5, prompts >100k tokens | $0.50 | $0.625 | $1 | $0.05 | $2.50 | $0.25 / $1.25 |
| Claude Haiku 4.5 | $1 | $1.25 | $2 | $0.10 | $5 | $0.50 / $2.50 |

All prices are per million tokens — [Anthropic pricing, 2026-10-12](https://platform.claude.com/docs/en/about-claude/pricing); IDs, 1M context, 128K max output and retirement "not sooner than" September–October 2027 — [Anthropic models overview, 2026-10-12](https://platform.claude.com/docs/en/about-claude/models/overview)

- Sonnet 5's introductory $2/$10 "is now the standard price"; the planned rise to $3/$15 on 1 September 2026 "will not occur" — [Anthropic pricing](https://platform.claude.com/docs/en/about-claude/pricing)
- Claude 4.6 and later models (except Haiku 5.5) include the full 1M-token context "at standard pricing" — [Anthropic pricing](https://platform.claude.com/docs/en/about-claude/pricing)
- Tokenizer: Claude 4.7 and later models "use a newer tokenizer… approximately 30% more tokens for the same text" — [Anthropic pricing](https://platform.claude.com/docs/en/about-claude/pricing)
- Fast mode (research preview, first-party API only): Opus 5.5 costs $8/$40, Opus 5 and 4.8 cost $10/$50. It is not available with the Batch API. The US-only `inference_geo` setting multiplies all token prices by 1.1× — [Anthropic pricing](https://platform.claude.com/docs/en/about-claude/pricing)
- Computer use: declaring `computer_toolset_20260801` adds "about 4,500 input tokens" (about 410 fewer with zoom disabled). `browser_toolset_20260801` adds "about 6,600" (plus about 880 with all optional members). The older `computer_20251124` and `computer_20250124` versions add 466–499 system-prompt tokens and about 735 per tool definition. Screenshots are billed as image input. The tool-use system prompt is 286 tokens on Opus 5.5, Sonnet 5.5 and Haiku 5.5 — [Anthropic pricing](https://platform.claude.com/docs/en/about-claude/pricing)
- Server tools: web search costs "$10 per 1,000 searches" plus tokens; web fetch has "no additional charges"; code execution is free when used with web search or fetch, otherwise 1,550 free hours a month, then $0.05 per hour per container. Managed Agents costs $0.08 per session-hour plus tokens — [Anthropic pricing](https://platform.claude.com/docs/en/about-claude/pricing)

#### OpenAI
| Model | Input | Cached input | Output | Batch or Flex (input / output) | Fast mode, formerly Priority (input / output) |
|---|---|---|---|---|---|
| gpt-6-astra (flagship) | $10.00 | $1.00 | $50.00 | not captured | not captured |
| gpt-6.1-sol | $2.00 | $0.10 | $10.00 | not captured | not captured |
| gpt-6-sol | $2.00 | $0.20 | $10.00 | not captured | not captured |
| gpt-6-luna (cheapest text tier) | $0.10 | $0.01 | $0.50 | not captured | not captured |
| gpt-5.6-sol | $4.00 | $0.40 | $20.00 | $2.00 / $10.00 | $8.00 / $40.00 |
| gpt-5.6-terra | $2.00 | $0.20 | $12.00 | $1.00 / $6.00 | $4.00 / $24.00 |
| gpt-5.6-luna | $0.20 | $0.02 | $1.20 | $0.10 / $0.60 | $0.40 / $2.40 |
| gpt-5.5 (<272K context) | $5.00 | $0.50 | $30.00 | $2.50 / $15.00 | $12.50 / $75.00 |
| gpt-5.4 / -mini / -nano | $2.50 / $0.75 / $0.20 | $0.25 / $0.075 / $0.02 | $15.00 / $4.50 / $1.25 | half of standard | $5.00 / $30.00 (gpt-5.4) |
| gpt-5 / -mini / -nano | $1.25 / $0.25 / $0.05 | $0.125 / $0.025 / $0.005 | $10.00 / $2.00 / $0.40 | half of standard | $2.50 / $20.00 (gpt-5) |
| gpt-4.1 / -mini / -nano | $2.00 / $0.40 / $0.10 | $0.50 / $0.10 / $0.025 | $8.00 / $1.60 / $0.40 | — | — |

All prices are per million tokens — [OpenAI pricing, 2026-10-12](https://developers.openai.com/api/docs/pricing)

- The models page lists `gpt-6-astra` ("Our most capable model for the most demanding work"), `gpt-6.1-sol` (mid tier) and `gpt-6-luna` ("Our most efficient model for focused, high-volume tasks") as the current text models. Only these three list computer use. Knowledge cutoffs are 30 April 2026 (Astra, Sol) and 18 May 2026 (Luna). `computer-use-preview` no longer appears on the models page — [OpenAI models, 2026-10-12](https://developers.openai.com/api/docs/models)
- Computer use guide: "For GPT-6 Astra, we recommend code execution. The `computer` tool remains supported as an alternative." Examples use `gpt-6.1-sol`. No per-call fee is mentioned — [OpenAI computer use guide, 2026-10-12](https://developers.openai.com/api/docs/guides/tools-computer-use)
- Priority processing was renamed **Fast mode** on 30 July 2026 — [OpenAI pricing](https://developers.openai.com/api/docs/pricing)
- Built-in tools: web search $10 per 1,000 calls plus search content tokens at model rates. The web search preview on non-reasoning models costs $25 per 1,000 with free content tokens. File search costs $2.50 per 1,000 calls. Containers cost $0.03 (1 GB) to $1.92 (64 GB) per 20-minute session — [OpenAI pricing](https://developers.openai.com/api/docs/pricing)

#### Google Gemini API (paid tier)
| Model (ID) | Input | Output | Cached input | Batch input / output |
|---|---|---|---|---|
| Gemini 3.8 Flash (`gemini-3.8-flash`) | $0.75 through 2026-12-31; $1.50 from 2027-01-01 | $3.75; then $7.50 | $0.075; then $0.15 | $0.375 / $1.875; then $0.75 / $3.75 |
| Gemini 3.6 Flash (`gemini-3.6-flash`) | same as 3.8 Flash | same | same | same |
| Gemini 3.5 Flash-Lite (`gemini-3.5-flash-lite`) | $0.30 | $2.50 | $0.03 | $0.15 / $1.25 |
| Gemini 3.1 Flash-Lite (`gemini-3.1-flash-lite`) | $0.25 (text, image, video) | $1.50 | $0.025 | $0.125 / $0.75 |
| Gemini 3.1 Pro Preview (`gemini-3.1-pro-preview`) | $2.00 (≤200k); $4.00 (>200k) | $12.00; $18.00 | $0.20; $0.40 (+$4.50 per MTok per hour of storage) | $1.00 / $6.00 (≤200k) |
| Gemini 3 Flash Preview | $0.50 | $3.00 | $0.05 | $0.25 / $1.50 |
| Gemini 2.5 Pro / 2.5 Flash / 2.5 Flash-Lite | $1.25 / $0.30 / $0.10 | $10.00 / $2.50 / $0.40 | $0.125 / $0.03 / $0.01 | half of standard |

All prices are per million tokens — [Gemini API pricing, 2026-10-12](https://ai.google.dev/gemini-api/docs/pricing)

- Computer use is "Charged as regular tokens per model pricing", and no dedicated computer-use model appears on the page — [Gemini API pricing](https://ai.google.dev/gemini-api/docs/pricing)
- Grounding with Google Search: 5,000 free requests a month shared across Gemini 3 and newer models, then $14 per 1,000. On Gemini 2.5 models, 1,500 requests a day are free, then $35 per 1,000 grounded prompts — [Gemini API pricing](https://ai.google.dev/gemini-api/docs/pricing)

### Inferences
- Illustrative cost of 1,000 extraction calls of 10k input and 1k output tokens each, at standard (non-batch, uncached) rates:

  | Model | Cost per 1,000 calls |
  |---|---|
  | Claude Haiku 5.5 | $1.50 |
  | GPT-6 Luna | $1.50 |
  | gpt-oss-120b (Groq, Fireworks, Together) | $2.10 |
  | Gemini 3.1 Flash-Lite | $4.00 |
  | DeepSeek V4.1 Flash, direct, peak hours | $4.20 |
  | Gemini 3.8 Flash in 2026 | $11.25 ($22.50 from 2027) |
  | Claude Haiku 4.5 | $15 |
  | Claude Sonnet 5.5 | $30 |
  | GPT-6.1 Sol | $30 |
  | Claude Opus 5.5 | $60 |
  | GPT-6 Astra | $150 |

  Batch halves every figure. Token counts differ by tokenizer, so these are comparable only for the same token counts.
- For browser agents, caching the toolset matters. The 6,600-token Claude browser toolset costs $0.0132 per uncached call on Sonnet 5.5 but $0.00066 as a cache hit, and $0.0264 versus $0.00132 on Opus 5.5. Long-lived agent loops should be designed around cache hits.
- Gemini 3.8 and 3.6 Flash prices double on 1 January 2027. Any price Goodfetch Cloud sets for hosted models in Q4 2026 on these models should already account for that.

### Gaps
- OpenAI's Batch, Flex and Fast prices for the GPT-6 models, its long-context (>272K) prices for GPT-5.x, and its computer-use token overheads were not captured.
- Vertex AI's Claude and Gemini price tables did not render.
- No source was found on Claude Fable 5.1's suitability or speed for agent loops beyond Anthropic's own description.

## 6. LLM tokens: open-weight models on Bedrock, Together, Fireworks, Groq, Cerebras, DeepInfra, DeepSeek, Mistral and OpenRouter

### Takeaway
Open-weight prices cluster around gpt-oss-120b at $0.15/$0.60 per million input/output tokens (Groq, Fireworks and Together; Bedrock in Sydney is $0.1545/$0.618). DeepSeek V4.x Flash costs $0.06–$0.30 input and $0.18–$1.20 output, and Llama 3.3 70B costs $0.10/$0.32 on DeepInfra versus $1.04/$1.04 on Together. DeepInfra is usually cheapest; Bedrock is 2–5× dearer than specialists for DeepSeek but at parity for Mistral Large 3 and gpt-oss. Llama 4 is no longer prominent (Llama 4 Scout is $0.10/$0.30 on DeepInfra). Newer open-weight families (Qwen 3.5–3.8, DeepSeek V4, Kimi K3, GLM 5.3) dominate the price lists.

### Cited Findings
- **Groq:** `openai/gpt-oss-120b` $0.15/$0.60 at 500 tokens/s; `openai/gpt-oss-20b` $0.075/$0.30 at 1,000 tokens/s; `qwen/qwen3.8-27b` (preview) $0.80/$4.00 at 450 tokens/s; `llama-3.1-8b-instant` and `llama-3.3-70b-versatile` are "ContactSales" — [Groq models, 2026-10-12](https://console.groq.com/docs/models). Batch is "a 50% cost discount", with a window of 24 h to 7 days — [Groq batch](https://console.groq.com/docs/batch). Cached input tokens get "a 50% discount", on gpt-oss models only — [Groq prompt caching](https://console.groq.com/docs/prompt-caching)
- **Cerebras:** `gpt-oss-120b` at about 3,000 tokens/s and `qwen-3.8-27b` at about 1,850 tokens/s on shared inference; other model families are on dedicated inference — [Cerebras models, 2026-10-12](https://inference-docs.cerebras.ai/models/overview). The pricing page rendered no prices, only "a one-time $5 promotional credit" — [Cerebras pricing, 2026-10-12](https://www.cerebras.ai/pricing)
- **Fireworks** (input / cached input / output, standard tier): OpenAI GPT OSS 120B $0.15 / $0.015 / $0.60; DeepSeek V4.1 Flash $0.30 / $0.006 / $1.20; GLM 5.3 $1.40 / $0.26 / $4.40; GLM 5.3 Flash $0.15 / $0.03 / $0.50; Kimi K3 $3.00 / $0.30 / $15.00; Qwen 3.8 Max $2.00 / $0.25 / $6.00; MiniMax M3 $0.30 / $0.06 / $1.20; NVIDIA Nemotron 3.5 Lightning 30B A3B $0.05 / $0.01 / $0.20. "(US)" variants cost 1.5× (Kimi K3 US $4.50/$22.50); Priority costs 1.25×. Other models are priced by size: under 4B $0.10, 4B–16B $0.20, over 16B $0.90, MoE up to 56B $0.50, MoE 56.1B–176B $1.20. "Batch inference is billed at 50% of serverless pricing" — [Fireworks serverless pricing, 2026-10-12](https://docs.fireworks.ai/serverless/pricing)
- **Together:** gpt-oss-120B $0.15/$0.60; Llama 3.3 70B $1.04/$1.04; Qwen3 235B A22B Instruct 2507 FP8 $0.20/$0.60; DeepSeek V4 Pro 0813 $1.32 / $0.13 cached / $3.96; DeepSeek V4 Flash 0731 $0.14 / $0.03 / $0.28; DeepSeek V4.1 Flash $0.30 / $0.006 / $1.20; GLM-5.3 $1.40 / $0.26 / $4.40; Kimi K3 promo $2.70/$13.50, "valid until October 11, 2026" (list $3.00/$15.00) — [Together pricing, 2026-10-12](https://www.together.ai/pricing). Batch is "up to 50% off serverless rates", but only for listed models; "Models not listed run at standard rates" — [Together batch docs, 2026-10-12](https://docs.together.ai/docs/batch-inference)
- **DeepInfra** (input / cached / output): DeepSeek-V4-Flash-0731 $0.06 / $0.015 / $0.18; DeepSeek-V4-Pro $1.30 / $0.10 / $2.60; DeepSeek-V3.2 $0.26 / $0.13 / $0.38; DeepSeek-R1-0528 $0.50 / $0.35 / $2.15; Kimi-K3 $2.85 / $0.285 / $14.25; Qwen3.5-397B-A17B $0.45 / $0.22 / $3.00; Qwen3-235B-A22B-Instruct-2507 $0.09 / $0.55; Qwen3-Coder-480B-A35B-Instruct-Turbo $0.30 / $0.10 / $1.00; Qwen3-VL-235B-A22B $0.20 / $0.11 / $0.88; Llama-4-Scout-17B-16E $0.10 / $0.30; Llama-3.3-70B-Instruct-Turbo $0.10 / $0.32; Meta-Llama-3.1-8B-Instruct-Turbo $0.02 / $0.04; Mistral-Small-3.2-24B $0.075 / $0.20; gemma-4-31B-it $0.20 / $0.40. gpt-oss is not listed — [DeepInfra pricing, 2026-10-12](https://deepinfra.com/pricing)
- **DeepSeek, direct:** `deepseek-flash` (DeepSeek-V4.1-Flash) at peak: cache hit $0.006, cache miss $0.30, output $1.20. `deepseek-v4-pro` (V4-Pro-0813) at peak: $0.044 / $1.32 / $3.96. "Off-peak rates are half of the peak rates"; peak hours are 01:00–04:00 and 06:00–10:00 UTC on weekdays. 1M context, 384K maximum output — [DeepSeek pricing, 2026-10-12](https://api-docs.deepseek.com/quick_start/pricing)
- **Amazon Bedrock**, open-weight, on-demand, US East unless noted: DeepSeek v3.2 $0.62/$1.85; Mistral Large 3 $0.50/$1.50; Qwen3 Next 80B A3B $0.15/$1.20; Qwen3 VL 235B A22B $0.53/$2.66; Qwen3 Coder Next $0.50/$1.20; gpt-oss-20b and gpt-oss-120b (Sydney) $0.0721/$0.3090 and $0.1545/$0.6180. "Priority tier pricing is at 75% premium to Standard"; Flex and Batch are at 50% off. Llama 4 prices did not render — [Bedrock pricing, 2026-10-12](https://aws.amazon.com/bedrock/pricing/)
- **Mistral, direct:** "Mistral Large costs $0.5 /M tokens in and $1.5 /M tokens out"; batch is 50% off and cached input "up to 90%" off. Prices for Medium, Small, Devstral, Codestral and others were not listed on the page — [Mistral pricing, 2026-10-12](https://mistral.ai/pricing)
- **OpenRouter:** "We pass through the pricing of the underlying model providers without any markup", plus a 5.5% fee ($0.80 minimum) on credit purchases (5% for crypto). BYOK on pay-as-you-go includes $25,000 a month free, then a fee of 5% of the normal OpenRouter cost — [OpenRouter FAQ, 2026-10-12](https://openrouter.ai/docs/faq)

### Inferences
- For extraction and repair at scale, the cheapest hosted open-weight options are DeepSeek V4 Flash on DeepInfra ($0.06/$0.18), DeepSeek's own off-peak pricing ($0.15/$0.60, cache hit $0.003), and gpt-oss-120b ($0.15/$0.60) on Groq or Fireworks with batch and caching. Only the DeepSeek options undercut Claude Haiku 5.5 and GPT-6 Luna ($0.10/$0.50 list); gpt-oss-120b now costs slightly more than those two closed models. The open-weight cost advantage has narrowed to the DeepSeek and Qwen tier.
- Bedrock's open-weight premium (DeepSeek v3.2 at 2.4× DeepInfra on input and 4.9× on output) means a Bedrock-only strategy costs more for DeepSeek and Qwen. It is at parity for gpt-oss and Mistral Large 3.
- OpenRouter's 5.5% credit fee is effectively a 5.5% markup on every token. For a reseller with volume, direct contracts or BYOK (free up to $25,000 a month) avoid it.

### Gaps
- Cerebras per-token prices, and Groq prices for Llama, did not render.
- Together's batch prices per model, and which models get the discount, are unclear; the doc lists only whisper-large-v3.
- Llama 4 Maverick prices, OpenRouter per-provider price spreads, and Mistral's prices for models other than Large were not captured.

## 7. LLM tokens: batch and caching discounts, Bedrock versus direct parity, enterprise discounts and startup credits

### Takeaway
Batch processing is a standard 50% discount (Anthropic, OpenAI, Google, Groq, Fireworks, Bedrock, Mistral). Cached input costs 0.025–0.1× of input at Anthropic, 0.05–0.1× at OpenAI, 0.1× at Google (plus hourly storage), 0.5× at Groq, and about 2–3% at DeepSeek. Claude on Bedrock carries no premium on global endpoints and +10% on regional ones, but Bedrock's Claude endpoint lacks Message Batches, server tools and the 2026-08 computer-use and browser toolsets. That makes Claude Platform on AWS (billed at Claude API rates) the parity path. Enterprise discounts are "negotiated case-by-case" with no published figures. Startup credits run up to $100K from Anthropic (through partner VCs), $200K+ from AWS Activate and $150K from Microsoft.

### Cited Findings
- Anthropic: the Batch API is "a 50% discount on both input and output tokens". Cache writes cost 1.25× (5 minutes) or 2× (1 hour), and reads cost 0.1× (0.05× on Opus and Sonnet 5.5, 0.025× on Fable and Mythos 5.1). The multipliers "stack with other pricing modifiers, including the Batch API discount and data residency" — [Anthropic pricing, 2026-10-12](https://platform.claude.com/docs/en/about-claude/pricing)
- OpenAI: Batch and Flex are half the standard price for the GPT-5.x rows. Fast mode costs 2× for gpt-5.6 and gpt-5.4 and 2.5× for gpt-5.5. Cached input is 0.1× for most models and 0.05× for gpt-6.1-sol — [OpenAI pricing, 2026-10-12](https://developers.openai.com/api/docs/pricing)
- Google: Batch is half price (for example Gemini 3.8 Flash $0.375/$1.875); cached input is 0.1×, plus storage of $0.50–$4.50 per MTok per hour — [Gemini API pricing, 2026-10-12](https://ai.google.dev/gemini-api/docs/pricing)
- Bedrock and Claude: "Global: dynamic routing across all available regions… No pricing premium. Regional: … carry a 10% pricing premium over global endpoints." Features not supported on Claude in Amazon Bedrock: "Server-side tools (code execution, web search, web fetch, advisor)", "API endpoints (Message Batches, …)", "Structured outputs", Claude Managed Agents, and the `computer_toolset_20260801` and `browser_toolset_20260801` toolsets ("the beta computer use tool versions remain available"). Default quota is 2 million input tokens per minute, up to 5 million input and 500,000 output TPM without extra approval — [Claude in Amazon Bedrock, 2026-10-12](https://platform.claude.com/docs/en/build-with-claude/claude-in-amazon-bedrock)
- Claude Platform on AWS bills through AWS Marketplace in Claude Consumption Units at "$0.01 per CCU", rating token usage "at standard per-model, per-feature rates (same as Claude API pricing)". Discounts are applied as fewer CCUs, through private offers. Fast mode is first-party only — [Anthropic pricing, 2026-10-12](https://platform.claude.com/docs/en/about-claude/pricing)
- Bedrock's pricing page notes "Starting October 7, Sonnet 5.5 cache read pricing is reduced by 50% from the prior rate"; its Claude price table did not render — [Bedrock pricing, 2026-10-12](https://aws.amazon.com/bedrock/pricing/)
- Enterprise discounts: Anthropic says "Volume discounts may be available for high-volume users. These are negotiated on a case-by-case basis" — [Anthropic pricing](https://platform.claude.com/docs/en/about-claude/pricing). Exa offers "Volume discounts and postpaid invoice billing" — [Exa pricing, 2026-10-12](https://exa.ai/pricing)
- Anthropic startups: "up to $100K in additional API credits, claimed through your VC" if the VC is a partner. The $1,000 API credit offer is "currently over capacity". Members receiving credits get "higher API rate limits". The program is open to "bootstrapped, pre-seed, and venture-backed startups". Credits apply only to the first-party API, "not AWS Bedrock, Google Cloud Vertex AI" — [Claude for Startups, 2026-10-12](https://claude.com/programs/startups)
- AWS Activate: Founders up to $5,000; Portfolio up to $200,000 (needs an Activate Provider Org ID, pre-Series B, founded in the last 10 years); "AWS Credits for AI Startups: $200,000+", invite-only. It mentions "building an AI-native product with Amazon Bedrock" — [AWS Activate credits, 2026-10-12](https://aws.amazon.com/startups/credits)
- Microsoft for Startups: "up to $150,000 in credits… across eligible Azure services"; whether they cover Foundry models is not stated — [Microsoft for Startups, 2026-10-12](https://www.microsoft.com/en-us/startups)
- Small credits elsewhere: Parallel gives qualified startups "up to $250"; Exa gives $10 a month free; Cerebras gives a one-time $5 credit — [Parallel](https://parallel.ai/pricing), [Exa](https://exa.ai/pricing), [Cerebras](https://www.cerebras.ai/pricing) (all 2026-10-12)

### Inferences
- On Bedrock, Claude Opus 5.5 workloads that could be batched lose the 50% batch discount, so they cost up to 2× the first-party batch price. They also lose the server-side web search and fetch tools and the new browser toolset, which matters directly for Goodfetch's browser agents. Claude Platform on AWS keeps first-party rates and features with AWS billing; it is the better path if AWS credits or commitments are the reason for using AWS. Whether Activate credits can pay for Marketplace CCUs was not verified.
- Combining batch with cache hits gives the deepest discount. Repair or extraction jobs that are not time-sensitive should go through Batch with cached system prompts: for example, Sonnet 5.5 batch input is $1 per million, and cached reads are 0.05× of that.

### Gaps
- Google for Startups Cloud Program terms (the page truncated, and the startup.google.com URL returned 404) and OpenAI startup credits (not located without search).
- Published or second-hand enterprise discount percentages for Anthropic, OpenAI or Google were not found; search was unavailable.
- Whether Bedrock's global Claude prices equal first-party list prices (Bedrock's Claude table did not render).

## 8. Search inputs for agents: price per 1,000 queries

### Takeaway
Raw Google SERP data costs $0.30–$1.00 per 1,000 queries via Serper, about $1–$4 via SearchApi.io, $1.30–$1.50 via Bright Data's SERP API, and $1.96–$25 via SerpApi, depending on volume. AI-oriented search APIs cost $1–$16 per 1,000: Parallel and Perplexity's fast tiers $1, Brave and Perplexity $5, Exa $4–$7, Tavily $5–$16. Model-native search tools cost $10–$14 per 1,000 plus tokens. Google's Custom Search JSON API is closed to new customers and ends 1 January 2027. Bing's Search APIs are retired; their successor, Grounding with Bing ($14 per 1,000), only works inside Azure Foundry agents and returns no raw results.

### Cited Findings
| Provider | Price per 1,000 queries | Notes | Source |
|---|---|---|---|
| Serper | $1.00 (50k credits for $50) → $0.75 (500k) → $0.50 (2.5M) → $0.30 (12.5M for $3,750) | Credits valid 6 months; 50–300 QPS; 2,500 free queries | [Serper, 2026-10-12](https://serper.dev/) |
| SearchApi.io | $4 (10k for $40) → $2.86 (35k for $100) → $2.50 (100k) → $2 (250k) → $1.50 (1M) → $1 (5M for $5,000) | 100 free requests | [SearchApi pricing, 2026-10-12](https://www.searchapi.io/pricing) |
| SerpApi | $25 (Starter, 1,000 for $25) → $10 (15k for $150) → $7.25 (100k for $725) → $3.75 (1M for $3,750) → $2.11 (10M) → about $1.96 (54M for $106,050) | "Cached, errored, and failed searches are not" counted; Legal Shield from the Production plan up; 250 free a month | [SerpApi pricing, 2026-10-12](https://serpapi.com/pricing) |
| Bright Data SERP API | $1.5 PAYG; $499/mo for 380K requests, then $1.3 | "Pay only for success" | [Bright Data SERP, 2026-10-12](https://brightdata.com/pricing/serp) |
| Apify SERP proxy | $2.5 / $2 / $1.7 per 1,000 SERPs by plan | — | [Apify pricing, 2026-10-12](https://apify.com/pricing) |
| Brave Search API | Search $5, with "$5 in free credits every month", 50 QPS; Answers $4 plus $5 per million tokens, 2 QPS; Enterprise custom | Storing results needs "a plan that explicitly grants storage rights" | [Brave Search API, 2026-10-12](https://brave.com/search/api/) |
| Exa | `instant` $4; `fast` and `auto` $7 (up to 10 results, +$1 per 1,000 extra results); deep $12–$15; contents $1 per 1,000 pages per content type; Answer $5 | $10 free, reset monthly; volume discounts | [Exa pricing, 2026-10-12](https://exa.ai/pricing) |
| Tavily | Basic search 1 credit, advanced 2; PAYG $0.008 per credit ($8 / $16 per 1,000); plans $0.0075 (Project, $30 for 4,000) to $0.005 (Growth, $500 for 100,000) per credit ($5–$7.50 basic, $10–$15 advanced) | 1,000 free credits a month; extract 1 credit per 5 URLs | [Tavily pricing, 2026-10-12](https://www.tavily.com/pricing); [Tavily credits, 2026-10-12](https://docs.tavily.com/documentation/api-credits) |
| Perplexity Search API | $5 (standard); $1 (Fast Search); Sonar request fees $5–$12 and Sonar Pro $6–$14 plus tokens | "charges per successful request… with no token costs" | [Perplexity pricing, 2026-10-12](https://docs.perplexity.ai/getting-started/pricing) |
| Parallel | Search: Turbo and Fast $1, Basic and Advanced $5 (10 results); Extract $1 per 1,000 | 5,000 free requests a month | [Parallel pricing, 2026-10-12](https://parallel.ai/pricing) |
| Google Custom Search JSON API | $5 after 100 free a day, up to 10k a day | "closed to new customers"; existing customers have until January 1, 2027. Suggested alternative: Vertex AI Search, for up to 50 domains | [Google Custom Search, 2026-10-12](https://developers.google.com/custom-search/v1/overview) |
| Bing Search APIs | Retired: Microsoft Learn's overview is archived under "previous-versions" with `is_retired: true`, last updated 2025-08-11 | — | [Microsoft Learn, archived Bing Web Search overview, 2026-10-12](https://learn.microsoft.com/en-us/bing/search-apis/bing-web-search/overview) |
| Grounding with Bing Search / Custom Search | "$14 per 1,000 transactions" each; 150 TPS; 1 million transactions a day | "Developers and end users don't have access to raw content"; citations must be displayed; data leaves the Azure compliance boundary; credit-based subscriptions not eligible; doesn't work with gpt-5 models. Foundry (classic) agents retire 31 March 2027 | [Bing APIs, 2026-10-12](https://www.microsoft.com/en-us/bing/apis); [Grounding with Bing docs, updated 2026-07-11](https://learn.microsoft.com/en-us/azure/ai-foundry/agents/how-to/tools/bing-grounding) |
| Model-native search tools | Claude web search $10 plus tokens; OpenAI web search $10 plus tokens; Gemini grounding $14 (Gemini 3+, after 5,000 free a month) or $35 (2.5) | — | [Anthropic](https://platform.claude.com/docs/en/about-claude/pricing); [OpenAI](https://developers.openai.com/api/docs/pricing); [Gemini](https://ai.google.dev/gemini-api/docs/pricing) (all 2026-10-12) |

### Inferences
- An agent that calls a SERP API itself and feeds the results to the model pays $0.30–$2 per 1,000 searches at volume, against $10–$14 for model-native search. That is about 5–47× cheaper ($10 ÷ $2 to $14 ÷ $0.30), before the extra tokens the native tools also bill. Goodfetch Cloud's hosted-agent COGS should route search through its own SERP integration and keep native tools as a fallback.
- Neither Google Custom Search nor Bing is viable for a new build. Brave's storage-rights clause matters if Goodfetch caches search results for its crawl API.
- AI-oriented search APIs (Exa, Tavily, Parallel, Perplexity, Brave) cost $1–$16 per 1,000 and return cleaned content. They overlap with Goodfetch's own crawl and extraction product, so their price sets a ceiling on what customers will pay for "search plus content".

### Gaps
- No official Microsoft announcement page giving the exact Bing retirement date was fetched; the date comes from the archived page's metadata. The pricing of Microsoft's newer Foundry "Web Search tool" was not captured.
- Jina, You.com, Linkup, DataForSEO and Scrapingdog were not covered (search budget exhausted).
- Vertex AI Search and Google's "Web Grounding for Enterprise" pricing were not captured.
