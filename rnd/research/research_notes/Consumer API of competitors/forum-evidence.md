# Forum evidence: what developers like and dislike in crawl-framework APIs

Collected 2026-10-14. Read-only research. Every quote below is verbatim from a page, API response or issue body I opened, and each is under 15 words. `[sic]` marks spelling kept from the original. Search queries contained no personal data.

## Method, coverage and limits

| Source | Access | What was used |
|---|---|---|
| Hacker News (Algolia API) | **Opened** | Story search (typo tolerance off) and comment search for crawlee, scrapy, scrapy-playwright, spider-rs, crawl4ai, firecrawl, scrapling, colly and katana. I dumped the full comment trees of 21 stories through `/api/v1/items/<id>`. |
| Stack Overflow (Stack Exchange API 2.3) | **Opened** | Top-voted questions per tag, `search/advanced` theme counts, `tags/info` totals and question bodies for the key questions. |
| GitHub issues (gh CLI) | **Opened** | The most-reacted issues per repository and theme searches across 14 repositories. Issue authors with association `NONE` are counted as independent users. Maintainers and vendor staff are flagged. |
| Blogs | **Opened** for the posts cited | Fetched with curl and checked with grep. Thunderbit posts come from a competing product, so they are flagged `author-of-competitor`. |
| **Reddit** | **Blocked** | `old.reddit.com/*.json` redirects to a login wall (HTTP 302 to `/login`), `www.reddit.com` returns 403 and the pullpush mirror returns 429. WebSearch refuses `reddit.com` as a domain, and unrestricted searches returned no Reddit URLs. **No Reddit evidence is included.** I did not log in to get past the wall. |
| Lobsters | **Blocked** | An Anubis bot-check page is served. I did not try to get past it. |
| Apify Discord mirror (discord.apify.com) | **Blocked** | HTTP 429. One search snippet is recorded below as unverified. |
| DZone and Towards Data Science (Scrapy testing and notebook posts) | **Blocked** | HTTP 403. Not quoted. |

**Independence labels**
- `independent`: an ordinary user, with no stake in the framework.
- `vendor`: the framework's company, its staff or its maintainers. For Crawlee this covers the Apify staff seen in these threads: mnmkng, B4nan, metalwarrior665, jancurn, sauain, mvolfik, janbuchar, honzajavorek, vdusek and Mantisus. For Scrapy it covers kmike and elacuesta, and for spider-rs it covers j-mendez.
- `author-of-competitor`: someone who builds a competing tool, such as Thunderbit, flyscrape, geziyor or Transistor.

**Stack Overflow volume (tag totals, 2026-10-14):** scrapy 17,825; web-crawler 9,616; scrapy-splash 581; apify 207; go-colly 69; scrapy-playwright 28; crawlee 22; firecrawl 5. There is no crawl4ai, scrapling or spider-rs tag. The newer frameworks have almost no Stack Overflow footprint, so their pain shows up mostly in GitHub issues.

**HN anchor threads**

| Story | Points | Date | Link |
|---|---|---|---|
| Show HN: Crawlee (Node.js) | 282 | 2022-08-23 | https://news.ycombinator.com/item?id=32561127 |
| Show HN: Crawlee for Python | 254 | 2024-07-09 | https://news.ycombinator.com/item?id=40913736 |
| Python 3 comes to Scrapy | 249 | 2016-02-04 | https://news.ycombinator.com/item?id=11035946 |
| How to Crawl the Web with Scrapy | 189 | 2021-09-13 | https://news.ycombinator.com/item?id=28514998 |
| Colly: Fast and Elegant Scraping Framework for Golang | 120 | 2017-10-05 | https://news.ycombinator.com/item?id=15408784 |
| Colly – Scraping Framework for Golang | 116 | 2017-12-18 | https://news.ycombinator.com/item?id=15953629 |
| Katana: A crawling and spidering framework | 99 | 2022-11-08 | https://news.ycombinator.com/item?id=33523546 |
| Show HN: Apify SDK | 78 | 2018-09-25 | https://news.ycombinator.com/item?id=18064493 |
| Show HN: Firecrawl-Simple (self-host fork) | 35 | 2024-11-05 | https://news.ycombinator.com/item?id=42056056 |
| Show HN: API to turn entire websites into Markdown (Firecrawl) | 23 | 2024-04-16 | https://news.ycombinator.com/item?id=40055103 |
| Show HN: Draco (Firecrawl alternative in Rust) | 15 | 2026-08-02 | https://news.ycombinator.com/item?id=49148163 |
| Web Crawler and Scraper for AI (spider.cloud) | 8 | 2024-12-19 | https://news.ycombinator.com/item?id=42464984 |
| Crawl4AI is an open-source, LLM-friendly web crawler | 7 | 2025-04-29 | https://news.ycombinator.com/item?id=43827993 |
| Scrapling: Fast, Adaptive Web Scraping for Python | 4 | 2024-10-13 | https://news.ycombinator.com/item?id=41832425 |

No HN story about spider-rs or scrapy-poet scored above 2 points.

---

## 1. Crawlee (JS/TS) and its predecessor, the Apify SDK

### Likes

