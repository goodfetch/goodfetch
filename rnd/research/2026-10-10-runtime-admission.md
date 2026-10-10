# Research: cross-domain request admission and fairness in crawlers

Date: 2026-10-10. Facts only, no decisions. Every claim carries a source. "Verified (source)" means read in the cited code or document at the stated version. "Inference" marks my own reasoning; it is not a sourced fact.

Method note: the Exa search tool was out of credits, so I used WebFetch/WebSearch and shallow `git clone`s of each project's default branch (HEAD dates 2026-10-01..09). Where the default branch is ahead of the latest release, both are named.

Versions examined:

| Project | Latest release | Source read |
|---|---|---|
| Scrapy | 2.19.0 (2026-09-10), https://pypi.org/project/Scrapy/ | `master` (be7bdc4) |
| Crawlee JS | 3.18.2 `latest`; 4.0.0-rc.1 `rc` (npm dist-tags) | tag `v3.18.2` and `master` (4.0.0) |
| Crawlee Python | 1.10.4 (2026-10-06), https://pypi.org/project/crawlee/ | `master` (1.10.5); 1.10.4 wheel checked |
| spider-rs `spider` | 2.53.9 on crates.io | `main` (2.54.0) |
| Colly | v2.3.0 | `master` (5740023) |
| Katana | v1.8.0 | `dev` HEAD (3e65064) |
| Heritrix | 3.18.0 | `master` (3.18.1-SNAPSHOT) |
| StormCrawler | 3.7.0 | `main` (4.0.0-SNAPSHOT) |
| Nutch | 1.23 | `master` (1.24-SNAPSHOT) |
| Firecrawl | n/a (service) | `main` (6f892bc) |
| Stagehand | 4.2.0 (npm `latest`) | `main` (1fc8849) |

---

## 1. How each competitor orders and admits requests across domains

### Scrapy (2.19.0)

Two layers: a scheduler (priority queue) and a downloader that groups requests into "slots".

