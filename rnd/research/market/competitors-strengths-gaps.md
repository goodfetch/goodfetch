# Competitors: what users praise, what they criticize, why they switch

**Status:** v1, desk research. **Date:** 2026-10-08. **Corpus:** 421 items ([corpus.jsonl](corpus.jsonl)).
**Limitations:**

- **Source asymmetry.** OSS tools (Browser Use, Firecrawl, Crawl4AI, Crawlee, Scrapy, Skyvern, Scrapling, Spider) are judged mostly through their **GitHub issues**, which by nature carry complaints rather than praise. Closed services (Bright Data, Zyte, Oxylabs, ScrapingBee, ScraperAPI) are judged through **reviews**, which are more balanced but aggregated and sometimes written by competitors (`[P]`).
- **What this document does not do.** A complaint count does not measure a product's quality, and this document does not rank competitors. For verified capabilities and versions, see [the commercial landscape](../2026-10-07-commercial-landscape.md), [the browser frameworks](../2026-10-07-browser-frameworks.md) and [the crawl engines](../2026-10-07-crawl-engines.md).

## 1. Firecrawl (43 items)

- **Praised:**
  - Fast onboarding, clean LLM-ready Markdown, schema-based extraction, official MCP server for Claude and Cursor (`reviews-reports-29`, `reviews-reports-32`).
  - Effective for building company lists (`hn-124`).
  - The self-hosted version runs with fewer than 2% failures (`hn-63`).
  - Reported growth: 1.5M+ users, $75M Series B (`reviews-reports-56` `[P]`, `hn-107`).
- **Criticized:**
  - **Price**: considered excessive (`hn-60`); a subscription of about $190/month replaced by in-house code (`hn-58`).
  - **Credits**: hard to predict and not rolled over (`reviews-reports-29`, `reviews-reports-30`); extraction up to 10 times the base credit (`reviews-reports-32`); agent endpoint billed even on failure (`reviews-reports-31`).
  - **Hard sites**: a fallback to Bright Data is still needed (`reviews-reports-32`). Reddit and LinkedIn are refused (`hn-61`, `reviews-reports-31`).
  - **Self-hosting**: the Fire-Engine anti-bot engine and the agent and browser endpoints are missing (`reviews-reports-31`), as are proxies (`github-52`); database migrations are absent (`github-57`).
  - **Robustness**: infinite loops on blocked pages (`github-53`, `github-54`), opaque error "All scraping engines failed" (`github-55`), crawls stuck in "scraping" status (`github-63`), non-deterministic screenshots (`github-64`).
  - **Change tracking**: it compares sorted characters, so a change from $12 to $21 is seen as unchanged (`demand-use-cases-40`).
  - **LLM choice**: extract limited to OpenAI, Ollama requiring a Google key (`github-61`, `github-62`).
  - **Crawl**: robots.txt handling considered inconsistent (`hn-62` `[P]`), site operators asking for an identifiable user agent (`github-60`), a $40 retry loop on the client side (`reddit-57`).
- **Switches:**
  - To Kagi Extract with a fallback to Cloudflare's browser API, for cost (`hn-56`).
  - To in-house code, for price and polish (`hn-58`).
  - To self-hosted Firecrawl, to get away from credits (`hn-63`).
  - Departure considered to control crawl etiquette (`hn-62` `[P]`).
- **Of note:** most of the well-ranked "Firecrawl reviews" are reportedly written by competitors (`reviews-reports-30`). One page hides text aimed at AI agents to steer their sign-up (`hn-108`).

## 2. Browser Use (41 items)

