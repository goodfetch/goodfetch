# Pain ranking: raw counts and weighted score

**Status:** v1, desk research. **Date:** 2026-10-08. **Corpus:** 421 canonical items ([corpus.jsonl](corpus.jsonl)), coded according to the [codebook](codebook.md).
**What this ranking measures:** the frequency and recent visibility of **publicly expressed** pains. It measures neither their prevalence among customers, nor the intensity of the need, nor willingness to pay.

## 1. Method

- **n**: number of canonical items carrying the code, in any position.
- **W**: `Σ (1 + ln(1 + engagement)) × 0.5^(age_months / 6)`, computed as of 2026-10-08. The half-life is 6 months. A `YYYY-MM` date is taken as the 1st of the month.
- **Sensitivity**: *W without promo* excludes the 98 `promo` or `vendor_affiliated` items. *Capped W* limits engagement to 50, which neutralizes review aggregates (969, 724, 611 reviews…) and the two HN posts with more than 350 points.
- The 24 **pain or need codes** are ranked together. The 6 **context codes** (use cases, price, market) are ranked separately (§4), because they describe demand and not a pain.
- The scores are ordinal (codebook §4). With an inter-coder agreement of 83% on the primary code, a score gap of less than about 15% does not allow two codes to be separated.

## 2. Pain ranking

Sorted by W. The rank by n is given in the second column for side-by-side reading. In parentheses, the rank in each variant.

| W rank | n rank | Code | Label | n | W | n without promo | W without promo (rank) | Capped W (rank) |
| ---: | ---: | --- | --- | ---: | ---: | ---: | ---: | ---: |
| 1 | 1 | `access-antibot-ip` | Anti-bot, captcha, fingerprint and IP blocks | 76 | 92.4 | 59 | 72.4 (1) | 88.4 (1) |
| 2 | 2 | `agent-token-latency-cost` | Token bloat and slow per-step loops | 44 | 50.0 | 28 | 21.5 (5) | 48.9 (2) |
| 3 | 3 | `integ-mcp-agent-tooling` | MCP, CLI and agent-host integration | 37 | 37.7 | 30 | 25.2 (3) | 37.3 (3) |
| 4 | 5 | `access-auth-session` | Authenticated sessions and logins | 32 | 36.9 | 20 | 20.7 (7) | 36.6 (4) |
| 5 | 7 | `cost-unpredictable-runaway` | Unpredictable or runaway spend | 28 | 36.0 | 21 | 25.9 (2) | 30.3 (5) |
| 6 | 8 | `compliance-site-owner-politeness` | Site-owner backlash and polite crawling | 25 | 29.3 | 19 | 25.2 (4) | 26.2 (7) |
| 7 | 4 | `compliance-security-privacy` | Security, privacy and regulatory requirements | 35 | 28.0 | 22 | 19.3 (8) | 26.7 (6) |
| 8 | 16 | `dx-setup-docs` | Setup, configuration and docs friction | 18 | 24.6 | 16 | 20.8 (6) | 19.8 (11) |
| 9 | 6 | `agent-deterministic-replay` | Compile/cache agent runs to deterministic code | 30 | 24.0 | 22 | 15.0 (13) | 23.5 (8) |
| 10 | 18 | `silent-false-success` | Silent empty results or false success | 17 | 23.1 | 14 | 18.5 (9) | 23.1 (9) |
| 11 | 9 | `agent-unreliable-drift` | Agent nondeterminism and task drift | 22 | 20.8 | 16 | 15.6 (11) | 20.8 (10) |
| 12 | 10 | `integ-sdk-llm-runtime` | SDK, runtime and bring-your-own LLM/browser | 21 | 19.7 | 17 | 13.9 (14) | 19.7 (12) |
| 13 | 13 | `cost-infra-proxy-spend` | Proxy/captcha/infra spend growth | 19 | 18.4 | 12 | 12.6 (18) | 17.1 (14) |
| 14 | 14 | `data-extraction-quality` | Extraction accuracy and quality measurement | 19 | 17.8 | 14 | 13.1 (15) | 17.8 (13) |
| 15 | 11 | `ops-scale-ratelimit-scheduling` | Rate limiting, scheduling and orchestration | 20 | 17.7 | 17 | 16.0 (10) | 16.7 (15) |
| 16 | 21 | `rel-hang-crash` | Hangs, loops and crashes | 14 | 16.3 | 13 | 15.0 (12) | 16.3 (16) |
| 17 | 12 | `selfhost-parity-lockin` | Self-host parity and cloud lock-in | 20 | 16.1 | 18 | 12.5 (19) | 16.1 (17) |
| 18 | 19 | `agent-human-in-loop` | Human-in-the-loop and approval gates | 16 | 15.0 | 13 | 12.8 (17) | 15.0 (18) |
| 19 | 15 | `dx-observability` | Observability, logs and replay | 18 | 14.2 | 16 | 13.1 (16) | 14.2 (19) |
| 20 | 17 | `maint-site-drift` | Target-site change and selector drift | 17 | 13.1 | 13 | 10.8 (21) | 13.1 (20) |
| 21 | 22 | `maint-upgrade-regression` | Framework upgrade regressions | 12 | 12.5 | 12 | 12.5 (20) | 12.5 (21) |
| 22 | 20 | `perf-memory-concurrency` | Memory, process and concurrency limits | 15 | 11.5 | 11 | 7.3 (23) | 11.5 (22) |
| 23 | 23 | `rel-incorrect-incomplete` | Duplicate, missing or incomplete data | 11 | 10.5 | 10 | 9.7 (22) | 9.6 (23) |
| 24 | 24 | `durability-resume` | Crash-tolerant resume and checkpointing | 5 | 6.7 | 5 | 6.7 (24) | 6.7 (24) |

