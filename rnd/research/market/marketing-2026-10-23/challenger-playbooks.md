# Challenger playbooks: how developer tools took users from incumbents

Research date: 2026-10-23. Read-only web research for Goodfetch (Rust engine; TS/Python/Rust SDKs; claims are correctness, not speed). Target incumbents: Scrapy, Crawlee, spider-rs, Firecrawl, Stagehand.

Evidence tags used below:
- **[P]** primary source (launch post, release notes, official docs, HN thread, survey, registry/API pulled today).
- **[S]** secondary source (press, third-party blog). Treat as weaker.
- **[I]** my inference from the sources, not stated by any of them.
- **[U]** unverified or contradictory; do not rely on it.

## 0. Method and data caveats

- Star counts: GitHub REST API, pulled 2026-10-23 [P] (`https://api.github.com/repos/<owner>/<repo>`).
- npm weekly downloads: npm registry API (`https://api.npmjs.org/downloads/range/...`), averaged over the last 28 days of each window ending 22 October [P]. **The 2026-10 column is 2x to 10x the 2025-10 column for nearly every package (playwright 21.9M to 88.4M, pnpm 32.9M to 168.7M, eslint 63.2M to 129.0M) while jest rose only 11% (32.8M to 36.4M). A common cause (CI, mirror or agent install traffic, or a counting change) is plausible, so treat the 2026 point as [U] and prefer ratios from 2021 to 2025.**
- PyPI downloads: `https://pypistats.org/api/packages/<pkg>/recent`, pulled 2026-10-23 [P]. PyPI counts include CI and mirrors; they measure installs, not users.
- Download and star counts measure attention, not production conversion. Where I could only find vendor-run benchmarks I say so.
- Some "2026" comparison blogs that surfaced in search are SEO-grade [S]; I used them only where flagged.

### Snapshot (2026-10-23)

| Tool | GitHub stars | First commit/creation | Source |
|---|---|---|---|
| uv | 91,087 | 2023-10-02 | https://api.github.com/repos/astral-sh/uv |
| Ruff | 50,246 | 2022-08-09 | https://api.github.com/repos/astral-sh/ruff |
| Bun | 96,362 | 2021-04-14 | https://api.github.com/repos/oven-sh/bun |
| Deno | 108,733 | 2018-05-15 | https://api.github.com/repos/denoland/deno |
| Biome | 25,989 | 2023-07-27 | https://api.github.com/repos/biomejs/biome |
| Playwright | 98,380 | 2019-11-15 | https://api.github.com/repos/microsoft/playwright |
| Vitest | 17,278 | 2021-12-03 | https://api.github.com/repos/vitest-dev/vitest |
| Polars | 40,173 | 2020-05-13 | https://api.github.com/repos/pola-rs/polars |
| pnpm | 36,912 | 2016-01-28 | https://api.github.com/repos/pnpm/pnpm |
| Tauri | 112,149 | 2019-07-13 | https://api.github.com/repos/tauri-apps/tauri |
| Electron | 123,507 | 2013-04-12 | https://api.github.com/repos/electron/electron |
| OpenTofu | 30,590 | 2023-08-16 | https://api.github.com/repos/opentofu/opentofu |
| Valkey | 27,499 | 2024-03-22 | https://api.github.com/repos/valkey-io/valkey |
| Firecrawl | 194,666 | 2024-04-15 | https://api.github.com/repos/firecrawl/firecrawl |
| Browser Use | 118,982 | 2024-10-31 | https://api.github.com/repos/browser-use/browser-use |
| Crawlee (JS) | 26,237 | 2016-08-26 (as Apify SDK) | https://api.github.com/repos/apify/crawlee |
| Crawlee for Python | 9,617 | 2024-01-10 | https://api.github.com/repos/apify/crawlee-python |
| Scrapy | 64,981 | 2010-02-22 | https://api.github.com/repos/scrapy/scrapy |
| Stagehand | 25,791 | 2024-03-24 | https://api.github.com/repos/browserbase/stagehand |
| spider-rs/spider | 2,794 | 2018-01-07 | https://api.github.com/repos/spider-rs/spider |