- **Praised:** no explicit praise item in the corpus, which contains 15 `pain` items and 14 requests. One user signed up for the paid cloud tier at launch (`hn-13`).
- **Criticized:**
  - **First launch** looping on `about:blank` (`github-13`, 55; `github-14`, 51).
  - **Tokens** doubled between 0.8.1 and 0.11.1 (`github-22`); up to 40k characters returned at each step (`reddit-22` `[P]`).
  - **Stealth** regressing after Patchright and the move to all-CDP (`github-26`, `github-27`).
  - **Real Chrome profile** broken by Chrome 136 (`github-19`); unreliable `storage_state` (`github-28`).
  - **Cloud** failing while the status page is green (`github-29`).
  - **Agent behavior**: completion declared too early (`github-30`), no recovery after a page crash (`github-23`), Claude Code context corrupted by a screenshot (`github-24`).
  - **Dependencies and security**: versions pinned with `==` (`github-32`), litellm compromised via the CLI (`github-33`), main Chrome driven over CDP without a sandbox (`hn-7`, criticism from a competitor `[P]`).
  - **Unpredictability, slowness and cost** (`reddit-25`, `hn-10`, `hn-11`).
- **Requested:**
  - Human control: human intervention (`github-11`, 42), MFA (`github-12`).
  - Replay: export of runs as a script (`github-15`), cache (`github-16`), assertions (`github-25`).
  - Integration: Docker image for self-hosting (`github-17`), MCP (`github-18`, `hn-8`), captcha (`github-20`), tokens in logs (`github-21`).
  - Compliance: EU AI Act (`demand-use-cases-39`).
- **Switches:**
  - Abandoned for unpredictability (`reddit-25`).
  - In a 5-task pilot, Computer Use was cheaper in 4 cases, and the DOM won on a dense product grid (`reddit-61`).
  - On an internal administration task: 47 steps and ~495k tokens for the vision agent, versus 8 calls and ~12k tokens for an API agent (`hn-99` `[P]`).

## 3. Stagehand and Browserbase (34 items)

- **Praised:**
  - Proxies and captchas built into the abstraction (`hn-4`).
  - Browser sessions for agents that are easy to launch and scale (`reviews-reports-33`).
  - Much cheaper than Simplex (`hn-36`).
  - v4 groups commands (batching) in an extension (`hn-86` `[P]`).
  - Volumes: about 37M sessions/month and 10,000+ companies (`demand-use-cases-48`); 4M npm downloads per month (`hn-86` `[P]`).
- **Criticized:**
  - **OSS seen as an on-ramp to the cloud**: persistence reserved for paid accounts (`github-42`), action cache tied to Browserbase sessions (`hn-87`), acceleration reserved for the cloud (`hn-88`).
  - **Version regressions**: session persistence broken in v3 (`github-43`), incompatibility with the AI SDK (`github-49`).
  - **Extraction and accuracy**: URLs hallucinated by `extract()` (`github-46`), `act()` spilling over into navigation (`github-47`), brittle cached XPaths (`github-41`), inaccurate local models (`hn-5`).
  - **Coverage**: Python SDK requested (`github-48`), attaching to existing Playwright (`github-38`), user-supplied browser (`github-44`), edge runtime (`github-45`).
  - **LLM cost** at runtime in a test suite (`hn-0`, `hn-1`).
  - **Browserbase risks**: bans for ToS violations without explanation (`reviews-reports-35`), shared IPs blocked wholesale because of other customers (`demand-use-cases-31`, `hn-110`), account-ban risk unchanged (`hn-109` `[P]`), thin tutorials (`reviews-reports-34`).
- **Switches:**
  - Libretto left Stagehand for code generation at development time, deterministic at runtime (`hn-27` `[P]`).
  - "Why Stagehand if the agent writes CDP?": the vendor answers tokens, performance, observability and policies (`hn-90`).
  - Stagehand experienced as "babysitting" (`reddit-25`).

## 4. Playwright, Playwright MCP and Chrome DevTools MCP (40 items)

- **Praised:**
  - Models already know the Playwright API, so a dedicated MCP or skill becomes superfluous (`hn-42`).
  - DevTools MCP is considered more reliable and more economical than other CDP MCPs (`hn-52`).
  - Console and network inspection helps agent-driven debugging (`hn-29`).
- **Criticized:**
  - **Tokens**: multiplied by 6 between two MCP versions (`github-106`); snapshots too long (`github-107`); context saturated after a few searches (`github-108`); 11.7 to 13.7k tokens of tool definitions (`hn-98`, `reddit-14` `[P]`); DevTools MCP very hungry (`hn-45`).
  - **Detection**: Google sign-in blocked (`github-109`); Chromium launched by Playwright spotted through its automation flags (`reddit-9`).
  - **Containers**: profile lock in a devcontainer (`github-111`).
  - **Resources**: Chrome for Testing imposed, more than 20 GB per instance (`github-104`); OOM around 18-20 concurrent sessions (`reddit-60`).
  - **Beyond simple tests** (`hn-43`).
