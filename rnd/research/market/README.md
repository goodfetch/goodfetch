# "Market demand" track: synthesis for the decision register

**Status:** v1, desk research complete; interviews not started. **Date:** 2026-10-08.
**Scope:** what developers, teams and buyers say publicly about their pains, needs, budgets and competing tools (scraping, browser automation, LLM agents), from January 2025 to October 2026.
**What this track does not provide:** a measure of willingness to pay, of the market addressable by Goodfetch or of the prevalence of a pain among customers. These points fall under interviews and price tests ([pricing-signals.md](pricing-signals.md), §5).

| File | Content |
| --- | --- |
| [corpus.jsonl](corpus.jsonl) | 421 coded and sourced canonical items: one JSON line per item |
| [codebook.md](codebook.md) | 30 leaf codes, coding protocol, inter-coder agreement, source check, exclusion rules |
| [pains-ranking.md](pains-ranking.md) | Ranking of pains by raw count and by weighted score, sensitivity variants, representative items |
| [segments-and-needs.md](segments-and-needs.md) | Needs by segment, new LLM and agent uses, buyer and compliance requirements |
| [pricing-signals.md](pricing-signals.md) | Observed budgets and prices, accepted or rejected units, reading for `D-pricing` |
| [competitors-strengths-gaps.md](competitors-strengths-gaps.md) | Competitor by competitor: what is praised, what is criticized, and the reasons for switching |
| [interview-guide.md](interview-guide.md) | "Switch" (JTBD) interview guide: recruitment by segment, timeline of the switch, four forces, coding |
| [pricing-questionnaire.md](pricing-questionnaire.md) | Van Westendorp and Gabor-Granger questionnaire on the verified result, preferred unit and ceiling |
| [recruitment.md](recruitment.md) | Interview recruitment: channels, ready-to-publish announcements, direct message, anonymized tracking |

## 1. Key findings

Each finding indicates the register decisions (`lab/decisions/`) it informs, its evidence (identifiers from `corpus.jsonl`) and its robustness. A finding **informs** a decision; it does not settle it. Technical decisions are settled by the lab campaigns. Product decisions (`D-segment`, `D-pricing`, `D-feature-priority`) are settled after interviews.

### C1. Access is the top pain, and session continuity comes before stealth
`D-identity` · `D-browser-driver` · `D-feature-priority`

- **Rank.** `access-antibot-ip` is first in all four ranking variants: 76 items, W 92.4, i.e. at least 1.8 times the second.
- **Session continuity matters more than the fingerprint.** Reusing a real logged-in session removes more blocks than work on the fingerprint (`reddit-27`, `reddit-9`, `reddit-55`). What gives automation away is automation flags and a blank profile, not headless mode (`reddit-9`).
- **Common barriers are simple.** According to Zyte, WAF alone (31%) and WAF + JS (about 20%) dominate; the full stack affects only 3.3% of sites (`reviews-reports-43` `[P]`).
- **For Goodfetch.** The priority is a persistent identity profile carried by the session (option `session-identity-profile` of `D-identity`), more than heavy stealth. For `D-browser-driver`: the driver must be able to attach via CDP to an existing Chrome, given that Chrome 136 and later block the default profile (`github-19`).

### C2. Authenticated sessions must hold over time, and credentials must stay out of the LLM
`D-identity` · `D-agent-interface` · `D-segment`

- **Volume.** 32 items, 4th in W.
- **Persistence.** It breaks from one version to the next (`github-19`, `github-43`, `github-99`, `github-28`).
- **Expectations.**
  - Sessions lasting several months and a human → agent handoff for 2FA (`hn-34`).
  - Credentials resolved on the tool side, never seen by the model (`demand-use-cases-38`, `hn-35`, `demand-use-cases-68` `[P]`).
- **Usage signal.** 14% of the agent requests measured by HUMAN target account or authentication pages (`demand-use-cases-41`).
- **For Goodfetch.** Add to `D-identity` a contract gate "credential never exposed to the model". It is also the main argument for the `b2b-portals` option of `D-segment`.

### C3. Token cost and agent slowness are real, but their weight is inflated by promotional sources
`D-agent-interface` · `D-ai-layer`

