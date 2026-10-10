# Availability of the name Automan — October 10, 2026

Checks run on October 10, 2026 (Paris time) for wayfinder ticket [#68](https://github.com/Jbmanllr/automan/issues/68), then completed the same day for grilling ticket [#83](https://github.com/Jbmanllr/automan/issues/83) (see [Re-verification](#re-verification-2026-10-10)). Registry and DNS results come from direct API calls and lookups. Trademark results come from the official search tools (USPTO, EUIPO, INPI, J-PlatPat, IP Australia) and the WIPO Global Brand Database (GBD). This note records what the sources show on that date. It is not legal advice and not a trademark clearance opinion.

## Verdict

**Usable as the open-source working name. Weak as a worldwide commercial brand.** The US, EU and French registers hold no live AUTOMAN mark for software or SaaS. Elsewhere, registered AUTOMAN marks do cover software, and a small company already sells an "Automan" in the same field:

- **Registered AUTOMAN marks cover software in three countries.**
  - Japan 6334989 is a logo mark reading AUTO MAN, in class 42. It covers providing computer programs and designing, creating and maintaining computer programs. It is live, and the owner paid the second-half registration fee in December 2025.
  - Brazil 912622377 is an AUTOMAN word mark in class 9, covering downloadable and recorded computer programs. It runs to 2028.
  - Thailand ค94373 is an AUTOMAN mark in class 9, covering packaged computer software. It runs to 2027.

  An Automan SDK or cloud sold in these countries would meet these marks.
- **The UK class 42 mark is weaker than it first looked.** UK00002490392 shows only "scientific and technological services and research and design relating thereto (and related services)" in class 42, and excludes anything for trucks, coaches, buses and engines. Its owner, Electro-Replacement Ltd, is an electrical spare-parts wholesaler that went into administration on 9 October 2025. The full UK wording could not be read.
- **Appliqation's Automan is in the same field, but small and unregistered.**
  - Appliqation Pty Ltd, of Melbourne, sells Automan as a module of its testing platform, in a $79 per user per month plan. Automan turns a tester's session into a Playwright script.
  - It launched between 23 February and 22 August 2026.
  - Appliqation has no trademark filed in Australia or on any register the GBD covers, and claims only "Appliqation" as a trademark.
  - Its public footprint is small: 13 npm packages, about 1,000 monthly downloads for the main SDK, 10 GitHub repositories with no stars, and nothing on PyPI.
  - It overlaps with Automan on tools and buyers (Playwright, MCP, browser automation, developers) but not on the job: test automation versus web data collection.
- **The namespace is unchanged.** crates.io is the only primary channel that is free. The unscoped npm `automan` package, the npm `@automan` scope, PyPI `automan`, GitHub `automan` and the .com, .io, .ai and .dev domains are all taken.

Keeping Automan as the open-source working name is still workable with qualified identifiers (see [Fallbacks](#fallbacks)). A commercial brand needs a professional clearance before launch. That clearance should cover the Japanese, Brazilian and Thai software marks, whether those marks are in use, the full UK specification, and Appliqation's prior use in the same field. If those markets matter, a separate commercial name is the safer plan.

## Summary table

| Channel | Status | Evidence (checked 2026-10-10) |
| --- | --- | --- |
| crates.io `automan` | **Free** | `curl -A … https://crates.io/api/v1/crates/automan` → 404 "crate `automan` does not exist". Search `?q=automan` returns only `azure_mgmt_automanage` and `azure_resourcemanager_automanage`. `automan-core`, `-cli`, `-sdk`, `-mcp`, `-rs` → 404. |
| npm `automan` (unscoped) | **Taken** (placeholder) | `npm view automan` / [registry.npmjs.org/automan](https://registry.npmjs.org/automan): single version 0.0.0, no description, BSD-2-Clause, created 2014-01-22, two maintainers. |
| npm `@automan` scope | **Taken** (no public packages) | `https://registry.npmjs.org/-/org/automan/package` → 200 `{}`. For comparison, an unused scope returns 404 "Scope not found". A user or org named `automan` therefore exists. `search?text=scope:automan` → 0 packages. |
| npm related names | Mixed | `@automan-dev` scope exists (2 packages, 2020). `@automan-component/*` (map widgets, 2020–2022), `automan-cli` (2018), `automan-cmd` (2024, a Chinese scaffold whose description uses 奥特曼, "Ultraman"), `automan-runner` (2024). `automan-sdk`, `automan-mcp`, `automan-core` → 404. `@appliqation/playwright-pom-adapter` (2026) mentions Automan in its description. |
| PyPI `automan` | **Taken** (active) | [pypi.org/pypi/automan/json](https://pypi.org/pypi/automan/json): "A simple Python-based automation framework", v0.6 (2024-07-09), BSD, by Prabhu Ramachandran, home [pypr/automan](https://github.com/pypr/automan) (29 stars, last push 2025-05-30). Paper in *Computing in Science & Engineering* 20(5), 2018 ([PyPI page](https://pypi.org/project/automan/)). `automan-sdk`, `automan-client`, `automan-py`, `pyautoman`, `automan-core`, `automan-cli`, `automan-mcp` → 404. |
| GitHub `automan` | **Taken** (user) | `gh api users/automan`: User, created 2010-12-07, 7 repos, profile company taobao.com. Repos include `automan-client`, described as a "Web UI Test Framework, By taobao Test Team" (2011). `gh api orgs/automan` → 404, because the name is a user, not an org. |
| GitHub variants | Mixed | `automan-dev` (user, created 2026-05-25, 0 repos) and `AutomanLabs` (user, 2025) are taken. `automanhq`, `automan-rs`, `getautoman`, `automan-sh` → 404. |
| automan.com | **Taken** | RDAP ([Verisign](https://rdap.verisign.com/com/v1/domain/automan.com)): registered 1996-08-12, GoDaddy. HTTPS redirects to automann.com, a heavy-duty truck parts business. |
| automan.io | **Taken, listed for sale** | `whois -h whois.nic.io`: created 2021-11-20, Spaceship, privacy proxy. NS `afternic.com`. The site redirects to `/lander`, which redirects to `forsale.godaddy.com/forsale/automan.io`. |
| automan.ai | **Taken** | RDAP (Identity Digital): registered 2025-10-17, Namecheap, privacy proxy. No NS or A records. |
| automan.dev | **Taken** | RDAP ([Google Registry](https://pubapi.registry.google/rdap/domain/automan.dev)): registered 2026-03-10, Cloudflare. Serves a redirect to `login.php`. |
| automan.app / .cloud / .co | Taken | .app: 2022-10-03, Cloudflare, with MX. .cloud: 2025-09-24, Hostinger parking. .co: 2014-05-19, Xin Net. |
| automan.sh / .run / .so / .tools | Unregistered | .sh and .so WHOIS: "Domain not found" / "No Object Found". .run and .tools: registry RDAP 404. |
| USPTO | No live mark for software | [tmsearch.uspto.gov](https://tmsearch.uspto.gov/search/search-results), query `automan`: 21 results. Live: AUTOMAN Reg. 4475600 (Tyco Fire Products, cl. 9, fire-extinguisher release mechanism, renewed 2024); ANSUL AUTOMAN (Tyco, cl. 9); AUTOMAN Reg. 5688603 / SN 87582559 (Mutual Rich Enterprise, cl. 9, batteries, chargers, solar); AUTOMAN SN 87557557 (Mutual Rich, cl. 21, hose nozzles); AUTOMAN SN 99035578 (MYT Works, cl. 9, camera-motion motor with controls, intent-to-use, pending, extension granted 2026-04-21); HOOSIER AUTOMAN (cl. 37, pending); CY AUTOMAN (cl. 7). No live class 42 mark. Dead: STERLING AUTOMAN (cl. 9 computer programs, abandoned); AUTOMAN data processing (Fleetcor, cancelled 2007). Status details: [TSDR sn85469586](https://tsdr.uspto.gov/statusview/sn85469586), [sn99035578](https://tsdr.uspto.gov/statusview/sn99035578), [sn73548316](https://tsdr.uspto.gov/statusview/sn73548316). |
| EUIPO | No AUTOMAN mark in 9/42 | [eSearch plus](https://euipo.europa.eu/eSearch/#basic/1+1+1+1/100+100+100+100/automan), basic search `automan`: 7 marks. Live: AUTOMAN 002675049 (Atlas Copco Airpower, cl. 7); AUTOMANAGER 017970265 (figurative, BIZZ NIZZ, cl. 9/35/39/42); AUTOMANAGER 009338088 (cl. 12/35). Expired: AUTOMANIAX, AUTOMANUAL (cl. 42), ITAUTEC AUTOMANAGER. |
| INPI (France) | No AUTOMAN mark in 9/42 | [data.inpi.fr brand search](https://data.inpi.fr/search?advancedSearch=%7B%7D&displayStyle=List&filter=%7B%7D&nbResultsPerPage=100&order=asc&page=1&q=automan&sort=relevance&type=brands) (FR, EU and international bases, updated 2026-10-09): 5 marks. AUTOMAN 002675049 (EU, Atlas Copco, cl. 7, registered); Automan 624776 (international, Ecolab, cl. 5/25); Auto'man (EU, cl. 38, expired); two French AUTOMAN marks (Atlas Copco, cl. 7/8, expired). |
| J-PlatPat (Japan) | **Live AUTOMAN mark for software (cl. 42)** | [J-PlatPat](https://www.j-platpat.inpit.go.jp/t0100), mark text `AUTOMAN` → 2 live marks; reading `オートマン` → 4 live marks. Class 42 [6334989](https://www.j-platpat.inpit.go.jp/c1801/TR/JP-2019-144614/40/ja) is software; class 9 [7079812](https://www.j-platpat.inpit.go.jp/c1801/TR/JP-2025-143469/40/ja) is laser measuring tools; [5501332](https://www.j-platpat.inpit.go.jp/c1801/TR/JP-2011-084439/40/ja) AUTOMAN MINI is electric shutter openers; 2141846 (Bunka Shutter) is in cl. 6/19. Details under [Conflicts](#conflicts-found). |
| IP Australia | No AUTOMAN mark for software; nothing by Appliqation | [Australian Trade Mark Search](https://search.ipaustralia.gov.au/trademarks/search/quick?q=automan), quick search `automan` → 3 results. AUTOMAN [173459](https://search.ipaustralia.gov.au/trademarks/search/view/173459/details) (cl. 9, old record with no status, owner or goods data); AUTOMAN 235649 (Atlas Copco, cl. 7, removed); YEKEI (owner Shenzhen Automan Industrial, cl. 18). A quick search for `appliqation` returns 0 results, as owner or as mark. |
| WIPO Global Brand Database | AUTOMAN registered for software outside US/EU/FR | [branddb.wipo.int](https://branddb.wipo.int/en/similarname), brand name contains the word "automan" → 122 records, all read. 14 are registered in Nice 9 or 42, listed below. The only pending 9/42 record is the USPTO MYT Works application (SN 99035578). |
| Docker Hub / Homebrew | User taken / free | `hub.docker.com/v2/users/automan/` → 200, 0 public repositories. Homebrew formula and cask `automan` → 404. |

The 14 GBD records registered in class 9 or 42, as checked on 2026-10-10 (detail pages linked where read):

| Mark | Office, number | Owner | Goods and services | Software? |
| --- | --- | --- | --- | --- |
| AUTO MAN (logo, colour) | Japan 6334989 | 合同会社エヌ・コネクト (N-Connect LLC) | Cl. 42: providing computer programs for digital content management and other computer programs; designing, creating or maintaining computer programs; design (not for advertising). Filed 2019-11-14, registered 2020-12-25. | **Yes** |
| AUTOMAN (word) | [Brazil 912622377](https://branddb.wipo.int/en/similarname/brand/BR500000912622377) | AFHF Serviços de Tecnologia da Informação Ltda | Cl. 9: downloadable computer programs; recorded computer programs; game software. Registered 2018-10-30, expires 2028-10-30. | **Yes** |
| AUTOMAN | [Thailand ค94373](https://branddb.wipo.int/en/similarname/brand/TH501997000343478) | บริษัท คริสตอลซอฟท์แวร์แพ็คเกจ จำกัด (Crystal Software Package Co.) | Cl. 9: packaged computer software. Registered 1999-06-28, expires 2027-09-08. "AUTO" disclaimed. | **Yes** |
| AUTOMAN (word) | [UK00002490392](https://branddb.wipo.int/en/similarname/brand/GB50UK00002490392) | Electro-Replacement Ltd | Cl. 7, 8, 10, 11, 37–40, and cl. 42 "scientific and technological services and research and design relating thereto (and related services)". All exclude goods or services for trucks, coaches, buses and engines. Filed 2008-06-18, registered 2010-06-25, expires 2028-06-18. | Unclear |
| AUTOMAN (word) | [UK00001457591](https://branddb.wipo.int/en/similarname/brand/GB50UK00001457591) | Electro-Replacement Ltd | Cl. 9: time switches, control panels and switches for electric or electronic purposes, not for motor vehicles. Registered 1993, expires 2028-03-05. | No |
| AUTOMAN | [Korea 4007136220000](https://branddb.wipo.int/en/similarname/brand/KR502006400041172) | 주식회사 오토만 | Cl. 9 hardware: transformers, switches, players, radios, modems, telephones, GPS and navigation devices, mouse, simulators. Expires 2027-06-15. | No (hardware) |
| AUTO.MAN | [Korea 4002585260000](https://branddb.wipo.int/en/similarname/brand/KR502003560002357) | LG Electronics (주식회사 엘지이아이) | Cl. 9, goods not listed in GBD. Registered 2003-09-18, expiry date 2023-02-11, although GBD still shows "Registered". This is the record the first pass did not read. | Unknown |
| AUTOMAN | Korea 4002547670000 | LG Electronics | Cl. 9, goods not read. | Unknown |
| AUTOMAN | Japan 7079812 | 山真製鋸株式会社 (Yamashin Seikyo) | Cl. 9: laser marking devices, laser measuring instruments, measuring instruments. Registered 2026-09-02, in the opposition period. | No |
| AUTOMAN | India 543336 | Vijay Kathuria | Cl. 9 (1991), goods not read. | Unknown |
| AUTOMAN | Uruguay 436300 | Sylvia Monica Katz Polak | Cl. 9/11, goods not read. | Unknown |
| AUTOMAN eyewear | Bulgaria 105224 | "Омега Оптик-1" ЕООД | Cl. 9/35, an eyewear brand; goods not read. | Unlikely |
| AUTOMAN | US 4475600 | Tyco Fire Products | Cl. 9: fire-extinguisher release mechanism. | No |
| AUTOMAN | US 5688603 | Mutual Rich Enterprise | Cl. 9: batteries, chargers, solar. | No |

## Conflicts found

Ordered by practical risk for this project, an open-source Rust automation framework with SDKs, a CLI, an MCP server and a commercial cloud.

1. **Appliqation "Automan": same field, commercial, unregistered.**
   - **What it is.** [appliqation.io/automan](https://appliqation.io/automan) says Automan turns a tester's manual session into a Playwright script. It checks the script in a real browser, auto-heals selectors, opens a branch, commits and raises a pull request. Automan is a module of the Appliqation test-management platform, which calls itself "The MCP-native testing platform". It sits alongside TARS, Qonsole and Runman.
   - **Pricing.** Automan is part of the "Elevate" plan at $79 per user per month ([pricing](https://appliqation.io/pricing)).
   - **The company.**
     - The seller is Appliqation Pty Ltd, "a company incorporated in Australia" ([terms](https://appliqation.io/terms), updated 2 June 2026), based in Melbourne ([about](https://appliqation.io/about)).
     - ABN Lookup shows ABN 72 674 923 776, ACN 674 923 776, active from 12 February 2024, Victoria, not registered for GST ([ABN Lookup](https://abr.business.gov.au/ABN/View?abn=72674923776)).
     - The domain appliqation.io was created on 2023-09-07 (`whois -h whois.nic.io appliqation.io`; GoDaddy, privacy proxy, Cloudflare).
     - Crunchbase lists 1–10 employees and calls the company customer-funded ([Crunchbase](https://www.crunchbase.com/organization/appliqation), a third-party profile).
   - **Launch date: between 23 February and 22 August 2026.**
     - In 2023–2024 the domain hosted a WordPress newsletter for testers. The CDX listing of all archived appliqation.io URLs (`https://web.archive.org/cdx/search/cdx?url=appliqation.io&matchType=domain&collapse=urlkey`) ends on 2026-02-23 and contains no `/automan` page.
     - On that date the archived homepage already described MCP test creation, yet neither it, the about page nor the [sitemap](https://web.archive.org/web/20260223233756/https://appliqation.io/sitemap.xml) mentions Automan.
     - The first dated mentions are on npm on 2026-08-22. `@appliqation/automation-sdk` 2.8.0 is the first version whose README refers to `tests/automan/`; checked per version on jsDelivr, version 2.7.0 of 2026-05-15 has none. `@appliqation/playwright-pom-adapter` 0.1.0 "converts Automan-generated Playwright scripts".
     - The current [sitemap](https://appliqation.io/sitemap.xml) lists `/automan`, and the homepage quotes platform figures "as at August 2026".
   - **Trademarks.**
     - The terms claim only "The 'Appliqation' name and logo" as trademarks. The Automan pages show no ™ or ®.
     - No AUTOMAN or APPLIQATION filing appears at IP Australia (quick searches `automan` and `appliqation`), at USPTO (first pass), or anywhere in the GBD (owner contains "appliqation" → "No results found!").
     - Very recent filings may not be indexed yet.
   - **Visibility.**
     - npm: 13 packages under `@appliqation` (`https://registry.npmjs.org/-/org/appliqation/package`), all published by one maintainer, all except the SDK created in August–September 2026. Last-month downloads, 2026-09-09 to 2026-10-08: automation SDK 1,030, agent-core 186, scriptgen 99, POM adapter 63 (`https://api.npmjs.org/downloads/point/last-month/@appliqation/<name>`).
     - GitHub: user `appliqation`, created 2023-09-28, 10 public repositories (agents created August–September 2026), 0 stars each (`gh api users/appliqation/repos`). No public Automan repository.
     - PyPI: `appliqation`, `appliqation-sdk`, `appliqation-automation-sdk` and `appliqation-mcp` → 404.
     - The documentation site docs.appliqation.io has an Automan section of about 10 pages.
     - The homepage claims 413,843 test executions, 11,224 MCP calls and a 5.0 G2 average.
     - Product Hunt and G2 answered with bot checks and were not read. A web search found no Product Hunt listing.
   - **Overlap with this project.**
     - Same: browser automation built on Playwright-style control, an MCP surface, developer buyers and a paid SaaS.
     - Different: the job. Automan generates and maintains QA regression tests. This project collects web data and automates browsers. They also sell to different teams: QA and test leads versus data and agent builders.
     - In trademark terms both are class 9/42 software. An identical name on related software is the classic setting for a likelihood-of-confusion argument, so this is a question for counsel.
     - This repository was created on 2026-10-08 and is private (`gh repo view`), so Appliqation's public use comes first. As a sub-brand of a 1–10 person company with no registration, its reach looks limited for now.
2. **Registered AUTOMAN software marks in Japan, Brazil and Thailand.**
   - **Japan 6334989 is the closest match.**
     - The [J-PlatPat gazette](https://www.j-platpat.inpit.go.jp/c1801/TR/JP-2019-144614/40/ja) lists class 42 services: providing computer programs for digital content management and other computer programs, and designing, creating or maintaining computer programs. That covers software provision and development.
     - The mark is a colour logo whose word element reads AUTO MAN (search text ＡＵＴＯＭＡＮ, readings オートマン / オート / マン).
     - It is held by 合同会社エヌ・コネクト (N-Connect LLC), represented by 特許業務法人森脇特許事務所.
     - J-PlatPat notes that a registration-fee receipt for the installment payment was added on 2025-12-24, so the owner is keeping the mark alive.
     - A quick web search found no product of this company named AUTO MAN. Whether the mark is in use was not established.
   - **Brazil 912622377** is a word mark for downloadable and recorded software (see the table).
   - **Thailand ค94373** is held by a software-package company for packaged software, renewed to 2027.

   These are the marks an examiner or owner would most plausibly raise against an Automan software filing, or against use in those countries.
3. **PyPI `automan`: active automation framework.** It is maintained (release 2024, repository activity 2025) and academically cited. Under [PEP 541](https://peps.python.org/pep-0541/), a name transfers only if the owner is unreachable, there was no release in 12 months and there is no activity. All three must hold, and they do not. The Python SDK would also clash on the import name if both packages were installed.
4. **npm `automan` and `@automan`.** The unscoped name is a 2014 0.0.0 placeholder, and the scope is held by an account with no public packages. npm ["does not transfer … simply because another user wants the name"](https://docs.npmjs.com/policies/disputes). The formal dispute route requires an infringed trademark that the requester owns.
5. **GitHub `automan`.** It is an old personal account whose repositories include a Taobao web UI test framework (2011–2016). GitHub ["does not release … usernames on the basis that they appear inactive"](https://docs.github.com/en/site-policy/other-site-policies/github-username-policy), except on a valid trademark complaint.
6. **Domains.** The .com belongs to an unrelated business (redirects to automann.com, truck parts). The .io is for sale through GoDaddy/Afternic, at a price not checked. The .ai and .dev were registered recently (2025-10 and 2026-03) by unknown parties, and .dev already serves a login page.
7. **Other trademarks.**
   - **United States.** Identical AUTOMAN marks in class 9 cover unrelated goods. Whether software is "related" to fire-suppression release mechanisms, batteries or camera-motion motors is a likelihood-of-confusion question for counsel, and an examiner could cite these marks against a class 9 filing.
   - **UK00002490392.** Its class 42 text in GBD is the abbreviated class heading "scientific and technological services and research and design relating thereto (and related services)".
     - At the 2008 filing date, the full Nice heading (9th edition) also read "industrial analysis and research services; design and development of computer hardware and software" ([WIPO, Nice 9th ed., Part I](https://www.wipo.int/export/sites/www/classifications/nice/en/pdf/9engp1.pdf)). Whether the UK record includes that wording could not be checked.
     - The owner is an electrical spare-parts business: SIC 46439 wholesale of electrical household appliances. It has been in administration since 9 October 2025, with accounts overdue ([Companies House 01013271](https://find-and-update.company-information.service.gov.uk/company/01013271), [insolvency](https://find-and-update.company-information.service.gov.uk/company/01013271/insolvency)). The administrators may sell or drop its marks.
     - UK law allows revocation of a registration not put to genuine use for five years ([Trade Marks Act 1994, s. 46](https://www.legislation.gov.uk/ukpga/1994/26/section/46)). The mark has been registered since 2010.
   - **UK00001457591** covers time switches and control panels, not software.
   - **Korea.** The Korean class 9 marks cover hardware, or their goods were not read.
   - **Japan.** 7079812 covers laser measuring tools and 5501332 (AUTOMAN MINI, Bunka Shutter) covers electric shutter openers.
8. **Other software projects named AutoMan or Automan.**
   - [AutoMan](https://en.wikipedia.org/wiki/AutoMan) is a z/OS job-scheduling and console automation product by Exspans (Ottawa). exspans.com no longer resolves and exspans.ca shows a bare directory index, so the product looks dormant.
   - [automan-lang/AutoMan](https://github.com/automan-lang/AutoMan) is a "Human-Computation Runtime" from academic research (GPL-2.0, docs.automanlang.org, 49 stars, last push 2024).
   - [tier4/AutomanTools](https://github.com/tier4/AutomanTools) is a 3D annotation tool for self-driving AI (66 stars, 2023).
   - [automan-bot/AutoX](https://github.com/automan-bot/AutoX) (225 stars) and [automan-bot/android-bot](https://github.com/automan-bot/android-bot) (93 stars) are Android automation projects from a Chinese community, with activity in 2025.
   - GitHub search `automan in:name` returns 837 repositories (`gh api search/repositories`), most of them small.
   - The name is also a 1983 TV series ([Automan](https://en.wikipedia.org/wiki/Automan)), and Chinese developers use it as a transliteration of Ultraman (奥特曼), which explains several npm packages.

## What could not be verified

- **TMview** (tmdn.org) still answers with its bot-detection page ("Problem detected"), from both `curl` and a browser, at the start and the end of the session. It was not bypassed. The EUIPO and TMview APIs need a registered developer account, so they were not used. WIPO GBD, J-PlatPat, IP Australia and the national registers were used instead.
- **UK IPO** (`trademarks.ipo.gov.uk`) returns "Service Unavailable" in a browser and a captcha page to `curl`. Its journal pages (`ipo.gov.uk/t-tmj`) also return the captcha. The Internet Archive holds no capture of the UK record. So the full class 42 wording of UK00002490392, and any recorded change behind its 2024 status date, are known only from GBD.
- **Goods not read:** Korea 4002547670000 and the LG AUTO.MAN record (GBD lists no goods), India 543336, Uruguay 436300 and Bulgaria 105224.
- **Use of the Japanese, Brazilian and Thai software marks** was not investigated. It matters for any non-use attack, which is a question for counsel.
- **Appliqation's exact launch date** is bounded but not known, because the Wayback Machine has no capture between 2026-02-23 and today. Its Product Hunt and G2 pages answered with bot checks. Common-law or Australian Consumer Law rights from its use cannot be searched in any register.
- **Register lag.** Filings made in the last days or weeks may not yet appear in IP Australia, USPTO or GBD search results.
- **Domain availability** is inferred from DNS, RDAP and WHOIS. No registrar was queried for price or premium status, and nothing was registered or reserved.
- **npm and GitHub:** a 404 for a scope or user means no account currently holds the name. It does not guarantee that the name can be created, because reserved and blocked names are not exposed.

## Fallbacks

The conflicts are serious enough to plan for qualified identifiers if the name is kept. The following were free on 2026-10-10. None was reserved.

| Need | Candidates observed free |
| --- | --- |
| Rust crates | `automan` itself, plus `automan-core`, `automan-cli`, `automan-mcp` |
| npm scope | `@automanhq`, `@automan-sh`, `@automan-rs`, `@automanjs`, `@automan-js`, `@getautoman`, `@useautoman`, `@automan-io`, `@automan-ai`, `@automan-labs`, `@automan-hq` (each → 404 "Scope not found") |
| npm unscoped | `automan-sdk`, `automan-mcp`, `automan-core` |
| PyPI | `automan-sdk`, `automan-client`, `automan-core`, `automan-mcp`, `automan-cli`, with an import package other than `automan` (for example `automan_sdk`) |
| GitHub org | `automanhq`, `automan-rs`, `getautoman`, `automan-sh` |
| Domains | `getautoman.com`, `automanhq.com`, `useautoman.com`, `automan.sh`, `automan.run`, `automan.tools`, `automan.so`; `automan.io` is purchasable at an unknown price |

If the commercial cloud needs a clean brand, evaluate a different name with the same procedure: registries, GitHub, DNS/RDAP, USPTO, EUIPO, INPI, J-PlatPat, IP Australia, WIPO GBD (reading goods and services, not just classes), and a web search for same-field products.

## Re-verification (2026-10-10)

The first pass left four gaps. They were redone the same day for [#83](https://github.com/Jbmanllr/automan/issues/83). No bot check or captcha was bypassed, and nothing was registered or reserved.

**Checked:**

- **TMview**, retried with `curl` and in a browser at the start and end of the session. Still blocked. Replaced by WIPO GBD detail pages, J-PlatPat and IP Australia.
- **The UK class 42 mark.** UK IPO is still unavailable, so the record was read through GBD. The owner was checked at Companies House.
- **The Japanese class 42 mark.** Read in the J-PlatPat gazette, with searches on the mark text and on the reading オートマン.
- **All 122 GBD records** for brand name "automan", read in full, including the record the first pass never rendered.
- **Software-relevant GBD detail pages:** Brazil, Thailand and Korea.
- **Appliqation:** its site; the Wayback CDX API (online this time); per-version npm READMEs; its npm, GitHub and PyPI footprint; ABN Lookup; WHOIS; IP Australia; and a GBD owner search.

**Changed:**

- **The verdict.** The first pass said no live mark covers software on the registers checked. That still holds for the US, the EU and France, but not worldwide. Japan 6334989 (class 42: providing and developing computer programs), Brazil 912622377 (class 9 software) and Thailand ค94373 (class 9 packaged software) are live AUTOMAN registrations for software. The verdict moves from "usable with heavy caveats" to "usable as the open-source working name, weak as a worldwide commercial brand".
- **The UK class 42 mark turned out weaker.** Its visible class 42 text is the abbreviated research-and-design heading, with a carve-out for trucks and engines. Its owner is an electrical spare-parts wholesaler in administration since 2025-10-09.
- **The unread GBD record** is LG Electronics' AUTO.MAN, Korea 4002585260000, in class 9. Its expiry date (2023-02-11) has passed, although GBD still shows "Registered".
- **Appliqation is now characterised.** Australian company with ABN from 2024-02-12. Automan launched between 2026-02-23 and 2026-08-22. No trademark filed anywhere searched. Small footprint. Same tools and buyers, different job.

**Still unverified:** the full UK specification (UK IPO down), the goods of the remaining Korean, Indian, Uruguayan and Bulgarian records, whether the Japanese, Brazilian and Thai software marks are in use, Appliqation's exact launch date and any unregistered rights, and filings too recent to be indexed.