- **Switches:**
  - To Playwright CLI with a skill, about 1-2k of context per interaction (`hn-49`, `hn-50`).
  - Wrappers that write to disk or summarize snapshots (`hn-47` `[P]`, `hn-48`).
  - Indecision between MCP, CLI, the Codex browser and Claude for Chrome (`hn-46`).

## 5. Crawl4AI (14 items)

- **Praised:** no praise item. The corpus contains only issues.
- **Criticized:**
  - **Resources**: Docker server memory leak (`github-69`), orphaned Chrome processes (`github-70`).
  - **Navigation**: "Target closed" (`github-71`, 37), `ERR_ABORTED` during pagination (`github-72`, 40).
  - **Incomplete crawl**: DFS regression (`github-73`), incomplete `arun_many` (`github-74`), infinite scroll losing items (`github-79`).
  - **Docker server gaps**: parity with the library (`github-76`), Groq key ignored (`github-77`), LLM table extraction unreachable through the job queue (`github-81`).
  - **Poorly reported failures**: empty LLM extraction returned as a success (`github-78`), blocking returned as HTTP 500 (`github-80`).
  - **Serverless**: Lambda (`github-75`).

## 6. Crawlee and the Apify platform (24 items)

- **Crawlee, criticized:**
  - **Autoscaling**: pushed to OOM on Kubernetes (`github-84`); no backoff on 429s, hence a spiral acknowledged by the maintainers (`github-85` `[P]`); domain hammered in v4 (`github-96`); RAM climbing with large proxy pools (`github-86` `[P]`).
  - **Request queue**: deadlock (`github-87` `[P]`), requests marked as handled without the handler running (`github-91`), expired locks producing duplicates (`github-92`).
  - **Anti-bot**: Cloudflare handler that does not wait for the challenge to finish (`github-93`).
- **Crawlee, requested:** hybrid HTTP-then-browser crawler (`github-94`), OpenTelemetry (`github-95`), web API in Python (`github-89`), Scrapling as an option (`github-90`).
- **Apify, praised:**
  - Ready-to-use Actors, built-in scheduling and proxies that replace fragile scripts (`reviews-reports-0`, Trustpilot 4.8 from 439 reviews).
  - Integrations and marketplace (`reviews-reports-3`, G2 4.7 from 611 reviews).
  - Store of 64,812 Actors, of which 97.1% are pay-per-event and 2,265 are MCP Actors (`reviews-reports-59`).
  - 87% of runs launched via API (`reviews-reports-58` `[P]`).
- **Apify, criticized:**
  - **Reliability and support**: inconsistent scrapers, ignored parameters, silent support (`reviews-reports-1`); unmaintained community Actors (`reviews-reports-7`).
  - **Price**: compute units expensive at volume and custom Actors hard to write (`reviews-reports-3`); cumulative proxy fees (`reviews-reports-2`); short runs billed, item limits not respected, expiring credit (`reviews-reports-7`).
  - **Reviews**: rewards offered to review authors weaken the 4.8 rating (`reviews-reports-7`).
  - **Debugging and MCP**: debugging scattered across several screens, MCP reconnection needed at each IDE change (`reviews-reports-5`).
- **Switches:** a KYC startup abandoned its in-house Facebook scraper, stuck below 80% accuracy, and cut its costs by 30% compared with a SERP API (`demand-use-cases-56` `[P]`).

## 7. Scrapy (11 items)

- **Criticized:**
  - **Politeness**: aggressive defaults that have taken sites down; robots.txt and 403s ignored (`github-0`, 44; `github-1`, 49; `github-2`).
  - **Performance**: 2.13 regression (`github-3`).
  - **Durability**: `JOBDIR` only survives a clean shutdown (`github-5`).
  - **Correctness**: duplicates after retry (`github-7`), possible loss of items (`github-8` `[P]`), rate limiting to be rethought according to a maintainer (`github-6` `[P]`).
