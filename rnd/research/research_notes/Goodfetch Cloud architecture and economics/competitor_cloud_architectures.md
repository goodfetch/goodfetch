# How commercial scraping, crawling, hosted-browser and browser-agent clouds are built and operated (as of 2026-10-12)

Evidence gathered on 2026-10-12. Sources are primary pages (docs, trust centers, subprocessor lists, DPAs, status pages, engineering blogs, job posts), public repositories read through the GitHub API, and DNS/ASN lookups.

How to read the tags:
- An untagged finding is stated on the cited page.
- `[inferred: basis]` marks a deduction from DNS/ASN, job posts or code.
- `[unverified]` marks a claim seen only in a search snippet, or one that could not be confirmed.
- "possibly stale" marks a source dated before 2025.
- Undated pages were accessed on 2026-10-12.
- Vendor scale and latency figures are the vendors' own claims.
- Pricing is out of scope. Cost appears only where a company explains an architecture choice by cost.

## 1. Which cloud providers or bare-metal hosts run each service, and in which regions?

### Takeaway
**AWS runs most control planes**, usually in a single US region:

| Service | AWS region(s) |
|---|---|
| Apify | us-east-1 |
| Browserbase | 4 regions, behind Kubernetes-style load balancers (inferred from DNS) |
| Browser Use | us-east-2 |
| Skyvern | us-east-1 |
| Anchor | us-east-2, plus eu-central-1 |
| Airtop | us-east-2 |
| Kernel (API only) | us-east-1 |
| Steel (connect gateway) | us-east-1 |
| ZenRows | us-east-1 |
| Bright Data (platform) | AWS, multiple Availability Zones |
| Spider (front door) | Global Accelerator, browser gateway in us-east-2 |

**Google Cloud** runs Firecrawl (its API IP is in us-east4), Hyperbrowser (us-central1), ScrapingBee (Paris) and Zyte's browser endpoint (us-east1).

**Cheaper dedicated or VPS providers carry the heavy-traffic or cost-sensitive layers:**
- Zyte: Hetzner and Servers.com.
- Bright Data: super proxies on Vultr.
- ScrapingBee: DataPacket dedicated servers.
- Lightpanda: Hetzner for its EU region.
- Kernel: browsers on bare metal, likely Leaseweb.
- Browserless and ScraperAPI: DigitalOcean.

**Regions:** only hosted-browser vendors run several. Their stated reason is CDP round-trip latency (Browserbase 4, Hyperbrowser 5, Kernel 3, Browserless 3, Lightpanda 2). Steel went back down to one region.

### Cited Findings

