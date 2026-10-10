# Availability of the name Automan — October 10, 2026

Checks run on October 10, 2026 (Paris time) for wayfinder ticket [#68](https://github.com/Jbmanllr/automan/issues/68). Registry and DNS results come from direct API calls and lookups; trademark results come from the official search tools (USPTO, EUIPO, INPI) and the WIPO Global Brand Database. This note records what the sources show on that date. It is not legal advice and not a trademark clearance opinion.

## Verdict

**Usable with caveats, and the caveats are heavy.** The name is legally open on the registers checked for software: no live AUTOMAN mark at USPTO, EUIPO or INPI covers software or SaaS. In practice the namespace is crowded:

- **crates.io is the only primary channel that is free.** The unscoped npm `automan` package, the npm `@automan` scope, PyPI `automan`, GitHub `automan` and the four main domains (.com, .io, .ai, .dev) are all taken.
- **A commercial "Automan" is live in the same field.** Appliqation sells Automan, a cross-browser test automation product that turns tester sessions into Playwright scripts, on an "MCP-native testing platform". The overlap with Playwright, browser automation and MCP is the most serious conflict, because it creates a risk of confusion for the future commercial cloud. No registered mark for it was found.
- **PyPI `automan` is an active Python automation framework** with a peer-reviewed paper. The Python SDK cannot use that distribution name or the `automan` import name.
- **Identical AUTOMAN marks are registered in class 9 for unrelated goods in the US**: fire-extinguisher release mechanisms, batteries and chargers, and camera motors (pending). Outside US/EU/FR there are AUTOMAN registrations in classes 9 and 42, in the UK, Japan, Brazil, Korea, Thailand and India.

Keeping Automan as an open-source working name is workable with qualified identifiers (see [Fallbacks](#fallbacks)). Before the commercial cloud launches, a professional clearance search should check the Appliqation overlap and the UK and Japan class 42 marks.

## Summary table

| Channel | Status | Evidence (checked 2026-10-10) |
| --- | --- | --- |
| crates.io `automan` | **Free** | `curl -A … https://crates.io/api/v1/crates/automan` → 404 "crate `automan` does not exist". Search `?q=automan` returns only `azure_mgmt_automanage` and `azure_resourcemanager_automanage`. `automan-core`, `-cli`, `-sdk`, `-mcp`, `-rs` → 404. |
| npm `automan` (unscoped) | **Taken** (placeholder) | `npm view automan` / [registry.npmjs.org/automan](https://registry.npmjs.org/automan): single version 0.0.0, no description, BSD-2-Clause, created 2014-01-22, two maintainers. |
| npm `@automan` scope | **Taken** (no public packages) | `https://registry.npmjs.org/-/org/automan/package` → 200 `{}`. For comparison, an unused scope returns 404 "Scope not found". A user or org named `automan` therefore exists. `search?text=scope:automan` → 0 packages. |
| npm related names | Mixed | `@automan-dev` scope exists (2 packages, 2020). `@automan-component/*` (map widgets, 2020–2022), `automan-cli` (2018), `automan-cmd` (2024, a Chinese scaffold whose description uses 奥特曼, "Ultraman"), `automan-runner` (2024). `automan-sdk`, `automan-mcp`, `automan-core` → 404. |
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
| WIPO Global Brand Database | AUTOMAN registered in 9/42 outside US/EU/FR | [branddb.wipo.int](https://branddb.wipo.int/en/similarname), brand name contains "automan", status Registered, Nice 9 or 42 → 14 results (13 read). UK00002490392 (Electro-Replacement Ltd, cl. 7/8/10/11/37/38/39/40/**42**, 2010) and UK00001457591 (same owner, cl. **9**, 1993); Japan 6334989 (合同会社エヌ・コネクト, cl. **42**, 2020); Brazil 912622377 (AFHF Serviços de Tecnologia da Informação, cl. 9, 2018); Korea 4002547670000 (LG Electronics, cl. 9, 2003) and 4007136220000 (주식회사 오토만, cl. 9, 2007); Japan 7079812 (cl. 9, 2026); Thailand ค94373 (a software-package company, cl. 9, 1999); India 543336 (cl. 9, 1991); Uruguay 436300 (cl. 9/11); Bulgaria "AUTOMAN eyewear" (cl. 9/35); plus the US Tyco and Mutual Rich marks above. |
| Docker Hub / Homebrew | User taken / free | `hub.docker.com/v2/users/automan/` → 200, 0 public repositories. Homebrew formula and cask `automan` → 404. |

## Conflicts found

Ordered by practical risk for this project, an open-source Rust automation framework with SDKs, a CLI, an MCP server and a commercial cloud.

1. **Appliqation "Automan": same field, commercial.** [appliqation.io/automan](https://appliqation.io/automan) presents "Automan: cross-browser test automation, run for you". It records a tester's session, converts it to Playwright, validates it in a real browser, opens a pull request and auto-heals the script. It is sold on the "Elevate" plan of Appliqation, which calls itself "The MCP-native testing platform". The npm package [`@appliqation/playwright-pom-adapter`](https://registry.npmjs.org/@appliqation%2fplaywright-pom-adapter) (created 2026-08-22) converts "Automan-generated Playwright scripts", so the name was in commercial use by August 2026 at the latest. The pages show no ™/® notice, and no AUTOMAN registration in its name appeared in the USPTO or WIPO GBD results. Unregistered rights from prior use can still exist in common-law countries. An earlier launch date could not be established because the Internet Archive was offline.
2. **PyPI `automan`: active automation framework.** It is maintained (release 2024, repository activity 2025) and academically cited. Under [PEP 541](https://peps.python.org/pep-0541/), a name transfers only if the owner is unreachable, there was no release in 12 months and there is no activity. All three must hold, and they do not. The Python SDK would also clash on the import name if both packages were installed.
3. **npm `automan` and `@automan`.** The unscoped name is a 2014 0.0.0 placeholder, and the scope is held by an account with no public packages. npm ["does not transfer … simply because another user wants the name"](https://docs.npmjs.com/policies/disputes). The formal dispute route requires an infringed trademark that the requester owns.
4. **GitHub `automan`.** It is an old personal account whose repositories include a Taobao web UI test framework (2011–2016). GitHub ["does not release … usernames on the basis that they appear inactive"](https://docs.github.com/en/site-policy/other-site-policies/github-username-policy), except on a valid trademark complaint.
5. **Domains.** The .com belongs to an unrelated business (redirects to automann.com, truck parts). The .io is for sale through GoDaddy/Afternic, at a price not checked. The .ai and .dev were registered recently (2025-10 and 2026-03) by unknown parties, and .dev already serves a login page.
6. **Trademarks.** In the US, identical AUTOMAN marks in class 9 cover unrelated goods. Whether software is "related" to fire-suppression release mechanisms, batteries or camera-motion motors is a likelihood-of-confusion question for counsel, and an examiner could cite them against a class 9 filing. Outside the requested registers, the UK (classes 9 and 42) and Japan (class 42) registrations are the closest to software services. Their goods and services were not read.
7. **Other software projects named AutoMan or Automan.**
   - [AutoMan](https://en.wikipedia.org/wiki/AutoMan) is a z/OS job-scheduling and console automation product by Exspans (Ottawa). exspans.com no longer resolves and exspans.ca shows a bare directory index, so the product looks dormant.
   - [automan-lang/AutoMan](https://github.com/automan-lang/AutoMan) is a "Human-Computation Runtime" from academic research (GPL-2.0, docs.automanlang.org, 49 stars, last push 2024).
   - [tier4/AutomanTools](https://github.com/tier4/AutomanTools) is a 3D annotation tool for self-driving AI (66 stars, 2023).
   - [automan-bot/AutoX](https://github.com/automan-bot/AutoX) (225 stars) and [automan-bot/android-bot](https://github.com/automan-bot/android-bot) (93 stars) are Android automation projects from a Chinese community, with activity in 2025.
   - GitHub search `automan in:name` returns 837 repositories (`gh api search/repositories`), most of them small.
   - The name is also a 1983 TV series ([Automan](https://en.wikipedia.org/wiki/Automan)), and Chinese developers use it as a transliteration of Ultraman (奥特曼), which explains several npm packages.

## What could not be verified

- **TMview** (tmdn.org) blocked the session with a bot-detection page, so it was not used. EUIPO, INPI, USPTO and WIPO GBD were searched directly instead.
- **WIPO GBD** renders results in a virtualized list. Of the 14 registered class 9/42 results, 13 were read and one was never rendered. The single pending class 9/42 result was not listed either; it is probably the USPTO MYT Works application.
- **Goods and services** of the UK (UK00002490392, UK00001457591), Japanese (6334989, 7079812), Brazilian and Korean marks were not read, because the UK IPO service was unavailable and J-PlatPat and INPI Brazil were not queried. Their relevance to software is unknown.
- **Unregistered or common-law use** (Appliqation and others) cannot be searched in any register.
- **Domain availability** is inferred from DNS, RDAP and WHOIS. No registrar was queried for price or premium status, and nothing was registered or reserved.
- **npm and GitHub:** a 404 for a scope or user means no account currently holds the name. It does not guarantee that the name can be created, because reserved and blocked names are not exposed.
- **Wayback Machine** was offline, so the history of Appliqation's Automan and of automan.dev could not be traced.

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

If the commercial cloud needs a clean brand, evaluate a different name with the same procedure: registries, GitHub, DNS/RDAP, USPTO, EUIPO, INPI, WIPO GBD, and a web search for same-field products.