**The two rankings side by side (top 10):**

| Rank | By raw count (n) | By weighted score (W) |
| ---: | --- | --- |
| 1 | `access-antibot-ip` (76) | `access-antibot-ip` (92.4) |
| 2 | `agent-token-latency-cost` (44) | `agent-token-latency-cost` (50.0) |
| 3 | `integ-mcp-agent-tooling` (37) | `integ-mcp-agent-tooling` (37.7) |
| 4 | `compliance-security-privacy` (35) | `access-auth-session` (36.9) |
| 5 | `access-auth-session` (32) | `cost-unpredictable-runaway` (36.0) |
| 6 | `agent-deterministic-replay` (30) | `compliance-site-owner-politeness` (29.3) |
| 7 | `cost-unpredictable-runaway` (28) | `compliance-security-privacy` (28.0) |
| 8 | `compliance-site-owner-politeness` (25) | `dx-setup-docs` (24.6) |
| 9 | `agent-unreliable-drift` (22) | `agent-deterministic-replay` (24.0) |
| 10 | `integ-sdk-llm-runtime` (21) | `silent-false-success` (23.1) |

## 3. Robustness

- **Stable across all four variants.** `access-antibot-ip` stays first, with a score more than 1.8 times that of the second in every variant. `integ-mcp-agent-tooling` stays third. `cost-unpredictable-runaway` and `compliance-site-owner-politeness` stay in the top 7.
- **Inflated by commercial sources.** `agent-token-latency-cost` drops from 2nd to 5th place without the promo or affiliated items: the Stagehand v4 launch (`hn-86`), the Browserbase benchmark (`hn-89`) and Kuri (`hn-53`) weigh heavily. `agent-deterministic-replay` drops from 9th to 13th place (Intuned, `demand-use-cases-26`). The pain remains documented by independent sources (§5), but its relative weight is overestimated.
- **Inflated by review aggregates.** 68% of the weight of `dx-setup-docs` and 57% of that of `cost-unpredictable-runaway` come from review pages or posts whose engagement exceeds 100, that is, review counts. With the cap at 50, `dx-setup-docs` drops from 8th to 11th place. `cost-unpredictable-runaway`, by contrast, rises to 2nd place without promo: its signal does not depend on the aggregates.
- **Underestimated by raw counting.** `silent-false-success` (n rank 18, W rank 10) and `rel-hang-crash` (21 → 16) are recent and concentrated in 2026. They remain 9th and 12th without promo.
- **Reddit date bias** (codebook §5). Dividing the weight of each Reddit item by 1.47 keeps the same ten codes at the top. Only ranks 3 to 5 swap: `cost-unpredictable-runaway` moves to 3rd, ahead of `integ-mcp-agent-tooling` and `access-auth-session`.
- **Source bias.** 116 GitHub items come from OSS tools with a public tracker, such as Browser Use, Firecrawl or Crawl4AI. Closed services (Bright Data, Zyte, Oxylabs) appear only through reviews. `rel-hang-crash`, `maint-upgrade-regression` and `selfhost-parity-lockin` are therefore overrepresented for OSS products.