#### Apify (Actors, Crawlee, Store, Proxy)
- **AWS us-east-1.**
  - The platform runs on AWS us-east-1 — [Apify shared responsibility](https://docs.apify.com/security/shared-responsibility) (undated).
  - Single AWS region across multiple Availability Zones, built on managed AWS services — [Apify platform architecture](https://docs.apify.com/security/architecture) (undated).
  - In 2020, core servers and storage were in Northern Virginia, and the MongoDB Atlas cluster was in us-east-1 too. CDN and proxy servers in other countries held no user data — [Apify security whitepaper](https://apify.com/security-whitepaper.pdf) (2020-01-13; possibly stale).
- **Personal data** is processed mainly in the EU and the US — [Apify privacy policy](https://docs.apify.com/legal/privacy-policy.md) (updated 2026-07-09).
- **Proxy allowlisting:** the published proxy IPs for `proxy.apify.com` are 18.208.102.16 and 35.171.134.41. Both map to EC2 us-east-1 in AWS's range file — [Apify Proxy docs](https://docs.apify.com/proxy.md) (undated); [AWS ip-ranges.json](https://ip-ranges.amazonaws.com/ip-ranges.json) (createDate 2026-10-11) [inferred: IP ranges].
- **Second region:** none found on the architecture, shared-responsibility, GDPR or privacy pages — [Apify platform architecture](https://docs.apify.com/security/architecture) [unverified: absence].

#### Zyte (Scrapy Cloud, Zyte API, Smart Proxy Manager)
- **Hosting subprocessors** — [Zyte subprocessors, Annex III](https://www.zyte.com/terms-policies/subprocessor/) (undated, © 2026):
  - Amazon Web Services (hosting, Seattle);
  - Google Cloud (infrastructure);
  - Hetzner (hosting, Gunzenhausen, DE);
  - Servers.com (hosting, Dallas, US).
- The trust center says all hosting providers are ISO 27001-certified, without naming regions — [Zyte trust center](https://www.zyte.com/trust-center/) (undated).
- Scrapy Cloud 2.0 moved spider execution to Germany — [Introducing Scrapy Cloud 2.0](https://www.zyte.com/blog/introducing-scrapy-cloud-2-0/) (2016-05-04; possibly stale).
- **AWS dependency:** an AWS DNS problem affecting DynamoDB raised error rates across several Zyte services on 2025-10-20 (07:00–09:27 UTC) — [Zyte status incident](https://status.zyte.com/incidents/g9ry3nzrspfl). The AWS region is [unverified].
- **CDP browsers:** Playwright and Puppeteer connect to `browser.zyte.com` — [Zyte CDP support](https://www.zyte.com/blog/introducing-zyte-cdp-support/) (2026-09-01). That host resolves to a Google Cloud us-east1 IP — [Google Cloud IP ranges](https://www.gstatic.com/ipranges/cloud.json) (2026-10-11) [inferred: DNS].

#### Browserbase (Stagehand, Director, Functions)
- **Four public regions:** us-west-2 (default), us-east-1, eu-central-1 and ap-southeast-1, chosen with a `region` parameter at session creation — [Browserbase multi-region](https://docs.browserbase.com/guides/multi-region) (undated).
- The enterprise security page frames regions as data residency (US West, US East, EU/Germany, Asia/Singapore) and names no provider — [Browserbase enterprise security](https://docs.browserbase.com/account/enterprise/security) (undated).
- **History:** the service began in one AWS region and expanded to us-east-1, eu-central-1 and ap-southeast-1, with ap-northeast-1 (Tokyo) also mentioned — [Becoming multi-region](https://www.browserbase.com/blog/becoming-multi-region) (2024-12-06; possibly stale).
  - The post contradicts itself on the original region: it says us-west-1, then calls us-west-2 the main location.
  - Tokyo is not in the current docs, and `connect.apne1.browserbase.com` does not resolve [inferred: docs + DNS].
- **Egress IPs:** no list is published. Customers who need allowlisting send traffic through their own static proxy or VPN — [Browserbase IP allowlisting](https://docs.browserbase.com/platform/browser/security/ip-allowlisting.md) (undated).

#### Steel.dev
- **Today, one region:** Steel runs browsers only in `us-east`. Any other `region` value is rejected. The public IP a site sees comes from proxy geolocation, not from the region — [Steel multi-region docs](https://docs.steel.dev/overview/sessions-api/multi-region) (undated).
- **March 2026, several regions:** a Steel article named regions `lax` and `iad` and said sessions defaulted to the nearest datacenter — [Steel multi-region article](https://llms.steel.dev/articles/multi-region-browser-sessions/) (2026-03-31). So Steel consolidated to one region between April and October 2026 [inferred: conflicting docs; no announcement found].
- **No Fly.io evidence** behind the airport-style region codes. The repo has `render.yaml`, Docker files and a Railway template, but no `fly.toml` — [steel-browser repo](https://github.com/steel-dev/steel-browser) [unverified: hosting of Steel Cloud].
- **Self-hosting:** Steel offers "Steel Outpost" and self-hosting so data stays in the customer's network — [steel.dev](https://steel.dev) (undated).

#### Browserless
- **Three regional endpoints:** `production-sfo` (San Francisco, the default), `production-lon` and `production-ams`. Every API path works in every region — [Browserless load balancers](https://docs.browserless.io/baas/infrastructure/load-balancers) (undated).
- **Trust page** — [Browserless trust](https://browserless.io/trust) (updated 2026-09-11):
  - AWS, DigitalOcean and Supabase store customer data, mainly in the US.
  - Sessions execute in San Francisco, London or Amsterdam.
  - On the shared fleet, telemetry, logs, replays, profiles and crawl results leave the EU whichever endpoint you use.
- In 2024 the company ran thousands of concurrent browsers on AWS and DigitalOcean — [Rebuilding Browserless](https://www.browserless.io/blog/rebuilding-browserless) (2024-02-21; possibly stale).
- **Egress IPs** on the shared fleet change every few months and are listed through a GraphQL `workerIps` query — [Browserless IP allowlisting](https://docs.browserless.io/baas/troubleshooting/whitelisting-ips.md) (undated).
- **Origins:** in 2018 the founder created a DigitalOcean droplet per new signup through the DigitalOcean API — [Indie Hackers interview](https://www.indiehackers.com/interview/how-ive-grown-my-headless-browser-business-to-1-000-mo-6abb96ab8a) (2018; possibly stale).
- The status page lists a "US East" component that does not appear among the documented public endpoints — [Browserless status](https://status.browserless.io) [unverified: what it serves].

#### Kernel (formerly onkernel)
- **Bare metal:**
  - The CTO's engineering post is titled "Lessons learned from scaling Chromium on bare metal". It names no provider or hardware — [Kernel blog: scale](https://www.kernel.sh/blog/scale) (dated only "May", most likely 2026).
  - Kernel maintains `leaseweb-cli` (MIT, created 2026-02-24), a CLI for Leaseweb dedicated servers, cloud and networking — [kernel/leaseweb-cli](https://github.com/kernel/leaseweb-cli) [inferred: Leaseweb is the likely bare-metal provider].
- **Unikraft:**
  - In 2025 the founders said the unikernel build could only run on Unikraft Cloud — [HN launch thread](https://news.ycombinator.com/item?id=43705144) (2025-04-16).
  - The open-source run script still deploys with `kraft cloud` and needs `UKC_METRO`/`UKC_TOKEN` — [kernel-images](https://github.com/onkernel/kernel-images).
  - Unikraft's customer story confirms Kernel runs on Unikraft snapshot microVMs, but does not say whether on Unikraft Cloud or Kernel's own hosts — [Unikraft customer story](https://unikraft.com/customer-stories/scaling-browser-infrastructure) (updated 2026-01-24).
- **Security docs** say only that all infrastructure is cloud-hosted, with physical security left to the provider — [Kernel security](https://kernel.sh/docs/security.md) (undated).
- **Regions** — [Kernel regions](https://kernel.sh/docs/browsers/regions.md) (undated):
  - `us-east` (default), `eu-west` and `ap-southeast`;
  - no data-residency guarantee: profiles, replays and metadata may be stored or processed in the US;
  - region choice only on the Start-Up and Enterprise plans;
  - regions for GPU browsers, Apps and Invocations are "coming soon".
- **Egress prototype:** browser traffic leaves through the user's own Mac or iPhone, via a shared AWS Lightsail relay and a reverse SSH tunnel — [kernel/user-device-egress](https://github.com/kernel/user-device-egress).

#### Hyperbrowser
- **Session regions:** `us-central`, `us-east`, `us-west`, `europe-west` and `asia-south`, on Enterprise and Select plans only, chosen with `region` — [Hyperbrowser regions](https://hyperbrowser.ai/docs/sessions/regions.md) (undated). The names follow Google Cloud's region families [inferred: naming].
- **Subprocessors:** Data Packet, Amazon Web Services, Anthropic, Better Stack, Google Cloud Services, OpenAI, Tigris. Production data sits in managed cloud databases in the United States unless agreed otherwise in writing — [Hyperbrowser trust center](https://trust.hyperbrowser.ai/).
  - Tigris (S3-compatible object storage) is the likely store for recordings and files [inferred: subprocessor list].
  - "Data Packet" may be the DataPacket dedicated-server provider [unverified].

#### Anchor Browser
- **AWS regions:** us-east-2 (Ohio) by default, and eu-central-1 (Frankfurt) for EU residency on the Growth plan, across multiple AZs — [Anchor security](https://docs.anchorbrowser.io/security.md) (undated).
- **Trust center:** AWS is the infrastructure provider; subprocessors include OpenAI, Anthropic and Google Cloud — [Anchor trust portal](https://trust.anchorbrowser.io/).
- **Conflict:** the plan table lists US for Free and Starter; US, EU, Asia and Australia for Growth; and "any region" plus bring-your-own-cloud or on-premise for Enterprise. The security page names only two AWS regions — [Anchor plans](https://docs.anchorbrowser.io/pricing.md) (undated).

#### Airtop
- No page about regions appears in the docs index — [Airtop docs index](https://docs.airtop.ai/llms.txt). See the DNS table below.

#### Browser Use Cloud
- **Browsers:** Firecracker microVMs on regular EC2 instances, not `.metal` — [Browser Use: Firecracker browser infra](https://browser-use.com/posts/firecracker-browser-infra) (2026-06-15).
  - Before that, browsers ran on Unikraft over EC2, and adding nodes was manual.
  - No AWS region is named.
- **Agent sandboxes, stage 1:** Unikraft Cloud micro-VMs on dedicated bare-metal machines in AWS, spread across several Unikraft metros — [Two ways to sandbox agents](https://browser-use.com/posts/two-ways-to-sandbox-agents) (2026-02-25).
- **Agent sandboxes, stage 2:** AWS Bedrock AgentCore Runtime. The rest of the stack is AWS-native: ECS Fargate, RDS, S3, IAM, CloudWatch, ECR, VPC — [AgentCore migration](https://browser-use.com/posts/agentcore-migration) (2026-07-12).
- **Locations:** no compute regions are published, only proxy egress countries (us, uk, fr, it, jp, au, de, fi, ca, in) — [Browser Use CLOUD.md](https://github.com/browser-use/browser-use/blob/main/CLOUD.md) (undated).
- **Job post** ("Infra god"): AWS, Kubernetes, Terraform, GPU orchestration, 99.99% uptime goals, and AWS/Azure/GCP for on-prem deployments — [YC job post](https://www.workatastartup.com/jobs/79630) (updated about 2026-08-21) [unverified: search snippet; describes goals, not current state].

#### Skyvern Cloud
- **OSS config:**
  - defaults `AWS_REGION` to `us-east-1`;
  - names S3 buckets for artifacts, screenshots, browser sessions and uploads;
  - a code comment says cloud workers download S3 profiles at startup.
  - [Skyvern repo, `skyvern/config.py`](https://github.com/Skyvern-AI/skyvern) [inferred: the cloud uses these defaults].
- **Images** are published on AWS ECR Public (`public.ecr.aws/skyvern/skyvern`) — [Skyvern repo, `docker-compose.yml`](https://github.com/Skyvern-AI/skyvern).
- **Trust center:** trust.skyvern.com loads by JavaScript and returned no content. The SOC 2 post names no auditor or type — [Skyvern x SOC 2](https://www.skyvern.com/blog/skyvern-x-soc-2/) (2025-08-07).

#### Notte
- **Trust center:** SOC 2 Type 2 and about 79 control names, but no subprocessors and no regions — [Notte trust center](https://trust.notte.cc).
- **Deployment:** cloud or on-premises — [notte.cc](https://www.notte.cc/) (undated).
- **Anything API product** (not the browser fleet):
  - its agent loop runs in Notte's backend on the Vercel AI SDK;
  - Daytona sandboxes run only the tools;
  - OpenRouter, Redis, Sentry and PostHog are also used.
  - [Why we moved the agent out of the sandbox](https://www.notte.cc/blog/why-we-moved-the-agent-out-of-the-sandbox) (2026-09-09).

#### Lightpanda Cloud
- **Two regions:** West Europe (`wss://euwest.cloud.lightpanda.io/ws`) and West US (`wss://uswest.cloud.lightpanda.io/ws`) — [Lightpanda Cloud getting started](https://lightpanda.io/docs/run-on-lightpanda-cloud/getting-started) (undated).

#### Firecrawl
- **GCP today:**
  - A 2026 infrastructure job ad asks the hire to design and operate GCP and on-prem infrastructure — [Firecrawl job ad](https://haystackapp.io/jobs/6690a216-14d3-407a-ba74-4066b1b843a1) (2026-08-20).
  - The code uses Google Bigtable, Cloud Storage and Pub/Sub — [bigtable-client.ts](https://github.com/firecrawl/firecrawl/blob/main/apps/api/src/lib/bigtable-client.ts), [gcs-jobs.ts](https://github.com/firecrawl/firecrawl/blob/main/apps/api/src/lib/gcs-jobs.ts), [pubsub-log-metrics.ts](https://github.com/firecrawl/firecrawl/blob/main/apps/api/src/lib/pubsub-log-metrics.ts).
- **Fly.io in 2024:**
  - The API and workers ran on Fly.io, and Redis moved from Railway to Fly.
  - The team was setting up Kubernetes because Fly would not autoscale smoothly.
  - [An adventure in scaling](https://www.firecrawl.dev/blog/an-adventure-in-scaling) (2024-09-13; possibly stale).
  - I found no write-up of the move from Fly.io to GCP/Kubernetes itself [inferred: from the 2024 post plus 2026 code, job ads and DNS].
- An X post attributed a 2025-06-12 outage to upstream providers including GCP — [Firecrawl on X](https://x.com/firecrawl/status/1933231981506277813) [unverified: title only; the page returned HTTP 402].
- **CI:** GitHub Actions on Blacksmith runners build amd64 and arm64 images and push them to GHCR — [deploy-image.yml](https://github.com/firecrawl/firecrawl/blob/main/.github/workflows/deploy-image.yml).

#### Spider Cloud (spider-rs)
- **Vendor claim:** Spider runs on its own browser engine and network, and the same network serves the crawler and the browser. No regions are published; location is chosen by proxy exit country (199+) — [spider.cloud](https://spider.cloud/), [Spider docs overview](https://spider.cloud/docs/overview/) (undated).
- The open-source `headless-browser` repo describes deployment on Fargate, Cloud Run or Kubernetes — [spider-rs/headless-browser](https://github.com/spider-rs/headless-browser) (last pushed 2026-02-09) [unverified: whether production uses it].

#### Cloudflare Browser Run (formerly Browser Rendering)
- **Rename:** "Browser Rendering" became "Browser Run" on 2026-04-15 — [Cloudflare changelog](https://github.com/cloudflare/cloudflare-docs/blob/production/src/content/changelog/browser-run/2026-04-15-br-rename.mdx).
- **Network:** headless Chrome runs on Cloudflare's global network, and sessions open near users — [Browser Run docs](https://github.com/cloudflare/cloudflare-docs/blob/production/src/content/docs/browser-run/index.mdx); [Browser Run for AI agents](https://blog.cloudflare.com/browser-run-for-ai-agents) (2026-04-15).
- **Egress:** traffic leaves from Cloudflare IP ranges, with no per-request IP rotation — [Browser Run FAQ](https://github.com/cloudflare/cloudflare-docs/blob/production/src/content/docs/browser-run/faq.mdx).

#### Bright Data (Browser API / Scraping Browser, Web Unlocker, proxy network)
- **Primary cloud and backups** — [Bright Data security overview](https://docs.brightdata.com/general/security/security-overview) (undated; cites a May–June 2025 penetration test):
  - AWS across multiple Availability Zones;
  - disaster-recovery site in an AWS EU region;
  - AWS Backup snapshots, with critical backups also on Microsoft Azure.
- An older trust-center page says the same about AWS multi-AZ — [Bright Data data security](https://brightdata.com/trustcenter/data-security-overview-protection-measures) (2023-12-06, modified 2024-08-28; possibly stale).
- **Super-proxy routing:**
  - A load balancer picks the fastest super proxy.
  - Customers can pin a country with `servercountry-<cc>.brd.superproxy.io`, which Bright Data warns can hurt performance.
  - [Bright Data super-proxy selection](https://docs.brightdata.com/api-reference/proxy/select_super_proxy_in_specific_country) (undated).
- **Third-party traffic data (Netify):** — [Netify: Bright Data](https://www.netify.ai/resources/applications/bright-data); [Netify: zproxy host](https://www.netify.ai/resources/hostnames/zproxy.lum-superproxy.io) (undated) [inferred: third-party data]
  - hosting IPs: DigitalOcean 2,595, Vultr 1,732, UpCloud 45, AWS 20;
  - CDN IPs: about 490 CloudFront;
  - `zproxy.lum-superproxy.io` maps to about 1,772 IPs in 12 countries.
- **Where IPs come from:**
  - Residential peers come from an opt-in SDK embedded in partner apps — [Bright Data sourcing](https://brightdata.com/trustcenter/sourcing) (modified 2024-07-16; possibly stale).
  - ISP proxies are IPs from real ISPs, leased long term — [Bright Data proxy types](https://brightdata.com/proxy-types) (modified 2026-09-24).

#### ScrapingBee
- **DPA hosting** — [ScrapingBee DPA](https://www.scrapingbee.com/data-processing-agreement/) (revised 2026-03-23):
  - Google Cloud Platform, Paris as the main region, with EU multi-datacenter backups;
  - Datapacket dedicated servers in the Netherlands, France and the Czech Republic;
  - processing is primarily in the EU.
- **Older, conflicting notice:** Netlify for the website and Clever Cloud (Nantes) for the API — [ScrapingBee legal notices](https://www.scrapingbee.com/legal-notices/) (effective 2024-07-15; possibly stale).

#### ScraperAPI
- **Owner:** listed in the saas.group portfolio — [saas.group family](https://saas.group/family/) (undated).
- **Hosting:** no subprocessor or region page found; see the DNS table for DigitalOcean.

#### ZenRows
- **Privacy policy:** AWS, Stripe, Amplitude, ProfitWell, HubSpot and Google appear among international data-transfer recipients — [ZenRows privacy](https://www.zenrows.com/legal/privacy) (updated Sep 2024; possibly stale).
- **Compliance:** SOC 2 Type II and ISO 27001 claimed — [ZenRows llms.txt](https://www.zenrows.com/llms.txt) (undated).

### Inferences

**DNS/ASN sweep (2026-10-12):**
- Method: `dig` for CNAME and A records, the whois.cymru.com IP-to-ASN service, and a match against [AWS ip-ranges.json](https://ip-ranges.amazonaws.com/ip-ranges.json) (createDate 2026-10-11-23-37-06) and [Google Cloud cloud.json](https://www.gstatic.com/ipranges/cloud.json) (2026-10-11).
- Caveats:
  - One vantage point; geographic DNS (as at Bright Data) may answer differently elsewhere.
  - CDNs and anycast (Cloudflare, Global Accelerator) hide origins.
  - The table shows where entry points live, not every worker.

| Service | Hostname → target | Network / region |
|---|---|---|
| Apify | `api.apify.com`, `proxy.apify.com`, `mcp.apify.com` | AWS EC2 us-east-1 (AS14618) |
| Apify | `console.apify.com` | AWS CloudFront |
| Zyte | `app.zyte.com` → `webheads.zyte.group`; `app.scrapinghub.com`; `storage.scrapinghub.com` → `storage.zyte.com` | Hetzner (AS24940, DE) |
| Zyte | `api.zyte.com`; `proxy.zyte.com` → `proxy.crawlera.com` | Servers.com (AS7979, US) |
| Zyte | `browser.zyte.com` | Google Cloud us-east1 |
| Browserbase | `api.browserbase.com` → `k8s-api-api-…us-west-2.elb.amazonaws.com` | AWS us-west-2 |
| Browserbase | `connect.{usw2,use1,euc1,apse1}.browserbase.com` → `k8s-connect-goconnec-<hash>` ALBs | AWS us-west-2, us-east-1, eu-central-1, ap-southeast-1 |
| Browserbase | `api.stagehand.browserbase.com` → `k8s-stagehandapi-…us-west-2` | AWS us-west-2 |
| Steel | `api.steel.dev` | Cloudflare (origin hidden) |
| Steel | `connect.steel.dev` → `connect.steel-edge.net` | AWS EC2 us-east-1 instance IPs |
| Browserless | `production-sfo/lon/ams.browserless.io` (one IP each) | DigitalOcean (AS14061) |
| Kernel | `api.onkernel.com` → `kernel-api-use1-prod-prod-api-…us-east-1.elb.amazonaws.com` | AWS us-east-1 |
| Kernel | dashboard, status | Vercel; incident.io |
| Hyperbrowser | `api.hyperbrowser.ai` | Google Cloud global anycast load balancer |
| Hyperbrowser | `connect.hyperbrowser.ai`, `app.hyperbrowser.ai` | Google Cloud us-central1 |
| Anchor | `api/connect.anchorbrowser.io` | AWS EC2 us-east-2 |
| Airtop | `api/portal.airtop.ai` | AWS EC2 us-east-2 |
| Browser Use | `api.browser-use.com` → `browser-use-production-alb-…us-east-2.elb.amazonaws.com` | AWS us-east-2 |
| Browser Use | `connect.browser-use.com` → `u7aejtm3.up.railway.app` | Railway (AS400940) |
| Browser Use | `cloud.browser-use.com` | Vercel |
| Skyvern | `api.skyvern.com` → `skyvern-alb-production-…us-east-1.elb.amazonaws.com` | AWS us-east-1 |
| Skyvern | `app.skyvern.com` | Vercel |
| Notte | `api.notte.cc` | Cloudflare (origin hidden) |
| Notte | `console.notte.cc` | Vercel |
| Lightpanda | `cloud.lightpanda.io` and `euwest.cloud.lightpanda.io` → `load-balancer-1.hetzner.lightpanda.io` | Hetzner (AS24940, DE) |
| Lightpanda | `uswest.cloud.lightpanda.io` | AWS ELB us-west-2 |
| Lightpanda | `console.lightpanda.io` → `lightpanda-baas-webapp.fly.dev` | Fly.io |
| Firecrawl | `api.firecrawl.dev` | Google Cloud us-east4 |
| Spider | `api.spider.cloud`, `proxy.spider.cloud` | AWS Global Accelerator anycast (see note below) |
| Spider | `browser.spider.cloud` | AWS EC2 us-east-2 |
| Bright Data | `brd.superproxy.io`, `zproxy.lum-superproxy.io` | Vultr (AS20473) |
| Bright Data | `api.brightdata.com` | AWS EC2 us-east-1 |
| ScrapingBee | `app.scrapingbee.com` → `prod.primary.externalbee.cc` | CDN77/DataCamp (AS60068) |
| ScraperAPI | `api.scraperapi.com` (also `async.`, `datapipeline.`) | DigitalOcean |
| ScraperAPI | dashboard | DigitalOcean App Platform |
| ZenRows | `api.zenrows.com` → `api-external-…elb.us-east-1.amazonaws.com`; `browser.zenrows.com` → `browser-external-…` | AWS us-east-1, network-load-balancer-style names |

- **Spider proxy conflict:** a sub-researcher saw `proxy.spider.cloud` on an AT&T IP. My lookups through the system resolver, 1.1.1.1 and 8.8.8.8 all returned Global Accelerator IPs, so the AT&T observation is [unverified].
- **What the hostnames suggest about Browserbase:** the `k8s-<namespace>-<name>-<hash>` load-balancer names are what the AWS Load Balancer Controller generates for Kubernetes ingress. So Browserbase runs Kubernetes on AWS in all four regions, with a regional CDP gateway whose truncated name ("goconnec…") suggests a Go service [inferred: naming]. Whether this is EKS or self-managed Kubernetes cannot be told.
- **Split across providers:** a common pattern is a hyperscaler for the API, account and data plane, plus a cheaper provider for bandwidth-heavy edges:
  - Bright Data super proxies on Vultr and DigitalOcean;
  - Zyte's Scrapy Cloud and dashboard on Hetzner;
  - ScrapingBee on DataPacket;
  - Lightpanda's EU region on Hetzner;
  - Kernel browsers on bare metal.
- **Single US region:** most services run one US region, with us-east-1 and us-east-2 dominant. Multi-region exists where CDP latency is sold (hosted browsers). EU residency is a paid add-on at Anchor and Browserbase. Kernel and Browserless state that profiles, replays and logs may still be stored in the US.
- **Front-end hosting:** Vercel serves the consoles and docs of Browser Use, Skyvern, Notte, Kernel, Browserbase and Firecrawl; the APIs sit elsewhere.

### Gaps
- **Search budget:** the shared WebSearch budget for this session ran out early, so subprocessor lists that only search would surface were not found. Missing for Bright Data, Firecrawl, Spider, Kernel, Airtop, Skyvern, Steel and ScraperAPI.
- **JavaScript-rendered trust centers** returned no content: Browserbase, Skyvern, Airtop (Vanta), ZenRows (Vanta) and ScrapingBee. The Delve-hosted ones for Kernel and Browser Use returned HTTP 429.
- **Exact worker regions** are unknown for Spider, Firecrawl (beyond the us-east4 API IP), Airtop, Notte, Browser Use's browser fleet, and Cloudflare's browser data centers.
- **Kernel's bare-metal provider** is unconfirmed (Leaseweb is inferred from a tooling repo), and its hardware is not described.
- **Apify** shows no evidence of compute outside us-east-1, and its full subprocessor list is shared only under NDA ([Apify GDPR information](https://docs.apify.com/legal/gdpr-information)).

## 2. What isolation does each use per run or session, how do they get fast session start, and what start latencies do they publish?

### Takeaway
**The hosted-browser leaders have converged on one virtual machine per browser session:**
- Browserbase: a dedicated VM, destroyed after the session.
- Anchor: a dedicated VM, never reused.
- Kernel: unikernel microVMs on Unikraft, plus a QEMU-based data plane (Hypeman) for GPU and live migration.
- Browser Use: Firecracker microVMs.
- Steel: Firecracker, as of 2024.

**Actor and crawl platforms still use containers, or share one browser process:**
- Apify: Docker containers per run, with a 2026 job post also mentioning microVMs.
- Zyte: Docker containers per job.
- Browserless: containers on its shared fleet, VMs for private deployments.
- Spider: isolated browser contexts inside one multi-engine process.
- Cloudflare: undisclosed, plus a new V8-isolate browser (Kitesurf).

**Fast start comes from memory snapshots:**
- Kernel restores in about 30 ms from local NVMe, and keeps idle browsers in a snapshot "standby" while a proxy holds the CDP socket.
- Browser Use snapshots just before Chromium launch, then uses 2 MB pages, a `userfaultfd` prefetcher and vsock readiness signalling.

Warm pools remain the other technique (Cloudflare, Kernel's browser pools), but Kernel and Steel say pools of idle browsers cost too much.

**Published start times** range from Kernel's 30 ms p50 / 105 ms p99, to Browser Use's 825 ms p50 / 1.35 s p99, to Browserbase "under 2 s", and up to about a minute at Airtop when capacity is short.

### Cited Findings

#### Browserbase
- **Isolation:**
  - each browser runs in its own dedicated VM, isolated at the hardware-virtualization layer, in its own subnet with strict firewalls;
  - the VM is killed and recreated after every session, so no browser is reused;
  - browsers get no GPU, to avoid shared-GPU-memory attacks.
  - [Browserbase enterprise security](https://docs.browserbase.com/account/enterprise/security) (undated).
- **The VMM is not named.** A Core Infrastructure posting lists Go/TypeScript, Kubernetes/Docker, message queues, relational databases and fleets of hundreds to thousands of instances, with Firecracker or a similar VMM as a bonus skill — [Browserbase Ashby job board](https://api.ashbyhq.com/posting-api/job-board/browserbase) (posting 2025-03-25) [inferred: a Firecracker-class microVM].
  - Browserbase's own Firecracker explainer does not say it uses Firecracker — [What is Firecracker](https://www.browserbase.com/blog/what-is-firecracker) (2026-05-11).
- **Start time:** browsers are ready to drive in under 2 seconds. A browser service manages spin-up, recycling, crash recovery and memory — [What is a Browserbase browser](https://www.browserbase.com/blog/what-is-a-browserbase-browser) (2026-06-16).
- **Limits:**
  - concurrency caps per plan from 3 to 250+, and session creations per minute from 5 to 150+;
  - over-limit requests get HTTP 429 with `retry-after` and are dropped, not queued;
  - every session is metered for at least one minute.
  - [Browserbase concurrency](https://docs.browserbase.com/optimizations/concurrency/overview.md) (undated).
- **Duration:** sessions last up to 6 hours. Without `keepAlive` a session ends when the client disconnects — [Browserbase timeouts](https://docs.browserbase.com/platform/browser/long-sessions/timeouts.md) (undated).
- **Earlier latency work:** before going multi-region, the team tried bigger browser resources and removing virtualization layers — [Becoming multi-region](https://www.browserbase.com/blog/becoming-multi-region) (2024-12-06; possibly stale).

#### Steel.dev
- **2024 launch thread** — [HN: Steel launch](https://news.ycombinator.com/item?id=42245573) (2024-12-01; possibly stale):
  - one session per machine, never shared, with about 2 GB and 2 vCPU each;
  - Kubernetes was rejected because warm instances would cost too much and boot too slowly;
  - the browser image runs in Firecracker VMs, chosen for fast boot and a shared root filesystem.
- **2026:** Steel still describes booting a VM per session — [Steel multi-region article](https://llms.steel.dev/articles/multi-region-browser-sessions/) (2026-03-31).
- **Start time:** sessions start in under 1 second on average with no cold-start penalty, each in its own Chromium — [steel.dev](https://steel.dev) (undated). Whether a warm pool or snapshots achieve this is not published [unverified].
- **Limits:**
  - concurrency 10 / 100 / 1,000+ by plan;
  - maximum duration 15 min / 1 h / up to 24 h;
  - creations 60 / 600 per minute.
  - [Steel limits](https://docs.steel.dev/overview/pricinglimits) (edited 2026-06-30).
- **Lifecycle:** default timeout 5 min; an optional `inactivityTimeout` resets on CDP or input — [Steel session lifecycle](https://docs.steel.dev/overview/sessions-api/session-lifecycle) (undated).

#### Kernel
- **Isolation:**
  - each browser is its own VM, isolated at the hypervisor rather than sharing a host kernel — [Kernel docs](https://www.kernel.sh/docs) (undated);
  - the security page describes each session as a single-tenant Unikraft-based unikernel VM — [Kernel security](https://kernel.sh/docs/security.md);
  - unikernels have no internal security boundaries, so isolation inside the VM relies on Chromium's own sandbox — [Kernel unikernels](https://www.kernel.sh/docs/info/unikernels).
- **Architecture history** — [Kernel blog: scale](https://www.kernel.sh/blog/scale) (May, most likely 2026):
  - Docker warm pool without Kubernetes; idle containers were costly and a container escape would expose every tenant;
  - Unikraft unikernels, chosen for isolation, cold start and memory snapshots;
  - Hypeman, a QEMU-based data plane, added for GPU and live migration. Unikraft pins snapshots to one host and has no device passthrough. QEMU was preferred to Cloud Hypervisor for its ecosystem and early vGPU results.
- **Snapshot mechanics:** Chromium cold-boots in roughly 5–10 s. Its RAM is saved to disk, and a restore from local NVMe takes about 30 ms — same post.
- **Unikraft's description:** Kernel boots Chromium once, snapshots it, and resumes that snapshot on each request; idle VMs can be re-snapshotted. Live state is kept up to 72 h — [Unikraft customer story](https://unikraft.com/customer-stories/scaling-browser-infrastructure) (updated 2026-01-24).
- **Hypeman** (MIT) — [kernel/hypeman](https://github.com/kernel/hypeman):
  - runs OCI images as VMs on Cloud Hypervisor, Firecracker or QEMU (KVM), plus Apple Virtualization.framework;
  - snapshots to disk and resumes within milliseconds;
  - supports vGPU and VFIO.
  - The README does not say it runs Kernel's production.
- **Conflict:** the security page still says Unikraft unikernels, while the blog describes moving to Hypeman. The fleet may be mixed, for example CPU browsers on Unikraft and GPU browsers on QEMU [unverified].
- **Faster Firecracker restores:** an HN thread on Kernel's post about starting Chromium in Firecracker in under 20 ms discusses `userfaultfd` lazy memory restore — [HN thread](https://news.ycombinator.com/item?id=48412708) (about June 2026) [inferred: Kernel experimented with Firecracker plus userfaultfd].
- **Published latencies:**

  | Figure | Source |
  |---|---|
  | Create about 30 ms p50, 105 ms p99 | [Kernel docs](https://www.kernel.sh/docs) |
  | Cold start under 20 ms (the same page's heading says sub-millisecond, a conflict) | [Kernel unikernels](https://www.kernel.sh/docs/info/unikernels) |
  | 2025: boot in 10–20 ms vs about 5 s for Docker | [HN launch thread](https://news.ycombinator.com/item?id=43705144) (2025-04-16) |
  | Vendor table run with steel-dev/browserbench: Kernel median 776 ms / p99 1,105 ms vs Browserbase 2,888 / 4,309 ms; metric not named | [Kernel benchmarks](https://www.kernel.sh/benchmarks) (undated, © 2026) |

- **Standby** — [Kernel standby](https://kernel.sh/docs/browsers/standby) (undated):
  - starts after 5 s without CDP, WebDriver BiDi, live-view or computer-control activity;
  - costs zero usage;
  - not available on GPU browsers.
  - The open-source Kraftfile sets `scale_to_zero.policy: idle`, `stateful: true`, `cooldown_time_ms: 5000` — [kernel-images](https://github.com/onkernel/kernel-images).
- **Limits** — [Kernel concurrency and limits](https://kernel.sh/docs/browsers/concurrency-and-limits.md) (undated):
  - `timeout_seconds` defaults to 60, maximum 72 h;
  - 8 GB for headful (up to 16 GB), 1 GB headless;
  - browsers in standby still use a concurrency slot.
- **Browser pools:** pre-configured browsers that skip start-up and the creation rate limit — [Kernel features](https://kernel.sh/docs/overview/features).

#### Browser Use Cloud
- **Browsers** — [Browser Use: Firecracker browser infra](https://browser-use.com/posts/firecracker-browser-infra) (2026-06-15):
  - Isolation: one Firecracker microVM per session, with its own CPU, memory, disk and network devices. Because the EC2 host is itself a VM, each browser is a nested VM.
  - Snapshot: taken after the VM boots and pauses, just before Chromium launches.
  - Memory: 2 MB pages instead of 4 KB, and a custom `userfaultfd` handler that preloads the pages Chromium is likely to touch.
  - Readiness: the host learns the browser is ready over vsock instead of polling HTTP.
  - Published figures:

    | Measure | Value |
    |---|---|
    | VM cold start | under 400 ms |
    | Resume to CDP-ready | 9.8 s before the memory work, 3.1 s after |
    | Missing-memory pauses per resume | about 100,000 → about 1,100 |
    | Chromium start after resume | about 545 ms p50 |
    | End-to-end create through the public API | 825 ms p50, 1.35 s p99 |
    | Stress test | 10,000 sessions, all started |

  - CPU pinning: vCPUs are pinned after the launch burst, each browser on both hyperthreads of one core. Giving vCPU threads real-time priority took session loss in a 1,000-browser test from 17% to zero.
  - Hosts boot from a pre-built image and serve browsers about 30 s after launch.
- **Agent sandboxes, stage 1:** Unikraft micro-VMs booted in under a second and suspended when idle. The same image runs locally in Docker, selected by `sandbox_mode` (`docker` or `ukc`) — [Two ways to sandbox agents](https://browser-use.com/posts/two-ways-to-sandbox-agents) (2026-02-25).
- **Agent sandboxes, stage 2 (AgentCore)** — [AgentCore migration](https://browser-use.com/posts/agentcore-migration) (2026-07-12):
  - one micro-VM per session in a private VPC with no inbound traffic;
  - the role can only pull from ECR and write to CloudWatch, and no secrets are in the VM;
  - 15-minute idle timeout, 8-hour maximum lifetime, 1 GB durable workspace.
- **Session-length conflict:** CLOUD.md says sessions are limited to 15 minutes, but its API section says paid users get up to 4 h — [Browser Use CLOUD.md](https://github.com/browser-use/browser-use/blob/main/CLOUD.md).
- **Concurrency:** set per project in tiers from 10 to 1,000 — [Browser Use quickstart](https://docs.browser-use.com/cloud/browser/quickstart) (undated).

#### Anchor Browser
- **Isolation:** each browser runs in a dedicated, isolated VM that is terminated and erased at session end, never reused across sessions or customers. The VM technology is not named — [Anchor security](https://docs.anchorbrowser.io/security.md) (undated).
- **Limits:** concurrency 5 / 25 / 200 / 500+ and creations 5 to 200+ per minute by plan — [Anchor plans](https://docs.anchorbrowser.io/pricing.md).
- **Start:** async session creation returns without waiting for provisioning, and batch sessions exist. No start latency is published — [Anchor create session](https://docs.anchorbrowser.io/quickstart/create-session.md).

#### Hyperbrowser
- **Browser sessions:** isolation is not stated on any docs page read.
- **Sandboxes** (a separate product) — [Hyperbrowser sandboxes](https://hyperbrowser.ai/docs/sandboxes/introduction.md), [sandbox snapshots](https://hyperbrowser.ai/docs/sandboxes/snapshots.md) (undated):
  - claim startup under 50 ms;
  - memory snapshots capture memory, processes and filesystem, and many sandboxes can start from one snapshot.
  - This implies a microVM-class layer [inferred: snapshot semantics].
- **Lifecycle:** the default timeout is a team setting, overridable with `timeoutMinutes`. Without `keepAlive=true` a session stops when the client disconnects — [Hyperbrowser lifecycle](https://hyperbrowser.ai/docs/sessions/lifecycle.md).

#### Airtop
- **States:** `initializing`, `awaiting_capacity`, `running`, `ended` — [Airtop creating a session](https://docs.airtop.ai/guides/how-to/creating-a-session.md) (undated).
  - Start normally takes seconds, but about a minute when hardware is not available.
  - Default idle timeout is 10 minutes.
  - This points to capacity-gated scheduling from a finite pool [inferred: state names].
- **Isolation:** not documented.

#### Skyvern Cloud
- **Persistent cloud sessions:** a code comment says they run on Temporal and ECS Fargate, which are not available in the open-source build — [Skyvern repo, `default_persistent_sessions_manager.py`](https://github.com/Skyvern-AI/skyvern) (committed 2026-10-08) [inferred: one Fargate task per session].
- **Self-hosted Kubernetes:** one browser per pod; at least 2 GB per pod, 8 GB+ in production — [Skyvern Kubernetes self-host](https://www.skyvern.com/docs/developers/self-hosted/kubernetes) [unverified: search snippet].
- **Timeouts:**
  - default 60 min, allowed 5–240, extendable to 360;
  - new sessions appear within seconds;
  - no warm pool is described.
  - [Skyvern browser sessions](https://www.skyvern.com/docs/cloud/browser-management/browser-sessions) (undated).
  - Another page says up to 24 hours — [Skyvern core concepts](https://www.skyvern.com/docs/developers/getting-started/core-concepts) [unverified: search snippet].

#### Notte
- **Isolation:** each session gets an isolated Chromium instance; the technology is not disclosed — [What is Notte](https://www.notte.cc/what-is-notte) (undated).
- **SDK defaults:** 3-minute idle timeout, 15-minute maximum session — [Notte SDK types](https://github.com/nottelabs/notte) (`packages/notte-sdk/src/notte_sdk/types.py`).
- **Anything API sandbox recovery** (not browser start):
  - Vercel snapshot restore: median 2.30 s, p95 6.94 s;
  - Daytona stop/start: median 1.27 s, p95 1.68 s.
  - [Notte sandbox post](https://www.notte.cc/blog/why-we-moved-the-agent-out-of-the-sandbox) (2026-09-09).

#### Lightpanda Cloud
- **One process, many clients:**
  - one Lightpanda process can serve many CDP connections;
  - each connection gets its own OS thread and an isolated browser instance with its own V8 isolate;
  - the V8 platform and some caches are shared.
  - The post does not say whether the cloud uses this mode — [How we built multi-client](https://lightpanda.io/blog/posts/how-we-built-multiclient) (2026-02-27).
- **Memory:** the engine claims about 16× less memory than headless Chrome — [Lightpanda 1.0](https://lightpanda.io/blog/posts/lightpanda-1-0) (2026-10-02).
- **Limits** — [Lightpanda limits and billing](https://lightpanda.io/docs/run-on-lightpanda-cloud/limits-and-billing) (undated):
  - CDP connections close after 15 min;
  - MCP sessions close after 5 min without requests;
  - sessions over the concurrency cap get 429.
- **Chrome option:** the cloud can run Google Chrome (`browser=chrome`), but only for debugging, not production — [Lightpanda Cloud getting started](https://lightpanda.io/docs/run-on-lightpanda-cloud/getting-started).

#### Apify
- **Runs:**
  - every Actor is a Docker image and runs in a Docker container on the platform — [Apify Dockerfile docs](https://docs.apify.com/academy/deploying-your-code/docker-file); [usage and resources](https://docs.apify.com/platform/actors/running/usage-and-resources) (undated);
  - each run gets its own filesystem, memory and CPU with caps;
  - platform services and customer workloads use separate compute;
  - the environment is destroyed after the run, with no state left on workers;
  - the mechanism is not named.
  - [Apify platform architecture](https://docs.apify.com/security/architecture) (undated).
- **MicroVMs:** the Core Services team owns running Actors from the Console's Start button to the container or microVM on Apify's servers — [Apify job board](https://api.ashbyhq.com/posting-api/job-board/apify), Senior Backend Engineer, Core Services (2026-04-20) [inferred: some runs use microVMs; technology unverified].
- **Resource model:**
  - one CPU core per 4,096 MB;
  - memory from 128 MB to 32 GB;
  - disk twice the memory;
  - a startup boost of up to a full core;
  - per-second metering.
  - [Apify usage and resources](https://docs.apify.com/platform/actors/running/usage-and-resources).
- **Scheduling for latency:** the orchestrator queues runs and places them on Actor compute, tuned to minimize startup latency under load. No figures are published — [Apify platform architecture](https://docs.apify.com/security/architecture).
- **Standby mode:**
  - keeps pre-warmed HTTP-serving instances and spreads requests across them — [Actor Standby mode](https://blog.apify.com/actor-standby-mode/) (2024-07-26; possibly stale);
  - readiness probe is `GET /` with the header `x-apify-container-server-readiness-probe`;
  - the first response may take up to 5 minutes;
  - idle runs stop after a timeout.
  - [Apify Standby docs](https://docs.apify.com/platform/actors/development/programming-interface/standby) (undated).
- **Live migration of runs:** the SDK receives `migrating` and `persistState` (with `isMigrating`) events over a WebSocket — [apify-sdk-js](https://github.com/apify/apify-sdk-js) (`src/platform_event_manager.ts`, `src/actor.ts`). An incident on 2026-07-21 reported unusually many migrations — [Apify status incident](https://status.apify.com/incidents/sz8kkrbcrk45). Why runs migrate (spot reclaim, rebalancing) is [unverified].
- **Images:** AWS ECR is a status-page dependency — [Apify status](https://status.apify.com) [inferred: Actor images are stored in ECR].

#### Zyte
- **Scrapy Cloud 2.0** — [Introducing Scrapy Cloud 2.0](https://www.zyte.com/blog/introducing-scrapy-cloud-2-0/) (2016-05-04; possibly stale):
  - jobs run in isolated Docker containers;
  - each job has a disk quota and a 256k-inode limit, and writes only under `/scrapy/`;
  - "stacks" are predefined images, and custom images are allowed;
  - the stated reasons were no noisy neighbours and more compute per unit.
- **Stacks are still current:** `scrapy:2.18-20260824` is the default — [Zyte Scrapy Cloud tutorial](https://docs.zyte.com/web-scraping/tutorials/main/cloud.html) (undated).
- **Disk quotas in the entrypoint:** the in-container entrypoint has a disk-quota module — [scrapinghub-entrypoint-scrapy](https://github.com/scrapinghub/scrapinghub-entrypoint-scrapy).
- **Own hardware:** Zyte upgraded the hardware of its internal Docker registry in December 2025 — [Zyte status incident](https://status.zyte.com/incidents/7dnttnplbg76) [inferred: self-hosted on dedicated servers].
- **Start delays:** throttling delayed job starts from 2025-08-26 to 28 — [Zyte status incident](https://status.zyte.com/incidents/jq1yk4dbdk4n). No start latency is published.
- **CDP browsers:** isolation, region and latency are not described — [Zyte CDP support](https://www.zyte.com/blog/introducing-zyte-cdp-support/) (2026-09-01).

#### Browserless
- **Isolation by tier:**
  - the shared fleet uses containers with network segmentation — [Browserless trust](https://browserless.io/trust) (2026-09-11);
  - private deployments run workers on isolated VMs not shared with other customers — [Browserless private deployment](https://docs.browserless.io/enterprise/private-deployment/overview.md) (undated).
- **Many browsers per container:** one container runs many browsers behind a concurrency limiter and a queue — [browserless repo](https://github.com/browserless/browserless) (`src/limiter.ts`) [inferred: file names].
  - In 2019 the guidance was 10–20 sessions per machine — [Headless browser practices](https://www.browserless.io/blog/headless-browser-practices) (2019; possibly stale).
- **Reconnect:** `reconnect()` keeps the browser process alive after a disconnect for `timeout` (default 30 s), and the waiting browser still holds a concurrency slot — [Browserless reconnects](https://docs.browserless.io/bap/session-management/reconnects.md) (undated).

#### Firecrawl
- **Engine order:**
  1. index (cache);
  2. `fire-engine;chrome-cdp` (with stealth and retry variants);
  3. `fire-engine;tlsclient`;
  4. `playwright`;
  5. `fetch`.
  - [engines/index.ts](https://github.com/firecrawl/firecrawl/blob/main/apps/api/src/scraper/scrapeURL/engines/index.ts) (accessed 2026-10-12) [inferred: `tlsclient` is a TLS-fingerprinting HTTP client].
- **Hangar:** interactive browser sessions come from an internal service called Hangar (`HANGAR_URL`) — [hangar.ts](https://github.com/firecrawl/firecrawl/blob/main/apps/api/src/lib/hangar.ts) (verified 2026-10-12):
  - it returns `cdp_url`, `view_url`, `control_url` and `playlist_url`;
  - sessions have `suspending`, `suspended` and `resuming` states, which suggests snapshot-capable sandboxes [unverified].
- **Self-hosted Playwright container:**
  - drops all capabilities and sets `no-new-privileges`;
  - 2 CPU and 4 GB;
  - `MAX_CONCURRENT_PAGES=10`, `BROWSER_POOL_SIZE=5`.
  - [docker-compose.yaml](https://github.com/firecrawl/firecrawl/blob/main/docker-compose.yaml).
- **Limits:**
  - concurrent browsers from 2 (Free) to 100+ (Scale);
  - time spent queued counts against the request timeout;
  - queued jobs expire after 48 h, and a full queue returns 429.
  - [Firecrawl rate limits](https://docs.firecrawl.dev/rate-limits) (undated).

#### Spider Cloud
- **Isolation:** each session gets its own isolated browser context, from a runtime that drives Chromium, Gecko and WebKit in one process. No cold-start penalty is claimed, with no numbers — [Spider MCP v2](https://spider.cloud/blog/spider-mcp-v2-browser-automation-for-ai-agents/) (2026-02-16) [inferred: isolation at the context level, not per VM].
- **Pooling building block** — [spider-rs/headless-browser](https://github.com/spider-rs/headless-browser) [inferred: pre-started Chrome behind a load balancer]:
  - a control server on port 6000 forks and shuts down Chrome processes;
  - Chrome listens on 9223 behind a TCP proxy on 9222;
  - its headless-shell image is about 300 MB, against 1.2 GB for Playwright's.
- **Limits:** 100 concurrent browsers on every plan. Nothing queues: concurrency is requests in flight — [Spider docs overview](https://spider.cloud/docs/overview/).

#### Cloudflare Browser Run
- **Isolation:**
  - each binding gets a dedicated Chromium instance — [wrangler reference](https://github.com/cloudflare/cloudflare-docs/blob/production/src/content/docs/browser-run/reference/wrangler.mdx);
  - incognito contexts isolate tasks within a session, and the docs advise a fresh browser for process-level isolation — [Browser Run FAQ](https://github.com/cloudflare/cloudflare-docs/blob/production/src/content/docs/browser-run/faq.mdx);
  - request headers `cf-biso-request-id` and `cf-biso-devtools` suggest shared roots with Cloudflare Browser Isolation — [automatic request headers](https://github.com/cloudflare/cloudflare-docs/blob/production/src/content/docs/browser-run/reference/automatic-request-headers.mdx) [inferred].
- **Fast start:** a global pool of warm instances with no cold-start wait — [Browser Run for AI agents](https://blog.cloudflare.com/browser-run-for-ai-agents) (2026-04-15).
- **Limits on Workers Paid** — [Browser Run limits](https://developers.cloudflare.com/browser-run/limits/) (verified 2026-10-12):
  - 200 concurrent browsers per account, 3 new per second;
  - Quick Actions at 30 requests per second;
  - 60 s idle timeout, extendable to 10 min with `keep_alive`;
  - no fixed maximum lifetime, but sessions close when a new release rolls out.
- **Limit history:**
  - 2025-01-31: 2 → 10 concurrent;
  - 2026-04-15: 30 → 120 concurrent;
  - no release note found for the later step to 200.
  - [Browser Run release notes](https://github.com/cloudflare/cloudflare-docs/blob/production/src/content/release-notes/browser-run.yaml).
- **Kitesurf (beta), a browser built on Workers** — [Kitesurf](https://blog.cloudflare.com/kitesurf) (2026-08-06):
  - Design:
    - Rust compiled to WebAssembly;
    - Blitz and Stylo for HTML/CSS, Parley for text, Boa for `eval`;
    - each page's scripts run in a Dynamic Workers isolate;
    - only one SandboxOutbound component can reach the network;
    - every session starts fresh.
  - Against warm-pool Chromium: 3.1–3.8× less CPU and 4.7–7.0× less memory, but 1.7–1.8× slower wall time.
  - It cannot yet handle WebGL, TLS-fingerprint bot challenges or long logged-in sessions.

#### Bright Data
- **Session limits:**
  - at most 60 minutes;
  - closed after 5 minutes without CDP commands or without network activity;
  - the client must connect within 30 s;
  - one domain per session.
  - [Bright Data Browser API error codes](https://docs.brightdata.com/scraping-automation/scraping-browser/error-codes) (undated).
  - Bright Data's own agent-skill reference still says 30 minutes, so the limit was likely raised — [opencode-brightdata 1.0.0 reference](https://cdn.jsdelivr.net/npm/@brightdata/opencode-brightdata@1.0.0/skills/bright-data-best-practices/references/browser-api.md) [inferred].
- **Error codes** `no_free_workers`, `worker_disconnect` and `job_killed` show browsers running as jobs on a finite worker pool. The skill reference also says a 503 means the service is scaling and to retry after a minute [inferred: reactive pool scaling].
- **Isolation:** container or VM per session is not stated — [Bright Data Browser API introduction](https://docs.brightdata.com/products/scraping-browser/introduction).

#### ScrapingBee, ScraperAPI, ZenRows
- **ScrapingBee** — [ScrapingBee docs](https://www.scrapingbee.com/documentation/) (undated):
  - JavaScript rendering is on by default;
  - `wait_browser` accepts Puppeteer lifecycle event names [inferred: Puppeteer and Chrome];
  - `session_id` keeps requests on the same IP;
  - per-request isolation is not documented.
- **ZenRows:**
  - Browser Sessions connect at `wss://browser.zenrows.com`, with a TTL of 1–15 minutes — [ZenRows Browser Sessions](https://www.zenrows.com/products/browser-sessions); [ZenRows docs index](https://docs.zenrows.com/llms.txt) (undated);
  - requests over the concurrency limit get 429 and are not queued;
  - a cancelled request can hold its slot for up to 3 minutes — [ZenRows concurrency](https://docs.zenrows.com/fetch/features/concurrency.md) [inferred: work continues server-side after a client disconnect].
- **ScraperAPI:** recommends a 70 s client timeout and limits requests to 50 MB — [ScraperAPI docs](https://docs.scraperapi.com) (undated).

### Inferences
- **A VM per browser is now the norm for products where untrusted parties drive a browser.** Goodfetch's plan (an engine process per run, isolated in a container or VM, with a Browser host beside each hosted browser) matches it. Containers survive where the operator controls the code (Zyte stacks), or where the tier is cheap (Browserless shared fleet, Cloudflare).
- **Snapshot-before-launch is the published path to sub-second starts.** It is preferred to keeping idle browsers warm:
  - Kernel restores RAM snapshots in about 30 ms.
  - Browser Use resumes a pre-Chromium snapshot, then launches Chromium for about 825 ms p50 end-to-end.
  - Both Kernel and Steel rejected warm pools of idle browsers for cost.
- **Kernel's standby matches Goodfetch's checkpoint/resume model.** The ingress proxy keeps the client's CDP socket open while the VM is snapshotted, which is the transport half of a resumable hosted browser.
- **Live migration is a requirement in practice.** Kernel left Unikraft partly because snapshots were pinned to one host. Apify migrates runs between servers through SDK events. Goodfetch's portable checkpoint state addresses the same need at the application layer.
- **Lightweight engines change the cost model.** Lightpanda (multi-client threads per process) and Cloudflare Kitesurf (isolates) trade compatibility for 4–16× lower memory. That suits an HTTP-first crawl tier escalating to full Chromium, which is Foxhound's design.

### Gaps
- **Browserbase's VMM** (Firecracker, Cloud Hypervisor or another), warm pools, and per-host density: not published.
- **Steel's 2026 isolation technology** (still Firecracker?) and how it avoids cold starts: not published.
- **Isolation technology not documented** for Anchor, Hyperbrowser (browser sessions), Airtop, Notte and Cloudflare's Chromium fleet.
- **Apify's microVM technology** and the share of runs on it: unknown.
- **Search could not be used** to find conference talks (Unikraft, Firecracker or KubeCon) about these fleets.

## 3. What orchestration, scheduling, autoscaling, queues, storage and live-view/CDP proxying do they use?

### Takeaway
**Control planes commonly run on Kubernetes:**
- Browserbase: Kubernetes on AWS with ALB ingress in four regions.
- Apify: EKS, plus its own container orchestrator for Actor runs.
- Firecrawl: Kubernetes on GCP.
- Zyte: Kubernetes and Knative per a 2026 job post.

**The VM-isolated browser data planes do not use Kubernetes scheduling:**
- Custom control planes: Browser Use (with stateless CDP edge routers), Kernel (Hypeman per host, plus a CDP-holding ingress proxy) and Steel.
- Managed serverless:
  - Skyvern: ECS Fargate plus Temporal.
  - Browser Use agents: SQS → Lambda, then Bedrock AgentCore.
  - Cloudflare: Durable Objects for session reuse.

**Regional WebSocket gateways front CDP:** Browserbase `connect.<region>`, Steel `connect.steel-edge.net`, Bright Data super proxies on port 9222, `browser.spider.cloud`, Lightpanda's regional load balancers.

**Object storage holds recordings and artifacts:** S3 at Browserbase, Browser Use and Skyvern; GCS at Firecrawl; R2 at Cloudflare; Tigris probably at Hyperbrowser.

**Recordings** come as HLS/MP4 video or rrweb DOM streams. **Queues** are often built in-house or on Postgres: Firecrawl's NuQ, Apify's DynamoDB/S3-backed request queue.

### Cited Findings

#### Apify
- **Own orchestration:** Apify runs its own container orchestration platform to balance performance and cost. The CEO calls scaling to millions of container runs per day, and isolating them, a hard problem — [Cerebral Valley interview with the CEO](https://cerebralvalley.beehiiv.com/p/apify-is-building-the-infrastructure-for-ai-s-data-problem) (2025-09-16).
- **Stack in 2026 job posts** — [Apify job board](https://api.ashbyhq.com/posting-api/job-board/apify) (Core Services 2026-04-20; Billing 2026-10-01):
  - AWS compute: EKS, EC2, Lambda;
  - Helm, Terraform, CloudFormation, ArgoCD;
  - MongoDB, Redis, DynamoDB, S3;
  - Grafana, Prometheus, OpenTelemetry, Mezmo, Sentry, PagerDuty;
  - TypeScript/Node.js with Nest.js.
- **Status-page dependencies:** AWS EC2, S3, SQS, ELB, ElastiCache, ECR, EKS and DynamoDB, plus Mailgun, Stripe and npm — [Apify status](https://status.apify.com) (accessed 2026-10-12).
- **2020 stack:** EC2, S3, DynamoDB, SQS, ECS, Lambda and CloudFront — [Apify security whitepaper](https://apify.com/security-whitepaper.pdf) (2020; possibly stale). ECS in 2020 against EKS in 2026 suggests a migration; no write-up was found [inferred].
- **Storage:**
  - datasets, key-value stores and request queues are on DynamoDB and S3, replicated across AZs;
  - MongoDB Atlas is the primary application database, replicated with managed backups;
  - capacity autoscales, and the API enforces per-account rate limits.
  - [Apify platform architecture](https://docs.apify.com/security/architecture) (verified 2026-10-12).
- **Request queue** — [Apify request queue](https://docs.apify.com/storage/request-queue.md) (undated):
  - `listAndLockHead` with `lockSecs`; locks expire, so failed requests come back;
  - per-queue limits: 350 req/s for CRUD, 35 req/s for batch and lock;
  - unnamed storages expire after 7 days.
- **Queueing under pressure:** when capacity was capped on 2026-07-21, runs were held in READY and many timed out — [Apify status incident](https://status.apify.com/incidents/fqkp6kdb0wy2).
- **Run web server:** each run can expose a public web server through `ACTOR_WEB_SERVER_URL` and `ACTOR_WEB_SERVER_PORT` while it runs — [Apify environment variables](https://docs.apify.com/platform/actors/development/programming-interface/environment-variables).
- **Proxy:**
  - built on Apify's open-source `proxy-chain` and served at `proxy.apify.com:8000`;
  - groups include datacenter, RESIDENTIAL, GOOGLE_SERP and UNBLOCKER;
  - traffic from inside the platform bypasses the public internet.
  - [Apify Proxy docs](https://docs.apify.com/proxy.md).
  - Status entries mention fixes "upstream" for some proxy groups — [Apify status history](https://status.apify.com/history.atom) [inferred: some IP pools come from third-party providers].

#### Zyte
- **2026 platform job post:**
  - Kubernetes and Knative as core ownership, Mesos as a bonus;
  - GPU scheduling and autoscaling;
  - Kafka, plus Argo, Temporal or Airflow;
  - Go, Rust, Python, Java and C++.
  - [Zyte job post (Built In mirror)](https://builtin.com/job/platform-engineering-team-lead-remote/7827909) (removed 2026-01-14) [inferred: Kubernetes is the direction; Mesos may be legacy].
- **AutoExtract (2019):** ran on GKE, with Kafka feeding downloader and ML instances. Zyte moved from self-run Kafka to Confluent Cloud to cut operations work — [Confluent blog](https://www.confluent.io/blog/why-scrapinghub-chose-confluent-cloud-kafka-service/) (2019-10-03; possibly stale).
- **Status components:**
  - Scrapy Cloud: Job Execution and Storage, Project Deployment, Periodic Job Scheduling;
  - Zyte API; Smart Proxy Manager; Web Dashboard.
  - [Zyte status](https://status.zyte.com).
  - A "cluster upgrade" maintenance window ran on 2025-10-14 — [Zyte status incident](https://status.zyte.com/incidents/dmw2h6bkzxzf).
- **Job data path:**
  - jobs write items and logs to a named pipe (`SHUB_FIFO_PATH`) that the job runner reads;
  - job identity comes from `SHUB_JOBKEY`, `SHUB_JOBAUTH` and `SHUB_APIURL`.
  - [scrapinghub-entrypoint-scrapy](https://github.com/scrapinghub/scrapinghub-entrypoint-scrapy) [inferred: the runner forwards to HubStorage at `storage.scrapinghub.com`].
- **Storage APIs:** Hub Crawl Frontier for request data and Collections as a key-value store. Container-local files are lost when the job ends — [Scrapy Cloud secrets](https://www.zyte.com/blog/scrapy-cloud-secrets-hub-crawl-frontier-and-how-to-use-it/) (2020-08-06, modified 2025-08-21; possibly stale).
- **Proxy pools:** after an upstream provider disruption, Zyte isolated the affected nodes from its proxy pools — [Zyte status incident](https://status.zyte.com/incidents/hc31ncl6hcwh) (2026-04-14).

#### Browserbase
- **Components** — [What is a Browserbase browser](https://www.browserbase.com/blog/what-is-a-browserbase-browser) (2026-06-16, verified):
  - a Fastify REST API gateway at `api.browserbase.com`;
  - a Connect service, a WebSocket bridge that proxies CDP to the right browser pod;
  - a browser service managing Chromium's lifecycle;
  - an identity layer for fingerprints, proxy chains, verified mode and CAPTCHA;
  - S3 for replays, contexts, uploads and extension bundles.
- **Replays** — [Browserbase session replay](https://docs.browserbase.com/platform/browser/observability/session-replay.md) (undated):
  - video per tab, up to 10 tabs, delivered as an HLS playlist of fragmented-MP4 segments on Browserbase's CDN;
  - signed URLs last 6 hours, and recordings are kept up to 30 days;
  - recording can be turned off with live view still working.
- **Live view:** an iframe (`debuggerFullscreenUrl`, per-tab `pages[]`) that can be read-only or interactive. The transport is not documented — [Browserbase live view](https://docs.browserbase.com/platform/browser/observability/session-live-view.md).
- **Region routing:** the hosted Stagehand API routes each call to the session's region (`REGION_API_URLS`), and a region mismatch returns 400 — [Stagehand PR #1673](https://github.com/browserbase/stagehand/pull/1673) (merged 2026-02-19).
- **Functions:** TypeScript runs next to the browser, under 5 ms away, only in us-west-2. Each invocation creates a session, and invocations are asynchronous with webhooks — [Browserbase Functions](https://docs.browserbase.com/platform/functions/overview.md) (undated).
- **Why multi-region:** sequential CDP round trips stack. Moving browsers near customers cut network latency by about 87–98% (Europe 152 → 3 ms, Asia 169 → 5 ms) and made multi-step tasks more than twice as fast. The migration took about a month — [Becoming multi-region](https://www.browserbase.com/blog/becoming-multi-region) (2024-12-06; possibly stale; verified).

#### Steel.dev
- **Orchestrator:** a closed cloud orchestrator hosts a pool of browser instances and routes each connection to one. In 2024 the founders planned to open-source it shortly; no such repo exists today — [HN: Steel launch](https://news.ycombinator.com/item?id=42245573) (2024-12-01; possibly stale); [steel-dev on GitHub](https://github.com/steel-dev) [inferred: absence].
- **Open-source runtime:**
  - Node 22 with Fastify;
  - `puppeteer-core` driving Chromium under Xvfb;
  - `http-proxy` proxies CDP over WebSocket on port 9223;
  - `proxy-chain` for upstream proxies, and fingerprint generator/injector packages;
  - logs in a DuckDB file;
  - an rrweb-based recorder extension, and WebSocket handlers for cast, logs and recording.
  - [steel-browser repo](https://github.com/steel-dev/steel-browser) [inferred: whether Steel Cloud uses the same recorder is unknown].
- **Status incidents** on the edge gateway, the streaming/recording pipeline, database maintenance and capacity (July–October 2026) show a gateway → orchestrator → recording-pipeline layout — [Steel status](https://status.steel.dev) [inferred: from incident titles].
- **Cloud-only features:** managed proxies, CAPTCHA solving, Credentials and Files APIs; local self-hosting is limited to 1 concurrent session — [Steel local vs cloud](https://docs.steel.dev/overview/self-hosting/steel-local-vs-steel-cloud) (undated).

#### Browserless
- **Routing:** regional endpoint → regional load balancer → instance, using modified least-connections; requests to overloaded machines are rerouted — [Browserless load balancers](https://docs.browserless.io/baas/infrastructure/load-balancers).
- **Router history:** v1 used custom NGINX + Lua load balancers; v2 a custom TypeScript router. Failing WebSocket routes mid-request were hard, and routing was redesigned several times — [Rebuilding Browserless](https://www.browserless.io/blog/rebuilding-browserless) (2024-02-21; possibly stale).
- **Private deployments:** regional load balancers over dedicated workers, which can also be addressed directly for sticky sessions — [Browserless private deployment](https://docs.browserless.io/enterprise/private-deployment/overview.md).
- **No Kubernetes artifacts:** the repo has no Helm, Kubernetes or Terraform files, and the docs have NGINX load-balancing pages instead — [browserless repo](https://github.com/browserless/browserless); [Browserless docs index](https://docs.browserless.io/llms.txt).
- **Session Replay** — [Browserless session replay](https://docs.browserless.io/baas/monitor-sessions/session-replay.md):
  - rrweb DOM recording, reinjected after each navigation;
  - stopped with the custom CDP command `Browserless.stopSessionRecording`, then uploaded;
  - kept 30 days.
  - A separate screen recording produces WebM.

#### Kernel
- **Hypeman on each host:** a JWT-authenticated remote API running as a systemd service, with built-in ingress (TLS termination and subdomain routing) — [kernel/hypeman](https://github.com/kernel/hypeman). The scheduler that places VMs across hosts is not described.
- **Standby connection hold:** an ingress proxy keeps the client's CDP connection open while the VM is snapshotted, then resumes the VM on activity — [Kernel blog: scale](https://www.kernel.sh/blog/scale); [Unikraft customer story](https://unikraft.com/customer-stories/scaling-browser-infrastructure) (both verified).
- **Live view and recording** — [kernel-images](https://github.com/onkernel/kernel-images):
  - the open-source image offers noVNC, or WebRTC adapted from Neko;
  - on Unikraft Cloud, WebRTC needs a TURN server;
  - a recording API on port 10001 produces H.264 MP4.
- **Code beside browsers:** the App Platform runs customer code in isolated sandboxes next to the browsers, on the same Unikraft technology, to cut CDP round trips — [Unikraft customer story](https://unikraft.com/customer-stories/scaling-browser-infrastructure).
- **Telemetry:** navigation, network, CDP, captcha and proxy events stream out — [Kernel features](https://kernel.sh/docs/overview/features).
- **Profiles and replays** are not region-bound; no storage backend is named — [Kernel regions](https://kernel.sh/docs/browsers/regions.md).

#### Browser Use Cloud
- **Browser control plane:**
  - watches the fleet in real time rather than on CloudWatch's one-minute windows;
  - places sessions on hosts with room, scales hosts up and down, and drains hosts being removed;
  - stateless edge routers accept the WebSocket, ask the control plane where the browser lives, and forward raw CDP bytes.
  - [Browser Use: Firecracker browser infra](https://browser-use.com/posts/firecracker-browser-infra) (2026-06-15, verified).
- **Agent pipeline for the library-based product** — [Browser Use production architecture](https://browser-use.com/posts/production-architecture-browser-use) (2026-05-09, verified):
  - a FastAPI service on ECS Fargate puts each run on one standard SQS queue, and Lambda workers pick runs up;
  - to get past Lambda's 15-minute limit, agents checkpoint to S3 and re-enqueue themselves, up to 12 times (about 3 h);
  - failed messages go to a dead-letter queue after 3 tries and are not redriven automatically;
  - S3 holds checkpoints, screenshots and logs; Postgres holds task metadata.
- **Sandboxed-agent control plane** — [Two ways to sandbox agents](https://browser-use.com/posts/two-ways-to-sandbox-agents) (2026-02-25):
  - a stateless FastAPI service on Fargate behind an ALB, scaling on CPU;
  - it holds all credentials, proxies LLM calls, stores conversation history, enforces cost caps and handles billing;
  - sandboxes upload files through presigned S3 URLs.
- **AgentCore wiring** — [AgentCore migration](https://browser-use.com/posts/agentcore-migration) (2026-07-12):
  - `invoke_agent_runtime` and `stop_runtime_session`, managed with Terraform;
  - session state in SQLite on the durable mount;
  - outputs promoted to S3;
  - the control plane is the source of truth for recovery.
- **Recording and live preview:** recording is off by default, live preview works only while the browser is active, and zero-data-retention projects never record — [Browser Use quickstart](https://docs.browser-use.com/cloud/browser/quickstart).

#### Skyvern Cloud
- **Workflow engine:** Temporal runs cloud-side, per the code comment cited in section 2. The schedule type carries `cron_expression` and `temporal_schedule_id` — [Skyvern repo](https://github.com/Skyvern-AI/skyvern) (`skyvern/client/types/workflow_schedule.py`) [inferred: Temporal Schedules].
- **AWS calls in the code:** the AWS client wraps ECS (`run_task`, `stop_task`) and AWS Batch (`submit_job`) — [Skyvern repo](https://github.com/Skyvern-AI/skyvern) (`skyvern/forge/sdk/api/aws.py`). What Batch is used for is [unverified].
- **Live view:**
  - config sets `chromium-headful` with VNC streaming mode;
  - the Dockerfile installs `x11vnc` and `websockify`;
  - recordings use Playwright's ffmpeg encoder;
  - storage can be local, S3, Azure Blob or GCS;
  - Postgres 14 in the open-source stack.
  - [Skyvern repo](https://github.com/Skyvern-AI/skyvern) (`config.py`, `Dockerfile`, `docker-compose.yml`) [inferred: VNC over WebSocket].
- **Status components:** Skyvern API, Skyvern Cloud web app and Skyvern Async Workers — [Skyvern status](https://status.skyvern.com).
- **Local browser tunnel:** `skyvern browser serve --tunnel` lets the cloud drive a local Chrome through an ngrok URL — [Skyvern repo README](https://github.com/Skyvern-AI/skyvern).

#### Hyperbrowser, Anchor, Airtop, Notte, Lightpanda
- **Hyperbrowser:**
  - session creation returns `wsEndpoint` and `liveUrl` — [Hyperbrowser lifecycle](https://hyperbrowser.ai/docs/sessions/lifecycle.md);
  - recordings are rrweb JSON by default, MP4 optional, fetched after the session stops — [Hyperbrowser recordings](https://hyperbrowser.ai/docs/sessions/recordings.md);
  - static IPs are allocated per team — [Hyperbrowser static IPs](https://hyperbrowser.ai/docs/sessions/static-ips.md).
- **Anchor:**
  - sessions return `cdp_url` and `live_view_url` — [Anchor create session](https://docs.anchorbrowser.io/quickstart/create-session.md);
  - Anchor publishes GoVNC, a pure-Go VNC server library (Apache-2.0) — [anchorbrowser/GoVNC](https://github.com/anchorbrowser/GoVNC) [inferred: VNC-based live view];
  - files go peer-to-peer to the customer and are not stored — [Anchor security](https://docs.anchorbrowser.io/security.md).
- **Airtop:** clients connect over CDP to `cdpWsUrl` with an `Authorization: Bearer` header. It offers profiles saved on termination, embeddable live view, recordings and a files API — [Airtop docs](https://docs.airtop.ai/); [Airtop docs index](https://docs.airtop.ai/llms.txt).
- **Notte:**
  - a code comment says CDP URLs are WebSocket endpoints on Notte's own API (CDP proxied through `api.notte.cc`);
  - `viewer_url` and `debug_url` provide live view.
  - [Notte SDK sessions endpoint](https://github.com/nottelabs/notte) (`packages/notte-sdk/src/notte_sdk/endpoints/sessions.py`) [inferred].
  - Status components include Session API, Scrape API, Proxies and Captcha solving — [Notte status](https://status.notte.cc).
- **Lightpanda:** the token travels in the CDP URL query string, and a `proxy` parameter selects a proxy — [Lightpanda Cloud getting started](https://lightpanda.io/docs/run-on-lightpanda-cloud/getting-started). An open-source Go `cdpproxy` logs every CDP message, but its cloud use is unknown — [lightpanda-io/cdpproxy](https://github.com/lightpanda-io/cdpproxy).

#### Firecrawl
- **Kubernetes:** the code has `IS_KUBERNETES` and pod-name settings — [config.ts](https://github.com/firecrawl/firecrawl/blob/main/apps/api/src/config.ts). A job ad lists search clusters, crawling fleets, queues and databases on Kubernetes — [Firecrawl job ad](https://haystackapp.io/jobs/6690a216-14d3-407a-ba74-4066b1b843a1) (2026-08-20) [inferred: GKE or self-managed, unknown].
- **NuQ, their own queue on Postgres 17 with `pg_cron`:**
  - a scrape-queue table with priority, locks, stall counters, and owner and group IDs, plus a per-owner backlog;
  - comments mention an 18M-row production table and tuning after checkpoints made prefetch queries return nothing.
  - [nuq.sql](https://github.com/firecrawl/firecrawl/blob/main/apps/nuq-postgres/nuq.sql).
- **Other queue backends:** RabbitMQ carries webhooks and indexer jobs. FoundationDB is an experimental replacement for NuQ's Postgres — [docker-compose.yaml](https://github.com/firecrawl/firecrawl/blob/main/docker-compose.yaml). In 2024 the queue was Bull/BullMQ on Redis — [An adventure in scaling](https://www.firecrawl.dev/blog/an-adventure-in-scaling) (possibly stale).
- **Storage and services** — [config.ts](https://github.com/firecrawl/firecrawl/blob/main/apps/api/src/config.ts); [db/connection.ts](https://github.com/firecrawl/firecrawl/blob/main/apps/api/src/db/connection.ts); [native/Cargo.toml](https://github.com/firecrawl/firecrawl/blob/main/apps/api/native/Cargo.toml):
  - Postgres through drizzle, with a read replica; Supabase variables appear;
  - Redis split by purpose;
  - GCS buckets for results, screenshots, media and the page index;
  - Bigtable tables for job and credit state;
  - ClickHouse analytics;
  - a Go HTML-to-Markdown service and a Rust napi module.
- **Autoscaling:** not published. The self-host Helm example uses fixed replicas — [Helm values](https://github.com/firecrawl/firecrawl/blob/main/examples/kubernetes/firecrawl-helm/values.yaml).
- **Proxy layer:**
  - fire-engine chooses basic, stealth (mobile) or `auto` proxies — [fire-engine index.ts](https://github.com/firecrawl/firecrawl/blob/main/apps/api/src/scraper/scrapeURL/engines/fire-engine/index.ts);
  - a job ad gives a hire ownership of proxy health, rotation and cost — [Firecrawl job ad](https://haystackapp.io/jobs/4afa3e6b-fcf2-4c99-aa80-3a629ea60454) (2026-07-12).

#### Spider Cloud
- **Pipeline:** fetch, process and deliver stages run concurrently. "Smart" mode picks HTTP or a browser per page — [Spider docs overview](https://spider.cloud/docs/overview/).
- **Crate hints:**
  - `cache` and `cache_chrome_hybrid` features, and a `hybrid_cache_server` repo, point to a shared remote cache;
  - `decentralized` and `disk_aws` features exist.
  - [spider Cargo.toml](https://github.com/spider-rs/spider/blob/main/spider/Cargo.toml); [spider-rs org](https://github.com/spider-rs) [inferred].
- **Proxy endpoints:**
  - `proxy.spider.cloud` on 8888/8889/8887;
  - options are encoded in the credentials;
  - residential and ISP pools totalling 215,913,040 IPs;
  - country pinning either honours the country or fails.
  - [Spider proxy locations](https://spider.cloud/proxy-locations/).
- **Output:** results stream as JSONL, with connectors to S3, GCS, Azure Blob, Sheets and Supabase — [Spider docs overview](https://spider.cloud/docs/overview/).

#### Cloudflare Browser Run
- **Session reuse:**
  - `puppeteer.connect(env.MYBROWSER, sessionId)` reconnects to a running browser — [Browser Run FAQ](https://github.com/cloudflare/cloudflare-docs/blob/production/src/content/docs/browser-run/faq.mdx);
  - a Durable Objects pattern keeps a browser for 60 s after last use — [Browser Run with Durable Objects](https://github.com/cloudflare/cloudflare-docs/blob/production/src/content/docs/browser-run/how-to/browser-run-with-do.mdx).
- **`/crawl`:** runs as an async job for up to 7 days, with results kept 14 days in R2 — [crawl endpoint](https://github.com/cloudflare/cloudflare-docs/blob/production/src/content/docs/browser-run/quick-actions/crawl-endpoint.mdx).
- **Caching and retention:** Quick Action output is cached for 5 s by default, and session recordings are kept 30 days — [Browser Run FAQ](https://github.com/cloudflare/cloudflare-docs/blob/production/src/content/docs/browser-run/faq.mdx).
- **Identity instead of proxies:** traffic is always signed as a bot with Web Bot Auth and carries fixed bot-detection IDs — [automatic request headers](https://github.com/cloudflare/cloudflare-docs/blob/production/src/content/docs/browser-run/reference/automatic-request-headers.mdx).
- **Live View** is at `live.browser.run` — [Browser Run release notes](https://github.com/cloudflare/cloudflare-docs/blob/production/src/content/release-notes/browser-run.yaml).

#### Bright Data
- **Connect endpoints:** CDP clients connect to `wss://USER:PASS@brd.superproxy.io:9222`, Selenium to port 9515, on the same hostnames as the proxy network — [Bright Data Browser API configuration](https://docs.brightdata.com/scraping-automation/scraping-browser/configuration) [inferred: browser and proxy traffic share one edge layer, on Vultr per DNS].
- **Custom CDP domains:**
  - `Proxy.setLocation` and `Proxy.useSession` (sticky peer);
  - `Captcha.setAutoSolve` and `Captcha.solve`, with captchas solved inside the hosted browser by default;
  - plus `Emulation.setDevice` and `Download.*`.
  - [opencode-brightdata reference](https://cdn.jsdelivr.net/npm/@brightdata/opencode-brightdata@1.0.0/skills/bright-data-best-practices/references/browser-api.md).
- **Peer scheduling:** errors `no_peers`, `reserve_no_result` and `proxy_cooling_peers` reveal peer reservation with per-peer cool-down. Compliance blocks `brob` (robots.txt) and `brul` (Bright Data rules) are enforced in the request path — [Bright Data error codes](https://docs.brightdata.com/scraping-automation/scraping-browser/error-codes) [inferred].

#### ScrapingBee, ScraperAPI, ZenRows
- **ScrapingBee:** Auto-Mode tries configurations from cheapest to most expensive and bills only the one that succeeds — [ScrapingBee docs](https://www.scrapingbee.com/documentation/) [inferred: an escalation ladder in the orchestrator]. Datadog handles logs and monitoring — [ScrapingBee DPA](https://www.scrapingbee.com/data-processing-agreement/).
- **ScraperAPI:** separate synchronous, async and DataPipeline (bulk and scheduled) endpoints — [ScraperAPI docs](https://docs.scraperapi.com) [inferred: a queued async tier].
- **ZenRows:**
  - retries swap upstream providers with backoff — [ZenRows reliability](https://www.zenrows.com/reliability) [inferred: several upstream proxy vendors];
  - Batch takes up to 100,000 URLs per job — [ZenRows docs index](https://docs.zenrows.com/llms.txt).

### Inferences
- **Two orchestration layers is the recurring shape.** An ordinary platform layer (Kubernetes or ECS) runs APIs, queues and storage. A custom or managed placement layer runs untrusted, VM-isolated workloads. Browser Use, Kernel and Steel each built their own browser scheduler and did not use Kubernetes for it. Apify built its own run orchestrator beside EKS.
- **The CDP gateway is a first-class, regional, stateless component.** It looks up placement, forwards bytes, and in Kernel's case holds sockets across VM sleep. Goodfetch's Browser host design (one high-level action per round trip, raw CDP opt-in) sits behind that kind of gateway.
- **Co-locating code with the browser is how vendors fight CDP latency.** Browserbase's multi-region measurements, Browserbase Functions and Kernel's App Platform all target it. Goodfetch's Browser host does the same thing at the protocol level.
- **Recording takes two forms:** video (HLS/MP4 at Browserbase, MP4 at Kernel) or rrweb DOM streams (Browserless, Hyperbrowser, Steel OSS). Both go to object storage with signed URLs and 7–30-day retention.
- **Queues are commonly built in-house on databases teams already run:** Firecrawl's Postgres NuQ (with FoundationDB in trial), Apify's DynamoDB/S3 request queue, Browser Use's SQS with S3 checkpoints. Durable queue semantics (locks with expiry, per-owner backlog, dead-letter queues) are the parts they write about.

### Gaps
- **Not published:** Browserbase's browser-pod scheduler and queue technology; Kernel's cross-host scheduler; Steel's orchestrator; Hyperbrowser's and Airtop's fleet management; Cloudflare's Chromium fleet scheduling.
- **Unknown:** whether Apify Actor runs are Kubernetes pods or placed by its own scheduler on EC2, and whether it uses spot capacity.
- **Spot or preemptible capacity:** none of the sources found mentions it, but no source was found to rule it out either; search was unavailable.
- **Live-view transport:** undocumented for Browserbase, Hyperbrowser and Airtop.

## 4. How does each connect its open-source library to its cloud, and which components are open or closed?

### Takeaway
**Every vendor uses the same pattern.** An open client or library switches to the cloud through an API key and a base-URL variable or flag. The fleet, scheduler, CDP gateway, stealth/anti-bot, proxies, CAPTCHA solving and billing stay closed. Examples:
- `APIFY_IS_AT_HOME` plus `APIFY_TOKEN` and default storage IDs.
- Stagehand `env: "BROWSERBASE"`.
- browser-use `Browser(use_cloud=True)`.
- the `spider_cloud` Cargo feature.
- `FIRECRAWL_API_URL`.
- `Skyvern(api_key)` vs `Skyvern.local()`.
- `NOTTE_API_URL`.
- Steel's base URL.

**Licences split into two camps:**
- Permissive: Crawlee and Apify SDKs (Apache-2.0), Stagehand (MIT), browser-use (MIT), steel-browser (Apache-2.0), spider (MIT), kernel-images (Apache-2.0), Hypeman (MIT), Scrapy and shub (BSD-3).
- Copyleft or source-available: Firecrawl, Skyvern, Lightpanda and HyperAgent (AGPL-3.0); Browserless (SSPL or commercial); Notte (SSPL).

### Cited Findings

#### Apify (Crawlee, Apify SDK, CLI)
- **Injected environment variables** — [Apify environment variables](https://docs.apify.com/platform/actors/development/programming-interface/environment-variables) (undated):
  - run and build IDs;
  - `ACTOR_DEFAULT_DATASET_ID`, `ACTOR_DEFAULT_KEY_VALUE_STORE_ID`, `ACTOR_DEFAULT_REQUEST_QUEUE_ID`;
  - `ACTOR_MEMORY_MBYTES`, `ACTOR_TIMEOUT_AT`, `ACTOR_MAX_TOTAL_CHARGE_USD`;
  - `ACTOR_EVENTS_WEBSOCKET_URL`, `ACTOR_STANDBY_URL`;
  - `APIFY_TOKEN`, `APIFY_IS_AT_HOME`;
  - the `APIFY_PROXY_*` set.
- **Python SDK:** `SmartApifyStorageClient` uses the cloud API client when `APIFY_IS_AT_HOME` is set and the filesystem client otherwise. `force_cloud=True` uses cloud storage from a local machine with a token — [apify-sdk-python](https://github.com/apify/apify-sdk-python) (`src/apify/storage_clients/_smart_apify/_storage_client.py`, `src/apify/_actor.py`).
- **JS SDK** — [apify-sdk-js](https://github.com/apify/apify-sdk-js) (`src/actor.ts`, `src/platform_event_manager.ts`):
  - `isAtHome()` reads `APIFY_IS_AT_HOME`;
  - locally it uses memory or filesystem storage backends;
  - `open*` calls accept `forceCloud`;
  - backends are wrapped in a `ChargingStorageBackend`;
  - a WebSocket to `ACTOR_EVENTS_WEBSOCKET_URL` delivers `cpuInfo`, `migrating`, `aborting` and `persistState`.
- **Run tokens:** each run gets a token tied to the account, normally scoped to its own input and storages. Full permissions need one-time approval — [Apify platform architecture](https://docs.apify.com/security/architecture).
- **CLI** — [apify-cli](https://github.com/apify/apify-cli) (`src/commands/run.ts`, `src/commands/actors/push.ts`):
  - `apify run` builds the platform environment locally and purges default Crawlee storages unless `--no-purge`;
  - `apify push` uploads source as files or a ZIP, then polls the build.
- **Base images:** Docker Hub `apify/actor-node:24` and `apify/actor-python:3.14` — [Apify Dockerfile docs](https://docs.apify.com/academy/deploying-your-code/docker-file).
- **Open source:**
  - Apache-2.0: crawlee, crawlee-python, apify-sdk-js, apify-sdk-python, apify-cli, the API clients, proxy-chain, fingerprint-suite, impit;
  - MIT: apify-mcp-server.
  - GitHub API, 2026-10-12 — [apify on GitHub](https://github.com/apify).
- **Closed:** the console, API, orchestrator, storage backends, billing and proxy service. The 2020 whitepaper says the platform is developed in-house — [Apify security whitepaper](https://apify.com/security-whitepaper.pdf) (2020; possibly stale) [inferred: no public repos].

#### Zyte (Scrapy, shub)
- **Deploy flow:**
  1. `pip install shub`;
  2. `shub login` with the Scrapy Cloud key, which is separate from the Zyte API key;
  3. `scrapinghub.yml` naming a stack;
  4. `shub deploy <project-id>`.
  - [Zyte Scrapy Cloud tutorial](https://docs.zyte.com/web-scraping/tutorials/main/cloud.html).
- **Open source (BSD-3-Clause):** scrapy, shub, scrapy-zyte-api, python-zyte-api, python-scrapinghub, the stack images and the job-runner entrypoint — [scrapinghub-entrypoint-scrapy](https://github.com/scrapinghub/scrapinghub-entrypoint-scrapy) and sibling repos (GitHub API, 2026-10-12).
- **Closed:** the Zyte API service and unblocking, the browser fleet, Smart Proxy Manager, the Scrapy Cloud runner, scheduler and HubStorage backend, and the extraction models [inferred: no public repos]. Smart Proxy Manager is closed to new customers in favour of Zyte API — [Meet Zyte](https://www.zyte.com/meet-zyte/).

#### Browserbase (Stagehand)
- **Stagehand v3:** dropped Playwright for direct CDP, added modular drivers and automatic action caching. It claims about 44% faster than v2, with larger gains on Browserbase from network-layer optimizations — [Stagehand v3](https://www.browserbase.com/blog/stagehand-v3) (2025-10-29).
- **v3 configuration:** `env: "LOCAL" | "BROWSERBASE"`, with the key from `BROWSERBASE_API_KEY`. `BROWSERBASE_PROJECT_ID` was used in v2 and is now deprecated in the Python SDK — [Stagehand v3 reference](https://docs.stagehand.dev/v3/references/stagehand); [Stagehand v2 browser config](https://docs.stagehand.dev/v2/configuration/browser); [stagehand-python](https://github.com/browserbase/stagehand-python) [search-result excerpts].
- **v4** — [Stagehand v4 browser configuration](https://docs.stagehand.dev/v4/configuration/browser) (undated):
  - you create the browser first (`browserbase.launch`, `localBrowser.launch` or `localBrowser.connect({cdpUrl})`), then `Stagehand.create({ browser })`;
  - Stagehand uploads its own browser extension when no `extensionId` is given, so the runtime lives in the extension;
  - Model Gateway and the managed cache work only on Browserbase.
- **Hosted API base:** `https://api.stagehand.browserbase.com` — [stagehand-net](https://github.com/browserbase/stagehand-net) [search-result excerpt].
- **Open source:** Stagehand (MIT), the Node and Python SDKs and stagehand-cli (Apache-2.0) — [browserbase on GitHub](https://github.com/browserbase).
- **Closed:** gateway, connect service, fleet, Functions runtime, Director, Fetch/Search [inferred: no repos].

#### Browser Use
- **Library (MIT):** `Browser(use_cloud=True)` plus `BROWSER_USE_API_KEY` provisions a cloud browser — [Browser Use: browser infra](https://browser-use.com/posts/browser-infra) (2025-09-30).
  - Settings: `BROWSER_USE_CLOUD_API_URL` (default `https://api.browser-use.com`), `cloud_profile_id`, `cloud_proxy_country_code`, `cloud_timeout` — [browser-use repo](https://github.com/browser-use/browser-use) (`browser_use/config.py`, `examples/browser/cloud_browser.py`).
- **Cloud SDK:** `browser-use-sdk` (Python and TypeScript) targets `/api/v4/browsers`, with the key in `X-Browser-Use-API-Key` — [Browser Use quickstart](https://docs.browser-use.com/cloud/browser/quickstart).
- **Production uses the library:** Browser Use Cloud runs the open-source library on the SQS/Lambda architecture — [Browser Use production architecture](https://browser-use.com/posts/production-architecture-browser-use) (2026-05-09).
- **Closed:** the cloud Chromium fork has proprietary performance and stealth changes — [Browser Use CLOUD.md](https://github.com/browser-use/browser-use/blob/main/CLOUD.md). No public repos exist for the control plane, edge routers or Firecracker tooling [inferred].

#### Steel.dev
- **steel-browser (Apache-2.0):**
  - images `ghcr.io/steel-dev/steel-browser-api` and `-ui`;
  - Render and Railway one-click deploys;
  - SDKs switch between cloud and self-hosted by base URL.
  - [steel-browser repo](https://github.com/steel-dev/steel-browser).
- **Other repos:** Node and Python SDKs are Apache-2.0; Go and Rust SDKs, the CLI and the MCP server are MIT — [steel-dev on GitHub](https://github.com/steel-dev).
- **Closed:** the orchestrator, edge gateway, stealth, CAPTCHA, proxies, Credentials and Files — [steel.dev](https://steel.dev).

#### Browserless
- **Licence:** `SSPL-1.0 OR Browserless Commercial License`. A commercial licence is needed for closed-source commercial products — [browserless repo](https://github.com/browserless/browserless).
- **Images:** `ghcr.io/browserless/chromium`, `/firefox` and `/multi`.
- **Cloud runs the open-source images:** in 2024 the cloud dogfooded them — [Rebuilding Browserless](https://www.browserless.io/blog/rebuilding-browserless) (possibly stale). Private and self-hosted enterprise deployments use the same enterprise image — [Browserless private deployment](https://docs.browserless.io/enterprise/private-deployment/overview.md).
- **Closed:** BrowserQL and Session Replay are premium features [inferred: closed].

#### Kernel
- **kernel-images (Apache-2.0):**
  - builds Chromium images for Docker and Unikraft;
  - exposes CDP on port 9222;
  - clients read `webSocketDebuggerUrl` from `/json/version`, then use Puppeteer `connect` or Playwright `connectOverCDP`.
  - [kernel-images](https://github.com/onkernel/kernel-images).
- **Other repos:** Hypeman (MIT); Node, Python and Go SDKs, CLI and a Terraform provider (Apache-2.0); MCP server (MIT) — [kernel/hypeman](https://github.com/kernel/hypeman) and the kernel org.
- **Protocols:** hosted browsers accept CDP, WebDriver BiDi and computer controls — [Kernel docs](https://www.kernel.sh/docs).
- **Closed:** control plane, scheduler, CDP-holding proxy, stealth, CAPTCHA and managed auth [inferred: no repos].

#### Hyperbrowser, Anchor, Airtop, Notte
- **Hyperbrowser:**
  - HyperAgent is AGPL-3.0 per its LICENSE (GitHub shows NOASSERTION) and can use Hyperbrowser or run locally — [HyperAgent](https://github.com/hyperbrowserai/HyperAgent);
  - SDKs and the MCP server are MIT — [hyperbrowserai on GitHub](https://github.com/hyperbrowserai);
  - integrates with Playwright and Puppeteer over CDP, the Browser Use agent, and MCP — [Hyperbrowser docs](https://hyperbrowser.ai/docs).
- **Anchor:**
  - SDKs and GoVNC are Apache-2.0 — [anchorbrowser/GoVNC](https://github.com/anchorbrowser/GoVNC);
  - integrates with Playwright, Puppeteer, Stagehand, Browser Use and OpenAI/Anthropic computer use — [Anchor docs index](https://docs.anchorbrowser.io/llms.txt).
- **Airtop:** Python SDK MIT; Node SDK with no licence detected; integrates with Puppeteer, Playwright, Selenium, n8n and Make — [airtop-ai on GitHub](https://github.com/airtop-ai); [Airtop docs index](https://docs.airtop.ai/llms.txt).
- **Notte:**
  - `nottelabs/notte` is SSPL v1, with an MIT `node-sdk/` folder;
  - `.env.example` sets `NOTTE_API_URL=https://api.notte.cc` and `NOTTE_API_KEY`;
  - a session can wrap an external `cdp_url` from another provider.
  - [notte repo](https://github.com/nottelabs/notte) (`LICENSE`, `COPYRIGHT.md`, `.env.example`, `types.py`).

#### Skyvern
- **Licence:** AGPL-3.0. Everything is open except the managed cloud's anti-bot measures — [Skyvern repo README](https://github.com/Skyvern-AI/skyvern).
- **Connecting:** `Skyvern(api_key=…)` targets the cloud, `Skyvern.local()` runs locally, and `launch_cloud_browser()` returns a cloud browser.
- **Closed:** the ECS/Temporal session orchestration (per the code comment) and anti-bot.

#### Lightpanda
- **Browser:** AGPL-3.0, written in Zig — [lightpanda-io/browser](https://github.com/lightpanda-io/browser).
- **Connecting:** the same `wss://…?token=` URL works with Puppeteer, Playwright and chromedp — [Lightpanda Cloud getting started](https://lightpanda.io/docs/run-on-lightpanda-cloud/getting-started).
- **Closed:** the console (a Fly.io app) and the cloud control plane [inferred].

#### Firecrawl
- **Licence:** the repo is AGPL-3.0 — [firecrawl repo](https://github.com/firecrawl/firecrawl).
- **Fire-engine is closed** — [Firecrawl self-host docs](https://docs.firecrawl.dev/contributing/self-host) (undated, pins v2.11.162):
  - self-hosters must run it separately;
  - screenshots and page actions require it;
  - Agent, Browser and interact are cloud-only.
- **Other closed services:** Hangar, firebill, a privacy service, an identity multiplexer, a search service and A/B fire-engine endpoints appear only as configured URLs — [config.ts](https://github.com/firecrawl/firecrawl/blob/main/apps/api/src/config.ts) [inferred: closed].
- **SDK connection:** the Rust SDK defaults to `https://api.firecrawl.dev` and offers `Client::new_selfhosted(url, key)` — [rust-sdk client.rs](https://github.com/firecrawl/firecrawl/blob/main/apps/rust-sdk/src/client.rs). Other SDKs use `FIRECRAWL_API_URL` [inferred: code-search hits].
- **Self-host stack:** API and workers, Playwright, Redis, RabbitMQ, NuQ Postgres and optional FoundationDB, with Kubernetes and Helm examples — [SELF_HOST.md](https://github.com/firecrawl/firecrawl/blob/main/SELF_HOST.md).

#### Spider Cloud
- **Crate on-ramp:** the `spider` crate (MIT) has a `spider_cloud` feature. `SpiderCloudConfig::new(key).with_mode(SpiderCloudMode::Smart)` (proxy by default, stealth when blocked) moves crawling to the cloud. The README says the same engine runs Spider Cloud — [spider repo](https://github.com/spider-rs/spider); [configuration.rs](https://github.com/spider-rs/spider/blob/main/spider/src/configuration.rs).
- **Browser access:** `wss://browser.spider.cloud/v1/browser?token=…` via `connectOverCDP`, with MIT `spider-browser` SDKs — [Spider docs overview](https://spider.cloud/docs/overview/); [spider-browser](https://github.com/spider-rs/spider-browser).
- **Other licences:** chromey Apache-2.0 — [chromey](https://github.com/spider-rs/chromey).
- **Closed:** the cloud scheduler, proxy network, anti-bot and Spider's own browser engine [inferred].

#### Cloudflare Browser Run
- **Connection methods:**
  - a Workers binding: `"browser": {"binding": "MYBROWSER"}` in wrangler config — [wrangler reference](https://github.com/cloudflare/cloudflare-docs/blob/production/src/content/docs/browser-run/reference/wrangler.mdx);
  - REST Quick Actions under `/accounts/<id>/browser-run/<action>` — [Quick Actions](https://github.com/cloudflare/cloudflare-docs/tree/production/src/content/docs/browser-run/quick-actions);
  - a raw CDP WebSocket usable from anywhere (since 2026-04-10) — [Browser Run CDP](https://github.com/cloudflare/cloudflare-docs/blob/production/src/content/docs/browser-run/cdp/index.mdx).
- **Open source:** Apache-2.0 forks of Puppeteer, Playwright and Playwright MCP — [cloudflare/puppeteer](https://github.com/cloudflare/puppeteer); [cloudflare/playwright](https://github.com/cloudflare/playwright); [cloudflare/playwright-mcp](https://github.com/cloudflare/playwright-mcp).
- **Closed:** the service and Kitesurf [inferred].

#### Bright Data, ScrapingBee, ScraperAPI, ZenRows
- **Bright Data:**
  - Proxy Manager (`luminati-io/luminati-proxy`) declares ISC in package.json but has no LICENSE file — [luminati-proxy](https://github.com/luminati-io/luminati-proxy);
  - the MCP server, CLI and SDKs are MIT — [brightdata on GitHub](https://github.com/brightdata);
  - super proxies, peer SDK, unblocker and browser fleet are closed [inferred].
- **ZenRows:** official Python, Node.js and Go SDKs, a CLI, and an MCP server at `https://mcp.zenrows.com/mcp` — [ZenRows docs index](https://docs.zenrows.com/llms.txt).
- **ScraperAPI:** SDKs and an MCP server exist; licences were not checked — [ScraperAPI docs](https://docs.scraperapi.com).

### Inferences
- **Goodfetch's on-ramp should be equally thin:**
  - one key plus one base-URL variable;
  - a "storage client" switch triggered by an environment variable the cloud injects (the `APIFY_IS_AT_HOME` pattern);
  - a run-scoped token and default storage IDs.
  Apify's design shows the value of local storage and cloud storage behind one interface, so the same Recipe runs locally and in the cloud unchanged.
- **The closed set is the same everywhere:** fleet, gateway, stealth/anti-bot, proxies, CAPTCHA, billing. That matches Goodfetch Cloud's plan to sell operations rather than withhold engine features.
- **Licensing is a visible differentiator.** The permissive leaders (Crawlee, Stagehand, browser-use, spider, steel-browser) compete with copyleft or source-available rivals (Firecrawl AGPL, Browserless SSPL, Notte SSPL, Skyvern AGPL). Goodfetch's MIT OR Apache-2.0 places it with the permissive group.

### Gaps
- **SDK licences not checked:** ZenRows, ScraperAPI and ScrapingBee.
- **Unclear:** how `ZYTE_API_KEY` is injected into Scrapy Cloud jobs.
- **Not read:** the Stagehand v4 extension runtime's protocol with Browserbase.

## 5. What scale and reliability figures do they publish, and what infrastructure lessons, migrations or cost cuts have they described?

### Takeaway
**Scale figures are company claims and often inconsistent.** Browserbase gives 35M+ or 50M+ sessions a month in different places. Others: Steel 35M+ sessions served; ZenRows 10B+ requests a month; Apify $25M ARR, 15,000+ customers and creators earning over $1.2M a month; Firecrawl an eight-figure ARR in its first year. Few publish uptime percentages, and post-mortems are rare. Apify's are the most detailed, and show its primary database cluster as the recurring point of failure.

**The richest lessons are migrations explained by cost or capability:**
- Firecrawl: Fly.io → Kubernetes on GCP, for autoscaling and after a Redis egress bill.
- Kernel: Docker warm pool → Unikraft snapshots → QEMU-based Hypeman on bare metal, for cost, isolation, GPU and live migration.
- Browser Use browsers: Unikraft → Firecracker on regular EC2, about 3× cheaper per browser-hour.
- Browser Use agents: Lambda → Unikraft → Bedrock AgentCore, for no idle bill.
- Browserbase: single region → multi-region, for CDP round-trip latency.
- Steel: rejected Kubernetes warm pools for cost and boot time.
- Cloudflare: built Kitesurf to cut per-session CPU and memory cost.

### Cited Findings

#### Scale and usage claims
- **Browserbase:**
  - homepage: 35M+ sessions a month, 10,000 customers — [browserbase.com](https://www.browserbase.com);
  - job posts from September 2026: 50M+ sessions a month — [Browserbase Ashby job board](https://api.ashbyhq.com/posting-api/job-board/browserbase) (conflicting with the homepage);
  - customers running 6,000+ concurrent sessions — [What is a Browserbase browser](https://www.browserbase.com/blog/what-is-a-browserbase-browser) (2026-06-16);
  - Series B coverage: 50M sessions in 2025 to date against 25M in 2024 — [Upstarts Media](https://www.upstartsmedia.com/p/browserbase-raises-40m-and-launches-director) (2025) [search excerpt].
- **Steel:** millions of sessions a week, 35M+ sessions served, 1.5M+ browser hours — [steel.dev](https://steel.dev).
- **Apify:**
  - 6,000+ Actors, $25M ARR, 15,000+ customers, about 40,000 active developers — [Cerebral Valley interview](https://cerebralvalley.beehiiv.com/p/apify-is-building-the-infrastructure-for-ai-s-data-problem) (2025-09-16);
  - creators earn more than $1.2M a month — [Apify Billing job post](https://api.ashbyhq.com/posting-api/job-board/apify) (2026-10-01, verified);
  - the jobs page says 85,421 Actors and $1.6M a month for creators — [Apify jobs](https://apify.com/jobs) (conflicting scope or date).
- **Firecrawl:**
  - 300k requests a day — [An adventure in scaling](https://www.firecrawl.dev/blog/an-adventure-in-scaling) (2024-09-13; possibly stale);
  - job ads claim an eight-figure ARR in its first year and a $75M Series B led by Smash Capital — [Firecrawl job ad](https://haystackapp.io/jobs/6690a216-14d3-407a-ba74-4066b1b843a1) (2026-08-20).
- **Browser Use:** millions of untrusted agents in production, and thousands of concurrent sandboxes during spikes — [AgentCore migration](https://browser-use.com/posts/agentcore-migration) (2026-07-12).
- **Kernel:** the blog subtitle speaks of thousands of Chromium browsers — [Kernel blog: scale](https://www.kernel.sh/blog/scale). A $22M round led by Accel, announced 2025-10-09 — [Accel](https://accel.com/news/our-investment-in-kernel-the-best-browser-infrastructure-for-agents) [unverified: search summary].
- **Others:**
  - ZenRows: 10B+ requests a month and 10,000+ companies — [zenrows.com](https://www.zenrows.com/).
  - Bright Data: 400M+ proxy IPs a month in 195 countries, 99.99% network uptime, 50,000+ customers — [Bright Data proxy types](https://brightdata.com/proxy-types) (modified 2026-09-24).
  - Lightpanda: billions of pages a day across users, some above 30M a day each — [Lightpanda 1.0](https://lightpanda.io/blog/posts/lightpanda-1-0) (2026-10-02).
  - Spider: 100K+ pages per second (vendor claim) — [spider.cloud](https://spider.cloud/).
  - Zyte: 3,000+ companies and 250+ staff in a job post; Built In shows 219 employees (conflict) — [Zyte job post (Built In)](https://builtin.com/job/platform-engineering-team-lead-remote/7827909).

#### Reliability: status pages and incidents
- **Apify:**
  - Postmortem 2025-02-22, full outage from 22:00 to 23:00 UTC — [Apify postmortem](https://status.apify.com/incidents/4k6n5swkwx5h) (published 2025-02-26, verified):
    - signup growth plus a misconfigured query loaded a poor index, and the memory spike crashed the database cluster;
    - running Actors were paused and resumed, with no data loss;
    - fixes: re-architect the primary database clusters into smaller ones, tune indexes, add metrics.
  - Later incidents:
    - 2026-04-08: a sharding procedure slowed then partly stopped all services — [incident](https://status.apify.com/incidents/cblcl48ry0jc).
    - 2026-04-29: full Actors/API outage, about 12:35–15:13 UTC — [incident](https://status.apify.com/incidents/5sykk7dm1935).
    - 2026-05-15: in-memory cache problems — [incident](https://status.apify.com/incidents/rjv0tgcqqvr9).
    - 2026-07-21: the database cluster degraded into a major outage; capacity was capped and runs held — [incident](https://status.apify.com/incidents/fqkp6kdb0wy2).
    - 2026-03-06: an employee's GitHub account was compromised and force-pushed commits; branch protection kept the default branches clean — [incident](https://status.apify.com/incidents/k950nq1k7lgl).
  - No uptime percentages are shown — [Apify status](https://status.apify.com).
- **Zyte incidents:**
  - an upstream provider's networking problem disrupted `app.zyte.com` (2026-06-27) — [incident](https://status.zyte.com/incidents/8rsr93qgzb2x);
  - a provider router outage degraded Scrapy Cloud — [incident](https://status.zyte.com/incidents/qb2g7lfxsz78);
  - Browser Rendering had high latency and 520 errors (2026-09-10) — [incident](https://status.zyte.com/incidents/2jk79t1ff4s4);
  - the AWS DynamoDB DNS event (2025-10-20) — [incident](https://status.zyte.com/incidents/g9ry3nzrspfl).
- **Steel:** Main API at 99.764% (window not stated), with 10 incidents from 2026-07-20 to 2026-10-01 — [Steel status](https://status.steel.dev). Examples:
  - capacity and session-creation errors, 5 h 41 min (2026-08-23);
  - streaming and recording issues, 6 h 50 min (2026-09-25);
  - domain/SSL outages (2026-07-21/22).
- **Browserless:** regional components at 99.997–100%; Account API 99.827% — [Browserless status](https://status.browserless.io).
- **Skyvern:** runs using vault credentials failed on 2026-10-09/10 because a credential-storage secret had expired — [Skyvern status](https://status.skyvern.com).
- **Notte:** Session API 99.977%, Scrape API 99.726%; a Scrape API and Proxies outage on 2026-10-05 — [Notte status](https://status.notte.cc).
- **ScrapingBee 90-day uptime:** HTML 99.993%, with rendering 99.941%, Walmart API 93.298% — [ScrapingBee status](https://status.scrapingbee.com/).
- **ZenRows:** the status page shows Fetch at 99.998% while the homepage claims 100% — [ZenRows status](https://status.zenrows.com/).
- **Pages with no incidents or percentages shown:**
  - Browserbase (incident.io): components API, Connect, Dashboard, Director, Stagehand API — [Browserbase status](https://status.browserbase.com);
  - Kernel — [Kernel status](https://status.kernel.sh/);
  - Anchor — [Anchor status](https://status.anchorbrowser.io/);
  - Airtop — [Airtop status](https://status.airtop.ai/);
  - Browser Use — [Browser Use status](https://status.browser-use.com).
- **Bright Data:** no public status page found; `status.brightdata.com` has no DNS record and the trust center links none — [Bright Data trust center](https://brightdata.com/trustcenter). It runs annual disaster-recovery drills and an RCA for every high-severity incident — [Bright Data security overview](https://docs.brightdata.com/general/security/security-overview).

#### Infrastructure lessons, migrations and cost cuts
- **Firecrawl** — [An adventure in scaling](https://www.firecrawl.dev/blog/an-adventure-in-scaling) (2024-09-13; possibly stale; verified):
  - Platform and costs:
    - a Railway bill of about $15k, mostly Redis egress, led them to self-host Redis on Fly; Upstash and Dragonfly were tried and dropped;
    - Fly lacked smooth autoscaling, so they moved toward Kubernetes.
  - Queue and worker problems:
    - a queue lock duration of 2 h was cut to 1 min;
    - memory leaks forced redeploys every 2 h;
    - Fly sometimes hard-killed machines before the shutdown timeout;
    - ioredis needed `family=6` (IPv6) on Fly;
    - BullMQ trimmed its event stream, so they switched to polling.
  - A 2026 job goal is to reduce infrastructure cost per request — [Firecrawl job ad](https://haystackapp.io/jobs/6690a216-14d3-407a-ba74-4066b1b843a1).
- **Kernel** — [Kernel blog: scale](https://www.kernel.sh/blog/scale) (verified):
  - the Docker warm pool worked with fewer than ten API users;
  - after that, hundreds of idle hot containers and multi-day sessions made it expensive, and container escapes risked every tenant;
  - Unikraft could not live-migrate or pass through GPUs;
  - bare metal is meant to cut cloud spend sharply, which the CTO links to his AWS cost experience at a previous company.
  - Unikraft's story says snapshots took idle compute cost to zero (disk only), making a low hobby tier viable — [Unikraft customer story](https://unikraft.com/customer-stories/scaling-browser-infrastructure).
  - In 2025 the team saw fewer bot-detection hits from Unikraft Cloud IPs — [HN launch thread](https://news.ycombinator.com/item?id=43705144).
- **Browser Use:**
  - The move from Unikraft to Firecracker on regular EC2 cut cost from $0.06 to $0.02 per browser-hour. Regular EC2 is faster to obtain and cheaper to keep idle — [Firecracker browser infra](https://browser-use.com/posts/firecracker-browser-infra) (2026-06-15).
  - AgentCore was chosen for pay-per-use with no idle bill. A fixed fleet either wastes money at average load or drops traffic at peaks — [AgentCore migration](https://browser-use.com/posts/agentcore-migration).
  - Running agent loops inside the API process meant redeploys killed agents and memory-hungry agents slowed the API — [Two ways to sandbox agents](https://browser-use.com/posts/two-ways-to-sandbox-agents).
- **Browserbase:** multi-region took about a month and cut network latency by 87–98%. Bigger browsers and removing virtualization layers had not helped — [Becoming multi-region](https://www.browserbase.com/blog/becoming-multi-region) (2024-12-06; possibly stale).
- **Steel:**
  - Kubernetes warm instances were rejected for cost and boot time, in favour of Firecracker — [HN: Steel launch](https://news.ycombinator.com/item?id=42245573) (2024-12-01; possibly stale).
  - Regions went from `lax`/`iad` to `us-east` only between March and October 2026 — [Steel multi-region docs](https://docs.steel.dev/overview/sessions-api/multi-region) [inferred: consolidation; reason not published].
- **Browserless:**
  - the rebuild took about 400 developer-days over 12 months with eight people;
  - dropping Selenium cut maintenance;
  - load balancers moved from NGINX+Lua to a TypeScript router.
  - [Rebuilding Browserless](https://www.browserless.io/blog/rebuilding-browserless) (2024-02-21; possibly stale).
- **Cloudflare:**
  - Kitesurf's lower memory and CPU are meant to lower costs for Cloudflare and its customers — [Kitesurf](https://blog.cloudflare.com/kitesurf) (2026-08-06);
  - observability was added because customers could not diagnose failed automations — [Browser Run for AI agents](https://blog.cloudflare.com/browser-run-for-ai-agents) (2026-04-15).
- **Notte:** moved its Anything API agent loop out of the sandbox into the backend, keeping sandboxes for tools only — [Notte sandbox post](https://www.notte.cc/blog/why-we-moved-the-agent-out-of-the-sandbox) (2026-09-09).
- **Zyte:** moved from self-run Kafka to Confluent Cloud to reduce operations work — [Confluent blog](https://www.confluent.io/blog/why-scrapinghub-chose-confluent-cloud-kafka-service/) (2019; possibly stale).
- **Apify:** the CEO names running other people's code (millions of files or sockets) as the hardest operational problem — [Cerebral Valley interview](https://cerebralvalley.beehiiv.com/p/apify-is-building-the-infrastructure-for-ai-s-data-problem) (2025-09-16).

### Inferences
- **Idle capacity is the dominant cost driver in browser fleets.** Every migration story targets it: Kernel snapshots to zero idle compute, Browser Use cut its idle host fleet, Steel avoided warm pools, AgentCore has no idle bill. Goodfetch's ability to checkpoint, abort and resume runs from portable state is a lever on exactly that cost.
- **The control-plane database is the most common single point of failure.** That applies to multi-tenant actor platforms (Apify, three platform-wide incidents tied to its primary cluster in 2025–2026) more than to compute. Goodfetch Cloud's multi-tenant console and marketplace should shard or partition tenant metadata from the start.
- **Fly.io and Railway serve early-stage hosting; scale-ups leave for Kubernetes on a hyperscaler or for bare metal**, citing autoscaling and egress costs (Firecrawl). Lightpanda keeps a Fly.io console with Hetzner/AWS data planes; Browser Use's connect host on Railway is an inference.
- **Multi-region is justified for hosted browsers driven over CDP,** where round trips stack. It matters less for batch crawl APIs; Firecrawl and Spider publish no compute regions.

### Gaps
- **Not found:** post-mortems for Browserbase, Kernel, Browser Use, Firecrawl, Hyperbrowser, Anchor and Airtop. Search was unavailable to look for incident write-ups beyond the status pages.
- **Unclear time windows:** several status pages show uptime percentages without stating the window.
- **Not searched:** conference talks (KubeCon, re:Invent, Unikraft or Firecracker) and podcasts, because the search budget ran out.

## 6. What have they said about their metering and billing infrastructure?

### Takeaway
**Few name their billing vendor.** Where they do:
- Firecrawl: Autumn for entitlements and credits, behind an in-house billing gate ("firebill") with idempotency keys and Redis-batched usage, plus Stripe (inferred).
- Zyte and ScrapingBee: Chargebee for subscriptions with Stripe for payments (Zyte also uses Braintree and Xero).
- Apify: an in-house Billing team owning subscriptions, invoices, payments and creator payouts, with Stripe as a dependency.
- Browser Use and Lightpanda: Stripe for invoices and the customer portal, with in-house per-minute or per-second metering.
- Cloudflare: its own billing.

No source found names Orb, Metronome or Lago for these companies, but those vendors' customer pages could not be searched.

**Metering units are vendor-specific:**
- Apify: compute units (GB-hours, metered per second) plus pay-per-event via `Actor.charge`, with idempotency and a run spending cap.
- Cloudflare: browser hours plus monthly-average concurrency, with an `X-Browser-Ms-Used` response header.
- Bright Data: traffic only, for its Browser API.
- ZenRows: minutes plus GB.
- Airtop: 30-second increments and credits.
- Kernel: no usage in standby.

### Cited Findings
- **Firecrawl:**
  - **Autumn** (`autumn-js`) handles features TEAM, CREDITS, SEARCH_CREDITS, CONCURRENCY and `rate_limits`, and its balances also drive rate limits and concurrency — [autumn.service.ts](https://github.com/firecrawl/firecrawl/blob/main/apps/api/src/services/autumn/autumn.service.ts) (verified 2026-10-12).
  - **firebill**, an in-house service in front of Autumn:
    - its `/v1/lock` call answers locked, denied (`out_of_credits`, `job_revoked`, `gate_unavailable`) or unavailable;
    - retries are safe because idempotency keys are stable, so Autumn de-duplicates;
    - a 5 s client timeout.
    - [firebill.ts](https://github.com/firecrawl/firecrawl/blob/main/apps/api/src/services/autumn/firebill.ts) (verified).
  - **Batching:** usage goes onto a Redis list under a lock and is flushed 5,000 at a time every 15 s. A request-credits table lives in Bigtable — [batch_billing.ts](https://github.com/firecrawl/firecrawl/blob/main/apps/api/src/services/billing/batch_billing.ts); [credit_billing.ts](https://github.com/firecrawl/firecrawl/blob/main/apps/api/src/services/billing/credit_billing.ts).
  - **Stripe:** `STRIPE_SECRET_KEY` is in config — [config.ts](https://github.com/firecrawl/firecrawl/blob/main/apps/api/src/config.ts) [inferred: Stripe handles payments].
- **Apify:**
  - **In-house billing:** the Billing team owns the billing system (subscriptions, invoices, payments) and the creator payout infrastructure — [Apify Billing job post](https://api.ashbyhq.com/posting-api/job-board/apify) (2026-10-01, verified).
  - **Stripe** is a status-page dependency — [Apify status](https://status.apify.com).
  - **Compute units:** 1 CU = 1 GB of memory for 1 hour, metered per second — [Apify usage and resources](https://docs.apify.com/platform/actors/running/usage-and-resources).
  - **Pay-per-event** — [Apify pay-per-event](https://docs.apify.com/actors/publishing/monetize/pay-per-event.md) (undated):
    - code calls `Actor.charge({eventName})` or passes an event to `pushData`, or uses the CLI or API;
    - the platform enforces `ACTOR_MAX_TOTAL_CHARGE_USD` and aborts the run at the cap;
    - a graceful abort sends `aborting` and force-stops the run 30 s later;
    - creator payout is 80% of revenue minus platform costs.
  - **SDK charging code** — [apify-sdk-js](https://github.com/apify/apify-sdk-js) (`src/charging.ts`):
    - calls the run "charge" endpoint with an idempotency key;
    - synthetic events (`apify-actor-start`, `apify-default-dataset-item`) are charged by the platform;
    - local tests write a `charging_log` dataset.
  - **Agentic payments:** x402, MPP and Skyfire — [Apify monetization](https://docs.apify.com/platform/actors/publishing/monetize).
- **Zyte:**
  - **Billing subprocessors:** Chargebee (subscriptions), Stripe and Braintree (payments), Xero (invoicing), Breadwinner (financial reporting) — [Zyte subprocessors](https://www.zyte.com/terms-policies/subprocessor/) (verified).
  - **Billing upgrade:** 2026-04-13/14 — [Zyte status incident](https://status.zyte.com/incidents/c906r79mw4lj).
  - **CDP sessions** are billed by session time in "browser units" — [Zyte CDP support](https://www.zyte.com/blog/introducing-zyte-cdp-support/) (2026-09-01).
- **ScrapingBee:** Chargebee for subscriptions and billing, Stripe for payments — [ScrapingBee DPA](https://www.scrapingbee.com/data-processing-agreement/) (2026-03-23, verified).
- **Browser Use:**
  - **Stripe and credits:** invoices and history are in Stripe; usage is prepaid credits per project, with browser time per minute (rounded up), network per MB/GB and auto top-up — [Browser Use billing](https://docs.browser-use.com/cloud/guides/billing.md);
  - **spend caps:** a monthly soft cap per API key returns HTTP 402 `api_key_monthly_spend_limit_reached`;
  - **in-house metering:** the agent control plane enforces LLM cost caps and handles billing — [Two ways to sandbox agents](https://browser-use.com/posts/two-ways-to-sandbox-agents) [inferred: in-house metering];
  - **conflict:** CLOUD.md says browser time is charged upfront and refunded on stop, against per-minute billing in the billing docs — [CLOUD.md](https://github.com/browser-use/browser-use/blob/main/CLOUD.md).
- **Lightpanda:** browser hours metered per second and summed per billing period. Upgrades, invoices and cancellation go through the Stripe customer portal — [Lightpanda limits and billing](https://lightpanda.io/docs/run-on-lightpanda-cloud/limits-and-billing).
- **Cloudflare:**
  - billed through Workers plans;
  - Quick Actions by browser hours; sessions by browser hours plus monthly-average concurrent browsers;
  - every Quick Action response carries an `X-Browser-Ms-Used` header (since 2025-09-02);
  - requests failing with `waitForTimeout` are not charged.
  - [Browser Run pricing doc](https://github.com/cloudflare/cloudflare-docs/blob/production/src/content/docs/browser-run/pricing.mdx) (metering model only).
- **Bright Data:** the Browser API is metered on traffic only. Adding custom headers or cookies switches billing from successful requests only to all requests — [opencode-brightdata reference](https://cdn.jsdelivr.net/npm/@brightdata/opencode-brightdata@1.0.0/skills/bright-data-best-practices/references/browser-api.md). No billing vendor is disclosed.
- **ZenRows:** Browser Sessions are metered in minutes plus GB — [ZenRows Browser Sessions](https://www.zenrows.com/products/browser-sessions). Stripe appears in the privacy policy — [ZenRows privacy](https://www.zenrows.com/legal/privacy) (Sep 2024; possibly stale).
- **Airtop:** credits cover AI calls, browser CPU time and proxy bandwidth, with LLM tokens converted to credits — [Airtop FAQ](https://docs.airtop.ai/guides/misc/faq.md). Sessions are billed in 30-second increments — [Airtop creating a session](https://docs.airtop.ai/guides/how-to/creating-a-session.md).
- **Kernel:** standby accrues no usage — [Kernel standby](https://kernel.sh/docs/browsers/standby). A spending-caps page is indexed — [Kernel docs index](https://kernel.sh/docs/llms.txt). No billing vendor was found.
- **Browserbase:** every session is metered for at least one minute — [Browserbase concurrency](https://docs.browserbase.com/optimizations/concurrency/overview.md). A dashboard-engineer posting covers authentication and billing in a Next.js/Node/Postgres dashboard — [Browserbase Ashby job board](https://api.ashbyhq.com/posting-api/job-board/browserbase) (2026-07-02) [inferred: in-house billing UI].
- **Steel:** usage draws on a prepaid credit balance with top-ups and auto top-up — [Steel limits](https://docs.steel.dev/overview/pricinglimits).
- **Separate billing services on status pages:**
  - Browserless lists a "Unit and Usage Services" component — [Browserless status](https://status.browserless.io) [inferred: a dedicated metering service].
  - ScraperAPI lists a "Billing & Subscriptions" microservice — [ScraperAPI status](https://status.scraperapi.com/).

### Inferences
- **The Goodfetch split is the common pattern:** the engine counts usage; the cloud prices it.
  - Firecrawl's firebill gates work before it starts (lock, then confirm or deny) and batches usage events to Autumn with idempotency keys.
  - Apify's SDK reports chargeable events with idempotency keys, and the platform enforces a run-level spending cap.
  - Cloudflare returns measured browser milliseconds on every response.
  - All three need a usage stream that tolerates duplicates. Goodfetch's append-only per-run usage records fit them.
- **Reserve before running:** firebill's lock call, Apify's `ACTOR_MAX_TOTAL_CHARGE_USD` and Browser Use's 402 spend cap all show admission control happening before work starts, not only billing afterwards.
- **Marketplace payouts get their own team:** Apify shows that creator payouts (80% of revenue minus platform costs) need dedicated in-house infrastructure beyond a usage-billing vendor.

### Gaps
- **Not searched (budget):** Orb, Metronome, Lago and Autumn customer lists and case studies. Absence of those vendors here is not evidence they are unused.
- **Billing vendor unknown:** Browserbase (a `stripe-verification` DNS TXT record only weakly suggests Stripe), Steel, Kernel, Hyperbrowser, Anchor, Airtop, Spider, Notte, Skyvern (no billing integration in its open-source code), Bright Data and ScraperAPI.