- **Rank.** `agent-token-latency-cost` is 2nd in W (44 items), but drops to 5th place without promo or affiliated items.
- **Independent evidence.**
  - Tokens multiplied by 6 between two versions of Playwright MCP (`github-106`), by 2 between two versions of Browser Use (`github-22`).
  - 11.7k tokens of tool definitions per request (`hn-98`).
  - A 6-field form takes 4 to 5 minutes (`github-98`).
- **The cost comes from the harness more than from the model.** With the same model, $1.50 versus about $10 per task (`hn-89` `[P]`). Pixels can cost less than the DOM in 4 tasks out of 5 (`reddit-61`).
- **For Goodfetch.** This finding confirms the `tokens-per-verified-task` and `interface-turns-per-task` metrics already planned. It also calls for a check of the **stability across versions** of output size.

### C4. Demand for "compile once, replay without an LLM, repair on breakage" has been constant since 2025
`D-ai-layer` · `D-feature-priority`

- **Volume.** 30 `agent-deterministic-replay` items (6th in n). The requests range from January 2025 (`github-39`, `hn-0`) to September 2026 (`hn-87`), and appear in job postings (`demand-use-cases-19`).
- **Lock-in.** Stagehand's cache is tied to Browserbase sessions and is useless locally (`hn-87`, `hn-88`, `github-42`).
- **Weak point.** The announced cost gain (from $0.50-3 to $0.01-0.05 per task, `reddit-62`) is a vendor claim `[P]`. Without the promo items, the code drops from 9th to 13th place in W.
- **For Goodfetch.** This is the best market support for the `compiled-replay` option of `D-ai-layer` and for the `C-replay-without-llm` claim. The cache must be **local and versionable**. The cost gain remains to be demonstrated by the lab.

### C5. The agent interface has no winner: MCP and CLI coexist
`D-agent-interface`

- **Volume.** 37 `integ-mcp-agent-tooling` items (3rd in W, without promo and with cap; 4th after correcting the Reddit dates).
- **CLI arguments.** Advanced users switch to Playwright CLI with a skill to save context (`hn-49`, `hn-50`).
- **MCP arguments.** MCP keeps the advantage for non-developers, multi-tenant OAuth, credential isolation and audit (`hn-51`). It also presents integration risks: standard output that corrupts the protocol (`github-112`), screenshot that breaks the context (`github-24`).
- **Hosts.** Claude Code (65.5%) and Copilot (58.7%) dominate (`reviews-reports-54`).
- **For Goodfetch.** The market does not settle `D-agent-interface`. The lab's measurements (turns and tokens per verified task) are therefore the right arbiter. At least the combination of Claude Code with reduced MCP, with CLI, and with code mode must be tested.

### C6. Silent false successes are a recent and cross-cutting pain
`D-feature-priority` · `D-extraction` · `D-ai-layer`

- **Rank.** 17 `silent-false-success` items, 18th in n but 10th in W: recent and engaged.
- **Examples.**
  - Block returned as a 500 (`github-80`), handler executed on a challenge page (`github-93`), queue marked processed without execution (`github-91`).
  - Clicks reported as "ok" but ignored (`reddit-1`), completion declared too early (`github-30`).
  - Selectors that return empty strings after a redesign (`hn-101` `[P]`), inverted price diff seen as unchanged (`demand-use-cases-40`).
- **Trust.** 48% of developers trust AI only if they can verify; 78.7% consider source attribution important (`reviews-reports-51`).
- **For Goodfetch.** The `false-success-count` gate and the `C-zero-false-success` claim respond directly to a market complaint. It is a candidate positioning argument. For `D-extraction`, quality must be **measurable**: recall, tables, provenance (`github-83`, `github-66`, `demand-use-cases-61` `[P]`).

### C7. Buyers hate unpredictable bills and ask to pay per successful result
`D-pricing`

- **Rank.** `cost-unpredictable-runaway` is 5th in W and 2nd without promo (28 items). The signal does not depend on review aggregates.
- **Rejected.**
  - Credit multipliers (`reviews-reports-13`, `reviews-reports-19` `[P]`).
  - Billed failures (`reviews-reports-10` `[P]`, `reviews-reports-31`).
  - No cap: a bill about 40 times what was expected (`reviews-reports-23`), a $40 retry loop (`reddit-57`).
  - Bandwidth (`reviews-reports-12`), expiring credits (`reviews-reports-30`).
