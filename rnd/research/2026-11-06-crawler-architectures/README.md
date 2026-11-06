# How the leading crawlers organise the work around one page (2026-11-06)

Synthesis for the wayfinder research ticket [How do the leading crawlers organise the work around one page?](https://github.com/goodfetch/goodfetch/issues/2159), on the map [Foxhound matches or beats its competitors by default](https://github.com/goodfetch/goodfetch/issues/2141). Four parallel source readings, one per family, each pinned to a release and cited by file and line:

| Report | Crawlers and versions |
|---|---|
| [crawlee.md](crawlee.md) | Crawlee JS 3.18.2 (and master, 4.0.0-rc), Crawlee Python 1.11.1, got-scraping 4.2.2, fingerprint-injector 2.1.89 |
| [scrapy.md](scrapy.md) | Scrapy 2.19.0, scrapy-playwright 0.0.48 with Playwright 1.64.1, Scrapling 0.5.1 |
| [compiled.md](compiled.md) | Spider 2.53.9 with Chromey 2.58.2, Colly 2.3.0, Katana 1.8.0 |
| [scale.md](scale.md) | Heritrix3 3.18.0, StormCrawler 3.7.0, Nutch 1.23, crawl4ai 0.9.5, Firecrawl 2.11.0 |

Inputs not repeated: the Spider gap measurement (R1–R9, the single-actor chain), the browser gap measurement and the frontier persistence survey of the same day.

**Nothing was built or run.** Every per-page figure below is a count of calls, commands or writes read from source, except Scrapy's seen set (≈ 95 bytes a URL, measured). Each report lists what it leaves unverified.

## The per-page path, across crawlers

| Point | What the leaders do | Foxhound today |
|---|---|---|
| **Hand-offs a page crosses** | None on a hot path: Heritrix carries a page through fetch, extraction, WARC and outlink scheduling in one thread; Spider, Crawlee and Scrapy keep it on one task or event loop. Frontier calls run in place, under a per-queue lock or a map lookup. | Three single-threaded hops (Politeness actor, analysis pool, run-state writer), 33 futex calls a page. |
| **Duplicate check** | Memory first, everywhere: Crawlee JS's 2 M-entry LRU and 1 M-slot hashed array in front of storage; StormCrawler's cache of discovered URLs; Heritrix's host-local fingerprints behind BerkeleyDB's cache (Bloom filter optional); Scrapy's set of 20-byte fingerprints; Spider's `HashSet`. No crawler reads the disk to learn a URL was seen. | Every links batch goes to the run file through the writer. |
| **Claiming work** | In batches or from memory: Crawlee JS locks 25 requests per call and serves them from memory; Nutch feeds fetchers through a bounded in-memory queue. | One claim and one lease per page on the writer. |
| **Politeness** | A counter and a timestamp per host, checked under a lock (StormCrawler, Nutch, Heritrix's snooze queue); Crawlee and Scrapy cost one map lookup when no delay is set; Colly a channel send, Katana a token bucket. | A Permit from an in-memory SQLite on its own actor, about 9 statements, even when it cannot bind. |
| **Concurrency** | Fixed by default (Scrapy 16, Spider's semaphore), or adapted to load (Crawlee's AutoscaledPool, crawl4ai's memory-pressure dispatcher: stop above 90 % memory, resume below 85 %). Scrapy also caps parse work: it stops pulling when 5 MB of responses wait for parsing. | Fixed; the analysis queue has no byte budget. |
| **Parsing** | Once, and only when something reads it: Scrapy parses lazily; StormCrawler builds its W3C tree only when a filter needs it; Firecrawl extracts links only for the links format. Spider streams its link reading through lol_html. Several parse twice or more (Crawlee JS v3, Firecrawl three to four times, crawl4ai twice): an anti-pattern. | One parse (spec #1577), after the whole body. |
| **Progress** | Counters per page, a snapshot on a timer (Crawlee every 10 s, Heritrix every 20 s, Scrapy every 60 s) or on request (Firecrawl). | In-flight progress serialized to JSON 32 times a page. |
| **Writes per page** | None by default for most (Scrapy, Spider, Colly, crawl4ai); Crawlee one file per item; StormCrawler bulks status 500 actions or 5 s; Heritrix defers BerkeleyDB updates to checkpoints. No one syncs per URL. | Claim, lease and outcome committed per page, at every Durability level. |
| **Browsers per crawl** | One (Spider, scrapy-playwright, Crawlee JS by default, Heritrix's Firefox, StormCrawler per instance). | 13 at once, 43 per crawl. |
| **Contexts** | One per crawl (scrapy-playwright, Spider, StormCrawler), one per browser (Crawlee JS, whose Session rotation retires the browser), one per configuration signature, at most 20 (crawl4ai). Only Firecrawl opens one per request. | One per Session: 100. |
| **Pages** | A new page per request, closed after, is the norm. Scrapling and Katana's headless engine reuse pages; Spider has a tab pool it never calls; Spider closes tabs on a dedicated thread so the page's task never waits. | A new target per request; close then detach, which Chrome refuses once every 2.5 pages. |
| **Recycling** | Never for memory alone in the browser crawlers read. Crawlee retires a browser after 100 pages or 10 s idle; crawl4ai after N pages (off by default) and drains it while new pages go to a fresh one. Memory slows the autoscaler (Crawlee) or stops admission (crawl4ai). | Twice the footprint read at the second page: healthy browsers recycled. |
| **Commands and scripts per page** | Spider about 21 CDP commands, scrapy-playwright about 20, plus interception. Init scripts only when a feature is on (crawl4ai), once per context (Crawlee's 27 KB fingerprint, marked so it is not injected twice), or per tab but small (Spider, tens of KB). Content read in one composed call (Spider, Katana, Crawlee JS). | 57–60 commands; the 583 KB Consent runner bundle in every page; 8 isolated worlds and 16 calls to read a page. |
| **Machine pressure** | Sampled in the background (Spider every 200 ms; a page reads an atomic). | A PSS walk of the browser's processes at every page opening. |

## What Foxhound can adopt, each as a setting with its off state

Mapped to the map's tickets. "Default" is the reading's recommendation, for the owner to decide in those tickets.

| Adoption | Precedent | Off state gives back | Ticket on the map |
|---|---|---|---|
| The page stays on its own task; frontier, lease and Permit are in-memory operations of one owner | Heritrix, Spider, Crawlee | `run_state.durability: full` with `run_state.memory.budget: 0` | [What are the memory-first Run state's batch and memory defaults?](https://github.com/goodfetch/goodfetch/issues/2144) |
| A seen-cache in front of the run file, sized in bytes | Crawlee JS, StormCrawler, Heritrix | budget 0 | the same |
| Claims in batches from memory | Crawlee JS (25), Nutch | a batch of 1 | the same |
| Politeness as per-host counters and timestamps; skipped when it cannot bind | StormCrawler, Nutch, Crawlee, Scrapy | `politeness.sharing: shared` keeps the file | spec #2138 (run-private Politeness) |
| Progress counted per page, published on a timer or on read | Crawlee, Heritrix, Firecrawl | none needed (no behaviour lost) | #2134, outside the map |
| A byte budget on responses waiting for analysis | Scrapy (5 MB) | unset | Not yet specified on the map |
| Host-fair dispatch: next URL from the host with fewest in flight | Scrapy's `DownloaderAwarePriorityQueue` | today's order | Not yet specified on the map |
| Sessions open on demand; one context per Session kept, or `session.isolation: browser` as Crawlee does | Crawlee, crawl4ai, scrapy-playwright | `session.pool.growth: eager` | [How does a crawl's Session pool grow?](https://github.com/goodfetch/goodfetch/issues/2146) |
| Recycling by growth after warm-up, with page-count retirement as a setting and drain-then-replace | Crawlee (100 pages), crawl4ai (N pages, drain) | today's rule through the settings | [When is a crawl's browser recycled?](https://github.com/goodfetch/goodfetch/issues/2147) |
| A Session's page reused; tabs closed off the page's path | Scrapling, Katana headless; Spider's tab closer | `crawl.browser.page_reuse: false` | [Does a Session's page serve its next request?](https://github.com/goodfetch/goodfetch/issues/2148) |
| Init scripts once per context, and only for features on (the Consent runner only where Consent is active, and smaller) | Crawlee, crawl4ai | today's per-page registration | Not yet specified on the map ("browser cuts beyond #2139") |
| Content and detectors read in one composed call | Spider, Katana, Crawlee JS | none needed | #1892, #1894, outside the map |
| Machine pressure from a background sample | Spider | none needed | spec #2139 (pool reads the governor's last measure) |
| Parse only for what reads it; build the tree as chunks arrive | Scrapy, StormCrawler, Firecrawl (links on demand); html5ever's chunked input | today's eager parse | [What does the extraction-path spec decide?](https://github.com/goodfetch/goodfetch/issues/2155) |

## What not to copy

- **Durability below the bar:** Crawlee writes one file per request with lock files and no `fsync`; Colly marks a URL seen before it is fetched; Spider and default Colly have no resume.
- **Losing correctness for speed:** Spider counts depth as URL path segments and rewrites every link's scheme to the parent's; Crawlee Python shares one context across Sessions, mixing cookies; Spider swaps a crawl's concurrency for a process-wide pool above 70 % CPU.
- **Waste:** Firecrawl's three to four parses and one Redis round trip per discovered link; crawl4ai copying the whole visited set after every page when a resume callback is set; Katana's 1 s queue polling and 10 s idle end; StormCrawler and Nutch sleep-polling 100–500 ms; Heritrix and Nutch opening a new connection for every URL.
- **Scaling:** Crawlee's autoscaler takes about 150 s to reach 16; Crawlee JS serializes every page open, browser launches included, behind a pool-wide lock; Colly starts one goroutine per link with no bound.
- **Interception everywhere:** scrapy-playwright, Firecrawl and StormCrawler route every sub-request through a script handler, and Playwright then turns Chrome's HTTP cache off. Interception only when a feature on the page needs it (#1895).

## Unverified across the four readings

- Every CPU or time effect: these are counts read from source.
- CDP commands behind each Playwright call (crawl4ai, StormCrawler, Firecrawl) and behind rod (Katana); Spider's 21 was counted from source, not captured.
- The size of Spider's stealth script; Crawlee Python's fingerprint injection cost.
- Library defaults read from documentation rather than source: OkHttp, undici, impit, Node's keep-alive agents, Go 1.26's HTTP/2 rule.