- **Slot key = hostname, not registrable domain.** `Downloader.get_slot_key()` returns `request.meta["download_slot"]` if set, otherwise `urlparse_cached(request).hostname`. Verified: https://github.com/scrapy/scrapy/blob/master/scrapy/core/downloader/__init__.py (lines ~257-265). So `CONCURRENT_REQUESTS_PER_DOMAIN` and `DOWNLOAD_DELAY` are applied per host despite the "DOMAIN" name.
- **Built-in defaults** (`scrapy/settings/default_settings.py`, https://github.com/scrapy/scrapy/blob/master/scrapy/settings/default_settings.py): `CONCURRENT_REQUESTS = 16`, `CONCURRENT_REQUESTS_PER_DOMAIN = 8`, `DOWNLOAD_DELAY = 0`, `DOWNLOAD_DELAY_JITTER = 0.5`, `DOWNLOAD_SLOTS = {}`, `ROBOTSTXT_OBEY = False`, `AUTOTHROTTLE_ENABLED = False`, `SCHEDULER_PRIORITY_QUEUE = "scrapy.pqueues.DownloaderAwarePriorityQueue"`, `SCHEDULER_MEMORY_QUEUE = LifoMemoryQueue`, `SCHEDULER_DISK_QUEUE = PickleLifoDiskQueue`, `DEPTH_PRIORITY = 0`.
- **`startproject` template overrides:** `ROBOTSTXT_OBEY = True`, `CONCURRENT_REQUESTS_PER_DOMAIN = 1`, `DOWNLOAD_DELAY = 1` (https://github.com/scrapy/scrapy/blob/master/scrapy/templates/project/module/settings.py.tmpl). The template changed in 2.13.3 (2025-07-02): DOWNLOAD_DELAY 0 to 1 and CONCURRENT_REQUESTS_PER_DOMAIN 8 to 1 (https://github.com/scrapy/scrapy/blob/master/docs/news/2.x.rst, "Scrapy 2.13.3"). The docs show these as "Default: 1 (fallback: 8)" (https://docs.scrapy.org/en/latest/topics/settings.html). Optimize guide: a startproject project "gets one request per second per domain" (https://github.com/scrapy/scrapy/blob/master/docs/topics/optimize.rst).
- **Default order inside a priority level is LIFO (depth-first).** BFO needs `DEPTH_PRIORITY=1` plus FIFO queues (https://docs.scrapy.org/en/latest/topics/scheduler.html).
- **`ScrapyPriorityQueue`:** one internal queue per integer priority, lower number = higher priority; not slot-aware (https://docs.scrapy.org/en/latest/topics/scheduler.html; https://github.com/scrapy/scrapy/blob/master/scrapy/pqueues.py).
- **`DownloaderAwarePriorityQueue` (DAPQ):** one `ScrapyPriorityQueue` per download slot. `pop()` picks the slot with the fewest active downloads. On a tie it takes the next slot name after the last one chosen, so ties rotate round-robin (`_next_slot`, https://github.com/scrapy/scrapy/blob/master/scrapy/pqueues.py lines ~417-470). Priority is only honoured inside the chosen slot. Docs: "domains (slots) with the least amount of active downloads are dequeued first" (scheduler docs above).
- **DAPQ became the default in 2.14.0 (2026-01-05)** ("The default priority queue is now DownloaderAwarePriorityQueue", https://github.com/scrapy/scrapy/blob/master/docs/news.rst; PR https://github.com/scrapy/scrapy/pull/6940, issue https://github.com/scrapy/scrapy/issues/6924). Scheduling across multiple domains was improved in 2.15.0 (gh 7293/7351; same changelog). `CONCURRENT_REQUESTS_PER_IP` was deprecated in 2.14.0 (same changelog).
- **Global cap mechanism:** `Downloader.needs_backout()` is `0 < CONCURRENT_REQUESTS <= len(self.active)`. `active` counts requests sitting in a slot's `deque` waiting on that slot's concurrency or delay, not just those on the wire (`fetch()` adds the request to `self.active` before `_enqueue_request` appends it to `slot.queue`). Verified in the downloader source above.
  - Inference: with a slot-unaware scheduler queue, requests for one slow or delayed host can fill all `CONCURRENT_REQUESTS` places while they wait in that host's deque. That is head-of-line blocking at the global cap.
  - Fact: this happened in the field. Issue #6982 "Slot issue, one slot takes all CONCURRENT_REQUESTS=100, even if it has concurrency = 1" was closed as "Fixed by #6940 in 2.14.0", the DAPQ default switch (https://github.com/scrapy/scrapy/issues/6982).
- **AutoThrottle:** off by default. Adjusts each slot's delay toward `latency / AUTOTHROTTLE_TARGET_CONCURRENCY` (start 5 s, max 60 s, target 1.0). `DOWNLOAD_DELAY` is the floor and `CONCURRENT_REQUESTS_PER_DOMAIN` is still respected (https://docs.scrapy.org/en/latest/topics/autothrottle.html).
- **robots.txt Crawl-delay is not applied:** "Scrapy does not act on its Crawl-delay and Request-rate directives" (https://github.com/scrapy/scrapy/blob/master/docs/topics/optimize.rst).
- Broad crawls: "the default settings are optimized for crawls targeting a single website". Recommends `CONCURRENT_REQUESTS` close to `CONCURRENT_REQUESTS_PER_DOMAIN` x number of domains (optimize.rst, "Speeding up broad crawls").

### Crawlee JS (3.18.2 latest; 4.0.0-rc.1)

- **Queue:** a single `RequestQueue`, FIFO, with `forefront: true` putting a request at the head. It "supports both breadth-first and depth-first crawling orders" (https://github.com/apify/crawlee/blob/master/packages/core/src/storages/request-queue.ts). The queue has no per-domain structure.
- **Concurrency:** one `AutoscaledPool` for the whole crawler. Defaults in v3 are `minConcurrency = 1`, `maxConcurrency = 200`, `maxTasksPerMinute = Infinity` (https://github.com/apify/crawlee/blob/v3.18.2/packages/core/src/autoscaling/autoscaled_pool.ts lines ~46-128, 250-263). v4 keeps the same defaults in `ConcurrencySystem` (`maxConcurrency` default 200; https://github.com/apify/crawlee/blob/master/packages/basic-crawler/src/internals/autoscaling/concurrency-system.ts). It scales on CPU, memory and event-loop load, not per domain.
- **`maxRequestsPerMinute`:** a global rate (maps to `maxTasksPerMinute`), default Infinity (basic-crawler.ts in both versions).
- **`sameDomainDelaySecs`** (default 0):
  - v3: `delayRequest()` keeps a `Map<domain, lastAccessTime>` keyed by `getDomain(url)` from `tldts`. A request fetched too early is put back after a `setTimeout` via `reclaimRequest` (https://github.com/apify/crawlee/blob/v3.18.2/packages/basic-crawler/src/internals/basic-crawler.ts lines ~1555-1595). It is a re-queue loop, not per-domain queues. Added in 3.x via PR #2003 (https://github.com/apify/crawlee/blob/master/CHANGELOG.md).
  - v4 (rc): `sameDomainDelaySecs > 0` wraps the request manager in a `ThrottlingRequestManager` with `domains: 'all'`, `throttleBy: 'registrableDomain'` (basic-crawler.ts master lines ~1092-1115). That manager routes each throttled domain's requests into its own sub-queue. "`fetchNextRequest()` serves the domain that has been waiting longest", skips domains in backoff, never blocks, and returns `null` so "the crawler idles instead of holding a concurrency slot open". It runs two clocks per domain: 429 backoff (honours `Retry-After`) and crawl delay (robots.txt `Crawl-delay` floored by `minCrawlDelaySecs`) (https://github.com/apify/crawlee/blob/master/packages/basic-crawler/src/internals/throttling-request-manager.ts lines ~223-260). Its default `throttleBy` when built directly is `'hostname'` (line ~359).
- **No per-domain concurrency cap or fairness by default in v3.** A maintainer answered the Python equivalent question: "No, you cannot currently set different concurrency for different domains", and suggested one crawler instance per domain (https://github.com/apify/crawlee-python/issues/1263, 2025-06-20).
- `respectRobotsTxtFile` defaults to `false` (v3 basic-crawler.ts line ~662; v4 line ~940).

### Crawlee Python (1.10.4)

- `ConcurrencySettings` defaults: `min_concurrency=1`, `max_concurrency=100`, `desired_concurrency=10`, `max_tasks_per_minute=inf` (https://github.com/apify/crawlee-python/blob/master/src/crawlee/_types.py lines ~100-125).
- Opt-in `ThrottlingRequestManager` (present in the released 1.10.4 wheel, checked) covers explicitly listed `domains`: "takes from the sub-manager whose domain has been waiting the longest, skipping domains in a cooldown". Delay sources are HTTP 429 and robots.txt crawl-delay. Defaults are `base_delay=2s`, `max_delay=60s`, `max_domain_stall=900s` (https://github.com/apify/crawlee-python/blob/master/src/crawlee/request_loaders/_throttling_request_manager.py lines ~46-100). Crawl-delay is only enforced with this manager: "Crawl-delay directives from robots.txt will not be enforced" otherwise (`_basic_crawler.py` ~line 720).
- `respect_robots_txt_file=False` default (`_basic_crawler.py` line ~306). No `same_domain_delay` option found in the Python source (grep).

### spider-rs `spider` (2.53.9 / main 2.54.0)

- **Model:** one `Website` object crawls one site. Subdomains and TLD variants are opt-in via `with_subdomains` and `tld` (https://github.com/spider-rs/spider/blob/main/spider/src/configuration.rs). No cross-site frontier was found.
- `delay: u64` is the "Polite crawling delay in milli seconds", default 0. `Configuration` derives `Default`, so `respect_robots_txt` defaults to false. A robots.txt `Crawl-delay` overwrites `configuration.delay` for the whole website, capped at 60 000 ms (https://github.com/spider-rs/spider/blob/main/spider/src/website.rs ~2836-2840).
- **Concurrency:** a per-website `concurrency_limit` (None means CPU-derived `DEFAULT_PERMITS = calc_limits(1)`, at least 20-30). Under CPU or memory load the crawl swaps to a process-global semaphore `SEM_SHARED`. That swap ignores the per-website limit unless `SPIDER_RESPECT_CONCURRENCY_UNDER_LOAD=1` (https://github.com/spider-rs/spider/blob/main/spider/src/website.rs ~881-1045; https://github.com/spider-rs/spider/blob/main/spider/src/utils/mod.rs ~10040-10150).
- Inference: fairness between several `Website` crawls in one process rests only on contention for the shared tokio semaphore. No round-robin across sites was found.

### Colly (v2.3.0)

- `LimitRule{DomainRegexp|DomainGlob, Delay, RandomDelay, Parallelism}`. The first matching rule applies, matched on `request.URL.Host` (host:port). Each rule has a single `waitChan` of capacity `Parallelism`. So a rule like `DomainGlob: "*"` caps all matching hosts together, not each host separately (https://github.com/gocolly/colly/blob/master/http_backend.go lines ~52-160, 202-215).
- The delay is a `time.Sleep(Delay+random)` inside the deferred release, so the parallelism token is held during the delay (same file, `Do`).
- With no rules, `GetMatchingRule` returns nil and requests are not limited (same file).
- `IgnoreRobotsTxt = true` by default (`colly.go` `Init`, ~line 515).
- The optional `queue` package uses a single in-memory FIFO linked list with `MaxSize: 100000` (https://github.com/gocolly/colly/blob/master/queue/queue.go).

### Katana (v1.8.0)

- Default `-strategy depth-first` (also breadth-first), `-concurrency 10` ("concurrent fetchers"), `-parallelism 10` ("concurrent inputs"), `-rate-limit 150` req/s global, `-delay 0`. Opt-in `-host-rate-limit` / `-hrlm` per host (https://github.com/projectdiscovery/katana/blob/dev/cmd/katana/main.go lines ~145, 221-227; `pkg/types/crawler_options.go` ~32-35, 175-178).
- Each input (target) gets its own crawl queue and `Concurrency` workers. Up to `Parallelism` inputs run at once (`internal/runner/executer.go` line 37; `pkg/engine/common/base.go` ~398-406).
- Inference: this isolates targets but shares one global token bucket.

### Heritrix 3 (3.18.0)

- **Per-host work queues.** Default `SurtAuthorityQueueAssignmentPolicy`, `parallelQueues=1` (https://github.com/internetarchive/heritrix3/blob/master/engine/src/main/resources/org/archive/crawler/restlet/profile-crawler-beans.cxml ~615-630). "Only one URI per host is processed at a time" (https://heritrix.readthedocs.io/en/latest/configuring-jobs.html).
- **Politeness defaults:** `delayFactor=5.0` (multiple of the last fetch duration), `minDelayMs=3000`, `maxDelayMs=30000`, `respectCrawlDelayUpToSeconds=300`, `maxPerHostBandwidthUsageKbSec=0` (profile cxml ~409-413).
- **Queue rotation / budgeting:** `WorkQueueFrontier` "Round-robins between all queues" (https://github.com/internetarchive/heritrix3/blob/master/engine/src/main/java/org/archive/crawler/frontier/WorkQueueFrontier.java class doc).
  - Each queue gets a session budget, `balanceReplenishAmount` (default 3000 cost units per activation). When it is spent (`isOverSessionBudget`), the queue is deactivated to the back of the inactive queues, grouped by precedence. Exceeding `queueTotalBudget` (default -1, unlimited) retires it (same file ~439-485, 640-655; `WorkQueue.java` ~261).
  - Inactive queues are activated only when no ready queue exists (`findEligibleURI`, ~590-600).
  - Cost per URI comes from `costAssignmentPolicy`, default `UnitCostAssignmentPolicy` (profile cxml ~639).
  - The profile ships a `smallBudget` sheet (replenish 20, total 100) "causing it to have shorter 'active' periods while other queues are waiting" (cxml ~500-512).
  - Inference: this is quantum-based time-slicing, close in spirit to DRR with cost units.
- Snoozed queues sit in a `DelayQueue` by wake time. Queues are kept in BDB (Berkeley DB) on disk (`BdbFrontier`).

### Apache StormCrawler (3.7.0; main 4.0.0-SNAPSHOT)

- `crawler-default.yaml`: `partition.url.mode: byHost` (also `byDomain`, `byIP`), `fetcher.queue.mode: byHost`, `fetcher.threads.per.queue: 1`, `fetcher.server.delay: 1.0`, `fetcher.threads.number: 10`, `fetcher.max.urls.in.queues: -1`, `fetcher.max.queue.size: -1` (unbounded), `fetcher.max.crawl.delay: 30` (longer robots delays cause the queue's URLs to be skipped unless `.force`) (https://github.com/apache/stormcrawler/blob/main/core/src/main/resources/crawler-default.yaml lines 20-90).
- URLs are partitioned to bolts by host, so one host's politeness state lives in one place.
- `FetchItemQueues.getFetchItem()` on main polls a heap of queue "tickets" ordered by next allowed fetch time and returns an item from a queue "whose crawl delay has elapsed and which has a free slot" (https://github.com/apache/stormcrawler/blob/main/core/src/main/java/org/apache/stormcrawler/fetcher/FetchItemQueues.java ~290-340).
- Spout-side buffers: `SimpleURLBuffer` "rotates on the queues without applying any priority" (since 1.15). `PriorityURLBuffer` weights queues by acks per period (since 1.16). `SchedulingURLBuffer` releases based on recent fetch durations (https://github.com/apache/stormcrawler/tree/main/core/src/main/java/org/apache/stormcrawler/persistence/urlbuffer).

### Apache Nutch (1.23)

- **Generator (batch):** `-topN` caps the URLs selected for a segment, ordered by score. `generate.max.count` (default -1) caps URLs per host or domain per fetchlist, with counting by `generate.count.mode` (default `host`, or `domain`). Overflow spills to the next segment (https://github.com/apache/nutch/blob/master/conf/nutch-default.xml; https://github.com/apache/nutch/blob/master/src/java/org/apache/nutch/crawl/Generator.java ~560-600).
- Fetch lists are sorted by a URL hash "so that sorting by hashcode correlates less with by-host ordering" (`Generator.HashComparator`, ~671-700).
- **Fetcher:** `fetcher.queue.mode=byHost`, `fetcher.threads.per.queue=1`, `fetcher.server.delay=5.0`, `fetcher.threads.fetch=10`, `fetcher.max.crawl.delay=30` (nutch-default.xml).
- `FetchItemQueues.getFetchItem()` walks the queue map with a persistent iterator (round-robin) (https://github.com/apache/nutch/blob/master/src/java/org/apache/nutch/fetcher/FetchItemQueues.java ~180-210).
- `QueueFeeder` fills up to `threads x fetcher.queue.depth.multiplier` (10 x 50 = 500) items in total across all queues and "spin-wait[s]" when full. There is no per-queue cap in the feeder (https://github.com/apache/nutch/blob/master/src/java/org/apache/nutch/fetcher/QueueFeeder.java ~140-190; Fetcher.java ~246-250).
- Escape valves: `fetcher.timelimit.mins` and `fetcher.throughput.threshold.pages` exist "preventing slow queue's from stalling the throughput" (nutch-default.xml).
- Inference: one dominant host can fill the 500-item buffer and starve other hosts at the feeder. Nutch's mitigations are upstream: per-host caps in the generator and hash shuffling.

### Firecrawl (service, main)

- Fairness is keyed by tenant (team) and crawl, not by domain. Each team has a Redis sorted-set queue (`concurrency-limit-queue:<team>`), and each crawl has its own limiter (`crawl-concurrency-limiter:<crawl_id>`). On drain, jobs whose crawl is at its limit are skipped and put back (`crawlBlocked`), so they do not block other crawls (https://github.com/firecrawl/firecrawl/blob/main/apps/api/src/lib/concurrency-limit.ts ~33-60, 230-300).
- A full team queue raises `QueueFullError` with HTTP 429 (`queue-full-error.ts`). Inference: this is the analogue of `E_CAPACITY`.
- Crawl options: `maxConcurrency` (optional), `delay` (seconds, at most 60), `ignoreRobotsTxt=false` default (https://github.com/firecrawl/firecrawl/blob/main/apps/api/src/controllers/v2/types.ts ~1306-1390).

### Stagehand v4 (4.2.0)

- No request queue, crawl frontier, or per-domain concurrency appears in its packages (grep for requestqueue / per-domain / maxConcurrency in https://github.com/browserbase/stagehand, main). It drives browser sessions and leaves scheduling to the caller. This is a limited search: absence in the source, not a documented statement.

---

## 2. Literature: dominant hosts, head-of-line blocking, standard solutions

- **Mercator (Heydon & Najork 1999; Najork & Heydon 2001):**
  - F front queues implement priority and B back queues implement politeness. Each back queue "only contains URLs from a single host". A heap keyed by each back queue's earliest allowed time `t_e` picks the next host.
  - When a back queue empties, it is refilled from the front queues. URLs for hosts that already have a back queue are moved there.
  - Gap rule: "ten times the last fetch time". Sizing: "three times as many back queues as crawler threads".
  - Sources: Manning, Raghavan & Schütze, *Introduction to IR*, §20.2.3, https://nlp.stanford.edu/IR-book/html/htmledition/the-url-frontier-1.html. Patent US 6,321,265, "System and method for enforcing politeness while scheduling downloads in a web crawler", https://image-ppubs.uspto.gov/dirsearch-public/print/downloadPdf/6321265.
  - Inference: a dominant host occupies one back queue. Fairness among ready hosts is earliest-eligible-time, which works out to round-robin when delays are equal.
- **IRLbot (Lee, Leonard, Wang, Loguinov, ACM TWEB 3(3), 2009; WWW 2008):**
  - Names the dominant-host problem directly: preventing the crawler "from 'choking' when its entire RAM gets filled up with URLs pending for a small set of hosts" (§1.3). It also notes that "multimillion-page sites (both spam and legitimate) … became backlogged in politeness rate-limiting to the point of overflowing the RAM" (§1.4).
  - Solution: STAR assigns a per-PLD budget from in-degree out of other PLDs. Per-host delay `τh` is 40 s for low-ranked PLDs and scaled down in proportion to budget. A per-server (IP) delay `τs` defaults to 1 s.
  - The IP heap and the host heap enforce the two delays. "By properly controlling the coupling between budgets and crawl delays … the rate at which pages are admitted into RAM is no less than their crawl rate, which results in no memory backlog" (§7.1).
  - BEAST delays over-budget URLs into exponentially rechecked disk queues. It states that "low-ranked domains should be crawled in some approximately round-robin fashion" (§7.2).
  - IRLbot chose PLD (pay-level domain) over IP as the budget key because spammers must pay for PLDs (§6).
  - Source: https://irl.cse.tamu.edu/people/hsin-tsang/papers/tweb2009.pdf
- **BUbiNG (Boldi, Marino, Santini, Vigna; arXiv:1601.06919, 2016):**
  - A per-host *visit state* (FIFO + next-fetch) is grouped into per-IP *workbench entries*. The workbench is "a priority queue of priority queues of FIFO queues" ordered by next-fetch time, enforcing host and IP politeness. It finds a ready host in constant time and dequeues in log time (§4.2).
  - Memory is bounded "only in the number of hosts discovered". The workbench virtualizer spills per-host FIFO queues to disk.
  - It warns that "filling the workbench blindly with URLs coming out of the sieve would soon result in having in the workbench only URLs belonging to a limited number of hosts" (§4.6-4.7).
  - The distributor grows the "required front" (the number of ready hosts) whenever a fetch thread has to wait (§4.7).
  - Source: https://arxiv.org/abs/1601.06919. The published SP&E 2018 version was not checked.
- **DRR / WFQ:** Deficit Round Robin gives near-perfect throughput fairness at O(1) work per packet, against O(log n) for WFQ-class schemes (Shreedhar & Varghese, SIGCOMM 1995, doi:10.1145/217391.217453; https://en.wikipedia.org/wiki/Deficit_round_robin; https://people.cs.rutgers.edu/~sn624/552-S24/papers/drr.pdf). I found no crawler paper or major open-source crawler that uses DRR or WFQ by name (WebSearch, no hits).
- **Is round-robin the standard?** Fact: every frontier examined uses per-host (or per-host/IP) queues.
  - Selection policies:
    - Round-robin: Heritrix ("Round-robins between all queues"), Nutch (round-robin iterator), StormCrawler `SimpleURLBuffer` (rotation).
    - Earliest-eligible-time heaps: Mercator, BUbiNG, IRLbot, StormCrawler main.
    - Least-active-first with round-robin ties: Scrapy DAPQ.
    - Longest-waiting first: Crawlee ThrottlingRequestManager.
  - Large-scale crawlers add **budgets** on top: Heritrix session and total budgets, IRLbot STAR/BEAST per PLD, Nutch `generate.max.count` per host or domain.
  - Inference: "round-robin or earliest-eligible over per-host queues, plus a per-host budget or quantum" is the common pattern. DRR would be a formalisation of Heritrix's replenish budget. No source shows it beating these in a crawler.

---

## 3. Known pitfalls

1. **Memory and state growth with many hosts.**
   - BUbiNG designs memory to grow with the number of hosts and spills URLs to disk (§4.6).
   - IRLbot hit RAM overflow from backlogged hosts (§1.3-1.4).
   - Scrapy DAPQ with `JOBDIR` produced about 800 000 per-slot sub-directories on a 3M-URL crawl (https://github.com/scrapy/scrapy/issues/5275). Reclaiming drained slot directories was only added in 2.18.0 (gh 5275/7955, https://github.com/scrapy/scrapy/blob/master/docs/news.rst).
   - Scrapy's downloader garbage-collects idle slots (`_start_slot_gc`, downloader source).
   - Crawlee Python: per-domain sub-queues left by a previous run were invisible on resume, so the crawl "reported itself finished" (https://github.com/apify/crawlee-python/pull/2177, merged 2026-08-21). Relevant to journaled admission.
2. **Starvation and priority inversion.**
   - DAPQ ties always picked the same slot, "causing starvation of other domains" (https://github.com/scrapy/scrapy/issues/7293). Fixed with round-robin tie-breaking in 2.15.0.
   - Once a request reaches a Scrapy slot deque, "its priority is no longer respected", so a high-priority request can wait behind 9 low-priority ones (https://github.com/scrapy/scrapy/issues/1371, open since 2015).
   - In DAPQ, priority only orders requests within a slot. Slot choice ignores priority (pqueues.py). Inference: a globally high-priority request on a busy host waits behind other hosts' turns.
   - Mercator handles the conflict with priority front queues feeding politeness back queues (IR book §20.2.3).
   - Heritrix uses `queuePrecedencePolicy` across inactive queues and URI precedence inside queues (profile cxml).
3. **Interaction with politeness delays and robots.txt Crawl-delay.**
   - RFC 9309 does not define `Crawl-delay`; crawlers "MAY interpret other records" (https://www.rfc-editor.org/rfc/rfc9309.html §2.2.4).
   - Robots rules are cached for at most 24 h (§2.4) and are scoped to `scheme://authority` (§2.3).
   - Scrapy ignores Crawl-delay (optimize.rst). Crawlee enforces it only through ThrottlingRequestManager.
   - spider-rs applies one Crawl-delay per website, capped at 60 s.
   - Heritrix respects it up to 300 s. Nutch and StormCrawler skip queues whose delay exceeds 30 s unless forced. Nutch ignores Crawl-delay when `threads.per.queue > 1` (nutch-default.xml).
   - Colly holds the parallelism token while sleeping (http_backend.go).
   - Inference: a fair-share scheduler has to treat a delayed host as not ready rather than give it a turn. Crawlee v4 returns `null` instead of holding a slot. Otherwise delayed hosts consume global capacity, which is the Scrapy #6982 failure.
4. **Choice of key: host, registrable domain, or IP.**
   - Scrapy keys by hostname (IP option deprecated in 2.14). Colly matches on `URL.Host`.
   - Crawlee v3 `sameDomainDelaySecs` keys by `tldts.getDomain()`. Its default `allowPrivateDomains: false` maps `foo.github.io` to `github.io` (checked with tldts 7.4.18: `getDomain('foo.github.io') = 'github.io'`). So every GitHub Pages site shares one clock.
   - Public Suffix List private entries such as `github.io`, `blogspot.com`, `herokuapp.com`, `vercel.app`, `pages.dev`, `cloudfront.net`, `s3.amazonaws.com` sit below `===BEGIN PRIVATE DOMAINS===` (https://publicsuffix.org/list/public_suffix_list.dat, line 11247 onward).
   - Crawlee v4 ThrottlingRequestManager defaults to `hostname` when built directly but uses `registrableDomain` for `sameDomainDelaySecs`. IPs and single-label hosts fall back to the hostname.
   - Nutch, StormCrawler and BUbiNG offer `byHost`, `byDomain`, `byIP`. BUbiNG enforces both host and IP politeness.
   - IRLbot found per-host delays alone crashed shared hosting with "thousands of virtual hosts on the same physical server", so it added a per-IP delay (§7.1). It keyed budgets by PLD to resist spam farms of many hosts (§6).
5. **Global priority versus per-domain fairness.** See item 2. No surveyed system offers both a strict global priority order and per-host fairness. Each picks one as primary: Mercator puts priority first and fairness inside politeness, Scrapy DAPQ puts fairness first and priority within a slot (sources above).

---

## 4. Is a global FIFO acceptable or preferred for single-site crawls?

- Scrapy states its "default settings are optimized for crawls targeting a single website" (optimize.rst). For 11 years (until 2.14.0, Jan 2026) its default was the slot-unaware `ScrapyPriorityQueue`, which was LIFO inside each priority level (news.rst).
- Crawlee JS 3.x and Python 1.x ship a single global FIFO `RequestQueue` with global autoscaling. Per-domain pacing is opt-in, and maintainers recommend one crawler per domain for different per-domain limits (crawlee-python#1263).
- spider-rs is built around one site per `Website` (configuration.rs). Colly has no limit by default.
- Inference: with one host, per-host queues reduce to a single queue served in the same order, so fair admission costs nothing in ordering. The remaining costs are a per-key lookup and, under a registrable-domain key, merging subdomains onto one clock.
- No usage data was found on the share of Crawlee or Scrapy runs that target a single site. The claim "most runs target one site" is unsourced.
- Counter-evidence for even "mostly single-site" crawls: Scrapy #6982 shows HOL blocking at the global cap with only two slots (a proxied slot and a hard-coded slot). Scrapy's maintainers fixed it by making the per-slot queue the default.

---

## Comparison table

| System (version) | Queue structure across domains | Key | Per-key concurrency default | Per-key delay default | Cross-key selection |
|---|---|---|---|---|---|
| Scrapy 2.19 | Per-slot priority queues (DAPQ, default since 2.14) | hostname | 8 (template: 1) | 0 s (template: 1 s), AutoThrottle off | least active, round-robin ties |
| Crawlee JS 3.18 | Single global FIFO RequestQueue | registrable domain (delay only) | none | 0 s (`sameDomainDelaySecs`) | none (FIFO) |
| Crawlee JS 4.0 rc | Global FIFO; per-domain sub-queues only if throttling is on | hostname or registrable domain | none | 0 s; 429 backoff and Crawl-delay when manager is used | longest-waiting |
| Crawlee Py 1.10 | Global FIFO; opt-in ThrottlingRequestManager for listed domains | listed domains | none | 429 / Crawl-delay only | longest-waiting |
| spider-rs 2.53 | One crawl per site; process-wide semaphore under load | site | CPU-derived (≥20) | 0 ms, robots Crawl-delay ≤60 s | none |
| Colly 2.3 | Single FIFO (optional queue); LimitRule per pattern | host glob/regex | unlimited (no rule) | 0 | none |
| Katana 1.8 | Queue per input target | input | 10 per input | 0; global 150 rps | parallel inputs |
| Heritrix 3.18 | Per-host work queues (BDB) | SURT authority | 1 | 3-30 s (5x fetch time) | round-robin with session budget 3000 |
| StormCrawler 3.7 | Partition by host; per-host fetch queues | host | 1 | 1 s | earliest-due heap (main); rotation buffer |
| Nutch 1.23 | Generator per-host caps; per-host fetch queues | host | 1 | 5 s | round-robin iterator |
| Firecrawl | Per-team and per-crawl limits | tenant / crawl | per plan | `delay` optional | skip blocked crawls |
| Stagehand 4.2 | none | n/a | n/a | n/a | n/a |