- **Requested.**
  - Payment per successful result (`reviews-reports-28`, `demand-use-cases-64` `[P]`), per lead or per record (`demand-use-cases-16`, `demand-use-cases-18`).
  - Per-task caps (`reddit-58`), pay-as-you-go (`hn-32`).
- **For Goodfetch.** This is the corpus's clearest support for the `verified-result` option of `D-pricing` and for `C-cost-per-1k-verified`. **The amount remains unknown** (see C10).

### C8. Site operators form a first-rank constraint, and politeness defaults matter
`D-identity` · `D-feature-priority`

- **Rank.** 25 `compliance-site-owner-politeness` items, between 4th and 7th place depending on the variant. Engagement is the strongest in the corpus: 462 and 357 HN points (`hn-125`, `hn-126`), 44 and 49 on GitHub (`github-0`, `github-1`).
- **Defaults are blamed.** Scrapy's defaults took sites down (`github-1`). Shared IPs of cloud browsers are blocked wholesale (`demand-use-cases-31`, `hn-110`). Buyers want to control crawl etiquette (`hn-62` `[P]`).
- **For Goodfetch.** This finding confirms the `identified` and `standard` profiles of ADR-002, with robots.txt and throttling **enabled by default**. It is also an argument for enterprise buyers (C9).

### C9. For enterprises, compliance is the top purchase criterion, and part of it costs almost nothing if designed early
`D-segment` · `D-feature-priority`

- **Rank.** Compliance is the top code of the `enterprise` segment (15 items out of 29).
- **Requirements.** SOC 2, DPA, retention, subprocessors (`demand-use-cases-62` `[P]`, `demand-use-cases-64` `[P]`). Per-record provenance and EU AI Act (`demand-use-cases-63` `[P]`, `demand-use-cases-39`). BAA and zero retention (`hn-25`, `hn-31`, `hn-91`). Local execution (`demand-use-cases-10`, `hn-92`).
- **Order of priorities.** The buyer with a budget talks about compliance before quality (`demand-use-cases-59`).
- **For Goodfetch.** The per-action audit log and per-record provenance are a matter of architecture, not certification. They are inexpensive if planned from the start. SOC 2 is out of scope as long as `D-segment` has not chosen enterprise.

### C10. Segments: agent developers talk the most, B2B portals show the highest prices, and none is proven
`D-segment` · `D-pricing`

- **Agent developers.** 44% of the corpus (186 items), but low willingness to pay and strong DIY pressure via coding agents (`hn-71`, `demand-use-cases-27`). Only 6.4% switch tools for cost (`reviews-reports-53`).
- **B2B portals.** The only high prices, $2,500/month (`hn-33`) and Intuned's customer volumes (`demand-use-cases-26`), come from vendors `[P]`.
- **E-commerce price monitoring.** Bimodal: less than $30/month among SMBs (`reddit-43`); fastest growth according to Mordor (`demand-use-cases-45`).
- **Data teams.** Real budgets for proxies and infrastructure, but 46.7% do everything in-house (`reviews-reports-38` `[P]`).
- **For Goodfetch.** A hypothesis to test in interviews, not a conclusion: **adoption** would come through agent developers (OSS, MCP, CLI), **monetization** through verified-result workloads (portals, data). See [segments-and-needs.md](segments-and-needs.md), §5.

### C11. Switching tools: people move to hosted for reliability and leave it for price; self-hosting must be complete
`D-pricing` · `D-segment`

- **Toward hosted.** When proxy management becomes a full-time job (`reddit-3`, `reddit-6`, `reviews-reports-14`).
- **Away from hosted.** For price (`hn-58`, `hn-56`, `hn-63`).
- **Self-hosting.** 20 `selfhost-parity-lockin` items. OSS is perceived as an on-ramp to the cloud (`github-42`). Self-hosted versions lack anti-bot (`reviews-reports-31`), live view (`github-101`), proxies (`github-52`).
- **For Goodfetch.** The boundary between open source and paid must be legible. **Removing anti-bot or the cache from the self-hosted version is precisely what is criticized.**