## 4. Context codes (demand, price, market)

| W rank | n rank | Code | n | W | n without promo | W without promo (rank) | Capped W (rank) |
| ---: | ---: | --- | ---: | ---: | ---: | ---: | ---: |
| 1 | 2 | `market-adoption-stats` | 35 | 34.3 | 12 | 15.1 (5) | 31.5 (3) |
| 2 | 1 | `wtp-price-points` | 46 | 33.3 | 38 | 29.5 (1) | 33.3 (1) |
| 3 | 4 | `usecase-monitoring-leadgen` | 26 | 32.7 | 22 | 28.2 (2) | 32.7 (2) |
| 4 | 3 | `build-vs-buy-switching` | 31 | 29.1 | 19 | 22.8 (3) | 27.8 (4) |
| 5 | 5 | `usecase-qa-research-ingest` | 24 | 17.5 | 22 | 16.8 (4) | 17.5 (5) |
| 6 | 6 | `usecase-portal-workflow` | 22 | 17.4 | 14 | 10.9 (6) | 16.9 (6) |

`market-adoption-stats` is 66% promo or affiliated (23 out of 35): these are mostly vendor reports. The `usecase-portal-workflow` use cases do not withstand the removal of promo sources (rank 6 out of 6): the B2B portal is carried by vendor posts (Anchor, TinyFish, Thinslices, Intuned) and by freelance job offers, not by user complaints. See [segments-and-needs.md](segments-and-needs.md).

## 5. Representative items by primary pain

For each code in the union of the two top 10s, 2 to 4 items are presented. These are the items with the highest W weight, favoring those that are not promo. `[P]` flags a promo or affiliated item.