- **Switches:**
  - To a hosted API when every site requires rendering, proxies and captchas (`reddit-6`).
  - Question about the value of Scrapling (`github-9`).
  - Migration from Scrapy and BeautifulSoup to AI scrapers, because of selector brittleness, according to an aggregator of Reddit threads (`reddit-81`).
  - Scrapy remains the runtime when an agent writes the per-site configurations (`demand-use-cases-33` `[P]`).

## 8. Skyvern (12 items)

- **Praised:** customers use it mostly to log in, fill in forms and download files (`hn-23` `[P]`); multi-site purchases (`hn-81` `[P]`).
- **Criticized:**
  - **Slowness**: about 5 minutes per administration workflow (`github-97`), 4-5 minutes for a 6-field form (`github-98`).
  - **Sessions**: cookies not persisted (`github-99`).
  - **Self-hosting**: no proxy pool (`github-100`), live view and takeover reserved for the cloud (`github-101`), installation failing on Windows (`github-102`).
  - **Captchas**: the agent guesses a captcha it cannot see (`github-103`).
  - **Compliance**: compliance pages and BAA nowhere to be found (`hn-25`).

## 9. Scrapling and Spider (10 items)

- **Scrapling:**
  - Standard output corrupting the MCP protocol (`github-112`), temporary profile (`github-113`), blocking on Cloudflare (`github-115`), locale silently set to en-US (`github-116`).
  - Requests: agent skill (`github-114`), XHR capture (`github-117`).
  - Serves as a reference for in-house code generated by Codex (`hn-71`).
- **Spider (Rust):**
  - Links extracted before JS rendering finishes (`github-118`), logs flooded with remote CDP (`github-119`).
  - Requests: LLM-driven navigation (`github-120`), per-page duration for politeness (`github-121`).
  - Low engagement: little data.

## 10. Lightpanda (7 items)

- Interest: Chrome is among the most resource-hungry components of an MCP stack (`hn-18`), and browser cost weighs heavily at 20M pages/day (`hn-15` `[P]`).
- Skepticism: LLM cost dominates, and a RAM benchmark on a simple page is misleading (`hn-14`).
- Gaps: documentation of safe uses (`hn-17`); without rendering, no screenshot for debugging (`hn-19`); anti-bot is not a priority (`hn-20`); politeness requested (`hn-16`).

## 11. Access APIs and proxies

| Service | Praised | Criticized | Sources |
| --- | --- | --- | --- |
| **Bright Data** | Support, end of the fight against captchas and IP blocks (Trustpilot 4.4 from 969); breadth of the offering | Pricing structure, slow responses; failures billed with custom headers; complex configuration, sites that still block; trial via a sales call, KYC, unpredictable bandwidth | `reviews-reports-8`, `reviews-reports-9`, `reviews-reports-10` `[P]`, `reviews-reports-11`, `reviews-reports-12` |
| **Zyte** | Unblocks almost everything on the first try; discounts on hard sites; Scrapy ecosystem | No cap without a subscription, bill multiplied by about 40; card required despite the "no card" claim; onboarding and interface | `reviews-reports-20`, `reviews-reports-21`, `reviews-reports-22`, `reviews-reports-23`, `reviews-reports-24` `[P]` |
| **Oxylabs** | Proxy speed, unlimited bandwidth, consistent API (Gartner 4.7 from 15) | Trustpilot 3.7 from 724: billing discrepancies, refunds, blocked accounts (KYC, geo); advertised volumes calculated at the lowest rate | `reviews-reports-25`, `reviews-reports-26`, `reviews-reports-27`, `reviews-reports-28` |
| **ScrapingBee** | Consistent where cheaper services failed; a single API call; ends proxy management | Multiple credits per option, concurrency caps; no scheduler; 1k-credit trial; documentation of edge cases | `reviews-reports-14`, `reviews-reports-15`, `reddit-3`, `reviews-reports-13`, `reviews-reports-16`, `reviews-reports-17` |
| **ScraperAPI** | Simple and reliable | Confusing credit breakdown; unannounced multipliers; weak on hard targets | `reviews-reports-18`, `reviews-reports-19` `[P]` |