### C12. Memory performance and resumption: rarely expressed, therefore necessary but not differentiating
`D-browser-driver` · `D-feature-priority`

- **Rank.** `perf-memory-concurrency` (15 items, 22nd in W) and `durability-resume` (5 items, last) are at the bottom of the ranking.
- **Memory only draws complaints at volume.** OOM around 18-20 sessions (`reddit-60`), more than 20 GB with Chrome for Testing (`github-104`). LLM cost dominates browser cost in agent workflows (`hn-14`).
- **Resumption appears as a requirement** in specifications (`demand-use-cases-1`, `demand-use-cases-10`, `github-5`).
- **For Goodfetch.** For `D-browser-driver`, a speed or memory gain **is not enough as an argument**. Actionability compliance and session stability matter more. The `D-durable-runtime` decision remains a matter of correctness, not positioning.

## 2. Decisions × findings matrix

| Decision | Findings | What the market track contributes | What remains to be measured |
| --- | --- | --- | --- |
| `D-segment` | C2, C9, C10, C11 | Volume of voices, pains by segment, price signals (biased) | Interviews: real budget, urgency, purchase cycle; at least 5 per candidate segment |
| `D-pricing` | C7, C10, C11 | Rejected or accepted units; `verified-result` best aligned | Amount: interviews, landing-page price test, paid PoC |
| `D-feature-priority` | C1, C4, C6, C8, C9, C12 | Backlog ranked by pain (`pains-ranking.md` §2, no-promo variant) | Weighting by segment once `D-segment` is settled |
| `D-browser-driver` | C1, C12 | CDP attachment to a real Chrome, profile stability; speed not very differentiating | Actionability campaigns already planned |
| `D-ai-layer` | C3, C4, C6 | Constant demand for compiled replay; local cache required | `usd-per-1k-verified`, false successes |
| `D-agent-interface` | C2, C3, C5 | No market winner; dominant hosts; requirements on credentials | `interface-turns-per-task`, tokens per verified task |
| `D-identity` | C1, C2, C8 | Session continuity before stealth; polite defaults; credentials out of the LLM | `identity-coherence`, `policy-compliance` |
| `D-extraction` | C6 | Measurable quality: recall, tables, provenance, correct diffs | `extraction-f1`, per-field precision and recall |

## 3. Method

1. **Collection.** Five independent, read-only collectors:
   - **GitHub**: 12 repositories and keyword searches;
   - **Reddit**: via a third-party snapshot site;
   - **HN**: Algolia API;
   - **Reviews and reports**: Trustpilot, G2 summaries, Capterra mirrors, Gartner, Product Hunt, vendor reports;
   - **Demand and use cases**: Upwork, Workana, case studies, buying guides.

   The collectors surfaced 465 items according to their notes. The workflow passed on 429 canonical items.
2. **Coding.** A first LLM coder assigns 1 to 3 codes among 30 leaves ([codebook.md](codebook.md)). A second, independent coder recodes a stratified sample of 120 items.
3. **Source check** on 60 items: URL, date, fidelity of the summary. The 8 failures are excluded, which leaves **421 items**.
4. **Ranking.**
   - Raw count n and weighted score `W = Σ (1 + ln(1 + engagement)) × 0.5^(age_months / 6)` as of 2026-10-08.
   - Variants: without promo or affiliates, engagement capped at 50, correction of the Reddit date bias.

Breakdown of the final corpus:

- **Sources**: HN 127, GitHub issues 113, Reddit 68, reports 29, blog posts 28, reviews 26, job postings 23, GitHub discussions 6, other 1.
- **Kind**: `pain` 138, `feature-request` 83, `use-case` 65, `pricing-signal` 51, `statistic` 50, `switching` 21, `praise` 13.
- **Period**: from 2024-11 (a single item) to 2026-10; 262 items date from 2026.
- **Promo**: 98 `promo` or `vendor_affiliated` items, kept and flagged.

## 4. Inter-coder agreement and source check