### 5.1 `access-antibot-ip` (n 76, W 92.4)
- [`reddit-9`](https://reddit.sentinel-team.org/posts/1vuykgj/snapshots/2026-08-27T00%3A24%3A44.18942Z), 2026-08-27, r/LocalLLaMA. According to a commenter, it is not headless mode that triggers detection, but the automation flags and the blank profile of Chromium launched by Playwright. Attaching over CDP to the user's real Chrome works.
- [`reddit-27`](https://reddit.sentinel-team.org/posts/1v7m7q3/snapshots/2026-07-30T03%3A43%3A11.089892Z), 2026-07-30. Reusing the already logged-in browser removed more blocks than weeks of work on the fingerprint.
- [`reviews-reports-11`](https://www.outflo.io/sales-tools/brightdata), 2026-09. Even with Bright Data (G2 4.7 from 348 reviews, Capterra 4.7 from 68), target sites keep blocking or failing.
- [`reviews-reports-43`](https://www.zyte.com/sowa/2026/access/barrier-stacking/), 2026-06-22 `[P]`. WAF alone (31%) and WAF + JavaScript (about 20%) dominate; the full stack of barriers concerns only 3.3% of sites.

### 5.2 `agent-token-latency-cost` (n 44, W 50.0)
- [`github-106`](https://github.com/microsoft/playwright-mcp/issues/889), 2025-08-14. Tokens per task were multiplied by about 6 between Playwright MCP 0.0.30 and 0.0.32 (engagement 28).
- [`github-22`](https://github.com/browser-use/browser-use/issues/3768), 2025-12-15. Same 20 tasks, about twice as many tokens after the Browser Use update from 0.8.1 to 0.11.1.
- [`hn-98`](https://news.ycombinator.com/item?id=46942341), 2026-02-09. A single Playwright MCP server costs about 11.7k tokens of tool definitions on every request.
- [`hn-89`](https://news.ycombinator.com/item?id=49833761), 2026-09-24 `[P]`. Browserbase benchmark: the same model scores 74% at $1.50 per task with one harness, versus 71% at about $10 with another.

### 5.3 `integ-mcp-agent-tooling` (n 37, W 37.7)
- [`hn-50`](https://news.ycombinator.com/item?id=47392002), 2026-03-15 (14 replies). For this commenter, MCP constantly wastes context and advanced users drive Chromium via Playwright CLI. The thread debates MCP versus CLI at length.
- [`hn-51`](https://news.ycombinator.com/item?id=47392518), 2026-03-15. Cases where MCP keeps the advantage: zero-install integrations, non-developer users, multi-tenant OAuth, credential isolation, audit logs.
- [`github-24`](https://github.com/browser-use/browser-use/issues/4742), 2026-04-25. From Claude Code, a screenshot in a tool result corrupts the conversation, and every subsequent turn fails with a 400.
- [`demand-use-cases-34`](https://news.ycombinator.com/item?id=46616481), 2026-01-14 `[P]` (134 points). CLI built because the browser MCP servers tested could not reproduce an intranet SSO task.

### 5.4 `access-auth-session` (n 32, W 36.9)
- [`github-19`](https://github.com/browser-use/browser-use/issues/1520), 2025-04-29 (46). Chrome 136+ blocks CDP control on the default profile, which breaks the "my real logged-in Chrome" flow.
- [`github-43`](https://github.com/browserbase/stagehand/issues/1250), 2025-11-09. Stagehand v3: `userDataDir` has no effect and `storageState()` has disappeared, so it is impossible to persist a login.
- [`hn-34`](https://news.ycombinator.com/item?id=45452473), 2025-10-02. Need for a human login followed by a handoff to the agent, with sessions lasting several months and re-login alerts. No platform allows this simply at an acceptable cost.
- [`demand-use-cases-38`](https://github.com/microsoft/playwright-mcp/issues/922), 2025-08-20. Credential resolution on the MCP side so that the LLM never sees the real passwords.

### 5.5 `cost-unpredictable-runaway` (n 28, W 36.0)
- [`reviews-reports-23`](https://www.capterra.co.nz/software/180165/zyte), 2024-11-10. On pay-as-you-go, no spending cap without a subscription: a bill about 40 times higher than expected.
- [`reddit-57`](https://reddit.sentinel-team.org/posts/1u9t2m9/snapshots/2026-06-19T20%3A07%3A29.810032Z), 2026-06-19. An agent retried a timed-out Firecrawl call about 200 times overnight, for $40. The user asks for per-task budgets.
- [`reviews-reports-10`](https://thunderbit.com/blog/brightdata-review-costs-alternatives), 2026-04-20 `[P]`. Web Unlocker bills failures as soon as custom headers or cookies are enabled.
- [`hn-76`](https://news.ycombinator.com/item?id=44834439), 2025-08-08. Objection to paying for an AI result that can fail; the vendor is considering not billing failures caused by the platform.

### 5.6 `compliance-site-owner-politeness` (n 25, W 29.3)
- [`hn-125`](https://news.ycombinator.com/item?id=49211386), 2026-08-07 (462 points). A 1.5M-page site reports about 99% bot traffic.
- [`github-1`](https://github.com/scrapy/scrapy/issues/6597), 2025-01-01 (49). 38,000 requests in 17 minutes from a default Scrapy configuration; request for polite default values.
- [`demand-use-cases-31`](https://news.ycombinator.com/item?id=45459596), 2025-10-03. An operator blocks all of Browserbase after abuse by a single customer: a cloud browser's shared IPs are blocked wholesale.
- [`hn-62`](https://news.ycombinator.com/item?id=45374976), 2025-09-25 `[P]`. A startup is considering leaving Firecrawl to control crawl etiquette (robots.txt handling considered inconsistent).

### 5.7 `compliance-security-privacy` (n 35, W 28.0)
- [`github-33`](https://github.com/browser-use/browser-use/issues/4505), 2026-03-24. The CLI installation script pulled a compromised version of litellm (credential-stealing backdoor).
- [`demand-use-cases-10`](https://www.upwork.com/freelance-jobs/apply/Python-Playwright-Developer-for-Secure-Browser-Automation-500-PDF-Downloads_~022104387142669699637/), 2026-09-28. Download of about 3,500 customer PDFs: local execution required for confidentiality, resume after interruption, NDA.
- [`hn-35`](https://news.ycombinator.com/item?id=45458977), 2025-10-03. Customers refuse credential storage; automating 2FA defeats its purpose.
- [`demand-use-cases-39`](https://github.com/browser-use/browser-use/issues/4574), 2026-03-31. Human oversight, record retention and audit logs for the EU AI Act.

### 5.8 `dx-setup-docs` (n 18, W 24.6, weight carried by review aggregates)
- [`github-13`](https://github.com/browser-use/browser-use/issues/839), 2025-02-23 (55) and [`github-14`](https://github.com/browser-use/browser-use/issues/1020), 2025-03-13 (51). First launch: Chromium stays on `about:blank` and the agent loops at step 1.
- [`reviews-reports-3`](https://ai.g2.com/product/apify), 2026-05. G2 summary (611 reviews): building a custom Actor requires a long learning curve.
- [`hn-17`](https://news.ycombinator.com/item?id=46586819), 2026-01-12. The documentation does not say what Lightpanda can safely be used for as a Playwright engine.

### 5.9 `agent-deterministic-replay` (n 30, W 24.0)
- [`github-39`](https://github.com/browserbase/stagehand/issues/389), 2025-01-09 (30). `extract()` should return its static plan to avoid one LLM call per page.
- [`hn-87`](https://news.ycombinator.com/item?id=49763632), 2026-09-19. Stagehand's action cache is tied to Browserbase sessions and is useless with a local browser. The user wanted it as a versioned file for CI.
- [`reddit-24`](https://reddit.sentinel-team.org/posts/1ve9gkj/snapshots/2026-08-07T06%3A10%3A44.151998Z), 2026-08-07. The thread's consensus: resolve once, cache a workflow contract with a DOM fingerprint, replay, and call the LLM only when something breaks.
- [`demand-use-cases-19`](https://www.upwork.com/freelance-jobs/apply/Powered-Test-Agent-Playwright-LLM_~022097330742604282149/), 2026-09-08. Freelance job offer: a QA agent whose successful runs emit deterministic Playwright tests for CI.

### 5.10 `silent-false-success` (n 17, W 23.1)
- [`reddit-1`](https://reddit.sentinel-team.org/posts/1vnydav/snapshots/2026-08-15T02%3A07%3A43.063054Z), 2026-08-15. Clicks return "ok" on elements that ignore them. The proposed fix is to verify each action by rereading the page.
- [`github-80`](https://github.com/unclecode/crawl4ai/issues/2116), 2026-07-31. An anti-bot block comes back as HTTP 500 instead of a typed "blocked" result.
- [`github-93`](https://github.com/apify/crawlee/issues/3629), 2026-05-06. Handlers run on the Cloudflare challenge page.
- [`hn-101`](https://news.ycombinator.com/item?id=47296933), 2026-03-08 `[P]`. After a site redesign, CSS selectors silently return empty strings.

### 5.11 `agent-unreliable-drift` (n 22, W 20.8)
- [`reddit-0`](https://reddit.sentinel-team.org/posts/1vnydav/snapshots/2026-08-15T02%3A07%3A43.063054Z), 2026-08-15. 40 minutes for two Ticketmaster tickets: loops on captchas and modals, cart expired twice.
- [`reddit-25`](https://reddit.sentinel-team.org/posts/1uhvp85/snapshots/2026-06-29T19%3A40%3A40.485144Z), 2026-06-29. Best return on investment on fixed internal tools; brittle on public sites, unreliable on arbitrary tasks.
- [`github-30`](https://github.com/browser-use/browser-use/issues/1763), 2025-05-24. The agent loses track of a multi-step goal and declares the task finished too early.

### 5.12 `integ-sdk-llm-runtime` (n 21, W 19.7)
- [`github-31`](https://github.com/browser-use/browser-use/issues/2605), 2025-08-05 (31). Local models (gpt-oss via Ollama) fail on structured output.
- [`github-48`](https://github.com/browserbase/stagehand/issues/393), 2025-01-09 (33). Long thread of requests for a Python SDK for Stagehand.
- [`github-38`](https://github.com/browserbase/stagehand/issues/507), 2025-02-20 (31). Being able to attach to an existing Playwright test page.

## 6. Rarely expressed pains

`durability-resume` (n 5) and `perf-memory-concurrency` (n 15, 23rd without promo) are at the bottom of the ranking. Resume appears mostly as a **requirements-specification item** ("resumable" in `demand-use-cases-1` and `demand-use-cases-10`) and in a Scrapy proposal (`github-5`), rarely as a complaint. Memory is a pain only at high volume (`github-104`, `reddit-60`). An HN commenter considers that LLM cost dwarfs browser cost in agent workflows (`hn-14`). This does not mean these properties are useless. It means they **do not sell on their own**: buyers take them for granted or only discover them in production.
