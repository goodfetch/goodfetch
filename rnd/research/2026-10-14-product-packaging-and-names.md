# Naming, packaging and versioning of multi-product developer tools, and name availability for Collie and Foxhound

Research of **October 14, 2026** for ticket [#451](https://github.com/goodfetch/automan/issues/451). Question: how do multi-product developer tools name, package, version and distribute their products across npm, PyPI, crates.io, Homebrew and OCI, and which names are available for Collie and Foxhound? It informs the planning map's choice of whether to ship Collie and Foxhound as separately installable products, with Foxhound's binary containing Collie, under Goodfetch as the umbrella brand. The current decisions are ADR 0034 (Foxhound and Collie as namespaces that never stand alone), ADR 0030 (one product version), ADR 0032 (build once, publish per registry), ADR 0024 (one engine binary, thin Clients) and ADR 0039 (Platform signatures).

**Method.** Registry facts come from direct API calls made on 2026-10-14 (exact URLs in §2.1): the npm registry and download APIs, the PyPI JSON API and pypistats, the crates.io API (with the User-Agent `goodfetch-research (https://github.com/goodfetch/automan)`), Homebrew's formula, cask and executables indexes, sources.debian.org, packages.debian.org and packages.ubuntu.com, Repology, the AUR and Arch APIs, anaconda.org, Snapcraft, RubyGems, NuGet, JSR, Docker Hub, GHCR and the GitHub API. Trademark facts come from the USPTO trademark search backend, EUIPO eSearch plus and TMview, read on the same day; INPI's own pages and the WIPO Global Brand Database could not be used (§7). Packaging and versioning facts come from npm and PyPI metadata, source files read through the GitHub API, official docs and issue trackers; part of them was gathered earlier the same day by an interrupted attempt at this ticket and was spot-checked here (versions of 17 npm packages, the Stagehand protocol README, and three cited issues all matched). Nothing was registered, reserved or published. This is not legal advice or a clearance opinion.

**Versions inspected (npm unless stated, publish date in parentheses).** Playwright, `playwright-core` and `@playwright/test` 1.64.1 (2026-10-14), `@playwright/mcp` 0.0.83 (2026-09-28), PyPI `playwright` 1.64.0; `crawlee` and `@crawlee/*` 3.18.2 (2026-09-29), PyPI `crawlee` 1.10.4; `@browserbasehq/stagehand` 4.2.1 (2026-10-11), PyPI `stagehand` 4.2.1; `ai` 7.0.140 (2026-10-14), `@ai-sdk/openai` 4.0.93; `langchain` 1.5.16, `@langchain/core` 1.2.17; `prisma` 8.0.0-rc.22 on `latest` (2026-10-08), `@prisma/client` 7.10.0 (2026-08-25); `@biomejs/biome` 2.5.15; `esbuild` 0.28.2; `turbo` 2.11.8; `@sentry/node` 11.7.0, `@sentry/cli` 3.8.0; PyPI `ruff` 0.17.0, `uv` 0.13.1, `ty` 0.0.87.

Like everything in `rnd/`, this note informs a decision; it does not make one.

---

## 1. Summary

- **Bare `collie` and `foxhound` are taken on every primary registry except crates.io `foxhound`, and Homebrew has neither.**
  - npm `foxhound` is active: Retold's database query generator, 95 versions, last published 2026-10-07, 5,070 downloads in the last month.
  - npm `collie` is dormant: an "async hook" library, last published 2018-10-18, 555 downloads in the last month.
  - PyPI `foxhound` is a `0.0` placeholder that Fox-IT (NCC Group) reserved on 2023-10-27 "for future Dissect project".
  - PyPI `collie` is ShopRunner's recommender-system library, last released 2023-03-31, whose GitHub home now returns 404.
  - crates.io `collie` belongs to the Collie feed reader (0.2.0, 2025-11-15); `foxhound` is free.
  - No Homebrew formula, cask or formula executable, Debian or Ubuntu package, Arch or AUR package, or Repology project is named `collie`, `foxhound` or `goodfetch`.
- **Every `goodfetch-*` and `@goodfetch/*` name is free, and so are the base `goodfetch` names, except the GitHub organization.** npm `goodfetch`, the npm `@goodfetch` scope ("Scope not found"), PyPI `goodfetch`, crates.io `goodfetch`, Docker Hub `goodfetch` and `goodfetch.dev` (RDAP 404) are all unclaimed today. Only the GitHub organization `goodfetch` exists (created 2026-10-11, one private repository). ADR 0034's reservations of the npm scope, a PyPI placeholder, the crate and the domain have not happened yet.
- **Two active projects already use the Foxhound name for this product's own job.**
  - A project named Foxhound ([repository](https://github.com/sadewadee/foxhound)) is a "Go Scraping Framework with Native Camoufox Anti-Detection" (MIT, created 2026-03-17, v0.0.28 on 2026-09-25) that builds a `foxhound` executable.
  - FoxHound ([repository](https://github.com/amrelsagaei/FoxHound)) is an "Agent Browser Bridge for Firefox" that lets an AI agent drive Firefox through an MCP server, npm `foxhound-mcp` 1.0.4 (2026-05-11).
  - SAP's Project Foxhound, a Firefox fork for taint tracking, ships Playwright-patched builds ([`.PLAYWRIGHT_VERSION`](https://github.com/SAP/project-foxhound/blob/main/.PLAYWRIGHT_VERSION) pins v1.62.0) and released 142.0.1 on 2026-06-15. A Goodfetch user could plausibly drive "Foxhound" the browser from "Goodfetch Foxhound" the crawler.
- **A popular, fast-moving developer tool already installs `collie` on PATH.** ColliePWA ([repository](https://github.com/AltanS/collie)), a phone client for AI coding agents running in terminal multiplexers, has 1,338 stars, was created on 2026-06-28, released v1.21.2 on 2026-10-14, and its installer "puts `collie` on your PATH". It also offers `collie skill` and `collie docs` for coding agents. The RubyGems `collie` gem (1.0.0, 2026-05-19) and three npm packages also install a `collie` executable.
- **COLLIE is a registered EU mark for software in general, and FOXHOUND a registered French mark for software development.**
  - EUTM 018890716 `collie` (word, registered 2023-10-04, expires 2033-06-20) covers "Computer hardware; Software; Computer systems" in class 9 and "Software design and development; Software development" in class 42, held by Collie, an Amsterdam company. Collie B.V., apparently the same company (same city, same representative, CHIEVER BV), holds Benelux 1513918 and international registration 1853213 (classes 9 and 42, animal-management software), registered in the US on 2026-03-31 (Reg. 8190894) and filed in Canada.
  - Other live COLLIE marks in classes 9 or 42: US Reg. 8033356 (class 42, event-registration software), China 11835023 (class 9), Japan 2024082184 "§Collie" (classes 9, 35 and 42), and Tencent's TCA-COLLIE in China (classes 9 and 42).
  - French mark 4860964 FOXHOUND (word, registered 2022-08-05, class 42) covers software design and development, SaaS, cloud computing, server hosting and data storage, held by an individual. The EU has "Foxhound Games" (classes 9 and 41, games software). FOXHOUND has no live US mark in classes 9 or 42.
  - GOODFETCH has no live mark in any office TMview covers; its only record is an abandoned 2018 US application for an insurance marketplace.
- **Comparable vendors ship a meta-package over exactly pinned parts.** `crawlee` pins 14 `@crawlee/*` packages at 3.18.2; `@playwright/test` → `playwright` → `playwright-core` pin 1.64.1 exactly; binary wrappers (Biome, esbuild, Turborepo, Next.js SWC) pin their per-platform packages exactly. Heavy third-party engines are optional peers (Crawlee → Playwright) or Python extras (`crawlee[playwright]`).
- **When two products share an engine, users get two copies and skew.** `@playwright/mcp` 0.0.83 ships its own pre-release `playwright-core` (`1.64.0-alpha-…`), so browsers installed by `npx playwright install` do not match ([#41871](https://github.com/microsoft/playwright/issues/41871)). The Crawlee README tells Apify SDK users to add npm `overrides` "so that you don't end up with multiple versions of Crawlee installed". Stagehand's Python 3.x wheels each embedded a Node server binary (30.8–37.3 MB per wheel in 3.23.0, the last 3.x) until 4.0.0 (2026-08-10) moved to one pure wheel of about 0.4 MB.
- **Versioning splits along one line: lockstep inside a language, a separately versioned contract across products.** Playwright, Crawlee JS, Sentry JS and Prisma keep one version and exact pins. Stagehand v4 versions its SDKs, extension and protocol independently and gates compatibility only on `protocolVersion` (majors equal, server minor ≥ client minor). The AI SDK gates on each model's `specificationVersion` and adapts three spec majors. Crawlee and Sentry keep independent version lines per language.
- **Umbrella brands present open-source sub-products in one of three ways.** A funnel product gets its own domain and an unscoped or company-scoped package with "by X" (Stagehand, Crawlee). Peer tools get unscoped names and their own binaries under one docs host (Astral's uv, Ruff and ty). Parts of one product stay "Brand X" under one scope and domain, with a separate binary only for a separate audience (Playwright Test, Playwright MCP, Playwright CLI).

---

## 2. Name availability

### 2.1 Registry matrix

Checked 2026-10-14. "Free" means the API returned 404 or no result; it does not prove the name can be created, because reserved or blocked names are not exposed. PyPI normalizes `-`, `_` and `.` as the same name ([PEP 503](https://peps.python.org/pep-0503/#normalized-names)), and crates.io treats `-` and `_` as the same crate, so `goodfetch-collie` covers `goodfetch_collie` there. The Debian, Ubuntu, Repology and Homebrew-index searches match substrings, so their results for `collie` and `foxhound` also cover the `goodfetch-` names.

| Name | npm | PyPI | crates.io | Homebrew formula | Homebrew cask | Debian | Ubuntu | Repology | Docker Hub namespace | GitHub account |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| `collie` | **Taken**, dormant | **Taken**, dormant | **Taken**, active | Free | Free | Free | Free | Free | **Taken** (user) | **Taken** (org, dormant) |
| `foxhound` | **Taken**, active | **Taken** (placeholder) | Free | Free | Free | Free | Free | Free | **Taken** (user, empty) | **Taken** (org, dormant) |
| `goodfetch-collie` | Free | Free | Free | Free | Free | Free | Free | Free | n/a | n/a |
| `goodfetch-foxhound` | Free | Free | Free | Free | Free | Free | Free | Free | n/a | n/a |
| `@goodfetch/collie` | Free (scope unclaimed) | n/a | n/a | n/a | n/a | n/a | n/a | n/a | n/a | n/a |
| `@goodfetch/foxhound` | Free (scope unclaimed) | n/a | n/a | n/a | n/a | n/a | n/a | n/a | n/a | n/a |
| `goodfetch` | Free; `@goodfetch` scope free | Free | Free | Free | Free | Free | Free | Free | Free | **Held by the project** (org, 2026-10-11) |

The exact queries:

| Registry | Query | Result for each name |
| --- | --- | --- |
| npm package | `https://registry.npmjs.org/<name>`; scoped `https://registry.npmjs.org/@goodfetch%2fcollie` | 200 for `collie`, `foxhound`; 404 for `goodfetch`, `goodfetch-collie`, `goodfetch-foxhound`, `@goodfetch/collie`, `@goodfetch/foxhound`, `@goodfetch/goodfetch`, `@goodfetch/cli` |
| npm scope | `https://registry.npmjs.org/-/org/<scope>/package` | `@goodfetch` → 404 `{"error":"Scope not found"}`; `@collie` → 200, a user account listing one unscoped package (`vue-youtube-embed-fix`, 2022); `@foxhound` → 200 `{}`, an account with no packages |
| npm downloads | `https://api.npmjs.org/downloads/point/last-month/<name>` | `collie` 555, `foxhound` 5,070 (2026-09-13 to 2026-10-12) |
| PyPI | `https://pypi.org/pypi/<name>/json` | 200 for `collie`, `foxhound`; 404 for `goodfetch`, `goodfetch-collie`, `goodfetch_collie`, `goodfetch.collie`, `goodfetch-foxhound`, `goodfetch_foxhound` |
| PyPI downloads | `https://pypistats.org/api/packages/<name>/recent` | `collie` 104 last month, `foxhound` 10 |
| crates.io | `https://crates.io/api/v1/crates/<name>` | 200 for `collie`; 404 "crate … does not exist" for `foxhound`, `goodfetch`, `goodfetch-collie`, `goodfetch-foxhound`, `goodfetch-cli`, `goodfetch-protocol` |
| Homebrew | `https://formulae.brew.sh/api/formula/<name>.json`, `https://formulae.brew.sh/api/cask/<name>.json` | 404 for all five names in both (control: `ruff` → 200). The full indexes (`/api/formula.json`, 8,659 formulae; `/api/cask.json`, 7,783 casks) have no token, name or description matching `collie`, `foxhound` or `goodfetch` |
| Homebrew executables | `https://formulae.brew.sh/api/internal/executables.txt` (8,519 formulae) | No formula installs an executable named `collie`, `foxhound` or `goodfetch` |
| Debian | `https://sources.debian.org/api/src/<name>/`; `https://sources.debian.org/api/search/<name>/` | `{"error":404}` and no results for all three |
| Debian packages | `https://packages.debian.org/search?keywords=<name>&searchon=names&suite=all&section=all`; contents with `mode=exactfilename&suite=stable` | "Sorry, your search gave no results" for all three names; no file named exactly `collie`, `foxhound` or `goodfetch` in trixie |
| Ubuntu | `https://packages.ubuntu.com/search?keywords=<name>&searchon=names&suite=all&section=all`; contents with `mode=exactfilename&suite=noble` | No results for all three |
| Repology | `https://repology.org/api/v1/project/<name>`; `https://repology.org/api/v1/projects/?search=<name>` | `[]` for all three; the search finds only `collier` (a physics library) |
| Arch, AUR | `https://archlinux.org/packages/search/json/?name=<name>`; `https://aur.archlinux.org/rpc/v5/search/<name>?by=name` | 0 results for all three |
| conda | `https://api.anaconda.org/package/conda-forge/<name>`; `https://api.anaconda.org/search?name=<name>` | conda-forge 404 for all three. Personal channels hold `ea-group/collie` (a header-only library, 0.2.21) and `indico/foxhound` (0.3.0, Indico's archived GPU machine-learning library) |
| Snap, JSR | `https://api.snapcraft.io/v2/snaps/info/<name>` (header `Snap-Device-Series: 16`); `https://api.jsr.io/scopes/<name>` | 404 for all three |
| RubyGems, NuGet | `https://rubygems.org/api/v1/gems/<name>.json`; `https://api.nuget.org/v3-flatcontainer/<name>/index.json` | RubyGems `collie` taken (§2.3); NuGet `Collie` taken (a dependency-injection container, 2.2.2, 2023-09-30); `foxhound` and `goodfetch` free on both |
| Docker Hub | `https://hub.docker.com/v2/users/<ns>/`; `https://hub.docker.com/v2/repositories/<ns>/` | `collie` → user exists, one repository `get-started` (2018-12-20, 40 pulls); `foxhound` → user exists, no public repository; `goodfetch` → 404 |
| GHCR | `https://ghcr.io/token?scope=repository:<ns>/<image>:pull`, then `/v2/<ns>/<image>/tags/list` | `goodfetch/collie`, `goodfetch/foxhound` → `NAME_UNKNOWN`; `goodfetch/goodfetch` → anonymous token denied (no public image). GHCR namespaces follow GitHub accounts, so `ghcr.io/goodfetch/*` can only be published by the `goodfetch` organization ([GitHub Docs](https://docs.github.com/en/packages/working-with-a-github-packages-registry/working-with-the-container-registry)) |
| GitHub | `gh api users/<name>`, `gh api orgs/<name>/repos` | `collie` org (2011, one repository, last push 2019); `Foxhound` org (2014, one repository, last push 2014); `goodfetch` org (2026-10-11, no public repository, one private) |
| Domain | `https://pubapi.registry.google/rdap/domain/goodfetch.dev` | 404: not registered |

### 2.2 Who holds the bare names

| Name | Registry | Holder project | Last publish | Activity |
| --- | --- | --- | --- | --- |
| `foxhound` | npm | [Retold](https://github.com/fable-retold/foxhound) (GitHub organization `fable-retold`, 148 repositories): "A Database Query generation library", MIT, created 2015-02-18 | 2.0.33 on 2026-10-07 (95 versions; 1.0.45–1.0.47 in September 2026) | **Active**: repository pushed 2026-10-07, 5,070 downloads in the last month, depended on by the same organization's `meadow` and `retold` packages |
| `collie` | npm | [maichong/collie](https://github.com/maichong/collie) (organization 脉冲云, maichong.io): "async hook", MIT, created 2015-05-23 | 0.3.0 on 2018-10-18 | **Dormant**: repository last pushed 2018-10-18, 0 stars; still 555 downloads in the last month, so something depends on it |
| `foxhound` | PyPI | [Fox-IT's Dissect](https://github.com/fox-it/dissect) (part of NCC Group; a forensics framework with 1,169 stars, pushed 2026-10-09): "Placeholder for future Dissect project", version `0.0` | 2023-10-27 (one release) | **Reserved, no code**: 10 downloads in the last month |
| `collie` | PyPI | ShopRunner's [collie](https://pypi.org/project/collie/): "A PyTorch library for preparing, training, and evaluating deep learning hybrid recommender systems", BSD-3-Clause, docs at collie.readthedocs.io | 1.3.1 on 2023-03-31 (9 releases since 2021-07-13) | **Dormant**: the listed home `github.com/ShopRunner/collie` and the `ShopRunner` GitHub account return 404; 104 downloads in the last month |
| `collie` | crates.io | The [Collie feed reader](https://github.com/collie-reader) (organization `collie-reader`): "A library for the minimal feed reader", AGPL-3.0-or-later, created 2024-09-26 | 0.2.0 on 2025-11-15 | **Active project**: the desktop app `collie-reader/collie` has 249 stars and was pushed 2026-09-24; the crate has 103 recent downloads and declares no binary |
| `collie` | GitHub | Organization `collie` (2011): one repository, `collie/accord`, "A high-performance coordination service for write-intensive workloads", 86 stars | — | **Dormant**: last push 2019-08-09 |
| `Foxhound` | GitHub | Organization `Foxhound` (2014): one repository, "Security-focused Static Code Analyser and IPS for PHP", 0 stars | — | **Dormant**: last push 2014-08-25 |
| `collie`, `foxhound` | Docker Hub | User accounts; `collie/get-started` (2018) only | 2018-12-20 | **Dormant** |

What the registries allow:

- **PyPI.** A name moves only if the project is abandoned, which requires "ALL" of: owner not reachable, "no releases within the past twelve months", and "no activity from the owner on the project's home page (or no home page listed)" ([PEP 541](https://peps.python.org/pep-0541/)). Reuse of the name also needs the candidate project to meet "notability requirements" and download statistics showing the existing package "is not being used". A project that is "name squatting (package has no functionality or is empty)" is invalid. ShopRunner's `collie` has had no release since 2023 and its home page is gone, but it still has downloads. Fox-IT's `foxhound` is a functionless placeholder held by a reachable organization.
- **npm.** npm does not transfer a name "simply because another user wants the name"; the dispute route needs a trademark the requester owns ([npm disputes policy](https://docs.npmjs.com/policies/disputes), as cited in the [Automan name research](https://github.com/Jbmanllr/automan/blob/research/name-availability/rnd/research/2026-10-10-name-availability.md)).
- **crates.io.** "the crates.io team will not transfer ownership of existing crates without the explicit approval of the current owner"; crates that exist "only to reserve a name … without having any genuine functionality" break the usage policy ([crates.io policies source](https://github.com/rust-lang/crates.io/blob/main/svelte/src/routes/policies/+page.svelte)).

### 2.3 Executables that would clash on PATH

| Executable | Project | How it reaches PATH | Activity |
| --- | --- | --- | --- |
| `collie` | [ColliePWA](https://github.com/AltanS/collie) (MIT): "Herdr mobile client for iPhone and Android. A self-hosted PWA to drive Claude Code, Pi, Codex and OpenCode", topics `ai-agents`, `claude-code`, `codex` | `curl -fsSL https://colliepwa.dev/install.sh \| sh` "puts `collie` on your PATH" ([README](https://github.com/AltanS/collie)); release tarballs for Linux and macOS, x64 and arm64; install docs for Linux, Arch, Nix, macOS and Windows | **Very active**: 1,338 stars, created 2026-06-28, v1.21.1 and v1.21.2 both released 2026-10-14. Its `collie skill` and `collie docs` print agent briefs and docs "out of the binary and with no network", the same conventions as ADR 0030's `goodfetch skills` and `goodfetch docs` |
| `collie` | RubyGems [`collie`](https://rubygems.org/gems/collie): "a linter and formatter for Lrama Style BNF grammar files" | `gem install collie` installs `exe/collie` ([repository](https://github.com/ydah/collie)) | Active: 1.0.0 on 2026-05-19, 2,946 downloads |
| `collie` | npm `@collie-lang/cli` ("Collie CLI.") | `bin: {"collie": "dist/index.js"}` | 1.6.0 on 2026-01-17, 19 downloads in the last month |
| `collie` | npm `collie-js` ("A tiny, modern package manager for the web") and `underdog-collie` (a webpack-based compiler) | `bin: {"collie": …}` | Dormant (2022) |
| `collie` | [meshcloud/collie-cli](https://github.com/meshcloud/collie-cli) (landing zones on AWS, Azure and GCP, Apache-2.0, 184 stars) | Release binaries `collie-<target>` | Archived (last push 2026-03-30, last release v0.27.0 on 2024-08-28) |
| `foxhound` | [SAP/project-foxhound](https://github.com/SAP/project-foxhound) (GPL-3.0, 180 stars) | Release archives `foxhound-140.0.2.en-US.linux-x86_64.tar.xz`, `.mac.dmg`, `.win64.installer.exe`; binaries with and without Playwright integration hosted by TU Braunschweig ([README](https://github.com/SAP/project-foxhound)). The executable's name inside the archive was not inspected | Active: pushed 2026-10-14, release 142.0.1 on 2026-06-15 |
| `foxhound` | [Foxhound, a Go scraping framework](https://github.com/sadewadee/foxhound) (MIT) | `go build -tags playwright -o foxhound ./cmd/foxhound/`, then `foxhound init myproject` ([README](https://github.com/sadewadee/foxhound)) | Active: v0.0.28 on 2026-09-25 |
| `foxhound-mcp` | npm `foxhound-mcp` (FoxHound browser bridge) and npm `@foxhound-ai/mcp-server` ("MCP server for querying Foxhound traces") | Both declare the same `foxhound-mcp` bin | 1.0.4 (2026-05-11) and 0.3.0 (2026-04-13); 57 and 74 downloads in the last month |
| `goodfetch` | none found | — | — |

No distribution checked ships an executable named `collie`, `foxhound` or `goodfetch`: not Homebrew (formula executables index), Debian trixie or Ubuntu noble (exact-filename contents search), Arch, the AUR or Repology's index. The clashes above come from installers, language registries and source builds.

### 2.4 Well-known and same-field projects using the names

**Foxhound**

- **SAP's Project Foxhound**: "A web browser with dynamic data-flow tracking enabled in the Javascript engine and DOM, based on Mozilla Firefox", used for "large scale vulnerability scanning" through its Playwright support, and rated best of 18 tools in a 2025 study of dynamic JavaScript security analysis ([README](https://github.com/SAP/project-foxhound)). It is the closest well-known use, and it sits next to Goodfetch's own Playwright Firefox and Camoufox support (ADR 0024).
- **Foxhound, the Go scraping framework**: its README lists dual-mode fetching (TLS-impersonating HTTP plus Camoufox), "Checkpoint/resume", vendor block detection for Cloudflare, Akamai, DataDome and PerimeterX, 9 export formats including webhook, and a `Hunt` API ([README](https://github.com/sadewadee/foxhound)). It is small (11 stars) but describes the same job as Goodfetch Foxhound.
- **FoxHound, "Agent Browser Bridge for Firefox"**: "lets an AI agent drive your Firefox profile end to end" through a Firefox add-on and the npm MCP server `foxhound-mcp` ([README](https://github.com/amrelsagaei/FoxHound), 21 stars, created 2026-04-28). This is Collie's job under the Foxhound name.
- **Foxhound (AI observability)**: npm `@foxhound-ai/sdk`, "Observability SDK for AI agents — trace, replay, and audit every agent decision", and `@foxhound-ai/mcp-server` (both 0.3.0, April 2026); the linked repository returns 404.
- Others: Retold's query generator (npm), Fox-IT's placeholder (PyPI), [a Foxhound WordPress theme](https://github.com/ryelle/Foxhound) (a React blog theme, 346 stars, 2018), Indico's archived GPU ML library, and Foxhound Games (EU mark, §2.5).

**Collie**

- **ColliePWA**, above: the most visible current "Collie" for developers, in AI coding-agent tooling.
- **Collie feed reader** (`collie-reader`): desktop app, `collied` server and the `collie` crate.
- **Collie B.V.** (Netherlands): virtual fencing and animal-management collars, holder of the COLLIE software marks (§2.5).
- **Research and infrastructure projects**: [OpenMOSS/CoLLiE](https://github.com/OpenMOSS/CoLLiE) (collaborative LLM training, 421 stars, 2024), [princeton-nlp/Collie](https://github.com/princeton-nlp/Collie) (a constrained text-generation benchmark, ICLR 2024), [chaitin/collie](https://github.com/chaitin/collie) (the community edition of a host-security product, 156 stars), meshcloud's Collie CLI (archived), [bytedance/Collie](https://github.com/bytedance/Collie) (archived).
- GitHub search `collie in:name` returns 1,073 repositories and `foxhound in:name` 303 (`gh api search/repositories`, sorted by stars).

### 2.5 Trademarks

**USPTO** (the trademark search's backend, `POST https://tmsearch.uspto.gov/prod-stage-v1-0-0/tmsearch`, query on the word mark, 2026-10-14):

| Query | Records | Live marks in classes 9 or 42 |
| --- | --- | --- |
| `collie` | 85 | **COLLIE** Reg. 8033356 (SN 97980795, class 42, registered 2025-11-18): "Development of computer software designed for the management of event registration and fundraising; … Platform as a service (PAAS) …", Vizsla Technologies, Inc. **COLLIE** Reg. 8190894 (SN 79423162, the US part of international registration 1853213, classes 9 and 42, registered 2026-03-31, priority 2024-11-07): electronic animal collars, "downloadable computer software featuring software for … location tracking of animals", "software design and development; computer software development in the field of animal management", Collie B.V. **ORDERCOLLIE** Reg. 6926077 (classes 9 and 35, food-ordering software), Venue Eats, LLC |
| `foxhound` | 18 | None. FOXHOUND SN 78783884 and FOXHOUND+ SN 78881339 (class 9, "Computer programs for searching the contents of computers and computer networks by remote control; Computer search engine software", Harbinger Technologies Group) are cancelled under Section 8. Two class 42 FOXHOUND applications (MaxOrd, LLC) are abandoned. Live marks are FOXHOUND FUEL (class 5) and FOXHOUND BEE COMPANY (class 35) |
| `goodfetch`, `good fetch` | 1 | None. GOODFETCH SN 88204040 (class 35, an online marketplace for insurance quotes) is abandoned |

**EUIPO** (eSearch plus, `https://euipo.europa.eu/eSearch/#basic/1+1+1+1/100+100+100+100/<term>`, detail pages `#details/trademarks/<number>`):

| Query | Results | Marks that matter |
| --- | --- | --- |
| `collie` | 32 (most are COLLIER and COLLIERS) | **EUTM 018890716 `collie`** (word, filed 2023-06-20, registered 2023-10-04, expires 2033-06-20): class 9 "Computer hardware; Software; Computer systems", class 18 "Collars for animals", class 42 "Development of computer systems; Software design and development; Software development"; owner Collie, a legal entity in Amsterdam. **IR W01853213 COLLIE** designating the EU (Collie B.V., classes 9 and 42, "IR accepted", 2025-04-03). **EUTM 019333348 CollieCommand** (registered 2026-07-22, classes 9, 35, 38, 41 and 42 including "Computer software" and "Software as a service [SaaS]", "in particular for the management of vehicle fleets"), Industrieanlagen-Betriebsgesellschaft mbH (Germany) |
| `foxhound` | 2 | **EUTM 019140473 Foxhound Games** (word, registered 2025-06-08): class 9 "Games software", class 41 "Providing games"; FOXHOUND GAMES LTD (Cyprus). EUTM 000729947 FOXHOUND (classes 18, 24, 25) expired |
| `goodfetch` | 0 | — |

**TMview** (all participating offices, `https://www.tmdn.org/tmview/#/tmview/results?page=1&pageSize=100&criteria=C&basicSearch=<term>`, with `&fNiceClass=9,42` for COLLIE; detail pages `#/tmview/detail/<id>`):

| Query | Results | Live marks in classes 9 or 42 |
| --- | --- | --- |
| `foxhound` | 79 in all classes | **France 4860964 FOXHOUND** (word, filed 2022-04-12, registered 2022-08-05, class 42): "développement d'ordinateurs ; conception de logiciels ; développement de logiciels ; … logiciels en tant que services (SaaS) ; informatique en nuage ; … hébergement de serveurs ; stockage électronique de données", held by an individual ([TMview detail](https://www.tmdn.org/tmview/#/tmview/detail/FR500000004860964)). EUTM 019140473 Foxhound Games (above). Ended: Russia 2003707930 (class 9) and the US records above |
| `collie`, classes 9 and 42 | 206 (the first 100 by relevance read) | EUTM 018890716 `collie` and IR 1853213 COLLIE (above); **Benelux 1513918 COLLIE** and 1513917 "c collie" (classes 9 and 42, filed 2024-11-07, Collie B.V., the basis of the IR); **Canada 2397520-00 COLLIE** (classes 9 and 42, filed 2025-04-03, Collie B.V., pending); US Reg. 8033356 (above); **China 11835023 COLLIE** (class 9, filed 2012-12-03, a Shenzhen technology company); **Egypt EG/156061 COLLIE** (class 9, 2002); **Japan 2024082184 and 2024082185 "§Collie"** (classes 35, 9 and 42, 株式会社ITAGE), with "§BoardCollie" filed 2026-03-10; Japan 2024110111 "Colliebot" (class 9); **China TCA-COLLIE 64970378 and 64981173** (classes 9 and 42, 腾讯科技(深圳)有限公司, Tencent); China COLLIE INTELLIGENT SAFETY (class 9, Great Wall Motor); Germany 3020211181352 COLLIE-IT (classes 9, 35 and 42, an individual); Spain M4295120 SoftCollie (classes 25 and 42, an individual) |
| `goodfetch` | 1 | None. The one record is US SN 88204040, GOODFETCH, INC., class 35 insurance marketplace, "Ended" |

What this shows:

- **COLLIE.** The EU `collie` registration names "Software" and "Software development" without a field limit, so it covers any software sold under COLLIE in the EU, browser automation included; the same holder has filed in the Benelux, the US and Canada. ADR 0034 did not record the COLLIE marks.
- **FOXHOUND.** The French registration covers software development and SaaS in class 42, the class ADR 0034 plans for GOODFETCH. ADR 0034 records only SAP's Project Foxhound as an overlap, and accepts it because "only Goodfetch is filed as a mark".
- **Sub-name use.** Whether "Goodfetch Collie" or "Goodfetch Foxhound", used only as sub-names, would be found confusingly similar to these registrations is a question for counsel.

---

## 3. Packaging: how multi-product tools ship

### 3.1 Per vendor

**Playwright (Microsoft).** All npm packages pin each other exactly ([npm](https://www.npmjs.com/package/playwright)):

| Package | Depends on | Contents |
| --- | --- | --- |
| `playwright-core` 1.64.1 | nothing | The engine and the `playwright-core` CLI, no install script, 13.7 MB unpacked |
| `playwright` 1.64.1 | `playwright-core@1.64.1` | The test runner and the `playwright` CLI |
| `@playwright/test` 1.64.1 | `playwright@1.64.1` | A thin re-export |
| `playwright-chromium`, `-firefox`, `-webkit`; `@playwright/browser-*` | `playwright-core@1.64.1` | An install script that downloads one browser ([library-js.md](https://github.com/microsoft/playwright/blob/main/docs/src/library-js.md)) |
| `@playwright/mcp` 0.0.83 | `playwright` and `playwright-core` at `1.64.0-alpha-1790635538000` | Its own pre-release copy of the engine |

- **Python** (`playwright` 1.64.0, 2026-10-13, [PyPI](https://pypi.org/project/playwright/)): eight `py3-none-<platform>` wheels of 34.5–48.2 MB, each bundling a driver built from npm `playwright-core` plus an official Node.js binary, pinned by [`DRIVER_VERSION`](https://github.com/microsoft/playwright-python/blob/main/DRIVER_VERSION), which [`setup.py`](https://github.com/microsoft/playwright-python/blob/main/setup.py) says "doubles as the build cache key". Runtime dependencies are only `pyee` and `greenlet`.
- **Java**: `com.microsoft.playwright:driver-bundle` is "Node.js binaries for the Playwright driver on every supported platform", a 203.8 MB jar ([POM](https://github.com/microsoft/playwright-java/blob/main/driver-bundle/pom.xml)).
- **Browsers** live in one per-user cache keyed by revision (`~/Library/Caches/ms-playwright`, `~/.cache/ms-playwright`); "Each version of Playwright needs specific versions of browser binaries", and Playwright "will garbage collect them as you update" ([browsers.md](https://github.com/microsoft/playwright/blob/main/docs/src/browsers.md)). `PLAYWRIGHT_BROWSERS_PATH=0` puts them inside `node_modules`.
- **Docker** images (`mcr.microsoft.com/playwright:v<version>-noble`) carry browsers but not the package, and "If the Playwright version in your Docker image does not match the version in your project/tests, Playwright will be unable to locate browser executables" ([docker.md](https://github.com/microsoft/playwright/blob/main/docs/src/docker.md)).
- **Homebrew** has `playwright-cli` 0.1.22, built from npm `@playwright/cli` and depending on `node` ([formula](https://formulae.brew.sh/api/formula/playwright-cli.json)).

**Crawlee (Apify).**

- **npm**: `crawlee` 3.18.2 is a meta-package over 14 `@crawlee/*` packages pinned to exactly 3.18.2 (lerna fixed mode, [`lerna.json`](https://github.com/apify/crawlee/blob/master/lerna.json); [`scripts/copy.ts`](https://github.com/apify/crawlee/blob/master/scripts/copy.ts) rewrites internal dependencies to exact versions). `playwright` and `puppeteer` are optional peers at `*`, "not bundled with Crawlee to reduce install size" ([README](https://github.com/apify/crawlee/blob/master/README.md)). The native `@crawlee/fs-storage-native` (0.2.2) keeps its own version line.
- **PyPI** (`crawlee` 1.10.4, 2026-10-06, [PyPI](https://pypi.org/project/crawlee/)): one pure wheel of 431 kB with extras `beautifulsoup`, `parsel`, `playwright`, `stagehand` (`stagehand>=3.19.5,<4.0.0`), `curl-impersonate`, `redis`, `sql-*`, `otel`, `cli` and `all`.
- **Docker**: `apify/actor-node-playwright-chrome`, `apify/actor-python-playwright` and others, tagged `<node-major>-<playwright-version>`.

**Stagehand (Browserbase).**

- One monorepo with `sdk-ts`, `sdk-python`, `sdk-go`, `protocol`, `extension` and `cli` (npm `browse`) ([packages](https://github.com/browserbase/stagehand/tree/main/packages)).
- Its Playwright dependency changed by major: `playwright ^1.52.0` hard in 2.5.0 and 3.0.0; `playwright-core`, `puppeteer-core` and `patchright-core` as optional peers in 3.7.3; none in 4.x, whose engine runs as a Chrome extension service worker and whose SDKs speak JSON-RPC defined in `stagehand.v4.json` ([protocol README](https://github.com/browserbase/stagehand/blob/main/packages/protocol/README.md)).
- **The Python record of embedding an engine.** Stagehand 3.x Python wheels were per platform (`py3-none-macosx_11_0_arm64`, `manylinux2014_x86_64`, `win_amd64`, `macosx_10_9_x86_64`), 30.8–37.3 MB each in 3.23.0 (2026-08-20, the last 3.x; PyPI JSON), embedding a Node single-executable server under `src/stagehand/_sea` through a hatch build hook that forced platform tags ([hatch_build.py](https://github.com/browserbase/stagehand-python/blob/main/hatch_build.py)). 4.0.0 (2026-08-10) is one pure `py3-none-any` wheel of about 0.4 MB that ships the extension as data ([extension_assets.py](https://github.com/browserbase/stagehand/blob/main/packages/sdk-python/src/stagehand/extension_assets.py)).

**Vercel AI SDK.** `ai` 7.0.140 depends on exactly pinned `@ai-sdk/gateway`, `@ai-sdk/provider` 4.0.27 and `@ai-sdk/provider-utils` 5.0.59; provider packages such as `@ai-sdk/openai` 4.0.93 pin the same two. Provider packages carry dist-tags for older SDK majors (`ai-v5`, `ai-v6`) (npm metadata).

**LangChain.** npm `langchain` 1.5.16 declares `@langchain/core ^1.2.17` as a peer, and partner packages do the same; the docs say to install `langchain @langchain/core` explicitly ([install.mdx](https://github.com/langchain-ai/docs/blob/main/src/oss/langchain/install.mdx)). PyPI `langchain` 1.4.5 depends on `langchain-core>=1.6.10,<2` and offers each provider as an extra.

**Prisma.** `prisma` 7.10.0 depends on `@prisma/engines`, whose postinstall downloads the Schema Engine binary from `binaries.prisma.sh` (overridable with `PRISMA_ENGINES_MIRROR`) ([engines index.ts](https://github.com/prisma/prisma/blob/7.10.0/packages/engines/src/index.ts)). Prisma 7.0.0 (2025-11-19) made the Rust-free client the default ([changelog](https://www.prisma.io/changelog.md)). To run two majors side by side, `@prisma/prisma7` 7.10.0 exposes a `prisma7` bin over `prisma@7.10.0`.

**Shared code, separate products.** Astral's ty and Ruff share their Rust source: ty's README says development "takes place in the Ruff repository", whose `ruff` submodule "includes all of the Rust source code" ([ty README](https://github.com/astral-sh/ty)). They still ship as separate binaries with separate version lines and separate wheels: `ty` 0.0.87 (17 wheels of 12–14 MB, 2026-10-13) and `ruff` 0.17.0 (17 wheels of 8–10 MB, 2026-10-09) on PyPI. The Oxc project does the same on npm: `oxlint` 1.88.0 and `oxfmt` 0.73.0 each have their own `@oxlint/binding-*` and `@oxfmt/binding-*` platform packages. Installing both products of either pair installs the shared code twice.

**Native binaries over npm and PyPI.**

| Tool | Platform packages | Fallback | Override |
| --- | --- | --- | --- |
| Biome 2.5.15 | 8 `@biomejs/cli-<os>-<arch>[-musl]` optional deps, pinned exactly | none | `BIOME_BINARY` ([shim](https://github.com/biomejs/biome/blob/main/packages/@biomejs/biome/bin/biome)) |
| esbuild 0.28.2 | 26 `@esbuild/<os>-<arch>` | postinstall: `npm install` of the platform package, then a direct tarball download ([node-install.ts](https://github.com/evanw/esbuild/blob/main/lib/npm/node-install.ts)) | `ESBUILD_BINARY_PATH` |
| Turborepo 2.11.8 | 6 `@turbo/<os>-<arch>`, renamed from unscoped `turbo-<os>-<arch>` in 2.9.0 | none; the shim tries the scoped then the legacy name ([bin/turbo](https://github.com/vercel/turborepo/blob/main/packages/turbo/bin/turbo)) | `TURBO_BINARY_PATH` |
| `@sentry/cli` 3.8.0 | 8 `@sentry/cli-<platform>` | postinstall download from Sentry's CDN with a checksum ([install.js](https://github.com/getsentry/sentry-cli/blob/master/scripts/install.js)) | `SENTRYCLI_USE_LOCAL` |
| Ruff 0.17.0, uv 0.13.1 | PyPI `py3-none-<platform>` wheels (17 and 18), no npm channel; also Homebrew core, crates.io, `ghcr.io/astral-sh/{ruff,uv}` ([uv install](https://github.com/astral-sh/uv/blob/main/docs/getting-started/installation.md)) | — | — |

### 3.2 Two products that share an engine, both installed

- **Two engine copies and mismatched browsers.** `@playwright/mcp` pins a pre-release `playwright-core`, so `npx playwright install` from the project installs different browser revisions; a global Playwright MCP then reports `Browser "firefox" is not installed` ([#41871](https://github.com/microsoft/playwright/issues/41871), closed 2026-07-27).
- **One cache, garbage-collected by heuristics.** Playwright's browser GC deleted browsers another install still needed; the maintainers said it "detects which browsers are still needed via heuristics based on file presence" and kept the default ([#40995](https://github.com/microsoft/playwright/issues/40995), 2026-05).
- **Duplicate cores that break singletons.**
  - Crawlee: two copies of `@crawlee/core` mean two global configurations, so `Dataset.pushData` can silently use memory storage ([apify-sdk-js#237](https://github.com/apify/apify-sdk-js/issues/237)). The Apify SDK now calls `checkCrawleeVersion()` in `Actor.init` ([actor.ts](https://github.com/apify/apify-sdk-js/blob/master/src/actor.ts)), which gave false positives in monorepos ([#375](https://github.com/apify/apify-sdk-js/issues/375)), and the Crawlee README asks for npm `overrides`.
  - Playwright Test throws "Requiring @playwright/test second time", and lists "You have two different versions of @playwright/test" as a cause ([index.ts](https://github.com/microsoft/playwright/blob/main/packages/playwright/src/index.ts), [testType.ts](https://github.com/microsoft/playwright/blob/main/packages/playwright/src/common/testType.ts); [#31478](https://github.com/microsoft/playwright/issues/31478)).
  - LangChain: multiple `@langchain/core` copies break `instanceof` ([langchainjs#3918](https://github.com/langchain-ai/langchainjs/issues/3918)); [PR #11344](https://github.com/langchain-ai/langchainjs/pull/11344) (merged 2026-08-11) overrides `instanceof` because "multiple versions of core are installed".
- **Isolation instead of failure.** Sentry keys its global carrier by SDK version (`__SENTRY__[SDK_VERSION]`), so since 8.7.0 SDKs on one page "only interact with each other if they're using the same version" ([carrier.ts](https://github.com/getsentry/sentry-javascript/blob/develop/packages/core/src/carrier.ts), [micro-frontends.mdx](https://github.com/getsentry/sentry-docs/blob/master/docs/platforms/javascript/common/best-practices/micro-frontends.mdx)).
- **Global and local binaries.** "When a local binary is installed, the global binary defers to it" (Turborepo [installation docs](https://github.com/vercel/turborepo/blob/main/apps/docs/content/docs/getting-started/installation.mdx)). Angular's CLI runs a local-versus-global version check ([init.ts](https://github.com/angular/angular-cli/blob/main/packages/angular/cli/lib/init.ts)).
- **Platform packages left out.** [npm/cli#4828](https://github.com/npm/cli/issues/4828) (307 comments): a lockfile made on one OS omitted other platforms' optional binaries. It was closed on 2025-04-03 by [npm/cli#8184](https://github.com/npm/cli/pull/8184). Biome picked the glibc binary on Alpine images without `ldd` ([biome#10501](https://github.com/biomejs/biome/issues/10501)) and preview builds made bun and pnpm download every platform ([biome#5645](https://github.com/biomejs/biome/issues/5645)).
- **Disk.** Platform packages are 10–80 MB each (esbuild darwin-arm64 10.6 MB, Turborepo 34.2 MB, Biome 58.5 MB, Bun 70.5 MB, Deno 80.9 MB; npm metadata). ADR 0057 projects Goodfetch at about 95–120 MB raw and 35–45 MB compressed per platform, so two products each carrying the whole engine would install it twice.

---

## 4. Versioning across products of one vendor

| Vendor | Across packages of one language | Across languages | Compatibility check |
| --- | --- | --- | --- |
| Playwright | Lockstep, exact pins (1.64.1) | Same number; Python 1.64.0 (2026-10-13), Java and .NET 1.63.0 trail by days to weeks ([python](https://github.com/microsoft/playwright-python/releases), [java](https://github.com/microsoft/playwright-java/releases)) | Remote `connect()` needs equal major and minor ("1.2.3 → is compatible with 1.2.x"), enforced by [`playwrightServer.ts`](https://github.com/microsoft/playwright/blob/main/packages/playwright-core/src/remote/playwrightServer.ts) |
| Crawlee | Lockstep (lerna fixed mode), exact pins | Independent: JS 3.18.2, Python 1.10.4 | Apify SDK's `checkCrawleeVersion()` |
| Stagehand v4 | TS 4.2.1, Python 4.2.1, Go `v4.2.1`; protocol package 2.1.1 | Numbers aligned but formally independent: "SDK, extension, and protocol packages are versioned independently … only `protocolVersion` … gates runtime compatibility" ([protocol README](https://github.com/browserbase/stagehand/blob/main/packages/protocol/README.md)) | "compatible when the client and server protocol majors match and the server protocol minor is greater than or equal to the client protocol minor"; implemented in [`protocol-version.ts`](https://github.com/browserbase/stagehand/blob/main/packages/protocol/protocol-version.ts) |
| Vercel AI SDK | Independent (Changesets with `fixed: []`, `linked: []`, [config](https://github.com/vercel/ai/blob/main/.changeset/config.json)), exact pins to `provider` packages | — | `specificationVersion`: `ai` 7 accepts `v4`, `v3` and `v2` and adapts them, otherwise `AI_UnsupportedModelVersionError` ([resolve-model.ts](https://github.com/vercel/ai/blob/main/packages/ai/src/model/resolve-model.ts)) |
| LangChain | Independent, `@langchain/core` as a semver peer | Independent | Semver ranges; 1.0 is LTS with a year of maintenance after 2.0 ([release-policy.mdx](https://github.com/langchain-ai/docs/blob/main/src/oss/release-policy.mdx)) |
| Prisma | Lockstep: "Every workspace package … carries the same `version`" ([versioning.md](https://github.com/prisma/orm/blob/main/docs/oss/versioning.md)) | — | CLI warns "Versions of prisma@… and @prisma/client@… don't match"; [prisma/orm#24382](https://github.com/prisma/orm/issues/24382) asks to make it an error. Today npm `latest` is 8.0.0-rc.22 for `prisma` and 7.10.0 for `@prisma/client`, so a bare install of both mismatches |
| Sentry | Lockstep in JS (11.7.0), exact pins | Independent: Python `sentry-sdk` 2.72.0 | Per-version global carrier |
| Astral | — | Independent products: `ruff` 0.17.0, `uv` 0.13.1, `ty` 0.0.87 | — |
| Biome, esbuild, Turborepo, Next.js | Binary packages pinned exactly to the wrapper | — | esbuild: "Host version … does not match binary version" at startup ([common.ts](https://github.com/evanw/esbuild/blob/main/lib/shared/common.ts)); Next.js warns "Mismatching @next/swc version" ([swc/index.ts](https://github.com/vercel/next.js/blob/canary/packages/next/src/build/swc/index.ts)) |

Client and server systems outside the field state skew windows instead of equal versions: `kubectl` is "supported within one minor version (older or newer) of `kube-apiserver`" ([version-skew policy](https://github.com/kubernetes/website/blob/main/content/en/releases/version-skew-policy.md)), and Docker clients negotiate down to an older daemon's API version ([Engine API](https://github.com/docker/docs/blob/main/content/reference/api/engine/_index.md)).

Release tools support both models: Changesets `fixed` groups bump and publish "even when there are no changes" ([fixed](https://changesets.dev/guide/fixed-packages)); release-plz has `version_group` ([config](https://github.com/release-plz/release-plz/blob/main/website/docs/config.md)); cargo-release has `shared-version` ([reference](https://github.com/crate-ci/cargo-release/blob/master/docs/reference.md)).

**What users report.** Duplicate copies pulled in by a dependency (Playwright [#37749](https://github.com/microsoft/playwright/issues/37749), [#13092](https://github.com/microsoft/playwright/issues/13092)); contract drift within a line (AI SDK [#18179](https://github.com/vercel/ai/issues/18179): `@ai-sdk/amazon-bedrock@5.x` stamps `v4` while `ai@5` accepts only `v2`); host and binary out of step (esbuild [#1806](https://github.com/evanw/esbuild/issues/1806), [#2300](https://github.com/evanw/esbuild/issues/2300), [#3431](https://github.com/evanw/esbuild/issues/3431)); client and server out of step (Playwright [#16506](https://github.com/microsoft/playwright/issues/16506), [#17535](https://github.com/microsoft/playwright/issues/17535)); ecosystem pins a major behind (Crawlee Python still requires `stagehand<4`).

---

## 5. Umbrella brands with sub-products

| Vendor | Sub-products | Package names | CLI | Docs and domain |
| --- | --- | --- | --- | --- |
| Browserbase | Stagehand (MIT SDK), Browse CLI, MCP, Director (hosted) | npm `@browserbasehq/stagehand`, PyPI `stagehand`, Go module under `browserbase/stagehand`; `@browserbasehq/mcp` | `browse`; Stagehand has none | stagehand.dev and docs.stagehand.dev; browserbase.com |
| Apify | Crawlee (Apache-2.0), Apify SDK, Apify CLI | npm `crawlee` + `@crawlee/*`, `apify`; PyPI `crawlee`, `apify` | `crawlee`, `apify` | crawlee.dev ("Built with 🤍 by Apify"), crawlee.dev/python; docs.apify.com |
| Astral | uv, Ruff, ty, pyx (hosted) | PyPI `uv`, `ruff`, `ty`, unscoped | `uv`, `uvx`, `ruff`, `ty` | astral.sh; docs.astral.sh/uv, /ruff, /ty |
| Vercel | Next.js, Turborepo, AI SDK, v0 | npm `next`, `turbo`, `ai` ("AI SDK by Vercel"), `v0-sdk` | `next`, `turbo` | nextjs.org, turborepo.dev, ai-sdk.dev, v0.app |
| Playwright | Playwright Test, Playwright MCP, Playwright CLI | `playwright`, `@playwright/test`, `@playwright/mcp`, `@playwright/cli` | `playwright`, `playwright-mcp`, `playwright-cli` | playwright.dev |
| HashiCorp | Terraform, Vault, Consul | one binary each | `terraform`, `vault` | developer.hashicorp.com/terraform; one tap, `brew install hashicorp/tap/terraform` ([install](https://developer.hashicorp.com/terraform/install)) |

- **Brand and binary names diverge where the brand is long.** The product is "Turborepo", the package and binary `turbo`; `uvx` is an alias of `uv tool run` ([docs](https://docs.astral.sh/uv/guides/tools/)).
- **Moves between subcommand and standalone binary keep happening, each with a shim.** Docker Compose went from the standalone `docker-compose` to the `docker compose` plugin, with [compose-switch](https://github.com/docker/compose-switch) translating old calls. `ruff-lsp` was archived for the built-in `ruff server` ([ruff-lsp](https://github.com/astral-sh/ruff-lsp)). The `gh copilot` extension was deprecated on 2025-10-25 for the standalone `copilot` binary ([gh-copilot](https://github.com/github/gh-copilot)). Prisma's new [prisma-cli](https://github.com/prisma/prisma-cli) is "one binary for the ORM, Composer, and the Prisma Developer Platform".
- **Renames cost even before release.** The Apify SDK split into `crawlee` and `apify` (v3, 2022), kept the major line, and left the duplicate-version overrides ([upgrading-to-v3](https://crawlee.dev/js/docs/upgrading/upgrading-to-v3)). Rome became Biome because the npm name `rome` belonged to someone else and many version numbers were taken ([announcement](https://biomejs.dev/blog/annoucing-biome/)). Turborepo's domain moved from turbo.build to turborepo.dev (all redirect today), and Turborepo's platform packages moved under a scope in 2.9.0.

---

## 6. Implications for Goodfetch

Considerations for the planning map; none of them is a decision.

- **Separate products would need qualified names on every registry.** Bare `collie` and `foxhound` are unavailable on npm and PyPI and unlikely to be freed: npm and crates.io do not transfer without the owner, PyPI's `foxhound` holder is a reachable organization, and npm `foxhound` is actively maintained. `goodfetch-collie` and `goodfetch-foxhound` (crates.io, PyPI, Homebrew) and `@goodfetch/collie` and `@goodfetch/foxhound` (npm) are all free, and an owned `@goodfetch` scope and `goodfetch` GitHub organization (hence `ghcr.io/goodfetch/*`) leave no squatting risk inside them. crates.io `foxhound` is the only bare name still free.
- **The base names are not reserved yet.** ADR 0034's Reservation consequence (npm scope, PyPI placeholder, crate, `goodfetch.dev`) is not done: today only the GitHub organization exists. PyPI and crates.io treat a functionless placeholder as name squatting under their policies (§2.2), which bears on how a placeholder is published.
- **Bare executables named `collie` or `foxhound` would collide on users' PATH.** ColliePWA puts `collie` on PATH for exactly the audience Goodfetch addresses (developers running AI coding agents), and ships the same `skill` and `docs` subcommands. `foxhound` is the name of SAP's Playwright-drivable browser and of a Go scraper's binary, and two npm packages already declare a `foxhound-mcp` bin. `goodfetch-collie` or `goodfetch-foxhound`, or `goodfetch collie …` subcommands as ADR 0034 has them, have no clash found.
- **Both sub-names meet registered software marks.** EUTM 018890716 covers "Software" in the EU without a field limit, and the same holder has filed COLLIE in the Benelux, the US and Canada. French mark 4860964 covers FOXHOUND for software development and SaaS. FOXHOUND has no live US mark in classes 9 or 42, but two small projects use the name for crawling with Camoufox and for agent-driven Firefox. ADR 0034's clearance and filings cover GOODFETCH only; promoting either sub-name to a product brand would put it in scope.
- **Separately installable products that share an engine have precedents, at the cost of duplicated code.** Ruff and ty (one repository, two binaries, two version lines) and oxlint and oxfmt (one repository, two sets of platform packages) show the pattern. Playwright shows the other one: `@playwright/test` depends on `playwright`, which depends on `playwright-core`, so one engine copy is shared when the exact versions match.
- **If Foxhound's binary contains Collie, the two-copies problem follows unless installs share one engine.** The precedents are `@playwright/mcp` (its own engine, mismatched browsers) and Stagehand's Python 3.x (an embedded server in every wheel, then dropped). The mitigations the leaders use are already in ADRs 0024 and 0030: one browser cache keyed by digest, one product version with exact pins, and a protocol handshake (Stagehand's majors-equal, server-minor-≥-client-minor rule is the one ADR 0024 already uses). What two installable products would add is a rule for which engine runs when both are installed, as Turborepo's global binary defers to the local one.
- **Lockstep versions remain the norm for packages that ship one engine.** Every vendor that wraps one native engine pins it exactly (Biome, esbuild, Turborepo, Next.js, Playwright's Python driver). Independent lines appear only where products meet over a separately versioned contract (Stagehand's protocol, the AI SDK's specification versions), and they bring a compatibility table or dist-tags such as `ai-v5`.
- **Presentation options seen in the field.** The Playwright shape (one brand, one scope, "Playwright X", a second binary only for a second audience) is closest to ADR 0034. The Crawlee shape (its own domain and unscoped package, "by Apify") is what a funnel product gets, and it is the one blocked by the bare-name availability above. The Astral shape (peer products with bare names) is unavailable for the same reason.

---

## 7. Unverified / open

- **Trademark coverage gaps.** The WIPO Global Brand Database was opened but its results did not load from a URL-built query, so it was not used; TMview, which covers the WIPO, US, Japanese, Chinese, Korean and European offices among others, was used instead. For COLLIE in classes 9 and 42, only the first 100 of 206 TMview results (sorted by relevance) were read. INPI's own page for FOXHOUND 4860964 answered with a bot check and was read through TMview. Offices outside TMview were not searched. **Unverified** beyond those registers.
- Whether the COLLIE and FOXHOUND software marks are in use (non-use matters for any challenge), and whether "Goodfetch Collie" or "Goodfetch Foxhound" would be found confusingly similar to them: a question for counsel.
- The goods of the Chinese, Egyptian and Japanese COLLIE marks were not read.
- That the EUTM owner "Collie" and Collie B.V. (Benelux, IR and US registrations) are one company is inferred from the shared city and representative, not from a register entry.
- The name of the executable inside SAP Foxhound's release archives (the archives are named `foxhound-<version>`; the archive was not downloaded).
- Whether ColliePWA is packaged in any distribution repository. Its README lists install guides for "Linux, Arch and Omarchy, Nix, macOS, Windows, or as a Herdr plugin"; Repology (which indexes the AUR and nixpkgs) and the AUR API show no `collie` package, and the guides themselves were not read.
- The `@foxhound-ai` packages' repository returns 404; the product behind them was not identified further.
- npm, GitHub and Docker Hub do not expose reserved or blocked names, so a 404 does not guarantee a name can be created.
- From the earlier attempt, not re-verified here: the npm release that shipped the [npm/cli#4828](https://github.com/npm/cli/issues/4828) fix, the internals of Playwright's .NET package, and the dates of the turbo.build domain moves.
- Register lag: filings from the last weeks may not be indexed by USPTO or EUIPO search.

---

## 8. Sources

Registries and APIs (all queried 2026-10-14): [npm registry](https://registry.npmjs.org/), [npm downloads API](https://api.npmjs.org/downloads/point/last-month/foxhound), [PyPI JSON API](https://pypi.org/pypi/collie/json), [pypistats](https://pypistats.org/api/packages/collie/recent), [crates.io API](https://crates.io/api/v1/crates/collie), [Homebrew formula API](https://formulae.brew.sh/api/formula.json), [Homebrew cask API](https://formulae.brew.sh/api/cask.json), [Homebrew executables index](https://formulae.brew.sh/api/internal/executables.txt), [sources.debian.org](https://sources.debian.org/api/src/collie/), [packages.debian.org](https://packages.debian.org/), [packages.ubuntu.com](https://packages.ubuntu.com/), [Repology](https://repology.org/api/v1/project/collie), [AUR RPC](https://aur.archlinux.org/rpc/v5/search/collie?by=name), [Arch packages](https://archlinux.org/packages/search/json/?name=collie), [anaconda.org](https://api.anaconda.org/search?name=collie), [Snapcraft](https://api.snapcraft.io/v2/snaps/info/collie), [RubyGems](https://rubygems.org/api/v1/gems/collie.json), [NuGet](https://api.nuget.org/v3-flatcontainer/collie/index.json), [JSR](https://api.jsr.io/scopes/goodfetch), [Docker Hub](https://hub.docker.com/v2/users/collie/), [GHCR](https://ghcr.io/), GitHub REST API, [Google Registry RDAP](https://pubapi.registry.google/rdap/domain/goodfetch.dev).

Trademark registers: [USPTO trademark search](https://tmsearch.uspto.gov/), [EUIPO eSearch plus](https://euipo.europa.eu/eSearch/) (EUTM [018890716](https://euipo.europa.eu/eSearch/#details/trademarks/018890716), [019140473](https://euipo.europa.eu/eSearch/#details/trademarks/019140473), [019333348](https://euipo.europa.eu/eSearch/#details/trademarks/019333348)), [TMview](https://www.tmdn.org/tmview/) (France [4860964](https://www.tmdn.org/tmview/#/tmview/detail/FR500000004860964)).

Registry policies: [PEP 541](https://peps.python.org/pep-0541/), [PEP 503](https://peps.python.org/pep-0503/), [crates.io policies](https://github.com/rust-lang/crates.io/blob/main/svelte/src/routes/policies/+page.svelte), [npm disputes policy](https://docs.npmjs.com/policies/disputes).

Projects holding or using the names: [fable-retold/foxhound](https://github.com/fable-retold/foxhound), [maichong/collie](https://github.com/maichong/collie), [fox-it/dissect](https://github.com/fox-it/dissect), [PyPI collie](https://pypi.org/project/collie/), [collie-reader](https://github.com/collie-reader), [ColliePWA](https://github.com/AltanS/collie), [the collie gem](https://github.com/ydah/collie), [meshcloud/collie-cli](https://github.com/meshcloud/collie-cli), [SAP/project-foxhound](https://github.com/SAP/project-foxhound), [Foxhound (Go)](https://github.com/sadewadee/foxhound), [FoxHound browser bridge](https://github.com/amrelsagaei/FoxHound), [OpenMOSS/CoLLiE](https://github.com/OpenMOSS/CoLLiE), [princeton-nlp/Collie](https://github.com/princeton-nlp/Collie), [chaitin/collie](https://github.com/chaitin/collie).

Packaging and versioning: linked inline in §3 to §5 (Playwright, playwright-python and playwright-java repositories and docs; apify/crawlee, apify/apify-sdk-js; browserbase/stagehand and stagehand-python; vercel/ai; langchain-ai/docs and langchainjs; prisma/prisma, prisma/orm and prisma.io changelog; biomejs/biome; evanw/esbuild; vercel/turborepo; getsentry/sentry-javascript, sentry-cli and sentry-docs; astral-sh/uv and ruff-lsp; vercel/next.js; angular/angular-cli; kubernetes/website; docker/docs and compose-switch; github/gh-copilot; prisma/prisma-cli; Changesets, release-plz and cargo-release docs; npm/cli issues).

Repository context: ADRs 0024, 0030, 0032, 0034, 0039 and 0057; the [Automan name research](https://github.com/Jbmanllr/automan/blob/research/name-availability/rnd/research/2026-10-10-name-availability.md); the [release and versioning research](https://github.com/Jbmanllr/automan/blob/research/release-and-docs/rnd/research/2026-10-11-release-and-docs/release-versioning-support.md).
