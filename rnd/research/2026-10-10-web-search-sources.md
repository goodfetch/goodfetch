# Web search sources for agents: provider tools, Google by API or browser, search APIs

Research of **October 10, 2026** for ticket [#93](https://github.com/Jbmanllr/automan/issues/93). It feeds the "web search through a provider (sources under research)" point of Q30 in [Adopt AI and agent decisions](https://github.com/Jbmanllr/automan/issues/72), and it reads the legal framing of [Legal exposure of stealth and challenge-solving tooling](https://github.com/Jbmanllr/automan/issues/65) ([note][legal-note]). Like the rest of `rnd/`, this note is input for inspiration: it lists facts and options and adopts no term or decision. It is not legal advice.

Prices, limits and terms are as published on the date above. Each claim links to its source. `[unverified]` marks a claim that rests on secondary sources or that I could not check against the owner's page. `[I]` marks the researcher's inference.

## Summary

- **Every model provider in Automan's provider layer except plain OpenAI-compatible endpoints sells a server-side search tool, and all of them cost $10–14 per 1,000 searches.** Anthropic `web_search` is $10/1k ([docs][anth-search]); OpenAI `web_search` is $10/1k plus search content tokens ([pricing][oai-pricing]); Gemini 3 grounding is $14/1k queries after 5,000 free per month ([Gemini pricing][gem-pricing]); Vertex charges the same for Gemini 3 ([Vertex pricing][vtx-pricing]); Grounding with Bing on Azure is $14/1k transactions ([Bing pricing][bing-pricing]); Amazon Nova web grounding is "an additional cost" whose rate I did not find ([Nova docs][nova-grounding]). Availability varies by platform: Anthropic web search is **not available on Amazon Bedrock**, and Google Cloud gets only the basic version ([docs][anth-search]).
- **Their terms differ sharply on what you may do with the results outside the answer.** Gemini grounding forbids caching, analyzing or training on results and explicitly forbids "using Links to identify destination pages for crawling or scraping" ([Gemini API terms][gem-terms]). Grounding with Bing gives **no raw results** and forbids copying, storing or caching the output ([Microsoft docs][bing-doc]; [terms][bing-legal]). Anthropic and OpenAI return the URLs and require visible citations ([Anthropic][anth-search]; [OpenAI][oai-search]); I found no storage clause specific to search in either provider's published terms `[unverified]`. Search-then-scrape, journaling and replay are therefore permitted or forbidden depending on the vendor.
- **Google offers no general web search API to an open-source framework user.** The Custom Search JSON API is closed to new customers and ends on **January 1, 2027**; new Programmable Search engines can only search up to 50 listed sites ([Google developers][cse-overview]; [PSE blog, Jan. 20, 2026][pse-blog]). Full-web access is a partner-only product with an interest form ([PSE blog][pse-blog]; [SEJ, Sept. 11, 2026][sej-wss] `[unverified]`). What remains is Gemini or Vertex grounding, under the terms above.
- **Bing's raw API is gone.** Microsoft retired the Bing Search APIs on **August 11, 2025** and points to Grounding with Bing in Foundry Agent Service ([Microsoft lifecycle][bing-retire]). Spider still ships a Bing v7 provider that targets the retired endpoint ([source][spider-bing]).
- **Independent indexes with a developer API:** Brave (40+ billion pages, $5/1k, $5 free credit per month; storage needs a plan that grants it, and its terms forbid training on results) ([Brave][brave-api]; [Brave API terms, Sept. 1, 2026][brave-terms]); Exa (claims a 100-billion-document index; $7/1k for `auto` search, $10 free monthly credit) ([Exa][exa-home]; [pricing][exa-pricing]); Perplexity Search API ($5/1k; Perplexity describes an index of "hundreds of billions" of pages) ([pricing][pplx-pricing]; [launch coverage][pplx-vb] `[unverified]`); Mojeek (own index, £2–3 per thousand, storage allowed on the Business plan, AI use allowed on all plans) ([Mojeek][mojeek]); Kagi (own small-web index plus calls to other providers; legacy API $25/1k) ([sources][kagi-sources]; [API][kagi-legacy]). Google-result resellers (Serper, SerpApi) are cheap or offer legal insurance, but they inherit the *Google v. SerpApi* exposure.
- **Searching through a self-driven browser is technically possible but fragile and contract-breaching.** Google's robots.txt disallows `/search` for all user agents, and Google's terms forbid automated access that violates robots.txt ([robots.txt][g-robots]; [Google ToS, effective July 30, 2026][g-tos]). Google has required JavaScript since January 2025 and runs SearchGuard, which challenges unrecognized clients ([TechCrunch via SerpApi blog][serpapi-js] `[unverified]`; [*Reddit v. SerpApi* opinion][reddit-op]). Bing's robots.txt disallows `/search`, and DuckDuckGo's disallows `/html` and `/lite` ([Bing][bing-robots]; [DuckDuckGo][ddg-robots]). Under #65's map, a stop-on-challenge browser search in the open-source framework is **Low** for circumvention (F3d), but run in Automan Cloud with masking or IP rotation it matches the SerpApi facts (**Medium to High**, B-F1/B-F2).
- **Competitors mostly buy search.** Browserbase Search is powered by Exa ([Browserbase blog][bb-blog]); Spider's library takes Serper, Brave, Bing or Tavily keys ([source][spider-search]); Browser Use's built-in `search` action opens DuckDuckGo, Google or Bing **in the agent's own browser**, defaulting to DuckDuckGo "because less captchas" ([source][bu-views]); Crawl4AI ships a Google SERP crawler that uses its own browser ([source][c4a-google]); Firecrawl's self-hosted fallback chain is its closed fire-engine, then SearXNG, then scraping `html.duckduckgo.com` ([source][fc-search]).

## 1. Search tools built into model providers

### 1.1 Anthropic: `web_search` and `web_fetch` server tools

| Aspect | Web search | Web fetch |
| --- | --- | --- |
| Versions | `web_search_20250305` (basic), `web_search_20260209` (dynamic filtering), `web_search_20260318` (adds `response_inclusion`) ([docs][anth-search]) | `web_fetch_20250910` (basic) through `web_fetch_20260318` (dynamic filtering, `use_cache`, `response_inclusion`) ([docs][anth-fetch]) |
| Price | **$10 per 1,000 searches**, plus tokens; results count as input tokens on later turns; failed searches are not billed ([docs][anth-search]) | **No extra charge**, only tokens ([docs][anth-fetch]) |
| Platforms | Claude API, Claude Platform on AWS, Microsoft Foundry (basic only when hosted on Azure), Google Cloud (basic only). **Not on Amazon Bedrock** ([docs][anth-search]) | Claude API, Claude Platform on AWS, Foundry (basic on Azure). **Not on Bedrock or Google Cloud** ([docs][anth-fetch]) |
| Controls | `max_uses`, `allowed_domains` or `blocked_domains`, `user_location`; org admins can disable it or restrict domains in the Console ([docs][anth-search]) | `max_uses`, domain lists, `max_content_tokens`, optional citations; it can fetch only URLs already present in the conversation, refuses URLs that seem to carry credentials, and honours `robots.txt` (`url_not_allowed`) ([docs][anth-fetch]) |
| Result shape | `web_search_result` with `url`, `title`, `page_age` and an **`encrypted_content`** blob that must be sent back unchanged on later turns; citations carry `url`, `title`, `cited_text` (up to 150 characters) and an `encrypted_index` ([docs][anth-search]) | A `document` block with full text (or base64 PDF), `url`, `retrieved_at`; content may come from Anthropic's cache ([docs][anth-fetch]) |
| Rate limits | Org-level web search rate limit shown in the Console; `too_many_requests` comes back inside a 200 response; batches are throttled ([docs][anth-search]) | `too_many_requests` error code ([docs][anth-fetch]) |
| Display / reuse | "When displaying API outputs directly to end users, citations must be included to the original source" ([docs][anth-search]). I found no clause restricting storage or reuse of the URLs in the [Commercial Terms][anth-terms] (effective June 17, 2025) `[unverified]` | Same citation note; it does not render JavaScript ([docs][anth-fetch]) |

- **Backing index.** The docs do not name it. Anthropic added Brave Search to its sub-processor list in March 2025, and Simon Willison matched Claude's results to Brave's ([Willison, Mar. 21, 2025][willison-brave]). A July 2026 analysis says the list still names Brave, plus turbopuffer ([xponent21][xponent-anth]). I could not load Anthropic's sub-processor page `[unverified]`. Anthropic also runs `Claude-SearchBot`, which "navigates the web to improve search result quality" ([Anthropic crawler help][anth-bots]).
- **Snippets are opaque.** The page text that reaches the model is in `encrypted_content`, so a client gets URLs and titles, and cited text only where Claude cites. For Automan, that means the raw result list is usable (URLs) but the snippets are not [I].

### 1.2 OpenAI: `web_search` (Responses API) and search models

- **Tools.** `web_search` is the current tool; `web_search_preview` is legacy and lacks `filters`, `external_web_access` and `return_token_budget` ([guide][oai-search]).
- **Models.** Responses API with reasoning models. In Chat Completions, `gpt-5-search-api`; `gpt-4o-search-preview` and `gpt-4o-mini-search-preview` were deprecated and shut down on 2026-07-23 ([guide][oai-search]). Deep research models do not support `user_location` ([guide][oai-search]).
- **Price.** `web_search`: **$10 per 1,000 calls** plus search content tokens at model rates. `web_search_preview` on non-reasoning models: $25/1k, content tokens free. `gpt-5-search-api`: $1.25 input / $10 output per 1M tokens ([pricing][oai-pricing]).
- **Result shape.** A `web_search_call` item (`search`, `open_page`, `find_in_page` actions) and `url_citation` annotations (`url`, `title`, `start_index`, `end_index`). With `include: ["web_search_call.action.sources"]` it returns every URL consulted, usually more than the cited ones ([guide][oai-search]).
- **Controls.** Up to 100 allowed or blocked domains, `user_location`, `search_context_size`, `external_web_access: false` for cache-only mode ([guide][oai-search]).
- **Rate limits.** "Same as tiered rate limits for underlying model" ([guide][oai-search]).
- **Display.** "Inline citations must be made clearly visible and clickable in your user interface" ([guide][oai-search]).
- **Backing index.** Not named in the docs. OpenAI runs `OAI-SearchBot` "to surface websites in search results in ChatGPT's search features" ([OpenAI bots][oai-bots]). Third-party sources link it to Bing `[unverified]`. OpenAI's Service Terms page returned 403, so I could not check for search-specific clauses `[unverified]`.

### 1.3 Google: Grounding with Google Search (Gemini API and Vertex AI)

- **How it works.** The `google_search` tool; the model issues one or more queries and returns text with `url_citation` annotations plus a `search_suggestions` HTML snippet to render ([Gemini docs][gem-search]).
- **Price (Gemini API).** Gemini 3 and newer: **5,000 free search queries per month** shared across models, then **$14 per 1,000 queries**, billed per query (one prompt can produce several). Gemini 2.5 Pro: 1,500 grounded prompts per day free, then $35 per 1,000 prompts ([pricing][gem-pricing]).
- **Price (Vertex AI).** Gemini 3: the same 5,000 free per month and $14/1k queries, billing from January 5, 2026; Gemini 2.5: $35/1k prompts; **Web Grounding for Enterprise**: $45/1k prompts on the older table; limit 1 million prompts per day ([Vertex pricing][vtx-pricing]). On the Gemini 3 table, customers "may decide not to display Search Suggestions … at standard pricing"; on older models, skipping them needs a waiver at alternate pricing ([Vertex pricing][vtx-pricing]).
- **Display requirements (Vertex).** Show the Search Suggestion "exactly as provided", keep it visible whenever the grounded answer is shown, no screens between the tap and Google's results page, and no other results next to the chip ([Vertex docs][vtx-grounding]).
- **Use restrictions (Gemini API terms, last updated 2026-04-28)** ([terms][gem-terms]):
  - Only in "an application that is owned and operated by you", shown "to the end user who submitted the prompt".
  - "You will not … cache, frame, syndicate, resell, analyze, train on, or otherwise learn from Grounded Results or Search Suggestions."
  - It is "a violation of these terms to use Grounding with Google Search to extract or collect one or more of these components for another purpose (for example, using programmatic or automated means to collect Links, using Links to build an index, or using Links to identify destination pages for crawling or scraping)".
  - Text of grounded results may be stored up to two years, only for display evaluation, the end user's chat history, or resubmission. No click tracking per link, no modification or interleaving.
  - Google keeps prompts and output for 30 days for debugging.
- **Enterprise variant.** Web Grounding for Enterprise uses a curated index (fast-changing content every 6 hours, full index every 24 hours), does not log customer data and supports VPC Service Controls; responses with Search suggestions are still under the grounding terms ([Vertex docs][vtx-wge]).
- **Publisher opt-out.** Grounding "does not use web pages for grounding that have disallowed Google-Extended" ([Vertex docs][vtx-grounding]).
- **Other engines on Vertex.** The Vertex grounding docs list "Grounding with Parallel web search" and "Grounding with Exa web search" next to Google ([Vertex docs navigation][vtx-wge]). I did not read those pages.

### 1.4 Amazon Bedrock: Nova web grounding

- A `nova_grounding` system tool in the Converse API; the model searches and returns text with `citationsContent` (URL and domain) ([Nova 2 docs][nova-grounding]).
- "You must retain and display these citations and links in the output you provide to your end users." It is "an additional cost"; I did not find the rate on the Bedrock pricing page `[unverified]` ([Nova 2 docs][nova-grounding]).
- Launched in US regions with Nova Premier in October 2025 ([AWS blog][nova-blog]); the current docs show `nova-2-lite` ([Nova 2 docs][nova-grounding]).
- Claude models on Bedrock have no server-side web search ([Anthropic docs][anth-search]).

### 1.5 Microsoft Azure: Grounding with Bing and the `web_search` tool

- **Bing Search APIs:** retired August 11, 2025, "decommissioned completely", no new signups; Microsoft points to Grounding with Bing ([lifecycle notice][bing-retire]).
- **Grounding with Bing Search / Bing Custom Search:** $14 per 1,000 transactions, 150 TPS, 1 million transactions per day ([pricing][bing-pricing]).
- "Developers and end users don't have access to raw content returned from Grounding with Bing Search." The output carries citation links and a link to the Bing query, which must be "retained and displayed in the exact form provided by Microsoft". Query data leaves the Azure compliance boundary ([Foundry docs][bing-doc]).
- **Terms** ([Grounding with Bing terms][bing-legal]): no training "any AI Models"; no "copy, store, cache, archive, or create a database of Output"; output may be used only in "internet search experiences"; it may not be used "for websites where you are restricted by the website from using such Output, including … where your crawler has been blocked via robots.txt".
- **Azure OpenAI `web_search`** in the Responses API runs on Grounding with Bing Search or Bing Custom Search and falls under those terms ([Microsoft Learn][az-websearch]).

### 1.6 Comparison

| Provider tool | Backing index | Price | Raw URLs returned | Reuse outside the answer |
| --- | --- | --- | --- | --- |
| Anthropic `web_search` | Not named; Brave per the sub-processor list `[unverified]` | $10/1k searches | Yes (snippets encrypted) | Citations required on display; no storage clause found `[unverified]` |
| Anthropic `web_fetch` | Live fetch, with cache | Tokens only | Yes (full text) | Same |
| OpenAI `web_search` | Not named; OAI-SearchBot index | $10/1k + content tokens | Yes (`sources`) | Clickable citations required; Service Terms not read `[unverified]` |
| Gemini / Vertex Google grounding | Google Search | $14/1k queries (Gemini 3), 5k/month free | Yes (links) | **No caching, analysis, training, indexing, or crawl targeting**; suggestions must be shown |
| Vertex Web Grounding for Enterprise | Curated Google index | $45/1k prompts (older table) | Yes | Same grounding terms |
| Bedrock Nova grounding | Not named | Extra cost, rate not found | Citations | Must retain and display citations |
| Azure Grounding with Bing / `web_search` | Bing | $14/1k transactions | Citations only, **no raw content** | **No storing or caching**; search-experience use only; no training |

## 2. Google by API: what an open-source user can get

| Product | Status on 2026-10-10 | What it gives |
| --- | --- | --- |
| Custom Search JSON API | **Closed to new customers**; existing customers "have until January 1, 2027"; 100 free queries per day, then $5/1k up to 10k/day for those customers ([overview][cse-overview]) | Google results for a Programmable Search Engine |
| Programmable Search Engine "Search the entire web" | Since January 20, 2026, new engines must use "Sites to search" (50 domains at most); existing engines keep the whole-web option until January 1, 2027 ([PSE blog][pse-blog]) | Site search only, for new users |
| Google's full web search solution | Contact form only ([PSE blog][pse-blog]). A partner-only "Web Search Service API" was documented in September 2026; it needs a client ID tied to a partner agreement ([SEJ][sej-wss] `[unverified]`; the documentation URL it cites returned 404 on 2026-10-10) | Full web results, for partners |
| Vertex AI Search | Recommended by Google for "up to 50 domains" ([overview][cse-overview]) | Site or data-store search, not the open web |
| Grounding with Google Search | Available (section 1.3) | Google-backed answers with links, under strict terms |

[I] For an Automan user without a Google partnership, the only legitimate route to Google-quality web results is grounding, and its terms forbid most of what a crawl/extraction engine would do with the links (index, crawl, journal). SearXNG added a "google cse" engine in July 2026 ([commit history][sxng-commits]), which will lose its whole-web use on January 1, 2027.

## 3. Third-party search APIs

| Service | Index source | Price (published) | Free tier | Storage / AI terms |
| --- | --- | --- | --- | --- |
| **Brave Search API** | Own index, "40+ billion pages", "100+ million page updates every day" ([Brave][brave-api]) | Search $5/1k (50 QPS); Answers $4/1k + $5 per 1M tokens ([Brave][brave-api]) | $5 credit per month ([Brave][brave-api]) | Terms forbid storing or caching results "other than transient storage", and training or benchmarking AI on them; results must not be retained after termination ([terms, Sept. 1, 2026][brave-terms]). Storage needs "a plan that explicitly grants storage rights" ([Brave][brave-api]) |
| **Exa** | Claims its own crawl and a 100-billion-document index ([Exa][exa-home]) | `instant` $4/1k, `fast`/`auto` $7/1k (10 results), deep $12–15/1k, contents $1/1k pages ([pricing][exa-pricing]) | $10 per month ([pricing][exa-pricing]) | Posted Terms of Service forbid copying or redistributing information obtained through the services "except … as otherwise expressly permitted", and grant Exa a perpetual license to user input and output ([terms PDF][exa-terms]); I found no API-specific storage clause `[unverified]` |
| **Tavily** (Nebius agreed to acquire it, Feb. 10, 2026 ([Nebius][nebius-tavily])) | Aggregator, no own full index per third-party analysis ([Sacra][sacra-tavily] `[unverified]`) | 1 credit basic / 2 advanced search; $0.008 per credit pay-as-you-go; plans $30–$500/month ([credits][tavily-credits]) | 1,000 credits per month ([credits][tavily-credits]) | 100 RPM dev key, 1,000 RPM production ([rate limits][tavily-rl]). Tavily may use customer input and output to train AI ([terms, May 4, 2026][tavily-terms]) |
| **Serper** | Google results; collection method not disclosed ([Serper][serper]) | $1.00/1k (50k) down to $0.30/1k (12.5M); credits valid 6 months; 50–300 QPS ([Serper][serper]) | 2,500 queries ([Serper][serper]) | Not found |
| **SerpApi** | Scraped Google and other engines; defendant in *Google v. SerpApi* and *Reddit v. SerpApi* ([legal note][legal-note]) | $25 for 1,000/month up to cloud tiers; "U.S. Legal Shield" up to $2 million of coverage from the Production plan ([pricing][serpapi-pricing]) | 250 searches per month ([pricing][serpapi-pricing]) | Not checked |
| **Kagi Search API** | Own indexes (Teclis, TinyGem) plus "anonymized API calls to all major search result providers" ([sources][kagi-sources]) | Legacy v0: $25/1k; v1 price not found ([legacy][kagi-legacy]) | Kagi account needed | Not found |
| **You.com** | "Same real-time index" for all APIs; ownership not described ([pricing][you-pricing]) | Search $5/1k; contents $1/1k pages ([pricing][you-pricing]) | $100 credit; 100 queries/day keyless via MCP ([pricing][you-pricing]) | Not checked |
| **Perplexity Search API** | Perplexity's own index, "hundreds of billions" of pages, launched Sept. 25, 2025 ([VentureBeat][pplx-vb] `[unverified]`) | Search $5/1k ($1/1k fast); Sonar $5–14/1k request fee plus tokens ([pricing][pplx-pricing]) | Not found | Not checked |
| **Jina `s.jina.ai`** | Not disclosed `[unverified]` | Fixed tokens per request, from 10,000 ([Jina Reader][jina]) | 10M tokens per new key ([Jina Reader][jina]) | Not checked |
| **Mojeek** | Own independent index, "billions of web pages" ([Mojeek][mojeek]) | £2 (Startup) or £3 (Business) per thousand ([Mojeek][mojeek]) | Trial on request | Storage allowed on Business (optional on Enterprise); "you can use the API results for AI with all plans" ([Mojeek][mojeek]) |
| **Browserbase Search** | Exa ([Browserbase blog][bb-blog]) | 1,000 calls included, then $7/1k; 2 RPS per project ([pricing][bb-pricing]) | 1,000 calls | Not checked |
| **SearXNG (self-hosted)** | **No index.** It aggregates "other search engines" ([about][sxng-about]); its Google engine fetches result pages with a Nokia user agent to get an XML layout, sets a `CONSENT` cookie, and raises a CAPTCHA error on Google's "sorry" pages ([source][sxng-google]) | Free, AGPL-3.0 ([source][sxng-google]) | n/a | Inherits each upstream engine's terms (section 4) |

Notes:

- **Brave's storage rule matters for a journaling engine [I].** A journal entry or replay cache that keeps search results is "storage" beyond transient use unless the plan grants storage rights.
- **SearXNG is browser-style search by proxy.** Its maintainers track Google's countermeasures: Google's July 1, 2026 change broke the GSA iPhone user agent ([issue #6359][sxng-6359]), a Nokia user agent became the fix on August 20, 2026, and the network layer moved to `curl_cffi` (TLS impersonation) on September 3, 2026 ([commit history][sxng-commits]).

## 4. Searching through a browser

### Feasibility

- **Google.** Google started requiring JavaScript for Search in January 2025 "to better guard Search against bots and spam" ([TechCrunch, via SerpApi blog and Daring Fireball][serpapi-js] `[unverified]`). SearchGuard sends unrecognized clients "a challenge or CAPTCHA" and is a gate "which keeps out automated users" ([*Reddit v. SerpApi*, ECF 104, at 37][reddit-op], quoted in the [legal note][legal-note]). A full Chrome runs the JavaScript, but detection still applies. SearXNG's code shows how the HTML route degrades: redirect to `sorry.google.com`, a `/sorry/` stub, or an "enable JS" page ([source][sxng-google]; [#6359][sxng-6359]). EU visitors also see a consent interstitial; SearXNG pre-sets `CONSENT=YES+` ([source][sxng-google]).
- **Bing.** Not tested here. Browser Use offers it as an option ([source][bu-service]).
- **DuckDuckGo.** Browser Use defaults to DuckDuckGo "because less captchas" ([source][bu-views]). Firecrawl's fallback scrapes `html.duckduckgo.com` with rotating user agents and handles "anti-bot measures" by retrying up to three times ([source][fc-ddg]).
- I did not run live queries against these engines for this note. A feasibility test would itself be an automated query under the terms below [I].

### Terms of service and robots.txt

| Engine | robots.txt (fetched 2026-10-10) | Terms |
| --- | --- | --- |
| Google | `User-agent: *` / `Disallow: /search` ([robots.txt][g-robots]) | Forbids "using automated means to access content from any of our services in violation of the machine-readable instructions on our web pages (for example, robots.txt files that disallow crawling, training, or other activities)" ([Terms, effective July 30, 2026][g-tos]; French country version served) |
| Bing | `Disallow: /search` under `User-agent: *` ([robots.txt][bing-robots]) | Microsoft Services Agreement: "Don't circumvent any restrictions on access to, usage, or availability of the Services (e.g., … impermissible scraping)" ([MSA][msa]) |
| DuckDuckGo | `Disallow: /html`, `/lite`, and `/*?` (query strings off the root) ([robots.txt][ddg-robots]); `html.duckduckgo.com` allows all ([robots.txt][ddg-html-robots]) | Acceptable Use Policy: no access "without authorization", no reselling "any portion of the services", no interfering with "the integrity or performance of the services" ([AUP][ddg-aup]); terms last updated 01-07-2025 ([terms][ddg-terms]). No explicit scraping clause found |

### Legal exposure, mapped to #65's levels

The [legal note][legal-note] defines placements (**A** public open-source framework, **B** Automan Cloud, **C** private component), families (**F1** masking, **F2** proxy rotation, **F3a–d** challenge handling) and levels (Low / Medium / High). Its status on *Google v. SerpApi* (N.D. Cal., No. 4:25-cv-10826): the July 20, 2026 order dismissed Google's §1201 claims, without leave to amend for results with no copyrighted content; Google filed an amended complaint on August 10; the October 13 hearing on SerpApi's renewed motion was vacated on October 5; **pending as of 2026-10-10** ([legal note][legal-note]; [ECF 42][google-order]; [ECF 60][google-vacate]). I did not find a later docket entry (CourtListener refused unauthenticated access) `[unverified]`.

| Browser-search design | Mapping [I] |
| --- | --- |
| Opt-in "browser" search provider in the OSS framework, run by the user's own Chrome at human pace, **stops on any challenge** (no solving, no rotation) | Circumvention: **Low** (A-F3d). Contract: the user breaches Google's terms and robots.txt, a contract and CFAA matter outside #65's §1201 map; CFAA risk is weak for logged-off public pages (*hiQ II*), per the [legal note][legal-note] |
| Same, combined with fingerprint masking and rotate-on-block proxies | **Medium** for rotate-on-block (A-F2); *Reddit* held that IP shifting and fake user agents "avoid" and "bypass" SearchGuard ([legal note][legal-note]) |
| Automan Cloud runs Google SERP queries for customers with masking or proxy rotation | **Medium to High** (B-F1, B-F2): the SerpApi posture; Google alleged "billions" of acts, and statutory damages scale per act ([legal note][legal-note]) |
| Any of the above that **solves** Google's CAPTCHA | **High** (A-F3a, B-F3a) |
| Search APIs and provider grounding (sections 1–3) | No circumvention; the exposure is contractual (storage, display, training clauses) |

[I] The *Google v. SerpApi* copyright-nexus defense helps only where results carry no copyrighted work, and Google's amended complaint pleads licensing deals to cure the authorization element. That defense is pending and should not be relied on.

## 5. How competitors source search

| Tool | Search source | Evidence |
| --- | --- | --- |
| **Firecrawl** `/search` | Hosted: "our upstream search provider", not named; 2 credits per 10 results ([docs][fc-docs]). Self-hosted code: fire-engine (closed service) if configured, else SearXNG if `SEARXNG_ENDPOINT` is set, else scraping `html.duckduckgo.com` ([source][fc-search]) | Code on `main`, DDG file last changed 2026-08-11 |
| **Spider** | Library and `spider_agent`: a `SearchProvider` trait with Serper, Brave, Bing (v7 endpoint, retired) and Tavily ([trait][spider-search]; [Bing][spider-bing]). Cloud `/search` "Run a search query and automatically crawl the top results"; provider not named ([docs][spider-docs]) | Code on `main` |
| **Stagehand / Browserbase** | `browserbase.search()` is cloud-only, 1–25 results, 120 requests per minute ([Stagehand docs][sh-search]); powered by Exa ([Browserbase blog][bb-blog]); $7/1k after 1,000 included ([pricing][bb-pricing]) | Docs |
| **Browser Use** | Built-in `search` action navigates the agent's browser to DuckDuckGo (default), Google (`udm=14`) or Bing ([service][bu-service]; [views][bu-views]). An example shows a custom Serper tool ([example][bu-serper]) | Release 0.13.11, Oct. 7, 2026 |
| **Crawl4AI** | `GoogleSearchCrawler` loads `google.com/search` in its headless browser ([source][c4a-google]); `AsyncUrlSeeder` discovers URLs from sitemaps and the Common Crawl index, without a search engine ([source][c4a-seeder]) | Code on `main` |
| **Tavily-based agents** | LangChain, LlamaIndex and others ship Tavily integrations; the prior survey lists Tavily among Spider's providers ([survey][survey]) | Not re-checked |
| **OpenAI / Anthropic agent products** | Their own server tools (sections 1.1–1.2): the Claude Agent SDK uses server-side web search ([survey][survey]); OpenAI's Agents SDK has a hosted web search tool `[unverified]` | Docs |

Pattern [I]: open-source frameworks either let the agent drive a search page in its own browser (Browser Use, Crawl4AI, Firecrawl's last fallback) or take a key for a search API (Spider, Browser Use examples). Cloud products resell an API (Browserbase via Exa) or hide the provider (Firecrawl, Spider Cloud).

## 6. Options for Automan's web search operation (facts and trade-offs, not decisions)

Context: Q30 in [#72](https://github.com/Jbmanllr/automan/issues/72) says "web search through a provider (sources under research)". Automan has its own model-provider layer and a catalog of operations with journaling and replay.

### Option 1: bring-your-own search API, behind one provider interface

A search provider interface, like Spider's `SearchProvider` trait ([source][spider-search]), with adapters for Brave, Exa, Tavily, Perplexity, You.com, Kagi, Mojeek, Serper/SerpApi and a SearXNG URL.

- **For:** results are plain data the engine can feed into crawl or extract; the user holds the key and the contract; no circumvention by Automan; competitive with Spider and Browser Use examples.
- **Against:** each provider's storage terms differ (Brave transient only unless the plan allows it; Mojeek allows storage on Business; Tavily may train on inputs). Journal and replay of results need a per-provider retention flag [I]. Google-reseller adapters (Serper, SerpApi) carry the SerpApi litigation risk to the user, and that should be stated in docs [I].

### Option 2: provider-native search pass-through in the model layer

Expose Anthropic `web_search`/`web_fetch`, OpenAI `web_search`, Gemini/Vertex `google_search`, Nova `nova_grounding` and Azure `web_search` as options of AI operations.

- **For:** no extra key or vendor; quality of Google or Bing via grounding; citations come with the answer.
- **Against:** availability depends on the platform (Anthropic search is absent on Bedrock and limited on Google Cloud). Results are tied to the answer: Gemini forbids using links to crawl, index or cache; Bing returns no raw results and forbids storing. Anthropic snippets are encrypted. Display obligations (citations, Google search suggestions) fall on the user's UI. Cost is $10–14/1k on top of tokens.
- **Shape [I]:** it fits as an option of an AI operation ("answer with search"), not as a source for the search operation that feeds a crawl. Anthropic and OpenAI URLs are the exception if their terms allow it `[unverified]`.

### Option 3: browser search as a provider

Automan's own Chrome opens a results page (DuckDuckGo, Bing, Google) and extracts results.

- **For:** no key, no cost; matches Browser Use and Crawl4AI; works with Automan's existing browser primitives.
- **Against:** robots.txt disallows the result paths on Google and Bing, and Google's terms forbid automated access against robots.txt. Google requires JavaScript, runs SearchGuard and changes layouts often (SearXNG's fixes in July–September 2026). EU consent pages add friction. Legal levels per section 4: Low only with stop-on-challenge, no rotation, user-run, never in Cloud.

### Option 4: SearXNG endpoint as a provider

The user points Automan at a self-hosted SearXNG.

- **For:** one HTTP adapter covers many engines; Automan ships no scraper.
- **Against:** it is option 3 run by another program, with the same terms issues. AGPL-3.0 applies to SearXNG, not to a client that calls its HTTP API [I].

### Option 5: search-free discovery

Sitemaps, Common Crawl index, and a seed URL list, as Crawl4AI's URL seeder does ([source][c4a-seeder]).

- **For:** no search terms at all; good for "all pages of these sites".
- **Against:** it does not answer open questions.

[I] These options combine. A plausible layered shape is: options 1 and 5 as data sources for the search operation, option 2 for AI answers, and option 3 or 4 as an explicit opt-in with stop-on-challenge. Whether to ship option 3 at all is a decision for #72 and for counsel under #65.

## Method and limits

- **Sources:** vendor docs, pricing pages and terms fetched on 2026-10-10 (WebFetch and `curl` with HTML-to-text); GitHub source read through `gh api` for Firecrawl, Spider, Browser Use, Crawl4AI and SearXNG; the #65 legal note for case law, re-verified there on the same day.
- **Exa search was unavailable** (credits exhausted); I used other web search and fetch tools.
- **Not reachable:** Anthropic's sub-processor page, OpenAI's Service Terms (403), Perplexity's launch post (403), CourtListener's API (authentication), Google's Web Search Service API docs (404). Claims depending on them are marked.
- **Not read in full:** Vertex Parallel and Exa grounding pages, the Bing "LLM legal" variant linked from the grounding terms page, Kagi v1 API pricing, You.com and Perplexity API terms, and the Bedrock pricing line for Nova grounding.
- **No live tests** of SERP scraping were run.

[legal-note]: https://github.com/Jbmanllr/automan/blob/research/stealth-legal-exposure/rnd/research/2026-10-10-stealth-legal-exposure.md
[survey]: https://github.com/Jbmanllr/automan/blob/research/ai-browser-frameworks/rnd/research/2026-10-10-ai-browser-frameworks.md
[anth-search]: https://platform.claude.com/docs/en/agents-and-tools/tool-use/web-search-tool
[anth-fetch]: https://platform.claude.com/docs/en/agents-and-tools/tool-use/web-fetch-tool
[anth-terms]: https://www.anthropic.com/legal/commercial-terms
[anth-bots]: https://privacy.claude.com/en/articles/8896518-does-anthropic-crawl-data-from-the-web-and-how-can-site-owners-block-the-crawler
[willison-brave]: https://simonwillison.net/2025/Mar/21/anthropic-use-brave/
[xponent-anth]: https://xponent21.com/insights/claude-web-search-brave-turbopuffer/
[oai-search]: https://developers.openai.com/api/docs/guides/tools-web-search
[oai-pricing]: https://developers.openai.com/api/docs/pricing
[oai-bots]: https://developers.openai.com/api/docs/bots
[gem-search]: https://ai.google.dev/gemini-api/docs/google-search
[gem-pricing]: https://ai.google.dev/gemini-api/docs/pricing
[gem-terms]: https://ai.google.dev/gemini-api/terms#grounding-with-google-search
[vtx-grounding]: https://docs.cloud.google.com/vertex-ai/generative-ai/docs/grounding/grounding-with-google-search
[vtx-wge]: https://docs.cloud.google.com/vertex-ai/generative-ai/docs/grounding/web-grounding-enterprise
[vtx-pricing]: https://cloud.google.com/vertex-ai/generative-ai/pricing
[nova-grounding]: https://docs.aws.amazon.com/nova/latest/nova2-userguide/web-grounding.html
[nova-blog]: https://aws.amazon.com/blogs/aws/build-more-accurate-ai-applications-with-amazon-nova-web-grounding
[bing-retire]: https://learn.microsoft.com/en-us/lifecycle/announcements/bing-search-api-retirement
[bing-doc]: https://learn.microsoft.com/en-us/azure/ai-foundry/agents/how-to/tools/bing-grounding
[bing-pricing]: https://www.microsoft.com/en-us/bing/apis/grounding-pricing
[bing-legal]: https://www.microsoft.com/en-us/bing/apis/grounding-legal
[az-websearch]: https://learn.microsoft.com/en-us/azure/foundry/openai/how-to/web-search
[cse-overview]: https://developers.google.com/custom-search/v1/overview
[pse-blog]: https://programmablesearchengine.googleblog.com/2026/01/updates-to-our-web-search-products.html
[sej-wss]: https://www.searchenginejournal.com/google-documents-partner-only-api-for-full-web-search-results/589194/
[brave-api]: https://brave.com/search/api/
[brave-terms]: https://api-dashboard.search.brave.com/terms-of-service
[exa-home]: https://exa.ai/
[exa-pricing]: https://exa.ai/pricing
[exa-terms]: https://exa.ai/terms
[tavily-credits]: https://docs.tavily.com/documentation/api-credits
[tavily-rl]: https://docs.tavily.com/documentation/rate-limits
[tavily-terms]: https://tavily.com/terms
[nebius-tavily]: https://nebius.com/newsroom/nebius-announces-agreement-to-acquire-tavily-to-add-agentic-search-to-its-ai-cloud-platform
[sacra-tavily]: https://sacra.com/c/tavily/
[serper]: https://serper.dev/
[serpapi-pricing]: https://serpapi.com/pricing
[kagi-sources]: https://help.kagi.com/kagi/search-details/search-sources.html
[kagi-legacy]: https://help.kagi.com/kagi/api/search-legacy.html
[you-pricing]: https://you.com/pricing
[pplx-pricing]: https://docs.perplexity.ai/getting-started/pricing
[pplx-vb]: https://venturebeat.com/ai/perplexity-launches-massive-search-api-to-take-on-googles-dominance
[jina]: https://jina.ai/reader/
[mojeek]: https://www.mojeek.com/services/search/web-search-api/
[bb-blog]: https://www.browserbase.com/blog/search
[bb-pricing]: https://www.browserbase.com/pricing
[sh-search]: https://docs.stagehand.dev/v4/add-ons/search
[sxng-about]: https://docs.searxng.org/user/about.html
[sxng-google]: https://github.com/searxng/searxng/blob/master/searx/engines/google.py
[sxng-6359]: https://github.com/searxng/searxng/issues/6359
[sxng-commits]: https://github.com/searxng/searxng/commits/master/searx/engines/google.py
[g-robots]: https://www.google.com/robots.txt
[g-tos]: https://policies.google.com/terms
[bing-robots]: https://www.bing.com/robots.txt
[msa]: https://www.microsoft.com/en-us/servicesagreement
[ddg-robots]: https://duckduckgo.com/robots.txt
[ddg-html-robots]: https://html.duckduckgo.com/robots.txt
[ddg-aup]: https://duckduckgo.com/acceptable-use
[ddg-terms]: https://duckduckgo.com/terms
[serpapi-js]: https://serpapi.com/blog/google-now-requires-javascript/
[reddit-op]: https://storage.courtlistener.com/recap/gov.uscourts.nysd.651592/gov.uscourts.nysd.651592.104.0.pdf
[google-order]: https://storage.courtlistener.com/recap/gov.uscourts.cand.461513/gov.uscourts.cand.461513.42.0.pdf
[google-vacate]: https://storage.courtlistener.com/recap/gov.uscourts.cand.461513/gov.uscourts.cand.461513.60.0.pdf
[fc-docs]: https://docs.firecrawl.dev/features/search
[fc-search]: https://github.com/firecrawl/firecrawl/blob/main/apps/api/src/search/v2/index.ts
[fc-ddg]: https://github.com/firecrawl/firecrawl/blob/main/apps/api/src/search/v2/ddgsearch.ts
[spider-search]: https://github.com/spider-rs/spider/blob/main/spider/src/features/search.rs
[spider-bing]: https://github.com/spider-rs/spider/blob/main/spider/src/features/search_providers/bing.rs
[spider-docs]: https://spider.cloud/docs/api/search/
[bu-service]: https://github.com/browser-use/browser-use/blob/main/browser_use/tools/service.py
[bu-views]: https://github.com/browser-use/browser-use/blob/main/browser_use/tools/views.py
[bu-serper]: https://github.com/browser-use/browser-use/blob/main/examples/custom-functions/advanced_search.py
[c4a-google]: https://github.com/unclecode/crawl4ai/blob/main/crawl4ai/crawlers/google_search/crawler.py
[c4a-seeder]: https://github.com/unclecode/crawl4ai/blob/main/crawl4ai/async_url_seeder.py