## 12. Automation- and agent-generation platforms

This section groups Intuned, Libretto, Simplex, Notte, BrowserBook, Smooth and Hyperbrowser.

- **Praised:**
  - No AI at runtime, which provides reliability (`hn-73` `[P]`, `demand-use-cases-26` `[P]`).
  - Brittle but deterministic scripts, preferred over agents (`hn-41`).
  - Approach validated by a developer who built a similar internal tool, where the agent fixes the script after a failure (`hn-28`).
- **Criticized:**
  - **Price**: Simplex goes from a free trial to $2,500/month (`hn-32`); opaque Notte credits before clarification (`hn-75` `[P]`).
  - **Reliability**: complete failure of Notte on a hotel site (`hn-74`); Libretto waits for the production outage before repairing (`hn-30`).
  - **Security and compliance**: Smooth without security documentation or zero retention by default (`hn-91`) or self-hosting (`hn-92`); BrowserBook stores notebooks on its servers (`hn-40`); health data sent to the LLM (`hn-31`).
  - **Installation**: painful MCP installation (`hn-78` `[P]`).
- **Competitive pressure:** a general coding agent combined with an OSS library would cover most needs (`hn-71`, `demand-use-cases-27`).

## 13. Cloudflare, a gatekeeper rather than a competitor (6 items)

- **Default settings**: agents and training bots blocked by default; Web Bot Auth signature considered favorable to the big labs (`reddit-70`).
- **Paying for crawling**: skepticism about pay-per-crawl (`reddit-72`, `reddit-73`); request for an open permissions and payment protocol (`hn-112`).
- **Limit of its own service**: its crawler does not get past its own protections either (`hn-111`).

## 14. Reasons for switching: summary

| Move | Dominant reason | Sources |
| --- | --- | --- |
| In-house code or OSS → hosted API | Proxies, rendering and captchas become a full-time job; consistency | `reddit-3`, `reddit-6`, `reviews-reports-14`, `reviews-reports-0` |
| Hosted API → in-house code or self-hosting | Price, credits, polish; coding agents make DIY cheap | `hn-58`, `hn-56`, `hn-63`, `hn-71`, `demand-use-cases-27` |
| Agent at runtime → generated or replayed code | Non-determinism, cost and latency | `hn-27` `[P]`, `hn-0`, `hn-37`, `reddit-62` `[P]`, `hn-100` `[P]` |
| MCP → CLI and skill | Context and tokens | `hn-49`, `hn-50`, `hn-47` `[P]` |
| Several providers → a single access point | Total cost and success rate | `demand-use-cases-53` `[P]`, `demand-use-cases-55` `[P]` |
| Mixing providers by task | None covers everything | `hn-66`, `reviews-reports-37` `[P]` |

## 15. Recurring gaps that no competitor in the corpus fills

These are hypotheses for the lab to settle, not demonstrated advantages of Goodfetch:

1. **Local, versionable cache and replay, independent of any cloud.** Stagehand's cache is tied to Browserbase and persistence is paid (`hn-87`, `hn-88`, `github-42`); this is a lock-in issue.
2. **Typed failures, with no false success.** A block must not come back as a 500, an empty list must not pass for a success, a queue must not claim to be handled when it is not (`github-80`, `github-78`, `github-91`, `github-29`). See `C-zero-false-success`.
3. **Billing that does not count failures, with hard caps** ([pricing-signals.md](pricing-signals.md)). See `C-cost-per-1k-verified`.
4. **Sessions and profiles that survive updates** of Chrome and the framework (`github-19`, `github-43`, `github-99`).
5. **The same product when self-hosted**, including anti-bot and live view (`reviews-reports-31`, `github-52`, `github-101`).
6. **Politeness and identity by default** (`github-0`, `github-1`, `hn-62`, `demand-use-cases-31`). See `D-identity`.
7. **An agent interface that is economical and stable from one version to the next** (`github-106`, `github-22`, `hn-98`). See `D-agent-interface`.
