# License and open-core models of comparable projects — October 10, 2026

Research for issue #66 (map #61). **Evidence, not a decision, and not legal advice.** The current R&D proposal names Apache-2.0 for the framework ([proposition](../proposals/proposition-automan.md), [R&D README, row 18](../README.md)); this note collects what comparable projects did and what happened to them, so that the choice can be made on evidence.

## Summary

- **Frameworks that feed a cloud are mostly permissive.** Crawlee (Apache-2.0), Spider (MIT), Stagehand (MIT), Browser Use (MIT), Scrapy (BSD-3-Clause) and Playwright (Apache-2.0) keep the library fully open. The cloud sells operations: fleet, proxies, anti-bot or stealth, CAPTCHA, hosted models and caches.
- **The AGPL group are product engines, not embeddable libraries.** Firecrawl (relicensed Apache-2.0 → AGPL-3.0 three weeks after its first commit, in May 2024), Skyvern and Lightpanda use AGPL-3.0. Firecrawl moved its SDKs to MIT in July 2024. Firecrawl and Skyvern keep anti-bot closed. Lightpanda alone uses a CLA, and it sells proprietary licenses.
- **Crawl4AI is "Apache-2.0 plus".** Since v0.5.0 (2025-02-21), its LICENSE has had an extra attribution requirement covering "public uses". PyPI still declares plain `Apache-2.0`, and the README says attribution is both "recommended" and something you "must include".
- **Relicensing an established project to source-available caused forks when other companies depended on it**: OpenTofu (Terraform), OpenSearch (Elasticsearch) and Valkey (Redis). Elastic (August 2024) and Redis (May 2025) later added AGPL-3.0 back. RedMonk found no notable change in revenue growth after such moves (August 2024). CHAOSS found that the forks have more organizational diversity than the originals.
- **Sentry started source-available and stayed there.** It moved from BSD-3 to BSL in 2019 and then to FSL in 2023. FSL is a two-year non-compete that converts to Apache-2.0 or MIT. This pass did not look for adoption data on Sentry.
- **Relicensing later needs contributor rights.** Lightpanda, Elastic, HashiCorp and Microsoft use a CLA; Sentry uses a PR-template grant. Most AI-scraping projects use neither, so contributions arrive under the repository license (GitHub's inbound = outbound rule). OpenTofu and OpenSearch use a DCO, which does not grant relicensing rights.
- **The recommended dependencies are compatible with Apache-2.0, MIT, AGPL-3.0 and FSL/BSL**, with two conditions. Lightpanda (AGPL-3.0-only) must stay a separate process unless Automan is AGPL or buys a Lightpanda license. Camoufox (MPL-2.0, with an LGPL-3.0-or-later component) and Patchright (Apache-2.0) must stay separate binaries or plugins. Two crates are Apache-2.0-only (htmd, wreq), so an "MIT" Automan still ships Apache-2.0 obligations.

## Method and scope

- **Primary sources.** License files and their commit history were read through the GitHub API. Registry license metadata comes from crates.io, PyPI and npm, all read on 2026-10-10. Contributor terms come from CONTRIBUTING files, CLA documents and CLA workflows. Change announcements come from the vendors' own blogs. SPDX ids and OSI flags were checked against the [SPDX License List 3.29.0](https://github.com/spdx/license-list-data) (2026-09-16).
- **Secondary sources.** Adoption and outcome figures come from the CHAOSS paper ([Foster 2024](https://arxiv.org/abs/2411.04739)), [RedMonk](https://redmonk.com/rstephens/2024/08/26/software-licensing-changes-and-their-impact-on-financial-outcomes/) and foundation press releases. Vendor statements are reported as claims.
- **Limits.** Exa search credits ran out at the start of this pass, so web discovery used other search and fetch tools; primary files were read directly. Star counts are weak proxies for adoption. "No CLA found" means none in the repository's contributing files or workflows; an off-repo agreement is not excluded. No exhaustive search was made for third parties hosting AGPL projects. None of this is legal advice: a lawyer should review the final license, the CLA or DCO choice and the treatment of AGPL, MPL and LGPL dependencies.

## 1. Comparable projects

Stars and last push are from the GitHub API on 2026-10-10.

| Project | License (SPDX, extras) | Open vs cloud-only | Contributor terms | License history and notable facts |
|---|---|---|---|---|
| **Crawlee / Apify** (26.1k stars JS, 9.6k Python) | `Apache-2.0`: [crawlee](https://github.com/apify/crawlee/blob/master/LICENSE.md), [crawlee-python](https://github.com/apify/crawlee-python), Apify SDK JS/Python, Apify CLI, [Impit](https://github.com/apify/impit). npm `crawlee` 3.18.2 and PyPI `crawlee` 1.10.4 declare `Apache-2.0`. | The library "is open-source and runs anywhere". The Apify platform (Actor runtime, storage, proxy, Store) is the paid service ([README](https://github.com/apify/crawlee#usage-on-the-apify-platform)). | No CLA or DCO in [CONTRIBUTING.md](https://github.com/apify/crawlee/blob/master/CONTRIBUTING.md). | No license change found. The file was renamed to LICENSE.md on 2018-09-27. Apify also runs third-party OSS: it [wraps Scrapy projects as Actors](https://apify.com/run-scrapy-in-cloud). |
| **Spider / spider.cloud** (2.8k stars) | `MIT`. All 2,227 published versions of crate `spider` are MIT, from 1.0.3 (2018-02-05) to 2.53.9 (2026-09-05) (crates.io). Chromey is `MIT OR Apache-2.0`. | "The core crawling engine is identical". The cloud adds per-request proxy rotation with block detection, a "full stealth engine", a "custom Rust browser", per-site Markdown tuning and AI extraction. "The open-source library is MIT-licensed and always will be." ([open source vs cloud](https://spider.cloud/open-source-vs-cloud/)) | [CONTRIBUTING.md](https://github.com/spider-rs/spider/blob/main/CONTRIBUTING.md): "By contributing, you agree that your contributions will be licensed under the MIT License." No CLA. | MIT since the 2018 initial commit. |
| **Stagehand / Browserbase** (25.6k stars) | `MIT`. npm `@browserbasehq/stagehand` 4.2.0 and PyPI `stagehand` 4.2.0 are MIT. LICENSE created 2024-10-29. | The framework runs on a local browser. Browserbase adds hosted browsers, the Model Gateway and server-side caching of repeated actions ([README, "Run it on Browserbase"](https://github.com/browserbase/stagehand#run-it-on-browserbase)). | No CLA or DCO in [CONTRIBUTING.md](https://github.com/browserbase/stagehand/blob/main/CONTRIBUTING.md). | No change found. The separate `stagehand-python` repo is archived; Python now ships from PyPI `stagehand`. |
| **Firecrawl** (190k stars) | `AGPL-3.0` at the root. The SDKs and "some UI components" are `MIT` ([README](https://github.com/firecrawl/firecrawl#license)). PyPI `firecrawl-py` and npm `@mendable/firecrawl-js` are MIT. | Self-hosting includes scrape, crawl, map and search with fetch and Playwright. **Fire-engine and "its advanced anti-bot behavior" are "not included".** Screenshots and page actions require Fire-engine. Agent, Browser, interact and the specialized formats need Firecrawl Cloud ([self-host guide](https://docs.firecrawl.dev/contributing/self-host)). | No CLA or DCO in CONTRIBUTING.md. | `Apache-2.0` at the initial commit (2024-04-15). **Switched to AGPL-3.0 on 2024-05-07** ([commit dc97757](https://github.com/firecrawl/firecrawl/commit/dc977577c9)). **SDKs moved to MIT on 2024-07-08** ([commit afb49e2](https://github.com/firecrawl/firecrawl/commit/afb49e21e7)). Fork: Trieve's [firecrawl-simple](https://github.com/devflowinc/firecrawl-simple) (AGPL-3.0, created 2024-09-19, last push 2025-05-23). It removes billing and AI and calls the closed Fire-engine "the biggest deal breaker". |
| **Browser Use** (117k stars) | `MIT`. LICENSE added 2024-11-05; PyPI classifier MIT. | The OSS agent is a Python/TS library. The cloud offers a hosted agent API and cloud browsers "with stealth, CAPTCHA solving and residential proxies", plus the BU2 model ([README](https://github.com/browser-use/browser-use)). | No CLA; CONTRIBUTING links the docs guide. | No change found. **Competitor hosting:** Hyperbrowser sells a hosted browser-use agent API ([docs](https://docs.hyperbrowser.ai/agents/browser-use)). |
| **Crawl4AI** (85.1k stars) | Apache-2.0 text plus an appended **"Attribution Requirement"**: "All distributions, publications, or public uses… must include" a fixed credit line, in the help output for CLIs and an About page for web apps ([LICENSE](https://github.com/unclecode/crawl4ai/blob/main/LICENSE)). No SPDX id covers the combination. PyPI `crawl4ai` 0.9.4 declares only `Apache-2.0`. | The library and self-hosted Docker server are free. Crawl4AI Cloud (`api.crawl4ai.com`) adds bot-wall handling, `/search`, `/answer` and extraction without your own LLM key ([README](https://github.com/unclecode/crawl4ai#license--attribution)). | No CLA or DCO in CONTRIBUTING.md. | The attribution clause was **added on 2025-02-21 with v0.5.0** ([commit 367cd71](https://github.com/unclecode/crawl4ai/commit/367cd71db9)). The README is internally inconsistent: attribution is "recommended", then "you must include". |
| **Skyvern** (23.2k stars) | `AGPL-3.0` since the initial commit (2024-02-28). PyPI `skyvern` has no license metadata. | "All of the core logic… is available… with the exception of anti-bot measures available in our managed cloud offering". The cloud bundles "anti-bot detection mechanisms, proxy network, and CAPTCHA solvers" ([README](https://github.com/Skyvern-AI/skyvern)). | CONTRIBUTING.md: contributors agree their content "may be provided under the project license". No CLA. | No change found. The README invites licensing questions by e-mail. |
| **Lightpanda** (36.2k stars) | `AGPL-3.0-only` ([LICENSING.md](https://github.com/lightpanda-io/browser/blob/main/LICENSING.md)), added 2024-05-13. The npm wrapper `@lightpanda/browser` 1.6.0 is `Apache-2.0`. | Same engine locally and in the cloud. Lightpanda Cloud adds the console, usage plans, a managed proxy pool and an HTTP API that returns HTML or Markdown ([local vs cloud](https://lightpanda.io/docs/core-concepts/local-vs-cloud)). "The core… will always remain open source". For "proprietary licensing, or enterprise features such as multi-context tabs and sandboxing", contact sales ([quickstart](https://lightpanda.io/docs/quickstart)). | **CLA**, based on the ASF CLA, signed through CLA Assistant Lite ([CLA.md](https://github.com/lightpanda-io/browser/blob/main/CLA.md), [workflow](https://github.com/lightpanda-io/browser/blob/main/.github/workflows/cla.yml)). It grants Lightpanda a copyright license that includes the right to sublicense. | No change found. This is the only comparable project here with a dual-licensing pattern: AGPL plus proprietary on request, enabled by the CLA. |
| **Playwright** (97.4k stars) | `Apache-2.0`. npm `playwright` 1.64.0 and PyPI 1.63.0. | Fully open. Microsoft's hosted service "Microsoft Playwright Testing" was retired on 2026-03-08 in favour of Playwright Workspaces in Azure App Testing ([retirement notice](https://learn.microsoft.com/en-us/azure/playwright-testing/monitor-playwright-testing-reference), [Workspaces](https://learn.microsoft.com/azure/app-testing/playwright-workspaces/overview-what-is-microsoft-playwright-workspaces)). | **Microsoft CLA**, enforced by a bot ([CONTRIBUTING.md](https://github.com/microsoft/playwright/blob/main/CONTRIBUTING.md)). | No change found. |
| **Scrapy** (64.7k stars) | `BSD-3-Clause`. PyPI `scrapy` 2.19.0 `license_expression`. | Fully open. Zyte maintains it and sells Scrapy Cloud ([scrapy.org deploy](https://www.scrapy.org/deploy)). Competitor Apify also runs Scrapy spiders ([Apify](https://apify.com/run-scrapy-in-cloud)). | No CLA or DCO in [docs/contributing.rst](https://github.com/scrapy/scrapy/blob/master/docs/contributing.rst). | No change found. Scrapy is the clearest case of a vendor's framework being hosted by a direct competitor. |
| *Adjacent: Browserless* (13.8k stars) | `SSPL-1.0 OR` Browserless commercial license ([LICENSE](https://github.com/browserless/browserless/blob/main/LICENSE)). | A closed-source commercial use needs the paid license. | Not inspected. | `GPL-3.0-or-later OR commercial` (2019) → `SSPL-1.0 OR commercial` with v2.0.0 (2023-10-30). |
| *Adjacent: Steel Browser* (7.8k stars) | `Apache-2.0` ([repo](https://github.com/steel-dev/steel-browser)). | Self-hostable server with paid cloud operation ([commercial landscape](2026-10-07-commercial-landscape.md)). | Not inspected. | — |

Where no CLA or DCO is required, contributions arrive under the repository license. [GitHub's Terms of Service, D.6](https://docs.github.com/en/site-policy/github-terms/github-terms-of-service) say: "Whenever you add Content to a repository containing notice of a license, you license that Content under the same terms". A separate agreement such as a CLA takes precedence.

### Observed open-core boundary

| Cloud-only element | Where observed |
|---|---|
| Anti-bot, stealth, fingerprinting | Firecrawl (Fire-engine), Skyvern, Spider ("full stealth engine"), Browser Use, Crawl4AI ("bot walls") |
| Managed proxies | Spider, Browser Use (residential), Skyvern, Lightpanda (proxy pool) |
| CAPTCHA solving | Browser Use, Skyvern |
| Hosted models, caches and AI extraction | Stagehand (Model Gateway, server-side cache), Browser Use (BU2), Spider (AI extraction), Crawl4AI (`/extract` without your own key) |
| Fleet, scaling, console, billing | All vendors |
| Simple HTTP API in front of the engine | Lightpanda, Crawl4AI, Firecrawl |
| Enterprise engine features under a separate license | Lightpanda (multi-context tabs, sandboxing) |

In this sample, the boundary is almost always **operations and the access/anti-bot layer**, not the crawling or automation API. ADR-002 proposes the same split, keeping stealth as an opt-in catalog with an open source/cloud split ([ADR-002](../adr/0002-access-identity-stealth.md)).

## 2. Source-available moves and what followed

### Sentry: BSD-3 → BSL (2019) → FSL (2023)

- **2019-11-06: BSD-3-Clause → BSL.** "You cannot offer a commercial version of Sentry's service", and the code becomes Apache-2.0 after 36 months. Sentry itself calls the BSL "formally _not_ an open-source license" ([Re-Licensing Sentry](https://blog.sentry.io/relicensing-sentry/)).
- **2023-11-17: Functional Source License** ([announcement](https://blog.sentry.io/introducing-the-functional-source-license-freedom-without-free-riding/); LICENSE.md commits: [FSL-1.0](https://github.com/getsentry/sentry/commit/d4d89e2fa9), then [FSL-1.1 on 2024-02-27](https://github.com/getsentry/sentry/commit/1760ab3c6b)). The current file is "FSL-1.1-Apache-2.0"; its SPDX id is `FSL-1.1-ALv2`, which is not OSI-approved.
  - A "Competing Use" is a commercial product or service that substitutes for the Software or offers "substantially similar functionality".
  - Internal use, non-commercial education and research, and professional services are permitted.
  - The code converts to Apache-2.0 on the second anniversary.
  - Sentry's reasons for leaving the BSL: its four-year default is "too long", and the Additional Use Grant makes every BSL "a different license". Sentry chose an Apache-2.0 or MIT future license because "Copyleft doesn't fit our values".
- **2024: Fair Source.** Sentry launched the Fair Source initiative ([fair.io](https://fair.io/about/)). Its criteria are a public repository, use "with minimal restrictions to protect the producer's business model", and delayed open-source publication.
- **Contributor terms.** Since 2022-05-10 the PR template carries a "Legal Boilerplate": the contributor confirms that Sentry "can use, modify, copy, and redistribute my contributions, under Sentry's choice of terms" ([template](https://github.com/getsentry/sentry/blob/master/.github/PULL_REQUEST_TEMPLATE.md), [PR #34182](https://github.com/getsentry/sentry/pull/34182)). This is a light alternative to a CLA.
- **Outcome.** This pass did not look for forks or adoption data on Sentry.

### HashiCorp Terraform: MPL-2.0 → BUSL-1.1 (2023), and the OpenTofu fork

- **2023-08-10: relicensing announced.** "All future releases" moved from MPL-2.0 to BUSL-1.1; "APIs, SDKs, and almost all other libraries will remain MPL 2.0". The stated reason: "Open source has reduced the barrier to copying innovation and selling it through existing distribution channels" ([HashiCorp](https://www.hashicorp.com/en/blog/hashicorp-adopts-business-source-license); [LICENSE commit](https://github.com/hashicorp/terraform/commit/b145fbcaad)).
- **License terms.** The Additional Use Grant forbids offering the work "on a hosted or embedded basis" in a paid "competitive offering". It converts to MPL-2.0 four years after each release ([LICENSE](https://github.com/hashicorp/terraform/blob/main/LICENSE)). Contributors sign a **CLA** ([CONTRIBUTING](https://github.com/hashicorp/terraform/blob/main/.github/CONTRIBUTING.md)).
- **IBM.** IBM completed the acquisition on 2025-02-27 ([IBM](https://newsroom.ibm.com/2025-02-27-ibm-completes-acquisition-of-hashicorp,-creates-comprehensive,-end-to-end-hybrid-cloud-platform)). The LICENSE names IBM as licensor since a [2026-03-11 commit](https://github.com/hashicorp/terraform/commit/4eba7c0596).
- **The fork.**
  - Two weeks after the change, OpenTofu reported over 100 companies, 10 projects and 400 individuals pledging support, and four companies pledging 14 full-time engineers, against "about 5 FTEs" maintaining Terraform ([fork announcement](https://opentofu.org/blog/opentofu-announces-fork-of-terraform/), 2023-08-25).
  - The Linux Foundation launched OpenTofu on 2023-09-20 with "140+ organizations and 600+ individuals" ([LF](https://www.linuxfoundation.org/press/announcing-opentofu)).
  - First stable release, v1.6.0: 2024-01-10 (GitHub release).
  - HashiCorp sent a cease-and-desist over the `removed` block on 2024-04-03. OpenTofu answered that the code came from older MPL-2.0 code ([response](https://opentofu.org/blog/our-response-to-hashicorps-cease-and-desist)).
  - CNCF accepted OpenTofu at Sandbox level on 2025-04-23 ([CNCF](https://www.cncf.io/projects/opentofu/)).
  - OpenTofu reported 9.8 million downloads from GitHub releases alone at its 1.10.0 release (2025-06-23, [blog](https://opentofu.org/blog/opentofu-1-10-0)). The latest release is v1.13.1 (2026-10-01). OpenTofu uses a **DCO** ([CONTRIBUTING](https://github.com/opentofu/opentofu/blob/main/CONTRIBUTING.md)).
- **Contributor data** ([Foster 2024](https://arxiv.org/abs/2411.04739), CHAOSS, Tables 3–4):
  - In the year before and the year after the change, only two non-HashiCorp people made five or more commits to Terraform, so the relicensing had "no substantial impact on the contributor community". The impact fell on users.
  - In OpenTofu's first year, 31 people from 11 organizations made five or more commits. Spacelift alone made 55% of additions. "No contributors to OpenTofu… had previously contributed to Terraform."

### Elastic: Apache-2.0 → SSPL/ELv2 (2021) → AGPL added (2024)

- **2021-01-14: relicensing announced.** Starting with 7.11, the Apache-2.0 code of Elasticsearch and Kibana became dual SSPL / Elastic License, "restricting cloud service providers from offering Elasticsearch and Kibana as a service without contributing back" ([Elastic](https://www.elastic.co/blog/licensing-change)).
- **The fork.** AWS announced an Apache-2.0 fork on 2021-01-21 ([AWS](https://aws.amazon.com/blogs/opensource/stepping-up-for-a-truly-open-source-elasticsearch)). OpenSearch forked from 7.10.2 on 2021-04-12 ([Foster 2024](https://arxiv.org/abs/2411.04739)). On 2024-09-16 AWS moved it to the Linux Foundation's OpenSearch Software Foundation, which cites "more than 700 million software downloads" ([LF](https://www.linuxfoundation.org/press/linux-foundation-announces-opensearch-software-foundation-to-foster-open-collaboration-in-search-and-analytics)). OpenSearch uses a **DCO**.
- **2024-08-29: AGPL added** "as another license option next to ELv2 and SSPL" ([Elastic](https://www.elastic.co/blog/elasticsearch-is-open-source-again); [LICENSE.txt commit, 2024-09-13](https://github.com/elastic/elasticsearch/commit/a59c182f9f)).
  - The repository default is now a triple license: `AGPL-3.0-only`, `SSPL-1.0` and `Elastic-2.0`; code licensed solely under Elastic-2.0 lives only in x-pack ([LICENSE.txt](https://github.com/elastic/elasticsearch/blob/main/LICENSE.txt)). SSPL-1.0 and Elastic-2.0 are not OSI-approved (SPDX 3.29.0).
  - Elastic's own assessment: "while it was painful, it worked". Amazon "is fully invested in their fork", market confusion "has been (mostly) resolved", and the AWS partnership "is stronger than ever". Elastic also asserts that AGPL "doesn't affect usage or popularity". Both are vendor claims.
- **Contributor terms.** A **CLA** that gives Elastic "the right to distribute your code without restriction" ([CONTRIBUTING](https://github.com/elastic/elasticsearch/blob/main/CONTRIBUTING.md)).
- **Contributor data** ([Foster 2024](https://arxiv.org/abs/2411.04739), Tables 1–2):
  - Elastic employees made over 95% of the lines added and deleted before and after the change. Non-Elastic people with ten or more commits went from 3, to 2, to 1.
  - OpenSearch: Amazon made 80% of additions in its first year, falling to 63% in the year before the LF move.

### Redis (supplementary, same pattern)

- **2024-03-20: relicensing.** Redis moved from BSD-3 to RSALv2/SSPLv1; Valkey forked under the Linux Foundation ([Foster 2024](https://arxiv.org/abs/2411.04739)).
- **2025-05-01: AGPL added.** Redis added AGPLv3 starting with Redis 8. Its CEO: the change "achieved our goal—AWS and Google now maintain their own fork—but… hurt our relationship with the Redis community" ([Redis](https://redis.io/blog/agplv3/)).
- **Contributor data.** Within six months of the relicense, every external contributor with more than five commits in the prior year (from Amazon, Alibaba, Tencent, Huawei and Ericsson) had stopped contributing ([Foster 2024](https://arxiv.org/abs/2411.04739)).

### Cross-case evidence

- **Financial effect.** RedMonk studied MongoDB, Elastic, HashiCorp and Confluent: "we don't see a notable change in the rate of growth" of revenue after relicensing. Valuation outcomes were mixed, and causality is not established ([RedMonk, 2024-08-26](https://redmonk.com/rstephens/2024/08/26/software-licensing-changes-and-their-impact-on-financial-outcomes/)).
- **Forks and governance.** Forks created under a neutral foundation show more organizational diversity than the vendor-run originals. Relicensing is only possible when the vendor controls governance and holds the necessary rights, for example through a CLA ([Foster 2024](https://arxiv.org/abs/2411.04739)).
- **When forks happen.** The forks that formed had a coalition of companies whose business depended on the code: Spacelift, env0, Scalr and Gruntwork for Terraform; AWS for Elasticsearch; AWS, Google and others for Redis. Where the original project had almost no outside contributors (Terraform, Elasticsearch), the relicense barely changed the contributor base. Its cost fell on users and ecosystem companies.

### Evidence on competitor hosting and adoption

- **Permissive code is hosted by competitors.** Hyperbrowser sells a hosted Browser Use agent (MIT). Apify runs Scrapy projects (BSD), which belong to its competitor Zyte. Before 2021, AWS sold hosted Elasticsearch (Apache-2.0), which is what triggered Elastic's change.
- **AGPL code was forked, not hosted.** This pass found no third party selling hosted Firecrawl, Skyvern or Lightpanda; the search was not exhaustive. The visible AGPL fork, firecrawl-simple, serves Trieve's own product and appears inactive since May 2025.
- **Adoption.** The license does not visibly cap stars in this market. The AGPL Firecrawl has the most stars in the sample (190k), and AGPL Lightpanda (36k) out-stars the MIT Stagehand (25.6k) and the Apache Crawlee (26.1k). Stars mix product appeal, timing and marketing, so they do not isolate the effect of the license.

## 3. Licenses of the dependencies recommended by the R&D

Registry metadata was read on 2026-10-10. The scope follows the [R&D README](../README.md) (rows 2, 4, 13 and 14) and [ADR-002](../adr/0002-access-identity-stealth.md) (stealth candidates).

| Dependency | Version checked | License (registry) | Provenance and notes |
|---|---|---|---|
| rs-trafilatura | 0.2.2 (2026-04-03) | `MIT OR Apache-2.0` (crates.io) | Port of trafilatura and go-trafilatura ([README](https://github.com/Murrough-Foley/rs-trafilatura)). Both upstreams are Apache-2.0: trafilatura switched from **GPL-3.0** on 2024-03-20 ([commit](https://github.com/adbar/trafilatura/commit/c7e00f3a31)), go-trafilatura on 2024-06-26. rs-trafilatura was created in 2026-01, after both changes. Treat the ported parts as carrying Apache-2.0 notice obligations. Direct deps are MIT or MIT/Apache; optional `spider` is MIT. It embeds an XGBoost quality model whose training-data provenance was not audited. |
| htmd | 0.5.5 (2026-07-27) | `Apache-2.0` only | "Inspired by turndown.js" (MIT). Deps: html5ever and markup5ever_rcdom (MIT OR Apache-2.0), phf (MIT). |
| html5ever | 0.40.1 | `MIT OR Apache-2.0` | GitHub reports NOASSERTION; the crate metadata is authoritative. |
| scraper | 0.27.0 | `ISC` | Permissive. |
| reqwest | 0.13.5 | `MIT OR Apache-2.0` | — |
| wreq | 0.16.1 (2026-08-27) | `Apache-2.0` only | The predecessor crate `rquest` was MIT up to 0.20.2 and Apache-2.0 from 0.20.10 (August 2024). Its TLS comes from `btls` (Apache-2.0) and `btls-sys` (MIT), which vendor BoringSSL as a submodule. BoringSSL switched to Apache-2.0 on 2025-02-03 ([commit](https://github.com/google/boringssl/commit/33d1049b1f)). btls `main` pins a 2026-07-28 BoringSSL commit, but the LICENSE still lists some third-party BSD-style code. Optional `webpki-root-certs` is `CDLA-Permissive-2.0` (data). |
| Chromey | 2.58.2 | `MIT OR Apache-2.0` | GitHub detects Apache-2.0 only, but the repo ships LICENSE-MIT and LICENSE-APACHE. Fork lineage: chromiumoxide. Optional feature `adblock` pulls an **MPL-2.0** crate; optional `lol_html` is BSD-3-Clause. |
| Chromiumoxide | 0.9.1 | `MIT OR Apache-2.0` | — |
| Lightpanda | browser `main` | **`AGPL-3.0-only`** | CLA-backed; a proprietary license is offered on request. Telemetry is on by default and can be disabled with `LIGHTPANDA_DISABLE_TELEMETRY` ([docs](https://lightpanda.io/docs/core-concepts/local-vs-cloud)). |
| Camoufox (stealth) | repo `main`, PyPI 0.5.8 | Browser: **`MPL-2.0`** (Firefox fork). Python package: `MIT` | The README states that vendored cursory-js "is **LGPLv3-or-later, not MPL-2.0** like the rest of the browser" ([README](https://github.com/daijro/camoufox)). |
| Patchright (stealth) | npm 1.63.0, PyPI 1.63.0 | `Apache-2.0` | Fork of Playwright (Apache-2.0). |

### Compatibility with each candidate Automan license

- **Apache-2.0 (current proposal).**
  - Every permissive crate above can be linked. Binary distributions must carry the license texts and any NOTICE files of the Apache-2.0 dependencies.
  - Under the MPL-2.0 "Larger Work" rule (§3.3), Camoufox, or Chromey's `adblock` feature, can be combined as long as the MPL-covered files keep their terms ([MPL-2.0](https://www.mozilla.org/en-US/MPL/2.0/)).
  - Lightpanda must stay **out of process**. The GPL FAQ says "pipes, sockets and command-line arguments are communication mechanisms normally used between two separate programs", unless the exchange is "intimate" ([GPL FAQ](https://www.gnu.org/licenses/gpl-faq.html#MereAggregation)). If Automan ships the Lightpanda binary, the AGPL source obligations apply to that binary. If Automan's cloud runs a *modified* Lightpanda, AGPL §13 requires offering its source to network users ([AGPL-3.0 §13](https://www.gnu.org/licenses/agpl-3.0.txt)).
- **MIT.** Same as Apache-2.0. The combined artifact still carries Apache-2.0 obligations from the Apache-only crates (htmd, wreq), and MIT gives no explicit patent grant. The Rust project recommends `MIT OR Apache-2.0` "for maximum compatibility" ([Rust API Guidelines, C-PERMISSIVE](https://rust-lang.github.io/api-guidelines/necessities.html)). Five of the recommended crates use that dual license.
- **AGPL-3.0.**
  - Every dependency above can be incorporated. Apache-2.0 code "can therefore be included in GPLv3 projects" one way ([ASF](https://www.apache.org/licenses/GPL-compatibility.html)). MIT, ISC and BSD are compatible. MPL-2.0 allows GPL-family secondary licenses unless it is marked otherwise. LGPL-3.0 is compatible. Lightpanda (AGPL-3.0-only) could then even be linked.
  - The cost moves to Automan's own users. Embedding the SDK or engine in closed software, or hosting a modified version, triggers source obligations. Firecrawl answered this by relicensing its SDKs to MIT.
- **FSL or BUSL (source-available).**
  - Permissive dependencies can be redistributed inside a source-available product, keeping their notices. MPL files keep MPL terms.
  - AGPL Lightpanda can only be an aggregate process or a separately licensed component. Its code cannot be absorbed under FSL or BUSL.
  - Neither FSL nor BUSL is OSI-approved (SPDX 3.29.0), so "open source" could not be used to describe Automan.
- **Dual licensing (AGPL plus commercial).** It needs the right to relicense every contribution, which is the Lightpanda and Browserless pattern. In practice that means a CLA or assignment from the first external PR. Any AGPL dependency, such as Lightpanda, must stay out of process or be covered by its own commercial license on the commercial side.

## 4. Implications for Automan (evidence, not a decision)

| Option | Precedent in the sample | What the evidence says | Main open risk |
|---|---|---|---|
| Apache-2.0, or `MIT OR Apache-2.0` for crates, with a closed cloud for operations | Crawlee/Apify, Spider (MIT), Stagehand (MIT), Browser Use (MIT), Playwright, Scrapy/Zyte | This is the dominant pattern for frameworks that feed a cloud. The cloud differentiators are operational: anti-bot, proxies, CAPTCHA, fleet, hosted models and caches. It fits the Rust crate convention and the dependency set. | Competitors can host the engine: Hyperbrowser hosts Browser Use, and Apify hosts Scrapy. The moat must come from operations, data and repair quality, not the license. |
| MIT | Spider, Stagehand, Browser Use | Same as above, with no patent grant. The Apache-only dependencies make the "MIT simplicity" partly nominal. | Same hosting risk. |
| AGPL-3.0 core, permissive SDKs | Firecrawl (AGPL plus MIT SDKs), Skyvern, Lightpanda | It deters hosted copies; no third-party hosted Firecrawl or Skyvern was found. Without a CLA, the cloud-only parts must stay separate services, as Fire-engine and Skyvern's anti-bot do. Adoption by stars is not visibly penalized. | Friction for embedding the engine in closed software. Many enterprises restrict AGPL use: plausible but not measured here. Firecrawl needed MIT SDKs. |
| AGPL plus commercial (dual) | Lightpanda (CLA), Browserless (SSPL/GPL plus commercial) | Revenue from a license as well as the cloud. | Requires a CLA from day one. CLAs add contributor friction compared with a DCO (OpenTofu, OpenSearch). |
| FSL (two-year non-compete) or BUSL | Sentry, HashiCorp | Starting source-available (Sentry) avoids the relicensing backlash seen when an OSI license was withdrawn (Terraform, Elasticsearch, Redis). | It is not open source under the OSI definition. In this market, ecosystem companies that might build on Automan (browser clouds, scraping APIs) are the profile that formed the OpenTofu, OpenSearch and Valkey coalitions. RedMonk saw no clear financial gain from such moves. |
| Change later | Firecrawl (Apache → AGPL, three weeks after its first commit); Elastic and Redis (back to AGPL after about 3.5 years and about 14 months) | An early, small-community switch went without a visible fork (Firecrawl). Late switches on projects with outside companies invested produced forks, and later partial reversals. | Changing later requires holding the rights: a CLA or Sentry-style grant. Choosing a DCO now forecloses a unilateral relicense of contributed code. |

### Points that hold whatever the choice

- **Contributor terms are the hinge decision.** No CLA means contributions arrive under the repository license (inbound = outbound). That is fine for a permanent permissive license, but it blocks later relicensing or dual licensing. A CLA (Lightpanda, Microsoft, Elastic, HashiCorp), a template grant (Sentry) or a DCO (OpenTofu, OpenSearch) each fixes a different future.
- **Keep Lightpanda out of process** under any non-AGPL license. Keep Camoufox and Patchright as separately distributed browser binaries or plugins. Record the MPL-2.0 and LGPL-3.0-or-later parts of Camoufox, and Chromey's optional `adblock` (MPL-2.0), in a third-party notice inventory.
- **Do not vendor Crawl4AI code.** Its extra attribution clause reaches "public uses", including websites, and the registry metadata understates it.
- **The license does not cover the brand.** Apache-2.0 §6 grants no trademark rights ([Apache-2.0 text](https://www.apache.org/licenses/LICENSE-2.0)). A trademark policy is a separate lever, whatever the code license.

## Open questions

1. Which features does Automan intend to keep cloud-only? Is it operations only (fleet, proxies, stealth catalog, hosted repair models), or also engine features such as Lightpanda's multi-context and sandboxing? A permissive license fits the first case; the second needs a CLA or separate services.
2. Is it acceptable for a browser cloud (Browserbase, Hyperbrowser, Steel, Apify) to host the Automan engine? Under Apache-2.0 or MIT it will happen if Automan succeeds, as with Browser Use and Scrapy.
3. Which mechanism should apply before the first external contribution: CLA, Sentry-style grant or DCO?
4. Should the Rust crates be `MIT OR Apache-2.0` (the Rust convention) while the engine, server or cloud components use a different license, as Firecrawl splits AGPL engine and MIT SDKs?
5. Dependency audits still to do: the training data and license of the rs-trafilatura and `web-page-classifier` models; the exact BoringSSL revision vendored by the published `btls-sys` 0.5.6 (not checked); and whether Camoufox's LGPL component affects cloud images shipped to self-hosters.
6. Adoption evidence for FSL projects (Sentry, Codecov and others) was not gathered. Fill this gap if FSL stays on the table.