| Check | Result |
| --- | --- |
| Agreement on the primary code (n = 120) | **83.3%** (100/120), without chance correction |
| Mean Jaccard over the full set of codes (n = 120) | **0.838** |
| Sources verified | 60: **52 passed, 8 failed (13.3%), excluded** |
| Failure reasons | 7 dates: 5 Reddit snapshot dates later than the post, by 4 to about 102 days (`reddit-5`, `reddit-10`, `reddit-23`, `reddit-69`, `reddit-71`), and 2 unverifiable review months (`reviews-reports-4`, `reviews-reports-6`); 1 unfaithful summary (`github-4`: a workaround existed) |
| Caveats on retained items | 8 `check_note` notes, for example generalization from a single author (`github-94`) or an unestablished qualifier (`demand-use-cases-4`, `demand-use-cases-15`) |
| Residual bias | Reddit dates are snapshot dates: up to about 3 months of offset, recency weight up to about 1.5 times too strong. Corrected in a test: same top ten pain codes, ranks 3 to 5 permuted |

**Reading.** The agreement is sufficient to rank pain families. It does not separate two codes less than about 15% apart in score. The 13.3% failure rate mostly concerns **dates**, not content: only 1 unfaithful summary out of 60.

## 5. Coverage and gaps

According to the collectors' notes:

- **GitHub.**
  - The search API caps at 30 results per query, so the long tail is missing.
  - Comments were not read in depth.
  - There are few explicit price discussions, and Scrapling and Spider have little engagement.
  - Playwright issues were filtered on scraping, memory and agents.
- **Reddit.**
  - No direct access (403, domain refusal, Anubis), and no workaround attempted.
  - Everything goes through third-party snapshots. Coverage runs from March to September 2026 only, with no 2025.
  - Engagement corresponds to the captured comments, without votes. Subreddits are not tagged.
  - r/dataengineering is poorly covered. There are few direct complaints about Apify, Browserbase or Bright Data prices.
- **HN.**
  - Steel and Kernel are almost absent. No Apify items: the threads found were promotional.
  - Crawl4AI is little discussed. The September-October 2026 threads are still small.
- **Reviews and reports.**
  - The G2 and capterra.com pages were inaccessible: G2's AI summary and regional Capterra mirrors were used instead.
  - The Trustpilot pages of Firecrawl, ScrapingBee and ScraperAPI were inaccessible. Independent reviews of Firecrawl are thin.
  - Summaries written by competitors (Thunderbit) are flagged `[P]`.
  - The Bright Data 2026 PDF was unreadable: only the titles are cited. The Apify survey does not publish its sample size.
  - Several undated items were discarded.
- **Demand.**
  - Upwork postings skew toward September-October 2026 and toward low budgets.
  - G2, Capterra, LinkedIn and X were not covered.
  - Case studies come mostly from vendors. Willingness to pay expressed by end users is thin.
  - Segments are a matter of the coder's judgment.
- **Absent from the entire track:**
  - interviews;
  - closed communities (Discord, Slack);
  - enterprise procurement voices, outside vendor guides;
  - non-English sources, except two cases cited in English (Turkey, Brazil);
  - Asian markets.

## 6. Limits

1. **Public voice ≠ market.** Agent developers are overrepresented (44%) because their tools have public trackers and HN discusses them. Enterprise buyers and data teams speak little in public.
2. **Heterogeneous engagement.** Reactions, points, comments and review counts are not comparable with one another. W is ordinal, and review aggregates weigh heavily (68% of the weight of `dx-setup-docs`).
3. **Promotional sources.** Market statistics are 66% promo or affiliated. High prices and announced gains come from vendors.
4. **LLM coding.** The first coder is an LLM and agreement with the second coder is good, but no human validation of the sample is documented in this synthesis. The detail of the disagreements is not available.
5. **Desk research ≠ willingness to pay.** No figure in this dossier should serve as a Goodfetch price without interviews or a test (`D-pricing`, register note).
6. **Dated snapshot.** Corpus frozen as of 2026-10-08, with a 6-month half-life. An update requires new coding and a new check (codebook §6).

## 7. Next actions for the track

1. **Interviews**: guide derived from C1 to C12. At least 5 interviews per candidate segment of `D-segment`; questions on the budget breakdown and price thresholds for a defined "verified result".
2. **Price test**: landing page comparing `verified-result` and a capped subscription.
3. **Quarterly refresh**: same protocol, adding the missing sources (§5).