PyPI downloads, last month / last week (pypistats, 2026-10-23) [P]: uv 127.8M / 35.4M; ruff 304.0M / 89.4M; pandas 546.9M / 147.0M; polars 50.7M / 14.2M; poetry 40.4M / 10.1M; playwright 97.0M / 28.4M; selenium 28.0M / 7.3M; **browser-use 8.06M / 1.52M; firecrawl-py 5.34M / 1.33M; scrapy 4.30M / 1.20M; stagehand 1.27M / 0.47M; crawlee 0.82M / 0.26M.** (https://pypistats.org/api/packages/browser-use/recent and the same path for each package.)

### npm weekly downloads (npm registry API, average of the last 28 days of each window ending 22 October; millions) [P]

Source for every row: `https://api.npmjs.org/downloads/range/<start>:<end>/<package>` pulled 2026-10-23.

| Package | 2021 | 2022 | 2023 | 2024 | 2025 | 2026 (flagged [U]) |
|---|---|---|---|---|---|---|
| playwright | 0.31 | 0.88 | 2.19 | 7.97 | 21.9 | 88.4 |
| puppeteer | 2.68 | 3.78 | 5.01 | 3.81 | 5.89 | 9.14 |
| selenium-webdriver | 2.10 | 2.82 | 1.84 | 1.67 | 1.62 | n/a (rate-limited) |
| vitest | 0.00 | 0.53 | 2.77 | 6.46 | 16.7 | 97.0 |
| jest | 13.9 | 18.6 | 22.0 | 24.1 | 32.8 | 36.4 |
| pnpm | 0.31 | 1.86 | 7.97 | 16.6 | 32.9 | 168.7 |
| @biomejs/biome | 0 | 0 | 0.07 | 1.14 | 3.20 | 14.1 |
| eslint | 19.9 | 27.8 | 34.4 | 41.5 | 63.2 | 129.0 |
| prettier | 17.6 | 25.2 | 31.8 | 39.6 | 56.2 | 109.9 |
| electron | 0.53 | 0.59 | 0.86 | 0.73 | 1.71 | 6.45 |
| @tauri-apps/cli | 0.002 | 0.013 | 0.108 | 0.082 | 0.201 | 2.14 |
| crawlee | 0 | 0.003 | 0.017 | 0.023 | 0.044 | 0.125 |
| @mendable/firecrawl-js | 0 | 0 | 0 | 0.013 | 0.212 | 0.940 |

Reading rules: (1) the 2026 column is 2x to 10x the 2025 column for every package except jest (+11%) and puppeteer (+55%), which is more consistent with a registry-wide counting or traffic change (CI, mirror, agent installs) than with real adoption, so use 2021-2025 only [U]; (2) selected 2025 ratios: vitest/jest 0.51; biome/eslint 0.05; @tauri-apps/cli/electron 0.12 (different package roles, rough); playwright/selenium-webdriver 13.5; (3) `@mendable/firecrawl-js` is the legacy package name of the Firecrawl JS SDK; the SDK may also ship as `firecrawl`, so this undercounts [U].


---

## 1. Cases

Order: tooling rewrites (uv, Ruff, Bun/Deno, Biome/Rome), reliability/DX challengers (Playwright, Vitest, pnpm), architecture challengers (Polars, Tauri), license-born forks (OpenTofu, Valkey), and the crawler/agent field (Firecrawl, Browser Use, Crawlee, plus Stagehand as adjacent evidence).

### 1.1 uv vs pip / pip-tools / Poetry (Astral)

**(a) Wedge and message.**
- Launch post, 2024-02-15: "uv is an extremely fast Python package resolver and installer, written in Rust ... designed as a drop-in replacement for `pip` and `pip-tools`, and is ready for production use today in projects built around those workflows." https://astral.sh/blog/uv [P]
- Same post, on tactics: "Optimized for adoption ... uv's initial release is centered on supporting the `pip` and `pip-tools` APIs behind our `uv pip` interface, making it usable by existing projects with zero configuration." https://astral.sh/blog/uv [P]
- Second act, 2024-08-20: "uv is an extremely fast Python package manager ... It's Cargo, for Python." https://astral.sh/blog/uv-unified-python-packaging [P]
- Today's tagline: "A single tool to replace pip, pip-tools, pipx, poetry, pyenv, twine, virtualenv, and more. 10-100x faster than pip." https://docs.astral.sh/uv [P]

**(b) Tactics.**
- Drop-in `uv pip` surface first, then project/lockfile/Python-install features in later releases (Feb 2024 to Aug 2024) [P] (same two blog posts).
- Benchmark chart in the README and a `BENCHMARKS.md` file; early PyPI description already said "Tested at-scale against the top 10,000 PyPI packages" https://pypi.org/project/uv/0.1.11/ [P].
- Brand borrowed from Ruff ("backed by Astral, the creators of Ruff") https://pypi.org/project/uv/0.1.11/ [P].
- Migration tooling: third-party `migrate-to-uv` converts Poetry/Pipenv/pip-tools/pip projects (`uvx migrate-to-uv`) https://github.com/osprey-oss/migrate-to-uv [P]; uv also documents its pip deviations in one page https://docs.astral.sh/uv/pip/compatibility [P].
- Stewardship of an adjacent project: Rye was taken over in the 2024-02 post https://astral.sh/blog/uv [P].
- Launch venue: Astral blog plus Twitter/X (`@charliermarsh`); I did not retrieve a launch-day HN thread, so I do not claim one [U].

**(c) Effect.**
- Jane Street talk: "it's at 16 million downloads a month" (the talk's date is not stated on the page; given the Aug 2024 post it was after Feb 2024, I infer 2024 H2 [U]) https://www.janestreet.com/tech-talks/uv-an-extremely-fast-python-package-manager [P].
- Astral, 2025-08-13: "over 100 million installs per month across the Astral toolchain and uv powering over 500 million requests per day" https://astral.sh/blog/introducing-pyx [P].
- Wagtail download data: uv became the second most used installer for Wagtail after pip, overtaking Poetry by Mar 2025 (about 13 months after launch) https://wagtail.org/blog/uv-overtakes-poetry [P]. By Sep 2025 uv was 28% of Wagtail downloads (139k of 480k) and 66% of Wagtail's CI downloads; uv was 31% of FastAPI downloads and 60% of FastAPI CI downloads https://wagtail.org/blog/uv-overtakes-pip-in-ci [P].
- Stack Overflow Developer Survey 2025: uv "the most admired (74%) SO tag technology this year" https://survey.stackoverflow.co/2025/technology [P].
- PyPI today: uv 127.8M downloads/month vs poetry 40.4M (https://pypistats.org/api/packages/uv/recent, https://pypistats.org/api/packages/poetry/recent) [P].

**(d) What backfired or got friction.**
- "Drop-in" with deliberately different defaults: Plotly found `uv pip install -r requirements.txt --extra-index-url ...` resolves differently from pip (uv picks the first index that has the package, to prevent dependency confusion), which surprised a pip-compat user base https://plotly.com/blog/uv-python-package-manager-quirks and https://docs.astral.sh/uv/pip/compatibility [P]. Lesson: every intentional deviation needs a documented, searchable page.
- uv's initial lockfile was not machine-agnostic ("does not yet produce a machine-agnostic lockfile") https://pypi.org/project/uv/0.1.16 [P]; this limited Poetry converts until the Aug 2024 project mode. 

**(e) Time to conversion.** Roughly 6 to 10 months to 16M downloads/month [U on date]; about 13 months to overtake Poetry in a major package's download mix; about 19 months to 31% of FastAPI downloads (sources above) [P]. Fastest conversion in this set.

---

### 1.2 Ruff vs Flake8 / Black / isort (Astral)

**(a) Wedge and message.**
- Original announcement (Aug 2022): "As a proof-of-concept, I'm releasing Ruff, an extremely fast Python linter, written in Rust." and "Ruff is ~150x faster than Flake8 on macOS (~25x faster if you hack Flake8 to enable multiprocessing)" https://notes.crmarsh.com/python-tooling-could-be-much-much-faster [P].
- It opens with an anti-claim: "(Ruff is not a drop-in replacement for those other tools. It's not even production-ready - it's "rough". See "What ruff is missing" below.)" and "What Ruff is missing: Mostly: lint rules ... Unlike Flake8 and others, Ruff doesn't support plugins." Same URL [P].
- Thesis: "An integrated toolchain could read every file exactly once, generate the AST exactly once" (the "replaces N tools" argument, made on day one). Same URL [P].
- Formatter post, 2023-10-24: "Optimized for adoption. Ruff's formatter is designed as a drop-in replacement for Black ... >99.9% compatibility with Black" https://astral.sh/blog/the-ruff-formatter [P].

**(b) Tactics.**
- Benchmarks on CPython and pip's code base, with the multiprocessing caveat disclosed (above) [P].
- Rule-parity as the roadmap: "We launched with something like 20 lint rules. Today, Ruff supports 376 lint rules ... near-complete coverage across the most popular Flake8 plugins" https://notes.crmarsh.com/ruff-the-first-200-releases [P]. Now "over 900 built-in rules" https://github.com/astral-sh/ruff [P].
- Migration tool: `flake8-to-ruff` ("a tool to automate the migration from Flake8") https://notes.crmarsh.com/ruff-the-first-200-releases [P].
- Ecosystem integration: `pyproject.toml` config from day one, editor integrations, Black-compatible linter rules https://notes.crmarsh.com/python-tooling-could-be-much-much-faster and https://docs.astral.sh/ruff/faq [P].
- Measured compatibility on named projects: "When run over extensive Black-formatted projects like Django and Zulip, > 99.9% of lines are formatted identically." https://docs.astral.sh/ruff/formatter [P]; known deviations enumerated at https://docs.astral.sh/ruff/formatter/black [P].
- Launch venue: personal blog plus a Hacker News thread (id 32666035), where Marsh wrote "I don't consider it production-ready, it's only a proof-of-concept" (quoted by https://analyticsindiamag.com/ai-trends/this-rust-based-python-linting-tool-is-quick-as-lightning [S]; the HN item id is from the same article, I did not load the thread [U]).
- Reference adopters courted early: Pandas, FastAPI, Apache Airflow, Bokeh, Zulip, Pydantic (the N.B. on the original post and https://thenewstack.io/the-python-linter-ruff-is-a-win-for-open-source-and-rust) [P/S].

**(c) Effect.**
- Five months after launch Ruff was the primary linter for Pandas, FastAPI, Apache Airflow https://thenewstack.io/the-python-linter-ruff-is-a-win-for-open-source-and-rust [S].
- By 2023-04-18: "Since August, Ruff has blown past one million monthly downloads and 12,000 stars" https://astral.sh/blog/announcing-astral-the-company-behind-ruff [P].
- Today: 304.0M PyPI downloads/month, 50k stars (pypistats and GitHub API above) [P].

**(d) What backfired.** Early crashes and missing rules were disclosed up front, which limited backlash [P]. The risk named by the author himself: Rust makes contribution harder for Python developers and failures yield no Python trace (the "Tradeoffs" list in the original post) [P]. Formatter deviations from Black remain a documented migration cost https://docs.astral.sh/ruff/formatter/black [P].

**(e) Time to conversion.** About 5 months to flagship OSS projects; about 8 months to 1M monthly downloads; formatter (the second wedge) arrived 14 months after the linter (Aug 2022 to Oct 2023) [P].

---

### 1.3 Bun vs Node (and Deno as the contrast)

**(a) Wedge and message.**
- Bun 1.0 (2023-09-08): "Bun is a drop-in replacement for Node.js" and "While perfect compatibility with Node.js isn't possible - looking at you `node:v8` - Bun can run virtually any Node.js application in the wild." https://bun.com/blog/bun-v1.0 [P].
- "Bun is a single integrated toolkit that avoids these integration problems" (replaces node, npx, dotenv, cross-env, jest, bundlers) and speed claims: "13x faster" vs zod's test suite under Jest, "1.75x faster than esbuild" (vendor benchmarks). Same URL [P].
- Today: "Four tools, designed together. Adopt one, or all of them. `bun install` and `bun test` drop into existing Node.js projects. No runtime switch required." https://bun.com/ [P]. The fall-back to an incremental wedge (package manager first) is the notable shift.

**(b) Tactics.**
- Launch on HN: Bun v1.0.0 reached 1,089 points https://news.ycombinator.com/item?id=37434117 [P]. "20k GitHub stars in the first week" (Bun's own account; I infer it refers to the Jul 2022 v0.1 release) https://bun.com/blog/bun-joins-anthropic [P/U].
- Maintainer answered questions in the thread and filed compat fixes publicly ("Please file an issue ... include the code to reproduce it") [P, HN thread above].
- Bun 1.2 (2025-01-22): changed how compat is tested, "running the Node.js test suite for every change made to Bun"; 11 Node modules pass more than 90% of Node's own tests https://www.infoworld.com/article/3809862/bun-1-2-squashes-node-js-compatibility-bugs.html and https://bun.com/blog/bun-v1.2 [P/S].
- Public compat page with a standing promise: "If a module works in Node.js but doesn't work in Bun, we consider it a bug in Bun." https://bun.com/docs/runtime/nodejs-compat [P].
- Windows support delivered late: experimental at 1.0, tracked as a percentage of Bun's test suite ("Bun for Windows currently passes 92.51%") https://bun.sh/blog/bun-v1.0.31 [P].

**(c) Effect.**
- Anthropic's acquisition post, 2025-12-03: "Bun gets more than 7 million monthly downloads, has earned over 82,000 stars on GitHub, and has been adopted by companies like Midjourney and Lovable" https://www.anthropic.com/news/anthropic-acquires-bun-as-claude-code-reaches-usd1b-milestone [P]. Bun powers Claude Code's native build (same post) [P].
- Bun's own "Bun is joining Anthropic" post lists production users X and Midjourney and Tailwind's standalone CLI https://bun.com/blog/bun-joins-anthropic [P].
- Compare npm scale: pnpm alone is about 33M weekly downloads in 2025-10 (npm API table above), so Bun's runtime is a niche next to Node [I].

**(d) What backfired.**
- HN comments on 1.0 on the "drop-in" claim: "The only obviously bad thing is that the 'drop-in replacement' is bs: they're aiming for it but there are a bunch of node std lib modules missing." (jrvarela56); "I am annoyed with the bombastic claims behind bun." (tarasglek); a user list: "Missing node:http2 ... Vite and ESLint do not work. Still occasionally segfaults ... some code runs slower in Bun than in Node" and "it's not a 10x slam dunk like the benchmarks on Bun's homepage imply" (e1g) https://news.ycombinator.com/item?id=37434117 [P].
- Segfaults still appear as open issues in 2026 (e.g. https://github.com/oven-sh/bun/issues/31832) [P].
- Third-party testing shows native addons remain "the main breakage point" and Bun's HTTP lead shrinks outside Bun's own benchmark https://strapi.io/blog/bun-vs-nodejs-performance-comparison-guide [S].

**(e) Time to conversion.** v0.1 (Jul 2022) to 1.0 (Sep 2023) to 1.2's compat rework (Jan 2025) to about 7M monthly downloads (Dec 2025): 3.5 years, still a minority runtime [P].

#### Deno as the failure contrast (compat refused, then reversed)
- Deno 1.0 (2020-05-13): "Deno is not compatible, in general, with Node (NPM) packages ... Unfortunately, many users will find a frustrating lack of compatibility" https://deno.com/blog/v1 [P].
- Reversal: "make 80-90% of npm packages work in Deno within the next three months" (Big Changes Ahead) https://deno.com/blog/changes [P]; npm support stabilized in Deno 1.28 https://deno.com/blog/v1.28 [P]; Deno 2 (2024-10-09): "backwards compatibility with Node.js and npm" https://deno.com/blog/v2.0 [P].
- Dahl, 2024: "There's always going to be a long tail of incompatibilities and getting to 100% is essentially impossible" https://stackoverflow.blog/2024/08/20/ryan-dahl-deno-20-scale-improve-npm-nodejs [P].
- Today's homepage: "Deno, the drop-in JavaScript runtime for Node developers ... fully Node-compatible" https://deno.com/ [P]. 108.7k stars vs Node-compat winner Bun 96.4k (GitHub API) [P]; stars do not equal adoption.
- Time: 4.5 years from 1.0 to a compatibility-first message (2020-05 to 2024-10) [P].

---

### 1.4 Biome vs ESLint / Prettier, and Rome Tools' collapse

**(a) Wedge and message.**
- Biome launch (2023-08-29): "We are happy to announce Biome, toolchain of the web ... Biome is the official fork of Rome ... led and maintained by the same people that maintained Rome so far." https://biomejs.dev/blog/announcing-biome [P].
- Wedge today: "a Prettier-compatible formatter" that "matches Prettier's output in 97% of cases", and a linter that "ports most rules from ESLint". "If your project already uses either tool, run `biome migrate eslint` or `biome migrate prettier`. Each command reads the existing configuration and writes the equivalent `biome.json`." https://biomejs.dev/introduction/why-biome/ [P].

**(b) Tactics.**
- Compatibility as a public contest: Biome won the "Prettier challenge" launched by Christopher Chedeau (Prettier's co-creator): "Before the challenge, Biome had roughly a compatibility rate of 85%"; after, "over 96%" (v1.4, 2023-11-27) https://biomejs.dev/blog/biome-wins-prettier-challenge [P]. v1.5 (2024-01-08) reached 97% https://biomejs.dev/internals/changelog_v1 [P].
- Migration commands (`biome migrate eslint`, `biome migrate prettier`) added in 1.x changelog [P]; the "Differences with Prettier" page lists intentional divergences (tabs by default, stricter parser) https://biomejs.dev/formatter/differences-with-prettier [P].
- Rule-name mapping to ESLint ("ESLint's `eqeqeq` is Biome's `noDoubleEquals`") https://biomejs.dev/introduction/comparison-with-other-tools [P].
- Format malformed code while typing, editor-first design (npm description) https://www.npmjs.com/package/@biomejs/biome [P].

**(c) Effect.** npm weekly downloads of `@biomejs/biome`: 0.07M (2023-10), 1.14M (2024-10), 3.20M (2025-10) vs eslint 34.4M, 41.5M, 63.2M and prettier 31.8M, 39.6M, 56.2M (npm API table above) [P]; the npm page shows 18.0M in 2026 https://www.npmjs.com/package/@biomejs/biome [P, flagged with the 2026 column]. 25,989 stars (GitHub API) [P]. Biome is growing about 3x a year but is about 5% of ESLint's base after two years.

**(d) What backfired (Rome).**
- Rome announced a monolith: it would "replace Babel, ESLint, webpack, Prettier, Jest, and others"; in May 2021 it raised $4.5M with only a beta linter shipped; Sep 2021 it began a full Rust rewrite; v10 shipped Nov 2022, 14 months after the rewrite began, and "all Rome employees were laid off within a few months" https://bytes.dev/archives/219 [S] (corroborated by Biome's own telling at https://biomejs.dev/blog/announcing-biome [P]).
- The npm name `rome` belonged to another person, so the team "always struggled with versioning" and had to rename https://biomejs.dev/blog/announcing-biome [P].
- Biome's correction: narrower scope first (formatter + linter), compat with the incumbent measured in public, and migration commands.

**(e) Time to conversion.** Fork Aug 2023; 97% Prettier parity Jan 2024 (5 months); about 2 years later 3.2M weekly downloads vs ESLint's 63.2M (2025 window) [P].

---

### 1.5 Playwright vs Selenium / Puppeteer (and Cypress)

**(a) Wedge and message.**
- InfoQ, 2020-01-30: "Playwright is an open-source Node.js library started by Microsoft for automating browsers based on Chromium, Firefox, and WebKit through a single API." The team: "We are the same team that originally built Puppeteer at Google, but has since then moved on." and expected "migration between the Puppeteer and Playwright should be straightforward." https://www.infoq.com/news/2020/01/playwright-browser-automation [P/S].
- Reliability message: "Playwright automatically waits for the UI to be ready, which ensures tests are reliable to execute and simpler to author." https://devblogs.microsoft.com/python/announcing-playwright-for-python-reliable-end-to-end-testing-for-the-web [P].
- Homepage now: "Playwright enables reliable web automation for testing, scripting, and AI agents. One API to drive Chromium, Firefox, and WebKit". "No artificial timeouts, no flaky tests." https://playwright.dev/ [P].

**(b) Tactics.**
- Credibility transfer: same authors as the incumbent (Puppeteer) [P].
- Near-identical API plus an official "Migrating from Puppeteer" cheat sheet: "Most Puppeteer APIs can be used as is" https://playwright.dev/docs/puppeteer [P].
- Correctness made visible: auto-waiting/actionability checks ("locator resolves to exactly one element, visible, stable, receives events, enabled") https://playwright.dev/docs/actionability [P]; Trace Viewer with DOM snapshots, network, console per step https://playwright.dev/docs/trace-viewer [P]; codegen that suggests role/text/test-id locators https://playwright.dev/docs/codegen-intro [P].
- Language SDKs (Python Sept 2020, then .NET, Java) to reach Selenium's multi-language base https://devblogs.microsoft.com/python/announcing-playwright-for-python-reliable-end-to-end-testing-for-the-web [P].
- Launch venue: GitHub/Microsoft channels and press; I did not retrieve a launch HN thread [U].

**(c) Effect (npm API, weekly downloads, last 28 days to 22 Oct) [P].**

| Window | playwright | puppeteer | selenium-webdriver |
|---|---|---|---|
| 2020-10 | 0.10M | 1.67M | 1.75M |
| 2021-10 | 0.31M | 2.68M | 2.10M |
| 2022-10 | 0.88M | 3.78M | 2.82M |
| 2023-10 | 2.19M | 5.01M | 1.84M |
| 2024-10 | 7.97M | 3.81M | 1.67M |
| 2025-10 | 21.9M | 5.89M | 1.62M |

- Playwright passed selenium-webdriver on npm between the 2022 and 2023 windows and passed puppeteer between the 2023 and 2024 windows; Selenium's npm base shrank from 2.8M (2022) to 1.6M (2025) [P, my reading of the table].
- State of JS 2025 testing page: "Jest has long enjoyed a comfortable spot as the most-used testing tool" while Vitest and Playwright are "climbing the ranks" https://2025.stateofjs.com/en-US/libraries/testing [P].
- PyPI: playwright 97.0M/month vs selenium 28.0M/month (pypistats) [P].

**(d) What backfired or limited it.** Not a drop-in for Selenium; migration is a rewrite, and testguild's practitioner survey says teams ask "how do I migrate without breaking everything?" (Playwright 71 vs Selenium 50 users in that sample) https://testguild.com/automation-testing-trends [S]. Stagehand later argued Playwright's auto-wait "add[s] overhead in automation scenarios where throughput and control take priority" https://www.browserbase.com/blog/stagehand-v3 [P], which is relevant: Playwright's correctness defaults are test-oriented, not crawl-oriented.

**(e) Time to conversion.** Launch Jan 2020; passed selenium-webdriver on npm between the 2022 and 2023 windows (about 3 to 3.75 years); passed puppeteer between the 2023 and 2024 windows (about 3.75 to 4.75 years) [P, my reading of the table]. Slow, steady, reliability-driven.

---

### 1.6 Vitest vs Jest

**(a) Wedge and message.** "Next generation testing framework powered by Vite." "Vitest has been designed with a Jest compatible API, in order to make the migration from Jest as simple as possible." https://github.com/vitest-dev/vitest and https://vitest.dev/guide/migration [P]. Site: "By building on top of Vite, Vitest natively understands your Vite config and is able to reuse the same resolve and transform pipelines ... You can also use Vitest even if you are not using Vite. It is Jest-compatible and works for backend code too." https://vitest.dev/ [P].

**(b) Tactics.**
- API compat: `expect`, snapshots, mocking/spies ("Jest-compatible mocking, stubbing, and spies") [P].
- Zero extra config for the Vite audience: reads `vite.config.*` by default so existing plugins "work out-of-the-box" https://vitest.dev/guide [P].
- Documented gaps in a Jest migration page: globals off by default, legacy timers unsupported, `jest.mock` becomes `vi.mock` https://vitest.dev/guide/migration [P].
- Distribution by an existing ecosystem (Vite/Vue/Nuxt/Anthony Fu), with funding from the Vite Ecosystem Fund https://opencollective.com/vitest [P].

**(c) Effect.** npm weekly downloads (npm API): vitest 0.53M (2022-10), 2.77M (2023-10), 6.46M (2024-10), 16.7M (2025-10) vs jest 18.6M, 22.0M, 24.1M, 32.8M [P]. The ratio went from 0.03 to 0.51 in three years while Jest kept growing. State of JS 2025: "Vitest is climbing the ranks so fast that it wouldn't be surprising to see it overtake it in the upcoming year, especially with Jest's satisfaction ratio trending down." https://2025.stateofjs.com/en-US/libraries/testing [P]. A 2026 comparison reports parity of monthly downloads (45.1M vs 44.8M, May 2026) https://codeoxi.com/blog/vitest-vs-jest [S], but that conflicts with the pull above for the Jest side [U].

**(d) What backfired.** Vitest was not a perfect Jest clone; the migration guide itself lists breaking diffs [P]. Other runtimes' breakage with Vitest was visible (Bun 1.0 users: "Vitest ... remains broken in v1.0" https://news.ycombinator.com/item?id=37434117 [P]), showing how a new tool's compat reputation is hostage to its neighbours.

**(e) Time to conversion.** Created Dec 2021; about 2.8M weekly downloads at 22 months; about 16.7M at 46 months; still behind Jest in raw installs [P].

---

### 1.7 Polars vs pandas

**(a) Wedge and message.** Polars 1.0 post (2024-07-01): "Since its inception four years ago, Polars has gained 27.5K GitHub stars, has over 7 million monthly downloads" https://pola.rs/posts/announcing-polars-1/ [P]. Vink: "Our initial (and achieved) goal was offering an alternative for pandas with a flexible API that does enable query optimization, and parallel streaming execution." https://pola.rs/posts/polars-cloud-what-we-are-building [P]. Homepage: "Compared to pandas, it can achieve more than 30x performance gains." https://pola.rs/ [P].

**(b) Tactics.**
- Deliberately not a drop-in: the official "Coming from pandas" guide says "polars != pandas ... If your Polars code looks like it could be pandas code, it might run, but it likely runs slower than it should" and "Polars does not have a multi-index/index" https://docs.pola.rs/user-guide/migration/pandas/ [P].
- Benchmarks: own PDS-H suite (open source, derived from TPC-H and labelled "not comparable to published TPC-H Benchmark results") https://pola.rs/posts/benchmarks [P]; independent DuckDB Labs db-benchmark cited by JetBrains https://blog.jetbrains.com/pycharm/2024/07/polars-vs-pandas/ [S].
- Lazy API/optimizer as the second reason beyond raw speed [P].
- Company and cloud layered later (seed of about $4M in 2023 per https://en.wikipedia.org/wiki/Polars_(software) [S]; $21M Series A Sep 2025 per https://techcrunch.com/2025/09/29/the-startup-behind-open-source-tool-polars-raises-21m-from-accel/ [S]).

**(c) Effect.** JetBrains Python Developers Survey 2024: tools for data exploration and processing: pandas 80%, NumPy 75%, Spark 16%, **Polars 15%** https://lp.jetbrains.com/python-developers-survey-2024 [P]. PyPI: polars 50.7M/month vs pandas 546.9M/month (pypistats) [P], about 9%. 40k stars.

**(d) What backfired / limits.** Not drop-in means migration cost is real; a Hacker News commenter described a failed pandas-to-Polars migration of a large codebase (thread 38920043; I could only see the first line of the comment, so I do not quote more) https://news.ycombinator.com/item?id=38920043 [U]. JetBrains: "Ecosystem: Pandas integrates more broadly with Python tools, ML libraries, and plotting frameworks." https://blog.jetbrains.com/pycharm/2024/07/polars-vs-pandas/ [S]. The leading benchmark is vendor-run.

**(e) Time to conversion.** First commit Jun 2020; 1.0 Jul 2024; about 15% survey share at 4 years; about 9% of pandas' PyPI volume at 6 years. Additive adoption (new pipelines, large data), not displacement [P/I].

---

### 1.8 pnpm vs npm / Yarn

**(a) Wedge and message.** "Fast, disk space efficient package manager" (site/npm title). Bullets: Fast, Efficient ("Files inside node_modules are linked from a single content-addressable storage"), "Strict. A package can access only dependencies that are specified in its package.json", "Deterministic" lockfile, "Great for monorepos" https://www.npmjs.com/package/pnpm [P]. Today: "pnpm is a drop-in replacement for npm - and then keeps going" https://pnpm.io/ [P].

**(b) Tactics.**
- Uses the npm registry and `package.json` unchanged (drop-in at the data layer) [P].
- Social proof by named flagship projects: Next.js, Vite, Vue, Nuxt, Astro, Prisma, Turborepo on the homepage https://pnpm.io/ [P]; Microsoft quote on the npm page: "Microsoft uses pnpm in Rush repos with hundreds of projects and hundreds of PRs per day, and we've found it to be very fast and reliable." https://www.npmjs.com/package/pnpm [P].
- Escape hatches for strictness: `shamefully-hoist`, `public-hoist-pattern` (added 2020 after "many stacks that don't work well with pnpm without shamefully-hoist=true") https://github.com/pnpm/pnpm/issues/2628 [P].
- Monorepo/workspace features as the adoption driver (div RIOTS migration story: "it also comes with out of the box support for workspaces") https://divriots.com/blog/switching-to-pnpm [S].

**(c) Effect (npm API, weekly downloads).** 0.31M (2021-10), 1.86M (2022-10), 7.97M (2023-10), 16.6M (2024-10), 32.9M (2025-10) [P]. The 2026-10 reading of 168.7M is implausibly steep next to the previous years; flagged [U]. Registry-level share vs npm itself is not published by npm, so I make no share claim.

**(d) What backfired.** Strictness exposes undeclared dependencies in the ecosystem ("phantom dependencies"): migrations still break UI libraries and APM agents and need hoist patterns (practitioner accounts: https://dev.to/jtorchia/pnpm-vs-npm-vs-yarn-in-2026-i-ran-all-three-on-my-real-monorepo-and-it-forced-me-to-change-my-mind-9mc [S], https://danieljoffe.com/blog/pnpm-phantom-dependencies [S]). InfoQ reports pnpm "removed Bun and Yarn from its public comparison" (the reason is cut off in the excerpt I saw) https://www.infoq.com/news/2026/09/pnpm-12-rust [S], while Bun's homepage advertises its package manager as "npm-compatible, up to 30x faster" than pnpm https://bun.com/ [P]; benchmark claims between challengers are contested.

**(e) Time to conversion.** Started Jan 2016; mid-2021 at about 0.3M weekly; reached 8M weekly 7.5 years in. Slowest compounding of the set but a durable base [P].

---

### 1.9 Tauri vs Electron

**(a) Wedge and message.** Tauri 2.0 (2024-10-02): "Tauri is a framework for building tiny and fast binaries for all major desktop (macOS, linux, windows) and mobile (iOS, Android) platforms." https://tauri.app/blog/tauri-20 [P]. Tauri 1.0 (2022-06-19) stressed security/privacy/local-first: "more secure, more performant desktop app while using the same web technologies" (customer quote) https://tauri.app/blog/tauri-1-0/ [P].

**(b) Tactics.** Smaller bundles (uses the OS webview rather than bundling Chromium) and Rust back end; keeps the web front end so teams reuse React/Vue code; permission/scope system (2.0 capabilities) [P]. Measured by third parties: Tauri about 8.6 MiB vs Electron about 244 MiB; memory about 172 MB vs 409 MB with six windows https://www.besthub.dev/articles/tauri-vs-electron-which-framework-wins-in-performance-and-size-df6129e617e6 [S, vendor-adjacent, the underlying Hopp blog is https://www.gethopp.app/blog/tauri-vs-electron].

**(c) Effect.** 112.1k stars vs Electron 123.5k (GitHub API) [P]. npm weekly (2025 window): `@tauri-apps/cli` 0.20M vs `electron` 1.71M, about 12%, though the packages play different roles [P/I]. Production share of shipped apps is not established by what I found [U].

**(d) What backfired.** The cost of a speed/size win is correctness variance: "the windows/linux webview inconsistency of using webkit is a major flaw" (Tauri issue #3988) https://github.com/tauri-apps/tauri/issues/3988 [P]; HN: "As much as I love Tauri (and Wails), WebView requires to make sure the application renders exactly the same" and that is "what makes me end up using Electron most of the time" https://news.ycombinator.com/item?id=32550267 [P]; "Use QTWebEngine instead of webkitgtk" was closed as not planned https://github.com/tauri-apps/tauri/issues/4554 [P]; Linux NVIDIA blank-window documentation https://v2.tauri.app/develop/debug/linux-graphics [P]. Electron's pitch is the reverse: "Same version across all platforms" (comparison table at https://www.urhoba.net/en/post/system-webview-or-bundled-chromium [S]).

**(e) Time to conversion.** 1.0 in Jun 2022, 2.0 in Oct 2024. Over 4 years Tauri is a credible alternative for new small apps, not a displacer of Electron [I].

---

### 1.10 OpenTofu (Terraform) and Valkey (Redis): forks born from license changes

**(a) Wedge and message.**
- OpenTofu manifesto: "Our aim with this manifesto is to return Terraform to a fully open-source license." Fork after "no response from Hashicorp by August 25" https://opentofu.org/manifesto and https://opentofu.org/fork/ [P].
- Linux Foundation press release 2023-09-20: "an open source alternative to Terraform's widely used infrastructure as code provisioning tool" under "a neutral governance model" https://www.linuxfoundation.org/press/announcing-opentofu [P].
- Valkey: "a drop-in replacement for Redis OSS" (AWS) after Redis Inc. announced in March 2024 that future versions would no longer be open source https://aws.amazon.com/elasticache/what-is-valkey [P].

**(b) Tactics.**
- Compatibility promise: "we're aiming to maintain a reasonable amount of compatibility with Terraform ... We're not going to be pushing for big DSL changes, we're not going to be pushing for provider protocol changes, nothing of the sort." https://opentofu.org/blog/opentofu-is-going-ga [P]; migration guide, "safe and reversible" https://opentofu.org/docs/intro/migration [P].
- Fast staging: Aug 10 license change; Aug 25 fork; Sep 20 Linux Foundation; Oct 4 alpha; Nov 29 beta; Jan 10, 2024 GA (5 months) https://opentofu.org/blog/opentofu-release-candidate-is-out [P].
- Replaced the piece the incumbent could withdraw: a new registry ("Hosted on CloudFlare R2 ... publishing a new provider or module is now just a pull request away") https://www.infoq.com/news/2024/01/opentofu-1-6-now-ga [S].
- Corporate backers as distribution: Valkey fork began with "contributors from AWS, Ericsson, Oracle, and Google" https://www.linuxfoundation.org/blog/a-year-of-valkey [P]; AWS ElastiCache for Valkey launched 2024-10-08 "priced 33% lower [serverless] and 20% lower [node-based] than other supported engines", reserved-node discounts carried over https://aws.amazon.com/about-aws/whats-new/2024/10/amazon-elasticache-valkey [P].
- Foundation accepted into CNCF Sandbox 2025-04-23 https://www.cncf.io/projects/opentofu [P].

**(c) Effect.** OpenTofu: 31,000 downloads and 60 committers by Dec 2023 https://opentofu.org/blog/opentofu-release-candidate-is-out [P]; 30.6k stars today (GitHub API) [P]. Valkey: "nearly 50 contributing companies ... 19.8K stars" by Apr 2025 https://www.linuxfoundation.org/blog/a-year-of-valkey [P]; 27.5k today [P]. Incumbent response: Redis 8 re-added OSI-approved AGPLv3 in May 2025 https://redis.io/blog/agplv3 [P] ("Is It Too Late?", InfoQ) https://www.infoq.com/news/2025/05/redis-agpl-license [S].

**(d) What backfired / limits.** A fork needs a trigger event (license change) and a large coalition; it cannot be manufactured [I]. After Redis returned to AGPL the AWS page still positions Valkey as the permissive option because AGPL "for many organizations is unviable" https://aws.amazon.com/elasticache/redis [P].

**(e) Time to conversion.** OpenTofu GA in 5 months; Valkey hyperscaler managed service in 7 months; incumbent reversed in 14 months [P].

---

### 1.11 Firecrawl (2024-2025 rise)

**(a) Wedge and message.**
- HN title, 2024-05: "Firecrawl: Turn entire websites into LLM-ready Markdown or structured data" https://news.ycombinator.com/item?id=40404615 [P]. First HN post (2024-04-13): "Turn entire websites into LLM-ready data" https://news.ycombinator.com/item?id=40026782 [P].
- YC Launch post: "an open source API that transforms any web data into a clean, LLM-ready format for RAG, agentic tasks, or training. Since launching in April we gained 8000 stars" and "We launched a cloud offering over a weekend in April" https://www.ycombinator.com/launches/LTf-firecrawl-open-source-crawling-and-scraping-for-ai-ready-web-data [P].
- Origin pain: "Our story began while building Mendable.ai, one of the first managed RAG platforms" (they needed it themselves) [P, same URL]. Series A post: "Every AI app was rebuilding the same infrastructure ... scraping websites, extracting structured data, handling JavaScript, dealing with rate limits, and parsing messy HTML" https://www.firecrawl.dev/blog/firecrawl-v2-series-a-announcement [P].

**(b) Tactics.**
- One API call, one output format (markdown) as the entire product; positioning on the consumer (LLM), not on the crawler technology [P].
- Hosted cloud on day one plus open-source repo; "Start getting Web Data for free ... No credit card needed" and 1,000 free credits/month https://www.firecrawl.dev/crawl [P].
- Integration with the AI ecosystem rather than with Scrapy: official LangChain loader, LlamaIndex, MCP server ("Over 400,000 MCP servers have been installed"), n8n, Zapier, Cursor/Claude Code skills https://docs.firecrawl.dev/integrations/langchain and https://www.firecrawl.dev/ [P]. Zapier integrated "in a single afternoon" (Series A post) [P].
- Launch Week series (Aug 2024) and recurring Show HN side-launches such as the llms.txt Generator (Nov 2024) https://www.firecrawl.dev/blog/launch-week-i-day-4-introducing-firecrawl-v1 and https://news.ycombinator.com/item?id=42207756 [P].
- Versioned API with migration docs (v0 deprecated 2025-04-01, "Migrating from v0") https://docs.firecrawl.dev/v0/features/extract [P].

**(c) Effect.** 8k stars in 3 months; 34k+ by early 2025 (InfoWorld) [S]; 48k at Series A, "grown 15x in the past year", 350,000 developers signed up (Aug 2025) [P]; Series A $14.5M (Aug 2025) [P]; today 194.7k stars [P], "1.25M+ developers signed up", "5B+ requests", "2.5M+ weekly downloads" (company claims) https://www.firecrawl.dev/about [P-vendor]. PyPI `firecrawl-py` 5.3M/month (pypistats) [P]; npm `@mendable/firecrawl-js` 0.013M weekly (2024-10) to 0.212M (2025-10), possibly undercounted [P/U].

**(d) What backfired.**
- Open-core friction: AGPL core; the anti-bot "Fire-engine ... being closed source is the biggest deal breaker" for self-hosters, which prompted a fork "Firecrawl-Simple" on HN https://news.ycombinator.com/item?id=42056056 [P]. Docs now list Cloud-only surfaces ("Agent, Browser, managed dashboards, enhanced proxy paths") https://docs.firecrawl.dev/contributing/open-source-or-cloud.md [P].
- Politeness criticism at launch: an HN reader objected that the crawler's "specific agent string conveniently doesn't appear in the docs" and "presumption of default consent" (the comment is truncated in my view of the thread) https://news.ycombinator.com/item?id=40404615 [P/U on exact wording].
- Breaking API versions (v0 to v1 to v2): official LangChain JS loader "still calls v1 SDK methods" and fails per Firecrawl's docs https://docs.firecrawl.dev/integrations/langchain [P].

**(e) Time to conversion.** About 3 months to 8k stars; about 16 months to Series A with 48k stars; about 30 months to 195k stars [P]. Note this is category creation (LLM-ready web data) more than conversion of Scrapy users [I].

---

### 1.12 Browser Use (2024-2025 rise)

**(a) Wedge and message.**
- Show HN, 2024-11-05, 180 points: "I made Browser-Use, an open-source tool that lets (all Langchain supported) LLMs execute tasks directly in the browser just with function calling ... We created a layer that simplifies website interaction for LLMs by extracting xPaths and interactive elements like buttons and input fields." https://news.ycombinator.com/item?id=42052432 [P].
- Launch HN, 2025-02-25, 259 points: "We started coding a decade ago with Selenium bots and macros ... Last November, we asked ourselves, 'How hard could it be to build the interface between LLMs and the web?'" https://news.ycombinator.com/item?id=43173378 [P].
- Seed post: "In just four days, we built an initial prototype and launched it on Hacker News (if you have an idea, just build it and launch it)." https://browser-use.com/posts/seed-round [P].
- Platform-moment hook: YC Launch post "Browser Use: Open Source alternative to OpenAI Operator" - "Four days ago, OpenAI launched Operator." https://www.ycombinator.com/launches/Mh9-browser-use-open-source-alternative-to-openai-operator [P].

**(b) Tactics.**
- Demo-first README with many task videos; MIT license; "You can use any LLM, from Gemini to Sonnet, Qwen, or even DeepSeek-R1" [P, HN Launch].
- Rode an unplanned distribution event: Manus used Browser Use; daily downloads "more than quintupled from around 5,000 on March 3 to 28,000 on March 10" https://techcrunch.com/2025/03/12/browser-use-one-of-the-tools-powering-manus-is-also-going-viral/ [S].
- Hosted cloud at $30/month "significantly lower than OpenAI's Operator" as the monetization after the OSS pull [P, HN Launch].
- Funding as marketing: $17M seed (Felicis; Paul Graham, YC) March 2025 https://techcrunch.com/2025/03/23/browser-use-the-tool-making-it-easier-for-ai-agents-to-navigate-websites-raises-17m [S].
- Explicit anti-vision positioning: "Most existing automation solutions rely on vision-based approaches ... slow, expensive, and unreliable. Our approach is different: we convert website interfaces into structured text" https://browser-use.com/posts/seed-round [P].

**(c) Effect.** Repo created 2024-10-31; 25k stars at about 3 months (YC post, Jan 2025) [P]; 46k at seed time (Felicis) https://www.felicis.com/blog/investing-in-browser-use [S]; 118,982 today (GitHub API) [P]. PyPI `browser-use` 8.06M downloads/month vs Scrapy 4.30M/month (pypistats) [P]. Caveat: agent-framework installs are a new population, not evidence that Scrapy users converted [I].

**(d) What backfired.** HN threads were the stress test and are the best primary evidence of unmet needs:
- Reliability/verification: "it hallucinated form input values ... I want to know that what I expected to happen inside the browser, actually happened, and I want it to be verifiable. Otherwise I feel like I'm sending request into a black hole." (agotterer) https://news.ycombinator.com/item?id=43173378 [P].
- Replay: users "directly rerun the actions of browser-use deterministically", and the team says it is "working on robust task reruns" (founder) [P, same thread].
- Politeness: "AI agents have lead to a big surge in scraping/crawling activity ... many don't use proper user agents and don't stick to any scraping best practices ... (robots.txt, rate limits)" [P, same thread].
- Security: the documented real-browser setup launches Chrome with remote debugging on the user's profile, called "inherently insecure" [P, same thread].
- Cost/latency: "relatively expensive to run and are very slow" [P, same thread].

**(e) Time to conversion.** First commit to 25k stars about 3 months; to $17M seed about 5 months; to 119k stars about 2 years [P].

---

### 1.13 Crawlee vs Scrapy (and Stagehand as adjacent evidence)

**(a) Wedge and message.**
- Crawlee launch, 2022-08-11: "Crawlee is an open-source web scraping and automation library that helps you build reliable scrapers. Fast." and "Crawlee, the successor to Apify SDK ... full TypeScript support ... an interface similar to Apify SDK, making the transition for SDK developers a breeze." https://blog.apify.com/announcing-crawlee-the-web-scraping-and-browser-automation-library/ [P].
- "Batteries included": builds on Cheerio, Puppeteer, Playwright with `CheerioCrawler`, `PuppeteerCrawler`, `PlaywrightCrawler` that "automatically scale with available system resources, rotate proxies based on success rates, manage a queue of URLs, persistently store data" (same URL) [P].
- Reliability angle: "built-in modern anti-blocking features" with browser-like headers and TLS fingerprints "with zero configuration necessary" (same URL) [P].
- Explicit framing against Scrapy: "Crawlee has the advantage of being the latest library in the market, so it already has many features that Scrapy lacks, like autoscaling, headless browsing, working with JavaScript rendered websites"; Scrapy "does not work very well with JavaScript rendered websites, as it was designed for static HTML pages" https://crawlee.dev/blog/scrapy-vs-crawlee [P-vendor].
- Python launch, 2024-07: "Unified interface for HTTP and headless browser crawling, with automatic retries, proxy rotation, session management, and pluggable storage." and "libraries like Scrapy require additional installation of middleware, i.e, scrapy-playwright" https://blog.apify.com/announcing-crawlee-for-python/ and https://crawlee.dev/blog/launching-crawlee-python [P-vendor].

**(b) Tactics.**
- Existing user base via rename/split of Apify SDK (not a cold start): "Despite being open source, the library's name caused users to think its features were restricted to the Apify platform" (the rename motivation) [P].
- Upgrade guide for incumbents' users (v3 upgrading guide) and a customer case study where a retail data agency moved from Scrapy to Apify SDK ("saved 90% costs") https://blog.apify.com/daltix-python-vs-apify-sdk/ and https://apify.com/success-stories/daltix-analytics-scrapy-python-to-apify [P-vendor, single case].
- Templates and CLI parity with Scrapy ("Crawlee supports CLI just like Scrapy, but also provides pre-built templates") https://crawlee.dev/blog/scrapy-vs-crawlee [P-vendor].
- Platform tie-in: runs anywhere, deploys to the Apify platform https://pypi.org/project/crawlee [P].

**(c) Effect.** Crawlee JS "more than 12K stars" at the time of the Scrapy-vs-Crawlee post [P-vendor, undated]; 26.2k today [P]; npm weekly downloads 0.003M (2022-10), 0.017M (2023-10), 0.023M (2024-10), 0.044M (2025-10) [P]. Crawlee for Python: 9.6k stars 21 months in [P]. Scrapy: 65.0k stars. PyPI last month: scrapy 4.30M vs crawlee 0.82M [P]. Reading: Crawlee established a JS/TS and browser-first niche and a Python foothold, not a Scrapy displacement [I].

**(d) What backfired / limits.** Comparison pages are vendor-authored; I found no independent survey data on Scrapy-to-Crawlee migration [U]. The Python port arrived about 2 years after the JS launch, leaving the largest crawling audience (Python) to Scrapy meanwhile [I].

**(e) Time to conversion.** Not demonstrable from public data; 4 years in, Crawlee Python is at about 19% of Scrapy's PyPI volume [P/I].

#### Stagehand (Browserbase): the closest feature-level competitor to the "local replay" claim
- Started as a Playwright layer: "Returning a Stagehand-enhanced Page / Context meant your Playwright code kept working ... Result: minimal migration, lots of adoption, and a fast feedback loop with real users." https://www.browserbase.com/blog/stagehand-playwright-evolution-browser-automation [P].
- v3 (2025-10-29) left Playwright because "Playwright's actionability checks and auto-waiting are ideal for tests, but they add overhead in automation scenarios" and now "automatically caches discovered elements and actions that you can reuse without any additional LLM inference cost or time, and intelligently involves AI when workflows break." https://www.browserbase.com/blog/stagehand-v3 [P].
- Caching docs: "repeated calls with the same inputs return instantly without consuming any LLM tokens"; "Caching requires a Browserbase browser and the Browserbase API key" (v4 docs); a local `cacheDir` mode exists in v3 https://docs.stagehand.dev/v4/best-practices/caching and https://docs.stagehand.dev/v3/best-practices/caching [P]. Implication: replay without model calls is already marketed; the open slot is local, verifiable, vendor-independent replay (cache hit/miss is a best-effort cost optimization, "If the cache is unreachable, Stagehand falls back to normal inference") [P/I].
- Stagehand PyPI 1.27M/month; 25.8k stars [P].

#### spider-rs / Spider (Rust crawler; speed-first)
- Positioning: "The fastest web crawler and scraper for Rust"; hosted claims "99.9% success, <1s" vs Firecrawl "~95% ... ~3s", with the admission "that benchmark is ours" https://github.com/spider-rs/spider and https://spider.cloud/blog/best-firecrawl-alternatives [P-vendor]. 2.8k stars despite speed-first Rust positioning [P]; a data point that speed alone has not produced Scrapy/Firecrawl-scale adoption [I].

---

## 2. Cross-case lessons

Each lesson is backed by at least two cases. "Transfer" rates how well it applies to Goodfetch, whose claims are correctness (no false successes, local replay of AI steps, resumable runs, polite defaults, one API across HTTP and browser), not speed.

### L1. Ship a compatibility surface before the differentiator, and make the first install cost near zero.
Evidence: uv's `uv pip` ("zero configuration"), Ruff/Black and flake8 parity, Biome/Prettier 97%, Vitest/Jest API, Playwright/Puppeteer "most APIs can be used as is", OpenTofu/Valkey "drop-in", Stagehand-on-Playwright "minimal migration, lots of adoption". Counter-cases that went the other way: Deno 1.0 (incompatible, reversed after 4.5 years), Polars (not drop-in, slower, additive growth). 
Transfer: **High, but choose the surface carefully.** For Goodfetch the incumbent surfaces are Scrapy spiders/pipelines (Python), Crawlee crawler/router classes (TS/Python), Firecrawl's `/scrape`, `/crawl`, `/map` JSON shapes, and Stagehand's `act/extract/observe`. A thin Firecrawl-compatible HTTP endpoint or Stagehand-shaped `act/extract/observe` facade is the cheapest "paste your existing code" path [I].

### L2. Measure compatibility publicly with a number, a named corpus and a deviation list.
Evidence: Ruff ">99.9% of lines ... Django and Zulip" plus "Known Deviations" page; Biome Prettier challenge (85% to 96% to 97%) with "Differences with Prettier"; uv "top 10,000 PyPI packages" and a pip-compat page; Bun 1.2 runs Node's own test suite. Failure: Bun 1.0 asserted "drop-in" before measurement and drew "bombastic claims" / "'drop-in replacement' is bs".
Transfer: **High.** Publish a conformance suite: run the same fixture crawl through Scrapy/Crawlee/Firecrawl and diff outputs; report the percentage and the intentional deviations (e.g. refusing to report success when a postcondition fails).

### L3. Provide a migration tool or a cheat sheet, per incumbent.
Evidence: `flake8-to-ruff`, `biome migrate eslint|prettier`, `migrate-to-uv`, Playwright "Migrating from Puppeteer" table, Vitest Jest migration page, OpenTofu migration guide ("safe and reversible"), Stagehand v2-to-v3 guide, Firecrawl v0-to-v1 docs. I found no controlled evidence that migration tools raise conversion rates themselves, only that every winner shipped one [U on causality].
Transfer: **High, low cost.** A `goodfetch migrate scrapy|crawlee` reader that converts settings (concurrency, retries, robots) is mostly config mapping [I].

### L4. Be honest about limits at launch; overclaiming is the expensive failure.
Evidence: Ruff opened with "not a drop-in replacement ... not even production-ready" and still won flagship adopters in about 5 months; Deno 1.0 listed its incompatibility. Rome promised to replace Babel, ESLint, webpack, Prettier and Jest while shipping a beta linter and collapsed; Bun's 1.0 "drop-in" and homepage benchmarks were challenged ("not a 10x slam dunk").
Transfer: **High, and aligned with the correctness brand.** A tool whose claim is "no false successes" cannot afford its own launch to overstate; publish the known-failure list.

### L5. Win a narrow beachhead, then consolidate ("replaces N tools") as act two.
Evidence: Ruff linter first, formatter 14 months later, then uv; uv began as `pip`/`pip-tools` only and by Aug 2024 (6 months) described itself as Cargo for Python, now "replace pip, pip-tools, pipx, poetry, pyenv ..."; Biome narrowed Rome's scope to formatter + linter; Firecrawl is one job ("URL in, markdown out"); Bun's homepage retreated to "adopt one, or all of them."
Transfer: **High.** Pick one job to beat Firecrawl/Crawlee/Scrapy at (for example "a crawl that fails loudly and can resume") before the one-API-across-HTTP-and-browser story, which Crawlee already markets ("Unified interface for HTTP and headless browser crawling") [P/I].

### L6. Land inside flagship OSS projects and CI first; installed base follows.
Evidence: Ruff adopted by Pandas, FastAPI, Airflow within about 5 months; uv overtook pip in CI before laptops (66% of Wagtail's uv downloads are CI; 60% of FastAPI's); pnpm's homepage leads with Next.js, Vite, Vue, Nuxt, Prisma and Microsoft Rush; Vitest rode Vite/Vue/Nuxt; Bun landed in Tailwind's CLI and Claude Code.
Transfer: **High.** Target maintainers of well-known scraping repos and AI-agent repos (LangChain loaders, MCP servers, LlamaIndex readers) for first adoption; CI-friendly output (exit codes, JUnit-style reports for "this crawl lied") is a cheap wedge [I].

### L7. Integrate with the incumbent's ecosystem instead of replacing it, until you are big enough not to.
Evidence: Stagehand rode Playwright for adoption and left in v3 after outgrowing it; Crawlee wraps Playwright, Puppeteer, Cheerio; Vitest reuses `vite.config`; Biome reads ESLint/Prettier configs; Bun/Deno retrofitted `package.json`/`node_modules`; uv and pnpm use the PyPI/npm registries unchanged; Firecrawl and Browser Use plug into LangChain/LlamaIndex/MCP.
Transfer: **High.** Accept Scrapy items/pipelines or Crawlee request queues as import/export formats; ship an MCP server and LangChain/LlamaIndex loaders from day one; use Playwright/CDP as the browser layer rather than reinventing it [I].

### L8. Speed benchmarks open the door but draw the scrutiny; keep every comparative number reproducible and labelled.
Evidence: Ruff disclosed the 150x vs 25x (multiprocessing) caveat; Polars labels PDS-H as "not comparable" to TPC-H, open-sources it and also points to DuckDB Labs' db-benchmark; Bun's homepage numbers were publicly disputed; pnpm removed Bun and Yarn from its page while Bun publishes 30x vs pnpm; Spider's "that benchmark is ours".
Transfer: **High in method, low in content.** Goodfetch's "benchmark" should be a public failure-injection suite (timeouts, soft-404s, JS-gated content, bot walls) reporting false-success rate, not requests per second [I].

### L9. Make correctness visible through an artifact people screenshot and share.
Evidence: Playwright's Trace Viewer (DOM snapshots, network, console per step), codegen, and auto-wait stated as "No artificial timeouts, no flaky tests"; pnpm's strictness reveals phantom dependencies (users reacted, then used hoist patterns); Ruff's per-rule docs; Biome's contextual diagnostics ("detailed and contextualized diagnostics"); Browser Use users ask for "a feedback loop ... verifiable".
Transfer: **Very high for this product.** A replay/trace viewer and a verifiable run report (what was expected, what was observed, why it counts as success) is the Playwright analogue in crawling [I].

### L10. Strict defaults need escape hatches, or correctness becomes the reason people leave.
Evidence: pnpm added `public-hoist-pattern` and `shamefully-hoist` because many stacks "don't work well with pnpm without shamefully-hoist=true"; uv's index-resolution default caught pip users out (Plotly); Tauri's webview inconsistency drives HN users to Electron; Playwright's test-oriented auto-wait was a reason Stagehand left it.
Transfer: **High.** "Polite by default" (robots.txt, rate limits, identifiable user agent) and "fail rather than guess" will annoy some users on day one; ship explicit, logged overrides and a one-line explanation in every error [I].

### L11. Plan for 2-4 years of incremental conversion and measure new-project share, not installed base.
Evidence: Playwright passed Selenium after roughly 3 to 3.75 years and Puppeteer after roughly 3.75 to 4.75 on npm; Vitest created Dec 2021 is at about half Jest's weekly installs 46 months later; Biome at about 5% of ESLint's weekly downloads (3.2M vs 63.2M, 2025 window) two years after the fork; Polars at 15% survey share (pandas 80%) after four years; Crawlee Python at about 19% of Scrapy's PyPI monthly volume; Bun at about 7M monthly downloads 3.5 years in. Fast cases are the exception: uv (about 13 months to overtake Poetry in Wagtail data; 16M monthly downloads within roughly a year of launch) and Ruff (about 5 months to flagship adopters), both with a high-recognition team and a drop-in surface.
Transfer: **High.** Set targets on "new crawler projects started" and CI usage in named repos; do not expect Scrapy users to migrate whole code bases [I].

### L12. A trigger event plus a neutral governance home turns a clone into a movement (forks only).
Evidence: OpenTofu (Terraform's BSL on 2023-08-10; GA 5 months later; LF then CNCF), Valkey (Redis license change 2024-03; hyperscaler support within 7 months; Redis returned to AGPL 14 months later). Related: Firecrawl's closed Fire-engine spawned a self-host fork.
Transfer: **Low as a strategy** (cannot be manufactured), **medium as a contingent play:** the open slot is a fully open, self-hostable Firecrawl-class engine whose docs do not carve out cloud-only features [I].

---

## 3. Does it transfer when speed is NOT the main claim?

Which cases won mainly on something other than speed [P/I]:
- **Playwright**: reliability (auto-wait, isolation, trace viewer), cross-browser one API. The cleanest analogue, and it took roughly 3 to 3.75 years to pass Selenium on npm.
- **pnpm**: strictness plus disk efficiency; adoption driven by monorepo needs; strictness also generated the most friction.
- **Vitest**: ESM/TypeScript correctness and config reuse; compat API.
- **Tauri**: security and size; lost ground on rendering consistency.
- **OpenTofu/Valkey**: license/governance trust.
- **Firecrawl / Browser Use**: output shape for LLMs (markdown; accessibility-tree text) rather than crawler speed.
- **Deno** (security + standards) failed until it adopted Node compatibility.

Mapping to Goodfetch's five claims:

| Claim | Best precedent | Competitive gap seen in sources | Confidence |
|---|---|---|---|
| No false successes (verifiable outcome) | Playwright actionability checks and web-first assertions; Browser Use HN request for a "verifiable" feedback loop | Browser Use users report hallucinated form values; no incumbent in this set markets postcondition-verified success [I] | Medium-high: demand shown in HN, no survey |
| Local replay of AI steps without model calls | Stagehand v3 caching ("without any additional LLM inference cost"); Browser Use rerun of xPaths | Stagehand's cache is hosted-first (Browserbase) and best-effort; v3 also "intelligently involves AI when workflows break" | Medium: capability exists in market; local/portable/verifiable replay is the differentiator [I] |
| Resumable runs | Crawlee "persistent queue" and "persistently store data"; Scrapy job persistence is built in (unverified here) [U] | Table stakes in Crawlee; weaker as a wedge | Low as a headline, high as a hygiene feature |
| Polite defaults | HN complaints on agents ignoring robots.txt and rate limits (Browser Use Launch HN); Firecrawl's early user-agent criticism | spider.cloud advertises "honours robots.txt by default and includes built-in rate limiting" [P-vendor], so the claim is not unique | Medium: real concern, easy for competitors to copy |
| One API across HTTP and browser | Crawlee for Python "Unified interface for HTTP and headless browser crawling" | Already marketed by Crawlee; not novel | Low as a differentiator |

Net: the correctness claims fit the "Playwright path" (slow, durable, artifact-driven) rather than the "uv path" (fast, benchmark-driven). L1-L7 transfer unchanged; L8 changes content (publish failure-rate benchmarks); L9-L10 become the core of the plan. Expect conversion to be additive first: new projects and AI-agent pipelines, not Scrapy migrations [I].

## 4. Unverified items and gaps

- Launch-day HN threads for uv, Playwright and Crawlee were not retrieved; I do not claim their reception.
- The Ruff HN thread (item 32666035) was cited via a secondary article; the quote is from that article.
- State of JS 2024/2025 pages returned respondent counts and prose but I did not extract percentage usage for Vitest/Jest/Playwright; the 2025 prose quoted above is the only survey claim I rely on.
- Electron vs Tauri production share, Scrapy vs Crawlee migration counts, and any controlled evidence that migration tools increase conversion were not found.
- The 2026-10 npm points for playwright, vitest, pnpm are suspiciously high relative to 2025 and are flagged; use 2021-2025 ratios.
- Third-party "2026 comparison" blogs (tech-insider.org, codeoxi.com, devtoolsvs.com, besthub.dev, dev.to) are SEO-grade and used only where tagged [S].
