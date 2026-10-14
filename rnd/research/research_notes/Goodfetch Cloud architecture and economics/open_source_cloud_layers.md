# Open-source cloud and platform layers around frameworks, and the building blocks a Goodfetch Cloud on AWS could reuse (as of 2026-10-12)

How the numbers were gathered: stars, detected SPDX license, archive status, latest GitHub release, newest tag, commits on the default branch since 2026-07-14 (about 90 days) and open issues come from the GitHub GraphQL and REST APIs, queried on 2026-10-12. "No release" means the repository has no GitHub release object (tags are given instead when they exist). Where GitHub reports `NOASSERTION` or no license, the LICENSE file itself was read. Security advisories are the repository-level GitHub Security Advisories (GHSA) published by each project. Halfway through the research the session's shared web-search budget was used up, so later checks used the GitHub API, GitHub topic listings and direct fetches of primary documentation only; claims that could not be checked that way are listed under Gaps.

## 1. Which open-source projects host or orchestrate a scraping or browser framework as a service?

### Takeaway
Where the open "platform around a framework" layer is substantive, it is mostly copyleft or source-available: Firecrawl, Skyvern and Browsertrix are AGPL-3.0, Lightpanda is AGPL-3.0-only, and Browserless and Notte are SSPL-1.0. These servers also hold back the hard anti-bot layer for their clouds: Firecrawl's Fire-engine and Skyvern's anti-bot measures are not in the open code. The permissively licensed projects are building blocks: Kernel's browser images and its VM runtime hypeman, Steel Browser, neko, Selenium Grid with KEDA, Apify's proxy, fingerprint and impersonation libraries, and Apify's Actor whitepaper. The Scrapy-era managers (Crawlab, Gerapy, ScrapydWeb, SpiderKeeper, Splash) are dormant, single-tenant admin panels. No maintained, permissively licensed, multi-tenant actor platform with a marketplace exists in the open.

### Cited Findings

#### Catalogue snapshot (GitHub API, checked 2026-10-12)