| # | Theme | Quote | Source, date | Independence |
|---|---|---|---|---|
| CJ1 | switching HTTP to browser (unified API) | "I'm especially excited about the unified API for browser and HTML scraping" | HN [32561840](https://news.ycombinator.com/item?id=32561840), 2022-08-23 | independent. The same comment calls the Scrapy equivalent a hack: see SP3. |
| CJ2 | switching HTTP to browser | "a single api for both traditional and headless scraping" | HN [32561621](https://news.ycombinator.com/item?id=32561621), 2022-08-23 | independent |
| CJ3 | extensibility, readable code, production use | "It's extremely versatile and very pleasant to build on." The poster reports "hundreds of millions of requests" per month since v0.20. | HN [32562550](https://news.ycombinator.com/item?id=32562550), 2022-08-23 | independent |
| CJ4 | API ergonomics compared with Python | "I found the api a lot better than any python scraping api till date." Also "IT JUST WORKS!" | HN [40916771](https://news.ycombinator.com/item?id=40916771), 2024-07-09 | independent |
| CJ5 | reliability | "your library has worked better than all the others I've tried" | HN [40915166](https://news.ycombinator.com/item?id=40915166), 2024-07-09 | independent |
| CJ6 | time to result | "it's been faster for me to get what I've needed done" | HN [43957806](https://news.ycombinator.com/item?id=43957806), 2025-05-11 | independent |
| CJ7 | ecosystem position | "Crawlee framework is cool, while not strictly necessary" | HN [38232032](https://news.ycombinator.com/item?id=38232032), 2023-11-11 | author-of-competitor (wrote the geziyor Go framework) |
| CJ8 | docs (Apify SDK era) | "this looks solid, with a really extensive documentation" | HN [18064802](https://news.ycombinator.com/item?id=18064802), 2018-09-25 | independent |
| CJ9 | resume and persistence | Default local storage is "convenient for resuming" (Crawlee 3.17.0). | [Thunderbit review](https://thunderbit.com/blog/crawlee), updated 2026-08-17 | author-of-competitor |
| CJ10 | design rationale (vendor) | "It's not uncommon to switch from HTTP to headless back to HTTP" | HN [32561720](https://news.ycombinator.com/item?id=32561720), 2022-08-23 | vendor |

### Pain points

| # | Theme | Quote or fact | Source, date, version | Independence |
|---|---|---|---|---|
| CJ11 | storage purge on start, second run | "The second time the code is run it does not work." The poster found that `CRAWLEE_PURGE_ON_START` had no effect. | [apify/crawlee#1602](https://github.com/apify/crawlee/issues/1602), 2022-10-17 | independent |
| CJ12 | storage purge on start, second run | "my crawler runs again but shows that all routes have already finished". The poster could not get `purgeDefaultStorages()` to work. 3 votes, 2,363 views. | SO [74709844](https://stackoverflow.com/q/74709844), 2022-12-06 | independent |
| CJ13 | resume and persistence | After killing the process and restarting with `purgeOnStart: false`: "nothing happen" [sic]. | [apify/crawlee#3315](https://github.com/apify/crawlee/issues/3315), 2025-12-22, crawlee 3.15.3 | independent |
| CJ14 | storage configuration | "it outputs to `./storage` even after I update my storage directory in code" | [apify/crawlee#2196](https://github.com/apify/crawlee/issues/2196), 2023-11-25 | independent |
| CJ15 | local file storage at scale | "I get the no such file error every time." (`ENOENT … request_queues/default/*.json` with 64,000 pages) | SO [76782655](https://stackoverflow.com/q/76782655), 2023-07-27 | independent |
| CJ16 | storage purge docs (vendor admits) | "People are still asking about the storages, purging, resuming etc." (6 reactions) | [apify/crawlee#1782](https://github.com/apify/crawlee/issues/1782), 2023-02-09 | vendor |
| CJ17 | handler context extensibility | "I now need to extend the enqueLinks function" [sic] (passed into `requestHandler`) | SO [73477231](https://stackoverflow.com/q/73477231), 2022-08-24, crawlee 3.0.4 | independent |
| CJ18 | dedup and handler semantics | "It's Crawlee who's invoking handlers multiple times with the same request." | SO [77358550](https://stackoverflow.com/q/77358550), 2023-10-25 | independent |
| CJ19 | configuration ignored | "I still see the default timeout" (after setting `requestQueue.timeoutSecs`) | SO [74157958](https://stackoverflow.com/q/74157958), 2022-10-21 | independent |
| CJ20 | concurrency and locking | `enqueueLinks` inside `Promise.all` crashes with "Lock file is already being held" (4 reactions, open). | [apify/crawlee#2322](https://github.com/apify/crawlee/issues/2322), 2024-02-07 | vendor-reported |
| CJ21 | browser install | Title: "Playwright is unable to launch chrome on Alpine Docker" (5 votes) | SO [76225651](https://stackoverflow.com/q/76225651), 2023-05-11 | independent |
| CJ22 | browser install | "without it, `PlaywrightCrawler` failed to launch" (Chromium is not installed by npm). | [Thunderbit review](https://thunderbit.com/blog/crawlee), 2026-08-17, 3.17.0 | author-of-competitor |
| CJ23 | runtime support | "Deno doesn't work with crawlee either unfortunately" | HN [45214372](https://news.ycombinator.com/item?id=45214372), 2025-09-11 | independent |
| CJ24 | runtime support | "next Playwright refused working with Bun (via Crawlee)" | HN [45211391](https://news.ycombinator.com/item?id=45211391), 2025-09-11 | independent |
| CJ25 | runtime and bundling | The issues "Support Deno runtime" (10 reactions), "Support bundling" (10, still open) and "Add support for Bun runtime" (7). | [#1913](https://github.com/apify/crawlee/issues/1913), [#2208](https://github.com/apify/crawlee/issues/2208), [#2046](https://github.com/apify/crawlee/issues/2046), 2023 | independent (titles and reaction counts only) |
| CJ26 | remote browser | "Add a `connectOptions` similar to the `launchOptions`" (24 reactions, the most-reacted Crawlee issue) | [apify/crawlee#1822](https://github.com/apify/crawlee/issues/1822), 2023-03-09 | independent |
| CJ27 | switching HTTP to browser (the promise versus reality) | "this was not a one-line class-only swap". Extraction code "may need an adapter or rewrite". | [Thunderbit review](https://thunderbit.com/blog/crawlee), 2026-08-17, 3.17.0 | author-of-competitor |
| CJ28 | surprising storage default | "mildly annoying the first time you find a storage/ folder" | same review | author-of-competitor |
| CJ29 | pluggable storage | "I didn't see any obvious way of decoupling" (in the context of a new storage engine) | HN [32585288](https://news.ycombinator.com/item?id=32585288), 2022-08-24 | independent |
| CJ30 | docs | "I think the documentation (both clawlee & Apify) need some work." [sic] | HN [32562035](https://news.ycombinator.com/item?id=32562035), 2022-08-23 | independent |
| CJ31 | patch-release breakage | Title: "TypeError: requests is not iterable after upgrading from 3.13.8 to 3.13.9" | [apify/crawlee#3063](https://github.com/apify/crawlee/issues/3063), 2025-07-18 | independent |
| CJ32 | process lifecycle (Apify SDK) | "The final action of main() is to exit the node process" | SO [58543882](https://stackoverflow.com/q/58543882), 2019-10-24 | independent |
| CJ33 | handler context and page events (Apify SDK) | Title: "page.on('response') is not accessible in handlePageFunction" | SO [64355710](https://stackoverflow.com/q/64355710), 2020-10-14 | independent |
| CJ34 | docs: sessions | Title: "Improve Session Management guide." (21 reactions) | [apify/crawlee#796](https://github.com/apify/crawlee/issues/796), 2020-10-13 | title only |

---

## 2. Crawlee for Python

### Likes

| # | Theme | Quote | Source, date | Independence |
|---|---|---|---|---|
| CP1 | migration from Selenium | "I already can tell it works flawlessly" | HN [40937540](https://news.ycombinator.com/item?id=40937540), 2024-07-11 | independent |
| CP2 | minimal setup, asyncio and type hints (vendor pitch) | "Set up your scraper with fewer than 10 lines of code." The pitch also claims full type-hint coverage and plain asyncio rather than Twisted. | HN [40914659](https://news.ycombinator.com/item?id=40914659), 2024-07-09 | vendor |

### Pain points

| # | Theme | Quote | Source, date, version | Independence |
|---|---|---|---|---|
| CP3 | docs and undocumented config | "your cutesy coding style makes that a non-starter". The poster cites the undocumented `tiered_proxy_urls: list[list[str]]`. | HN [40918032](https://news.ycombinator.com/item?id=40918032), 2024-07-09 | independent |
| CP4 | docs | "I literally had to go through the entire codebase" | HN [40919056](https://news.ycombinator.com/item?id=40919056), 2024-07-09 | independent |
| CP5 | settings and middleware versus opinionated defaults | Pain for "anyone who isn't on the happy path of the way that crawlee was designed" | HN [40917985](https://news.ycombinator.com/item?id=40917985), 2024-07-09 | independent |
| CP6 | middleware flexibility | "you need the flexibility middleware gives you" | HN [40922731](https://news.ycombinator.com/item?id=40922731), 2024-07-10 | independent |
| CP7 | missing extension points | Asks how to evict blocked proxies and run an HTTP cache that "enables recrawls to fix logic bugs" | HN [40928731](https://news.ycombinator.com/item?id=40928731), 2024-07-10 | independent |
| CP8 | examples | "it didn't really do much besides apparently grepping for links" | HN [40925560](https://news.ycombinator.com/item?id=40925560), 2024-07-10 | independent |
| CP9 | examples | "bringing more code snippets from the test cases to documentation as examples" | HN [40919331](https://news.ycombinator.com/item?id=40919331), 2024-07-09 | independent |
| CP10 | JS-to-Python port parity | "That looks like Javascript?" (about the Python homepage example) | HN [40915767](https://news.ycombinator.com/item?id=40915767), 2024-07-09 | independent |
| CP11 | JS-to-Python port parity (vendor admits) | Adaptive crawling is something "the JS version of Crawlee has which unfortunately isn't in Python yet". | HN [40919198](https://news.ycombinator.com/item?id=40919198), 2024-07-09 | vendor |
| CP12 | storage purge and cache | "I don't want crawlee to cache anything". With `persist_storage=False` and `purge_on_start=True`, the poster still got concatenated results. | [apify/crawlee-python#369](https://github.com/apify/crawlee-python/issues/369), 2024-07-27 | independent |
| CP13 | storage purge (metadata) | Reports that `purge_on_start=True` leaves the queue `__metadata__.json` behind, so the next run shows "Crawled 0/4 pages". | Apify Discord mirror [m/1456667356334919790](https://discord.apify.com/m/1456667356334919790), about 2026-09 | **unverified (snippet only)**; the page returned HTTP 429 |
| CP14 | composing techniques (login, then browser, then HTTP) | "figuring out how to connect all the pieces for complicated use cases" | [apify/crawlee-python#862](https://github.com/apify/crawlee-python/issues/862), 2025-01-05 | independent |
| CP15 | passing data between handlers | "I struggle to see how I pass the information down" (20 comments) | [apify/crawlee-python#524](https://github.com/apify/crawlee-python/issues/524), 2024-09-15 | vendor employee writing as a user |
| CP16 | handler context data model | Title: "The user_data doesn't belong exclusively to the user". Crawlee stores `label` in `user_data` itself. | [apify/crawlee-python#1261](https://github.com/apify/crawlee-python/issues/1261), 2025-06-20 | vendor employee |
| CP17 | conceptual model and architecture docs | "it's all just magic" (asks for a Scrapy-style architecture diagram) | [apify/crawlee-python#603](https://github.com/apify/crawlee-python/issues/603), 2024-10-19 | vendor employee |
| CP18 | dependency breakage | Title: "Updating to Pydantic >3.11.9 causes TypeError on import" (3 reactions) | [apify/crawlee-python#1464](https://github.com/apify/crawlee-python/issues/1464), 2025-10-07 | title only |

---

## 3. Scrapy

### Likes

| # | Theme | Quote | Source, date | Independence |
|---|---|---|---|---|
| S1 | testability and well-defined callbacks | "very well structured, testable, and has solved every problem" | HN [32570837](https://news.ycombinator.com/item?id=32570837), 2022-08-23 | independent |
| S2 | selector API (Parsel) | "Parsel (the selector library powering Scrapy) is A++". The poster also says Scrapy "makes testing spider methods super easily" [sic]. | HN [32585288](https://news.ycombinator.com/item?id=32585288), 2022-08-24 | independent |
| S3 | middleware ecosystem and batteries included | The poster likes "not having to worry about storage backends, request throttling" and the many open-source middlewares. | HN [28515392](https://news.ycombinator.com/item?id=28515392), 2021-09-13 | independent |
| S4 | general | "I love scrapy! It’s a wonderful tool." | HN [28515747](https://news.ycombinator.com/item?id=28515747), 2021-09-13 | independent |
| S5 | fine-grained control | "the value of scrapy is you get more fine grained control." | HN [39442882](https://news.ycombinator.com/item?id=39442882), 2024-02-20 | independent |
| S6 | ecosystem | "I don't even consider alternatives at this point." | HN [15954482](https://news.ycombinator.com/item?id=15954482), 2017-12-18 | independent |
| S7 | HTTP cache middleware for replay | Scrapy's cache "enables recrawls to fix logic bugs" and better tests. | HN [40928731](https://news.ycombinator.com/item?id=40928731), 2024-07-10 | independent |

### Pain points

| # | Theme | Quote or fact | Source, date, version | Independence |
|---|---|---|---|---|
| S8 | Twisted reactor: running from a script | "I can't figure out where I should put my spider code" (88 votes, 90k views) | SO [13437402](https://stackoverflow.com/q/13437402), 2012-11-18 | independent |
| S9 | Twisted reactor: ReactorNotRestartable | "For the first time it works, then I get error." (41 votes) | SO [39946632](https://stackoverflow.com/q/39946632), 2016-10-09 | independent |
| S10 | Twisted reactor (aggregate) | A `q=ReactorNotRestartable` search returns **54 questions with 228 votes in total**. A title search for "script" in the scrapy tag returns **more than 100 questions, and the top 100 hold 555 votes**. A title search for "celery" returns **41 questions with 164 votes**. | SO `search/advanced`, run 2026-10-14 | independent (aggregate) |
| S11 | Twisted reactor: running from a script | Title: "Issue with running scrapy spider from script." (28 reactions, 42 comments) | [scrapy/scrapy#2473](https://github.com/scrapy/scrapy/issues/2473), 2017-01-02 | independent |
| S12 | Twisted reactor | "A limitation of its core component, the reactor, is that it cannot be restarted." | [Tryolabs blog](https://tryolabs.com/blog/2011/09/27/calling-scrapy-python-script), 2011-09-27, Scrapy 0.13 | independent (consultancy) |
| S13 | embedding in an app | "the documentation could be a bit better about integrating scrapy with other Python projects" | HN [28515747](https://news.ycombinator.com/item?id=28515747), 2021-09-13 | independent |
| S14 | Twisted dependency weight | "Twisted is great library but it is a huge dependency to maintain." | HN [11036136](https://news.ycombinator.com/item?id=11036136), 2016-02-04 | independent |
| S15 | Twisted documentation | "Twisted was very powerful, but the documentation is a mess" (the poster then moved to Go and Colly) | HN [37233593](https://news.ycombinator.com/item?id=37233593), 2023-08-23 | independent |
| S16 | Twisted version breakage | "I had to fix to the previous version, Twisted==22.10.0" (20 reactions) | [scrapy/scrapy#6024](https://github.com/scrapy/scrapy/issues/6024), 2023-08-29, Scrapy ≥2.9 | independent |
| S17 | reactor setting placement | Title: "TWISTED_REACTOR setting not honored from Spider.custom_settings" (7 reactions) | [scrapy/scrapy#4485](https://github.com/scrapy/scrapy/issues/4485), 2020-04-12 | vendor (maintainer) |
| S18 | codebase size and Twisted | "Scrapy's codebase is so big and also running Twisted" | HN [19441694](https://news.ycombinator.com/item?id=19441694), 2019-03-20 | author-of-competitor (Transistor) |
| S19 | passing data between callbacks | "understanding request.meta is a common struggle for beginners" (18 reactions; led to `cb_kwargs`) | [scrapy/scrapy#1138](https://github.com/scrapy/scrapy/issues/1138), 2015-04-06 | vendor (maintainer) |
| S20 | passing data between callbacks (aggregate) | SO [13910357](https://stackoverflow.com/q/13910357) has 56 votes, [41414528](https://stackoverflow.com/q/41414528) 39, [32252201](https://stackoverflow.com/q/32252201) 38 and [9334522](https://stackoverflow.com/q/9334522) 35. That is **4 questions and 168 votes** asking how to pass data or items between requests. | SO, 2012–2017 | independent (aggregate) |
| S21 | settings and middleware complexity | Title: "What is the difference between Scrapy's spider middleware and downloader middleware?" (24 votes). Title searches return 100+ "settings" questions (the top 100 hold 257 votes) and 100+ "pipeline" questions (496 votes). | SO [17872753](https://stackoverflow.com/q/17872753), 2013 | independent (aggregate) |
| S22 | framework rigidity | "like any battery included framework it has downsides in terms of flexibility" | HN [28515392](https://news.ycombinator.com/item?id=28515392), 2021-09-13 | independent |
| S23 | scale and distribution | "quite surprisingly, it was hard to design a distributed web crawler" | HN [28516132](https://news.ycombinator.com/item?id=28516132), 2021-09-13 | independent (runs a news API, which is not a scraping framework) |
| S24 | outgrowing the framework | "I am using Scrapy basically to only download the raw bytes". The poster also says concurrency parameters "could be explained a bit better". | HN [28531713](https://news.ycombinator.com/item?id=28531713), 2021-09-14 | independent |
| S25 | control flow driven by item results | "I couldn't see any other way of making calculations based on items returned" | HN [11041095](https://news.ycombinator.com/item?id=11041095), 2016-02-05 | independent |
| S26 | JS rendering (aggregate) | "Can scrapy be used to scrape dynamic content…AJAX?" has 168 votes and "selenium with scrapy for dynamic page" has 101. A title search for "dynamic" returns 100+ questions (467 votes). scrapy-splash has 581 tagged questions. | SO [8550114](https://stackoverflow.com/q/8550114), [17975471](https://stackoverflow.com/q/17975471) | independent (aggregate) |
| S27 | anti-bot | "You simply can't convince Cloudflare you're a human with Scrapy alone." | HN [28524016](https://news.ycombinator.com/item?id=28524016), 2021-09-14 | author-of-competitor (Apify) |
| S28 | unsafe defaults and politeness | "Scrapy needs to have sane defaults that do no harm". After a 429, "Scrapy did not slow down" (17 reactions, open). | [scrapy/scrapy#6597](https://github.com/scrapy/scrapy/issues/6597), 2025-01-01, Scrapy 2.11.2 | independent (site owner) |
| S29 | unsafe defaults | "it opened like a thousand simultaneous HTTP connections" (20 reactions) | [scrapy/scrapy#6755](https://github.com/scrapy/scrapy/issues/6755), 2025-04-22 | independent (site owner) |
| S30 | type hints | "Intellisense  enabled IDES like PyCharm need Type hints" [sic] (15 reactions) | [scrapy/scrapy#4041](https://github.com/scrapy/scrapy/issues/4041), 2019-09-27 | independent |
| S31 | performance regression | Title: "Scrapy 2.13 slower than 2.12 about 2–3×" (13 reactions) | [scrapy/scrapy#7158](https://github.com/scrapy/scrapy/issues/7158), 2025-11-20 | independent |
| S32 | per-request control | "Sometimes I feel like scrapy is missing per request delays." (14 reactions, open since 2014) | [scrapy/scrapy#802](https://github.com/scrapy/scrapy/issues/802), 2014-07-14 | contributor |
| S33 | cookies API | Title: "Expose cookiejars" (25 reactions, open) | [scrapy/scrapy#1878](https://github.com/scrapy/scrapy/issues/1878), 2016-03-24 | title only |

## 3a. scrapy-playwright

| # | Like or pain | Theme | Quote or fact | Source, date | Independence |
|---|---|---|---|---|---|
| SP1 | like | switching HTTP to browser | "scrapy-playwright is good, and Playwright is awesome." | HN [31390994](https://news.ycombinator.com/item?id=31390994), 2022-05-15 | independent |
| SP2 | like | browser integration | "Scrapy which I think has decent Playwright integration" | HN [31209642](https://news.ycombinator.com/item?id=31209642), 2022-04-29 | independent |
| SP3 | pain | switching HTTP to browser | A unified API was "something I've had to hack on top of Scrapy in the past", and "it really wasn't a good experience". | HN [32561840](https://news.ycombinator.com/item?id=32561840), 2022-08-23 | independent |
| SP4 | like | per-request opt-in to the browser | "Playwright only kicks in for pages that actually need JavaScript" | dev.to comment on [unixnick post](https://dev.to/unixnick/practical-web-scraping-in-python-playwright-scrapy-and-where-they-meet-44), 2026-07-29 | independent |
| SP5 | pain | Twisted reactor setup order | The setting "must be configured before Twisted imports or initializes its default reactor". 50 concurrent tabs "consumes gigabytes of memory". | [unixnick, dev.to](https://dev.to/unixnick/practical-web-scraping-in-python-playwright-scrapy-and-where-they-meet-44), 2026-07-28 | independent |
| SP6 | pain | hangs and silent failure | Title: "Scrapy hangs with no exception raised" (11 comments) | [scrapy-playwright#266](https://github.com/scrapy-plugins/scrapy-playwright/issues/266), 2024-03-23 | independent |
| SP7 | pain | memory | "there's an unresolved memory leak with playwright itself" | [scrapy-playwright#183](https://github.com/scrapy-plugins/scrapy-playwright/issues/183), 2023-03-13 | independent |
| SP8 | pain | meta-dict API | Title: "Scrapy-playwright scraper does not return 'page' or 'playwright_page' in response's meta" (4,594 views) | SO [75943441](https://stackoverflow.com/q/75943441), 2023-04-05 | independent |
| SP9 | pain | Windows and hangs | "execution getting stucked at Enabled pipeline … on Windows" (18 comments). SO [79793980](https://stackoverflow.com/q/79793980) (2025-10-18) reports the same freeze. | [scrapy-playwright#343](https://github.com/scrapy-plugins/scrapy-playwright/issues/343), 2025-06-17 | independent (title) |

## 3b. scrapy-poet (thin evidence)

There are no HN or SO discussions. Evidence comes from GitHub only.

| # | Like or pain | Theme | Fact | Source | Independence |
|---|---|---|---|---|---|
| PO1 | pain | interaction with Scrapy tooling | Title: "`scrapy parse` is broken" (3 reactions, 11 comments) | [scrapy-poet#39](https://github.com/scrapinghub/scrapy-poet/issues/39), 2020-09-15 | independent |
| PO2 | pain | docs | Title: "Make configuration documentation more visible" | [scrapy-poet#36](https://github.com/scrapinghub/scrapy-poet/issues/36), 2020-08-20 | title only |
| PO3 | pain | interaction with middlewares | Title: "Unintended interaction with RobotsTxtMiddleware, FilesPipeline, and ImagePipeline" | [scrapy-poet#48](https://github.com/scrapinghub/scrapy-poet/issues/48), 2021-05-30 | title only |

---

## 4. Spider-rs (spider crate, spider-py, spider-nodejs, spider.cloud clients)

There is no independent blog review and no HN or SO discussion beyond the two thin threads below. All usable evidence comes from GitHub issues. Vendor benchmark claims (for example "faster than scrappy [sic] by at least 2x" on PyPI) appear only as **unverified (snippet only)** WebSearch summaries of the PyPI page and are not used as evidence.

### Likes

| # | Theme | Quote | Source, date | Independence |
|---|---|---|---|---|
| R1 | general | "Thank you for this amazing crate!" | [spider#127](https://github.com/spider-rs/spider/issues/127), 2023-08-13 | independent |
| R2 | builder API in the bindings | "`with_full_resources(True)` is fantastic!" | [spider-py#6](https://github.com/spider-rs/spider-py/issues/6), 2024-07-19 | independent |
| R3 | speed (spider.cloud) | "Very cool - and impressive speed!" | HN [42466282](https://news.ycombinator.com/item?id=42466282), 2024-12-19 | independent |
| R4 | maintainer responsiveness | Feature requests in bindings were shipped the same day (spider-py v0.0.42, spider-nodejs 0.0.148 and spider-clients v0.0.24). | spider-py#6, spider-nodejs#5, spider-clients#3 | observed fact |

### Pain points

| # | Theme | Quote or fact | Source, date, version | Independence |
|---|---|---|---|---|
| R5 | configuration API shape | "as we add more features to `Website`, it gets more complicated" (the poster asks for a builder pattern) | [spider#115](https://github.com/spider-rs/spider/issues/115), 2023-05-07 | independent |
| R6 | docs | "where does the data go?" | [spider#78](https://github.com/spider-rs/spider/issues/78), 2023-01-05 | independent |
| R7 | Cargo feature flags and defaults | "If I set container to run in privileged mode, then it works fine." The cause was a tokio-uring panic after an update (15 comments). | [spider#243](https://github.com/spider-rs/spider/issues/243), 2025-01-05, after 2.23.7 | independent |
| R8 | Cargo feature flags leaking into the bindings | spider-py failed in Docker with normal security settings. The maintainer's fix was: "Remove default features and \"io_uring\"". | [spider-py#13](https://github.com/spider-rs/spider-py/issues/13), 2025-02-19 | independent report, vendor reply |
| R9 | Cargo feature-flag combinations | Title: "Bug: Broken `chrome` Feature" (compile errors) | [spider#349](https://github.com/spider-rs/spider/issues/349), 2026-02-06, 2.44.19 | independent |
| R10 | pluggable HTTP client | "it is necessary to use a library that impersonates web browser's TLS fingerprints" | [spider#222](https://github.com/spider-rs/spider/issues/222), 2024-10-22 | independent |
| R11 | page context and internals | Title: "Access to reqwest internal structures ?" | [spider#226](https://github.com/spider-rs/spider/issues/226), 2024-11-12 | independent |
| R12 | page context (final URL) | `page.get_url()` returns the pre-redirect URL, so the poster asks how to get the URL after redirect. | [spider#127](https://github.com/spider-rs/spider/issues/127), 2023-08-13 | independent |
| R13 | bindings lag the Rust crate | "I can't figure out how to duplicate some of the current functionality" (porting a Scrapy spider to spider-py) | [spider-py#6](https://github.com/spider-rs/spider-py/issues/6), 2024-07-19 | independent |
| R14 | type hints in bindings | "Please add python type hints or annotation". Asked again in [spider-py#16](https://github.com/spider-rs/spider-py/issues/16) "Add type hints" (2025-04-30). | [spider-py#2](https://github.com/spider-rs/spider-py/issues/2), 2024-06-05 | independent |
| R15 | bindings lag the Rust crate | Title: "Build failure with current spider" | [spider-py#18](https://github.com/spider-rs/spider-py/issues/18), 2026-01-19 | independent |
| R16 | binding releases and packaging | "there haven't been any builds tagged since 0.0.70". Also "crawlSmart just fails and doesn't throw an error". | [spider-nodejs#2](https://github.com/spider-rs/spider-nodejs/issues/2), 2024-06-03 | independent |
| R17 | binding docs | "Is there documentation for how to use it?" (asked about `wait_for` after it shipped) | [spider-nodejs#5](https://github.com/spider-rs/spider-nodejs/issues/5), 2024-08-23 | independent |
| R18 | lifecycle control | Title: "website.stop() does not stop the crawl" | [spider-py#14](https://github.com/spider-rs/spider-py/issues/14), 2025-04-03 | independent |
| R19 | packaging (wheels) | Titles: "Publish wheels" ([spider-py#15](https://github.com/spider-rs/spider-py/issues/15)) and "python: publish a wheel" ([spider-clients#31](https://github.com/spider-rs/spider-clients/issues/31)), 2025 | GitHub | independent (titles) |
| R20 | spider.cloud client types | "the api is missing types for a lot of method parametesr" [sic] | [spider-clients#3](https://github.com/spider-rs/spider-clients/issues/3), 2024-06-05 | independent |
| R21 | spider.cloud client dependencies | Title: "The javascript client needs supabase to be installed" | [spider-clients#24](https://github.com/spider-rs/spider-clients/issues/24), 2024-09-30 | independent |
| R22 | positioning | "It has nothing to do with AI at all." (on the "for AI" rebrand) | HN [42465185](https://news.ycombinator.com/item?id=42465185), 2024-12-19 | independent |

---

## 5. Colly (Go)

| # | Like or pain | Theme | Quote or fact | Source, date | Independence |
|---|---|---|---|---|---|
| G1 | pain | handler and callback design | "Not a big fan of callbacks when you have channels." | HN [15408911](https://news.ycombinator.com/item?id=15408911), 2017-10-05 | independent |
| G2 | like | handler and callback design | "Callbacks makes sense in this context and is the idiomatic way" | HN [15410356](https://news.ycombinator.com/item?id=15410356), 2017-10-05 | independent |
| G3 | like | handler and callback design | Channels "shouldn't be exposed in a public API" | HN [15409717](https://news.ycombinator.com/item?id=15409717), 2017-10-05 | independent |
| G4 | like | speed and concurrency | "it was a pleasure: the new programs are impressive fast" | HN [33763075](https://news.ycombinator.com/item?id=33763075), 2022-11-27 | independent |
| G5 | like | speed (migrated from Scrapy) | "Immediately went from CPU bottleneck to network." | HN [15955945](https://news.ycombinator.com/item?id=15955945), 2017-12-18 | independent |
| G6 | like | ergonomics | "Colly is a great library for builing scrapers if you are familiar with Go." [sic] | HN [37872019](https://news.ycombinator.com/item?id=37872019), 2023-10-13 | author-of-competitor (flyscrape) |
| G7 | pain | JS rendering | "Any plan to add JavaScript engines to this framework." (49 reactions, the top issue, open since 2017). SO [64192184](https://stackoverflow.com/q/64192184) asks "How to enable Js on colly". | [colly#4](https://github.com/gocolly/colly/issues/4), 2017-10-06 | independent |
| G8 | pain | maintenance | "I have nearly zero time to work on Colly nowadays." | [colly#840](https://github.com/gocolly/colly/issues/840), 2025-03-19 | vendor (maintainer) |
| G9 | pain | maintenance | "last release in 2020." Related: "New release?" (17 reactions) and "Why the latest version was released on 2020?" (9 reactions). | [colly#833](https://github.com/gocolly/colly/issues/833), [#817](https://github.com/gocolly/colly/issues/817), [#825](https://github.com/gocolly/colly/issues/825), 2024 | independent |
| G10 | pain | handler context and cancellation | "adding a context to the callback function signature" (8 reactions, open) | [colly#240](https://github.com/gocolly/colly/issues/240), 2018-10-24 | independent |
| G11 | pain | shared defaults across collectors | "I have to call `.NewCollection()` call `.Clone()` methods" | [colly#729](https://github.com/gocolly/colly/issues/729), 2022-09-30 | independent |
| G12 | pain | concurrency model clarity | "I don't quite understand what queues are for." SO [76824016](https://stackoverflow.com/q/76824016) reports that async mode with a queue gives inconsistent results. | [colly#676](https://github.com/gocolly/colly/issues/676), 2022-01-17 | independent |
| G13 | pain | cache extensibility | "the default caching behavior is just different enough from what I need" | HN [40930346](https://news.ycombinator.com/item?id=40930346), 2024-07-10 | independent |
| G14 | pain | distribution and storage | "Colly only supports a single machine via map of visited URL's." | HN [15955627](https://news.ycombinator.com/item?id=15955627), 2017-12-18 | independent |
| G15 | pain | politeness defaults | Title: "Robots.txt respect should be ON by default" (6 reactions) | [colly#614](https://github.com/gocolly/colly/issues/614), 2021-05-21 | title only |

---

## 6. Crawl4AI

| # | Like or pain | Theme | Quote or fact | Source, date, version | Independence |
|---|---|---|---|---|---|
| A1 | like | modular, optional LLM use | "it works very well without these dependencies and is modular" | HN [42215031](https://news.ycombinator.com/item?id=42215031), 2024-11-22 | independent |
| A2 | like | workflow clarity | "crawl4ai's workflow is easy to follow" | HN [43478906](https://news.ycombinator.com/item?id=43478906), 2025-03-26 | independent |
| A3 | like | config objects | "It's an async-first Python API, which reads cleanly once the naming clicks." | [Thunderbit review](https://thunderbit.com/blog/crawl4ai-review), updated 2026-07-17 | author-of-competitor |
| A4 | like | cheap local first tier | Uses local Crawl4AI and keeps Firecrawl for the last resort, describing Firecrawl as "hosted (and rather expensive)firecrawl" [sic]. | HN [46353048](https://news.ycombinator.com/item?id=46353048), 2025-12-22 | independent |
| A5 | pain | config churn and docs | "LLMconfig barely has documentation" (after a deprecation message) | SO [79512095](https://stackoverflow.com/q/79512095), 2025-03-16 | independent |
| A6 | pain | composing features | With deep crawl plus `arun_many()`, "the output results only contain the CrawlResult from the base URLs". [#1509](https://github.com/unclecode/crawl4ai/issues/1509) repeats the problem in 2025-09. | [crawl4ai#855](https://github.com/unclecode/crawl4ai/issues/855), 2025-03-18, 0.5.0.post4 | independent |
| A7 | pain | composing features | Per-page waits and deep-crawl strategies: "they don't fuse on their own". Also "Skip the wait_for and you're grabbing a half-built page." | Thunderbit review, 2026-07-17 | author-of-competitor |
| A8 | pain | docs example drift | `fit_markdown` from the documentation example fails (8 reactions). | [crawl4ai#453](https://github.com/unclecode/crawl4ai/issues/453), 2025-01-14, 0.4.247 | independent |
| A9 | pain | release quality | "The tests have diverged from the code base" | [crawl4ai#893](https://github.com/unclecode/crawl4ai/issues/893), 2025-03-26, 0.5.0.post1 | independent |
| A10 | pain | breaking changes and self-host | The title includes "silent API removals" (0.9.x Docker). | [crawl4ai#2091](https://github.com/unclecode/crawl4ai/issues/2091), 2026-07-21 | vendor (collaborator) |
| A11 | pain | self-host memory | Title: "Memory Leak on Repeated /md Requests via Docker" (10 reactions, open) | [crawl4ai#1256](https://github.com/unclecode/crawl4ai/issues/1256), 2025-06-29, 0.6.0-r2 | independent |
| A12 | pain | install weight and operations | "tracking library and browser releases, managing memory, tuning concurrency" | [Zack Proser blog](https://zackproser.com/blog/firecrawl-vs-crawl4ai), 2026-08-02 | independent, with an affiliate-commission disclosure (Firecrawl) |
| A13 | pain | install weight | "the browser downloads alone will annoy you" | Thunderbit review, 2026-07-17 | author-of-competitor |
| A14 | pain | production configuration docs | "new developers often struggle with production configurations" | HN [46646807](https://news.ycombinator.com/item?id=46646807), 2026-01-16 | independent (built an unofficial docs site) |
| A15 | pain | DOM coverage | "the crawl4ai, firecrawl tools do not pierce shadow dom." | HN [47874774](https://news.ycombinator.com/item?id=47874774), 2026-04-23 | likely author-of-competitor (posted their own tool) |
| A16 | pain | anti-bot | "issues on browerbase, crawl4ai and similar repos around being detected as a bot" [sic] | HN [44338997](https://news.ycombinator.com/item?id=44338997), 2025-06-21 | independent |
| A17 | pain | politeness defaults | Title: "Please respect robots.txt" (31 reactions, the top issue) | [crawl4ai#107](https://github.com/unclecode/crawl4ai/issues/107), 2024-09-29 | title only |

## 7. Firecrawl SDKs and self-hosting

| # | Like or pain | Theme | Quote or fact | Source, date, version | Independence |
|---|---|---|---|---|---|
| F1 | like | minimal SDK surface | "API key plus one SDK or HTTP call" | [Zack Proser](https://zackproser.com/blog/firecrawl-vs-crawl4ai), 2026-08-02 | independent with affiliate disclosure |
| F2 | like | output quality | "I am getting much better results from Firecrawl" ("not affiliated") | HN [47335513](https://news.ycombinator.com/item?id=47335513), 2026-03-11 | independent |
| F3 | like | product velocity | "their core offerings have become really good." | HN [48848008](https://news.ycombinator.com/item?id=48848008), 2026-07-09 | independent |
| F4 | like | self-host option | Self-host works, "albeit missing some features I never use anyways" | HN [49968986](https://news.ycombinator.com/item?id=49968986), 2026-10-05 | independent |
| F5 | like | simple self-host at v1.0.0 | Steps were only `USE_DB_AUTHENTICATION=false`, then `docker compose up -d`. | [codenote.net](https://codenote.net/en/posts/firecrawl-oss-self-host-introduction/), 2024-09-07 | independent |
| F6 | pain | self-host complexity and closed parts | "being closed source is the biggest deal breaker" (Fire-engine anti-bot), which led to a fork | HN [42056056](https://news.ycombinator.com/item?id=42056056), 2024-11-05 | independent (fork author) |
| F7 | pain | self-host proxies | "don't have proxy service which is the heart of any crawler and scraper" | HN [42086482](https://news.ycombinator.com/item?id=42086482), 2024-11-08 | independent |
| F8 | pain | self-host proxies | "We urgently need proxy support for the self-hosted version of the software." (18 reactions) | [firecrawl#1129](https://github.com/firecrawl/firecrawl/issues/1129), 2025-02-04 | independent |
| F9 | pain | self-host errors | "Error: All scraping engines failed!" (14 reactions) | [firecrawl#1390](https://github.com/firecrawl/firecrawl/issues/1390), 2025-03-31 | independent |
| F10 | pain | self-host hangs | "stuck in an infinite loop" (engine waterfall, 25 comments) | [firecrawl#2350](https://github.com/firecrawl/firecrawl/issues/2350), 2025-11-01 | independent |
| F11 | pain | self-host build and images | Titles: build failure in html-transformer (17 reactions), "Availability of a Pre-Built Playwright Image" (9) and "helm chart for k8s" (7). | [#1103](https://github.com/firecrawl/firecrawl/issues/1103), [#1322](https://github.com/firecrawl/firecrawl/issues/1322), [#1255](https://github.com/firecrawl/firecrawl/issues/1255), 2025 | titles only |
| F12 | pain | self-host operations | "The cost is that you're now operating those six containers." Also "My first scrapes got blocked by Firecrawl's private-IP / SSRF protection". | [Thunderbit review](https://thunderbit.com/blog/firecrawl-review), 2026-08-10, firecrawl-py 4.32.0 | author-of-competitor |
| F13 | pain | SDK type breaking change | "you are now required to write `ScrapeResponse<undefined>`" | [firecrawl#675](https://github.com/firecrawl/firecrawl/issues/675), 2024-09-16 | independent |
| F14 | pain | SDK breaking change | "can you please avoid breaking changes?" Also "Class doesn't exist anymore ScrapeOptions." | [firecrawl#2017](https://github.com/firecrawl/firecrawl/issues/2017), 2025-08-23 | independent |
| F15 | pain | error contract | Jobs return "`failed` and `cancelled` jobs to the caller exactly like successful ones". | [firecrawl#4212](https://github.com/firecrawl/firecrawl/issues/4212), 2026-08-03, firecrawl-py 4.13.0 to 4.34.0 | independent |
| F16 | pain | cost | "Firecrawl is egregiously expensive" | HN [45375331](https://news.ycombinator.com/item?id=45375331), 2025-09-25 | independent |
| F17 | pain | cost | "didn’t want to pay for the Firecrawl API too" | HN [43182981](https://news.ycombinator.com/item?id=43182981), 2025-02-26 | independent (built an alternative) |
| F18 | pain | speed | "firecrawl has no problem scraping them (although rather slowly)" | HN [46265550](https://news.ycombinator.com/item?id=46265550), 2025-12-14 | independent |
| F19 | pain | extraction quality | "the scrape with only main content needs some work" | HN [40093878](https://news.ycombinator.com/item?id=40093878), 2024-04-20 | independent |
| F20 | pain | reliability | "Firecrawl simply doesn't work..." | HN [43010362](https://news.ycombinator.com/item?id=43010362), 2025-02-11 | author-of-competitor |
| F21 | pain | politeness and robots.txt | "It doesn't even respect general robots.txt exclusions" (at launch) | HN [40027403](https://news.ycombinator.com/item?id=40027403), 2024-04-14 | independent |
| F22 | pain | hidden policy | "Firecrawl explicitly blocks Reddit." | HN [45388445](https://news.ycombinator.com/item?id=45388445), 2025-09-26 | independent |

## 8. Scrapling

| # | Like or pain | Theme | Quote or fact | Source, date | Independence |
|---|---|---|---|---|---|
| L1 | like | it works | "scrapling seems to pretty much just work." | HN [49825368](https://news.ycombinator.com/item?id=49825368), 2026-09-24 | independent |
| L2 | like | rendering API | "landed on Scrapling's API for seamless content rendering" | HN [48040324](https://news.ycombinator.com/item?id=48040324), 2026-05-06 | independent |
| L3 | pain | output size for agents | Scrapling "sometimes is a bit context heavy" (via MCP) | HN [49677004](https://news.ycombinator.com/item?id=49677004), 2026-09-12 | independent |
| L4 | pain | packaging extras | Title: "Bare `pip install scrapling` imports fail; quickstart should use `scrapling[fetchers]`" | [Scrapling#343](https://github.com/D4Vinci/Scrapling/issues/343), 2026-06-06 | independent |
| L5 | pain | packaging extras | "base-install-looks-fine-then-explodes-on-first-fetch sequence is real" | [Thunderbit review](https://thunderbit.com/blog/scrapling-review), 2026-07-17 | author-of-competitor |
| L6 | pain | adaptive selector limits | "Scrapling relocated the first saved element, not all three." | same review | author-of-competitor |
| L7 | pain | import-time failures | Title: "Scrapling 0.4.12 fails to import on Linux (ValueError in browserforge at module load)" | [Scrapling#396](https://github.com/D4Vinci/Scrapling/issues/396), 2026-08-02 | title only |

## 9. Katana (CLI-first; little API-level evidence)

| # | Like or pain | Theme | Quote or fact | Source, date | Independence |
|---|---|---|---|---|---|
| K1 | expectation | frontier and scheduling | "the hardest things in distributed crawling at scale are a good URL frontier" | HN [33549493](https://news.ycombinator.com/item?id=33549493), 2022-11-10 | independent |
| K2 | pain | archive output | "a little surprising to see no support for the WARC" | HN [33549371](https://news.ycombinator.com/item?id=33549371), 2022-11-10 | independent |
| K3 | pain | browser concurrency | Concurrency "doesn’t work as expected in the browser environment" | [katana#1215](https://github.com/projectdiscovery/katana/issues/1215), 2025-03-18, v1.1.2 | independent |
| K4 | pain | output combined with headless mode | Title: "`-jsonl` option with `-headless` option results in error" (37 comments) | [katana#611](https://github.com/projectdiscovery/katana/issues/611), 2023-10-01 | title only |

## 10. Cross-cutting comments (no single framework)

| # | Like or pain | Theme | Quote | Source, date | Independence |
|---|---|---|---|---|---|
| X1 | expectation | HTTP first, browser on demand | Headless scraping "is in the order of 10-100x slower" | HN [32561621](https://news.ycombinator.com/item?id=32561621), 2022-08-23 | independent |
| X2 | pain | boilerplate without a framework | "wrote a lot of error handling boilerplate" (mixed HTTP and headless) | HN [32563071](https://news.ycombinator.com/item?id=32563071), 2022-08-23 | independent |
| X3 | expectation | raw-response archive and replay | "best to to crawl and store full html that can be scraped offline" [sic] | HN [28518949](https://news.ycombinator.com/item?id=28518949), 2021-09-14 | independent |
| X4 | expectation | raw-response archive | Streaming the HTML to blob storage means retries "reference the saved raw data" | HN [32570745](https://news.ycombinator.com/item?id=32570745), 2022-08-23 | independent |
| X5 | expectation | DX over stealth | "The biggest value for me is being able to respond to site changes quickly." | HN [32561745](https://news.ycombinator.com/item?id=32561745), 2022-08-23 | independent |
| X6 | expectation | stealth over DX | The main "selling point" "will always be stealthiness/human like behavior" | HN [32561435](https://news.ycombinator.com/item?id=32561435), 2022-08-23 | independent |

---

## Ranked cross-framework themes

The count covers **independent** items only, as labelled above. A Stack Overflow aggregate row counts as one item and its size is shown in brackets. Vendor and competitor items are listed separately and are not counted. Ranking is by independent count, with intensity (votes and reactions) breaking ties.

| Rank | Theme (API level) | Independent reports | Frameworks | Key links | Supporting non-independent items |
|---|---|---|---|---|---|
| 1 | **Embedding and lifecycle: running from a script or app, restart, event loop** (Twisted reactor, `ReactorNotRestartable`, reactor setup order, Twisted version pins, Apify `main()` exiting the process, `stop()` not stopping) | **12**: S8, S9, S10 [54 questions and 228 votes; 100+ script questions; 41 Celery questions], S11, S12, S13, S14, S15, S16, SP5, CJ32, R18 | Scrapy, scrapy-playwright, Apify SDK, spider-py | [SO 13437402](https://stackoverflow.com/q/13437402), [scrapy#2473](https://github.com/scrapy/scrapy/issues/2473), [scrapy#6024](https://github.com/scrapy/scrapy/issues/6024) | S17 (maintainer), S18 (competitor), CP2 (vendor pitch for asyncio) |
| 2 | **Documentation gaps, undocumented config, missing real-world examples** | **12**: CJ30, CP3, CP4, CP8, CP9, CP14, R6, R17, A5, A8, A14, S24 | Crawlee JS and Python, spider, Crawl4AI, Scrapy | [HN 40918032](https://news.ycombinator.com/item?id=40918032), [crawlee-python#862](https://github.com/apify/crawlee-python/issues/862), [SO 79512095](https://stackoverflow.com/q/79512095) | CJ16, CP17 (vendor) |
| 3 | **Install, packaging and runtime friction** (browser binaries not installed, Alpine and Docker, Deno and Bun, wheels, extras, binding builds) | **11**: CJ21, CJ23, CJ24, CJ25, L4, R15, R16, R19, A12, CP18 (title), L7 (title) | Crawlee, Scrapling, spider bindings, Crawl4AI | [SO 76225651](https://stackoverflow.com/q/76225651), [Scrapling#343](https://github.com/D4Vinci/Scrapling/issues/343), [spider-nodejs#2](https://github.com/spider-rs/spider-nodejs/issues/2) | CJ22, A13, L5 (competitor) |
| 4 | **Switching HTTP to browser through one API** (mostly a like; a pain where it is bolted on) | **10**: likes CJ1, CJ2, CJ4, SP1, SP2, SP4, X1; pains SP3, G7 (49 reactions), S26 [168 + 101 votes; 581 scrapy-splash questions] | Crawlee (like), Scrapy, scrapy-playwright, Colly (pain) | [HN 32561840](https://news.ycombinator.com/item?id=32561840), [colly#4](https://github.com/gocolly/colly/issues/4), [SO 8550114](https://stackoverflow.com/q/8550114) | CJ10 (vendor), CJ27 (competitor: "not a one-line class-only swap") |
| 5 | **Breaking changes and upgrade churn in SDKs and bindings** | **9**: F13, F14, F15, A5, A9, CJ31, S16, S31, R15 | Firecrawl SDKs, Crawl4AI, Crawlee, Scrapy and Twisted, spider-py | [firecrawl#2017](https://github.com/firecrawl/firecrawl/issues/2017), [firecrawl#675](https://github.com/firecrawl/firecrawl/issues/675), [crawlee#3063](https://github.com/apify/crawlee/issues/3063) | A10 (collaborator: "silent API removals") |
| 6 | **Extensibility: middleware, pluggable storage, HTTP client and cache** | **9 pains**: CJ29, CP5, CP6, CP7, G13, R10, R11, CJ17, CJ26. **4 likes for Scrapy's model**: S1, S2, S3, S7 | Crawlee JS and Python, Colly, spider (pain); Scrapy (like) | [HN 40917985](https://news.ycombinator.com/item?id=40917985), [spider#222](https://github.com/spider-rs/spider/issues/222), [crawlee#1822](https://github.com/apify/crawlee/issues/1822) | — |
| 7 | **Storage persistence, purge on start and resume** (second run does nothing, storage dir ignored, ENOENT at scale) | **7**: CJ11, CJ12, CJ13, CJ14, CJ15, CP12, plus CP13 (**unverified**) | Crawlee JS and Python | [crawlee#1602](https://github.com/apify/crawlee/issues/1602), [SO 74709844](https://stackoverflow.com/q/74709844), [crawlee-python#369](https://github.com/apify/crawlee-python/issues/369) | CJ16 (vendor admits), CJ9 and CJ28 (competitor) |
| 8 | **Bindings and ports lag the core** (spider-py and spider-nodejs versus the crate; Crawlee Python versus JS) | **7**: R13, R14, R15, R16, R17, R20, CP10 | spider-rs, spider.cloud clients, Crawlee Python | [spider-py#6](https://github.com/spider-rs/spider-py/issues/6), [spider-py#18](https://github.com/spider-rs/spider-py/issues/18) | CP11 (vendor admits) |
| 9 | **Concurrency and handler semantics are unclear or composition fails** (callbacks versus channels, queue plus async, duplicate handler calls, deep crawl plus batch, browser concurrency) | **7**: G1, G12, CJ18, A6, K3, CJ19; the callbacks-versus-channels debate also has 2 likes, G2 and G3 | Colly, Crawlee, Crawl4AI, Katana | [HN 15408911](https://news.ycombinator.com/item?id=15408911), [crawl4ai#855](https://github.com/unclecode/crawl4ai/issues/855), [katana#1215](https://github.com/projectdiscovery/katana/issues/1215) | CJ20 (vendor), A7 (competitor) |
| 10 | **Self-host complexity** (proxies missing, closed anti-bot engine, engines failing, hangs, memory, images) | **7**: F6, F7, F8, F9, F10, F11 (titles), A11. F4 and F5 are counter-likes. | Firecrawl, Crawl4AI Docker | [firecrawl#1129](https://github.com/firecrawl/firecrawl/issues/1129), [HN 42056056](https://news.ycombinator.com/item?id=42056056) | F12 (competitor) |
| 11 | **Handler context access** (page events, response internals, final URL, playwright page through meta, extending context helpers) | **6**: CJ17, CJ33, R11, R12, R13, SP8 | Crawlee and Apify SDK, spider, scrapy-playwright | [SO 64355710](https://stackoverflow.com/q/64355710), [spider#127](https://github.com/spider-rs/spider/issues/127) | CP16 (vendor employee: `user_data` shared with internals) |
| 12 | **Silent hangs and swallowed failures** | **6**: SP6, SP9, R16 ("fails and doesn't throw"), F10, F15, R18 | scrapy-playwright, spider-nodejs, Firecrawl, spider-py | [scrapy-playwright#266](https://github.com/scrapy-plugins/scrapy-playwright/issues/266), [firecrawl#4212](https://github.com/firecrawl/firecrawl/issues/4212) | — |
| 13 | **Passing data between callbacks or handlers** | **3**: S20 [aggregate of 4 SO questions, 168 votes], G10, CP14. Counting the 4 SO questions individually gives 6. Weighted by votes, this is the most-voted theme after lifecycle. | Scrapy, Colly, Crawlee Python | [scrapy#1138](https://github.com/scrapy/scrapy/issues/1138), [crawlee-python#524](https://github.com/apify/crawlee-python/issues/524) | S19 (maintainer), CP15 and CP16 (vendor employee) |
| 14 | **Unsafe defaults, politeness and robots.txt** | **5**: S28, S29, A17 (31 reactions), G15, F21 | Scrapy, Crawl4AI, Colly, Firecrawl | [scrapy#6597](https://github.com/scrapy/scrapy/issues/6597), [crawl4ai#107](https://github.com/unclecode/crawl4ai/issues/107) | — |
| 15 | **Type hints and typed SDK surfaces** | **5**: S30, R14, R20, F13, plus spider-py#16 within R14 | Scrapy, spider-py, spider.cloud client, Firecrawl JS | [scrapy#4041](https://github.com/scrapy/scrapy/issues/4041), [spider-py#2](https://github.com/spider-rs/spider-py/issues/2) | CP2 (vendor touts type hints) |
| 16 | **Cargo feature flags and default features** (io_uring default breaks containers, broken feature combos) | **3**: R7, R8, R9 | spider crate and spider-py | [spider#243](https://github.com/spider-rs/spider/issues/243), [spider#349](https://github.com/spider-rs/spider/issues/349) | — |
| 17 | **Raw-response archive, HTTP cache and replay for testing** (an expectation more than a complaint) | **4**: X3, X4, S7, S1 | Scrapy (has it), Crawlee and Colly (asked for: CP7, G13) | [HN 40928731](https://news.ycombinator.com/item?id=40928731) | — |
| 18 | **Maintenance and abandonment risk** | **4**: G9 (3 issues), F6 (fork) | Colly, Firecrawl | [colly#833](https://github.com/gocolly/colly/issues/833) | G8 (maintainer) |
| 19 | **Speed and efficiency as the main like** | **4**: G4, G5, R3, S5 (BeetleB also says Scrapy uses "an order of magnitude less power" than a browser) | Colly, spider, Scrapy | [HN 15955945](https://news.ycombinator.com/item?id=15955945) | — |
| 20 | **Cost of the hosted API** | **3**: F16, F17, A4 | Firecrawl | [HN 45375331](https://news.ycombinator.com/item?id=45375331) | — |

### Readout

- **Liked most often:** one API across HTTP and browser fetching, with per-request opt-in to the browser (Crawlee, scrapy-playwright when it works). Testable, well-defined callbacks and Parsel selectors (Scrapy). Raw speed (Colly, spider). Small, explicit API surfaces (Firecrawl SDK, Crawl4AI config objects once learned).
- **Biggest pains by volume:** embedding and lifecycle (Scrapy and Twisted). Documentation gaps. Install and runtime friction. Silent storage persistence that breaks the second run (Crawlee). SDK and binding churn (Firecrawl, Crawl4AI, spider bindings).
- **Opinionated versus extensible:** the Crawlee Python launch thread shows a real tension between "fewer than 10 lines" defaults (vendor) and users who "need the flexibility middleware gives you" (CP5, CP6, CP7).
- **Evidence gaps:** there is no Reddit or Lobsters evidence because both block access. Spider-rs and scrapy-poet have no independent long-form reviews. The Thunderbit hands-on reviews are detailed but come from a competitor. Each finding above is a single report unless an aggregate is stated.
