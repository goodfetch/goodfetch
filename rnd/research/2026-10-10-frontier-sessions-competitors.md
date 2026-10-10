# Frontier and sessions at competitors (2026-10-10)

Evidence for [Adopt collection decisions](https://github.com/Jbmanllr/automan/issues/71). Read from source at pinned tags and from official docs on 2026-10-10; Exa search had no credits, so nothing comes from web search. "Inferred" marks a reading of code that the docs do not state.

Versions: Crawlee JS [v3.18.2](https://github.com/apify/crawlee/tree/v3.18.2) and [v4.0.0-rc.1](https://github.com/apify/crawlee/tree/v4.0.0-rc.1), Crawlee Python [v1.10.4](https://github.com/apify/crawlee-python/tree/v1.10.4), Scrapy [2.19.0](https://github.com/scrapy/scrapy/tree/2.19.0) with w3lib v2.4.0, spider [2.53.9](https://docs.rs/crate/spider/2.53.9/source/) (crate source), Firecrawl [6f892bc](https://github.com/firecrawl/firecrawl/tree/6f892bc) (v2.11.504), Crawl4AI [v0.9.4](https://github.com/unclecode/crawl4ai/tree/v0.9.4), Stagehand [4.2.0 / 2d605b0](https://github.com/browserbase/stagehand/tree/2d605b0), Browser Use [0.13.11](https://github.com/browser-use/browser-use/tree/0.13.11), Skyvern [v1.0.55](https://github.com/Skyvern-AI/skyvern/tree/v1.0.55), plus docs.apify.com, docs.firecrawl.dev, docs.browserbase.com, docs.browser-use.com and skyvern.com/docs.

## Frontier

| | URL dedup rules | Learned rules | Trap controls | Reports a limit stop |
| --- | --- | --- | --- | --- |
| Crawlee JS | strip `utm_*`, sort params, drop fragment and one trailing `/`, lowercase scheme and host (`@apify/utilities` `normalizeUrl`) | none | `maxCrawlDepth`, `maxRequestsPerCrawl` (no defaults) | no: `FinalStatistics` has no stop reason; a log line only |
| Crawlee Python | same, but lowercases the whole URL, path and query included (`_utils/requests.py`) | none | depth, request cap | no: `stop(reason=…)` is logged only |
| Scrapy | w3lib `canonicalize_url`: sort params, uppercase `%xx`, drop fragment, lowercase host; no tracking-param stripping | none | `DEPTH_LIMIT` (0 = unlimited), `CLOSESPIDER_*` | yes, `finish_reason = closespider_*`; a depth cutoff still ends `finished` |
| Spider | case-insensitive whole URL, fragment dropped; no sorting, no `utm_` stripping | none | `depth: 25`, per-path `budget`, regex lists | per link only (`BudgetExceeded`); no crawl-level status |
| Firecrawl | `deduplicateSimilarURLs` (default on) merges `www`, http/https, trailing slash, `index.html`/`index.php`; hash routes kept | none | `limit` 10,000 default, `maxDiscoveryDepth` | no: `completed` at the limit |
| Crawl4AI | strips `utm_*`, `gclid`, `fbclid`, `ref`, `ref_src`, sorts params, drops fragment | none | `max_depth`, `max_pages`, score threshold | no: logged only |
| Stagehand, Browser Use, Skyvern | no crawl frontier | — | Skyvern loops capped at 1,000 | Skyvern only: `REACH_MAX_STEPS` on a failed run |

- Priority: Crawlee has forefront insertion only; Spider has an `i32` priority heap (`priority_frontier`); Scrapy pops by priority per download slot; Crawl4AI has best-first scoring.
- Pagination: no project has an offset-vs-cursor rule; it is user code (callbacks, `actions`, `js_code`, infinite-scroll helpers).
- Request leases: Crawlee locks requests with an expiring client-key lease (v3 `requestLockSecs` ≈ handler timeout + 5 s); Crawlee Python's SQL client uses `FOR UPDATE SKIP LOCKED`. None carries an epoch.

## Politeness (handed to site access)

| | robots.txt default | Crawl-delay applied | Retry-After |
| --- | --- | --- | --- |
| Crawlee JS v3 | off | no | no |
| Crawlee JS v4, Python | off | only via an opt-in `ThrottlingRequestManager` | same |
| Scrapy | off (on in the project template) | parsed since 2.18, not applied (inferred from grep) | no |
| Spider | off | yes, capped at 60 s | yes on 429, capped at 30 s (`headers` feature) |
| Firecrawl | on | parsed, applying it is commented out | not found |
| Crawl4AI | off | no | no |

## Sessions

| | Session pool | Lost-login detection | Login reuse | MFA | Human takeover | Account rotation |
| --- | --- | --- | --- | --- | --- | --- |
| Crawlee JS/Python | `SessionPool`: 1,000 sessions, error score max 3, 50 uses, ~50 min; proxy pinned per session; v4 rotates sessions instead of proxy tiers | blocked codes `[401, 403, 429]` plus 3 challenge selectors (Turnstile, Google sorry, Incapsula); no login check | session cookies, `userDataDir`; Python login guide | no | no | no |
| Apify | Crawlee's pool; proxy sessions ≤ 50 chars, ~26 h datacenter, ~30 min residential | no | community Actors only | no | no | no |
| Scrapy | per-request cookie jars (`meta["cookiejar"]`) | no | no | no | no | no |
| Spider | round-robin proxy rotator, `RetryStrategy` | anti-bot vendor detection, not login | external Chrome (inferred) | no | no | no |
| Firecrawl | `proxy: auto` escalates to stealth on 401/403/429 | no | `profile` (saved browser state) | via live view (implied) | `interactiveLiveViewUrl` | no |
| Crawl4AI | sticky round-robin proxies | layered anti-bot detector, not login | `BrowserProfiler` (visible browser, save profile) | via the profiler | visible browser | no |
| Stagehand v4 + Browserbase | none in SDK; Browserbase contexts persist the user data dir | none ("implement checks… to detect logged-out states") | contexts | no TOTP; human via Live View | Live View; Agents `PAUSED` with `%var%` secrets | no |
| Browser Use | profiles, `storage_state` synced every 30 s | LLM judgement only | profiles, cloud profile sync | TOTP from `*bu_2fa_code` secrets; 1Password in cloud | `pause()`; cloud live view | no |
| Skyvern | credentials with sticky IP per account, no concurrent use | LLM judgement, keyword failure categories | saved profile per credential | TOTP, email/SMS codes pushed or polled (10 s, 15 min), magic links | approve/reject block, "Take control" stream | no |

## Findings

- No competitor learns crawl traps or no-effect parameters; fixed rules differ by project, and Crawlee Python and Spider lowercase whole URLs, which can merge distinct pages.
- Only Scrapy names a limit stop; Crawlee, Firecrawl and Crawl4AI report a limit stop like a finished crawl.
- No competitor detects a lost login from page content; the HTTP-status approach lets a `200` login page through.
- MFA automation exists in Browser Use and Skyvern; human takeover by live view in Firecrawl, Browserbase, Browser Use and Skyvern.
- No competitor rotates accounts.
- Stagehand 4.x removed `agent()`; step budgets and pausing now live only in Browserbase Agents.