| Project (source) | Role | License as found | Stars | Latest release (date) | Commits since 2026-07-14 | Status |
|---|---|---|---|---|---|---|
| [firecrawl/firecrawl](https://github.com/firecrawl/firecrawl) | Crawl and extraction API | AGPL-3.0 | 190,547 | v2.11.0 (2026-06-19); tags to v2.11.506 (2026-10-11) | 607 | Very active |
| [unclecode/crawl4ai](https://github.com/unclecode/crawl4ai) | Crawler library with a Docker API server | Apache-2.0 plus an added attribution requirement | 85,263 | v0.9.4 (2026-09-23) | 124 | Active |
| [jina-ai/reader](https://github.com/jina-ai/reader) | URL-to-LLM-text API (r.jina.ai) | Apache-2.0 | 12,135 | no release | 0 (last push 2026-05-22) | Inactive |
| [webrecorder/browsertrix](https://github.com/webrecorder/browsertrix) | Hosted browser-based crawling service (Kubernetes) | AGPL-3.0 | 484 | v1.26.1 (2026-10-06) | 131 | Active |
| [webrecorder/browsertrix-crawler](https://github.com/webrecorder/browsertrix-crawler) | The crawler that Browsertrix runs | AGPL-3.0 | 1,159 | v1.15.0 (2026-10-02) | 70 | Active |
| [dgtlmoon/changedetection.io](https://github.com/dgtlmoon/changedetection.io) | Change monitoring app plus hosted service | Apache-2.0 | 34,908 | 0.60.8 (2026-09-28) | 189 | Active |
| [dgtlmoon/sockpuppetbrowser](https://github.com/dgtlmoon/sockpuppetbrowser) | Chrome CDP connection server | Apache-2.0 | 145 | 0.0.3 (2025-08-20) | 30 | Small |
| [crawlab-team/crawlab](https://github.com/crawlab-team/crawlab) | Language-agnostic spider management platform | BSD-3-Clause | 12,280 | v0.6.3 (2023-07-26); v0.6.3-dev pre-release (2024-06-14) | 0 | Dormant |
| [scrapy/scrapyd](https://github.com/scrapy/scrapyd) | Daemon that runs Scrapy spiders over an HTTP API | BSD-3-Clause | 3,098 | tag 1.6.0 (2025-07-22) | 11 | Maintained |
| [my8100/scrapydweb](https://github.com/my8100/scrapydweb) | Web UI for Scrapyd clusters | GPL-3.0 | 3,412 | v1.6.0 (2025-02-16) | 0 | Dormant |
| [Gerapy/Gerapy](https://github.com/Gerapy/Gerapy) | Scrapy and Scrapyd manager (Django and Vue) | MIT | 3,513 | v0.9.13 (2023-07-19) | 0 | Dormant |
| [DormyMo/SpiderKeeper](https://github.com/DormyMo/SpiderKeeper) | Admin UI for Scrapy | MIT (in setup.py; no LICENSE file) | 2,764 | no release | 0 (last push 2023-05-04) | Dead |
| [bitmakerla/estela](https://github.com/bitmakerla/estela) | Elastic scraping cluster on Kubernetes | MIT | 202 | no release | 7 | Tiny |
| [q-m/scrapyd-k8s](https://github.com/q-m/scrapyd-k8s) | Scrapyd API on container infrastructure | MIT | 16 | v0.9.0 pre-release (2026-05-29) | 0 | Tiny |
| [istresearch/scrapy-cluster](https://github.com/istresearch/scrapy-cluster) | Distributed Scrapy on Redis and Kafka | MIT | 1,222 | v1.2.1 (2018) | 0 | Archived |
| [scrapinghub/splash](https://github.com/scrapinghub/splash) | Browser as a service with an HTTP API | BSD-3-Clause | 4,188 | tag 3.5.0 (2020-06-16) | 0 (last push 2024-08-02) | Dormant |
| [browserless/browserless](https://github.com/browserless/browserless) | Headless browsers in Docker | SSPL-1.0 OR a Browserless commercial license | 13,806 | v2.57.0 (2026-09-30) | 95 | Active |
| [steel-dev/steel-browser](https://github.com/steel-dev/steel-browser) | Browser API for agents | Apache-2.0 | 7,786 | v0.5.4-beta (2026-08-25) | 10 | Public beta, slowing |
| [kernel/kernel-images](https://github.com/kernel/kernel-images) (was onkernel/kernel-images) | Browser images for Docker or Unikraft unikernels | Apache-2.0 | 1,099 | no release | 84 | Active |
| [kernel/hypeman](https://github.com/kernel/hypeman) | Multi-hypervisor VM runtime for OCI images | MIT | 370 | n/a | n/a (pushed 2026-10-12) | Active |
| [lightpanda-io/browser](https://github.com/lightpanda-io/browser) | New headless browser in Zig, with a CDP server | AGPL-3.0-only | 36,286 | 1.0.0 (2026-10-02) | 2,506 | Very active |
| [nottelabs/notte](https://github.com/nottelabs/notte) | Cloud browser and agent platform | SSPL-1.0 | 2,023 | v1.9.7 (2026-10-08) | 216 | Active |
| [Skyvern-AI/skyvern](https://github.com/Skyvern-AI/skyvern) | AI browser workflow automation, self-hostable | AGPL-3.0 | 23,182 | v1.0.55 (2026-10-01) | 1,153 | Very active |
| [browser-use/browser-use](https://github.com/browser-use/browser-use) | Browser agent library | MIT | 117,662 | 0.13.11 (2026-10-07) | 479 | Very active |
| [browser-use/web-ui](https://github.com/browser-use/web-ui) | Web UI to run the agent ("Run AI Agent in your browser") | MIT | 16,611 | v3.0.0 (2025-08-31) | 0 | Inactive, 271 open issues |
| [browserbase/stagehand](https://github.com/browserbase/stagehand) | Agent and browser SDK | MIT | 25,629 | @browserbasehq/stagehand 3.7.3 (2026-08-28) | 198 | Active |
| [browserbase/mcp-server-browserbase](https://github.com/browserbase/mcp-server-browserbase) | Self-hostable MCP server | Apache-2.0 | 3,403 | v3.0.0 (2026-03-31) | 1 | Archived |
| [browserbase/open-operator](https://github.com/browserbase/open-operator) | Web-agent template | MIT per README; no LICENSE file | 1,950 | no release | 0 | Archived |
| [hyperbrowserai/HyperAgent](https://github.com/hyperbrowserai/HyperAgent) | AI browser automation | AGPL-3.0-or-later (GitHub reports NOASSERTION) | 1,595 | v1.0.0 (2025-11-19) | 0 | Inactive |
| [SeleniumHQ/selenium](https://github.com/SeleniumHQ/selenium) | Selenium, including Grid | Apache-2.0 | 34,532 | selenium-4.51.0 (2026-10-09) | 472 | Very active |
| [SeleniumHQ/docker-selenium](https://github.com/SeleniumHQ/docker-selenium) | Grid containers and Helm chart | Apache-2.0 per its LICENSE.md text (GitHub reports NOASSERTION) | 8,657 | 4.49.0-20260909 (2026-09-17) | 57 | Active |
| [aerokube/selenoid](https://github.com/aerokube/selenoid) | Selenium hub running browsers in containers | Apache-2.0 | 2,654 | 1.11.3 (2024-05-25) | 0 | Archived |
| [zalando/zalenium](https://github.com/zalando/zalenium) | Container-based Selenium Grid | n/a | 2,361 | 3.141.59z (2020) | 0 | Archived |
| [m1k1o/neko](https://github.com/m1k1o/neko) | Virtual browser in Docker over WebRTC | Apache-2.0 | 22,477 | v3.1.6 (2026-09-30) | 52 | Active |
| [agent-infra/sandbox](https://github.com/agent-infra/sandbox) | All-in-one agent sandbox (browser, shell, files, MCP, VS Code) | Apache-2.0 | 6,140 | v1.11.0 (2026-06-23) | 7 | Slowing |
| [trycua/cua](https://github.com/trycua/cua) | Computer-use agent infrastructure | MIT, with FSL-1.1-MIT parts | 29,395 | cua-sdk-v0.4.1 (2026-10-05) | 1,614 | Very active |
| [e2b-dev/desktop](https://github.com/e2b-dev/desktop) | Desktop sandbox for LLMs | Apache-2.0 | 1,523 | python 2.4.2 (2026-07-23) | 32 | Active |
| [bytebot-ai/bytebot](https://github.com/bytebot-ai/bytebot) | Self-hosted desktop agent | Apache-2.0 | 11,077 | no release | 0 | Archived (last push 2025-09-12) |
| [ulixee/platform](https://github.com/ulixee/platform) | Hero scraping engine with Datastore, Cloud and Stream | MIT | 56 (hero: 1,560) | v2.0.0-alpha.34 (2025-09-08) | 0 | Inactive |
| [scrapoxy/scrapoxy](https://github.com/scrapoxy/scrapoxy) | Proxy aggregator | NOASSERTION | 2,408 | tag 4.22.1 (2025-06-05) | 0 | "Scrapoxy has been discontinued." |
| [apache/stormcrawler](https://github.com/apache/stormcrawler) | Scalable crawler on Apache Storm | Apache-2.0 | 1,007 | 3.7.0 (2026-07-29) | 168 | Active |
| [apache/nutch](https://github.com/apache/nutch) | Extensible web crawler | Apache-2.0 | 3,298 | tag release-1.23 (2026-08-07) | 35 | Active |
| [spider-rs/spider](https://github.com/spider-rs/spider) | Rust crawler | MIT | 2,774 | v2.52.2 (2026-03-31) | 39 | Active |
| [spider-rs/headless-browser](https://github.com/spider-rs/headless-browser) | Chrome setup for cloud providers | MIT | 86 | no release | 0 (last push 2026-02-09) | Inactive |

#### Crawl and extraction APIs
- Firecrawl's `apps/` folder holds `api`, `nuq-postgres`, `playwright-service-ts`, `go-html-to-md-service`, `redis` and `siem`, plus SDKs for many languages — [firecrawl/apps](https://github.com/firecrawl/firecrawl/tree/main/apps).
- Firecrawl's self-host guide sets the default queue to "Queue: NuQ PostgreSQL". It says that "At this revision, Compose runs the Firecrawl API and workers, Playwright, Redis, RabbitMQ, NuQ PostgreSQL, and FoundationDB services for the optional queue", and that the "Compose file is a source-aligned starting point, not a production architecture" — [SELF_HOST.md](https://github.com/firecrawl/firecrawl/blob/HEAD/SELF_HOST.md).
- Firecrawl's public self-host docs list what the open stack lacks:
  - Fire-engine: "Run and configure that service separately; it is not included."
  - Screenshots and page actions: "Not available in the default stack." Both need Fire-engine.
  - Agent, Browser, interact and the specialised formats are cloud-only.
  - Production cautions: "This trusted-network quickstart disables API authentication and is not a production architecture", and "No single .env switch makes the stack production-ready". The commands are pinned to v2.11.162.
  — [Firecrawl self-host docs](https://docs.firecrawl.dev/contributing/self-host)
- The Firecrawl org also publishes MIT-licensed pieces: firecrawl-mcp-server (7,583 stars), pdf-inspector (a Rust PDF library, 19,613 stars) and anydoc (22,732 stars) — [github.com/firecrawl](https://github.com/firecrawl).
- Crawl4AI's LICENSE is Apache-2.0 text followed by an "Attribution Requirement": "All distributions, publications, or public uses of this software, or derivative works based on this software, must include the following attribution" — [crawl4ai LICENSE](https://github.com/unclecode/crawl4ai/blob/HEAD/LICENSE). ADR 0018 already records that no Crawl4AI code is copied into Goodfetch for this reason — [ADR 0018](/Users/jb/Dev/automan/docs/adr/0018-permissive-license-private-components-only-for-legal-exposure.md).
- Crawl4AI's Docker server:
  - Serves on port 11235, with a `/playground` UI, MCP support, JWT authentication and "Asynchronous Jobs with Webhooks".
  - Images are multi-arch (amd64 and arm64).
  - Redis is "managed by supervisord" inside the container.
  - Defaults: rate-limit storage is `memory://`, and `trusted_hosts: ["*"]` comes with the advice to "use specific domains in production" and to "Always enable security in production".
  — [deploy/docker/README.md](https://github.com/unclecode/crawl4ai/blob/HEAD/deploy/docker/README.md)
- Jina Reader:
  - "This repository is the open source branch of the codebase behind https://r.jina.ai and https://s.jina.ai. It runs in stateless or bucket-cached mode; the MongoDB-backed SaaS storage layer is not included here."
  - In 2025-03 "Reader is no longer a Firebase application. The SaaS migrated off Firestore + Cloud Functions to a Cloud Run image with MongoDB Atlas".
  — [jina-ai/reader README](https://github.com/jina-ai/reader)
- Browsertrix:
  - Describes itself as "a cloud-native, high-fidelity, browser-based crawling service".
  - "Install Browsertrix to self host, or access hosted Browsertrix from app.browsertrix.com". It deploys "using Kubernetes, from a single-node setup to scalable clusters in the cloud".
  — [webrecorder/browsertrix README](https://github.com/webrecorder/browsertrix)
  - Its Helm chart templates include backend, frontend, mongo, minio, operators, signer, networkpolicies and priorities — [chart/templates](https://github.com/webrecorder/browsertrix/tree/main/chart/templates).
- sockpuppetbrowser (changedetection.io's Chrome server) has admission-control settings:
  - `DROP_EXCESS_CONNECTIONS`: at capacity, refuse new connections or queue them.
  - `CONNECTION_QUEUE_TIMEOUT`: 120 seconds.
  - `WS_MAX_QUEUE`: a per-connection receive queue of 128 messages, for backpressure.
  — [sockpuppetbrowser README](https://github.com/dgtlmoon/sockpuppetbrowser)

#### Scrapy-era and generic spider managers
- Crawlab's architecture has "a master node, worker nodes, SeaweedFS (a distributed file system) and MongoDB database". Master and workers talk over gRPC. The "Task queue is also stored in MongoDB", and the master "uploads (deploys) spiders to the distributed file system SeaweedFS, for synchronization by worker nodes" — [crawlab README](https://github.com/crawlab-team/crawlab).
- Scrapyd's HTTP server uses basic authentication only when configured: "Enable basic authentication by setting this and password to non-empty values" — [scrapyd docs/config.rst](https://github.com/scrapy/scrapyd/blob/HEAD/docs/config.rst).
- estela is "an elastic web scraping cluster running on Kubernetes". It has a queueing module for "real-time data feeds in a producer-consumer architecture" — [estela README](https://github.com/bitmakerla/estela).
- Ulixee describes "a scraping engine with a built-in deployment unit that enables out-of-the-box querying across a horizontal deployment". Its Datastore is a "Packaged 'database' containing API access to crawler functions and extractor functions", and its Cloud lets users "Run Ulixee tooling on a remote machine" — [ulixee/platform README](https://github.com/ulixee/platform).

#### Hosted-browser servers and browser clouds' open pieces
- Browserless:
  - LICENSE: "This work is dual-licensed under the MongoDB Server-Side Public License OR the browserless commercial license", with `SPDX-License-Identifier: SSPL-1.0 OR Browserless Commercial License`. "If you want to use browserless to build commercial sites, applications, or in a continuous-integration system that's closed-source then you'll need to purchase a commercial license" — [browserless LICENSE](https://github.com/browserless/browserless/blob/HEAD/LICENSE).
  - README: offers tiers "Open Source (Self-Hosted)", "Enterprise Docker (Self-Hosted)" and a cloud. Features include "Parallelism and queueing — Handle multiple sessions with configurable concurrency limits" and ARM64 support. The image is `ghcr.io/browserless/chromium`, and clients connect with Puppeteer or Playwright — [browserless README](https://github.com/browserless/browserless).
- Steel Browser:
  - "Uses Puppeteer and CDP for complete control over Chrome instances -- allowing you to connect using Puppeteer, Playwright, or Selenium".
  - Features: session management (cookies, local storage), "Built-in proxy chain management for IP rotation", Chrome extensions, a debugging UI, and APIs for markdown, readability, screenshots and PDFs.
  - Runs with `docker run -p 3000:3000 -p 9223:9223`. SDKs are "compatible with both Steel Cloud and self-hosted Steel instances (changeable using the baseURL option)". "Steel is in public beta."
  — [steel-browser README](https://github.com/steel-dev/steel-browser)
- Kernel images:
  - Run headful Chromium "in a Docker container or with Unikraft unikernels".
  - On a unikernel there is "Automated standby / 'sleep mode' when there is no network activity", and "the unikernel's state gets snapshotted and can be restored exactly as it was when it went to sleep".
  - CDP port 9222 is "exposed via ncat".
  - There is a read/write live view (noVNC, or WebRTC through neko, with a read-only option) and "Controllable video replays".
  - Unikraft Cloud needs a TURN server for WebRTC "as direct exposure of UDP ports is not currently supported". The deployment URL "is public, meaning anyone can access the remote GUI if they have the url".
  — [kernel-images README](https://github.com/kernel/kernel-images)
- Kernel hypeman:
  - "Run containerized workloads in VMs, powered by Cloud Hypervisor, Firecracker, QEMU, and Apple Virtualization.framework".
  - "Standby & restore — snapshot a VM to disk and resume it in milliseconds", plus GPU passthrough.
  - "KVM virtualization support required".
  — [hypeman README](https://github.com/kernel/hypeman)
- Lightpanda:
  - "The default license for this project is AGPL-3.0-only" — [LICENSING.md](https://github.com/lightpanda-io/browser/blob/HEAD/LICENSING.md).
  - "Not a Chromium fork. Not a WebKit patch. A new browser, written in Zig." It exposes a CDP server on port 9222.
  - The vendor claims it is "16x lighter and 9x faster than Chromium": 123 MB against 2 GB peak memory, and 5 s against 46 s, for 100 pages.
  — [lightpanda README](https://github.com/lightpanda-io/browser)
- Notte: "This project is licensed under the Server Side Public License v1". It offers Secrets Vaults for credentials — [notte README](https://github.com/nottelabs/notte).
- Skyvern:
  - "All of the core logic powering Skyvern is available in this open source repository licensed under the AGPL-3.0 License, with the exception of anti-bot measures available in our managed cloud". Skyvern Cloud "allows you to run multiple Skyvern instances in parallel and comes bundled with anti-bot detection mechanisms".
  - The pip quickstart defaults to SQLite. A Docker path runs "Postgres, API, UI".
  — [skyvern README](https://github.com/Skyvern-AI/skyvern)
  - Skyvern's rustwright (MIT, 913 stars) is "A Rust rewrite of Playwright" that "runs on an in-process Rust CDP engine". The vendor claims it is 2.55 times faster and uses 70% less memory — [rustwright README](https://github.com/Skyvern-AI/rustwright).
- Browser Use:
  - Offers three paths: a fully hosted cloud ("hosted agent, stealth browsers, and infrastructure for profiles, recordings, and data policies"), a CLI, and the open-source Python library — [browser-use README](https://github.com/browser-use/browser-use).
  - browser-harness (MIT, 18,394 stars): "Connect an LLM directly to your real browser through one editable CDP websocket" — [browser-harness README](https://github.com/browser-use/browser-harness).
  - workflow-use is AGPL-3.0 (4,220 stars) — [github.com/browser-use](https://github.com/browser-use).
- Browserbase:
  - The MCP server repo says: "This repository is archived and no longer maintained. It was originally published as a reference implementation and is retained for historical purposes. It should not be interpreted as representative of Browserbase's current production services or implementation" — [mcp-server-browserbase README](https://github.com/browserbase/mcp-server-browserbase).
  - open-operator: "The source code has been moved to browserbase/gtm-demos" — [open-operator README](https://github.com/browserbase/open-operator).
  - The separate stagehand-python repo is archived, and Python tags (stagehand-python@4.2.1, 2026-10-10) now live in the stagehand repo — [github.com/browserbase](https://github.com/browserbase), [stagehand](https://github.com/browserbase/stagehand).
- Hyperbrowser:
  - HyperAgent's LICENSE is the GNU AGPL v3 "or (at your option) any later version", Copyright (c) 2025 S2 Labs Inc. — [HyperAgent LICENSE](https://github.com/hyperbrowserai/HyperAgent/blob/HEAD/LICENSE).
  - The org's other public repos are examples, MIT SDKs and an MIT MCP server — [github.com/hyperbrowserai](https://github.com/hyperbrowserai).

#### Browser grids, desktops and multi-provider control planes
- Selenium Grid's Helm chart scales browser nodes with KEDA: "Selenium Grid has the ability to autoscaling browser nodes up/down based on the pending requests", and "To do this KEDA is used". The chart also covers Dynamic Grid and video recording — [selenium-grid chart README](https://github.com/SeleniumHQ/docker-selenium/blob/HEAD/charts/selenium-grid/README.md).
- agent-infra/sandbox ships its image from `enterprise-public-cn-beijing.cr.volces.com/vefaas-public/all-in-one-sandbox:1.11.0` and includes a Kubernetes deployment section — [agent-infra/sandbox README](https://github.com/agent-infra/sandbox).
- trycua/cua: "The root LICENSE.md is the MIT License". The cua-spaces apps and several crates (keyvault, teleport, volume and others) are under the "Functional Source License, Version 1.1, MIT Future License" (FSL-1.1-MIT) — [cua LICENSING.md](https://github.com/trycua/cua/blob/HEAD/LICENSING.md).
- Oya Browser (349 stars) describes itself as "The browser control plane for AI agents", with "One API over Oya Cloud, Browserbase, Steel, Anchor, Browser Use or your own Chrome". Its license badge reads "Sustainable Use + MIT SDK", and a self-hosted server needs a license key for serious use — [oya-browser README](https://github.com/OyadotAI/oya-browser).
- GitHub topic listings, checked 2026-10-12:
  - topic:browser-as-a-service: the top result was hfreire/browser-as-a-service (54 stars, MIT).
  - topic:headless-chrome-cluster and topic:scraping-platform returned no repositories.
  - topic:crawler-management: crawlab-lite (231 stars, last push 2023-02-09).
  - topic:browser-infrastructure: oya-browser, plus repositories with 0 or 1 stars.
  — [GitHub topic: browser-as-a-service](https://github.com/topics/browser-as-a-service), [crawler-management](https://github.com/topics/crawler-management), [browser-infrastructure](https://github.com/topics/browser-infrastructure)

#### Apify's open infrastructure pieces
- [proxy-chain](https://github.com/apify/proxy-chain) (Apache-2.0, 1,044 stars, v3.0.1 on 2026-09-08): a "TypeScript/Node.js implementation of a proxy server (think Squid) with support for SSL".
- [fingerprint-suite](https://github.com/apify/fingerprint-suite): Apache-2.0, 2,629 stars, v2.1.88 (2026-08-05).
- got-scraping: package.json declares `"license": "Apache-2.0"`, though GitHub detects no LICENSE file. 774 stars, v4.2.1 (2026-02-24), tag v4.2.2 (2026-10-06), 3 commits since 2026-07-14 — [package.json](https://github.com/apify/got-scraping/blob/HEAD/package.json).
- [apify-cli](https://github.com/apify/apify-cli): Apache-2.0, 256 stars, v1.10.0 (2026-09-02), betas up to v1.10.1-beta.37 (2026-10-09).
- [actor-templates](https://github.com/apify/actor-templates): package.json declares `"license": "ISC"`. 62 stars, 86 commits since 2026-07-14, with templates, agent-bases and wrappers folders.
- Actor whitepaper (Apache-2.0):
  - "Actors are programs packaged as Docker images". "The Apify platform takes care of securely hosting the Actor's Docker containers", and "provides an open API, cron-style scheduler, webhooks".
  - The input schema "is used by the system to generate user interface, API examples".
  - Sections cover the key-value store, datasets, Metamorph, webhooks, "Standby mode", "Multi-tenant Standby mode" and "Monetization".
  — [actor-whitepaper README](https://github.com/apify/actor-whitepaper)
- [crawlee](https://github.com/apify/crawlee) (Apache-2.0, 26,114 stars, v3.18.2; v4.0.0-rc.1 on 2026-10-06), [impit](https://github.com/apify/impit) (Apache-2.0, a "rust library for browser impersonation", 611 stars), [apify-mcp-server](https://github.com/apify/apify-mcp-server) (MIT, 10,631 stars).
- [Camoufox](https://github.com/daijro/camoufox): MPL-2.0, 12,474 stars, v156.0.1-beta.36 (2026-10-06).

### Inferences
- License fit for a closed commercial Goodfetch Cloud, read from the license texts above (not legal advice):
  - **Reusable code (permissive):** kernel-images, hypeman, Steel Browser, neko, sockpuppetbrowser, Selenium and docker-selenium, agent-infra/sandbox, e2b-dev/desktop, the Apify pieces (proxy-chain, fingerprint-suite, got-scraping, impit, crawlee, CLI, whitepaper), Stagehand, the Browser Use library, harness and web-ui, rustwright, Scrapyd, Crawlab, Gerapy, estela, Splash, Jina Reader, changedetection.io, StormCrawler, Nutch and spider.
  - **Reusable with conditions:**
    - Crawl4AI: the attribution clause reaches "public uses".
    - Camoufox: MPL-2.0 file-level copyleft.
    - ScrapydWeb: GPL-3.0 has no network clause, so it applies on distribution.
    - cua: MIT core; the FSL parts carry a non-compete until they convert to MIT.
  - **Run only as unmodified, separate services, never linked into Goodfetch code:** the AGPL projects Firecrawl, Skyvern, Lightpanda, Browsertrix, HyperAgent and workflow-use. Modifications would have to be offered to network users. This matches ADR 0018, which keeps Lightpanda out of process.
  - **Avoid for a closed cloud:**
    - Browserless and Notte (SSPL-1.0): Browserless itself requires a commercial license for closed-source commercial use.
    - Oya (Sustainable Use License): fair-code, not open source.
- Market pattern: vendors open the SDK or agent framework (Stagehand, Browser Use, Crawlee) and keep the browser fleet closed. When they open a server (Firecrawl, Skyvern), they hold back the anti-bot engine. Firecrawl's docs present Fire-engine as a separately configured engine, which looks like a pluggable interface. That resembles ADR 0018's rule that the public engine keeps the interface a Private component plugs into, but Goodfetch's stated reason (legal exposure, not selling features) is different.
- None of the open projects is a multi-tenant actor platform with a marketplace. Crawlab, Gerapy and ScrapydWeb are single-tenant admin panels, and estela is tiny. Apify's Actor whitepaper is the only open, written specification of an actor model with storage, standby, multi-tenant standby and monetization, so it is the best checklist for Goodfetch's Recipe manifest and run model.
- For hosted browsers, the reusable value is mostly in patterns, since Goodfetch ships its own stealth Chromium and Browser host:
  - Standby on idle with a snapshot, then restore (Kernel on Unikraft; hypeman).
  - A read-only live view over WebRTC with TURN (Kernel with neko).
  - Admission control and backpressure on CDP connections (sockpuppetbrowser, Browserless).
  - Autoscaling browser nodes on queue depth with KEDA (Selenium Grid).
  - Never a public, unauthenticated live-view URL (the caveat Kernel documents).
- Rustwright, Skyvern's MIT Playwright-compatible Rust CDP engine, is a direct reference point for Goodfetch's own CDP driver, and possibly a competitor to it.

### Gaps
- The web-search budget ran out. Hacker News threads, talks and vendor blogs about these projects were not searched, and discovery relied on GitHub org and topic listings, which are sparse.
- Lightpanda's hosted offering, and the engine features it holds back from the open code (ADR 0018 mentions multi-context tabs and sandboxing), were not re-verified; the README mentions no cloud.
- Browser Use: no open-source hosted-browser server appeared among the org's nine most-starred public repos. The full listing was not reviewed.
- Steel's internal architecture (API framework, session pooling, what port 9223 serves) was not verified beyond the README.
- Spider's cloud components were not inventoried beyond spider-rs/spider and spider-rs/headless-browser.
- Skyvern published no repository-level advisories. An older Skyvern template-injection CVE recalled from memory could not be verified without search.

## 2. Which open-source "cloud around an SDK or framework" platforms in adjacent fields are the best reference architectures for an open-core cloud?

### Takeaway
E2B's Apache-2.0 runtime (formerly e2b-dev/infra) is the strongest reference, and is reusable code, for a per-run isolated fleet:
- snapshot-resume Firecracker microVMs with lazy memory over userfaultfd, a copy-on-write rootfs over NBD, and fork;
- a strict split between control plane and data plane;
- a per-sandbox nftables egress firewall with SNI and Host allow and deny lists;
- Terraform for GCP, AWS and Azure.

Among workflow and data platforms, Temporal (MIT), Hatchet (MIT), Trigger.dev (Apache-2.0), Dagster and Prefect (Apache-2.0) and Supabase (Apache-2.0) are permissive. Restate (BUSL-1.1), Inngest (SSPL-1.0 with a delayed Apache-2.0 grant), Airbyte (ELv2) and Windmill (AGPL plus a proprietary EE) restrict hosting. Two recurring open-core moves: the cloud keeps capabilities self-hosting lacks (Trigger.dev checkpoints, Firecrawl's Fire-engine), and some vendors retreat entirely (Daytona went private in June 2026; Gitpod Classic is deprecated).

### Cited Findings

#### E2B (e2b-dev/runtime, formerly e2b-dev/infra)
- The old e2b-dev/infra name now resolves to e2b-dev/runtime: "The runtime behind every E2B stack: Cloud, Enterprise, and your own machine". Apache-2.0, Go, 1,709 stars, release 2026.30 (2026-09-10), 723 commits since 2026-07-14 — [e2b-dev/runtime](https://github.com/e2b-dev/runtime).
- "E2B Runtime is the complete backend that powers E2B Cloud: the control-plane API, the per-node orchestrator that drives Firecracker, the agent that runs inside" each sandbox. "The same code serves the public cloud, dedicated enterprise deployments, and the single-machine" setup — [runtime README](https://github.com/e2b-dev/runtime).
- "A sandbox is a resumed snapshot. Templates are pre-booted VMs (memory, disk, and machine state) stored in object storage. 'Creating' a sandbox means restoring one, not booting a kernel. Memory pages are served lazily on page fault through userfaultfd" — [runtime README](https://github.com/e2b-dev/runtime).
- "Control plane and data plane never mix. The API decides where a sandbox runs and records that it runs. The orchestrator on each node owns how it runs: the Firecracker process, the network namespace, the block device, the cgroup" — [runtime README](https://github.com/e2b-dev/runtime).
- "One Firecracker microVM per sandbox, in its own cgroup and network namespace, with a per-sandbox nftables egress firewall and SNI/Host-inspecting domain allow and deny lists". "Fork checkpoints a live sandbox in place and starts new sandboxes from that snapshot, up to a hundred per request" — [runtime README](https://github.com/e2b-dev/runtime).
- The request path is SDK/CLI, then REST API, then gRPC to the orchestrator, then the Firecracker microVM, then envd, then the user's processes. Backing stores are PostgreSQL, Redis, object storage (templates and snapshots) and ClickHouse (events and metrics). "E2B Embed" runs the whole stack on one Linux host with KVM, and "The same package ships as Terraform for GCP, AWS, and Azure and as a Kubernetes manifest" — [runtime README](https://github.com/e2b-dev/runtime).
- From the architecture document:
  - "the template rootfs stays read-only; writes go to a per-sandbox COW cache exposed to Firecracker as an NBD block device".
  - Placement samples "K ready nodes" and scores them. Nodes are "discovered via Nomad, Kubernetes, or a" static list. Service discovery supports "Kubernetes, DNS, a static list, or the legacy Nomad".
  - A Nomad autoscaler plugin lives in `packages/nomad-nodepool-apm`. LaunchDarkly flags gate placement. ClickHouse "runs on its own nodes".
  — [docs/ARCHITECTURE.md](https://github.com/e2b-dev/runtime/blob/HEAD/docs/ARCHITECTURE.md)
- The SDK repo e2b-dev/E2B is Apache-2.0 with 14,280 stars — [e2b-dev/E2B](https://github.com/e2b-dev/E2B).

#### Daytona
- "This repository is no longer maintained. As of June 2026, Daytona's core development has moved to a private codebase. This repository will receive no further updates, fixes, or releases." It "remains public and free to use, fork, and build on". The repo is archived, with 71,565 stars; the last release was v0.190.0 (2026-06-23) — [daytona README](https://github.com/daytonaio/daytona).
- The license at v0.190.0 is the GNU AGPL v3 — [LICENSE at v0.190.0](https://github.com/daytonaio/daytona/blob/v0.190.0/LICENSE).
- Architecture:
  - Control plane: a NestJS API "backed by Redis and PostgreSQL", which hosts the snapshot builder and the sandbox manager.
  - A proxy that routes on `{port}-{sandboxId}` hostnames.
  - Runners that "poll the API for jobs", and a sandbox daemon that exposes a Toolbox API.
  - An internal OCI registry on S3-compatible storage as the snapshot store, and S3-backed volumes.
  - "Each sandbox runs as an isolated instance with its own Linux namespaces". The page names no microVM or runtime.
  — [Daytona architecture docs](https://www.daytona.io/docs/architecture/)
- At v0.190.0 the apps are api, cli, daemon, dashboard, docs, otel-collector, proxy, runner, snapshot-manager and ssh-gateway, and the compose file runs the API and runner images with `privileged: true` — [apps at v0.190.0](https://github.com/daytonaio/daytona/tree/v0.190.0/apps), [docker-compose.yaml at v0.190.0](https://github.com/daytonaio/daytona/blob/v0.190.0/docker/docker-compose.yaml).

#### Trigger.dev
- Apache-2.0, 16,522 stars, v4.7.3 (2026-10-06), 731 commits since 2026-07-14 — [trigger.dev](https://github.com/triggerdotdev/trigger.dev).
- During waits, "the system uses CRIU (Checkpoint/Restore In Userspace) to create a checkpoint of the task's entire state". Resources are released, and the checkpoint "is loaded back into a new execution environment" when the wait ends — [docs/how-it-works.mdx](https://github.com/triggerdotdev/trigger.dev/blob/HEAD/docs/how-it-works.mdx).
- Self-hosting is "functionally the same as Trigger.dev Cloud with some exceptions". The feature table lists Checkpoints ("Non-blocking waits, less resource usage") as available in Cloud but not self-hosted. Guides cover Docker Compose and Kubernetes — [docs/self-hosting/overview.mdx](https://github.com/triggerdotdev/trigger.dev/blob/HEAD/docs/self-hosting/overview.mdx).

#### Hatchet, Windmill, Inngest, Temporal, Restate
- Hatchet: MIT, 8,103 stars, v0.110.5 (2026-10-06). A "platform for orchestrating background tasks, AI agents, and durable workflows", also offered as Hatchet Cloud. It argues that "Traditional task queues like BullMQ and Celery trade off durability for throughput. Tasks persist on the broker (typically Redis or RabbitMQ) while the task is executing, but are not persisted afterwards" — [hatchet README](https://github.com/hatchet-dev/hatchet).
- Windmill:
  - License: "variously licensed under the Apache License Version 2.0 ..., or the AGPLv3 License ... and a proprietary license for certain enterprise features". Files under `backend/` are AGPLv3 except snippets under the "enterprise" compile flag — [windmill LICENSE](https://github.com/windmill-labs/windmill/blob/HEAD/LICENSE).
  - Architecture: "Rust - stateless API servers and workers pulling jobs from a Postgres queue", with "nsjail for filesystem/resource isolation, and PID namespace isolation (enabled by default) to prevent jobs from accessing worker process memory", and "One encryption key per workspace" — [windmill README](https://github.com/windmill-labs/windmill).
  - 18,161 stars, v1.830.0 (2026-10-09), 1,277 commits since 2026-07-14.
- Inngest: LICENSE.md combines the "Server Side Public License, Version 1.0" with an "Apache 2.0 Future License", including a "Grant of Future License ... under the Apache License, Version 2.0, that is effective on the third" (anniversary; the line is truncated in the extraction) — [inngest LICENSE.md](https://github.com/inngest/inngest/blob/HEAD/LICENSE.md). "Self-hosting the Inngest server is possible and easy to get started with" — [inngest README](https://github.com/inngest/inngest). 5,933 stars, v1.46.0 (2026-10-06).
- Temporal: MIT, 23,608 stars, v1.32.1 (2026-10-08) — [temporal](https://github.com/temporalio/temporal). Temporal Cloud operates in 14 AWS regions and 6 Google Cloud regions, and the page lists no Azure regions — [Temporal Cloud regions](https://docs.temporal.io/cloud/regions).
- Restate: BUSL-1.1.
  - The Additional Use Grant bars "a Public Restate Platform Service", defined as "a managed service that allows third parties (other than your employees and contractors) to access Restate's APIs and register their own service deployments".
  - The license says that "Any type of production deployment of Restate that is invoking services or workflows written by the Licensee" is not such a service.
  - Change Date: "4 years after release"; Change License: Apache-2.0.
  - 4,547 stars, v1.7.13 (2026-10-01), v1.8.0-rc.2 (2026-10-08).
  — [restate LICENSE](https://github.com/restatedev/restate/blob/HEAD/LICENSE)

#### Airbyte, Dagster, Prefect
- Airbyte: the airbyte and airbyte-platform LICENSE files are ELv2: "You may not provide the software to third parties as a hosted or managed service, where the service provides users with access to any substantial set of the features or functionality of the software" — [airbyte LICENSE](https://github.com/airbytehq/airbyte/blob/HEAD/LICENSE), [airbyte-platform](https://github.com/airbytehq/airbyte-platform).
- The Airbyte README shows both MIT and ELv2 badges, pointing to its licenses page. It contrasts Airbyte Open Source with Airbyte Cloud and cites "600+ connectors", a no-code Connector Builder and a low-code CDK — [airbyte README](https://github.com/airbytehq/airbyte). Latest release v2.0.0 (2025-10-15); 22,208 stars; 5,148 commits since 2026-07-14.
- Dagster: Apache-2.0, 16,268 stars, 1.13.26 (2026-10-09) — [dagster](https://github.com/dagster-io/dagster).
- Prefect: Apache-2.0, 24,005 stars, 3.8.8 (2026-10-06). Self-hosted with `prefect server start` (UI on port 4200). The README says Prefect Cloud is "automating over 200 million data tasks monthly" — [prefect README](https://github.com/PrefectHQ/prefect).

#### Supabase
- Apache-2.0, 111,372 stars, tag self-hosted/v0.8.2 (2026-09-23) — [supabase](https://github.com/supabase/supabase).
- Components are Postgres, Studio, GoTrue (Auth), PostgREST, Realtime, the Storage API, Edge Functions (Deno), postgres-meta, Supavisor ("A multi-tenant Postgres connection pooler") and Envoy as the API gateway. "Our cloud offering is compatible with our self-hosted product" — [Supabase architecture](https://supabase.com/docs/guides/getting-started/architecture).
- The hosted platform runs on AWS: "Supabase will deploy your project to an available AWS region within that area", for example us-east-1, eu-central-1, ap-southeast-1 and sa-east-1 — [Supabase regions](https://supabase.com/docs/guides/platform/regions).
- Supavisor: Apache-2.0, 2,271 stars, v2.9.13 (2026-09-10) — [supavisor](https://github.com/supabase/supavisor).

#### Gitpod (now Ona) and Coder
- "Gitpod has been renamed to Ona ... We no longer recommend this version of Gitpod". The repo is AGPL-3.0 and its last release was 2022.11.3 (2023-06-05) — [gitpod README](https://github.com/gitpod-io/gitpod).
- The "We're leaving Kubernetes" post (2024-10-31) reports six years on Kubernetes with about 1.5 million users. It concludes that "Kubernetes is not the right choice for building development environments", and that the replacement control plane is "heavily inspired by Kubernetes" — [Ona blog](https://ona.com/stories/we-are-leaving-kubernetes).
- Coder: AGPL-3.0, 16,964 stars, v2.36.7 (2026-10-05) — [coder](https://github.com/coder/coder).

### Inferences
- E2B's runtime maps closely onto Goodfetch's decisions:
  - E2B's template snapshot, resume, pause and fork correspond to one engine process per run, isolated in a VM, with checkpoint, migrate and resume. The same mechanism could cut cold starts for the engine binary and for the stealth Chromium.
  - Its per-sandbox nftables plus SNI egress firewall is the enforcement point Goodfetch needs to force run traffic through managed proxies and block metadata and internal ranges.
  - Being Apache-2.0, it can be forked or reused freely.
- Goodfetch's engine-level checkpoints know the run's own state, which avoids the CRIU path Trigger.dev depends on. Trigger.dev keeping checkpoints Cloud-only suggests CRIU is hard to operate outside its own fleet.
- For the cloud's own control-plane workflows (schedules, retries, migrations, billing jobs), Temporal (MIT; Temporal Cloud on AWS) or Hatchet (MIT) carry no license risk. Restate is allowed for Goodfetch's own services. However, a design where users' Recipes register as Restate services could fall under the "Public Restate Platform Service" bar, and Inngest's SSPL raises the same problem for user code.
- Airbyte's move to ELv2 and Daytona's retreat to a private codebase show license and maintenance drift in vendor-led open source. Critical cloud dependencies are safer when foundation-governed or permissive and widely forked.
- Supabase's split (a dedicated Postgres per project on AWS, shared multi-tenant services such as the Supavisor pooler, an Envoy gateway) mirrors the target of single-tenant engines per run under a multi-tenant control plane.
- Airbyte's connector platform and Apify's Actors both package user code as container images with a declared spec, which supports Goodfetch's Recipe manifest approach.
- Windmill's Postgres queue plus nsjail is a credible, simple model for scheduling Recipes at modest scale. Its 2026 nsjail escape (see question 5) argues for microVM isolation for untrusted Recipe code.

### Gaps
- Which cloud E2B Cloud, Daytona, Trigger.dev Cloud, Hatchet Cloud, Windmill Cloud, Inngest Cloud, Restate Cloud, Airbyte Cloud, Dagster+ and Prefect Cloud run on was not verified, because the search budget was exhausted. Only Temporal Cloud (AWS and GCP) and Supabase (AWS) were confirmed. E2B's Terraform targets GCP, AWS and Azure.
- Dagster+ hybrid and serverless deployment, and Prefect Cloud's hybrid worker model, were not verified from primary docs in this session.
- Hatchet's storage and queue internals (Postgres-only or with RabbitMQ) and Inngest's self-hosted components were not verified in detail.
- Airbyte's per-connector licensing (MIT or ELv2) was not verified; its licenses page was not read.
- The isolation runtime used by Daytona's runners (plain containers, Sysbox, Kata or microVMs) is not stated in its docs; only privileged runner containers were observed.

## 3. Which isolation and runtime primitives are viable and mature, with production users and AWS requirements?

### Takeaway
- **Firecracker** is the most proven microVM. It powers AWS Lambda and Fargate, E2B and Vercel Sandbox, and every platform it lists as tested is an EC2 `.metal` instance. Since 2026-02-16, AWS also supports nested virtualization (KVM or Hyper-V) on C8i, M8i and R8i instances, which opens finer-grained hosts but sits outside Firecracker's tested matrix.
- **gVisor** runs anywhere: its systrap platform is recommended inside VMs.
- **Kata Containers and Cloud Hypervisor** need KVM.
- **Kubernetes SIG agent-sandbox** exists. It reached v1.0.0 on 2026-08-28 with v1beta1 APIs, and it orchestrates gVisor or Kata pods with warm pools.
- **Others:** microsandbox is still beta. firecracker-containerd is stagnant and Weave Ignite is archived. Nomad is under BUSL-1.1 and E2B now calls it "legacy". OpenFaaS CE needs a commercial license. KEDA (CNCF graduated) and Karpenter are active and current on EKS.

### Cited Findings

| Primitive (source) | License | Stars | Latest release (date) | Commits since 2026-07-14 | Notes |
|---|---|---|---|---|---|
| [Firecracker](https://github.com/firecracker-microvm/firecracker) | Apache-2.0 | 37,298 | v1.17.0 (2026-09-10) | 299 | KVM; Lambda and Fargate |
| [firecracker-containerd](https://github.com/firecracker-microvm/firecracker-containerd) | Apache-2.0 | 2,958 | no release | 2 | 84 open issues |
| [Cloud Hypervisor](https://github.com/cloud-hypervisor/cloud-hypervisor) | Apache-2.0 and BSD-3-Clause (LICENSES/) | 6,319 | v53.0 (2026-07-12) | 803 | KVM or MSHV |
| [Kata Containers](https://github.com/kata-containers/kata-containers) | Apache-2.0 | 8,919 | 4.2.0 (2026-09-15) | 1,054 | 1,021 open issues |
| [gVisor](https://github.com/google/gvisor) | Apache-2.0 | 19,640 | release-20261005.0 (2026-10-08) | 1,190 | Userspace kernel in Go |
| [Unikraft](https://github.com/unikraft/unikraft) | BSD-3-Clause (main license) | 3,889 | RELEASE-0.21.0 (2026-05-20) | 18 | Unikernel; [kraftkit](https://github.com/unikraft/kraftkit) BSD-3-Clause v0.12.16 |
| [microsandbox](https://github.com/superradcompany/microsandbox) (was zerocore-ai) | Apache-2.0 | 8,651 | v0.7.8 (2026-10-09) | 537 | Beta |
| [libkrun](https://github.com/libkrun/libkrun) (was containers/libkrun) | Apache-2.0 | 2,771 | v1.19.6 (2026-09-29) | 209 | Library VMM |
| [Hyperlight](https://github.com/hyperlight-dev/hyperlight) | Apache-2.0 | 4,729 | v0.17.0 (2026-08-28) | 188 | CNCF sandbox project |
| [Sysbox](https://github.com/nestybox/sysbox) | Apache-2.0 | 3,903 | v0.7.1 (2026-07-31) | 17 | runc alternative for system containers |
| [kernel/hypeman](https://github.com/kernel/hypeman) | MIT | 370 | n/a | n/a | Runs Firecracker, Cloud Hypervisor or QEMU |
| [Flintlock](https://github.com/liquidmetal-dev/flintlock) | MPL-2.0 | 1,511 | v0.16.0 (2026-10-03) | 150 | MicroVM lifecycle manager |
| [Weave Ignite](https://github.com/weaveworks/ignite) | Apache-2.0 | 3,530 | v0.10.0 (2021-07-19) | 0 | Archived |
| [agent-sandbox](https://github.com/kubernetes-sigs/agent-sandbox) | Apache-2.0 | 4,221 | v1.0.6 (2026-10-08) | 463 | Created 2025-08-12 |
| [Knative Serving](https://github.com/knative/serving) | Apache-2.0 | 6,108 | knative-v1.23.0 (2026-07-29) | 17 | Scale-to-zero |
| [KEDA](https://github.com/kedacore/keda) | Apache-2.0 | 10,585 | v2.21.0 (2026-09-23) | 194 | CNCF graduated |
| [Karpenter (AWS provider)](https://github.com/aws/karpenter-provider-aws) | Apache-2.0 | 7,722 (core: 2,222) | v1.15.0 (2026-10-10) | 171 | EKS node autoscaler |
| [Nomad](https://github.com/hashicorp/nomad) | BUSL-1.1 (licensor IBM) | 17,000 | v2.0.7 (2026-09-18) | 172 | 1,504 open issues |
| [OpenFaaS faas](https://github.com/openfaas/faas) | OpenFaaS CE EULA for OpenFaaS Ltd code; MIT for third-party contributions | 26,243 | 0.27.13 (2025-08-29) | 0 | faasd 0.19.7 (2025-03-18); faas-netes 0.18.17 (2026-05-13) |

- Firecracker:
  - The README says Firecracker uses KVM and was built for services "like AWS Lambda and AWS Fargate", that it "is already running production workloads within AWS", and that the jailer is for "starting Firecracker in production".
  - Its tested platforms are m5n.metal, m6i.metal, m7i.metal-24xl and -48xl, m8i.metal-48xl and -96xl, m6a.metal, m7a.metal-48xl, m6g.metal, m7g.metal and m8g.metal-24xl. All are bare-metal EC2 instances.
  — [Firecracker README](https://github.com/firecracker-microvm/firecracker)
- Firecracker snapshots: "Diff snapshots are still in developer preview". The document has sections on "Snapshot security and uniqueness", a "VMGenID device limitation" and network connectivity for clones — [snapshot-support.md](https://github.com/firecracker-microvm/firecracker/blob/HEAD/docs/snapshotting/snapshot-support.md).
- AWS nested virtualization was posted on Feb 16, 2026, for C8i, M8i and R8i instances. Customers can create nested VMs by "running KVM or Hyper-V on virtual EC2 instances", in "all commercial regions". "Previously, customers could only create and manage virtual machines inside bare metal EC2 instances" — [AWS What's New](https://aws.amazon.com/about-aws/whats-new/2026/02/amazon-ec2-nested-virtualization-on-virtual).
- gVisor platforms: "when running on bare-metal (not inside a VM), the KVM platform will provide the best performance. The systrap platform is a better choice when running inside a VM, or on a machine without virtualization support". "While running within a nested VM is feasible with the KVM platform, the systrap platform will often provide better performance in such a setup" — [gVisor platforms.md](https://github.com/google/gvisor/blob/HEAD/g3doc/architecture_guide/platforms.md). The README points to "ADOPTERS.md for a list of known production users" — [gVisor README](https://github.com/google/gvisor).
- Production users:
  - Modal isolates containers "using primitives like gVisor or microVMs" — [Modal security](https://modal.com/docs/guide/security).
  - In Vercel Sandbox, "Each sandbox runs in a secure Firecracker microVM with its own filesystem and network". It supports persistent sandboxes, snapshots and millisecond starts — [Vercel Sandbox docs](https://vercel.com/docs/vercel-sandbox).
  - E2B runs "One Firecracker microVM per sandbox" — [e2b-dev/runtime](https://github.com/e2b-dev/runtime).
- Cloud Hypervisor "Runs on KVM or MSHV". Its LICENSES folder contains Apache-2.0.txt, BSD-3-Clause.txt and CC-BY-4.0.txt — [cloud-hypervisor](https://github.com/cloud-hypervisor/cloud-hypervisor).
- Kata Containers documents several hypervisors and an "optional built-in VMM", dragonball — [Kata README](https://github.com/kata-containers/kata-containers).
- Unikraft: "The main license of the project is the following BSD 3-clause license" — [unikraft COPYING.md](https://github.com/unikraft/unikraft/blob/HEAD/COPYING.md).
- Unikraft Cloud (vendor marketing claims):
  - Performance: "<10ms — COLD START", "Scale to zero and resume in <10ms", "Fork a live virtual computer in 10 milliseconds", "strong VM isolation" and "+100K microVMs per server".
  - Deployment: on-prem, BYOC or bare metal, and on AWS EC2, GCP and Azure VMs or on EKS, GKE and AKS.
  - Named customers: Netlify, Prisma, Indices and Cuemby.
  — [unikraft.com](https://unikraft.com/)
  - Kernel's browser images deploy to Unikraft Cloud — [kernel-images README](https://github.com/kernel/kernel-images).
- microsandbox: "Microsandbox is still beta software. Expect breaking changes, missing features, and rough edges". It needs Linux with KVM or macOS on Apple Silicon, builds on libkrun, supports fork and snapshot, and has how-tos for Browser Use and Playwright — [microsandbox README](https://github.com/superradcompany/microsandbox).
- Hyperlight is "A Cloud Native Computing Foundation sandbox project", "a lightweight Virtual Machine Manager (VMM) designed to be embedded within" applications — [hyperlight README](https://github.com/hyperlight-dev/hyperlight).
- Kubernetes SIG agent-sandbox exists, under SIG Apps:
  - It is "developing a Sandbox Custom Resource Definition (CRD) and controller for Kubernetes".
  - "Agent Sandbox is a sandbox orchestrator. It delegates low-level container isolation to secure 'Sandbox Runtimes' (like gVisor or Kata Containers) by managing Pods configured to use these runtimes (via RuntimeClass)".
  - A Sandbox is "a single, stateful pod with a stable identity and persistent storage". Extensions add SandboxClaim and SandboxWarmPool ("a pool of pre-warmed Sandboxes").
  — [agent-sandbox README](https://github.com/kubernetes-sigs/agent-sandbox)
- agent-sandbox releases: v0.1.0 on 2025-11-07 and v1.0.0 on 2026-08-28. In v1.0.0, the core and extension APIs moved to `v1beta1` and `v1alpha1` was removed, and the release added "browser-facing path-based routing with session-cookie authentication in sandbox-router", the "next-generation sandboxd runtime" and NVIDIA NeMo Gym and Gymnasium integrations. The latest is v1.0.6 (2026-10-08) — [v1.0.0 release notes](https://github.com/kubernetes-sigs/agent-sandbox/releases/tag/v1.0.0).
- KEDA: "We are a Cloud Native Computing Foundation (CNCF) graduated project" — [KEDA README](https://github.com/kedacore/keda). Selenium Grid's chart uses KEDA's Selenium Grid scaler to autoscale browser nodes — [selenium-grid chart](https://github.com/SeleniumHQ/docker-selenium/blob/HEAD/charts/selenium-grid/README.md).
- Nomad's BUSL-1.1 covers "Nomad Version 1.7.0 or later" with IBM as licensor. Production use is allowed provided it does not include offering the software "to third parties on a hosted or embedded basis in order to compete with IBM Corp's paid version(s)" — [Nomad LICENSE](https://github.com/hashicorp/nomad/blob/HEAD/LICENSE).
- OpenFaaS: "All contributions from Alex Ellis & OpenFaaS Ltd are licensed under the OpenFaaS Community Edition (CE) EULA", and "A license is required for commercial use of OpenFaaS CE". faasd and faas-netes carry the same terms — [OpenFaaS LICENSE](https://github.com/openfaas/faas/blob/HEAD/LICENSE), [faasd LICENSE](https://github.com/openfaas/faasd/blob/HEAD/LICENSE).

### Inferences
- The AWS options for Goodfetch's per-run engine isolation and hosted browsers:
  1. **Firecracker, Cloud Hypervisor or Kata on `.metal` instances.** Tested by Firecracker and proven at E2B, but capacity comes in whole-host units.
  2. **Firecracker or Kata on C8i, M8i or R8i with nested virtualization.** Smaller hosts, but outside Firecracker's tested list, with nested overhead unmeasured.
  3. **gVisor systrap on any instance type.** Simplest on EKS, but a weaker boundary than a VM, and Chrome's compatibility and performance under gVisor need testing.
  4. **A managed microVM service such as Unikraft Cloud.** Vendor claims of sub-10 ms resume, but lock-in.

  For hosted browsers with standby on idle, option 1 or 2 with snapshots (the Kernel and E2B patterns) fits best. For short crawl runs, warm pools of pre-booted engine VMs fit best.
- **A Kubernetes path on EKS:** Karpenter for nodes, KEDA for queue-depth scaling, agent-sandbox for warm pools, claims and a session-cookie router, and gVisor or Kata via RuntimeClass. agent-sandbox is about 14 months old with beta APIs, so API churn is likely.
- **A non-Kubernetes path:** an E2B-style orchestrator on bare-metal hosts. Nomad remains usable internally under its BUSL grant (Goodfetch would not be offering Nomad itself), but E2B now treats it as legacy.
- **Avoid:**
  - OpenFaaS CE: a commercial license is required.
  - firecracker-containerd: stagnant.
  - Ignite: archived.
  - microsandbox, for multi-tenant production: beta, with eight advisories in Sep–Oct 2026 (see question 5).
  - Knative: activity is low (17 commits in about 90 days), and request-driven scale-to-zero matters less for long-lived browser sessions.
- Diff snapshots are in developer preview, and clone uniqueness needs care (VMGenID limits). Goodfetch's engine-level checkpoint is therefore the safer basis for migrating runs between hosts. VM memory snapshots are best kept for fast template starts and idle standby, and clones must refresh randomness, clocks and TLS state.

### Gaps
- Firecracker's official position on running under EC2 nested virtualization was not found, and no nested-overhead benchmarks were gathered.
- The contents of gVisor's ADOPTERS.md were not parsed. Kata and Cloud Hypervisor production users were not verified.
- The CNCF maturity of Knative, Keycloak and OpenFGA was not verified in this session.
- Chrome's stealth fidelity and performance under gVisor, Kata or nested Firecracker were not researched.
- Unikraft Cloud's figures are vendor marketing; no independent benchmarks were found, since search was unavailable.
- Whether Kernel's production fleet runs on Unikraft Cloud or on its own hypeman hosts was not verified.

## 4. Which open-source metering and billing systems fit usage-based pricing, and which open-source multi-tenant control-plane pieces (auth, API gateways) fit?

### Takeaway
- **Metering and billing:**
  - OpenMeter (Apache-2.0) ingests CloudEvents into Kafka, ClickHouse and PostgreSQL, and its site is now branded "OpenMeter by Kong" with "Konnect Metering & Billing".
  - Kill Bill (Apache-2.0) is the mature permissive billing engine.
  - Lago and Flexprice are AGPL-3.0. Polar (Apache-2.0) is a merchant of record with event meters, and Autumn (Apache-2.0) is a pricing layer over Stripe. Lotus is dead.
  - Stripe's docs now recommend Metronome, "now part of Stripe", over classic Billing Meters for new usage-based integrations.
- **Auth:** Ory and Keycloak are Apache-2.0; Zitadel is AGPL-3.0-only and published many account-takeover advisories in 2026. OpenFGA and SpiceDB (Apache-2.0) cover authorization.
- **Gateways:** Envoy Gateway, Apache APISIX and KrakenD CE are Apache-2.0 and active. Kong's open-source line has had only 3.9.x patch releases since December 2024.

### Cited Findings

| Project (source) | License | Stars | Latest release (date) | Commits since 2026-07-14 |
|---|---|---|---|---|
| [OpenMeter](https://github.com/openmeterio/openmeter) | Apache-2.0 | 2,377 | v1.0.0-beta.236 (2026-10-09) | 447 |
| [Lago](https://github.com/getlago/lago) | AGPL-3.0 | 10,672 | v1.55.0 (2026-10-07) | 49 |
| [Flexprice](https://github.com/flexprice/flexprice) | AGPL-3.0 | 6,896 | v2.1.35 (2026-10-08) | 2,342 |
| [Kill Bill](https://github.com/killbill/killbill) | Apache-2.0 | 5,791 | 0.24.22 (2026-09-29) | 72 |
| [Polar](https://github.com/polarsource/polar) | Apache-2.0 | 10,357 | SDK tags only (e.g. 2026-10-06) | 1,702 |
| [Autumn](https://github.com/useautumn/autumn) | Apache-2.0 | 2,814 | tag autumn-js-v1.4.24 (2026-10-11) | 7,607 |
| [Lotus](https://github.com/uselotus/lotus) | MIT | 1,840 | 0.10.2 (2023-03-23) | 2 |
| [Ory Kratos](https://github.com/ory/kratos) / [Hydra](https://github.com/ory/hydra) / [Keto](https://github.com/ory/keto) / [Oathkeeper](https://github.com/ory/oathkeeper) | Apache-2.0 | 13,915 / 17,600 / 5,409 / 3,608 | v26.2.0 (2026-03-20) | 39 / 25 / 18 / 11 |
| [Keycloak](https://github.com/keycloak/keycloak) | Apache-2.0 | 37,296 | 26.8.0 (2026-10-01) | 958 |
| [Zitadel](https://github.com/zitadel/zitadel) | AGPL-3.0-only | 15,280 | v4.19.4 (2026-10-01) | 116 |
| [Logto](https://github.com/logto-io/logto) | MPL-2.0 | 14,668 | v1.44.0 (2026-09-30) | 359 |
| [authentik](https://github.com/goauthentik/authentik) | MIT, with a separately licensed `authentik/enterprise/` | 25,923 | 2026.8.3 (2026-09-17) | 1,577 |
| [OpenFGA](https://github.com/openfga/openfga) | Apache-2.0 | 5,951 | v1.22.0 (2026-10-06) | 50 |
| [SpiceDB](https://github.com/authzed/spicedb) | Apache-2.0 | 7,140 | v1.56.2 (2026-09-11) | 93 |
| [Kong](https://github.com/Kong/kong) | Apache-2.0 | 44,262 | 3.9.3 (2026-06-17) | 11 |
| [Apache APISIX](https://github.com/apache/apisix) | Apache-2.0 | 17,212 | 3.19.0 (2026-09-28) | 159 |
| [Envoy Gateway](https://github.com/envoyproxy/gateway) | Apache-2.0 | 3,074 | v1.9.2 (2026-09-28) | 344 |
| [Tyk](https://github.com/TykTechnologies/tyk) | MPL-2.0, except `ee/` (commercial) | 10,857 | v5.15.1 (2026-10-02) | 68 |
| [KrakenD CE](https://github.com/krakend/krakend-ce) | Apache-2.0 | 2,690 | v3.0.0 (2026-09-30) | 54 |
| [Unkey](https://github.com/unkeyed/unkey) | AGPLv3 outside `packages/` and `internal/icons` | 5,462 | api/v1.1.58 (2026-10-09) | 651 |
| [Svix webhooks](https://github.com/svix/svix-webhooks) | MIT | 3,442 | v2.8.0 (2026-10-08) | 326 |
| [Hookdeck Outpost](https://github.com/hookdeck/outpost) | Apache-2.0 | 1,064 | v1.6.0 (2026-10-02) | 107 |

- OpenMeter:
  - "Ingest CloudEvents, attribute usage to customers, aggregate it in real time, and query it by time window or dimension".
  - "The core runtime is OpenMeter's API and worker processes plus Kafka, ClickHouse, and PostgreSQL. Redis is optional for distributed deduplication". Workers run "together with Kafka, ClickHouse, PostgreSQL, Redis, and Svix".
  - "OpenMeter is licensed under the Apache License 2.0".
  — [OpenMeter README](https://github.com/openmeterio/openmeter)
  - Its site is titled "OpenMeter by Kong - Open Source Billing and Usage Metering", invites users to "Get Started With Konnect Metering & Billing for Free Today!", and states "OpenMeter is available on GitHub under the Apache 2.0 license" — [openmeter.io](https://openmeter.io/).
- Lago:
  - Its open-source billing engine "can be paired with Premium deployment, customization, security, and support options". Some features, such as the Billing Assistant, are "Premium beta, available upon request".
  - "Lago is SOC 2 Type II certified and supports self-hosting".
  — [Lago README](https://github.com/getlago/lago)
- Flexprice is "usage-based, credit-based, and hybrid pricing with real-time metering", "Open-source and self-hostable". Its LICENSE is AGPL-3.0, copyright SQUIRRELLY TECHNOLOGIES PRIVATE LIMITED — [Flexprice README](https://github.com/flexprice/flexprice), [Flexprice LICENSE](https://github.com/flexprice/flexprice/blob/HEAD/LICENSE).
- Polar:
  - "We're the merchant of record, so we take on the billing boilerplate (receipts, customer accounts, dunning) and the headaches (sales tax, VAT)".
  - "Bill tokens, API calls, agent runs, GPU seconds, and storage with precision down to the event".
  — [Polar README](https://github.com/polarsource/polar)
- Autumn is "an open-source layer between Stripe and your application". For self-hosting, "Autumn depends on a bunch of services" — [Autumn README](https://github.com/useautumn/autumn).
- Stripe (not open source; translated from the French docs page):
  - Metronome is "Stripe's main usage-based billing platform, recommended for all new integrations". Classic usage-based billing on the Billing Meters API is "a low-level building block that remains fully supported for existing integrations".
  - Billing Meters "only reconciles usage at invoicing time". Metronome is recommended for "high-volume event ingestion at scale" and for prepaid credits.
  - "Metronome is now part of Stripe".
  — [Stripe usage-based billing docs](https://docs.stripe.com/billing/subscriptions/usage-based)
- Zitadel:
  - "This repository is licensed under the GNU Affero General Public License v3.0 (AGPL-3.0-only)", with listed directories under Apache-2.0 or MIT.
  - "All contributions must be licensed under the Apache License 2.0 to be accepted ... This approach avoids the need for a Contributor License Agreement". Commercial licensing is available.
  — [Zitadel LICENSING.md](https://github.com/zitadel/zitadel/blob/HEAD/LICENSING.md)
- Ory Kratos runs self-hosted or "As a managed service on the Ory Network", and the README mentions an "Enterprise License" — [Kratos README](https://github.com/ory/kratos).
- authentik's LICENSE: "Content outside of the above mentioned directories ... is available under the 'MIT' license", while `authentik/enterprise/` is under its own license — [authentik LICENSE](https://github.com/goauthentik/authentik/blob/HEAD/LICENSE).
- Tyk: "The code in the root directory and all subdirectories, except for the 'ee' folder, is licensed under the Mozilla Public License Version 2.0" — [Tyk LICENSE.md](https://github.com/TykTechnologies/tyk/blob/HEAD/LICENSE.md).
- Unkey's LICENSE puts content outside `packages/` and `internal/icons` under AGPLv3 — [Unkey LICENSE](https://github.com/unkeyed/unkey/blob/HEAD/LICENSE).
- Kong's release history on GitHub: 3.8.0 (2024-09-11), 3.9.0 (2024-12-13), then only 3.9.1 (2025-06-04), 3.9.2 (2026-06-04) and 3.9.3 (2026-06-17) — [Kong releases](https://github.com/Kong/kong/releases).

### Inferences
- A metering pipeline consistent with "the engine counts usage but knows no prices":
  1. Engine usage records become CloudEvents.
  2. OpenMeter (Apache-2.0) meters them, attributes them to customers and enforces real-time limits.
  3. A pricing and invoicing layer turns them into invoices: Stripe (Metronome, or Billing Meters), Polar as merchant of record, or Kill Bill.
- Because Stripe's Billing Meters reconcile only at invoice time, real-time spend caps and prepaid credits need an internal meter in the request path. This matters for agent runs and hosted browsers, which can burn budget quickly.
- Lago and Flexprice (AGPL-3.0) can run unmodified as separate services. Any modification would have to be offered to the users who interact with them over the network, for example through Lago's customer-facing portal.
- OpenMeter's critical 2026 SQL injection into shared ClickHouse queries (question 5) shows the metering store is itself a tenant boundary.
- Auth: Ory (Apache-2.0, headless, Go) or Keycloak (Apache-2.0, Java) fit a permissive stack. Zitadel brings AGPL obligations and a dense 2026 advisory record.
- Organization, team and project permissions fit OpenFGA or SpiceDB (Apache-2.0).
- API keys and rate limits: Unkey (AGPL, run unmodified) or an in-house layer.
- Gateway: Envoy Gateway (Kubernetes Gateway API) or APISIX on EKS. Kong's open-source line looks frozen on 3.9.x (an inference from releases only).
- Webhooks for finished runs and crawl jobs: Svix (MIT) or Outpost (Apache-2.0).

### Gaps
- Whether, when and on what terms Kong acquired OpenMeter is unverified. Only the "OpenMeter by Kong" branding and the Konnect links were observed; the site's footer copyright names "Tailfin Cloud Inc.".
- Kong's own statement on the future of open-source Kong Gateway was not found; only release cadence was observed.
- Lago's full Premium feature list and Flexprice's internals (ClickHouse, Kafka, Temporal) were not verified.
- When and on what terms Stripe acquired Metronome was not verified.
- The scope of Ory's Enterprise License was not verified.

## 5. What lessons (failures, scaling limits, security incidents, maintenance burden) do maintainers and users report?

### Takeaway
Five lessons recur:
1. Server modes of scraping and browser tools become RCE and SSRF surfaces once they accept user-supplied browser config, launch arguments, schemas, file paths or URLs. This happened to Crawl4AI, Firecrawl, changedetection.io and Browsertrix, and to Lightpanda's CDP and MCP endpoints.
2. Young multi-tenant control planes ship cross-tenant authorization bugs: Daytona, Trigger.dev, Hatchet, OpenMeter and Zitadel all did in 2026.
3. Isolation layers break, VM-based ones included: runc in November 2025, and Kata, Firecracker's jailer, Cloud Hypervisor and microsandbox in 2026.
4. Kubernetes fits stateful, interactive environments poorly, as Gitpod found.
5. Open "platform around a framework" repositories tend to decay, or go private, once the vendor's cloud matures.

### Cited Findings

#### Server-mode RCE, SSRF and file access in scraping and browser servers
- Crawl4AI Docker server (all 2026):
  - Unauthenticated RCE via Chromium launch-argument injection in `browser_config.extra_args` (CVE-2026-57572, critical, 2026-06-18) — [GHSA-r253-r9jw-qg44](https://github.com/unclecode/crawl4ai/security/advisories/GHSA-r253-r9jw-qg44).
  - Path traversal in crawler downloads, leading to RCE (CVE-2026-57571, critical) — [GHSA-2jq4-q6vv-4cp3](https://github.com/unclecode/crawl4ai/security/advisories/GHSA-2jq4-q6vv-4cp3).
  - Unauthenticated SSRF on `/crawl/stream` (CVE-2026-57573) — [GHSA-wm69-2pc3-rmmf](https://github.com/unclecode/crawl4ai/security/advisories/GHSA-wm69-2pc3-rmmf).
  - SSRF via proxy settings that bypasses the crawl-URL check (CVE-2026-53755) — [GHSA-6qhc-x826-342c](https://github.com/unclecode/crawl4ai/security/advisories/GHSA-6qhc-x826-342c).
  - DOM XSS in the Playground UI leading to operator API-token theft (CVE-2026-91944) — [GHSA-m446-hp3q-qfxp](https://github.com/unclecode/crawl4ai/security/advisories/GHSA-m446-hp3q-qfxp).
  - PDF redirects that bypass egress controls — [GHSA-q5rj-45vw-vp2g](https://github.com/unclecode/crawl4ai/security/advisories/GHSA-q5rj-45vw-vp2g).
  - Arbitrary file write through untrusted config bodies — [GHSA-xpp7-j28w-2gvx](https://github.com/unclecode/crawl4ai/security/advisories/GHSA-xpp7-j28w-2gvx).
  - An "Untrusted-config gate bypass via dict-wrapper laundering" that "leaks server env vars (LLM keys, SECRET_KEY)" (2026-09-23) — [GHSA-5w5p-vcv6-mm3f](https://github.com/unclecode/crawl4ai/security/advisories/GHSA-5w5p-vcv6-mm3f).
  - SSRF through `link_preview_config` — [GHSA-wh5w-hmj3-vgg7](https://github.com/unclecode/crawl4ai/security/advisories/GHSA-wh5w-hmj3-vgg7).
  - Blind SSRF via robots.txt fetches that "bypasses the Docker server egress controls" — [GHSA-f77g-77vp-r96v](https://github.com/unclecode/crawl4ai/security/advisories/GHSA-f77g-77vp-r96v).
- Firecrawl:
  - SSRF via a malicious scrape target (CVE-2024-56800, 2024-12-30) — [GHSA-vjp8-2wgg-p734](https://github.com/firecrawl/firecrawl/security/advisories/GHSA-vjp8-2wgg-p734).
  - SSRF via a malicious webhook (CVE-2025-57818, 2025-08-26) — [GHSA-p2wg-prhf-jx79](https://github.com/firecrawl/firecrawl/security/advisories/GHSA-p2wg-prhf-jx79).
  - "RCE via /scrape with changeTracking format's git-diff mode and an attacker-controlled website" (CVE-2026-72905, critical, 2026-07-29) — [GHSA-7843-ffpq-5f9p](https://github.com/firecrawl/firecrawl/security/advisories/GHSA-7843-ffpq-5f9p).
  - "Arbitrary file read via JSON Schema $ref expansion" (CVE-2026-72904) — [GHSA-3p54-jg6f-68r8](https://github.com/firecrawl/firecrawl/security/advisories/GHSA-3p54-jg6f-68r8).
- changedetection.io:
  - Authentication bypass via decorator ordering (CVE-2026-35490, critical) — [GHSA-jmrh-xmgh-x9j4](https://github.com/dgtlmoon/changedetection.io/security/advisories/GHSA-jmrh-xmgh-x9j4).
  - XPath arbitrary file read via `unparsed-text()` (CVE-2026-29039, critical) — [GHSA-6fmw-82m7-jq6p](https://github.com/dgtlmoon/changedetection.io/security/advisories/GHSA-6fmw-82m7-jq6p).
  - Zip Slip in backup restore (CVE-2026-29065, critical) — [GHSA-25g8-2mcf-fcx9](https://github.com/dgtlmoon/changedetection.io/security/advisories/GHSA-25g8-2mcf-fcx9).
  - SSRF via watch URLs (CVE-2026-27696) — [GHSA-3c45-4pj5-ch7m](https://github.com/dgtlmoon/changedetection.io/security/advisories/GHSA-3c45-4pj5-ch7m).
  - "SSRF protection bypass via CGNAT addresses" (2026-08-20) — [GHSA-gwph-fp79-379w](https://github.com/dgtlmoon/changedetection.io/security/advisories/GHSA-gwph-fp79-379w).
  - Environment-variable disclosure via the `jq env` builtin (CVE-2026-33981) — [GHSA-58r7-4wr5-hfx8](https://github.com/dgtlmoon/changedetection.io/security/advisories/GHSA-58r7-4wr5-hfx8).
- Browsertrix: "Arbitrary Command Injection due to Improper Command Sanitization in Git URLs specified as Custom Behaviors" (CVE-2026-54501, critical, 2026-06-08) — [GHSA-47vv-v544-r985](https://github.com/webrecorder/browsertrix/security/advisories/GHSA-47vv-v544-r985).
- Lightpanda:
  - "Missing Origin / Host validation on the CDP WebSocket handshake allows Cross-Site WebSocket Hijacking" (2026-08-17) — [GHSA-wjwj-jrwg-3wgm](https://github.com/lightpanda-io/browser/security/advisories/GHSA-wjwj-jrwg-3wgm).
  - "MCP HTTP server accepts any Origin / Host / content type - any web page can drive every MCP tool" (2026-09-04) — [GHSA-rqpx-pxch-q2cm](https://github.com/lightpanda-io/browser/security/advisories/GHSA-rqpx-pxch-q2cm).
  - A Same-Origin Policy bypass in the URL parser (CVE-2026-52842, critical) — [GHSA-mq6p-m9cc-q432](https://github.com/lightpanda-io/browser/security/advisories/GHSA-mq6p-m9cc-q432).
  - Cross-origin cookie attachment by `fetch`/XHR (CVE-2026-52843, critical) — [GHSA-36mm-v3c2-24cc](https://github.com/lightpanda-io/browser/security/advisories/GHSA-36mm-v3c2-24cc).
  - A WebSocket frame-length integer overflow allowing unauthenticated remote DoS — [GHSA-7c5w-p4q2-98gf](https://github.com/lightpanda-io/browser/security/advisories/GHSA-7c5w-p4q2-98gf).
- Browser Use: "`allowed_domains` can be bypassed by putting a decoy domain in http auth username portion of a URL" (CVE-2025-47241, critical, 2025-05-04) — [GHSA-x39x-9qw5-ghrf](https://github.com/browser-use/browser-use/security/advisories/GHSA-x39x-9qw5-ghrf).
- Gerapy: RCE before 0.9.8 (CVE-2021-43857), and authenticated arbitrary command execution before 0.9.9 — [GHSA-9w7f-m4j4-j3xw](https://github.com/Gerapy/Gerapy/security/advisories/GHSA-9w7f-m4j4-j3xw), [GHSA-756h-r2c9-qp5j](https://github.com/Gerapy/Gerapy/security/advisories/GHSA-756h-r2c9-qp5j).
- neko: self-service privilege escalation (CVE-2026-39386) — [GHSA-2gw9-c2r2-f5qf](https://github.com/m1k1o/neko/security/advisories/GHSA-2gw9-c2r2-f5qf).

#### Cross-tenant and control-plane bugs in multi-tenant platforms
- Daytona (June 2026):
  - "Cross-tenant organization takeover via invitation acceptance with an unverified email" (CVE-2026-54320) — [GHSA-m6hx-cffh-3f3h](https://github.com/daytonaio/daytona/security/advisories/GHSA-m6hx-cffh-3f3h).
  - "Cross-org IDOR in organization role update/delete" (CVE-2026-54322) — [GHSA-qxvm-pcfm-qc39](https://github.com/daytonaio/daytona/security/advisories/GHSA-qxvm-pcfm-qc39).
  - "Cross-tenant data leak in notification WebSocket gateway via unverified organizationId join" (CVE-2026-54324) — [GHSA-qwxf-2m7m-2m3x](https://github.com/daytonaio/daytona/security/advisories/GHSA-qwxf-2m7m-2m3x).
  - "Public sandbox previews remain accessible for up to one hour after being made private" (CVE-2026-54321) — [GHSA-ww63-pv5x-vfc8](https://github.com/daytonaio/daytona/security/advisories/GHSA-ww63-pv5x-vfc8).
- Trigger.dev (2026):
  - "Missing run authorization allows cross-tenant suspension and resume-data injection" — [GHSA-xrp6-5fpv-4fxf](https://github.com/triggerdotdev/trigger.dev/security/advisories/GHSA-xrp6-5fpv-4fxf).
  - "Restricted additional API keys can read the environment root secret via the env-var dump" — [GHSA-4gjx-f5hj-678x](https://github.com/triggerdotdev/trigger.dev/security/advisories/GHSA-4gjx-f5hj-678x).
  - "Tenant filtering bypass in query table placeholders" — [GHSA-m79r-84cc-xwg9](https://github.com/triggerdotdev/trigger.dev/security/advisories/GHSA-m79r-84cc-xwg9).
  - "Webhook SSRF fix bypass via non-global 198.18.0.0/15 benchmarking range" — [GHSA-fhh4-xvj5-m7pq](https://github.com/triggerdotdev/trigger.dev/security/advisories/GHSA-fhh4-xvj5-m7pq).
  - CLI debug deployment logs that "expose resolved environment secret values" — [GHSA-fj2x-mqqp-3v2w](https://github.com/triggerdotdev/trigger.dev/security/advisories/GHSA-fj2x-mqqp-3v2w).
- Hatchet:
  - "Cross-tenant write/DoS to other tenants' workers via Dispatcher gRPC UpsertWorkerLabels and Unsubscribe" (CVE-2026-54746) — [GHSA-8x7x-83cf-c3pg](https://github.com/hatchet-dev/hatchet/security/advisories/GHSA-8x7x-83cf-c3pg).
  - "Cross-Tenant Durable Task Event Log Disclosure" (CVE-2026-63342) — [GHSA-g26x-m427-f48f](https://github.com/hatchet-dev/hatchet/security/advisories/GHSA-g26x-m427-f48f).
  - Cross-tenant disclosure in `listTasksByDAGIds` (CVE-2026-42572) — [GHSA-55gc-6fmc-fpx9](https://github.com/hatchet-dev/hatchet/security/advisories/GHSA-55gc-6fmc-fpx9).
- OpenMeter:
  - "SQL injection through customer usageAttribution subjectKeys (and key) into the shared ClickHouse meter/event" queries (CVE-2026-18801, critical, 2026-08-05) — [GHSA-m2fw-9wxq-jgf5](https://github.com/openmeterio/openmeter/security/advisories/GHSA-m2fw-9wxq-jgf5).
  - SQL injection through meter creation (CVE-2026-8462) — [GHSA-wc3v-3457-c8cm](https://github.com/openmeterio/openmeter/security/advisories/GHSA-wc3v-3457-c8cm).
- Zitadel published at least 12 advisories between 2026-07-29 and 2026-09-28. Examples:
  - "Unauthenticated account takeover via passkey enrollment on Login V1 and Login V2" (critical) — [GHSA-45f2-5q3r-xgg6](https://github.com/zitadel/zitadel/security/advisories/GHSA-45f2-5q3r-xgg6).
  - Cross-organization account takeover through passkey enrollment codes (critical) — [GHSA-pq2q-2c6r-75c4](https://github.com/zitadel/zitadel/security/advisories/GHSA-pq2q-2c6r-75c4).
  - "Unauthenticated Account Takeover Through Predictable Session IDs" — [GHSA-jh92-5mrj-p2w2](https://github.com/zitadel/zitadel/security/advisories/GHSA-jh92-5mrj-p2w2).
- Ory Kratos: "SQL injection via forged pagination tokens" (CVE-2026-33503) — [GHSA-hgx2-28f8-6g2r](https://github.com/ory/kratos/security/advisories/GHSA-hgx2-28f8-6g2r).
- Windmill:
  - "Under nsjail security sandboxing, DuckDB scripts can escape the fs sandbox and poison shared worker caches" (2026-09-10) — [GHSA-2jfj-x8j7-7mfh](https://github.com/windmill-labs/windmill/security/advisories/GHSA-2jfj-x8j7-7mfh).
  - "SUPERADMIN_SECRET (rarely used) can be accessed publicly on versions < 1.603.3 using RCE" (CVE-2026-29059) — [GHSA-24fr-44f8-fqwg](https://github.com/windmill-labs/windmill/security/advisories/GHSA-24fr-44f8-fqwg).

#### Isolation-layer escapes
- runc (November 2025): three container-escape flaws.
  - CVE-2025-31133: a race in the `/dev/null` bind mount used to mask paths.
  - CVE-2025-52565: a `/dev/console` bind-mount race.
  - CVE-2025-52881: procfs write redirects that can bypass LSM protections.
  - All let a container gain write access to sensitive host `/proc` files. Fixed in runc 1.2.8, 1.3.3 and 1.4.0-rc.3.
  — [BleepingComputer](https://bleepingcomputer.com/news/security/dangerous-runc-flaws-could-allow-hackers-to-escape-docker-containers), [oss-security disclosure](https://openwall.com/lists/oss-security/2025/11/05/3), [Sysdig](https://sysdig.com/blog/runc-container-escape-vulnerabilities)
- Firecracker (2026):
  - "Arbitrary Host File Overwrite via Symlink in Firecracker Jailer" (CVE-2026-1386) — [GHSA-36j2-f825-qvgc](https://github.com/firecracker-microvm/firecracker/security/advisories/GHSA-36j2-f825-qvgc).
  - "Out-of-bounds Write in Firecracker virtio-pci Transport" (CVE-2026-5747, high) — [GHSA-776c-mpj7-jm3r](https://github.com/firecracker-microvm/firecracker/security/advisories/GHSA-776c-mpj7-jm3r).
- Kata Containers (2026):
  - "runtime-rs: Guest-root to host-root escape via virtiofs" (CVE-2026-47243, critical) — [GHSA-2gv2-cffp-j227](https://github.com/kata-containers/kata-containers/security/advisories/GHSA-2gv2-cffp-j227).
  - "Dragonball: missing length validation in virtio-blk allows guest-to-host VM escape" (CVE-2026-93417) — [GHSA-fgm4-mv68-h344](https://github.com/kata-containers/kata-containers/security/advisories/GHSA-fgm4-mv68-h344).
  - "VM Escape via virtiofsd Argument Injection through Default-Enabled Pod Annotations" (CVE-2026-44210) — [GHSA-rr59-xxvx-96qr](https://github.com/kata-containers/kata-containers/security/advisories/GHSA-rr59-xxvx-96qr).
  - "Untrusted pod annotation bind-mounts any host path into the Kata guest, bypassing the operator allowlist" (CVE-2026-86072) — [GHSA-7fhf-v3p3-rp56](https://github.com/kata-containers/kata-containers/security/advisories/GHSA-7fhf-v3p3-rp56).
  - "Config Path Annotation Arbitrary File Loading" (CVE-2026-50540, critical) — [GHSA-mp2j-xm59-qfgw](https://github.com/kata-containers/kata-containers/security/advisories/GHSA-mp2j-xm59-qfgw).
- Cloud Hypervisor:
  - "Use-after-free in virtio-block Async I/O Completion" (CVE-2026-45782) — [GHSA-f47p-p25q-83rh](https://github.com/cloud-hypervisor/cloud-hypervisor/security/advisories/GHSA-f47p-p25q-83rh).
  - "Host File Exfiltration via QCOW Backing File Abuse" (CVE-2026-27211) — [GHSA-jmr4-g2hv-mjj6](https://github.com/cloud-hypervisor/cloud-hypervisor/security/advisories/GHSA-jmr4-g2hv-mjj6).
- microsandbox (eight advisories, 2026-06-23 to 2026-10-09):
  - "Guest-controlled symlinks in msb copy allow arbitrary file write on the host" (CVE-2026-103034) — [GHSA-4vq3-cjpp-v7fg](https://github.com/superradcompany/microsandbox/security/advisories/GHSA-4vq3-cjpp-v7fg).
  - "NAT64 addresses can bypass network destination group policy" — [GHSA-cg9x-2vfm-3j58](https://github.com/superradcompany/microsandbox/security/advisories/GHSA-cg9x-2vfm-3j58).
  - "Egress allowlist bypass: Host/SNI check fails open on malformed HTTP/1 request lines" — [GHSA-gj24-29j2-3x78](https://github.com/superradcompany/microsandbox/security/advisories/GHSA-gj24-29j2-3x78).
  - "Microsandbox hostname allowlist bypass via mismatched HTTP Host/:authority" — [GHSA-3mpp-52gc-935r](https://github.com/superradcompany/microsandbox/security/advisories/GHSA-3mpp-52gc-935r).
  - A domain deny rule bypassed via direct-IP TLS — [GHSA-xgg5-hjwq-2gw8](https://github.com/superradcompany/microsandbox/security/advisories/GHSA-xgg5-hjwq-2gw8).

#### Architecture and scaling lessons reported by maintainers
- Gitpod/Ona, after six years on Kubernetes with about 1.5 million users (2024-10-31):
  - CPU noisy neighbours, and memory overbooking that was "nearly impossible" until swap arrived in Kubernetes 1.22.
  - Local SSDs tied data to nodes, and PVC attach and detach timing was unpredictable.
  - Workspace images over 10 GB set a floor on startup time, despite pre-pulls and ballast pods.
  - Root and Docker inside environments needed user-namespace workarounds (shiftfs, a masked `/proc`, seccomp notify, custom runc handling).
  - Per-environment Services and DNS were unreliable at scale.
  - Micro-VMs (Firecracker, Cloud Hypervisor, QEMU) were evaluated but judged too costly in overhead, image conversion, GPU and virtiofs limits, although suspend and resume were valued.
  — [Ona blog](https://ona.com/stories/we-are-leaving-kubernetes)
- E2B's runtime separates "control plane and data plane", resumes snapshots instead of booting, and enforces egress with "a per-sandbox nftables egress firewall and SNI/Host-inspecting domain allow and deny lists" — [e2b-dev/runtime](https://github.com/e2b-dev/runtime).
- Firecrawl warns that its self-host quickstart "disables API authentication and is not a production architecture", and that "No single .env switch makes the stack production-ready" — [Firecrawl self-host docs](https://docs.firecrawl.dev/contributing/self-host).
- Crawl4AI's server defaults to `trusted_hosts: ["*"]` and in-memory rate-limit storage, and tells operators to "Always enable security in production" — [Crawl4AI Docker README](https://github.com/unclecode/crawl4ai/blob/HEAD/deploy/docker/README.md).
- Kernel warns that its Unikraft deployment URL "is public, meaning anyone can access the remote GUI if they have the url" — [kernel-images README](https://github.com/kernel/kernel-images).
- Trigger.dev's CRIU checkpoints are available in its Cloud only, not when self-hosted — [Trigger.dev self-hosting](https://github.com/triggerdotdev/trigger.dev/blob/HEAD/docs/self-hosting/overview.mdx).
- Firecracker diff snapshots are "still in developer preview" — [snapshot-support.md](https://github.com/firecracker-microvm/firecracker/blob/HEAD/docs/snapshotting/snapshot-support.md).

#### Maintenance burden, abandonment and license drift
- Daytona moved core development to a private codebase in June 2026 and archived its 71.5k-star repository — [daytona README](https://github.com/daytonaio/daytona).
- Gitpod was renamed Ona, and "We no longer recommend this version of Gitpod" — [gitpod README](https://github.com/gitpod-io/gitpod).
- Browserbase archived its MCP server (as a "reference implementation"), open-operator and stagehand-python — [github.com/browserbase](https://github.com/browserbase). Bytebot is archived — [bytebot](https://github.com/bytebot-ai/bytebot).
- "Scrapoxy has been discontinued." — [scrapoxy](https://github.com/scrapoxy/scrapoxy). Selenoid, Zalenium, scrapy-cluster and Weave Ignite are archived (see the catalogue and the table in question 3).
- These repos had no commits on the default branch since 2026-07-14 (GitHub API, 2026-10-12):
  - Crawlab: last stable release 2023.
  - Gerapy: last release 2023.
  - ScrapydWeb.
  - SpiderKeeper: last push 2023.
  - Splash: last tag 2020.
  - Ulixee.
  - Jina Reader.
  - Browser Use's web-ui: 271 open issues.
  - HyperAgent.
  - The catalogue in question 1 links each repository.
- Steel Browser is still "public beta", with 10 commits since 2026-07-14 — [steel-browser](https://github.com/steel-dev/steel-browser).
- License drift around building blocks:
  - Nomad is BUSL-1.1 from version 1.7.0, now with IBM as licensor — [Nomad LICENSE](https://github.com/hashicorp/nomad/blob/HEAD/LICENSE).
  - OpenFaaS CE requires a license for commercial use — [OpenFaaS LICENSE](https://github.com/openfaas/faas/blob/HEAD/LICENSE).
  - Airbyte's platform is ELv2 — [Airbyte LICENSE](https://github.com/airbytehq/airbyte/blob/HEAD/LICENSE).
  - Zitadel is AGPL-3.0-only — [Zitadel LICENSING.md](https://github.com/zitadel/zitadel/blob/HEAD/LICENSING.md).
  - Browserless is SSPL-1.0 or commercial — [Browserless LICENSE](https://github.com/browserless/browserless/blob/HEAD/LICENSE).

### Inferences
- **Treat every user-controlled value that reaches the engine as hostile when it runs on behalf of a cloud user.** That covers Chromium launch arguments, download paths, JSON Schema `$ref`, git URLs or diffs, PDF fetches, robots.txt fetches, link previews, proxy settings, webhooks and custom behaviours. The Crawl4AI, Firecrawl and Browsertrix advisories map one-to-one onto features Foxhound and Collie will also expose through the Crawl API and Recipes.
- **Enforce egress outside the engine, at the VM boundary**, as E2B does with nftables and SNI. The single-tenant engine should not be its own last line of defence. The deny list has to cover:
  - all non-global ranges, including 198.18.0.0/15 (the Trigger.dev bypass), CGNAT 100.64.0.0/10 (changedetection.io), and NAT64 and IPv6 equivalents (microsandbox);
  - cloud metadata addresses;
  - redirects, which must be re-checked.

  This is an inference from the bypass patterns above.
- **Validate Origin and Host on every WebSocket and HTTP listener.** That includes the engine's JSON-RPC, the MCP server, the local Console, the raw-CDP opt-in and the Browser host. Lightpanda's CDP and MCP advisories are exactly the failure mode of a local automation server.
- **Write per-endpoint tenant-authorization tests and use parameterized metering queries.** The 2026 Daytona, Trigger.dev, Hatchet and OpenMeter advisories show IDOR and cross-tenant bugs are the main risk of a new multi-tenant control plane, and the Goodfetch Cloud Console and APIs are such a plane.
- **Isolation needs defence in depth.** VM boundaries had escapes too (Kata, Cloud Hypervisor, the Firecracker jailer). The layers are:
  - a microVM with the jailer and seccomp, and a minimal device model;
  - no annotation-driven runtime configuration (several Kata escapes came through pod annotations);
  - no shared caches across tenants (Windmill);
  - a fast patch cadence for runc and containerd on any container path.
- **Kubernetes:** Gitpod's experience suggests long-lived, interactive hosted-browser sessions (spiky CPU, large images, stateful profiles) can hit the same problems if run as plain pods. Snapshot-and-standby VMs (Kernel, E2B) answer exactly those startup and density problems.
- **Sustainability:** put foundation-governed or widely adopted permissive projects (Firecracker, gVisor, Kata, KEDA, Karpenter, Keycloak, Envoy, Temporal) on critical paths. Treat vendor-led open-source platform repos as references that may be archived, go private or change license.

### Gaps
- Hacker News threads, conference talks and blog post-mortems were not gathered because the web-search budget was exhausted. These would include Browserless's scaling posts, E2B or Fly.io Firecracker write-ups, Browserbase or Steel incident reports, and Firecrawl self-hosting complaints.
- No public post-mortem of a cross-tenant breach caused by a browser or scraping cloud was found; none was searched for after the budget ran out.
- Several repositories returned no repository-level advisories: Steel Browser, Browserless, Skyvern, Kernel images, the E2B runtime, gVisor, agent-sandbox, Notte, Stagehand, Crawlab, ScrapydWeb, Scrapyd, Lago and proxy-chain. That does not prove they have no vulnerabilities. A query against GitHub's global advisory database also returned no rows, which suggests a query problem rather than a clean record.
- CVE identifiers are reported exactly as GitHub lists them; some advisories have no CVE yet.
