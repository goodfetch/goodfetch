# Marketing angles: does Goodfetch fill a gap, and how to win users of Scrapy, Crawlee, Spider and their neighbours

**Status:** analysis for the owner, not a decision. **Date:** 2026-10-23.
**Built on:** the market track ([README.md](README.md), 421 sourced items, frozen 2026-10-08), the [consumer-API research](https://github.com/goodfetch/goodfetch/blob/research/consumer-api/rnd/research/2026-10-14-consumer-api.md), the [R&D report](../../README.md), the adopted ADRs, and three new desk studies made today:

| Note | Content |
| --- | --- |
| [competitors-oct-2026.md](marketing-2026-10-23/competitors-oct-2026.md) | Where Spider, Scrapy, Crawlee, Crawl4AI, Firecrawl, Stagehand, Browser Use, Playwright and agent-browser stand: stars, releases, licenses, funding, taglines, 2026 complaints |
| [challenger-playbooks.md](marketing-2026-10-23/challenger-playbooks.md) | How 13 developer tools (uv, Ruff, Bun, Biome, Playwright, Vitest, Polars, pnpm, Tauri, OpenTofu/Valkey, Firecrawl, Browser Use, Crawlee) took users from incumbents, and what backfired |
| [market-timing-channels.md](marketing-2026-10-23/market-timing-channels.md) | 2025–2026 shifts (Cloudflare, IETF drafts, lawsuits, EU AI Act, funding) and where scraping developers gather |

Star counts in this file were re-checked against the GitHub API on 2026-10-23. Lab figures come from `rnd/lab` runs that have not yet been repeated on the reference host, so they are evidence for design, **not yet publishable claims** (§7).

---

## 1. Verdict: yes, there is a gap, but it is not where it first looks

**What is not a gap.** Each headline feature already exists somewhere:

- **One API across HTTP and browser.** Crawlee markets it ("Unified interface for HTTP and headless browser crawling").
- **Replaying AI steps without the model.** Stagehand caches actions, though only on Browserbase browsers. Libretto (Show HN, 2026-04-15, 134 points) records and replays scripts.
- **Polite defaults.** spider.cloud advertises robots.txt and rate limiting by default.
- **Rust speed.** Spider and agent-browser, Vercel's Rust CLI with 44.5k stars, already claim it.
- **"Reliable."** It is Crawlee's own tagline: "Build reliable web scrapers. Fast."

Marketing any one of these alone puts Goodfetch second in line.

**What is a gap.** No product in the set **combines** these four, and the market is asking for each of them separately:

1. **Results you can trust.** No tool claims success without proof: a Page check, a Success check and typed outcomes (ADRs 0006, 0013, 0079).
2. **AI you pay for once.** A Plan is a local file you commit like a lockfile, checked against its targets before the first effect, and repaired on drift (ADRs 0009, 0091).
3. **A crawler and a browser driver in one engine.** The crawler hands every handler the same page on every fetch tier (ADRs 0084, 0093).
4. **Fully open and fully local.** The license is permissive, self-hosting has the same capabilities, there is no telemetry, and there are no credits (ADRs 0018, 0043).

The gap runs **across** existing products, as the table shows:

| Need (market evidence) | Scrapy | Crawlee | Spider | Firecrawl / Crawl4AI | Stagehand / Browser Use | Goodfetch |
| --- | --- | --- | --- | --- | --- | --- |
| No silent false success (C6; consumer-API pain 3) | partial | weak (queue marks unrun requests handled) | weak | weak (failed jobs look like successes) | weak (premature "done") | **core contract** |
| AI step replayed with no model, local and versioned (C4) | none | via Stagehand **v3 only** | none | none | cloud-tied or best-effort | **core contract** |
| Crawler + driver + AI on one page | plugin + reactor | yes, AI stuck on v3 | recompile to switch | API only | browser only | **yes** |
| Same product self-hosted (C11) | yes | yes, Apify for ops | crate yes; cloud extras | **no**: Fire-Engine, agent endpoints closed | cache and persistence paid | **yes** (ADR 0018) |
| Nothing leaves the machine | yes | yes | Spider Shield routes traffic through the device | n/a | n/a | **guaranteed by CI** (ADR 0043) |
| Polite by default (C8) | template only; robots fail-open bug in 2.19 (#8297) | 8 per domain, no backoff on 429 | yes (cloud) | inconsistent | n/a | **one request at a time per domain, robots obeyed** (ADR 0086) |
| Resume after a crash (not just a clean stop) | `JOBDIR` clean stop only | purge/resume confusion | no | no | no | **checkpoints** (ADR 0002) |

**Is it a "renewal"?** It can be told as one, and the timing supports it (§4). Each incumbent was designed for an earlier web:

- **Scrapy (2008)** was built for an HTML web.
- **Crawlee (2022)** was built for the headless-browser web.
- **Today's web** sorts visitors by declared purpose: Cloudflare has treated "Agent" as its own class since 2026-07-01. AI writes much of the scraping code, and buyers must prove where data came from (the EU GPAI Code of Practice, enforceable since 2026-08-02).

Goodfetch is the first framework *designed* for that web rather than retrofitted. This is a story about **trust**, not speed, and it is the right story to tell.

**The honest risks:**

- **Ship before you shout.** Nothing is released yet (M1 is in progress). Every precedent that overclaimed before shipping paid for it: Bun's "drop-in" claim and Rome's collapse.
- **Conversion is slow.** The closest precedent is Playwright, which won on reliability and took about 3–4 years to pass Selenium and Puppeteer on npm. The uv-style speed blitz is not available to a correctness claim.
- **The biggest pool is new users, not converts.** On PyPI, browser-use (8.1M a month) and firecrawl-py (5.3M) are above Scrapy (4.3M). Agent builders are a **new population**. Expect adoption from new projects first and migrations second.
- **Low willingness to pay where the noise is.** Agent developers are 44% of public voice but the segment least likely to pay (C10). That is fine for adoption and wrong for revenue.
- **Competition from doing nothing.** "A coding agent plus an OSS library" is the default alternative (`hn-71`, `demand-use-cases-27`).
- **Spider's community is small.** 2.8k stars and a 647-member Discord make it a small pool to convert. Treat it as a credibility audience (Rust developers), not a volume one.

---

## 2. Positioning

### Category

Do not use "fastest crawler" or "AI scraper". Those slots are crowded and owned by others. Claim a category nobody holds: **verifiable web automation**. Every result says how it was obtained and whether it was checked.

### Lines (drafts to test, not decided)

| Surface | Line | Why |
| --- | --- | --- |
| Goodfetch (brand) | **Web automation you can verify.** | Owns "verify". Crawlee holds "reliable", Playwright holds "reliable … for AI agents" |
| Foxhound | **The crawler that knows what it got.** / *Every page checked, every run resumable, polite by default.* | Turns C6 (false success) and C8 (site owners) into the promise |
| Collie | **Playwright's API. AI that runs once.** / *Act, observe and extract compile into a Plan you commit, then replay with no model call.* | Models already know Playwright's names (ADR 0085); C4's constant demand |
| Licence and trust | **Open all the way down. No credits, no cloud-only engine, nothing leaves your machine.** | The market's single monetization pattern is "OSS feeding a cloud". Zyte wins Scrapy users with "no rug pulls" |

### Rules the angles must respect (adopted)

- **ADR 0097: never lead with stealth or access**, although access is pain #1 (76 items). Talk about *sessions that hold* (C1: continuity matters more than the fingerprint) and *the right fetch tier*, never about getting past defences. The legal research agrees: plaintiffs in Reddit v. SerpApi plead stealth and CAPTCHA solving as DMCA §1201 circumvention, and that claim survived dismissal on 2026-07-31.
- ADR 0035's wording policy applies to every published text, so detection-defeat words go nowhere, including registry metadata.
- No speed or cost figure until it is measured on the reference host and reproducible (§7).

---

## 3. Angles by audience

Each angle keeps what that audience loves, names the pain they already complain about with evidence, and gives a proof plus a first step that costs nothing.

### 3.1 Scrapy users (65.0k stars; Python data teams)

- **What they love, which you must match:**
  - testable callbacks and offline replay (consumer-API like #7, "solved every problem");
  - middleware and fine-grained control (like #12);
  - a BSD licence and Zyte's "no paywalled features, no rug pulls".
- **What hurts, with evidence:**
  - `ReactorNotRestartable` when embedding: 54 Stack Overflow questions, 228 votes, Scrapy #2473;
  - browser rendering through scrapy-playwright's reactor, settings and per-request meta;
  - `JOBDIR` survives only a clean shutdown (`github-5`);
  - duplicates after retry (`github-7`);
  - library defaults that site owners call "a malicious denial-of-service tool" (#6597);
  - in 2.19, robots.txt enforcement failing open for concurrent requests (#8297, 2026-10-09).
- **Hook:** *"Everything you like about Scrapy (testable callbacks, hooks, an open licence), without the reactor, the Playwright plugin, or a crash that loses the job."*
- **Proof to show:**
  - a spider ported side by side;
  - a `runHandler` test on a saved page, then a WARC replay test reaching no site (ADR 0105);
  - `kill -9` mid-crawl, then resume from the checkpoint;
  - one handler moving from HTTP to the browser without a rewrite (ADR 0093).
- **First step without migrating:** keep the Scrapy spider and borrow its browser page into Collie for the one site that needs AI or a login (`run.borrow()`, ADR 0089).
- **Migration asset:** "Goodfetch for Scrapy users". Map Spider→Crawl, `parse`→Label handler, Item Pipeline→Datasets and Sinks, Downloader middleware→Hooks, `JOBDIR`→Checkpoint, scrapy-playwright→fetch tiers. ADR 0097 already requires the guides; make their code run in CI, which nobody else does.

### 3.2 Crawlee users (26.2k JS, 9.6k Python; TypeScript-first, often on Apify)

- **What they love:** one API for HTTP and browsers, the request queue and storage, TypeScript. Note that Crawlee *already* markets the unified API, so it is not your hook here.
- **What hurts:**
  - autoscaling pushed to OOM, no backoff on 429, domains hammered in v4 (`github-84`, `-85`, `-96`);
  - requests marked handled without the handler running (`github-91`);
  - the context changes shape with the crawler class;
  - purge and resume surprises (#1602, #3315, "people are still asking about … purging, resuming");
  - **the Stagehand integration is pinned to v3** while Stagehand v4 (2026-08-10) refuses Playwright pages;
  - Crawlee v4 at rc.1 brings breaking changes anyway (TLS verification, #4240).
- **Hook:** *"Already rewriting for Crawlee v4? Rewrite once, onto a crawler where AI on the page is first-class and a request is never 'handled' unless your handler finished."*
- **Timing:** the v4 migration window (rc.1 on 2026-10-06) is the best opening of the year. People rewriting anyway compare alternatives.
- **Proof:** a 429 storm and the crawl backs off, versus spiralling. A crashed run resumes. The same crawl with `act`/`extract` on the handler's page, replayed with no model call.
- **Migration asset:** the "Crawlee + Stagehand → Foxhound" guide that ADR 0037 already requires (rows V3-01 to V3-05). It is the most targeted piece of content you can publish, because those users are stranded.

### 3.3 Spider users (2.8k stars; Rust developers)

- **What they love:** Rust speed and concurrency.
- **What hurts:**
  - 132 Cargo features that change public types and break builds (#349);
  - a default `io_uring` that panics in Docker (#243);
  - a recompile to switch to the browser;
  - links extracted before JS rendering finishes (`github-118`).
  - The new Spider Shield terms (October 2026) route third-party traffic through the user's device in exchange for a free firewall. Expect trust-sensitive users to notice.
- **Hook:** *"A Rust engine where capabilities are settings, not Cargo features: one binary, same public types in every build, and the same engine from TypeScript and Python."*
- **Do not** claim to be faster than Spider: nothing measured supports it, and Spider publishes its own benchmarks.
- **Why bother:** this is a small pool but an influential one. Rust credibility comes from crates.io, r/rust and This Week in Rust, so serve it with a good Rust crate and docs.rs pages, not a campaign.

### 3.4 Firecrawl and Crawl4AI users (LLM-ready data; the largest new population)

- **What hurts:**
  - Firecrawl's credits (5–9 per page with JSON plus Enhanced Mode; billed failures);
  - the AGPL engine with a closed Fire-Engine, so self-hosting lacks anti-bot and agent endpoints (15 of its top 25 issues are self-hosting);
  - failed jobs returned "exactly like successful ones" (#4212);
  - Crawl4AI's Docker gaps, empty LLM extraction returned as success (`github-78`), and a block returned as HTTP 500 (`github-80`).
- **Hook:** *"The same one-call simplicity, running on your machine. MIT/Apache, no credits, and a page that failed says it failed."*
- **Proof:** URL → Markdown in one command with a typed outcome, and a blocked page reported as blocked (ADR 0079), not as content.
- **Licence point:** the AGPL is why Firecrawl had to move its SDKs to MIT (ADR 0018). Closed products and agents can embed Goodfetch.

### 3.5 Stagehand, Browser Use and Playwright-for-agents users (the agent lane)

- **What hurts:**
  - Stagehand's cache "has no effect" locally (`hn-87`), and persistence is paid (`github-42`);
  - v3 to v4 is "a rewrite";
  - Browser Use's tokens doubled between versions (`github-22`), it declares completion too early (`github-30`), it pulled in the compromised litellm (#4505), and its `==` dependency pins block adoption (#4824);
  - Playwright MCP's token weight.
- **Hook:** *"Pay the model once. Collie compiles act/observe/extract into a Plan file you commit next to your code, checks every target before the first click, and replays with zero model calls until the site changes."*
- **Differentiate from Libretto** (exported scripts) and Stagehand (cloud cache):
  - the Plan is verified by Target fingerprint before any effect;
  - a drift triggers one bounded repair;
  - a run's success is judged by your `expect`, never by the model (ADRs 0009, 0013).
- **Proof (lab, to re-measure):**
  - compiled replay at 0.17–0.34× the model spend of an agent that replans every run, with no false success across three real models;
  - raw code mode gave 17 false successes out of 45, and with the effect journal 0.

---

## 4. Why now: the timing story

| Shift (dated) | What it lets you say |
| --- | --- |
| **Cloudflare, 2026-07-01:** traffic split into Search, Agent and Training, with "browser-use agents" named as Agent. From 2026-09-15 Training and Agent bots are blocked on ad pages | "The web now asks who you are and why. Goodfetch answers." The `identified` Access mode sends a product UA and a Web Bot Auth signature (ADR 0021). ADR 0086 obeys robots.txt and AI-use signals |
| **EU GPAI Code of Practice**, enforceable 2026-08-02: respect machine-readable opt-outs, no spoofed identity | AI companies' data teams need **provenance and crawl logs** from their tools. Run reports, per-record provenance and the Usage ledger answer procurement (C9) |
| **DMCA §1201 is the live legal risk** (Reddit v. SerpApi/Perplexity survived dismissal 2026-07-31) | Leading with trust is also the legally safer brand (ADR 0097) |
| **Every incumbent is mid-rewrite:** Stagehand v4 (2026-08-10), Crawlee v4 rc.1 (2026-10-06), Scrapy reactor-less mode still experimental | Users who must rewrite anyway are the cheapest to convert |
| **Money flows to closed layers:** Firecrawl $75M (2026-09-22) with a paid "Alexandria" library, Crawl4AI Cloud beta (2026-10-08), Spider Shield | The "open all the way down" position gets stronger each time a peer adds a cloud-only piece |
| **Every project now pitches agents** (Scrapy 2.19 ships an MCP server, Playwright 1.62 bundles MCP and CLI) | "Supports agents" no longer differentiates; "agents can't fake success" does |

---

## 5. Proof assets: what people will screenshot

The challenger research is clear: correctness wins only when it is **visible** (Playwright's trace viewer, Ruff's compatibility percentage). Build these, roughly in order:

1. **The false-success benchmark** (ADR 0025's Proving Ground, made public).
   - A reproducible suite of hostile-but-legal scenarios: a portal answering `200` with its login form, a block returned as 500, offset pagination on a live table, a 429 storm, a site that changes under a cached plan.
   - It reports **false successes, lost records and duplicates**, not requests per second. In the lab, the expired-session portal produced 20 false successes out of 20 for a client trusting the HTTP status.
   - Publish the harness and invite competitors' maintainers to fix their adapters. A benchmark you might lose on some rows is the credible kind.
2. **The Plan file diff.** A screenshot of a committed Plan changing in a pull request after a repair. "AI changes reviewed like a lockfile" is instantly understood.
3. **The Run report.** Three axes (ADR 0029): how it ended, the verdict, and what the model claimed, side by side. That one screen explains "no false success".
4. **A measured compatibility number.** For example, "Collie runs N% of Playwright's page-API examples unchanged" (ADR 0085), with the deviation list. Do not say "drop-in" until it is measured.
5. **Before/after migration repos** for Scrapy, Crawlee+Stagehand and Firecrawl self-hosting, run in CI.

---

## 6. Channels and launch sequence (free only, per AGENTS.md "Money")

| Phase | Do | Notes |
| --- | --- | --- |
| **Now, before release** | Run the switch interviews already designed ([interview-guide.md](interview-guide.md), [recruitment.md](recruitment.md)): five or more per segment. Every interviewee becomes a design partner and a first user | Doubles as the price research C10 left open |
| | Publish the *problem*, not the product: an essay on "silent false successes in web automation" with the public issues as evidence, and the benchmark harness | Builds an audience without promising features |
| **Alpha** | Hand early builds to the interviewees. Land Goodfetch in 2–3 flagship open-source data or agent projects (Ruff and uv lesson: flagships and CI first) | |
| **Launch** | **Show HN** with a runnable repo and a plain explanation: the proven venue for this category (Stagehand 326 points, Skyvern 422, Browser Use 180 then 259; Libretto 134 in 2026) | No upvote solicitation (HN rules) |
| | **r/webscraping** (~102k members): open-source tooling may be posted anywhere *if it stays free*; any paid offering only in the monthly or weekly threads | Fits ADR 0018. Never post Cloud there outside those threads |
| | **The Web Scraping Club** newsletter (7k+ subscribers): pitch the benchmark as a story, not an ad | |
| | Rust: r/rust, This Week in Rust, docs.rs. Python: Python Weekly, PyCoder's | |
| **After** | Migration guides as search content ("scrapy-playwright alternative", "Crawlee Stagehand v4", "self-host Firecrawl alternative") | Long tail; these are the queries stranded users type |
| | Answer questions in competitors' issue trackers only where `run.borrow()` or a Lent page genuinely solves the asker's problem | Never pitch in other projects' Discords |

Conferences such as Extract Summit (Zyte's, Dublin 2026-11-10/11) and OxyCon cost money and are vendor-owned. Skip them until there is a budget.

---

## 7. What not to say until it is proven

- **No speed or cost multiplier** (0.51× Node CPU, 0.17–0.34× model spend, 120–200× for the discovered-API tier) until it is re-measured on the reference host and the harness is public. Every speed claim in the category is contested (Ruff disclosed 25× vs 150×; "that benchmark is ours" for Spider).
- **Not "drop-in" or "replaces Scrapy"** until a compatibility figure exists.
- **Not "reliable"**, which is Crawlee's word. Use "verified" or "checked".
- **Nothing about stealth, bypassing or unblocking** (ADRs 0035, 0097).
- **No "zero false successes, guaranteed".** Say "a result is reported as verified only when your check passed". That claim is exactly true by contract, while "zero" is a measurement.

---

## 8. Decisions for the owner

1. **Lead audience.** *Recommendation:* lead with **Foxhound for Python and TypeScript data teams and the Crawlee v4 / Stagehand v3 stranded users**, with Collie's Plans as the hook.
   - That is where the pain is specific, documented and timed.
   - Agent developers come anyway through MCP and the CLI.
   - Monetization (C10) points to verified-result workloads, not agent hobbyists.
2. **Category name.** *Recommendation:* adopt "verifiable web automation" and test the three lines in §2 in the interviews before choosing.
3. **The public benchmark.** *Recommendation:* make the false-success benchmark a deliverable of the release, since ADR 0025 already defines the Proving Ground. It is the one asset no competitor can copy quickly.
4. **When to start.** *Recommendation:* start the interviews and the problem essay now; announce nothing about the product until a build runs end to end.
