# Every Rust competitor to Foxhound and Collie, and the fundamentals they set (2026-11-07)

Read-only research for the engine refactor (specs #2138, #2139, #2164, #2165, #2194, #1838), whose mandate is to beat the reputed Rust crawlers on throughput, CPU and instructions per page, memory and start-up, at equal features. It complements [the Spider gap note](2026-11-06-spider-gap.md) and [the frontier persistence note](2026-11-06-crawler-frontier-persistence.md) and does not redo them.

**Method.** Discovery swept GitHub (repository search with `language:rust` over 45 queries and 12 topics, about 1,800 repositories), crates.io (14 keywords sorted by downloads, 337 crates), awesome-lists and Exa web search, then each organisation's own repositories. About 150 projects were triaged on health. The ones that matter were cloned shallow at a pinned commit and read with grep and sed. Nothing was built or run, except one size probe of `url::Url` written for this note. Every `file:line` cite is at the SHA or version its section states. Health comes from `gh api` (2026 commit counts summed over `commits?since=2026-01-01`, the last commit date, the latest release) and the crates.io, npm and PyPI download APIs, all on 2026-11-07. Benchmark figures are as published, with their method, and none was reproduced.

**Limits.**
- The token could not list stargazer timestamps (REST 403, GraphQL `FORBIDDEN`). Star plausibility is judged from forks, issues, PRs, contributors and downloads per star, compared with organic peers.
- Several 2026 projects are machine-generated at extreme speed: hundreds of thousands of lines in weeks, or about 100 commits a day. Each is flagged where it appears. Their claims are read as claims.
- Vendor benchmarks are marked as such. Cloud engines are assumed to match their open-source engines only on the vendor's word.

## Answer in one paragraph

**Spider is still the only reputed, maintained, general-purpose Rust crawler.** The other 2026 contenders are AI-era "Firecrawl alternatives" or SEO tools, mostly young and in-memory:

| Project | What it is | Where it stands |
|---|---|---|
| fastCRW (`crw`) | Firecrawl alternative | 1.2k stars, real downloads; its crawl loop is sequential per job |
| crawlberg | crawl core with 14 language bindings | 38k crate downloads in 90 days, against Spider's 51k; spawns one task per page |
| webclaw | extraction CLI and crawler | 2.4k stars |
| DonSeTch | fetch/search/crawl tool for agents | serial by default |
| pagesift | crawler and document parser | 18 days old, machine-generated; claims to beat Spider on multi-host throughput and memory |
| OpenSpider | SEO crawler | beta, built by agents; the only other crawler with a SQLite durable crawl file like Foxhound's |
| crawlee-rs | Apify's port of Crawlee to Rust | early, no release |

The classic Rust crawlers are dead or archived: crusty since 2021, Stract and Spyglass archived. On the Collie side, chromiumoxide is the most used CDP client but has had no commit since April. Spider's fork chromey carries the transport improvements. agent-browser (46.6k stars) is the leading agent-facing Rust CLI. obscura and moli are real Rust browser engines that serve CDP. None of the Rust browser clients launches Chrome over a pipe or recycles a browser after N pages, so #2139 has no Rust reference to beat there.

Read from source, **Spider's speed has three causes**:
- **No hand-off between threads:** one task per page under a semaphore does the fetch, the lol_html pass on the arriving chunks, the link set and the broadcast.
- **A small visited set:** interned in memory, about URL length plus 22–42 B per URL.
- **No durable state:** nothing is persisted at all.

What Spider gives up for that:
- no resumable state;
- robots.txt off by default;
- no per-host politeness;
- crawls in BFS waves with a barrier: in-flight requests drain to zero between waves;
- a broadcast channel that drops pages for slow consumers and keeps up to its capacity in memory.

Our own `xtask/compare-spider` subscribes with a capacity of 4,096 (`xtask/compare-spider/src/main.rs:127`), which likely inflates Spider's measured memory as pages grow.

The techniques worth taking come mostly from the young engines (pagesift, OpenSpider, crw, webclaw), from lychee and Stract for politeness, and from the building blocks (lol-html, reqwest, mimalloc v3, adblock-rust). They are listed by spec [below](#techniques-to-adopt-by-refactor-spec).

## Summary table: competitors to Foxhound and Collie

Feature letters, in order: **R**obots.txt, **P**oliteness (per-host delay or concurrency), **D**urable resumable crawl, **S**itemaps, **B**rowser rendering, **E**xtraction (main text or Markdown), proXies (**X**), sEssions or cookies (**C**). Upper case: present; lower case: partial; `-`: absent. "2026" is commits since 2026-01-01 / last commit. The building blocks have their own table in [Part C](#part-c-building-blocks).

| Project | Rivals | Kind | Licence | Stars | Last release | 2026 / last commit | Maintained | Features | Fundamentals in one line | Depth |
|---|---|---|---|---|---|---|---|---|---|---|
| [spider](#spider-crate-spider-2539) | Foxhound | crawler library + CLI | MIT | 2,835 | 2.53.9 (2026-09-05) | 842 / 2026-11-06 | yes (1 maintainer) | r p - S B e X C | task per page under a semaphore; lol_html on chunks; interned visited set ≈ URL + 22–42 B; BFS waves; no persistence; glibc | deep |
| [Spider Cloud](#spider-cloud-spidercloud) | Foxhound | paid API on spider + closed browser | proprietary | – | – | active | yes | R ? - S B E X C | claims only (100K+ pages/s, 74 pages/s average in its own test) | claims |
| [pagesift](#pagesift) | Foxhound | crawler, Firecrawl/Jina-compatible server | AGPL-3.0 + commercial | 251 (anomalous) | v1.2.0 (2026-11-07) | 617 since 2026-10-20 | very young, machine-generated | R P D S B E X C | no spawn per page; `HashSet<u64>` 10–18 B/URL; queue capped at the page limit; append-only enqueue log; own small-object allocator; zero-copy parser | deep |
| [crawlee-rs](#crawlee-rs-and-crawlee-storage-apify) | Foxhound | Apify's Rust port of Crawlee | Apache-2.0 | 1 | none | 33 / 2026-11-03 | early | R P D S - E X C | lol_html link pass, DOM built lazily once; fs storage; publishes 1.32 ms CPU/page | medium |
| [fastCRW (crw)](#fastcrw-crw) | Foxhound | Firecrawl-compatible server + cloud | AGPL-3.0 + commercial | 1,234 | v0.42.1 (2026-11-06) | 1,264 / 2026-11-07 | yes | R P - S B E X C | per-job crawl loop sequential (concurrency 1); all results in memory; browser context per navigation | deep |
| [crawlberg](#crawlberg-formerly-kreuzcrawl) | Foxhound | crawl core + 14 bindings | MIT (extras closed) | 195 | v1.13.0 (2026-11-06) | 2,327 / 2026-11-07 | yes (very high churn) | R P - S B E X C | `JoinSet` spawn per fetch; in-memory frontier; trait seams | medium |
| [webclaw](#webclaw) | Foxhound | extraction CLI/MCP/REST + crawler + cloud | AGPL-3.0 | 2,435 (mild anomaly) | v0.7.2 (2026-11-04) | 932 / 2026-11-04 | yes | R p d S B E X C | spawn per page; ≈ 100 B/URL; whole state as JSON every 1 s; renders from the HTTP body already fetched | deep |
| [DonSeTch](#donsetch) | Foxhound | fetch/search/crawl for agents | AGPL-3.0 | 975 | v4.10.5 (2026-11-03) | 1,160 / 2026-11-07 | yes | R P D S - E X C | serial by default; latency-EWMA governor per (host, proxy) | deep |
| [OpenSpider](#openspider-wojcikdominik) | Foxhound | SEO crawler (Tauri, CLI) | Apache-2.0 | 31 | v1.0.0-beta.2 (2026-11-04) | 2,479 since 2026-10-13 | yes, agent-built | R P D S B E X C | SQLite crawl file; seen set 21 B/URL; group commit sized in bytes; byte gate on in-flight pages | medium |
| [SiteOne Crawler](#siteone-crawler-janreges) | Foxhound | SEO/QA crawler (PHP → Rust) | MIT | 998 | v2.5.1 (2026-06-27) | ? / 2026-09-27 | yes | R p - S B E X c | spawn per URL; 10 ms polling; ≈ 200 B/URL in `DashMap`s | medium |
| [crawlie](#crawlie-spronta) | Foxhound | SEO + GEO crawler | MIT | 117 | v0.5.3 (2026-06-29) | 169 / 2026-10-26 | yes | R p d S B e ? ? | `FuturesUnordered`; optional SQLite page store | medium |
| [millipede](#millipede-satvik007--and-its-third-party-spider-benchmark) | Foxhound | Crawlee-like library | MIT/Apache-2.0 | 1 | crate 0.1.1 (2026-07-19) | 98 / 2026-07-19 | quiet | R P D S B e X C | full DOM; **the only third-party Spider benchmark with a method** | medium |
| [spire](#spire-spire-rs) | Foxhound | crawler framework | MIT | 6 | v0.11.0 (2026-11-06) | 75 / 2026-11-06 | yes (tiny) | R P D S B e ? ? | u64 fingerprints; Bloom 1.2 B/URL; latency-based per-host concurrency | medium |
| [lychee](#lychee-lycheeverse) | (Foxhound) | link checker, not recursive | Apache-2.0/MIT | 4,102 | v0.24.2 (2026-05-01) | 283 / 2026-11-02 | yes | - P d - - - ? C | per-host semaphore + token bucket + header-driven backoff + coalescing | deep |
| [crusty](#crusty--crusty-core-let4be) | Foxhound | broad crawler | GPL-3.0 | 88 | 0.12.0 (2021-06-06) | 0 / 2021-11-27 | **no** | R P - - - - - - | parser thread pool; DNS once per site; politeness per IP /24 | deep |
| [Stract](#stract-stractorgstract--crawler-part) | Foxhound | search engine + crawler | AGPL-3.0 | 2,372 | none | 0 / 2025-03-24 | **archived** | R P d S - E - - | delay = fetch time × 2^k; job file + read pointer; jemalloc | deep |
| [scrapix-v2](#short-reads-mediumtriaged) | Foxhound | Meilisearch's distributed crawler | undeclared | 5 | none | 282 / 2026-05-22 | quiet | R P D S B E X ? | Kafka frontier, RocksDB, Bloom dedup, DNS cache | triaged+ |
| [feroxbuster](#feroxbuster-epi052) | (recon) | recursive content discovery | MIT | 8,155 | v2.13.1 (2025-12-13) | 21 / 2026-11-07 | yes | r P D - - e X C | binary search for the highest error-free rate | medium |
| [servo-fetch](#servo-fetch-read-in-part) | Foxhound/Collie | Servo fetch + render + extract | MIT/Apache-2.0 | 163 | v0.15.2 (2026-11-01) | 505 / 2026-11-07 | yes | R p - s B E ? c | in-process Servo, 51–64 MB peak per its README | triaged+ |
| [chromiumoxide](#chromiumoxide-mattsse--afcc3a4313f2087249b4490d94e54bf8e3bfaccf) | Collie | CDP client | MIT/Apache-2.0 | 1,432 | 0.9.1 (2026-02-25) | 36 / 2026-04-03 | slowing | n/a | one flush per command; `Value` double serialization; central loop O(targets) per wake | deep |
| [chromey](#chromey-spider-rs--fork-delta-vs-chromiumoxide-only--0312d624ea2feb4dfa77699213f1d7ca388358ac) | Collie | Spider's chromiumoxide fork | MIT/Apache-2.0 | 52 | 2.58.2 (2026-09-12) | 166 / 2026-10-18 | yes | n/a | 128 commands per flush; sonic-rs; large frames off-thread; opt-in task per page; tab closer drop guard | deep |
| [agent-browser](#vercel-labsagent-browser--d44f6989bf5e9a9a1c6dd6d97175f8c33b138eca) | Collie | agent CLI + per-session daemon | Apache-2.0 | 46,638 | v0.41.2 (2026-11-03) | 816 / 2026-11-06 | yes | n/a | warm daemon over a Unix socket; `DevToolsActivePort`; lossless per-method taps | deep |
| [headless_chrome](#rust-headless-chrome-headless_chrome--transport-only--0a5c307a85debc450378a1f19e4dac1838d7b22d) | Collie | sync CDP client | MIT | 2,958 | 1.0.22 (2026-06-11) | 6 / 2026-06-11 | barely | n/a | OS threads; deprecated `sendMessageToTarget` (double encoding) | deep (transport) |
| [fantoccini / thirtyfour](#fantoccini--thirtyfour-webdriver--911871416ebc35818e62910cbd9f02e20a05f229--24f9114266465fa9f9cba8a2b47649d6b8dff446) | Collie | WebDriver (+ CDP, BiDi) clients | MIT/Apache-2.0 | 2,033 / 1,443 | 0.22.1 / v0.38.1 (2026-10-21) | 32 / 183 | yes | n/a | HTTP round trip per action; thirtyfour adds BiDi | deep (transport) |
| [obscura](#obscura--d21de16d4c0f10548a087e00efbc0b8f81b23d8c-headless-engine) | Collie (engine) | Rust browser engine serving CDP | Apache-2.0 | 30,511 (proportions organic) | v0.2.6 (2026-10-31) | 1,668 / 2026-11-07 | yes | n/a | V8 via deno_core, own DOM; glibc arenas capped at 2 | deep |
| [moli](#moli--c9140e76ffb87979acc37041734726e20eef895d-headless-engine) | Collie (engine) | Rust browser engine (CDP, WebDriver, BiDi) | Apache-2.0/MIT | 22,923 (suspicious) | v1.1.22 (2026-11-06) | 3,010 / 2026-11-07 | yes (one author) | n/a | rusty_v8 + Stylo + Taffy; jemalloc process-wide `narenas:1` | deep |
| [stygian-browser](#stygian-browser--pool-only--b94ae0e4dbd5d0f3b8845ed3effeba078e5abd43) | Collie | browser pool on chromiumoxide | AGPL-3.0 + commercial | 15 | v0.17.0 (2026-08-24) | 474 / 2026-08-25 | slowing | n/a | pool of browsers: min/max, idle eviction, health check on return | pool only |
| [playwright-rs](#part-b-competitors-to-collie-and-rust-headless-browser-engines) | Collie | bindings to the Node Playwright server | Apache-2.0 | 167 | 0.20.1 (2026-10-17) | 563 / 2026-11-07 | yes | n/a | Node driver underneath | triaged |
| Lightpanda | (out of scope) | **Zig** headless engine | AGPL-3.0 | 41,556 | 1.0.3 (2026-11-05) | 8,457 / 2026-11-07 | yes | n/a | reference only: 123 MB / 5 s vs Chrome 2 GB / 46 s for 100 pages (its README) | reference |

The per-cluster tables in Parts A to C list every other project triaged (about 120 more), with the same columns.

## Projects worth adding to `just compare` beside Spider

`just compare` has five scenarios (`xtask/src/compare/matrix.rs`): raw, durable, polite, extraction and browser. A candidate is worth a row when it is a crawler people would choose instead of Foxhound, and when it can fill cells honestly. Ranked:

1. **pagesift** (pin a commit).
   - **Why add it:** it is the only Rust engine that publishes multi-host crawl throughput and memory against Spider 2.53.9: 3,500–4,400 pages/s at 280–316 MB against Spider's 2,000–2,700 pages/s at 5.5 GB (`docs/benchmarks.md`). It can fill all five scenarios, robots, per-host politeness, an append-only resume log and a CDP browser ladder included. It ships a headless `crawl_bench` driver.
   - **Caveats:** the repository is 18 days old, about 313k lines that look machine-generated, with 1 watcher against 251 stars. Its Spider arm keeps a 256-page broadcast buffer per site across 200 sites, which may explain much of Spider's 5.5 GB.
   - **What it would prove:** whether Foxhound beats the newest claimant on the multi-host, durable and polite cells.
2. **crawlee-rs** (Apify; pin a commit, since it has no release).
   - **Why add it:** Crawlee is already a compare arm, and this is Apify's own Rust answer. It publishes a CPU-per-page figure of the same kind as our instructions per page: 1.32 ms against 13.46 ms for Crawlee JS Cheerio, at 2,326 pages/s and 120 MiB, on its own 4 vCPU fixture. It has robots, sitemaps, throttling, a session pool and durable fs storage, so it can fill raw, durable, polite and extraction.
   - **Related check:** our Crawlee JS arm pins 3.18.2 (`xtask/compare-crawlee/package.json`). Crawlee JS 4.0 beta's storage is already Rust (`@crawlee/fs-storage-native`), so the arm must stay on v3 to remain a JS baseline, or say which one it measures.
3. **fastCRW (crw).**
   - **Why add it:** it is the best-known Rust "Firecrawl alternative", with organic engagement (16 contributors, 589 PRs, PyPI and npm downloads) and a hosted product. It is the one users name.
   - **Expected result:** it should lose on throughput, because its per-job crawl loop awaits each fetch inline (`crates/crw-crawl/src/crawl.rs:356-470`). Showing that costs little and is credible.
   - **Cells it fills:** raw, polite, extraction and browser; no durable cell. It needs a server plus job polling, as pagesift's harness does.
4. **crawlberg.**
   - **Why add it:** it is the most-downloaded Rust crawl crate after spider (38k against 51k in the last 90 days), with Python, Node and 12 other bindings.
   - **Cells it fills:** raw, polite, extraction and browser, from an in-memory engine that spawns one task per fetch. It gives ecosystem coverage more than a high bar.
5. **OpenSpider**, once it leaves beta.
   - **Why add it:** it is the only other Rust crawler with a SQLite crawl file, resume, per-host additive-increase, multiplicative-decrease throttling and its own 1M-URL load measurements. It is therefore the one honest opponent for the **durable** cell, where Spider has no answer.
   - **Caveat:** it is agent-built at about 100 commits a day, and macOS/Windows first.

Not rows, but worth measuring beside the matrix:
- **obscura**, then **moli**, as alternative rendering engines behind Collie's CDP in the browser scenario.
- **libreadability**, **Resiliparse 2.x**, **rs-trafilatura** and **astral-html** as micro-benchmarks behind the extraction and raw cells.

Not candidates:
- the Spider bindings and Spider Cloud (paid);
- webclaw (a single-site, quality-oriented CLI);
- DonSeTch (serial by default);
- SiteOne Crawler (audits on every page, a weak engine);
- lychee (not recursive);
- crusty and Stract (dead or archived);
- every CDP client library (no crawler).

**Harness fixes these readings suggest for the existing Spider arm:**
- **Buffer capacity:** `subscribe(4096)` plus the idle receiver that `Website` keeps can hold up to 4,096 pages alive (`spider/src/website.rs:14676-14690` @ v2.53.9; tokio `sync/broadcast.rs:1766-1773`). Measure Spider at a small capacity too, so its memory figure is not our artefact.
- **Feature rows:** consider extra, labelled Spider rows with its default features (io_uring, which the arm turns off), with `bloom`, and with `smart` for the browser cell. Label them as sensitivity rows.
- **Harness design to borrow:**
  - from millipede (`benchmarks/PLAN.md`): exact count, byte and checksum gates, a fresh process per trial, and a raw-client ceiling row;
  - from pagesift: a multi-host local origin, the peak PSS of the whole process tree, and a site simulator that serves 429s and URL traps.

## Techniques to adopt, by refactor spec

Each line names the source that proves it works. "Avoid" lines record a mistake a competitor makes, so Foxhound does not copy it.

### #2138 Run state: memory-first, written in batches

| Technique | Source |
|---|---|
| Dedup on 64-bit fingerprints in hashbrown, about 10–20 B/URL, with the full URL only in the Run file. The collision probability is about n²/2⁶⁵ (2.7×10⁻⁶ at 10M URLs). | pagesift `pagesift-engine/src/crawl/mod.rs:93-99` @ 529f03ae; spire `frontier/seed/fingerprint.rs:4-10`; computed in Part C |
| Dense, id-indexed seen columns (64-bit key hash + 32-bit FNV fingerprint + 4 B depth/flags, about 21 B/URL), with an overflow set for true collisions and SQLite `UNIQUE` as the truth. The measured cost of the alternative: sharded hash maps added 80–160 MB of peak RSS at 250k–500k pages. | OpenSpider `crates/os-engine/src/seen.rs:1-26` @ 6d879a26 |
| Intern visited URLs (8-byte symbols over one string buffer, about URL + 22–42 B) when the URL text must stay in memory. | spider `utils/interner.rs:29-90` @ e4b53c5b |
| Stop recording links that can never be fetched: with a page budget, enqueue nothing once queued + scheduled ≥ limit, so memory and writes do not grow with discovered links. | pagesift `runner/mod.rs:58-60` |
| Group commit sized by bytes that grows under backlog: 200 pages or 100 ms normally, up to 4,000 pages, 8 MiB or 500 ms. Measured: 42 % less writer time at 1,000 pages per commit, 71 % less at 5,000. | OpenSpider `writer_thread.rs:1-30,47-73` |
| Keep a bounded RAM window of the queue head over the on-disk ordered queue, with a watermark so only already-persisted entries leave RAM. | OpenSpider `queue.rs:1-40`, `queue_disk.rs` |
| Write "new URLs" before the page that found them, in one ordered step, so a resume never meets a page whose links are missing. | OpenSpider `worker.rs` header |
| Append-only log of newly enqueued URLs; on resume, re-mark them seen and re-queue the unfinished ones in order. Cheapest durable plan file: Stract's append-only job file + persisted read pointer. **Avoid** rewriting the whole state on a timer (webclaw: whole JSON every 1 s). | pagesift `pagesift-api/src/jobs/sink.rs:326-340`; Stract `crawler/file_queue.rs:31-182`; webclaw `crawler.rs:365-400` |
| Optional Bloom or binary-fuse tier only as a pre-check before a disk lookup (1.2 B/URL at 1 %); an FST for sealed tiers (16.4 B/URL on 1.65 billion Common Crawl URLs). | spider `utils/bloom.rs` (opt-in `bloom` feature, off by default); spire `seen/bloom.rs:11-29`; BurntSushi `fst` blog |
| Demote URL templates that keep yielding duplicate content to a low tier; learn rewrite rules (`?print=1`, `index.html`) from duplicates. pagesift's own trap benchmark shows waste falling from about 0.4 to 0.12. | pagesift `runner/mod.rs:62-64,143-153`, `pagesift-frontier/src/frontier/learn.rs` |
| Conditional revalidation cache (ETag / Last-Modified) for recrawls. | crawlberg `src/traits.rs:288-305` |

### #2139 Browser pool: grows with concurrency, recycles after warm-up

| Technique | Source |
|---|---|
| Long-lived CDP connections with a fresh browser context per navigation, 1–2 s create/dispose/close timeouts, health check after 60 s idle. | crw `crw-renderer/src/browser_pool.rs:1-13,207-236` @ 4feae45b |
| Pre-warm `min_size` browsers, evict idle ones down to `min_size` on a timer, health check on return. No Rust pool recycles after N pages, so #2139 must add that itself. | stygian `crates/stygian-browser/src/pool.rs:198-334` |
| Close tabs from a dedicated closer fed by a drop guard, so a cancelled navigation can never leak a target. | spider `features/chrome.rs:1933-2160` |
| Batch CDP writes (up to 128 commands per flush), bounded writer and reader channels. **Avoid** chromiumoxide's one flush per command. | chromey `src/conn.rs:490-532,873-912` @ 0312d624; chromiumoxide `src/conn.rs:86-101` @ afcc3a43 |
| Route by session (one task per page behind a router, bounded inboxes spilling to a bounded FIFO) instead of a central loop that polls every target on each wake. Flatten-mode auto-attach only; never `sendMessageToTarget`. | chromey `src/handler/parallel/mod.rs:1-25`; chromiumoxide `src/handler/mod.rs:527-650`; headless_chrome `transport/mod.rs:140-151` |
| Find the DevTools port from `DevToolsActivePort` in the user-data dir (delete a stale one first) instead of scraping stderr. Nobody uses `--remote-debugging-pipe`: a chance to measure it. | agent-browser `cli/src/native/cdp/chrome.rs:810-812,888-975` @ d44f6989 |
| Lossless per-method taps for events that need a reply (`Fetch.requestPaused`, auth) beside a lossy broadcast for the rest. | agent-browser `client.rs:33-46` |
| Feed the already-fetched HTTP response into Chrome with `Fetch.fulfillRequest`, so a rendered page costs only its subresources. | webclaw `render.rs:952-963,1460` @ 6226b2a8 |
| Learn per host whether rendering helps: after 3 renders, always render if it helped in at least 80 % of cases, never if in at most 20 %. Variants: crw's failure-window preference with a LightPanda/Chrome hedge; Firecrawl's engpicker (Levenshtein agreement of TLS-client and CDP Markdown); Spider's `smart` scoring of framework markers. | webclaw `render.rs:2213-2222,2388-2398`; crw `crw-renderer/src/preference.rs`; Firecrawl `apps/api/native/src/engpicker.rs:67-160`; spider `page.rs:8406-8635` |
| Block resources at `Fetch.requestPaused` inside the client (images, media, fonts, CSS, adblock lists). | chromey `src/handler/network.rs:248-371` |
| Launch the browser lazily on the first render. | webclaw `render.rs:1775-1782`; spider `OnceBrowser` |

### #2164 Extraction path

| Technique | Source |
|---|---|
| Feed lol_html each body chunk as it arrives, into one buffer pre-sized from Content-Length. | spider `utils/mod.rs:6486-6571`, `page.rs:5204-5215` |
| Parse lol_html selectors once per Run and lend them (`Cow::Borrowed`); the `element!` macro re-parses its selector on every page. Never register `*` or attribute-only selectors: they keep the full lexer on for the whole page. Spider rebuilds its handlers per page too. | lol-html `src/rewriter/settings.rs:431-437,972` @ 1d5ee9e8; Cloudflare's "HTML parsing 2" blog; spider `page.rs:5169-5194` |
| Build the DOM lazily, once, only when the format needs it; links come from the streaming pass. | crawlee-rs README and `crawlee-utils` @ 082f2d0b |
| Allocation discipline in the link pass: a minimal emitter that discards comments, doctype and text; per-page arena; reused buffers. | hyperlink `src/html/parser.rs:18-62,209-299` @ cdfc6408 |
| A zero-copy arena tree (text and attributes borrow the input, nodes in one `Vec` with u32 indices, tags as an enum) if a lighter tree than html5ever is wanted. astral-html extracts links 2.6× faster than lol_html on a 419 KB page but neither streams chunks nor builds a spec tree. | pagesift `crates/pagesift-html/src/lib.rs:1-8`; astral-html `benchmarks/results/README.md` @ 7ea072ad |
| Single-policy readability (`parse_with_policy`) on the fast path. dom_smoothie 0.14 scores F1 0.865 on ScrapingHub's set against 0.947 for Readability.js and 0.970 for rs-trafilatura: re-measure 0.19 before choosing. | dom_smoothie README; ScrapingHub article-extraction-benchmark @ 62364afa |
| Serialise the adblock engine at build time and deserialise at start (24.5 ms vs 174.6 ms, 62 MB build peak avoided); query cosmetic rules per host once, and hiding selectors only for class/id tokens seen in the streaming pass. | adblock-rust `src/engine.rs:367,381,436,450` @ 09d297c2; PR #554 CI figures |
| Heavy parse off the runtime only above a size threshold (OpenSpider: `spawn_blocking` above 1 MiB; crw: a pool of cores×2/3 with a reserved interactive quarter). | OpenSpider `worker.rs:52`; crw `crw-crawl/src/extract_pool.rs:1-40` |

### #2165 Memory: about 21 MiB peak

| Technique | Source |
|---|---|
| mimalloc v3 (which Foxhound already runs) with THP off (`allow_thp=0`, or `prctl(PR_SET_THP_DISABLE)`) and a short `purge_delay`: v3 waits 1,000 ms against v2's 10 ms, and v3.5.1+ enables THP on Linux, which "may increase rss". | mimalloc `src/util/options.c:102-107,148,176-177` @ v3.6.1; kernel THP docs |
| If glibc remains anywhere (C libraries, a fallback), cap arenas (`M_ARENA_MAX`) and pin `M_MMAP_THRESHOLD`. obscura measured RSS falling from 350–826 MiB to 166–269 MiB with no throughput change. moli instead routes the whole process (V8, libcurl) through one jemalloc with `narenas:1`. | obscura `crates/obscura-cdp/src/server.rs:703-750`; moli `moli/src/allocator.rs:1-27`; pagesift `pagesift-sys/src/lib.rs:23-52` |
| Return freed heap after large batches (`malloc_trim` or the allocator's purge) only where measured useful. | OpenSpider `writer_thread.rs:676-690` |
| Fewer threads: no second runtime for connects (Spider starts a niced `num_cpus` runtime for them); no blocking-pool DNS (hyper-util's `GaiResolver` spawns blocking `getaddrinfo` per lookup, and Tokio's blocking pool grows to 512 threads, each kept 10 s). | spider `utils/connect.rs:36-104`; hyper-util `connect/dns.rs:122-124`; tokio `runtime/builder.rs:324` |
| Cap idle connections per host at the per-host concurrency (reqwest's default is unlimited, 90 s idle). | reqwest `src/async_impl/client.rs:306-307` @ 8e1c8061 |
| A byte budget on bodies held, never a disk spill, and no broadcast ring that pins pages (see #2194). | spider `utils/html_spool.rs:1-60`, `website.rs:14676-14690` |
| Release profiles seen: fat LTO + `codegen-units=1` + `panic=abort` (DonSeTch, feroxbuster, hyperlink); jemalloc + thin LTO (Stract). None of the release profiles read uses PGO, so #2165's PGO static-PIE has no Rust crawler precedent. | per-project `Cargo.toml` |

### #2194 Backpressure and the free slot

| Technique | Source |
|---|---|
| A byte gate on admitted-but-unwritten pages (3 × batch bytes) and a writer channel sized from batch bytes. A 256-message channel left 41 % of workers blocked on 1M URLs. | OpenSpider `writer_thread.rs:57-114` |
| A global atomic count of body bytes held. Spider spills to disk past 512 MiB; Foxhound should instead stop granting slots. | spider `utils/html_spool.rs:1-60` |
| **Avoid** a non-blocking broadcast to consumers (drops pages on lag) and an idle receiver that pins up to `capacity` pages. | spider `website.rs:15080-15120,14676-14690` |
| **Avoid** BFS waves with a barrier: refill each freed slot from the frontier at once. | spider `website.rs:9143,9165-9169` |
| A host-busy-aware pop: skip hosts at their per-host in-flight cap. | pagesift `runner/mod.rs:69,144` |
| Per host: a semaphore, a token bucket with burst 1, backoff driven by `RateLimit` / `X-RateLimit-*` / `Retry-After`, and coalescing of identical in-flight requests. | lychee `lychee-lib/src/ratelimit/host/host.rs:42-107,158-184,237-330` @ d119b686 |
| Delay proportional to measured fetch time (Mercator's rule) with exponential 429 state: `max(min, fetch_time) × 2^k`. | Stract `worker.rs:398-419,503-517` @ 8ac40b02; mycel manual |
| Latency-based adaptive per-host concurrency: start at 1, +1 per round near the best RTT, halve on 429/503/timeout or above 2× best. Alternative for a global rate: feroxbuster's LimitHeap binary search with a single enforcer. | spire `crawler/concurrency.rs:42-50` @ ab9e08a5; feroxbuster `src/scanner/limit_heap.rs`, `requester.rs:179-220` |
| Latency-EWMA pre-emptive slowdown (above 3× baseline) and a per-host penalty box shared across proxy lanes. | DonSeTch `src/crawl/governor.rs` @ a9434c0c |
| A politeness key wider than the host (IP /24) as an option for shared hosting. | crusty `conf/default.yaml:45-55`, `redis-queue/src/lib.rs:49-96` |
| Reserved interactive lanes in shared semaphores, so crawl work cannot starve single-page requests. | crw `extract_pool.rs:1-40`, `host_limiter.rs:36-45` |
| A scheduler that waits on free slots, never on a timer. **Avoid** SiteOne's 10 ms empty-queue polling. | SiteOne `src/engine/crawler.rs:286-299` |

### #1838 Shared paths: counted work per page

| Technique | Source |
|---|---|
| Do the whole page in the task that holds the slot (fetch, parse, link set, hand-off), with no actor hop; drive in-flight fetches from one `FuturesUnordered` instead of a `tokio::spawn` per page. Spider spawns into a `JoinSet` but hops nowhere. | spider `website.rs:8377-8415,9100-9116`; pagesift `runner/mod.rs:125-170` |
| Build `http::Uri` and the header map once per request: reqwest re-parses `Url`→`Uri` and clones default headers on every send. | reqwest `src/into_url.rs:77-81`, `client.rs:2671-2677` |
| A process-wide async DNS cache with pre-built `SocketAddr`s (Spider: hickory behind a 5,000-entry TTL cache), or resolve once per site before scheduling (crusty). | spider `utils/dns_cache.rs:1-50`; crusty-core `crawler.rs:311-318` |
| CDP hot path: decode once from the frame bytes and dispatch on `id` versus `method` by peeking. Avoid `serde_json::Value` round trips and `#[serde(untagged)]` Response/Event enums, which buffer every frame. Use `RawValue` or an `IgnoredAny` envelope for unrouted parts. Offload only frames above 256 KiB. | chromiumoxide `chromiumoxide_types/src/lib.rs:24,203`; chromey `src/conn.rs:555-572,1178-1182`; rustenium `listeners.rs:146`; agent-browser `client.rs:76-90` |
| Zero-copy robots.txt per host (rules borrow one buffer); streaming sitemaps on quick-xml (Spider still uses the unmaintained `sitemap` 0.4). | fast-robots `BENCHMARK.md` @ 0bcd35c4; Part C |
| Stay on Tokio: no published HTTP-client comparison supports an io_uring runtime, monoio is barely maintained, and glommio is unmaintained. | Part C |

## Corrections to earlier notes

- [The gap note](2026-11-06-spider-gap.md) took Spider's `ConcurrencyLimitLayer` for its concurrency cap. It only caps new connections. The cap is the per-`Website` semaphore (`website.rs:6967-6979` @ v2.53.9).
- Spider runs a second multi-thread Tokio runtime with `num_cpus` workers for connects (`utils/connect.rs:36-104`), so its thread count in `just compare` is about twice the core count.
- Under `basic`, Spider's visited set is case-sensitive (interned on the original string), while its pending set is case-insensitive.
- Several modules that `basic` compiles in are never wired into the crawl at 2.53.9: `priority_frontier`, the `rate_limit` token bucket, and the `balance` CPU/memory monitor.
- 2.53.9 is still the latest published `spider` release; `main` (2.54.0) is unreleased.

---

## Part A1: the Spider ecosystem (Foxhound's reference competitor)

Read on 2026-11-07. Sources: GitHub through the authenticated `gh` CLI, crates.io, PyPI and npm APIs, the cloned sources below (read with grep/sed only, nothing built or run), and spider.cloud pages fetched through Exa.

**The version that matters.** The latest **published** release of `spider` is **2.53.9** (crates.io, 2026-09-05; GitHub release objects for 2.53.5–2.53.9 were only created on 2026-10-30 by a new release workflow, commit `3f81520b`). That is exactly the version `just compare` pins (`xtask/compare-spider/Cargo.toml`: `spider = "=2.53.9", default-features = false, features = ["basic", "sync"]`). So the source below is read **at tag v2.53.9 = `e4b53c5b`**, and every `spider/src/...:line` cite refers to it. `main` is at `896231b7` (2026-11-06), version 2.54.0 unreleased, 33 commits ahead: page classification (content/challenge/block/error), browser cookie-jar sharing, robots.txt for single-page crawls, one WARC writer per Website, relative `<base href>` fixes, panics removed. None of them changes the HTTP crawl's fundamentals described here.

Clones: `spider` (blobless, full history), `chromey` @ `0312d624` (2026-10-18), and shallow `sp-*` clones of the satellites listed in the table.

### Summary table

| Project | Kind | URL | Licence | Stars | Last release (date) | 2026 commits / last commit | Maintained? | Robots | Politeness | Durable resume | Sitemaps | Browser | Extraction | Proxies | Sessions | Depth (deep/triaged) |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| spider (crate `spider`) | Rust crawler library | https://github.com/spider-rs/spider | MIT | 2,835 | 2.53.9 (2026-09-05 crates.io) | 842 / 2026-11-06 | Yes, very active (1 maintainer, ~600 commits AI-co-authored) | Yes, **off by default** | Global static delay + opt-in latency auto-throttle; no per-host concurrency | **No** (pause/resume in process only; SQLite is a visited-set spill deleted on drop) | Yes (`sitemap` feature, off by default) | Yes (`chrome`, `smart`, `webdriver` features) | Links + title/meta only in core; markdown/readability via `spider_transformations` | Yes, round-robin client rotation | Cookie jar per Website | deep |
| spider_cli | CLI over spider | same repo `spider_cli/` | MIT | (repo) | 2.53.9 (2026-09-05) | (repo) | Yes | via spider | via spider | No | via spider | Yes (default `chrome`) | via spider | Yes | Yes | triaged (allocator + runtime read) |
| spider_worker | Decentralized worker (warp/hyper, flexbuffers) | same repo `spider_worker/` | MIT | (repo) | 2.53.9 | (repo) | Yes | — | — | No | — | — | links/HTML | — | — | triaged |
| spider_utils | CSS-selector extraction helpers | same repo `spider_utils/` | MIT | (repo) | 2.53.9 | (repo) | Yes | — | — | — | — | — | CSS selectors | — | — | triaged |
| spider_agent / spider_agent_html / spider_agent_types | LLM agent automation (OpenAI-compatible, search providers) | same repo | MIT | (repo) | 2.53.9 | (repo) | Yes | — | — | — | — | via chrome/webdriver | LLM extraction, HTML cleaning | — | — | triaged |
| spider_mcp | MCP server (scrape/crawl/links/transform) | same repo `spider_mcp/` | MIT | (repo) | 2.53.9 | (repo) | Yes | — | — | — | — | — | markdown/text via transformations | — | — | triaged |
| spider-py (PyPI `spider-rs`) | Python binding (PyO3 over the Rust crate) | https://github.com/spider-rs/spider-py | MIT | 109 | 0.0.57 PyPI (2026-01-19) | 6 / 2026-01-20 | Stale since Jan 2026 | via spider | via spider | No | via spider | Yes | via spider | Yes | Yes | triaged |
| spider-nodejs (npm `@spider-rs/spider-rs`) | Node binding (napi-rs over the Rust crate) | https://github.com/spider-rs/spider-nodejs | MIT | 59 | 0.0.163 npm (2026-01-24) | 4 / 2026-07-23 | Low activity; 6,889 npm downloads last month | via spider | via spider | No | via spider | Yes | via spider | Yes | Yes | triaged |
| chromey (crates `chromey`, formerly `spider_chrome`) | CDP client, chromiumoxide fork (lib name `chromiumoxide`) | https://github.com/spider-rs/chromey | MIT OR Apache-2.0 | 52 | 2.58.2 crates.io (2026-09-12); `spider_chrome` last 2.37.130 (2026-03-25) | 166 / 2026-10-18 | Yes | — | — | — | — | CDP | — | per-browser | per-context | deep (relevant to Collie) |
| headless-browser | Chrome process manager for cloud (LB health, CDP proxy) | https://github.com/spider-rs/headless-browser | MIT | 88 | crate 0.1.24 | 5 / 2026-02-09 | Low activity | — | — | — | — | Chrome launcher | — | proxy | — | triaged |
| spider_transformations | HTML → markdown/text/XML, readability, chunking | https://github.com/spider-rs/spider_transformations | MIT | 2 | 2.39.13 (2026-04-30) | 26 / 2026-04-30 | Yes (quiet since Apr) | — | — | — | — | — | Yes (readability + markdown) | — | — | triaged |
| llm_readability | Readability (rewrite of readability-rs, html5ever DOM) | https://github.com/spider-rs/readability | MIT | 17 | 0.0.17 (2026-04-30) | 10 / 2026-04-30 | Yes | — | — | — | — | — | Main text | — | — | triaged |
| fast_html2md | HTML → Markdown, lol_html rewriter (default) or html5ever | https://github.com/spider-rs/html2md | MIT | 79 | 0.0.63 (2026-09-08) | 10 / 2026-09-08 | Yes | — | — | — | — | — | Markdown | — | — | triaged |
| spider_firewall | FST-based domain blocklists (13/26/52 MB tiers) | https://github.com/spider-rs/firewall | MIT | 1 | 2.42.1 (2026-10-24) | 56 / 2026-10-24 | Yes | — | — | — | — | blocks requests | — | — | — | triaged |
| spider_fingerprint | Browser fingerprint/stealth scripts and headers | https://github.com/spider-rs/spider_fingerprint | MIT | 15 | 2.39.0 (2026-03-30) | 8 / 2026-03-30 | Yes (quiet) | — | — | — | — | stealth | — | — | — | triaged |
| spider_network_blocker | Ad/tracker/script block lists for interception (phf, optional adblock) | https://github.com/spider-rs/spider_network_blocker | MIT | 1 | 0.0.86 (2026-07-06) | 36 / 2026-07-06 | Yes | — | — | — | — | blocks requests | — | — | — | triaged |
| spider-http-cache-reqwest (repo http-cache) | Fork of http-cache (RFC 9111 caching middleware) | https://github.com/spider-rs/http-cache | MIT OR Apache-2.0 | 0 | 1.0.0-alpha.7 (2026-05-14) | 32 / 2026-04-18 | Yes | — | — | — | — | — | — | — | — | triaged |
| spider_remote_cache | Client for a remote/hybrid page cache | https://github.com/spider-rs/spider_remote_cache | MIT (crates.io) | 0 | 0.5.0 (2026-09-29) | 8 / 2026-10-18 | Yes | — | — | — | — | — | — | — | — | triaged |
| hybrid_cache_server | Cache index server | https://github.com/spider-rs/hybrid_cache_server | MIT | 0 | 0.2.3 (2026-04-15) | 24 / 2026-10-18 | Yes | — | — | — | — | — | — | — | — | triaged |
| async_job | Async cron jobs (used by `cron` feature) | https://github.com/spider-rs/async_job | MIT | 2 | 0.1.4 (2023-12-27) | 0 / 2023-12-27 | No (dormant, finished) | — | — | — | — | — | — | — | — | triaged |
| spider-clients (crate `spider-client`) | Spider Cloud API SDKs (Rust, Python, JS, Go) | https://github.com/spider-rs/spider-clients | MIT | 26 | go/v0.1.92 (2026-07-14); crate 0.1.92 (2026-07-18) | 33 / 2026-10-01 | Yes | (cloud) | (cloud) | — | (cloud) | (cloud) | (cloud) | (cloud) | — | triaged |
| spider-cloud-agent (`spider-agent-cli`) | Agentic Rust client/CLI for Spider Cloud | https://github.com/spider-rs/spider-cloud-agent | MIT | 1 | 0.7.0 (2026-09-17) | 3 / 2026-09-21 | New | (cloud) | (cloud) | — | — | (cloud) | (cloud) | (cloud) | — | triaged |
| spider-browser | TS/Python/Rust SDK for the cloud browser | https://github.com/spider-rs/spider-browser | MIT | 3 | crate 0.4.0 (2026-07-19) | 29 / 2026-08-03 | Yes | — | — | — | — | remote CDP | AI extract | (cloud) | session | triaged |
| tendril / html5ever / markup5ever(_rcdom) forks | `Send`-able atomic-refcount html5ever stack | https://github.com/spider-rs/tendril (and siblings) | MIT OR Apache-2.0 | 0 | 0.5.2 / 0.40.0 (2026-09-08) | 4–5 / 2026-09-08 | Yes | — | — | — | — | — | DOM parsing | — | — | triaged |
| ua_generator, case_insensitive_string, auto_encoder | Support crates (UA strings; case-folded CompactString; charset decode) | github.com/spider-rs/{ua_generator,case_insensitive_string,auto-encoder} | MIT | 40 / 4 / 2 | 0.5.88 (2026-11-01) / 0.2.11 (2026-04-08) / 0.2.4 (2026-04-29) | 49 / 1 / 2 | Yes | — | — | — | — | — | — | — | — | triaged |
| **Spider Cloud** (spider.cloud) | Commercial crawl/scrape/search API, cloud browser, proxy network | https://spider.cloud | proprietary service | — | — | — | Active (blog posts through 2026-09) | On by default per its FAQ | (service) | — | yes | Closed-source "native" Rust browser runtime | markdown/text/JSON | 215M+ residential exits (claimed) | browser sessions | triaged (published claims only) |

Health notes:

- **spider:** created 2018-01-07; 2,835 stars, 323 forks, 2 open issues; crate downloads 2,111,986 all-time, 50,747 in the last 90 days. The star/fork ratio and download history look organic for a 2018 crate (the stargazer-timestamp API answered 403 to the token, so timestamps were not sampled). **Anomaly:** of 842 commits in 2026, 786 are by the single maintainer (j-mendez) and about 600 carry `Co-authored-by: Claude Opus 4.x` trailers (`git log --since=2026-01-01 --format='%(trailers:key=Co-authored-by,valueonly)'`). crates.io shows at least 100 `spider` versions published in 2026 (the API page cap). The feature surface grows very fast; several modules are compiled but not wired into the crawl (below).
- **chromey:** GitHub's "latest release" object says v2.46.39 (2026-04-20) while crates.io has 2.58.2 (2026-09-12) and `Cargo.toml` on `main` says 2.58.3: GitHub releases lag the crate.
- **spider-rs/benchmark** is a fork of `browser-use/benchmark` with **one** extra commit adding a Spider Cloud provider and self-reported Stealth Bench results (85%, 68/80 tasks; `stealth_bench/official_plots/category_breakdown_light.md`). It is not upstream.

---

#### spider (crate `spider` 2.53.9)

- **What:** A Rust crawling library ("The fastest web crawler and scraper for Rust", `README.md:9`): one `Website` per start URL crawls its host (optionally subdomains/TLD), streams each `Page` to subscribers over a Tokio broadcast channel, and gathers links with lol_html while the body arrives. 100% Rust; Python (PyO3) and Node (napi-rs) bindings wrap the same crate. The README states "The same engine runs Spider Cloud" (`README.md:28`). Workspace crates: `spider`, `spider_cli`, `spider_worker`, `spider_utils`, `spider_agent*`, `spider_mcp`.
- **Health:** see the table and notes above.
- **Features vs Foxhound:**
  - *robots.txt:* parser in `src/packages/robotparser/parser.rs`; fetched once per crawl when `respect_robots_txt` is set; a `Crawl-delay` becomes the crawl's global delay, capped at 60 s (`website.rs:2793-2820`). **Off by default** (`Configuration` derives `Default`; `configuration.rs:3589` asserts `!config.respect_robots_txt`). `robots_cache` (cross-crawl TTL cache) is an opt-in feature (`Cargo.toml:421`).
  - *Politeness:* a single static delay slept in the dispatch loop before each spawn (`website.rs:8362-8364`, `setup_crawl` `:4158-4168`), i.e. per Website, which is per host. `auto_throttle` (latency-based per-domain extra delay) is compiled in `basic` but only active when `configuration.auto_throttle` is set (`website.rs:4035`, `:8366-8376`). The per-domain token bucket `rate_limit` (`utils/rate_limiter.rs`) is compiled in `basic` but **referenced nowhere else in `src/`** at 2.53.9, so it is not wired. No per-host concurrency cap: concurrency is per Website.
  - *Durable resumable crawls:* **none.** `control` gives in-process pause/resume/shutdown through a `watch` channel (`utils/mod.rs:9536-9600`). The `disk` feature's SQLite holds only two tables, `resources(url COLLATE NOCASE)` and `signatures` (`features/disk.rs:113-128`), in `$TMPDIR/spider[_<crawl_id>].db` (`:352-378`), and the file is deleted on drop unless `persist` is set (`:323-330`). No frontier, no lease, no outcome is ever persisted; `persist_links()` only keeps the status Active between chained runs (`website.rs:2721-2724`).
  - *Sitemaps:* `sitemap` feature (off by default; `Cargo.toml:277`), `crawl_sitemap` / `sitemap_crawl_raw` (`website.rs:7377`, `:12117-12190`), quick-xml parsing.
  - *Browser rendering:* `chrome` (chromey), `smart` (HTTP first, Chrome when the page looks like it needs JS), `webdriver` (thirtyfour), `parallel_backends`/`servo` racing alternative engines. All opt-in.
  - *Extraction:* core extracts links, title, description, og:image in the streaming pass (`page.rs` `metadata_handlers`); `spider_utils` adds CSS-selector maps; main text and markdown come from the separate `spider_transformations` (readability + fast_html2md); `openai`/`gemini` features for LLM extraction.
  - *Proxies:* `with_proxies` adds every proxy to one reqwest client (`website.rs:3187-3221`); with 2+ proxies a `ClientRotator` round-robins whole clients with an `AtomicUsize` (`website.rs:1320-1345`); `hedge` races a second proxy after an adaptive delay (`website.rs:8549-8650`).
  - *Sessions/cookies:* `cookies` feature: one reqwest `Jar` per Website (`website.rs:1503-1504`); `with_cookies(str)`; no named, persisted sessions.
- **Fundamentals (from source, @ v2.53.9 `e4b53c5b`):**
  - *Runtime:* the library owns no runtime; the caller's `#[tokio::main]` (multi-thread, one worker per core) runs it, as in Goodfetch's harness. On top, `init_background_runtime()` (`utils/connect.rs:36-104`, called at crawl setup `website.rs:4072`) starts a **second multi-thread runtime with `num_cpus` workers at `nice(10)`** for connection establishment (`BackgroundProcessorLayer`, installed as a reqwest `connector_layer`, `website.rs:3241-3245`), unless io_uring is active, in which case it is a current-thread runtime. So the compare harness's Spider (io_uring off) runs about 2 × cores Tokio threads, plus the io_uring-less fallback. Connects are rare with keep-alive, so this pool idles.
  - *Concurrency:* one `tokio::sync::Semaphore` per Website sized to `concurrency_limit`, else `DEFAULT_PERMITS = calc_limits(1)` = max(core-derived, 20 or 30) (`website.rs:859-876`, `:1006-1020`, `setup_semaphore` `:6967-6979`). The driver loop draws links from `tokio_stream::iter(links)` only while `available_permits() > 0` (`website.rs:8327-8335`), acquires an owned permit (`:8377`) and **spawns one task per page into a `JoinSet`** (`spawn_set`, `utils/mod.rs:10010-10034`; call `website.rs:8415`). The task fetches, parses, broadcasts the page, drops the permit and returns its link set (`:9100-9116`); the driver merges results in `set.join_next()` (`:9126-9139`). The task does everything in place: no actor hop. A `tower` `ConcurrencyLimitLayer(limit)` is also put on the **connector** (`website.rs:3247-3252`): it caps concurrent connection set-ups, not requests (the prior gap note called it "the concurrency"; the semaphore is).
  - *Load balancing (`balance`):* `get_semaphore` can swap to the process-wide `SEM_SHARED` under CPU ≥ 70% or RSS pressure (`utils/mod.rs:10115-10148`), but the state comes from `detect_system`'s monitor, which only starts when the async getters `get_global_cpu_state()`/`get_global_memory_state()` are called (`detect_system.rs:174-189`, `:266-272`), and grep finds **no caller** in `src/`. So in a plain crawl (and in `just compare`) the sync getters return 0 and the configured limit holds. Same for the disk spill "under memory pressure".
  - *Frontier:* not a queue. The pending set is a `HashSet<CaseInsensitiveString>` (`links`); each `'outer` iteration takes the whole set into a stream (`website.rs:8327`); new links from finished tasks go into a fresh `links` set via `links_visited.extend_links` (`:9126-9136`, `interner.rs:448-480`) and are dispatched only when the current stream is exhausted **and** the `JoinSet` is empty (`else => break` at `:9143`, then `:9165-9169`). It is a **BFS wave with a barrier**: at the end of each wave in-flight requests drain to zero before the next wave starts. Hash-set iteration order is arbitrary within a wave. `priority_frontier` (`utils/frontier.rs`, a `BinaryHeap` with domain round-robin) is compiled in `basic` but **not used by the crawl loop** (no reference outside `utils/mod.rs:35`).
  - *Visited set and bytes per URL:* `links_visited: Box<ListBucket>` (`website.rs:1446`). Under `basic`, `string_interner_buffer_backend` is on, so `ListBucket` is `HashSet<SymbolUsize>` + `StringInterner<BufferBackend<SymbolUsize>>` (`utils/interner.rs:29-90`, insert `:250-275`). The interner keys on `link.as_ref()`, the original string, so **dedup is case-sensitive under `basic`**, while the non-interner variant uses `CaseInsensitiveString` (CompactString with ASCII case-folded hash/eq, `case_insensitive_string/src/lib.rs:189-220`, `:419-450`): a path-case difference is the same URL there. Each stored URL is the absolute serialization of a parsed `url::Url` (`page.rs:2878-2912`). **Estimate per visited URL** (from the layout, not measured): URL bytes + 1–2 B varint length in one growing `Vec<u8>`, plus two hashbrown tables of 8-byte symbols (interner dedup map, visited set), each 9 B per slot at a load of 44–87% ≈ 10–20 B: **≈ URL length + 22–42 B**, plus up to 2× slack on the buffer from doubling. Pending URLs cost more: CompactString 24 B inline (heap copy above 24 bytes) in a hashbrown set ≈ 30–57 B + URL heap. `LINKS_VISITED_MEMORY_LIMIT` = 15,000 (env-tunable, `website.rs:1023-1030`): beyond it `disk` spills inserts to SQLite one `INSERT` per URL (`website.rs:1888-1918`, `disk.rs:203-215`), and `bloom` (opt-in, `Cargo.toml:424`) switches to bloom-only membership (mmap, 7 hashes, 1% FP, `utils/bloom.rs:1-40`), accepting skipped pages. Without either, a warning only (`interner.rs:226-243`).
  - *Page signature dedup:* with `normalize`, a hash of the HTML goes into `signatures: HashSet<u64>` and duplicate content's links are dropped (`website.rs:9128-9132`).
  - *Persistence/durability:* none beyond the spill above. A crash loses everything.
  - *HTTP client:* reqwest 0.13 with `brotli, gzip, deflate, zstd, stream, http2, query` (`Cargo.toml:169-177`), built once per crawl (`configure_base_client`, `website.rs:3024-3101`): connect timeout 24 s, read timeout 42 s (×2 with proxies), `tcp_keepalive(30 s)`, HTTP/1 leniency options, title-case headers, `http09_responses`. **No** `pool_idle_timeout`, `pool_max_idle_per_host` or `tcp_nodelay` call: reqwest defaults. HTTP/2 by ALPN, or prior knowledge when configured. DNS: with `dns_cache` (in `basic`), a process-wide hickory `TokioResolver` behind a 5,000-entry TTL cache with pre-computed `SocketAddr`s (`utils/dns_cache.rs:1-50`; installed at `website.rs:3097-3098`). `wreq` (BoringSSL browser TLS emulation) is an alternative client feature.
  - *HTML parsing:* lol_html `send::HtmlRewriter` with a no-op output sink, **fed each body chunk as it arrives** in `handle_response_bytes_writer` (`utils/mod.rs:6486-6571`: `rewriter.write(&res_bytes)` then `collected_bytes.extend_from_slice`), so links are extracted during the download and the body is also kept as one `Vec` pre-sized from Content-Length (`page.rs:5204-5215`). Handlers are built per page (`build_link_extract_handlers`, `page.rs:5169-5194`). UTF-8 validity is tracked incrementally (`Utf8StreamValidator`). Per page it also clones the response `HeaderMap` (`utils/mod.rs:6505`) and allocates a fresh `HashSet::with_capacity(32)` for links (`website.rs:8657`).
  - *Hand-off to the consumer:* `channel_send_page` does a non-blocking `broadcast::Sender::send` (`website.rs:15080-15120`). **No backpressure:** a slow subscriber gets `Lagged` and loses pages (the harness subscribes with capacity 4096 and fails on lag). The `Website` also keeps an idle `Arc<broadcast::Receiver<Page>>` (`subscribe`, `website.rs:14676-14690`); Tokio frees a slot's value only after every receiver has read it (`tokio-1.54.1/src/sync/broadcast.rs:1766-1773`), and that idle receiver never reads, so **up to `capacity` pages stay alive until overwritten**. Inference, not measured: this likely explains the prior note's Spider peak growing with site size (18 → 24 → 60 MB at 100 → 1,000 → 10,000 pages with capacity 4096).
  - *Memory budget (`balance`):* the HTML spool keeps a global atomic count of HTML bytes held in memory; above a 2 MiB-per-page threshold or a 512 MiB total budget, pages are written to a per-process spool directory and reloaded on access (`utils/html_spool.rs:1-60`, used in `channel_send_page` `website.rs:15091-15114`). Spilling, not backpressure.
  - *Allocator:* the library sets none. `spider_cli` offers `jemalloc` or `mimalloc` as opt-in features (`spider_cli/src/main.rs:1-20`, `spider_cli/Cargo.toml:54-55`); benches have `bench_jemalloc`/`bench_mimalloc` (`Cargo.toml:439-440`). The compare harness runs glibc malloc.
  - *Release profile:* no `[profile.*]` anywhere in the spider workspace at 2.53.9 (grep), so its CLI builds with Cargo's default release profile. The harness imposes Goodfetch's (fat LTO, codegen-units 1, opt 3). chromey's own manifest sets `lto = "thin"`, `codegen-units = 1`.
  - *Default features matter for fairness:* `default = ["basic", "io_uring", "tcp_fastopen", "splice", "numa", "zero_copy"]` (`Cargo.toml:215`). `io_uring`/`tcp_fastopen`/`splice` act only in `utils/uring_fs.rs` (file I/O through an io_uring worker pool, `TCP_FASTOPEN_CONNECT` on io_uring connects, splice). The harness turns all of them off (`default-features = false`). `basic` = `__basic` (`sync, cookies, ua_generator, encoding, string_interner_buffer_backend, balance, real_browser, disk, time, adaptive_concurrency, priority_frontier, dns_cache, rate_limit, request_coalesce, auto_throttle, etag_cache, warc`) + native-TLS (`Cargo.toml:216-244`). Several of these are compiled but inert in a default crawl (see above). **Off by default**: `chrome`, `smart`, `sitemap`, `cron`, `cache*`, `control`, `bloom`, `robots_cache`, `h2_multiplex`, `hedge`, `cowboy`, `regex`, `glob`, `socks`, `wreq`, `firewall`, `adblock`.
  - *smart mode:* the HTTP body is re-parsed with lol_html (in chunks, yielding to Tokio every few chunks on large pages) to score "needs JS": a Nuxt data id, React SSR `data-target`, `/_nuxt/` assets or framework script paths set the score to the threshold 10 at once; 4+ non-tracker `<script src>` add 10; inline scripts calling DOM-mutation methods add 7; `<noscript>` "JavaScript required" text, hydration body ids or watched attributes, or an anti-bot body set it to 10 (`page.rs:8406-8635`). At ≥ 10, and when the `RenderUpgrade` enhancement is enabled, the page is re-fetched in Chrome through a lazily launched shared browser (`OnceBrowser`, `website.rs:11663`).
  - *Browser:* one Chrome per crawl (launched or connected via `CHROME_URL`), **a new CDP target per page** created with the URL directly (`features/chrome.rs:1467-1516`), and closed by a dedicated `spider-tab-closer` OS thread with its own current-thread runtime fed by a `TabCloseGuard` on drop, which avoids leaked tabs when a hedge race drops a future (`chrome.rs:1933-2160`). No tab reuse or browser recycling by page count in the crate. Request interception (`chrome_intercept`) blocks resources by type and by chromey's lists. Launch flags: 24–60 entries depending on the build (`features/chrome_args.rs:3`, `:43`, `:77`).
  - *Clever bits:* lol_html on the wire chunks (one pass, no DOM); string interning of the visited set; zero hop per page; DNS cache with pre-built `SocketAddr`s; a tab closer that cannot leak; `Content-Length` pre-sizing; a 512 MiB HTML byte budget with disk spill; io_uring file and connect path.
- **Published benchmarks:**
  - `benches/BENCHMARKS.md` (last changed 2024-09-02, `47ac130d`): 185 pages of `https://rsseau.fr/` over the network, "10 samples": Spider 73 ms (M1 Max) / 50 ms (2-core GitHub runner) against Colly 32 s / 30 s, node-crawler 15 s / 3.4 s, wget 70 s / 60 s. The method is thin (remote site, "one decentralized worker", sample handling unclear) and a 73 ms crawl of 185 remote pages is implausible as a wall time; treat as unreliable. Bench sources: `benches/crawl.rs`, `go_colly.rs`, `node_crawler.rs`.
  - Spider Cloud claims: see that section.
- **Relevance:** the reference for `just compare`'s HTTP cells, already in it. The source explains the gap note's figures: per page one task, one lol_html pass on the chunks, interned in-memory dedup, a broadcast send, no persistence and no hop. Also important for fairness: it gives none of Foxhound's guarantees (no resume, robots off, no per-host politeness, no backpressure to the consumer, wave barrier, case-sensitive dedup under `basic`).

#### chromey (crates `chromey`, `spider_chromiumoxide_cdp`; formerly `spider_chrome`)

- **What:** spider-rs's maintained fork of chromiumoxide (lib name kept as `chromiumoxide`), the CDP layer behind spider's `chrome` feature and the cloud. Rust only.
- **Health:** 52 stars; crate 2.58.2 (2026-09-12), 345,503 downloads (45,875 recent); `spider_chrome` 1.17 M downloads, last 2.37.130 (2026-03-25); `spider_chromiumoxide_cdp` 842 k downloads. 166 commits in 2026, last 2026-10-18. GitHub releases lag the crate (above).
- **Features vs Foxhound/Collie:** request interception with resource-type blocking and JS-framework allow lists (Aho-Corasick and phf sets, `src/handler/network.rs:96-190`); `adblock` (Brave adblock engine, `network.rs:429`) and `adblock_easylist`; `firewall` (spider_firewall FST lists); HTTP response cache served to the browser (`cache`, `cache_mem`, remote cache); fingerprint/stealth via spider_fingerprint; Chrome auto-download (`fetcher`).
- **Fundamentals:** default `simd` parses CDP JSON with sonic-rs (`Cargo.toml` features). One handler task drives every page by default; the opt-in `parallel-handler` feature replaces the single `select!` loop with a Router plus one task per attached page, a bounded WS queue that fails fast, and per-session inboxes with a bounded overflow FIFO (`src/handler/parallel/mod.rs:1-48`, `src/handler/mod.rs:1191-1215`). Its internal reqwest client sets `pool_idle_timeout(60 s)`, `pool_max_idle_per_host(10)` (`src/browser.rs:58-59`). Release profile: thin LTO, codegen-units 1.
- **Published benchmarks:** none found with figures (a `benches/` directory exists).
- **Relevance:** a Collie reference (another reader may cover chromiumoxide). Worth a look: the per-page session task design for CDP fan-out, sonic-rs for CDP decoding, interception lists.

#### Spider Cloud (spider.cloud)

- **What:** the commercial service from the same author: `api.spider.cloud` (crawl/scrape/search, markdown/text/JSON, JSONL streaming), `browser.spider.cloud` (CDP over WebSocket), `proxy.spider.cloud`. Pricing: $1 per GB of pre-transformation content plus $0.0001 per CPU-minute, failed requests free; or "Unlimited" by concurrency from $40/month for 2 in flight to $19,994/month for 1,538; 10,000 API requests/minute per account; robots.txt followed by default, per-request opt-out (https://spider.cloud/pricing, fetched 2026-11-07).
- **Engine:** the crawler is the open-source Rust engine per the README ("The same engine runs Spider Cloud"). The **browser is closed-source**: "a native browser built for automation, not headless Chrome with patches" (https://spider.cloud/browser/), described as "a browser runtime written in Rust that drives Chromium, Gecko, and WebKit from one process" (https://spider.cloud/blog/bypass-cloudflare-datadome-perimeterx-2026/). The public client's retry engine names its backends: `chrome`, `chrome-new`, then `firefox`, `lightpanda`, `servo` at maximum stealth (`spider-browser/typescript/retry/retry-engine.ts`). Smart mode: HTTP fetch, then browser render, then "full profile" plus premium network.
- **Published speed and price claims, with method:**
  - Home page: "100K+ pages per second" and "215M+ exits in 199 countries" (https://spider.cloud/, no method given).
  - 1,000-URL benchmark, February 2026 (https://spider.cloud/blog/firecrawl-vs-crawl4ai-vs-spider-honest-benchmark/): 400 static, 350 SPA, 250 protected URLs; one AWS c6i.4xlarge in us-east-2 calling the cloud APIs (Crawl4AI self-hosted, no proxies). Spider 182 pages/s static, 48 SPA, 21 protected, 74 average; Firecrawl 27/14/8/16; Crawl4AI 19/11/5/12; success 99.9 / 95.3 / 89.7%; first static result 45 ms / 310 ms / 480 ms; RAG recall@5 91.5 / 89.0 / 84.5%. The author states "Spider is our product and we designed the test"; the URL list is not published and the concurrency is not stated.
  - "1 million pages in 3 h 47 min" across 10,247 domains, 73.4 pages/s average, smart mode routing 68.1% to HTTP and 31.9% to Chrome (https://spider.cloud/blog/scraping-one-million-pages-what-actually-happens/): production telemetry, not reproducible.
  - Browser: 100% on "Browser Fidelity Bench V1" (80 tasks, 2026-09-18) and 999/999 URLs across 254 domains (`spider-rs/spider-browser-dataset`); 85% on Browser Use's Stealth Bench in the spider-rs fork only. All self-reported.
- **Relevance:** not a `just compare` candidate (paid, remote, network-bound). Its claims are useful only as marketing context.

#### Triaged only (no deep source read)

spider_cli, spider_worker, spider_utils, spider_agent*, spider_mcp, spider-py, spider-nodejs (bindings: PyO3 / napi-rs over `spider = "2"`, no lockfile committed, so a release builds against whatever 2.x resolves; both quiet since January 2026), headless-browser (Chrome process manager and CDP proxy for load-balanced cloud deployments; recommends chrome-headless-shell in a 300 MB image), spider_transformations (readability via llm_readability, markdown via fast_html2md's lol_html rewriter, plus html2text/XML, chunking), llm_readability (html5ever DOM, a readability-rs rewrite), fast_html2md (lol_html streaming rewriter by default, html5ever optional; no published figures found), spider_firewall (FST blocklists), spider_fingerprint, spider_network_blocker, http-cache fork, spider_remote_cache, hybrid_cache_server, async_job (dormant since 2023), spider-clients, spider-cloud-agent, spider-browser SDK, the `Send`-able html5ever/tendril forks (atomic refcount so a parsed DOM can cross threads), ua_generator, case_insensitive_string, auto_encoder. Discovery sweep: `gh repo list spider-rs -L 200` (66 repos; the rest are examples, plugins, MCP servers, n8n/Sheets add-ons, forks of rusty_v8, tokio-uring and fastrender, `llm_models` registry) and crates.io lookups; no other Rust crawler engine in the org.

---

### Comparison with the prior gap note (no re-measurement)

The gap note (`rnd/research/2026-11-06-spider-gap.md`) measured Spider 2.53.9 at 0.40 M instructions, 12 syscalls and 1.3 context switches a page, about 15 requests in flight out of 16, 25 MB peak at 1,000 pages. The source at the same version accounts for that:

- **Zero hops:** fetch, parse, link set and broadcast all happen in the spawned task under its permit; the driver only merges link sets in `join_next` (`website.rs:8415`, `:9126-9139`). That is why the futex count is about 5 a page.
- **One lol_html pass over the wire chunks** and no second analysis (raw/extraction both just broadcast).
- **Dedup in memory**, interned; no SQLite until 15,000 visited links, which none of the 100-, 1,000- or 10,000-page cells reaches (`disk`'s memory-pressure path never fires, see the `balance` note).
- **Two corrections to the note:** the concurrency limit is the per-Website `Semaphore` (`website.rs:6967-6979`); the `ConcurrencyLimitLayer` caps connection set-ups. And Spider's memory growth with site size likely comes from the broadcast ring keeping up to 4,096 pages alive because of the idle receiver the Website holds (above), not from the visited set (≈ 100 B a URL).
- **What Spider gives up** for those figures (for an honest matrix): no resumable run state, no per-host politeness actor, robots off unless asked, a BFS wave barrier, lossy delivery to a slow consumer, case-sensitive dedup under `basic`, and raw HTML only in the extraction cell (the note already flags the last).

## Part A2: AI-era Rust crawl and extraction engines, and commercial products on a Rust engine

Reader notes, 2026-11-07. Spider / Spider Cloud excluded (other reader). Source read by shallow clone (grep/sed only, nothing built or run). SHAs: webclaw `6226b2a8`, crw `4feae45b`, donsetch `a9434c0c`, pagesift `529f03ae`, crawlberg `68904f52`, servo-fetch `32df30e2`, plasmate `5cd974d1`, crawlee-rs `082f2d0b`, crawlee-storage `5bea28ed`, Firecrawl `bf6739d9` (read through `gh api`).

Health data: `gh api` on 2026-11-07 (stars, `pushed_at`, commits since 2026-01-01 summed over pages, latest commit, latest release); crates.io API; npm downloads API (last month); pypistats.org `recent`.

**Star plausibility could not be checked directly.** The token used cannot list stargazers: REST answers `403 Resource not accessible by personal access token`, and GraphQL `stargazers` answers `FORBIDDEN`. The fallback was to compare stars with watchers, forks, issues, PRs, contributors and registry downloads (see each section). Two projects look suspicious: **pagesift** (251 stars in 18 days, 1 watcher, 2 issues, 2 contributors, one of them the `claude` bot) and, less strongly, **webclaw** (2,435 stars against 12 watchers and 32 issues, and only 77 npm downloads a month for `webclaw-mcp`). xerj (18.5k stars in 4 months) has engagement to match (1,363 forks, 865 issues, 2,141 PRs, 186 watchers), but it is not a crawler.

| Project | Kind | URL | Licence | Stars | Last release (date) | 2026 commits / last commit | Maintained? | Robots | Politeness | Durable resume | Sitemaps | Browser | Extraction | Proxies | Sessions | Depth |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| webclaw (+ webclaw.io cloud) | CLI/MCP/REST extractor + crawler, hosted API | https://github.com/0xMassi/webclaw | AGPL-3.0 | 2,435 | v0.7.2 (2026-11-04) | 932 / 2026-11-04 | yes | yes, Crawl-delay ≤30 s | per-origin start spacing; global in-flight cap | JSON checkpoint every 1 s | yes | optional chromey (CDP), 4 tabs, learned per host | own scraper-DOM extractor → MD/"llm" | yes, pool | cookies; persistent Chrome profile | deep |
| fastCRW / crw (+ fastcrw.com cloud) | Firecrawl-compatible API server, CLI, MCP | https://github.com/fastcrw/crw | AGPL-3.0 (+ commercial) | 1,234 | v0.42.1 (2026-11-06) | 1,264 / 2026-11-07 | yes | yes | per-eTLD+1 rps + semaphore | no (in-memory) | yes | own CDP client: LightPanda → Chrome (+Camoufox), context pool | lol_html clean + scraper/readability + htmd | yes, BYOP sticky per host | reqwest cookies; crw-browse sessions | deep |
| DonSeTch | MCP/CLI fetch+search+crawl, no browser on the fetch path | https://github.com/dondai44423/donsetch | AGPL-3.0 | 975 | v4.10.5 (2026-11-03); v5 beta on main | 1,160 / 2026-11-07 | yes | yes (RFC 9309 states) | adaptive governor per (host, lane) + cross-process pace file | resume tokens on disk | yes | no browser for reads (V8 challenge runner); Chromium only for screenshots/login | own (scraper) | yes | cookies, `donsetch login` | deep |
| pagesift | Firecrawl/Jina-compatible server, doc parser, crawler | https://github.com/YOLOW92/pagesift | AGPL-3.0 (+ commercial) | 251 (anomalous) | v1.2.0 (2026-11-07) | 617 / 2026-11-07 (history starts 2026-10-20) | yes, but very young | yes | per-host interval + concurrency, 429 widening | yes (store-backed frontier log) | yes | own CDP client: Lightpanda → moli → Chrome, adaptive pool | own zero-copy HTML parser (`pagesift-html`) | yes | explicit cookie sessions | deep |
| crawlberg (ex kreuzcrawl; xberg) | Rust crawl core + 14 language bindings, MCP/REST | https://github.com/xberg-io/crawlberg | MIT (enterprise extras closed) | 195 | v1.13.0 (2026-11-06) | 2,327 / 2026-11-07 | yes | yes (crawlberg-robots) | `PerDomainThrottle`, adaptive | no in OSS (pluggable `Frontier`/`CrawlStore` traits) | yes | chromiumoxide fork, pool, WAF detection | tl + html5ever → html-to-markdown-rs | yes (per-host rotation trait) | cookie jars, auth | deep (medium) |
| servo-fetch | embedded Servo: fetch+JS+layout+extract | https://github.com/konippi/servo-fetch | MIT / Apache-2.0 | 163 | v0.15.2 (2026-11-01) | 505 / 2026-11-07 | yes | yes | delay 500 ms default, concurrency 1 | no | sitemap map | Servo in-process (SpiderMonkey) | dom_smoothie/dom_query + htmd, layout-aware | not checked | one-use worker per session | triaged+ |
| plasmate | browser engine for agents (V8 + html5ever → "Semantic Object Model" JSON), CDP subset | https://github.com/plasmate-labs/plasmate | Apache-2.0 | 39 | v0.5.1 (2026-04-05) | 2,517 / 2026-11-07 | active; no release for 7 months | – | – | – | – | is the engine | Semantic Object Model JSON | – | cookies (reqwest) | triaged |
| crawlee-rs (Apify) | Rust port of Crawlee (Basic/Http/Html crawlers) | https://github.com/apify/crawlee-rs | Apache-2.0 | 1 | none | 33 / 2026-11-03 (created 2026-09-30) | early | yes | yes (ThrottlingRequestManager, autoscaling) | yes (Crawlee fs storage) | yes | no (early) | lol_html streaming links + scraper lazy DOM | yes | SessionPool | deep (medium) |
| crawlee-storage (Apify) | Rust core of Crawlee JS/Python fs storage (`@crawlee/fs-storage-native`) | https://github.com/apify/crawlee-storage | Apache-2.0 | 4 | v0.2.2 (2026-09-17) | 108 / 2026-09-21 | yes | – | – | storage layer | – | – | – | – | – | triaged |
| Firecrawl `firecrawl_rs` (apps/api/native) | N-API helper lib inside Firecrawl's Node API | https://github.com/firecrawl/firecrawl/tree/main/apps/api/native | AGPL-3.0 | (repo 199k) | n/a | n/a | yes | texting_robots | – | – | sitemap parse | – | lol_html + kuchikiki transforms; MD is Go | – | – | triaged+ |
| `firecrawl` crate | Firecrawl API client (rust-sdk) | https://crates.io/crates/firecrawl | MIT | – | 2.21.8 (2026-10-23) | – | yes | client only | | | | | | | | triaged |
| firecrawl/simd-html-to-md | experiment: std::simd HTML→MD, no DOM | https://github.com/firecrawl/simd-html-to-md | none | 4 | none | 3 / 2026-02-04 | no (experiment) | | | | | | tokenizer→emitter | | | triaged |
| cortex-scout | MCP search+extract+CDP automation | https://github.com/cortex-works/cortex-scout | MIT | 71 | v3.3.7 (2026-06-07) | 206 / 2026-06-07 | stale since June | ? | ? | no | ? | Chromium CDP + HITL | readability + html2md | yes | stateful browser | triaged |
| stygian | graph-based scraping toolkit + stealth browser + proxy pool | https://github.com/greysquirr3l/stygian | AGPL-3.0 | 15 | v0.17.0 (2026-08-24) | 474 / 2026-08-25 | slowing | ? | ? | ? | ? | stealth CDP | scraper | pool | sticky | triaged |
| scrapling-rs | port of Python Scrapling | https://github.com/chrisabruce/scrapling-rs | MIT | 3 | none | 6 / 2026-04-15 | no (one-day dump) | | | | | | html5ever | | | triaged |
| paperfoot/search-cli | metasearch CLI (13 providers) + page extract | https://github.com/paperfoot/search-cli | MIT | 59 | v0.12.1 (2026-11-04) | 194 / 2026-11-04 | yes | not a crawler | | | | | | | | triaged |
| bug-ops/scrape-rs (fast-scrape) | HTML parsing lib, Py/Node/WASM bindings | https://github.com/bug-ops/scrape-rs | Apache-2.0 | 10 | v0.2.14 (2026-10-21) | 264 / 2026-11-02 | yes | not a crawler | | | | | parser | | | triaged |
| xerj | AI search engine/indexer over local data | https://github.com/xerj-org/xerj | Apache-2.0 | 18,554 | v1.0.0-rc.101 (2026-11-05) | 4,172 / 2026-11-07 | yes | **no crawler** (no robots.txt handling, no crawl in README): out of cluster | | | | | | | | triaged, dropped |
| Kirky-X/crawlrs | "enterprise" crawl platform, 5 engines, REST | https://github.com/Kirky-X/crawlrs | Apache-2.0 | 1 | v0.2.0 (2026-07-29) | 770 / 2026-10-21 | yes, no traction | ? | ? | ? | ? | yes | rs-trafilatura / dom_smoothie | ? | ? | triaged |
| stealthscraper-rs | stealth CDP + JA4 MITM proxy library | https://github.com/YpNo/stealthscraper-rs | MIT | 3 | v1.0.2 (2026-11-06) | 164 / 2026-11-06 | yes | not a crawler | | | | own CDP (pipe) | | | browser→HTTP identity handoff | triaged |
| drission-rs | Chrome automation for testing your own site | https://github.com/MageGojo/drission-rs | NOASSERTION | 33 | v0.6.1 (2026-08-21) | 52 / 2026-10-21 | slowing | not a crawler | | | | CDP | | | | triaged |
| rs-trafilatura | trafilatura port with page-type ML | https://github.com/Murrough-Foley/rs-trafilatura | Apache-2.0 | 68 | v0.1.1 (2026-03-24) | 24 / 2026-04-03 | stale | used only by crawlrs (feature) and pagesift's benchmark harness | | | | | | | | triaged |
| netweir (sweep) | Rust-core scraper with Python API | https://github.com/netweir/netweir | AGPL-3.0 | 7 | v0.7.1 (2026-11-02) | 575 / 2026-11-07 | yes, very young | ? | ? | claims resume | ? | ? | ? | ? | ? | triaged |
| fatsearch (sweep) | search+read CLI/MCP | https://github.com/fatsearch/fatsearch | Apache-2.0 | 102 | v0.2.5 (2026-11-06) | 13 / 2026-11-06 | young | not a crawler | | | | | | | | triaged |
| AgenticCrawler / acrawl (sweep) | LLM-driven crawler | https://github.com/Mingye-Lu/AgenticCrawler | MIT | 15 | v0.17.0 (2026-10-28) | 1,499 / 2026-11-05 | yes | ? | ? | ? | ? | ? | LLM | ? | ? | triaged |
| essence (sweep) | HTTP→Chromium fallback retrieval engine | https://github.com/ruchit-p/essence | MIT | 6 | v0.3.0 (2026-03-05) | 17 / 2026-04-18 | stale | yes | per-domain | no | yes | Chromium | own | ? | ? | triaged |
| fire_ctrl (sweep) | "spec-compliant Firecrawl v2 runtime in Rust" | https://github.com/cipher-rc5/fire_ctrl | MIT | 3 | none | 73 / 2026-11-02 | low traction | | | | | | | | | triaged |
| svipall (sweep) | MCP/CLI page→MD, site crawl | https://github.com/ilien-dev/svipall | AGPL-3.0 | 20 | v1.3.1 (2026-10-02) | 149 / 2026-10-02 | young | | | | | | | | | triaged |
| axon (sweep) | self-hosted RAG stack incl. crawl | https://github.com/dinglebear-ai/axon | AGPL-3.0 | 4 | v7.5.0 (2026-10-05) | 3,092 / 2026-11-07 | yes, no traction | | | | | | | | | triaged |

Commercial APIs checked for a Rust-engine claim (first-party sources only):
- **fastCRW** (fastcrw.com): states "The engine is written in Rust" (https://fastcrw.com/, FAQ). Verified; it is crw.
- **webclaw.io**: hosted API for the same engine (`README.md`: "webclaw.io is the hosted web extraction API for webclaw"). The repository's `cloud.rs` is the client for it.
- **Ilmenite** (https://ilmenite.dev/): "Dual engine: pure rust for static pages, real chromium when it gets hard". No public source found. A claim only, unverified.
- **BeeCrawl** (https://beecrawl.dev/): "A lean Rust core handles everyday extraction". The likely repository, `jacoobwang/beecrawl` (4 stars, Rust, last push 2026-09-06), was not read.
- **Not Rust**, by GitHub language data for their public engines: Jina Reader (`jina-ai/reader`: TypeScript), Steel (`steel-dev/steel-browser`: TypeScript), Browserless (TypeScript), Kernel (`kernel/kernel-images`: Go/TypeScript), Hyperbrowser (`HyperAgent`: TypeScript), ScrapeGraphAI (Python), Olostep (only TypeScript MCP/SDK repositories are public). Scrapfly, Exa, Parallel, BrowserCat and Cloudflare Browser Rendering `/crawl`: no first-party Rust claim found in an Exa search. Dropped.
- **Apify**: crawler-level Rust exists. `apify/crawlee-rs` is an early port, and `apify/crawlee-storage` is now the native storage core of Crawlee for JS v4: `@crawlee/fs-storage@4.0.0-beta.67` depends on `@crawlee/fs-storage-native@0.1.5-beta.18` (npm registry), which had 22k downloads last month. impit is covered by the building-blocks reader.

---

#### webclaw

- **What:** an all-Rust CLI, MCP server and REST server that turns URLs into Markdown, "llm" text or JSON, with a crawler, batch mode, PDF and Office reading, and site extractors. A Python wheel (`python/pyproject.toml`, maturin) ships the CLI binary. npm `create-webclaw` installs the MCP config. The commercial offering is the hosted API webclaw.io (the same engine, plus proxies and anti-bot). The sister crate `webclaw-tls` (Chrome TLS/H2 fingerprinting) is now archived; the project uses `wreq`.
- **Health:** created 2026-03-10; 2,435 stars, 246 forks, 12 watchers, 32 issues, 163 PRs, 9 contributors; 932 commits in 2026, last 2026-11-04; v0.7.2 (2026-11-04). Not on crates.io (installed from git). Low watcher, issue and download counts for this star count (npm `webclaw-mcp` 77 a month, `create-webclaw` 252 a month): a mild anomaly.
- **Features vs Foxhound:**
  - **robots.txt:** yes, including `Crawl-delay` capped at 30 s (`crates/webclaw-fetch/src/crawler.rs:950-962`). It fails closed: an unavailable robots.txt is treated as disallowed (`crawler.rs:338`).
  - **Politeness:** a per-origin `next_start` instant (`crawler.rs:599-604`, `:666-669`), default delay 100 ms and concurrency 5 (`crawler.rs:148-150`). There is no per-host concurrency cap beyond spacing.
  - **Durable resume:** whole-state JSON checkpoint, written to a tmp file then renamed, every **1 s** when dirty (`crawler.rs:365-400`, `:1028`). It holds `visited` and `frontier`, and in-flight URLs go back into the frontier. A `PageSink` streams pages so they survive a crash.
  - **Sitemaps:** yes (`sitemap.rs`; seeds the frontier, `crawler.rs:564-585`).
  - **Browser:** optional `chromey =2.58.2` (Spider's chromiumoxide fork) behind the `browser` feature.
  - **Extraction:** `scraper` 0.22 (html5ever DOM), its own noise and markdown modules, QuickJS for data islands (`webclaw-core/Cargo.toml`).
  - **Proxies:** a proxy pool picked at random per browser (`render.rs:1757-1761`).
  - **Sessions:** wreq `cookies`; persistent Chrome profiles (`render.rs:717`).
- **Fundamentals:**
  - **Concurrency:** default multi-thread `#[tokio::main]` (`webclaw-cli/src/main.rs:3717`). One coordinator loop with `FuturesUnordered`, but **each page is a `tokio::spawn`** (`crawler.rs:673` via `transfer::spawn`, `transfer.rs:27-34`, which carries a task-local byte observer). The query mode adds `spawn_blocking` for page reading.
  - **Frontier:** `BTreeMap<FrontierKey, String>` plus a per-host `HashMap<String, BTreeSet<FrontierKey>>` (`crawler.rs:1054-1060`). The pop is limited to `shallowest in-flight depth + 1` (strict BFS waves, `crawler.rs:625-631`) and skips hosts whose `next_start` lies in the future. The cap is `max_pages×10` or 500,000 when uncapped (`crawler.rs:262`, `:588-591`).
  - **Dedup:** `HashSet<String>` of the normalised https form (`crawler.rs:1290-1296`).
  - **Bytes per URL** (estimate, from the data types): about 24 B String header plus the URL bytes (~70) plus about 9 B hashbrown slot and control per visited URL, so **~100 B**. A frontier entry stores the URL string once plus a key in two B-trees, so **~150–200 B**. All `PageResult`s are also kept in a `Vec` for the result (`crawler.rs:593`).
  - **Persistence cost:** the checkpoint re-serialises everything every second, O(visited+frontier) per write.
  - **HTTP:** `wreq =6.0.0-rc.29` (BoringSSL impersonation), `pool_max_idle_per_host(32)`, idle timeout 90 s, an optional public-DNS resolver (`tls.rs:545-580`).
  - **Allocator:** none set. **Release profile:** none in `Cargo.toml` (default: no LTO, 16 codegen units).
  - **Browser pool** (`render.rs:1775-1798`, `:2154-2222`, `:2388-2398`): one browser launched lazily on the first render; `MAX_TABS = 4`. **Per-host learned render verdict:** after 3 renders, a host where the render gave at least `max(2×http_words, http_words+50)` words in ≥80% of cases always renders, and one where it helped in ≤20% never renders. The escalation heuristic (`cloud.rs:505-546`): `<script` present and (fewer than 50 words both extracted and visible, or fewer than 200 words with an empty `#root/#app/#__next…` or a `<noscript>` asking for JS). **Prefetched-body reuse:** the HTTP response (body and headers, `Set-Cookie` included) is fulfilled into Chrome through `Fetch.fulfillRequest`, so the document is not fetched twice (`render.rs:952-963`, `:1460`).
- **Published benchmarks:** `benchmarks/README.md`, `methodology.md`. On 2026-10-12, v0.6.24, 78 URLs × 3 rounds, compared with Crawl4AI 0.9.4, Scrapling 0.4.15 and Trafilatura 2.3.1. webclaw `-f llm`: 91% ok, 75.9% fact recall, 2,761 median tokens, p50 0.25 s / p95 0.62 s, including CLI start-up. Other suites cover politeness (a local 30-page site, measured gaps), redirects, tables, selectors and an install/RSS comparison for Python. Quality and latency only: no throughput comparison with Spider or Firecrawl.
- **Relevance:** the extraction and quality benchmark design and the **learned per-host render verdict + prefetch fulfil** apply to Foxhound→Collie escalation. Not a throughput rival: spawn per page, a 1 s full checkpoint, no release tuning. **Compare candidate: low** (no headless, high-throughput crawl mode; the CLI crawls one site).

#### fastCRW (crw)

- **What:** a Firecrawl-compatible REST server (`/v1`, `/v2`, `/firecrawl/v2/*`), CLI, MCP and `crw-browse`. Crates on crates.io: crw-core, crw-crawl, crw-renderer, crw-extract, crw-server and others. SDKs: PyPI `crw` (client), npm `crw-sdk`/`crw-mcp`. The hosted API is api.fastcrw.com with managed proxies, rendering and search. All Rust.
- **Health:** created 2026-03-02; 1,234 stars, 96 forks, 72 issues, 589 PRs, 16 contributors; 1,264 commits in 2026, last 2026-11-07; v0.42.1 (2026-11-06). crates.io `crw-core` 17.7k downloads (14.3k in 90 days); PyPI `crw` 1.6k a month; npm `crw-mcp` 2.2k a month. The engagement looks organic.
- **Features vs Foxhound:**
  - **robots.txt:** yes; it matches path and query (`crates/crw-crawl/src/crawl.rs:361-372`) and fails open (`:325-333`).
  - **Politeness:** a process-wide limiter keyed by **eTLD+1**: rate limiter plus `ReservedSemaphore` (per-host concurrency plus an interactive reserve), with a TTL sweep every 64 calls (`crw-renderer/src/host_limiter.rs:1-60`).
  - **Durable resume:** **none**. Crawl state is in memory (`crawl.rs:346-348`), and the blog says so: https://fastcrw.com/blog/crw-architecture.
  - **Sitemaps:** yes, with an anti-bot escalation arm and budgets (`crawl.rs:780-860`, `:1020-1060`).
  - **Browser:** its own CDP client (`cdp.rs`, `cdp_conn.rs`, tokio-tungstenite) driving LightPanda, Chrome, Camoufox and a "cloak" tier.
  - **Extraction:** a `lol_html` streaming removal pass (`crw-extract/src/clean.rs:101`), then `scraper`/html5ever readability (`readability.rs`), then `htmd` to Markdown. PDFs are handled too.
  - **Proxies:** a BYOP rotator, sticky per host, the same egress for robots and sitemap fetches (`crawl.rs:280-305`, `:386-399`).
  - **Sessions:** reqwest `cookies` feature (`Cargo.toml:43`); `crw-browse` has storage tools.
- **Fundamentals:**
  - **The `/crawl` job loop is sequential.** `run_crawl_inner` takes a semaphore permit and then **awaits the fetch inline**, in a `while let Some(..) = queue.pop_front()` loop with no spawn and no `FuturesUnordered` (`crawl.rs:356-470`). Real concurrency per crawl job is 1, and `max_concurrency` is ineffective there. The same bug was found and fixed only in `discover_urls` (map), which now streams each BFS level with `buffer_unordered` (`crawl.rs:1209-1291`, whose comment says "Nothing was ever spawned, so real concurrency was 1"). The marketing blog's "N async workers … a bloom filter tracks visited URLs" does not match the code: `HashSet<String>`, and no bloom anywhere in `crates/`.
  - **Frontier:** `VecDeque<(String,u32)>` plus `HashSet<String>`. **All `ScrapeData` results are kept in a `Vec` until the job ends** (`crawl.rs:348`, `:679`). Progress is sent with empty `data` to avoid O(N²) cloning (`:681-690`). Bytes per URL: about 100 B for dedup plus about 100 B queued, plus the full page output for the life of the job.
  - **Extraction off the reactor:** `spawn_blocking` with a process-wide semaphore of `max(2, cores×2/3)` and an interactive reserve of a quarter (`crw-crawl/src/extract_pool.rs:1-40`).
  - **HTTP:** reqwest 0.13 (rustls), optionally wreq for impersonation.
  - **Allocator:** default. **Release profile:** `lto = true`, `codegen-units = 1`, `strip`, plus a `release-small` profile at `opt-level="z"` (`Cargo.toml`, `[profile.release]`).
  - **Browser pool** (`crw-renderer/src/browser_pool.rs:1-13`, `:207-236`): long-lived CDP connections with **an ephemeral browser context per navigation** (`recycle_after_navs: 1`), size 4, idle 300 s, health check after 60 s, and tight 1–2 s timeouts for context create, dispose and target close. A per-slot `std::sync::Mutex` is never held across `.await`.
  - **Renderer ladder:** HTTP → LightPanda → Chrome. **Per-host renderer preference learning** from a sliding window of LightPanda-specific failures, held in a bounded `moka` cache (`preference.rs:1-20`). A **conditional hedge races LightPanda and Chrome** (`lib.rs:452-470`). Detector heuristics cover SPA shells, thin HTML, vendor walls and Cloudflare (`detector.rs:11-660`).
- **Published benchmarks:** `BENCHMARKS.md` (2026-05-08): Firecrawl's public 1,000-URL dataset (HuggingFace `firecrawl/scrape-content-dataset-v1`), concurrency 5, timeout 120 s. Truth-recall: crw 63.74%, crawl4ai 59.95%, Firecrawl 56.04%. p50: 1,914 / 1,916 / 2,305 ms. Idle RSS of about 14 MB (`docker top`, v0.30.0). Harness: `bench/diagnose_3way.py`. Scrape quality only, no crawl throughput.
- **Relevance:** the useful ideas are **context-per-navigation over long-lived CDP connections**, the **per-host renderer learning and hedging**, and reserved interactive lanes in the semaphores. Its crawl engine is weak on fundamentals: sequential per job, no durability, results held in memory. pagesift measured crw at 206 pages/s with 1.4 GB on its wide scenario (`pagesift/docs/benchmarks.md:236-246`). **Compare candidate: medium**: it is the best-known Rust "Firecrawl alternative", and beating it is cheap and credible, but it needs `crw serve` plus job polling, as in pagesift's harness.

#### DonSeTch

- **What:** an MCP/CLI with four tools (fetch, search, crawl, screenshot). Its own fetch engine, `v5core`, is built on `wreq =6.0.0-rc.31`, `btls =0.5.6` (a BoringSSL fork, vendored `btls-sys` patch for ClientHello parity) and `deno_core 0.413` (V8) to run bot-wall challenge programs in-process without a browser (`v5core/Cargo.toml`, `Cargo.toml:198-200`). npm `donsetch` (9.7k downloads a month). It has no hosted product: it is sponsorware with a Bright Data affiliate link (`README.md`).
- **Health:** created 2026-08-07; 975 stars, 64 forks, 123 issues, 307 PRs, 21 contributors; 1,160 commits in 2026, last 2026-11-07; v4.10.5 (2026-11-03), with v5.0.0-beta.3 on main. Engagement is consistent with the stars.
- **Features vs Foxhound:**
  - **robots.txt:** yes, with RFC 9309 "unreachable" handling (`src/crawl/mod.rs:157-205`).
  - **Politeness:** the **Crawl Governor**, paced per (host, lane = proxy). A host-wide PenaltyBox spans lanes. An EWMA latency above 3× baseline slows the crawl proactively. Jitter applies everywhere; the base gap is 200 ms plus jitter. Learned ladders persist across restarts (`src/crawl/governor.rs`, header). A **cross-process host pace file** gives one atomically written JSON per host under `<cache>/host-pace/`, capped at 2k files with a 7-day TTL (`src/crawl/host_pace.rs`, header).
  - **Durable resume:** a disk-backed resume-token store with one immutable file per token under `<cache>/crawl-resumes/` and an mtime TTL (`src/crawl/mod.rs:526-560`).
  - **Sitemaps:** yes, sitemap phase first (`mod.rs:327-331`).
  - **Browser:** none on the read path. Chromium serves screenshots and `login` only (`README.md:201`).
  - **Extraction:** its own, on `scraper` 0.27, run off the worker for PDFs (`mod.rs:1839-1845`).
  - **Proxies:** yes (`src/egress/proxy.rs`). **Sessions:** cookies (`wreq` cookies) and `donsetch login`.
- **Fundamentals:**
  - **Concurrency:** a fixed worker pool of `concurrency` tokio tasks, **default 1 = "pure polite serial"** (`mod.rs:458-459`, `:511`, `:1361-1399`).
  - **Frontier:** `BinaryHeap<Frontier>` (url, score, depth, retries, referer) plus `HashSet<String>`, with tracking parameters stripped and jitter within the head window (`src/crawl/frontier.rs:1-30`, `:196-301`).
  - **Release profile:** `lto = "fat"`, `codegen-units = 1`, `panic = "abort"`, `strip` (`Cargo.toml:148-152`). **Allocator:** default.
  - It is designed for agent-sized crawls, not throughput.
- **Published benchmarks:** README claims only ("first-try content on a hard corpus went from 13/28 to 26/28 once learned, with 40% fewer requests"). No throughput benchmark found.
- **Relevance:** a politeness idea for Foxhound: **latency-EWMA pre-emptive slowdown plus a per-host penalty box shared across proxy lanes** (Foxhound's Politeness actor). The cross-process pace file is irrelevant for a single Run. **Compare candidate: no** (serial by default, no browser crawling).

#### pagesift

- **What:** a self-hosted Rust binary: CLI, Firecrawl v1/v2 and Jina Reader-compatible APIs, MCP, and bindings for Python (`pagesift-py`), Node and a C ABI for about 12 languages. It covers web crawling, PDF/Office/OCR parsing and an agent mode. About **313k lines of Rust in 37 crates**, including a **vendored rustls fork** (`pagesift-rustls`, ~49k lines) for fingerprints, its own HTML parser, its own KV store and its own CDP client. Commercial licence offered (`docs/COMMERCIAL.md`). Comments are mostly in Chinese.
- **Health:** the GitHub repository was created 2026-10-20 and its first commit is dated 2026-10-20 (`gh api …/commits?page=617`). 617 commits since, 609 by YOLOW92 and 8 by `claude`. 251 stars, 32 forks, **1 watcher, 2 issues, 3 PRs, 2 contributors**. v1.2.0 (2026-11-07). npm `pagesift` 4.7k a month; PyPI 2.1k last week. **Anomaly:** a 300k-line codebase appearing in 18 days, almost certainly agent-generated, with stars far above engagement. Stars could not be verified (token limitation).
- **Features vs Foxhound:**
  - **robots.txt:** yes (policy layer; 0 violations in its own wild benchmark).
  - **Politeness:** per-host interval (default 1 s), per-host concurrency, a global concurrency ceiling, Retry-After handling, and **interval widening on 429** (`pagesift-engine/src/crawl/runner/mod.rs:65-68`, `:134-140`; `docs/benchmarks.md:290`).
  - **Durable resume:** yes. Newly enqueued URLs are appended to a store list `frontier` per job; on restart, enqueued URLs are re-marked seen and pending ones re-queued in their original order (`runner/mod.rs:70-108`; `pagesift-api/src/jobs/sink.rs:305-340`). The store is its own engine (`pagesift-store`: crc32 records, blake3, snapshots, no SQLite).
  - **Sitemaps:** yes, with a budget-bounded read (VOI stop) and 3-level index depth (`crawl/mod.rs` header).
  - **Browser:** its own CDP client (`pagesift-cdp`) that picks **Lightpanda → moli → Chrome/chrome-headless-shell** from an adaptive pool.
  - **Extraction:** its own `pagesift-html`, a zero-copy parser whose text and attributes borrow the source, with **nodes in one `Vec` linked by u32 indices and tags as an enum**; it depends only on `memchr` (`crates/pagesift-html/src/lib.rs:1-8`, `Cargo.toml`).
  - **Proxies:** yes (`pagesift-net`). **Sessions:** explicit `CookieSession` with no shared global jar (`docs/http-sessions.md`).
- **Fundamentals** (crawl runner `pagesift-engine/src/crawl/runner/mod.rs:53-200`):
  - **No spawn per page:** one `FuturesUnordered` in the crawl task, refilled while `inflight < concurrency`.
  - **Dedup as `HashSet<u64>`** of a fixed-key `DefaultHasher` (SipHash-1-3) over the normalised key (`crawl/mod.rs:93-99`): **about 8 B per URL plus about 1 B of control, ~10–18 B per URL** at hashbrown load factors. The collision probability is n²/2⁶⁵, about 3×10⁻⁶ at 10⁷ URLs.
  - **Queue bounded at `limit`:** "URLs found beyond the limit would never be reached, so they are neither enqueued nor recorded in the dedup table; memory does not grow with discovered links" (`runner/mod.rs:58-60`).
  - **Tiered queue:** URL templates that keep producing duplicate content are demoted behind normal URLs, and learned rewrite rules (`index.html`, `?print=1`, `more/`) are re-applied at pop time (`runner/mod.rs:62-64`, `:149-153`; `pagesift-frontier/src/frontier/learn.rs`).
  - **Host-aware pop:** pop skips hosts at their per-host in-flight cap (`busy` closure, `runner/mod.rs:144-146`).
  - **Retries** are class-based with a due-time heap, and the crawl never blocks on them (`retry.rs`).
  - **HTTP:** its own client on `hyper 1` (h1+h2) with the forked rustls, chardetng and brotli-decompressor (`pagesift-net/Cargo.toml`), plus reqwest for some paths.
  - **Allocator:** a custom global allocator, **`SmallCache`** (`crates/pagesift-sys/src/alloc.rs:1-30`, `:391-420`; installed in `crates/pagesift/src/main.rs:25-27`). Allocations of 512 B or less in 16 size classes get per-thread LIFO free lists with no locks or atomics, carved from 64 KB chunks. Batches of 32 move to a NUMA-sharded global pool. Each thread's cache is capped at its share of L2. Larger blocks go to glibc with `mallopt(M_MMAP_THRESHOLD = 256 KB)` and `M_ARENA_MAX = max(4, cpus/4)` (`pagesift-sys/src/lib.rs:23-52`). The comments cite callgrind: about 10% of PDF instructions were in malloc/free.
  - **Release profile:** `lto = "fat"`, `codegen-units = 1` (`Cargo.toml`).
- **Published benchmarks** (`docs/benchmarks.md`, harness `benchmarks/crawl/`): a local multi-host origin (`s{i}.localhost` on one port, real corpus pages, 10 ms added latency, robots allow-all), measured by `measure.py` (wall time, CPU, peak PSS of the process tree, killed at available memory minus 2 GB).
  - **wide** (200 sites × 100 pages, interval 0, 8 per host): pagesift **3,500–4,400 pages/s at 280–316 MB**; Spider 2.53.9 2,000–2,700 pages/s at **5.5 GB**; crw 206 pages/s at 1.4 GB; Crawl4AI 15 (single process) or 42 (multi-process, 7.8 GB).
  - **huge** (1,000 × 50): pagesift ~3,300 pages/s at ~390 MB; Spider stopped at over 9.5 GB.
  - **wild** (40 seeded messy sites, 60-page budget per site): coverage 0.75 vs Spider 0.36–0.47, waste 0.12 vs 0.37–0.45. Spider is fastest (1.4 s vs 21 s on fast).
  - **Browser:** auto pool ~80–95 pages/s; Lightpanda only 70.9; chrome-headless-shell 33; Chrome 11.6.
  - **Caveat on the Spider arm:** one `Website` per site, each with `subscribe(256)` and Markdown conversion in the receiver, default features `sync,encoding,time` (`benchmarks/crawl/src/bin/spider_crawl.rs:32-37`, `Cargo.toml:15-16`). 200 broadcast buffers of 256 full pages each can hold up to about 51k pages if receivers lag, which may explain much of Spider's 5.5 GB. The README also admits that on a one-host replay pagesift does 47.5 pages/s vs Spider's 156.6 because of its 1 s default interval (`docs/benchmarks.md:40-44`). Single-author numbers, not independently reproduced.
- **Relevance:** **high for ideas.** The u64 dedup with a limit-bounded queue, no spawn per page, a host-busy-aware pop, a duplicate-template-demoting frontier, the SmallCache allocator with mallopt tuning, the zero-copy arena HTML parser, and an append-only enqueued log for resume all map onto #2138, #2165, #1838 and #2194. Its crawl-benchmark harness (multi-host local origin, PSS of the whole process tree, the "wild" trap and 429 simulator) is a ready design for `just compare` scenarios. **Compare candidate: high, as a contender and as a harness reference**, with the caveat that the codebase is 18 days old and machine-generated.

#### crawlberg (formerly kreuzcrawl)

- **What:** a Rust crawl core with bindings for Python, Node, Ruby, Go, Java, Kotlin, C#, PHP, Elixir, Dart, Swift, Zig and WASM, plus MCP and REST. Part of the xberg (formerly kreuzberg) document-extraction family, renamed from `kreuzcrawl` (`CHANGELOG.md:3871-3875`). Managed extras (proxy pools, tuned evasion, sessions, scheduling, billing) live in the closed **xberg-enterprise** (`README.md`). Its own Chromium client fork is `crawlberg-chromiumoxide` (crates.io).
- **Health:** created 2026-03-09; 195 stars, 29 forks, 540 issues, 251 PRs, 11 contributors; **2,327 commits in 2026**, last 2026-11-07; v1.13.0 (2026-11-06). crates.io `crawlberg` 53.6k downloads (38k in 90 days); PyPI 2.9k last week; npm 1.6k a month. Organic-looking for its age, and very high-churn.
- **Features vs Foxhound:**
  - **robots.txt:** yes (`crawlberg-robots`).
  - **Politeness:** `PerDomainThrottle`, with adaptive `record_response` and `set_crawl_delay` (`src/defaults/rate_limiter.rs:167-226`, `src/traits.rs:210-219`).
  - **Durable resume:** **not in OSS**. The default frontier is in-memory `VecDeque` plus `AHashSet<String>` (`src/defaults/frontier.rs`); `Frontier`, `CrawlStore` and `CrawlCache` are traits for users to back (`src/traits.rs:125-300`). There is a conditional-revalidation cache trait (ETag / Last-Modified, `traits.rs:288-305`).
  - **Sitemaps:** yes.
  - **Browser:** a chromiumoxide fork, `BrowserPool` (one persistent Chrome with bounded concurrency), WAF detection for 8 vendors (`src/browser_pool.rs`, `src/waf/`).
  - **Extraction:** `tl` (astral-tl) plus html5ever, then `html-to-markdown-rs` 3.21 (Cargo.toml workspace), with BM25 relevance and "fit-content" pruning.
  - **Proxies:** a per-host rotation trait (`src/proxy.rs:1-5`). **Sessions:** cookie jars plus Basic, Bearer and header auth.
- **Fundamentals:**
  - **Concurrency:** `JoinSet` **spawn per fetch** plus a `Semaphore` (`src/engine/crawl_loop.rs:548-807`). The tokio features are `rt`, `macros`, `sync` and `time` (no `rt-multi-thread` in the core; the binding hosts choose).
  - **Frontier:** a bounded "window" handed to a `CrawlStrategy` (BFS, DFS, best-first, adaptive).
  - **HTTP:** reqwest 0.13 with cookies; an optional hyper/tokio-rustls path for SSRF interception.
  - **Allocator:** default. **Release profile:** none in the workspace `Cargo.toml`.
  - **Pluggable design:** traits for Frontier, RateLimiter, CrawlStore, EventEmitter, CrawlStrategy, ContentFilter and CrawlCache, plus a Tower middleware stack.
- **Published benchmarks:** none found with a comparison (`tools/benchmark-harness` exists; no published results).
- **Relevance:** the trait seams match Foxhound's design space, but there is nothing to adopt on fundamentals. The `CrawlCache.get_stale` design for If-None-Match revalidation is a nice touch for recrawls. **Compare candidate: low–medium** (popular bindings, but an in-memory, spawn-per-page engine; worth adding only for ecosystem coverage).

#### servo-fetch (read in part)

- **What:** embeds Servo 0.7 (SpiderMonkey JS plus parallel CSS layout and a software renderer) to fetch, render and extract Markdown, JSON and PNG with layout- and visibility-aware boilerplate removal. CLI, Rust crate, Python and Node SDKs, MCP. Health: created 2026-04-25; 163 stars; 505 commits in 2026; v0.15.2 (2026-11-01); crates.io 1.6k downloads, PyPI 1.3k a month.
- **Crawl:** BFS with `VecDeque` plus `HashSet<String>` and a content-hash set, defaults concurrency 1 and delay 500 ms, `JoinSet` (`crates/servo-fetch/src/crawl.rs:3-57`, `:413-423`). One-use worker process per isolated session.
- **Release profile:** `lto=true`, `codegen-units=1`, `opt-level="s"`, `panic="abort"`.
- **Benchmarks:** README, Apple M3 Pro, vs Playwright: static-small ~231 vs ~645 ms, spa-heavy ~331 vs ~798 ms, **peak RSS 51–64 MB vs 300–328 MB**. Extraction word-F1 0.819 vs Readability's 0.728 on 8 fixtures (`benchmarks/README.md`).
- **Relevance:** Collie, for an alternative render engine (in-process, about 60 MB) for light JS pages; layout-aware extraction is an idea for #2164 only if a layout is available. **Compare: no** (crawl is an afterthought); possibly a render-engine data point for the browser scenario.

#### crawlee-rs and crawlee-storage (Apify)

- **What:** `apify/crawlee-rs` is a Rust port of Crawlee: `BasicCrawler`, `HttpCrawler` and `HtmlCrawler`, the same storage format, SessionPool, an autoscaling `ConcurrencySystem`, robots.txt, sitemaps and `ThrottlingRequestManager` (`README.md` crates table). Created 2026-09-30, 1 star, 33 commits, no release, not on crates.io. The README calls the status early. `apify/crawlee-storage` is the Rust core of the Crawlee file-system storage clients (dataset, KV store, request queue; atomic temp-then-rename writes, `crawlee-storage/src/utils.rs:79-103`), shipped to Node as `@crawlee/fs-storage-native`. **Crawlee for JS 4.0 beta's `@crawlee/fs-storage` depends on it** (npm registry), so the JS Crawlee arm in `just compare` already runs Rust storage if it is on v4.
- **Fundamentals (crawlee-rs):** **link extraction streams the body through `lol_html` without a DOM**; the DOM (`scraper`) is parsed lazily, once, on the blocking pool, only when the handler asks for it (`README.md` example, `crawlee-utils/Cargo.toml`). Release profile: `lto="thin"`, `codegen-units=1`. Durable: a request queue persisted as one JSON file per request plus a `PersistState` event every 60 s (`README.md`, Storage/Events).
- **Published benchmark** (`README.md`, Benchmark): local fixture of 58 KB listings, 4 vCPU. 5,000 pages at concurrency 50: crawlee-rs **2,326 pages/s, 1.32 ms CPU per page, 120 MiB** vs Crawlee JS Cheerio 87 pages/s, 13.46 ms, 386 MiB. With 100 ms latency at concurrency 100: 856 pages/s, 72 MiB.
- **Relevance:** **compare candidate: medium–high once usable.** `just compare` already runs Crawlee, and crawlee-rs is Apify's own Rust answer, with a published CPU-per-page metric of the same kind as Foxhound's instructions per page. "Lazy DOM only when asked, streaming link extraction" matches #2164.

#### Firecrawl's Rust (triaged, source read)

- Firecrawl's engine is Node/TypeScript, with Go for HTML→Markdown (`apps/go-html-to-md-service`, `apps/api/sharedLibs/go-html-to-md`). Its Rust is **`apps/api/native` (`firecrawl_rs`)**, an N-API cdylib (`napi 3`) of helpers: link filtering and URL regex validation, sitemap XML parsing (`roxmltree`), robots (`texting_robots`), HTML transforms (`lol_html` 2.6 plus `kuchikiki`), metadata, links, images and attributes, Markdown post-processing, PDF inspection, and **"engpicker"** (`apps/api/native/Cargo.toml`; `src/html.rs`, `src/crawler.rs`, `src/engpicker.rs` @ `bf6739d9`).
- **engpicker** (`src/engpicker.rs:67-160`) scrapes sample URLs of a site with four engines (CDP basic and stealth, TLS-client basic and stealth). It takes the best CDP Markdown as the gold standard and computes a normalised Levenshtein similarity against the best TLS-client Markdown. If similarity clears a threshold over enough URLs, the site is marked "TLS client sufficient", so that site skips the browser.
- The `firecrawl` crate on crates.io (2.21.8) is only the API client (`apps/rust-sdk`, reqwest). `firecrawl/simd-html-to-md` is a 3-commit `std::simd` tokenizer→emitter experiment from February 2026.
- **Relevance:** engpicker is a third independent instance of "decide HTTP vs browser per site from measured content agreement" (with webclaw's verdict and crw's preference learning). **Compare: no** (not Rust at crawl level).

---

## Part A3: general-purpose, classic, SEO, link-checking, search-engine and recon crawlers

Reader notes, 2026-11-07. Health data from `gh api` (stars, created, pushed, 2026 commit count summed over
`commits?since=2026-01-01`, last commit date, latest release or tag) and the crates.io API. Source read from
shallow clones (SHA given per project); file:line cites are at that SHA. Spider ecosystem and AI/LLM
"Firecrawl-alternative" products are out of scope (other readers); they are only named where a crawler here
depends on them.

Legend: Y = present (cited below or in source), P = partial, N = absent, ? = not verified. "Depth" = deep (source
read for fundamentals), medium (key modules read), triaged (health + one line + feature grep).

| Project | Kind | URL | Licence | Stars | Last release (date) | 2026 commits / last commit | Maintained? | Robots | Politeness | Durable resume | Sitemaps | Browser | Extraction | Proxies | Sessions | Depth |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| let4be/crusty | broad crawler (multi-node) | https://github.com/let4be/crusty | GPL-3.0 | 88 | crate 0.12.0 (2021-06-06), tag v0.12.0 | 0 / 2021-11-27 | No (dormant 5 y) | Y | Y per domain + per /24 IP | N (Redis jobs with TTL, droppable) | N | N | links only (lol_html) | N | N | deep |
| let4be/crusty-core | crawler library | https://github.com/let4be/crusty-core | GPL-3.0 | 16 | crate 0.82.0 (2021-11-27) | 0 / 2023-01-04 | No | Y | Y (N workers/domain + delay) | N | N | N | pluggable parser | N | N | deep |
| StractOrg/stract | search engine with distributed crawler | https://github.com/StractOrg/stract | AGPL-3.0 | 2,372 | none (no tags) | 0 / 2025-03-24 (archived) | No (archived) | Y | Y adaptive (2^k × fetch time) | P (job plan file + pointer) | Y | N | full DOM (kuchiki fork), webpage model | N | N | deep |
| lycheeverse/lychee | link checker | https://github.com/lycheeverse/lychee | Apache-2.0/MIT | 4,102 | lychee-v0.24.2 (2026-05-01) | 283 / 2026-11-02 | Yes | N | Y per host (semaphore + token bucket + backoff) | P (`.lycheecache` result cache) | N | N | links from HTML/MD/CSS | ? (reqwest env) | Y cookie jar | deep |
| janreges/siteone-crawler | SEO/QA site crawler (rewritten PHP→Rust) | https://github.com/janreges/siteone-crawler | MIT | 998 | v2.5.1 (2026-06-27), tag v2.6.1 | ? (API count not run) / 2026-09-27 | Yes | Y | P (global req/s only) | N (HTTP cache dir only) | Y (input + generator) | Y (chromiumoxide, default feature) | HTML→Markdown, offline clone | Y | P (headers/cookies) | medium |
| mattsse/voyager | crawler/scraper library | https://github.com/mattsse/voyager | Apache-2.0 | 765 | crate 0.2.1 (2022-01-12) | 0 / 2024-12-14 | No | Y | Y per-domain delay | N | N | N | scraper DOM | N | N | medium |
| epi052/feroxbuster | recursive content discovery (recon) | https://github.com/epi052/feroxbuster | MIT | 8,155 | v2.13.1 (2025-12-13) | 21 / 2026-11-07 | Yes | P (extracts robots paths) | Y rate limit + auto-tune + auto-bail | Y (`--resume-from` state file) | N | N | link extraction | Y | Y (headers/cookies) | medium |
| untitaker/hyperlink | static-site link checker (local files) | https://github.com/untitaker/hyperlink | MIT | 231 | 0.3.2 (2026-06-27) | 20 / 2026-07-30 | Yes | N/A | N/A | N | N | N | links (html5gum custom emitter) | N/A | N/A | medium |
| wojcikdominik/openspider | SEO crawler (Screaming-Frog-like, Tauri) | https://github.com/wojcikdominik/openspider | Apache-2.0 | 31 | v1.0.0-beta.2 (2026-11-04) | 2,479 / 2026-11-06 | Yes (anomalous velocity) | Y | Y per host, additive-increase, multiplicative-decrease | Y (SQLite crawl file) | Y | Y (chromiumoxide) | Y SEO, html5ever DOM | Y | Y | medium |
| satvik007/millipede | Crawlee-like library | https://github.com/satvik007/millipede | MIT/Apache-2.0 | 1 | crate millipede-core 0.1.1 (2026-07-19) | 98 / 2026-07-19 | Unclear (quiet since July) | Y | Y same-domain delay, additive-increase, multiplicative-decrease autoscale | Y (fs storage) | Y | Y (chromiumoxide) | scraper DOM | Y | Y session pool | medium (benchmarks read) |
| spire-rs/spire | crawler framework (tower-like) | https://github.com/spire-rs/spire | MIT | 6 | v0.11.0 (2026-11-06) | 75 / 2026-11-06 | Yes | Y RFC 9309 | Y per host, adaptive concurrency | Y (redb / sqlx frontier crates) | Y | Y (thirtyfour/WebDriver) | typed selectors | ? | ? | medium |
| spronta/crawlie | SEO + GEO audit crawler, CLI+MCP+Tauri | https://github.com/spronta/crawlie | MIT (repo says NOASSERTION) | 117 | v0.5.3 (2026-06-29) | 169 / 2026-10-26 | Yes | Y | P (bounded concurrency) | P (SQLite page store, not resume) | Y | Y (render.rs) | SEO rules, scraper DOM | ? | ? | medium |
| meilisearch/scrapix-v2 | distributed crawler + indexer | https://github.com/meilisearch/scrapix-v2 | none declared | 5 | none | 282 / 2026-05-22 | Unclear (quiet since May) | Y | Y per domain | Y (RocksDB, Kafka) | Y | Y (chromiumoxide CDP + WebDriver) | scraper DOM, AI enrichment | Y | ? | triaged+ |
| spyglass-search/spyglass | personal search engine with crawler | https://github.com/spyglass-search/spyglass | AGPL-3.0 | 2,717 | v2024.11.1 (2024-11-18) | 0 / 2025-06-27 (archived) | No (archived) | Y | Y (≤2 in flight per domain) | Y (SQLite crawl_queue) | ? | N | readability-like | N | N | triaged+ |
| floneum/kalosm (crawl module) | LLM toolkit's crawler | https://github.com/floneum/floneum | Apache-2.0 | 2,235 | kalosm-0.4.0 (2025-02-09) | 38 / 2026-10-11 | Yes (repo), crawl module small | Y (texting_robots) | P (one sequential task per origin) | N | N | Y (headless mode) | page→text | N | N | triaged+ |
| AkashPriyadarshii/jev-seo | SEO/GEO CLI+MCP | https://github.com/AkashPriyadarshii/jev-seo | MIT | 99 | v0.4.0 (2026-10-04) | 131 / 2026-11-05 | Yes | Y | ? | P (SQLite) | Y | P | SEO rules | Y | ? | triaged |
| Kirky-X/crawlrs | "enterprise" scraping platform | https://github.com/Kirky-X/crawlrs | Apache-2.0 | 1 | v0.2.0-rc.2 (2026-09-21) | ? / 2026-10-21 | Yes (single author, 253 k LOC) | Y | Y | P | Y | Y (Playwright etc.) | Y | Y | Y | triaged |
| hominee/dyer | crawler framework (actors) | https://github.com/hominee/dyer | none | 126 | crate 3.3.2 (2022-07-27) | 0 / 2025-08-05 | No | N | ? | P (SQL via middleware) | N | N | user code | Y | ? | triaged |
| Gonzih/crabler | scraper engine with derive macros | https://github.com/Gonzih/crabler | MIT | 95 | v0.1.27 (2022-01-08), crate 0.1.28 | 2 / 2026-03-26 | Barely | N | N | N | N | N | scraper DOM | N | N | triaged |
| Skallwar/suckit | site downloader (mirror) | https://github.com/Skallwar/suckit | Apache-2.0 | 811 | v0.2.0 (2022-04-28) | 2 / 2026-03-12 | Barely | N | P (delay) | N | N | N | rewrite links | N | Y cookies | triaged |
| a11ywatch/crawler | gRPC crawler, built on spider 2 | https://github.com/a11ywatch/crawler | MIT | 55 | v0.8.7 (2023-07-04) | 0 / 2024-08-26 (archived) | No | via spider | via spider | N | via spider | via spider feature | N | via spider | N | triaged |
| aichat-bot/crawly | small async crawler | https://github.com/aichat-bot/crawly | MIT | 28 | crate 0.1.9 (2024-06-13) | 0 / 2024-06-13 | No | Y | P | N | N | N | N | N | N | triaged |
| noahfraiture/coma | CLI scraper | https://github.com/noahfraiture/coma | MIT | 27 | tag v0.2 / crate 0.2.3 (2024-10-04) | 0 / 2025-02-02 | No (paused per README) | N | N | N | N | P | text/links/images | N | N | triaged |
| raydroplet/crawler-rs | crawler with graph GUI | https://github.com/raydroplet/crawler-rs | none | 76 | none | 52 / 2026-08-04 | Low | P | P | N | N | N | links | N | N | triaged |
| oscar-project/ungoliant | Common Crawl corpus pipeline (not a crawler) | https://github.com/oscar-project/ungoliant | Apache-2.0 | 178 | v2.0.0 (2023-02-24) | 0 / 2023-11-09 | No | N/A | N/A | N/A | N/A | N/A | WET text filtering | N/A | N/A | triaged |
| commoncrawl/cc-downloader | Common Crawl bulk downloader | https://github.com/commoncrawl/cc-downloader | Apache-2.0 | 99 | v1.0.1 (2026-08-05) | 42 / 2026-08-05 | Yes | N/A | Y (retry backoff + jitter, ratelimit) | P (skips done files?) ? | N | N | N | N | N | triaged |
| maxmindlin/scout-lang | crawling DSL over WebDriver | https://github.com/maxmindlin/scout-lang | Apache-2.0 | 122 | v0.7.2 (2024-08-20) | 0 / 2024-08-21 | No | N | N | N | N | Y (fantoccini) | DSL selectors | N | N | triaged |
| spyweb-app/spyweb | monitoring/scraping engine (Lua) | https://github.com/spyweb-app/spyweb | Apache-2.0 | 46 | v1.5.5 (2026-09-28) | ? / 2026-09-29 | Yes | N | P | Y (SQLite/redb) | N | ? | scraper DOM | Y | P | triaged |
| vincenzo-afk/NEXUS-ENGINE | toy search engine + crawler | https://github.com/vincenzo-afk/NEXUS-ENGINE | none | 38 | none | 20 / 2026-08-23 | Low | Y | P | P (SQLite) | Y | N | BM25 index | Y (Tor) | P | triaged |
| wojcikdominik → see openspider | | | | | | | | | | | | | | | | |
| neur0map/docrawl | docs→Markdown crawler | https://github.com/neur0map/docrawl | MIT | 50 | tag v0.1.1 / crate 0.1.6 (2026-03-21) | 7 / 2026-05-23 | Low | Y | P (Semaphore) | Y (sled cache, resume) | Y | N | lol_html → Markdown | P | P | triaged |
| spk/maman | crawler saving to Redis | https://github.com/spk/maman | MIT | 44 | crate 0.13.1 (2018-12-01) | 0 / 2021-06-13 | No | Y | ? | P (Redis/Sidekiq) | N | N | N | N | N | triaged |
| hahwul/urx | OSINT URL extractor (Wayback/CC/OTX) | https://github.com/hahwul/urx | MIT | 194 | 0.11.0 (2026-09-18) | 640 / 2026-11-07 | Yes | Y (robots/sitemap as sources) | N/A | N | Y | N | URL lists | Y | N | triaged |
| Y2Z/monolith | single-page saver (note only) | https://github.com/Y2Z/monolith | CC0-1.0 | 15,595 | v2.12.0 (2026-11-02) | 73 / 2026-11-06 | Yes | N/A | N/A | N/A | N/A | N | inlines assets | ? | Y cookies | triaged |
| mat-1/x227f | 88x31-button crawler | https://github.com/mat-1/x227f | WTFPL | 40 | none | 2 / 2026-10-30 | Barely | N | P | N | N | N | images | N | N | triaged |
| Minoru/minoru-fediverse-crawler | Fediverse instance crawler | https://github.com/Minoru/minoru-fediverse-crawler | AGPL-3.0 | 27 | tag 1.0.0 | 189 / 2026-11-04 | Yes | Y | Y | Y (SQLite schedule) | N | N | JSON APIs | N | N | triaged |
| Isona/dirble | directory brute-forcer | https://github.com/Isona/dirble | GPL-3.0 | 631 | v1.4.2 (2019-07-09), crate 2024-03-07 | 0 / 2025-03-30 | No | N | P | N | N | N | N | Y | Y | triaged |
| wihlarkop/kumo | "Scrapy for Rust" framework | https://github.com/wihlarkop/kumo | MIT | 8 | kumo-v0.5.0 (2026-06-03) | 236 / 2026-06-30 | Unclear | Y | Y per domain + EWMA auto-throttle | Y (file frontier) | Y | Y (chromiumoxide) | CSS/XPath/LLM | Y | ? | triaged (README only) |
| splch/mycel | single-binary search engine + crawler | https://github.com/splch/mycel | AGPL-3.0 | 5 | v0.4.0 (2026-09-01) | 67 / 2026-09-21 | Low | Y RFC 9309 | Y 1/host + Mercator 10× rule | Y (SQLite + WARC) | Y | N | tantivy index | N | N | triaged (manual only) |
| marsara9/lemmy-search | Lemmy search engine | https://github.com/marsara9/lemmy-search | AGPL-3.0 | 83 | tag v0.4.7 | 0 / 2023-08-03 | No | ? | ? | ? | N | N | N | N | N | triaged |
| aperio-search/aperio | lightweight search engine | https://github.com/aperio-search/aperio | NOASSERTION | 62 | v1.0.1 (2026-06-05) | 235 / 2026-08-21 | Low | ? | ? | ? | ? | ? | ? | ? | ? | triaged (health only) |
| IndexFlowing/IndexFlow-core | indexing-submission infra (not a crawler) | https://github.com/IndexFlowing/IndexFlow-core | none | 68 | none | 51 / 2026-10-15 | Low | N/A | N/A | N/A | Y (sitemap libs) | N | SEO checks | N/A | N/A | triaged |
| KingOfBugbounty/enumrust | subdomain enum + simple crawler | https://github.com/KingOfBugbounty/enumrust | MIT | 457 | none | 31 / 2026-09-04 | Low | ? | ? | ? | ? | ? | ? | ? | ? | triaged (health only) |
| neon-mmd/websurfx | metasearch (verified: NO crawler) | https://github.com/neon-mmd/websurfx | AGPL-3.0 | 1,228 | — | — / 2026-11-06 | Yes | N/A | N/A | N/A | N/A | N/A | N/A | N/A | N/A | triaged |

Other discovered names not tabled (1–5 stars, single-author, mostly 2026 and AI-assisted): dedsecrattle/Argus (1 star, 2026-03; claims "1B URLs in 1.2 GB" bloom), BenjaminSRussell/Rust-sitemap (1 star; redb + WAL + bloom, "governor" adapting 32–512 workers on commit latency), aaronlifton/fastcrawl (2 stars; one single-thread tokio runtime per shard, lol_html, 75 pages/s own figure), danieljake/multithreaded-bfs-webcrawler (0 stars), lopuhin/rust-broad-crawl (2016 experiment), oiwn/capp-rs (3 stars), bartTC/siteprobe (7 stars, sitemap validator), xberg-io/crawlberg (195 stars; AI/Markdown product, belongs to the AI cluster; 2,327 commits in 2026 — velocity anomaly). Libraries met on the way: Smerity/texting_robots (robots parser by a former Common Crawl engineer; 1.15 M downloads, 0 commits in 2026, last 2024-02-14), zeon256/fast-robots, spire-rs/kit (`robotxt`, `sitemapo`).

Established crawler operators and Rust (sweep): Common Crawl's only Rust repos are cc-downloader and a WARC annotator poster (crawler itself is not Rust); Internet Archive has two unrelated small Rust repos (logaggrd, RevisionChest); Brave's Rust repos are adblock/privacy/crypto tooling, no crawler published; Marginalia is Java; Mwmbl is Python (`gh api repos/mwmbl/mwmbl` language Python); Websurfx is a metasearch engine whose only `robots.txt` code serves its own file (`src/lib.rs:89`, `src/routes/mod.rs:57` @ 171c6d614056). The only established organisations with a published Rust crawler are Stract ApS (archived) and Meilisearch (scrapix-v2, 5 stars, quiet since 2026-05).

---

#### crusty + crusty-core (let4be)
- **What:** A broad (internet-wide) crawler. `crusty` is the node binary plus two Redis modules written in Rust (`crusty.queue`, `redis-calc` for top-k); `crusty-core` is the crawling library (job = one domain, task = one page). 100 % Rust; no bindings; no commercial offering. Clones: crusty @ b128bd4a8020 (2021-11-27), crusty-core @ 8fc5d5ed5f3f (2023-01-04).
- **Health:** crusty 88 stars, created 2021-05-20, 0 commits since 2021-11-27, crates.io `crusty` 0.12.0 (2021-06-06), 19,139 downloads; `crusty-core` 16 stars, 0.82.0 (2021-11-27), 111,672 downloads, 377 recent. Dead for five years; deps are hyper 0.14, trust-dns 0.22, lol_html 0.3.
- **Features vs Foxhound:**
  - robots.txt: yes, Google's matcher port `robotstxt-with-cache`, as a task filter and a load filter (`crusty-core/main/src/crawler.rs:133-135,168-170`); no Crawl-delay, no sitemaps (README "no custom delays or sitemap yet").
  - Politeness: per domain, `concurrency` workers (default 2) each sleeping `delay + rand(0..jitter)` after every task (`task_processor.rs:388-397`; defaults `conf/default.yaml:199-205`: concurrency 2, delay 1 s, jitter 1 s). Across domains, the Redis queue never hands out two domains with the same `addr_key` = first resolved IP masked to /24 (`conf/default.yaml:45-55`; `redis-queue/src/lib.rs:49-96,109-151`), i.e. politeness per subnet, not only per host. Job soft/hard timeouts 180 s / 300 s + jitter (`default.yaml:212-217`).
  - Durable resume: no. Jobs live in Redis sets with a TTL ("dropping job here and there is no biggie", `default.yaml:58-61`); in-processing keys expire after 10 min; per-domain page dedup is in memory only.
  - Sitemaps, browser, proxies, sessions: no. Extraction: link extraction only (lol_html `a[href]` handler, `workspace/main/src/parsers/lolhtml.rs:47-71`), html5ever alternative behind a Cargo feature.
- **Fundamentals:**
  - Concurrency: two multi-thread tokio runtimes (one for Crusty control, one for the crawler) (`main.rs:73-100`). `MultiCrawler` spawns `domain_concurrency` long-lived tasks (default `physical_cores * 40`, `crusty-core/main/src/config.rs:165`; 300 in default.yaml, 30,000 in the c5.metal profile) each pulling jobs from a bounded flume channel (`crawler.rs:368-400`). Each job spawns `concurrency` `TaskProcessor` tasks reading one unbounded per-job task channel (`crawler.rs:323-334`, `task_scheduler.rs:19-20`). No spawn per page.
  - HTML parsing off the runtime: a pool of dedicated OS threads (default = physical cores, optional core pinning) fed through a bounded channel sized `domain_concurrency * 10`; the closure is sent and the result comes back on a one-slot channel (`parser_processor.rs:9-77`, `task_processor.rs:210-273`, `config.rs:170-171`).
  - Frontier/dedup: per job a `HashSetDedup` = `Arc<Mutex<HashSet<String>>>` of full URL strings, checked early and committed last in the filter chain (`task_filters.rs:69-72,181-208`, `crawler.rs:119-142`); budgets cap a job at 1,000 links and 100 links per page (`default.yaml:186-192`). Estimate ≈ 24 B String + ~70 B URL text + ~1 B control at 7/8 load ≈ 100 B per URL, but only for the live domain jobs. Cross-domain: a node-local 2Q LRU of 25 M domain names (`default.yaml:250-254`, `crusty.rs:100`) in front of Redis, and in Redis a RedisBloom filter of finished domains (capacity 10 M, error 0.001 ≈ 1.8 B/domain) (`default.yaml:66-75`, `redis-queue/src/lib.rs:65-75,187-194`). Frontier is therefore domain-level; URLs within a domain are rediscovered each job.
  - Body: buffered into `BytesMut` up to `max_response_size` 2 MiB, gzip/deflate decoded after the full read, then handed to the parser as a `Read` (`task_processor.rs:60-80`). lol_html then reads it in 32 KiB chunks (`lolhtml.rs:64-71`) — streaming parser but non-streaming fetch.
  - HTTP: hyper 0.14 + hyper-tls (OpenSSL) with HTTP/1+2; one `hyper::Client` per TaskProcessor, so connections are reused only within one worker of one job (`task_processor.rs:359`, `crawler.rs:221-244`); a counting connector measures bytes per task (`hyper_utils.rs`). Optional local-address binding across several NICs, random per task (`crawler.rs:196-233`).
  - DNS: trust-dns async resolver pointed at a local Unbound recursive cache (`infra/unbound/conf/unbound.conf`: cache-min-ttl 1200, prefetch, so-reuseport), with IPv4/IPv6 blacklists (`resolver.rs:54-110`); a separate resolver stage (150 green threads by default, 500 in c5.metal) resolves discovered domains before they are enqueued, so each job carries its addresses and uses an `AsyncStaticResolver` — no DNS on the page hot path (`crawler.rs:311-318`, `default.yaml:257-270`).
  - Kernel tuning shipped with the profile: port range 4096-65535, `tcp_tw_reuse`, conntrack 1 M, tcp_rmem/wmem max 32 KiB (small socket buffers for tens of thousands of sockets), SACK off, `tcp_slow_start_after_idle 0` (`infra/profiles/c5.metal/sysctl.conf`). FD limit raised at start (`main.rs:57-62`).
  - Allocator: default (none set). Release profile: `opt-level 3`, `panic = "unwind"`, no LTO (`Cargo.toml`); crusty-core's own workspace uses `panic = "abort"`.
  - Buffered writers ("relabuf": release at soft cap, block producer at hard cap, or after a time) for queue inserts and ClickHouse metrics (`default.yaml:62-65,121-127`).
- **Published benchmarks:** README claims "~10 gbit/s on 48 core (96 HT) c5.metal" and "less than 1 core while saturating 100 mbit" (screenshot of a Grafana dashboard, `infra/resources/grafana.png`). No pages/s, no method, no reproducible harness. Not a benchmark.
- **Relevance:** design reference only. Worth taking: parser pool off the async runtime, DNS resolved once per site before scheduling, subnet-level politeness key, small socket buffers. Not a `just compare` candidate: dead deps, needs Redis + ClickHouse + Docker, broad-crawl semantics only.

#### Stract (StractOrg/stract) — crawler part
- **What:** An open-source web search engine (Stract ApS, Denmark) with its own distributed crawler, webgraph, ranking, tantivy fork. Rust (with a WASM client); AGPL-3.0; archived. Clone @ 8ac40b023e0a (2025-03-24).
- **Health:** 2,372 stars, created 2022-05-19, archived, last commit 2025-03-24, no releases or tags, 0 commits in 2026.
- **Features vs Foxhound:**
  - robots.txt: yes, in-tree `robotstxt` crate, cached per host for `robots_txt_cache_sec` 3,600 s (`crates/core/src/crawler/robots_txt.rs:57-85`; `config/defaults.rs:102-104`); every request goes through `RobotClient::get`, which refuses disallowed paths (`crawler/robot_client.rs:66-72`). Crawl-delay raises the job's minimum delay (`worker.rs:316-325`).
  - Politeness: one job per domain at a time, and a job crawls its URLs strictly sequentially (`crawler/mod.rs:168-170`, `worker.rs:306-396`). Delay after each fetch = max(fetch time, min_crawl_delay) × 2^politeness_factor, capped at max (`worker.rs:503-517`); factor starts at 2, decreases by 1 per success (floor 0) unless a 429 was ever seen, increases on 429 (ceiling 11); Retry-After honoured, and a Retry-After above the max delay abandons the site (`worker.rs:329-375,398-419`). Defaults: min 10 s, max 180 s (`config/defaults.rs:101-131`). Very conservative.
  - Durable resume: partial. The crawl plan is a file of `Job`s written once (bincode records with fixed-size postcard headers), read through `mmap`, with a separate pointer file rewritten after each pop (`crawler/file_queue.rs:31-182`); the coordinator serves jobs from it (`coordinator.rs:20-37`). A restarted coordinator resumes at the pointer; jobs in flight on workers are lost. No per-URL durability; output is WARC uploaded to S3 (`warc_writer.rs:60-154`).
  - Sitemaps: yes, from robots.txt, depth ≤ 5, fetched with the same delay (`worker.rs:287-304,632-677`). Browser, proxies, sessions: no. Extraction: full DOM parse (`Html::parse`, kuchiki fork) for links and the search index.
- **Fundamentals:**
  - Planning instead of a live frontier: the planner assigns each host a budget proportional to its harmonic centrality, groups URLs on disk by harmonic-rank bucket in `speedy_kv` stores, and writes per-domain jobs (`crawler/planner.rs:75-147,174-360`). Inside a job, a "wander" budget lets the worker follow newly found same-domain links ranked by an in-memory `BTreeMap<Url, f64>` of accumulated inbound weight (`wander_prioritiser.rs:24-58`, `worker.rs:214-248`).
  - Concurrency: `num_worker_threads` tokio tasks (config default 1 in `configs/crawler/crawler.toml`), each runs one job to completion; parallelism comes from many workers/machines; routers pick a random coordinator over the in-house `sonic` RPC (`router.rs:52-84`, `worker.rs:86-126`).
  - Dedup per job: `hashbrown::HashSet<Url>` of crawled URLs (`worker.rs:135`), `url::Url` ≈ 80 B inline + heap text ≈ 150 B/URL, bounded by the job's budget. Global dedup is done offline in the plan.
  - HTTP: one shared reqwest client per worker set, redirects disabled (handled manually as new queue entries), HTTP/2 keep-alive pings off, `Accept: text/html` (`robot_client.rs:24-46`); tries https first for http URLs (`worker.rs:473-490`). Body buffered fully up to 32 MB then decoded with encoding_rs (`crawler/mod.rs:322-354`).
  - Allocator: jemalloc (`tikv-jemallocator`) as `#[global_allocator]` (`crates/core/src/main.rs:34-39`). Release: `lto = "thin"`, `debug = true` (`Cargo.toml` `[profile.release]`).
  - Clever: adaptive delay proportional to measured fetch time (Mercator/Heritrix "delay factor" idea) with exponential back-off state; https-first upgrade; planning budgets from centrality.
- **Published benchmarks:** none found for the crawler.
- **Relevance:** reference for politeness (fetch-time-proportional delay, 429 state) and the plan-file idea (append-only record file + persisted read pointer). Not a compare candidate (archived, needs webgraph + S3 + coordinator).

#### lychee (lycheeverse)
- **What:** The most used Rust link checker (Markdown, HTML, reStructuredText, websites). `lychee-lib` library + `lychee` CLI; GitHub Action. Rust only. No commercial offering. Clone @ d119b686aba7 (2026-11-02).
- **Health:** 4,102 stars, created 2020-08-04, latest release lychee-v0.24.2 (2026-05-01), 283 commits in 2026, last 2026-11-02; crates.io `lychee` 252,132 downloads (80,484 recent), `lychee-lib` 230,337 (83,131 recent). Organic.
- **Correction to the brief:** lychee is **not** a recursive crawler today. The pipeline has a recursion channel, but the response stage returns an empty vector: `let recursive_uris = vec![]; // currently unused.` (`lychee-bin/src/commands/check.rs:125-130`). It checks the links in the inputs you give it.
- **Features vs Foxhound:** robots.txt: no. Politeness: yes, per host (below). Durable resume: partial — a `.lycheecache` CSV of URI → status + timestamp, reloaded with a max age, errors never cached (`lychee-bin/src/cache.rs:52-100`). Sitemaps, browser: no. Extraction: links from HTML (html5gum tokenizer or html5ever), Markdown (pulldown-cmark), CSS, plaintext, XML; fragment checking (`lychee-lib/src/extract/`). Proxies: not an explicit option (reqwest env proxies by default — not verified). Sessions: cookie jar file (`lychee-bin/src/config/mod.rs:703`).
- **Fundamentals:**
  - Request pipeline: a single `futures` stream; `.buffer_unordered(max_concurrency)` is the only concurrent stage (default 128, `config/mod.rs:826-827`); `select_with_strategy` would prefer recursive requests over input ones "because recursive requests consume memory" (`check.rs:100-104,108-121`). WaitGroup guards terminate the stream.
  - Per-host pool: `DashMap<HostKey, Arc<Host>>` created on demand (`ratelimit/pool.rs:30,118-140`). Each `Host` has its own reqwest client (host-specific headers/cookies), a `tokio::Semaphore` (default 10 concurrent per host), a `governor` token bucket with burst 1 (default 50 ms interval), an adaptive backoff, a per-host response cache `DashMap<(Method, Uri), CacheableResponse>`, and a per-URL async mutex so identical concurrent requests coalesce (`ratelimit/host/host.rs:42-107,158-184,237-245`; defaults `ratelimit/config.rs:10-14`).
  - Backoff: 2xx resets; 429 doubles from 500 ms to 30 s; 5xx adds 200 ms up to 10 s; `RateLimit`/`X-RateLimit-*` headers parsed by the `rate-limits` crate raise the backoff to the reset time, capped at 60 s (`host.rs:256-330`).
  - Parsing: html5gum streaming tokenizer as one extractor (no tree) and html5ever as the other (`extract/html/mod.rs:1-3`).
  - Allocator: none. Release profile: none in the workspace `Cargo.toml` (defaults).
- **Published benchmarks:** none in repo (a `benches` crate micro-benchmarks extraction only); third-party comparison in hyperlink's README is offline-mode only.
- **Relevance:** the per-host module is a clean reference for #2194 (semaphore + token bucket + header-driven backoff + request coalescing). Not a compare candidate (not a crawler).

#### SiteOne Crawler (janreges)
- **What:** Website crawler and QA toolkit (SEO, security, accessibility, performance audits, offline clone, Markdown export, sitemap generation, cache warming, CI gate). README: "Now rewritten in Rust ... The transition from PHP+Swoole to Rust resulted in 25% faster execution and 30% lower memory consumption" (`README.md:5`). GitHub languages: Rust 5.13 MB, HTML 42 kB (`gh api .../languages`). Desktop GUI exists separately. Clone @ f3d967b190a7 (2026-09-27). Repo has `CLAUDE.md` and `prompts/` (AI-assisted development).
- **Health:** 998 stars, created 2023-10-04, latest release v2.5.1 (2026-06-27; tag v2.6.1 exists), last commit 2026-09-27; crates.io `siteone-crawler` 231 downloads (distributed as binaries).
- **Features vs Foxhound:** robots.txt: yes (`src/engine/robots_txt.rs`, cache per domain:port `crawler.rs:150-153`). Politeness: a single global `max_reqs_per_sec` turned into an "optimal delay" (`crawler.rs:182`); no per-host concurrency. Durable resume: no (HTTP cache dir only, `README.md:270,532`). Sitemaps: input (index, gzip) and generator (`README.md:110-118,239-249`). Browser: chromiumoxide CDP, on by default as a Cargo feature (`Cargo.toml` `[features] default = ["browser"]`), renders each HTML page after a plain fetch (fetched twice, `README.md:270`). Extraction: HTML→Markdown converter, offline exporter. Proxies: `--proxy` (`README.md:531`). Sessions: custom headers/cookies, HTTP auth.
- **Fundamentals:** coordinator loop pops from `queue_order: Mutex<VecDeque<String>>` + `queue: DashMap<String, QueueEntry>`, acquires a `Semaphore(workers)` permit, `tokio::spawn`s one task per URL, and when the queue is empty sleeps 10 ms and polls again (`src/engine/crawler.rs:270-345`). Visited/skipped are `DashMap<String, Entry>` with three owned strings per entry (`crawler.rs:78-126`) ≈ 200+ B per URL. Parsing: `scraper` full DOM. HTTP: reqwest 0.13 rustls, hickory resolver with a resolve cache (`crawler.rs:156`). Allocator: default; README notes musl builds are "~50–80% slower due to musl memory allocator" (`README.md:292`). No `[profile.release]` section seen.
- **Published benchmarks:** only the 25 %/30 % PHP→Rust claim, no method.
- **Relevance:** feature-rich SEO tool, weak engine (10 ms polling, global rate only, spawn per URL, per-URL string triples). Possible compare candidate for the "HTTP raw" scenario since it ships a single binary with a CLI and `--workers`, but it does audit work on every page; low priority.

#### voyager (mattsse)
- **What:** Small crawl/scrape library: a `Scraper` trait with typed states, the `Crawler` is a polled `Stream` (no spawned tasks). Author also wrote chromiumoxide. Clone @ bd02e410dc59 (2024-12-14).
- **Health:** 765 stars, crate 0.2.1 (2022-01-12), 13,908 downloads, 0 commits in 2026.
- **Features/fundamentals:** robots via `robotstxt` crate (`src/robots.rs`); per-domain `RequestQueue` with fixed or random delay implemented with `futures_timer::Delay` inside `poll_next` (`src/requests.rs:34-107,110-155`); `HashSet` dedup and `in_progress_*` vectors polled manually (`src/lib.rs:150,264-335`); full DOM via `scraper::Html`; reqwest 0.12. No persistence, sitemaps, browser, proxies.
- **Relevance:** none beyond the idea of a runtime-agnostic poll-driven crawler; not maintained.

#### feroxbuster (epi052)
- **What:** Recursive content discovery (wordlist brute force of paths, with link extraction and recursion). Recon/security, not a polite crawler, but its scan-limiting design is relevant. Clone @ e14ddf03323d (2026-11-07).
- **Health:** 8,155 stars, created 2020-08-22, v2.13.1 (2025-12-13), 21 commits in 2026, last 2026-11-07, crates.io 148,398 downloads (11,373 recent).
- **Fundamentals worth noting:**
  - Limits at three levels: `--threads` per scan, `--scan-limit` concurrent recursive scans via a `DynamicSemaphore` (`src/event_handlers/scans.rs:84-90`), `--parallel` processes.
  - `--rate-limit` uses a `leaky-bucket` limiter refilled every second with `limit` tokens, starting half full to cut the initial burst (`src/scanner/requester.rs:107-127`).
  - Auto-tune: when errors, 403s or 429s exceed thresholds after at least one full round of requests (max(threads, 50)), the rate is moved through a precomputed 255-node "LimitHeap": left child = increase, right child = decrease, each step halving the distance to the parent — a binary search for the highest error-free rate (`src/scanner/limit_heap.rs` header; `requester.rs:179-220,221-300,338-392`). A CAS on `cooling_down` makes exactly one task enforce policy (`requester.rs:181-189`). Auto-bail aborts the scan with most errors (`requester.rs:394-430`).
  - Resume: Ctrl+C writes `ferox-<host>-<ts>.state`; `--resume-from` reloads it (`src/utils.rs:595`, `src/parser.rs:59-64`, `main.rs:376`).
  - Release: `lto = true`, `codegen-units = 1`, `panic = 'abort'` (`Cargo.toml`). Allocator default. reqwest 0.12 native-tls.
- **Relevance:** the LimitHeap auto-tune is a neat, cheap way to search a per-site rate (#2194). Not a compare candidate (different job).

#### hyperlink (untitaker)
- **What:** Link checker for a built static site (local files only; "does not know how to check external links", `README.md:180`). Clone @ cdfc6408b2ee (2026-07-30).
- **Health:** 231 stars, 0.3.2 (2026-06-27), 20 commits in 2026, crates.io 48,689 downloads.
- **Fundamentals:** jwalk parallel directory walk + rayon; html5gum tokenizer with a custom `Emitter` that discards comments, doctypes and text and only buffers tag names and attributes (`src/html/parser.rs:209-299`); links are allocated in a per-thread `bumpalo` arena and parser buffers are reset and reused between files (`parser.rs:18-35,39-62`), `recycle_vec`; blake3 paragraph hashes for anchor checks. Release: `lto = true`, `codegen-units = 1`, `panic = 'abort'`.
- **Published benchmarks:** "1.1 GB of HTML ... in 4 seconds on a MacBook Pro 2018" (`README.md:5-7`), and an "unserious" ranking vs lychee/liche/htmltest/muffet (`README.md:211-240`). No harness.
- **Relevance:** best example in the cluster of per-page allocation discipline in link extraction (#1838/#2164): tokenizer-level emitter, arena per page, reused buffers. Not a compare candidate (no network).

#### OpenSpider (wojcikdominik)
- **What:** Desktop (Tauri) + CLI + MCP SEO crawler positioned against Screaming Frog. 379,150 lines of Rust in 17 crates (`os-engine`, `os-store`, `os-fetch`, `os-render`, …), German code comments and commit messages, specs and gate reports in `docs/`. Clone @ 6d879a261942 (2026-11-07).
- **Health:** 31 stars, created 2026-10-13, v1.0.0-beta.2 (2026-11-04), **2,479 commits in 2026, all since 2026-10-13** (≈100/day) — agent-driven development; stargazer timestamps could not be sampled (token lacks access, HTTP 403). Treat claims cautiously, but the engine documents its own measurements.
- **Features vs Foxhound:** robots (`robots_glue.rs`), per-host concurrency + Crawl-delay + adaptive throttling (additive increase, multiplicative decrease) (`host_pace.rs:1`), durable resume from the SQLite crawl file (`resume.rs` header), sitemaps (`sitemaps.rs`, `sitemap_ingest.rs`), rendering via chromiumoxide (`os-render`), SEO extraction (html5ever DOM, `os-parse`), proxies (reqwest socks + system proxy), cookie sessions (`cookies.rs`, `session_guard.rs`). Feature-equivalent to Foxhound's crawl side.
- **Fundamentals (frontier & persistence — closest design to Foxhound's Run file):**
  - Seen set: dense columns indexed by sequential URL id, 16 B per entry (64-bit key hash, independent 32-bit FNV-1a fingerprint, 4 B depth+flags) plus a 4 B slot in an open-addressing table of ids (load ≤ 3/4, doubled when exceeded, old block returned to the OS); true collisions (same key, different fingerprint) go to a small overflow set holding the full address; SQLite `UNIQUE` remains the truth (`crates/os-engine/src/seen.rs:1-26`). ≈ 21–22 B/URL. The header records the measurements that motivated it: `HashMap<u64, Seen>` cost 24 B + control byte, sharded versions ~40 MB at 1 M entries, and 256 sub-tables fragmented the allocator so peak RSS rose 80–160 MB at 250 k–500 k pages (`seen.rs:7-12`).
  - Queue: ordered by (priority, depth, seq); in RAM while small; above 2 × `QUEUE_WINDOW` (10,000) only the W smallest keys stay in RAM and the rest live only in the `queue` table, refilled by a background job from a covering index after a writer flush, with a `floor`/watermark invariant so nothing leaves RAM before it is on disk (`queue.rs:1-40,55-58`; `queue_disk.rs` header).
  - Writer: one OS thread owns the SQLite sink, fed by a bounded channel; group commit of 200 pages or 100 ms normally, up to 4,000 pages / 8 MiB / 500 ms under backlog; an in-flight byte gate (`IN_FLIGHT_BYTES` = 3 × 8 MiB) throttles new fetches; channel capacity derived at compile time from batch bytes / reference page size (18 KiB) (`writer_thread.rs:1-30,47-114`). Measured: 1,000 instead of 200 pages per commit −42 % writer time, 5,000 −71 % (`writer_thread.rs:20-22`); a 256-message channel left the writer idle 235 s and workers blocked 41 % on 1 M URLs (`writer_thread.rs:87-90`). After batches ≥ 8 MiB it calls glibc `malloc_trim(0)` (`writer_thread.rs:676-690`; `crates/os-store/src/profile.rs:196-203`). Bodies above 1 MiB are parsed in `spawn_blocking` (`worker.rs:52`). Threads capped at 200 (`crawl.rs:49`).
  - Ordering trick: the "new URLs" message is sent before the page message under the same lock via a pre-reserved channel slot, so the store never sees a page whose discovered URL rows are missing (`worker.rs` header, steps 5-6).
- **Published benchmarks:** internal load tests referenced (`docs/plan/M1-acceptance/loadtest/...`), not read; no external comparison found.
- **Relevance:** high as a design reference for #2138/#2194/#2165 (measured numbers on the same SQLite-crawl-file architecture). As a compare candidate: possible later (CLI exists), but beta, macOS/Windows-first, agent-generated at extreme velocity.

#### millipede (satvik007) — and its third-party Spider benchmark
- **What:** Crawlee-inspired Rust library (request queue, autoscaled pool, session pool, fingerprints, chromiumoxide browser pool, fs/memory storage). 44,876 lines. Not yet on crates.io as `millipede` 0.1.0 (only sub-crates, `README.md:9-11`). Clone @ 6a47106a4a6a (2026-07-19).
- **Health:** 1 star, created 2026-05-15, 98 commits in 2026, last 2026-07-19 (quiet since). Not a product.
- **Why it matters here: the only third-party, method-documented benchmark of Spider found in this cluster.** Harness `benchmarks/spider-bench` (+ `gocolly-bench`, `crawlee-bench`): synthetic axum site on loopback, HTTP/1.1, C = 32, 4 tokio workers (GOMAXPROCS 4 for Colly; Crawlee single JS thread), no delays/retries/robots, exact count/byte/checksum validation per trial, median of 5 fresh-process trials after one warm-up, spider `=2.52.9` with only the `sync` feature (`benchmarks/PLAN.md` §1-5; `benchmarks/spider-bench/README.md`). Results (Apple M2 Pro, macOS, 2026-07-19, `docs/benchmarks-vs-crawlers.md`):

  | scenario | millipede | spider | Colly | Crawlee |
  |---|---:|---:|---:|---:|
  | books (DOM extraction, 5,100 p) | 32,511 p/s | 16,512 | 25,687 | 973 |
  | hn (link-dense, 1,040 p) | 7,242 | 2,394 | 7,091 | 341 |
  | tree (8,191 × 4 KiB) | 38,006 | **60,749** | 31,803 | 1,127 |
  | wide (5,001) | 47,229 | **60,707** | 38,240 | 537 |
  | mesh (dedup stress) | 33,764 | **56,327** | 23,651 | 1,050 |
  | payload (256 KiB pages) | 1,132 | **11,942** | 2,924 | 254 |
  | compressed (gzip) | 4,354 | **20,353** | 4,784 | 328 |
  | peak RSS tree | 21.7 MiB | 62.9 MiB | 61.1 MiB | 515.1 MiB |
  | peak RSS payload | 55.8 MiB | 291.4 MiB | 126.1 MiB | 569.3 MiB |
  | peak RSS compressed | 28.7 MiB | 151.4 MiB | 46.3 MiB | 496.1 MiB |

  Caveats stated by the author: loopback success paths only; spider's extraction rows re-parse with `scraper` inside a broadcast subscriber, which penalises it; spider's SSRF guard rejects loopback redirects; raw samples are not committed ("on the machine that ran the suite"). The author's reading: "Spider is the raw-fetch/link-discovery leader. Its streaming link parser is especially strong on large payloads and gzip, while Millipede's full DOM is costly there."
- **Relevance:** the harness design (validation gates, fresh process per trial, raw-client ceiling row, sensitivity rows for upstream feature defaults) is a good model for `just compare`; the RSS figures for spider (45–291 MiB) are a useful external data point against Foxhound's ~21 MiB target. Millipede itself: not a compare candidate (unpublished 0.1, 1 star).

#### spire (spire-rs)
- **What:** Crawler/scraper framework: frontier → crawler (per-host limits, robots RFC 9309) → router to typed handlers; backends reqwest or thirtyfour (WebDriver); frontier/dataset backends `spire-redb`, `spire-sqlx`. 35,113 lines. Clone @ ab9e08a50994 (2026-11-06).
- **Health:** 6 stars, created 2023-10-24, v0.11.0 (2026-11-06), 75 commits in 2026, one author; crates.io `spire` 1,645 downloads. API unstable by its own warning.
- **Fundamentals:** frontier built from facades (`Filter`, `Dedup`, `MaxDepth`, `Retry`, tag backoff) over `InMemFrontier` (`crates/spire-core/src/frontier/mod.rs` header); dedup by a stable, persistable u64 `Fingerprint` (`frontier/seed/fingerprint.rs:4-10`); `InMemSeen` ≈ 16 B/URL or `BloomSeen` ≈ 1.2 B/URL at 1 % FP, 1.8 B at 0.1 % ("Ten million URLs at a 1% false-positive rate: about 12 MB") (`frontier/seen/bloom.rs:11-29`). Adaptive per-host concurrency: start at 1, +1 per round of responses close to the host's best latency, halve on 429/503/timeout or responses > 2 × best, at most once per round; at 1 the gap widens instead (`crawler/concurrency.rs:42-50`).
- **Relevance:** adaptive per-host concurrency rule (latency-based additive increase, multiplicative decrease) for #2194; seen-set sizing numbers for #2138. Not a compare candidate (6 stars, unstable).

#### crawlie (spronta)
- **What:** Technical SEO + GEO auditor: CLI, MCP server, Tauri desktop app, closed "Crawlie Cloud" (`README.md:306-312`); vendor Spronta sells site search. 18,708 lines. Clone @ 9641d31f8ef1 (2026-10-26).
- **Health:** 117 stars, created 2026-06-18, v0.5.3 (2026-06-29), 169 commits in 2026, last 2026-10-26; npm-distributed.
- **Fundamentals:** one host per crawl; robots + sitemap discovery before the loop (`crates/crawlie-core/src/crawler.rs:399-570`); bounded concurrency with `FuturesUnordered` (`crawler.rs:15`); `HashSet<String>` visited (`crawler.rs:386,494`); `scraper` DOM; reqwest with rustls/gzip/brotli/http2, the workspace comment says HTTP/2 multiplexing "big-crawl throughput and far fewer sockets" (`Cargo.toml` `[workspace.dependencies]`); optional streaming mode writes pages into SQLite (WAL, synchronous NORMAL, explicit BEGIN/COMMIT batches) so peak memory is bounded by a url→id map and an integer edge graph (`pagestore.rs:1-11,23-25,673-684`). Release: `lto = "thin"`, `codegen-units = 1`, `strip`.
- **Relevance:** product-level competitor for SEO users, not an engine reference. Low compare priority.

#### Short reads (medium/triaged+)
- **scrapix-v2 (meilisearch):** Meilisearch's Rust rewrite of Scrapix: Kafka/Redpanda frontier service, RocksDB local state, DragonflyDB cache, bloom dedup (`crates/scrapix-frontier/src/dedup.rs:1-6`), per-domain `PolitenessScheduler` with `RwLock<HashMap<String, DomainState>>` (`politeness.rs:68-78`), caching DNS resolver 5 min / negative 1 min / 10 k entries (`crates/scrapix-crawler/src/dns.rs:22-38`), reqwest pool 200 idle per host (`fetcher.rs:227`), simhash near-dup, chromiumoxide + WebDriver renderers; release LTO fat, codegen-units 1. 66 k lines, 282 commits Feb–May 2026, 5 stars, no licence file declared via API. Clone @ 2210264baa4c. Notable as the only established company publishing a Rust crawler in 2026, but inactive since 2026-05-22 and built for distributed deployment.
- **spyglass:** SQLite `crawl_queue` (sea-orm) whose dequeue runs a CTE computing per-domain indexed and in-flight counts on every call (`crates/entities/src/models/crawl_queue.rs:1057`, ≤ 2 in flight per domain) — an example of what not to do on a hot path (O(queue) per dequeue). Archived. Clone @ 498fb089438e.
- **kalosm crawl:** per-origin `DomainQueue` running one sequential task on a `tokio_util` `LocalPoolHandle` (`interfaces/kalosm-language/src/context/page/crawl.rs:306,372-373`), `HashSet<Url>` visited, robots via texting_robots fetched with a fresh `reqwest::get` per origin (`crawl.rs:273-289`). Clone @ 7a9ead2493b3.
- **mycel (manual only, not source):** scheduler "refills the pool as fetches finish: it waits for free slots, never on a timer"; delay = max(default 1 s, robots crawl-delay ≤ 30 s, sticky per-host delay doubled on 429, 10 × last fetch duration) — explicitly "Mercator's adaptive rule"; robots TTL extended to 24 h when validators exist; per-URL adaptive recrawl ×2 per unchanged fetch up to ×16; host circuit breaker after 25 consecutive host-level failures (https://github.com/splch/mycel/blob/main/docs/MANUAL.md).
- **kumo (README only):** "Scrapy for Rust": JoinSet concurrency, governor token bucket, EWMA auto-throttle, bloom dedup (1 M default), file-backed persistent frontier, Redis frontier, chromiumoxide (https://github.com/wihlarkop/kumo).
- **a11ywatch/crawler:** a gRPC wrapper around `spider = "2"` with optional jemalloc; README claims "millions of pages per second" without evidence (`Cargo.toml:6,22,25`; `README.md:3`). Belongs to the Spider ecosystem; archived.
- **dyer:** crates.io shows 1,370,918 total downloads but 112 recent and 0 commits since 2025-08 — download count looks inflated (mirrors/bots), not adoption.

Triaged only (health + one line, no source reading beyond feature grep): jev-seo, crawlrs, crabler, suckit, crawly, coma, crawler-rs, ungoliant, cc-downloader, scout-lang, spyweb, NEXUS-ENGINE, docrawl, maman, urx, monolith, x227f, minoru-fediverse-crawler, dirble, lemmy-search, aperio, IndexFlow-core, enumrust, websurfx, and the 1–5-star names listed under the table.

---

## Part B: competitors to Collie, and Rust headless browser engines

Collected 2026-11-07. Health comes from `gh api` and the crates.io API. "c2026" counts commits since 2026-01-01 on the default branch. Clones are shallow, at the SHAs given in each section. Most columns are crawler features, so for browser libraries they read "n/a" or "no". "Browser" means the project drives or is a browser. "Sessions" means cookies, profiles or storage persistence.

| Project | Kind | URL | Licence | Stars | Last release (date) | 2026 commits / last commit | Maintained? | Robots | Politeness | Durable resume | Sitemaps | Browser | Extraction | Proxies | Sessions | Depth (deep/triaged) |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| mattsse/chromiumoxide | CDP client lib | https://github.com/mattsse/chromiumoxide | MIT/Apache-2.0 | 1,432 | crate 0.9.1 (2026-02-25) | 36 / 2026-04-03 | Slowing: no commit in 7 months | no | no | no | no | drives Chrome (CDP, WS) | selectors, JS eval | Chrome arg | cookies via CDP | deep |
| spider-rs/chromey | chromiumoxide fork | https://github.com/spider-rs/chromey | MIT/Apache-2.0 | 52 | crate 2.58.2 (2026-09-12) | 166 / 2026-10-18 | yes | no | no | no | no | drives Chrome | markdown, content stream | yes | yes | deep (fork delta only; the other reader covers its internals) |
| rust-headless-chrome | CDP client lib (sync) | https://github.com/rust-headless-chrome/rust-headless-chrome | MIT | 2,958 | 1.0.22 (2026-06-11) | 6 / 2026-06-11 | barely | no | no | no | no | drives Chrome | JS eval | `--proxy-server` | cookies |  deep (transport only) |
| jonhoo/fantoccini | WebDriver client | https://github.com/jonhoo/fantoccini | MIT/Apache-2.0 | 2,033 | crate 0.22.1 (2026-02-28) | 32 / 2026-08-08 | yes, slow | no | no | no | no | any WebDriver browser | selectors | via caps | cookies | deep (transport only) |
| stevepryde/thirtyfour | WebDriver + CDP + BiDi client | https://github.com/stevepryde/thirtyfour | MIT/Apache-2.0 | 1,443 | v0.38.1 (2026-10-21) | 183 / 2026-10-21 | yes | no | no | no | no | any WebDriver browser; CDP and BiDi over WS | selectors, queries | via caps | cookies | triaged + transport |
| padamson/playwright-rust (`playwright-rs`) | Rust bindings to the Node Playwright server | https://github.com/padamson/playwright-rust | Apache-2.0 | 167 | crate 0.20.1 (2026-10-17) | 563 / 2026-11-07 | yes | no | no | no | no | Chromium/Firefox/WebKit through the Node driver | Playwright locators | yes | contexts | triaged |
| octaltree/playwright-rust | old Playwright port | https://github.com/octaltree/playwright-rust | none | 458 | crate 0.0.20 (2022-08-20) | 0 / 2022-08-20 | no (dead) | – | – | – | – | – | – | – | – | triaged |
| dashn9/rustenium | WebDriver BiDi + CDP client | https://github.com/dashn9/rustenium | MIT | 27 | crate 1.1.15 (2026-10-25) | 104 / 2026-10-25 | yes (small) | no | no | no | no | Chrome/Firefox | selectors | ? | ? | triaged + transport |
| chaser-oxide (ccheshirecat / 0xchasercat) | stealth chromiumoxide fork | https://github.com/ccheshirecat/chaser-oxide (API 404 now) | MIT/Apache-2.0 | ~247 (Exa snapshot) | crate 0.2.4 (2026-04-20) | repo gone / last push 2026-04-28 (Exa) | unknown, repo unreachable | no | no | no | no | Chrome | JS eval in isolated world | yes | yes | triaged (web only) |
| TurtIeSocks/zendriver-rs | stealth CDP lib (zendriver port) | https://github.com/TurtIeSocks/zendriver-rs | Apache-2.0 | 25 | zendriver 0.5.28 (2026-09-17) | 1,129 / 2026-09-18 | yes (bursty) | no | no | no | no | Chrome | – | ? | ? | triaged |
| shrimp-software/eoka | stealth automation | https://github.com/shrimp-software/eoka | MIT | 18 | crate 0.9.1 (2026-11-05) | 120 / 2026-11-07 | yes | no | no | no | no | Chrome | – | ? | ? | triaged |
| ai-ecoverse/slicc-cdp | CDP layer + playwright-cli clone (native/WASI) | https://github.com/ai-ecoverse/slicc-cdp | Apache-2.0 | 0 | 1.47.0 (2026-11-07) | 1,888 / 2026-11-07 | yes (very bursty, 0 stars) | no | no | no | no | Chrome | – | – | – | triaged |
| raultov/cdp-lite | minimal CDP client | https://github.com/raultov/cdp-lite | MIT | 2 | 0.3.1 (2026-10-25) | 28 / 2026-10-25 | yes (tiny) | no | no | no | no | Chrome, flat sessions | raw JSON | – | – | triaged + transport |
| cloudflare/chrome-devtools-rs | CDP types | https://github.com/cloudflare/chrome-devtools-rs | MIT | 66 | none | 0 / 2024-09-20 | no | – | – | – | – | – | – | – | – | triaged |
| mdrokz/auto_generate_cdp | CDP codegen | https://github.com/mdrokz/auto_generate_cdp | GPL-3.0 | 22 | crate 0.4.6 (2025-12-13) | 0 / 2025-12-13 | no | – | – | – | – | types only | – | – | – | triaged (3.7 M downloads, pulled in through headless_chrome) |
| keon/browser-control | agent CLI over CDP | https://github.com/keon/browser-control | MIT | 3,157 (anomaly) | v0.1.1 (2026-06-21) | 3 / 2026-08-27 | low | no | no | no | no | Chrome | snapshot | ? | ? | triaged |
| vercel-labs/agent-browser | agent CLI + per-session daemon | https://github.com/vercel-labs/agent-browser | Apache-2.0 | 46,638 | v0.41.2 (2026-11-03) | 816 / 2026-11-06 | yes | no | no | no | no | Chrome (own CDP client, WS) | a11y snapshot, read | `--proxy-server` | profiles, state save | deep |
| gsd-build/gsd-browser | agent CLI over CDP | https://github.com/gsd-build/gsd-browser | Apache-2.0 | 269 | v0.1.25 (2026-05-03) | 81 / 2026-05-03 | stalled (moved to open-gsd/gsd-browser, 58★) | no | no | no | no | Chrome | snapshot | ? | ? | triaged |
| aeroxy/chrome-devtools-cli | CDP CLI | https://github.com/aeroxy/chrome-devtools-cli | none | 255 | 2.2.1 (2026-11-06) | 252 / 2026-11-06 | yes | no | no | no | no | Chrome | – | – | – | triaged |
| theoxfaber/ferrous-browser | CDP lib | https://github.com/theoxfaber/ferrous-browser | MIT | 31 | crate 0.1.2 (2026-05-26) | 41 / 2026-08-24 | low | no | no | no | no | Chrome | – | – | – | triaged |
| BB-fat/browser-use-rs | CDP lib + MCP | https://github.com/BB-fat/browser-use-rs | MIT | 41 | v0.2.3 (2025-11-20) | 0 / 2025-11-20 | no | no | no | no | no | Chrome | – | – | – | triaged |
| maestrojeong/browser-rs-mcp | MCP server, one shared Chrome | https://github.com/maestrojeong/browser-rs-mcp | Apache-2.0 | 32 | v0.7.7 (2026-10-20) | 154 / 2026-10-20 | yes | no | no | no | no | Chrome | – | ? | ? | triaged |
| sderosiaux/chrome-agent | agent CLI | https://github.com/sderosiaux/chrome-agent | MIT | 96 | v0.16.0 (2026-08-30) | 286 / 2026-10-12 | yes | no | no | no | no | Chrome | JSON read-back | – | – | triaged |
| dravr-ai/dravr-browser | CDP lib (headless-chrome keyword) | https://github.com/dravr-ai/dravr-browser | none | 0 | crate 1.0.0 (2026-10-29) | 51 / 2026-10-29 | yes; 23 k recent downloads with 0 stars (CI or own use) | no | no | no | no | Chrome | – | – | – | triaged |
| greysquirr3l/stygian (`stygian-browser`) | scraping engine + browser pool (on chromiumoxide) | https://github.com/greysquirr3l/stygian | AGPL-3.0 + commercial | 15 | v0.17.0 (2026-08-24) | 474 / 2026-08-25 | slowing | ? | ? | ? | ? | Chrome pool | yes | stygian-proxy | yes | triaged + pool read |
| h4ckf0r0day/obscura | Rust headless engine (V8 via deno_core, own DOM, CDP server) | https://github.com/h4ckf0r0day/obscura | Apache-2.0 | 30,511 (see anomaly check) | v0.2.6 (2026-10-31) | 1,668 / 2026-11-07 | yes | no | no | no | no | is a browser engine (CDP, Puppeteer/Playwright connect) | text/html/markdown dump | HTTP/SOCKS | cookie jar, storage-dir | deep |
| lexmount/moli | Rust headless engine (rusty_v8, Stylo, Taffy, libcurl; CDP + WebDriver Classic + BiDi) | https://github.com/lexmount/moli | Apache-2.0 / MIT | 22,923 (see anomaly check) | v1.1.22 (2026-11-06) | 3,010 / 2026-11-07 | yes (one main author) | yes (`moli-robots`, RFC 9309) | ? | no | no | is a browser engine | dump, html2md | yes | cookie DB, profiles | deep |
| project-oxi/oxibrowser | engine: Blitz (Servo parts) + Boa JS, CDP | https://github.com/project-oxi/oxibrowser | MIT | 47 | v0.28.0 (2026-10-25) | 361 / 2026-10-25 | yes | ? | ? | no | no | engine | – | wreq | cookies | triaged |
| yfedoseev/browser_oxide | stealth engine (deno_core V8, html5ever, Taffy, BoringSSL), no CDP | https://github.com/yfedoseev/browser_oxide | non-standard | 125 | v0.1.3 (2026-07-28) | 4 / 2026-07-28 | barely | no | no | no | no | engine | – | ? | ? | triaged |
| koloss777/nokk | stealth engine (rusty_v8 152, html5ever, wreq) | https://github.com/koloss777/nokk | Apache-2.0 | 25 | v0.1.49-alpha (2026-11-06) | 530 / 2026-11-07 | yes (alpha) | no | no | no | no | engine | – | ? | ? | triaged |
| alejandroqh/browser39 | agent fetcher (deno_core 0.340, html5ever, scraper) | https://github.com/alejandroqh/browser39 | Apache-2.0 | 95 | v2.0.0 (2026-10-12) | 53 / 2026-10-12 | yes | ? | no | no | no | JS-capable fetcher | Markdown | ? | sessions | triaged |
| hongnoul/hwatu | warm WebKit daemon for UI checks | https://github.com/hongnoul/hwatu | MIT | 118 | v0.7.1 (2026-08-24) | 496 / 2026-10-17 | yes | no | no | no | no | WebKit | pixel diff | – | – | triaged |
| plasmate-labs/plasmate | engine: HTML to "Semantic Object Model", v8 139, CDP subset | https://github.com/plasmate-labs/plasmate | Apache-2.0 | 39 | v0.5.1 (2026-04-05) | 2,517 / 2026-11-07 | yes | ? | ? | ? | ? | engine | "Semantic Object Model" JSON | ? | ? | triaged (engine only) |
| DioxusLabs/blitz | HTML/CSS renderer (Stylo + Taffy + Parley), no JS | https://github.com/DioxusLabs/blitz | Apache-2.0/MIT | 4,421 | blitz-dom 0.3.0-beta.4 (2026-10-16) | 931 / 2026-11-06 | yes | – | – | – | – | layout/paint only | – | – | – | triaged |
| servo/servo | full engine, embeddable (`servo` crate 0.8.0) | https://github.com/servo/servo | MPL-2.0 | 38,419 | v0.7.0 (2026-10-05); crate 0.8.0 (2026-11-07) | 5,006 / 2026-11-07 | yes | – | – | – | – | engine (SpiderMonkey) | – | – | – | triaged |
| konippi/servo-fetch | Servo-embedded fetch → Markdown/JSON/screenshot | https://github.com/konippi/servo-fetch | Apache-2.0 | 163 | v0.15.2 (2026-11-01) | 505 / 2026-11-07 | yes | ? | ? | no | no | Servo 0.7 | Markdown, JSON | ? | ? | triaged (found in sweep) |
| putao520/bao | Servo + SpiderMonkey runtime with CDP | https://github.com/putao520/bao | ? | 14 | bao_cdp_client 0.6.0 (2026-10-27) | ? | ? | – | – | – | – | engine | – | – | – | triaged (found in sweep) |
| lightpanda-io/browser | **Zig** headless engine (out of scope) | https://github.com/lightpanda-io/browser | AGPL-3.0 | 41,556 | 1.0.3 (2026-11-05) | 8,457 / 2026-11-07 | yes | – | – | – | – | engine, CDP | – | – | – | reference only |

Also found in the sweep, triaged only: `raultov/bidi-lite` (WebDriver BiDi client for Firefox, 0★), `allwright-dev/allwright` (crates `allwright-core`/`-surface-web`, 10★), `kitewright/kitewright` (7 MB MCP server, 24★), `dondai44423/bladebro` (stealth agent driver, 343★, 574 commits in 2026), `MageGojo/drission-rs` (anti-detect, captcha OCR, 33★), `spider-rs/headless-browser` (Chrome launcher/proxy for cloud, 88★, 5 commits in 2026), `chrome-for-testing-manager` (crate that downloads Chrome for Testing), `cdpkit`, `cdp-protocol`, `cdp-html-shot`. No other Rust CDP client has more than about 50 k recent downloads besides chromiumoxide (2.3 M recent), headless_chrome (721 k), thirtyfour (700 k), fantoccini (571 k), playwright-rs (172 k) and spider_chrome/chromey (115 k + 46 k) (crates.io API, `sort=recent-downloads` for the keywords cdp, chrome-devtools, headless-chrome, webdriver, puppeteer, playwright, bidi and headless-browser).

---

### Star-anomaly check: obscura and moli

**Method limit:** this session's token cannot read stargazers. `GET /repos/{o}/{r}/stargazers` with the `star+json` media type and the GraphQL `stargazers{starredAt}` both return 403 "Resource not accessible by personal access token", and the unauthenticated REST call returns 401 "Requires authentication". **Star timestamps were not sampled.** The verdict below rests on proxy signals from `gh api` (repo, contributors, search/issues, releases), compared with two peers that grew organically.

| Repo | Stars | Created | Forks (fork/star) | Watchers | Issues (issue/star) | PRs | Contributors | Release-asset downloads |
|---|---|---|---|---|---|---|---|---|
| h4ckf0r0day/obscura | 30,511 | 2026-04-13 | 2,317 (7.6 %) | 80 | 623 (2.0 %) | 826 | 102 | 360,617 |
| lexmount/moli | 22,923 | 2026-08-10 | 639 (2.8 %) | 49 | 76 (0.33 %) | 2,313 | 33 | 41,633 |
| lightpanda-io/browser (peer) | 41,556 | 2023-02-07 | 2,061 (5.0 %) | 99 | 636 (1.5 %) | 3,856 | 91 | 177,769 |
| vercel-labs/agent-browser (peer) | 46,638 | 2026-01-11 | 3,197 (6.9 %) | 104 | 782 (1.7 %) | 1,579 | 119 | 634,696 |

- **obscura: engagement looks organic in proportion.** Its fork, issue and contributor ratios match the peers. The 300 most recent PRs come from many authors (top: SGavrl 170, mnaza 92, xrip 29, flolep2607 24 …), and so do recent issues. Release downloads (360 k) are twice Lightpanda's. Caveats: the owner account was created 2026-03-29, two weeks before the repo. The README carries paid proxy-sponsor blocks (NodeMaven and others, README:94-101), so the project has a commercial reason to push stars, and timing could not be checked.
- **moli: suspicious.** 22.9 k stars in under 3 months is about 250 a day. Yet it has 0.33 % issues per star (5–6× below the peers), 2.8 % forks per star and 41 k downloads. One author, `ldm0`, opened 573 of the last 600 PRs and wrote 97 of the last 100 commits. The README shows a "#1 Product of the Day" Product Hunt badge (README:43-44), so a launch spike explains part of it. The engagement profile does not fit 23 k organic users, so treat its stars as inflated or campaign-driven. **Unproven without timestamps.**
- **keon/browser-control (3,157★):** the repository dates from 2016, but its history holds only 3 commits, the first being "Prepare public release" on 2026-06-21. Its stars come from an earlier project and say nothing about the current CLI.

#### Do they work? (from source)
- **obscura @ d21de16d4c0f10548a087e00efbc0b8f81b23d8c**
  - **Engine:** V8 through `deno_core = "0.412"` (`crates/obscura-js/Cargo.toml`). Its own DOM sits on `html5ever 0.39` and `selectors 0.26` (`Cargo.toml` workspace deps). A layout and paint layer is optional: vendored Taffy, tiny-skia and ab_glyph, about 78 k lines in `crates/obscura-render`.
  - **CDP server:** `crates/obscura-cdp`, about 25.6 k lines, implements Page, Runtime, DOM, DOMSnapshot, Network, Fetch, Input, Emulation, Target, Storage, Accessibility, IO and PDF (`crates/obscura-cdp/src/domains/`). The README shows Puppeteer `connect` and Playwright `connectOverCDP`.
  - **Code size:** about 229 k lines of Rust in total.
  - **Benchmarks:** its own WPT run passes 83.3 % of "Core" subtests (obscura-benchmark README). An independent benchmark, moli's README, has obscura last among local engines at 29.7–44.9 % task success.
  - **Verdict:** a real JS engine and a real CDP surface; it works on static and moderate SPA pages, with clearly lower compatibility than Chrome.
- **moli @ c9140e76ffb87979acc37041734726e20eef895d**
  - **Engine:** `rusty_v8` 152 is vendored (`vendor/v8-152.2.0`; `Cargo.toml:119-120`). Style comes from a Stylo fork (`lexmount/stylo`, `Cargo.toml:122-127`) and layout from a Taffy fork. The network transport is libcurl; README §Architecture lists html5ever, Parley and Vello CPU.
  - **Protocols:** CDP, WebDriver Classic and BiDi share one endpoint (`moli-protocol-*`).
  - **Code size:** about 2.47 M lines of Rust outside `vendor/`, of which `moli-renderer-v8` alone is 1.37 M. That volume from essentially one author in 3 months points to heavy generation (code generated or AI-written).
  - **Verdict:** a real engine. Its own benchmarks claim 81.9 % task success against Chrome's 99.85 %.

---

### Deep reads

#### chromiumoxide (mattsse) @ afcc3a4313f2087249b4490d94e54bf8e3bfaccf
- **What:** The reference async CDP client in Rust. The protocol types are generated from PDL into one 5 MB `chromiumoxide_cdp/src/cdp.rs`. The crate is pure Rust, has no bindings, and no commercial offering. chromey (spider), chaser-oxide, stygian, crawlberg-chromiumoxide and yosoi-chromiumoxide all fork or wrap it.
- **Health:**
  - Created 2020-11-28, 1,432★, 204 forks.
  - Releases: v0.9.1 on GitHub and crates.io, 2026-02-25.
  - Activity: 36 commits in 2026; the last on 2026-04-03, with nothing in the 7 months since.
  - Downloads: 5.57 M total, 2.30 M recent, the most used Rust CDP crate.
  - Verdict: maintained but slowing.
- **Features vs Foxhound:**
  - Browser rendering: yes; it is the driver.
  - Request interception: `HandlerConfig.request_intercept` and `cache_enabled` (`src/handler/mod.rs:657-683`).
  - Proxies: only through raw Chrome args.
  - Cookies: through CDP.
  - Robots, politeness, sitemaps, resume, extraction pipeline: none.
- **Fundamentals:**
  - **Transport:**
    - Library and framing: `async-tungstenite 0.32` over WebSocket only (`Cargo.toml`). Message and frame size limits are removed (`src/conn.rs:37-39`).
    - Sending: one command is serialized with `serde_json::to_string` and then flushed before the next is sent (`src/conn.rs:86-101`), so every command costs one write syscall and nothing is batched.
    - Parameters: they travel as `serde_json::Value` (`chromiumoxide_types/src/lib.rs:24`). A typed command is converted to a `Value`, then to a string: a double serialization.
    - Receiving: incoming text is decoded with `serde_json::from_str::<Message<T>>` (`src/conn.rs:133`). `Message` is `#[serde(untagged)]` over Response and Event (`chromiumoxide_types/src/lib.rs:203`), so serde buffers each frame into its internal `Content` tree, tries Response first, then Event. `Response.result` stays a `Value` and is converted again into the typed result. No zero-copy anywhere.
  - **Dispatch:**
    - One central `Handler` stream that the user must drive in a spawned task.
    - On every wake-up it drains the browser channel, then loops over **all** targets, `swap_remove`/re-`insert` from a `HashMap`, and polls each target and its listeners (`src/handler/mod.rs:527-650`). The work per wake is O(open targets).
    - The Browser→Handler channel has capacity 1 (`src/browser/mod.rs:131`).
    - Pending commands live in an `FnvHashMap` evicted by a periodic job (`src/handler/mod.rs:59,119`).
  - **Sessions:** flatten mode with `Target.setAutoAttach{flatten:true, autoAttach:true}` (`src/handler/target.rs:572-574`). All sessions are multiplexed on one socket by `sessionId`.
  - **Launch:**
    - 24 Puppeteer-derived default flags (`src/browser/config.rs:460-485`); `--remote-debugging-port`; and a fixed temp `user-data-dir` (`config.rs:393-402`).
    - The WS URL is found by scraping the child's stderr (`src/browser/mod.rs:534`) with a 20 s timeout.
    - No `--remote-debugging-pipe`.
  - **Pooling:** none. One `Browser` per launch, `new_page` creates a target, and browser contexts are tracked but not pooled.
- **Published benchmarks:** none found.
- **Relevance:** Collie's baseline competitor at the client layer. Its central-loop, Value-based design is what chromey reworked.

#### chromey (spider-rs) — fork delta vs chromiumoxide only @ 0312d624ea2feb4dfa77699213f1d7ca388358ac
(The other reader covers its internals; this section lists only what differs as a CDP client.)
- **Health:** 52★, 166 commits in 2026, last 2026-10-18; crate `chromey` 2.58.2 (2026-09-12), plus the older name `spider_chrome` with 1.17 M downloads in total. The `[lib] name = "chromiumoxide"` line keeps it a drop-in replacement (`Cargo.toml`).
- **Transport delta:**
  - **Libraries:**
    - `tokio-tungstenite 0.29` replaces async-tungstenite.
    - With the default feature `simd`, `sonic-rs` stands in for `serde_json` (`src/lib.rs:130-133`).
    - An optional io_uring connect path (`src/conn.rs:274-280`, feature `io_uring`).
  - **Write buffer:** capped at 4 MiB (`src/conn.rs:262-271`).
  - **Batching:**
    - The send path writes every queued command, then flushes once (`src/conn.rs:490-532`).
    - In split mode, a background `ws_write_loop` `feed`s up to 128 commands per flush under one deadline (`src/conn.rs:873-912`).
    - Bounded channels: 2,048 commands to the writer and 1,024 reads (`src/conn.rs:539-546`).
  - **Decoding:**
    - Frames up to 256 KiB are decoded inline with `from_slice` straight from the frame bytes.
    - Larger frames go to `spawn_blocking`, at most 32 in flight (`src/conn.rs:555-572, 746-830`).
    - Decoding boxes `Message<T>` (`src/conn.rs:1178-1182`).
- **Dispatch delta:** the opt-in `parallel-handler` feature replaces the single loop with a Router task and one `SessionTask` per attached page (`src/handler/parallel/mod.rs:1-25`, `src/handler/mod.rs:1193-1196`). Its inboxes are bounded, an overflowing inbox spills to a bounded FIFO, and one page's panic or stall does not stop the router. The docs say multi-context support is still incomplete (`parallel/mod.rs:25`).
- **Request blocking:** in the network manager: `block_all`, `ignore_visuals`, `block_stylesheets`, `block_javascript`, plus the optional `adblock 0.12` engine and `spider_network_blocker` lists (`src/handler/network.rs:1-37, 248-371`). Blocking runs at Fetch-interception time inside the client.
- **Drop-time cleanup:** one background worker with a lock-free mpsc queue (`src/bg_cleanup.rs:1-30`), so `Drop` never calls `tokio::spawn`.
- **Benchmarks:** Criterion benches exist (`benches/handler_bench.rs`, `parallel_pages_bench.rs`, `alloc_audit_bench.rs`), but the README publishes no figures.

#### rust-headless-chrome (`headless_chrome`) — transport only @ 0a5c307a85debc450378a1f19e4dac1838d7b22d
- **Health:** 2,958★; 6 commits in 2026, last 2026-06-11 (release 1.0.22); 3.58 M downloads in total, 721 k recent.
- **Fundamentals:**
  - **Threads:** synchronous `tungstenite 0.29`, with OS threads per browser: a WS reader thread (`src/browser/transport/web_socket_connection.rs:103`) and a dispatch thread (`src/browser/transport/mod.rs:240`). The connection sits behind `Arc<Mutex<…>>`.
  - **Session routing:** commands to a tab are wrapped in **`Target.sendMessageToTarget`**, a JSON string inside JSON (`src/browser/transport/mod.rs:140-151`), and tabs attach with `flatten: None` (`src/browser/tab/mod.rs:219-221`). This is the deprecated non-flat mode: every tab message is serialized twice and escaped.
  - **Launch:** 23 default flags (`src/browser/process.rs:251-275`).
- **Relevance:** a counter-example for Collie. Do not route sessions this way.

#### fantoccini / thirtyfour (WebDriver) @ 911871416ebc35818e62910cbd9f02e20a05f229 / 24f9114266465fa9f9cba8a2b47649d6b8dff446
- **fantoccini:**
  - WebDriver classic over `hyper 1` HTTP/1 (`Cargo.toml`): each session is one `tokio::spawn`ed `Session` task fed by an unbounded mpsc (`src/session.rs:644-646`).
  - Every action is an HTTP round trip to chromedriver or geckodriver, so its latency floor is above CDP's.
  - 32 commits in 2026, last 2026-08-08.
- **thirtyfour:**
  - Uses reqwest for WebDriver.
  - Since 2026 it also offers typed CDP (feature `cdp`, default), CDP events over WS (`cdp-events`), and **WebDriver BiDi** (`bidi`), all over `tokio-tungstenite 0.30` (README:30-46; `thirtyfour/src/bidi/transport/ws.rs`).
  - 183 commits in 2026, release v0.38.1 on 2026-10-21. It is the most active WebDriver crate.
- **Relevance:** low for Foxhound rendering (CDP is faster, and spec #2139 needs no WebDriver). BiDi support in thirtyfour and rustenium (fastwebsockets + `serde_json` RawValue, `rustenium-core/src/transport.rs:1`, `listeners.rs:146`) is the main Rust path to Firefox.

#### vercel-labs/agent-browser @ d44f6989bf5e9a9a1c6dd6d97175f8c33b138eca
- **What:** A native Rust CLI for AI agents. Since 0.23.0 only the Rust daemon ships (benchmarks/README.md). The npm package only delivers the binary. Apache-2.0. Vercel ties it to Vercel Sandbox (cloud microVM browsers, paid service).
- **Health:**
  - Stars and age: created 2026-01-11, 46.6 k★, 3,197 forks.
  - Activity: 816 commits in 2026; last on 2026-11-06.
  - Releases: v0.41.2 (2026-11-03); 635 k release-asset downloads.
  - Engagement ratios match the peers, so the stars look organic.
- **Daemon model:**
  - The first command starts a background daemon per `--session`, and each daemon owns its own browser. The CLI talks to it over a Unix socket (named pipe on Windows) (`cli/src/connection.rs:15-98`).
  - A headless daemon closes after 1 h idle (README:52).
  - Latency comes from keeping the browser and the CDP connection warm between one-shot CLI calls. No browser launch per command.
- **CDP client:**
  - **Transport:** its own client on `tokio-tungstenite 0.24`, WS only, via `--remote-debugging-port=0`. It reads `DevToolsActivePort` from the user-data dir instead of scraping stderr, and deletes a stale file first (`cli/src/native/cdp/chrome.rs:467, 810-812, 888-975`).
  - **Concurrency primitives:**
    - The sink is split behind `Arc<tokio::Mutex>`.
    - Pending calls sit in a `tokio::Mutex<HashMap<u64, oneshot>>` (`client.rs:22-31`).
    - Events go out on one broadcast channel of 4,096 (`client.rs:261-262`). Sessions that need every event (screencast) get a private mpsc that bypasses the broadcast. "Method taps" are lossless unbounded channels for events that need a reply: `Fetch.requestPaused`, auth challenges (`client.rs:33-46, 571-580`).
  - **Parsing:**
    - A full `serde_json::from_str` into `CdpMessage` per frame (`client.rs:322`).
    - The inspect proxy additionally parses to a `Value` only when it has subscribers (`client.rs:312-320`).
    - A cheap envelope with `IgnoredAny` extracts the id when the full parse fails (`client.rs:76-90, 153`).
    - Lone surrogates are repaired before giving up (`client.rs:92`).
  - Flags: the usual 14-flag set plus `--headless=new` (`chrome.rs:467-510`).
- **Benchmarks:** `benchmarks/bench.ts` measures command latency (navigate, snapshot, click …), cold start, daemon RSS and process counts inside a Vercel Sandbox (benchmarks/README.md). It needs paid Sandbox credentials, and **no figures are published**. The README says latency is "dominated by CDP round trips".
- **Relevance:** for Collie's attached-engine use (an SDK drives it), this is the leading competitor for agents. Its warm-daemon design matches Collie's. It has no crawl features. Not a `just compare` candidate (not a crawler).

#### obscura @ d21de16d4c0f10548a087e00efbc0b8f81b23d8c (headless engine)
- **What:** A Rust browser engine that serves CDP, so Puppeteer and Playwright connect to it instead of Chrome. Covered under the anomaly check above.
- **Health:** 30.5 k★ (organic-looking proportions; timestamps unchecked); 1,668 commits in 2026; release v0.2.6 (2026-10-31).
- **Fundamentals:**
  - **Threading:**
    - Each CDP connection owns an OS thread with a `current_thread` Tokio runtime and `LocalSet`, which pins its pages' V8 isolates to that thread.
    - A second thread handles WS I/O, so JS evaluation never blocks the socket (`crates/obscura-cdp/src/server.rs:296-298, 800-806, 832-860`).
    - The number of live connections is capped (`DEFAULT_MAX_CONNECTIONS`).
  - **glibc allocator tuning:** worth copying. `mallopt(M_ARENA_MAX, 2)` and `M_MMAP_THRESHOLD = 1 MiB` (`server.rs:703-750`).
    - Measured cost without the cap: glibc gives each new thread a 64 MiB arena, and RSS tracks the *peak* number of connections and never shrinks (the comment at `server.rs:703-725`).
    - Google Maps, 4 concurrent connections per round: 350/619/826 MiB uncapped vs 166/235/269 MiB capped.
    - Throughput effect: none (1.53 s vs 1.50 s).
  - Release profile: `panic = "unwind"`, which is required to catch panics at the V8 FFI boundary. `release-dist` uses `opt-level="s"`, fat LTO and `codegen-units=1` (`Cargo.toml` profiles).
- **Published benchmarks** (obscura-benchmark README, run 2026-09-11 at commit 01e1caa3):
  - Speed vs Chrome: React page 81 ms / 30 MB vs Chrome 815 ms / 190 MB. **The method is one cold process per page, Chrome's worst case**, which they admit.
  - Throughput: 24 React loads reach 40 pages/s at 4 workers (112 MB) vs Chrome at 3 pages/s (4.2 GB).
  - Real-world corpus of 98 pages: obscura renders 94, Chrome 85 (Chrome misses mostly anti-bot walls). **Median latency is 5.2 s for obscura vs 2.1 s for Chrome**, at a median peak RSS of 64 MB vs 201 MB.
- **Relevance:** a possible lightweight rendering back end for Foxhound's browser scenario (an alternative engine behind Collie's CDP). Compatibility is clearly below Chrome. A `just compare` browser-scenario candidate only as "engine", and only if the scenario allows a non-Chrome engine.

#### moli @ c9140e76ffb87979acc37041734726e20eef895d (headless engine)
- **What:** A Rust "browser kernel": rusty_v8, Stylo, Taffy and Parley, libcurl, CDP + WebDriver Classic + BiDi, robots.txt support (`moli-robots`, RFC 9309; `moli/src/app.rs:75`). Lexmount sells a cloud browser (browser.lexmount.com).
- **Health:** 22.9 k★ (suspicious, see above); 3,010 commits in 2026; release v1.1.22 (2026-11-06).
- **Fundamentals:**
  - **Allocator:** jemalloc process-wide with the C symbols overridden, so V8 and libcurl use it too, configured `narenas:1, tcache_max:1024, dirty_decay_ms:250, muzzy_decay_ms:0` (`moli/src/allocator.rs:1-27`). Where an official pointer-compressed rusty_v8 archive exists, its PartitionAlloc shim is used instead.
  - Release profile: `panic="abort"`, `codegen-units=1` (`Cargo.toml:147-152`).
- **Published benchmarks** (README §Benchmark):
  - Mixed crawl of 192 public URLs: moli 53.6 % useful pages at 1.43 s median / 73 MiB median RSS; Chrome 52.6 % at 1.43 s / 773 MiB; Lightpanda 44.3 % at 0.97 s / 40 MiB; obscura 29.7 % at 1.30 s / 39 MiB.
  - "CDP ready": 34.85 ms vs Chromium 169 ms.
  - Lexbench, 1,308 comparable tasks: moli 81.9 % vs Chrome 99.85 %.
  - Method: Lexbench-Headless-Browser repo. **It is written by the same vendor, so it is not independent.**
- **Relevance:** the strongest non-Chrome engine on paper. Its allocator configuration is directly relevant to #2165.

#### stygian-browser — pool only @ b94ae0e4dbd5d0f3b8845ed3effeba078e5abd43
- **Pool design:** a pool of whole **browsers**, not tabs, on chromiumoxide.
  - A semaphore of `max_size` slots provides global backpressure.
  - Unscoped browsers wait in a shared idle `VecDeque`; per-context queues are keyed by `context_id` (`crates/stygian-browser/src/pool.rs:1-30`).
  - `min_size` browsers are pre-warmed (`pool.rs:198-238`).
  - A background loop evicts browsers idle longer than `idle_timeout` down to `min_size` (`pool.rs:252-334`).
  - Each browser gets a health check when it is returned (`browser.rs:175-215`).
- **What it lacks:** no recycling after N pages and no per-browser page cap. AGPL-3.0 plus a commercial licence.

---

### What no Rust client does (useful negatives for Collie and #2139)
- **No pipe transport:** none of the deep-read Rust CDP clients (chromiumoxide, chromey, headless_chrome, agent-browser, rustenium, cdp-lite, stygian) launches Chrome with `--remote-debugging-pipe`. The flag appears only inside generated protocol doc strings (grep over the clones). All use a WebSocket on a TCP port.
- **No recycling:** no Rust client implements "recycle a browser after N pages", "max pages per browser" or "grow the pool with concurrency". stygian has min/max/idle eviction, and agent-browser has one browser per session plus an idle timeout. The other reader should confirm whether spider (over chromey) does.
- **Memory per tab:** no published figure from any Rust client. The only measured memory curves are obscura's malloc-arena data (above) and the vendors' engine RSS tables.

## Part C: building blocks

Scope: the libraries a Rust crawler rests on, per role: HTTP client, HTML parse/rewrite, main-content extraction, HTML→Markdown, robots.txt/sitemaps, ad/tracker blocking, URL handling and dedup, allocators, async runtimes. For each role: the state-of-the-art crate, its published figures with method, and what a crawler that wants to beat Spider on instructions/page and memory should use.

Health data: `gh api` (stars, 2026 commit count via paginated `commits?since=2026-01-01`, last commit, latest release or tag) and crates.io API (max version, updated, downloads, recent downloads), collected 2026-11-07. Source read from shallow clones (SHA given per cite). Spider references are to `spider-rs/spider` @ `e4b53c5b5823` (already analysed in `rnd/research/2026-11-06-spider-gap.md`; only block choices are repeated here). Foxhound references are to goodfetch @ `2138a00d0`.

Column note: the crawler feature columns of the common brief do not apply to libraries; per the cluster brief they are replaced by `Role | Approach | Published benchmark?`.

### Table

| Project | Kind | URL | Licence | Stars | Last release (date) | 2026 commits / last commit | Maintained? | Role | Approach | Published benchmark? | Depth |
|---|---|---|---|---|---|---|---|---|---|---|---|
| hyper 1.x | lib | https://github.com/hyperium/hyper | MIT | 16399 | v1.12.0 (2026-10-06) | 124 / 2026-11-02 | yes | HTTP/1+2 protocol | low-level async, no pool | none published | deep (defaults) |
| hyper-util (legacy client + pool) | lib | https://github.com/hyperium/hyper-util | MIT | 181 | v0.1.21 (2026-09-24) | 52 / 2026-11-05 | yes | client, pool, GaiResolver | `Arc<Mutex<PoolInner>>`, `HashMap<Key, Vec<Idle>>` | none | deep |
| reqwest | lib | https://github.com/seanmonstar/reqwest | MIT/Apache-2.0 | 11905 | v0.13.5 (2026-09-08) | 58 / 2026-10-19 | yes | high-level client (Foxhound, Spider) | hyper-util + url::Url | none | deep (defaults) |
| hickory-dns (resolver) | lib | https://github.com/hickory-dns/hickory-dns | MIT/Apache-2.0 | 5486 | v0.26.3 (2026-09-10) | 872 / 2026-11-06 | yes | async DNS + cache | async, 8192-entry cache | none | triaged (defaults read) |
| wreq (+ wreq-python, ex-rnet) | lib | https://github.com/0x676e67/wreq | Apache-2.0 | 1100 (py: 1515) | v0.16.3 (2026-10-30); crates.io `wreq` 6.0.0-rc.31 | 295 / 2026-11-04 | yes | impersonating client | forked hyper/http2 + BoringSSL fork (`btls`) | wreq-python bench suite (Python clients) | triaged |
| impit (Apify) | lib | https://github.com/apify/impit | Apache-2.0 | 634 | js-0.14.8 (2026-10-29) | 236 / 2026-11-06 | yes | impersonating client | reqwest + forked rustls/h2/hyper-util | none | triaged |
| primp | lib | https://github.com/deedy5/primp | MIT | 625 | v2.0.1 (2026-09-12) | 113 / 2026-09-13 | yes | impersonating client | own `primp-rustls` + `primp-h2` forks | none in README | triaged |
| koon | lib | https://github.com/scrape-hub/koon | MIT | 40 | v2.0.0 (2026-11-05) | 88 / 2026-11-03 | yes (new, 2026-02) | impersonating client incl. HTTP/3 | BoringSSL fork + forked quinn/h3/http2 | none | triaged |
| ureq | lib | https://github.com/algesten/ureq | MIT/Apache-2.0 | 2228 | 3.5.1 (tag) | 58 / 2026-11-02 | yes | blocking client | sync, thread per request | none | triaged |
| isahc | lib | https://github.com/sagebind/isahc | MIT | 790 | 2.0.2 (2026-10-21) | 204 / 2026-11-06 | yes (revived) | libcurl-based async client | libcurl multi | none | triaged |
| cronet-rs | lib | https://github.com/sleeyax/cronet-rs | none | 39 | tag only | 0 / 2025-07-28 | **no (archived)** | Chromium net stack bindings | FFI | none | triaged |
| rama | framework | https://github.com/plabayo/rama | MIT/Apache-2.0 | 1258 | rama-0.4.0 (2026-08-19) | 893 / 2026-11-06 | yes | proxy/client framework with fingerprints | tower-style services | none found | triaged (discovered) |
| lol-html | lib | https://github.com/cloudflare/lol-html | BSD-3 | 2093 | v3.0.3 (2026-10-28) | 93 / 2026-10-30 | yes | streaming parse/rewrite (Foxhound, Spider) | streaming, dual parser + selector VM | yes (Cloudflare blog; third-party) | deep |
| html5ever | lib | https://github.com/servo/html5ever | MIT/Apache-2.0 | 2656 | crates 0.40.1 (2026-09-14) | 56 / 2026-11-03 | yes | spec tree builder | full WHATWG tree construction | third-party only | triaged |
| scraper | lib | https://github.com/rust-scraper/scraper | ISC | 2429 | v0.29.0 (2026-10-21) | 31 / 2026-11-03 | yes | DOM + CSS select | html5ever + ego-tree | third-party only | triaged |
| tl | lib | https://github.com/y21/tl | MIT | 408 | 0.7.8 (crates, 2024-01-29) | 0 / 2024-01-29 | **no** | zero-copy parser | not spec-compliant, borrowed slices | yes (author's bench repo) | triaged |
| astral-html | lib | https://github.com/astral-sh/astral-html | MIT/Apache-2.0 | 43 | v0.0.4 (2026-10-20) | 78 / 2026-10-22 | yes (new, 2026-10) | tokenizer + light document (uv) | WHATWG tokenizer, no tree repair | **yes, with method** | deep |
| html5gum | lib | https://github.com/untitaker/html5gum | MIT | 174 | 0.9.1 (2026-11-04) | 23 / 2026-11-04 | yes | WHATWG tokenizer (lychee) | tokenizer | in astral bench | triaged (discovered) |
| markupever | lib (Py) | https://github.com/awolverp/markupever | MIT | 40 | v0.5.0 (2026-11-05) | 105 / 2026-11-07 | yes | Python DOM lib, Rust core | html5ever `treedom` | docs page (Python) | triaged |
| dom_query | lib | https://github.com/niklak/dom_query | MIT | 95 | 0.29.0 (2026-10-23) | 242 / 2026-11-07 | yes | DOM query/mutation (Foxhound) | html5ever + tendril | none | triaged |
| kuchikiki | lib | https://github.com/brave/kuchikiki | MIT | 79 | v0.8.2 (2023-05-15) | 0 / 2025-11-15 | dormant | DOM manipulation | html5ever + rc tree | none | triaged |
| select.rs | lib | https://github.com/utkarshkukreti/select.rs | MIT | 1018 | 0.6.1 (tag) | 0 / 2025-03-19 | **no** | DOM select | html5ever rcdom | none | triaged |
| scrape-rs (bug-ops) | lib | https://github.com/bug-ops/scrape-rs | Apache-2.0 | 10 | v0.2.14 (2026-10-21) | 264 / 2026-11-02 | yes (new) | DOM + bindings | html5ever, optional lol_html | README claims, no method | triaged |
| Selma | Ruby gem | https://github.com/gjtorikian/selma | MIT | 86 | v0.5.4 (2026-10-29) | 75 / 2026-10-29 | yes | Ruby sanitizer | lol_html | none | triaged |
| rs-trafilatura | lib | https://github.com/Murrough-Foley/rs-trafilatura | Apache-2.0 (crate: MIT/Apache) | 68 | v0.1.1 GitHub (2026-03-24); crate 0.2.2 (2026-04-03) | 24 / 2026-04-03 | slowing (no commit since April) | main content | dom_query + page-type ML | **yes (ScrapingHub, WCXB)** | deep |
| trafilatura-rs (nchapman) | lib | https://github.com/nchapman/trafilatura-rs | Apache-2.0 | 9 | v0.6.0 (2026-10-28) | 124 / 2026-10-28 | yes | main content (go-trafilatura port) | scraper/html5ever | **yes (method, 960-entry set)** | deep |
| libreadability (nchapman/readability-rs) | lib | https://github.com/nchapman/readability-rs | MIT | 1 | v0.3.0 tag; crate 2026-10-15 | 44 / 2026-10-15 | yes | readability port | scraper/html5ever | yes (vs Go, M4 Max) | deep (README) |
| dom_smoothie | lib | https://github.com/niklak/dom_smoothie | MIT | 232 | 0.19.0 (2026-10-27) | 127 / 2026-11-05 | yes | readability port (Foxhound) | dom_query | **yes (ScrapingHub, third-party)** | deep |
| readabilityrs | lib | https://github.com/theiskaa/readabilityrs | Apache-2.0 | 95 | v0.1.4 (2026-08-10) | 74 / 2026-10-02 | yes | readability port | scraper | yes (vs Readability.js) + ScrapingHub | triaged |
| legible | lib | https://github.com/ryanfowler/legible | Apache-2.0 | 4 | crate 0.5.1 (2026-08-08) | 621 / 2026-09-30 | yes (commit volume anomalous: 621 commits, 4 stars) | content + Markdown | html5ever rcdom | none | triaged |
| llm-readability (spider) | lib | https://github.com/spider-rs/readability | MIT | 17 | crate 0.0.17 (2026-04-30) | 10 / 2026-04-30 | low | readability (Spider) | html5ever | ScrapingHub (third-party) | triaged |
| readability (kumabook) | lib | https://github.com/kumabook/readability | MIT | 134 | 0.3.0 (2023-12-20) | 0 / 2023-12-20 | **no** | arc90 readability | html5ever | ScrapingHub (third-party) | triaged |
| article_scraper (news-flash) | lib | https://gitlab.com/news-flash/article_scraper | GPL-3.0+ | 6 (GitLab) | 3.0.2 (2026-10-13) | 112 / 2026-10-13 | yes | FTR site rules + readability | dom_query + dom_smoothie | none | triaged |
| lectito | lib | https://github.com/stormlightlabs/lectito | MPL-2.0 | 12 | v0.3.1 tag (crate 2026-08-18) | 138 / 2026-08-18 | yes | readability variant | — | none | triaged |
| decruft | lib | https://github.com/jamtur01/decruft | MIT | 1 | v0.2.0 (2026-05-25) | 137 / 2026-05-25 | slowing | defuddle port | — | none | triaged |
| dom-content-extraction | lib | https://github.com/oiwn/dom-content-extraction | GPL-3.0 (crate MPL-2.0) | 45 | crate 0.4.5 (2026-09-22) | 19 / 2026-09-22 | yes | CETD text density | — | none | triaged |
| trek (officialunofficial; crate `trek-rs`) | lib | https://github.com/officialunofficial/trek | none stated | 53 | trek-rs 0.2.1 (2025-06-28) | 2 / 2026-09-18 | low | content extraction, WASM | — | none | triaged |
| Resiliparse 2.x | lib | https://github.com/chatnoir-eu/chatnoir-resiliparse | Apache-2.0 | 153 | v2.0.1 (2026-11-04) | 611 / 2026-11-04 | yes | HTML2Text main content, WARC | **Rust core since 2.0; HTML parse via Lexbor (C, FFI)** | **yes (SIGIR 2023 paper)** | deep |
| htmd | lib | https://github.com/letmutex/htmd | Apache-2.0 | 463 | v0.5.6 (2026-10-15) | 195 / 2026-10-20 | yes | HTML→Markdown (Foxhound) | html5ever rcdom (turndown-like) | yes (author, M4) | deep (README) |
| fast_html2md (spider-rs/html2md) | lib | https://github.com/spider-rs/html2md | MIT | 79 | crate 0.0.63 (2026-09-08) | 10 / 2026-09-08 | low | HTML→Markdown (Spider) | lol_html rewriter, optional stream | benches, no published figures | deep |
| mdka | lib | https://github.com/nabbisen/mdka-rs | Apache-2.0 | 59 | 3.3.0 (2026-10-06) | 434 / 2026-10-14 | yes | HTML→Markdown | scraper | yes (allocation per conversion; own history) | triaged |
| html2text | lib | https://github.com/jugglerchris/rust-html2text | MIT | 245 | 0.17.4 (2026-10-18) | 61 / 2026-10-18 | yes | HTML→text rendering | html5ever | none | triaged |
| html2md-rs | lib | https://github.com/izyuumi/html2md-rs | MIT | 26 | v0.12.2 (2026-08-09) | 32 / 2026-08-09 | yes | HTML→Markdown | own parser | none | triaged |
| html-to-markdown (xberg-io) | lib + bindings | https://github.com/xberg-io/html-to-markdown | MIT | 887 | v3.21.2 (2026-11-07) | 2708 / 2026-11-07 (anomalous volume) | yes | HTML→Markdown | astral-html + html5ever | claim "19–116 MB/s" | triaged (discovered) |
| texting_robots | lib | https://github.com/Smerity/texting_robots | MIT/Apache-2.0 | 28 | 0.2.2 (2023-03-29) | 0 / 2024-02-14 | **no** | robots.txt | nom + MinRegex | none | triaged |
| robotstxt (Google port) | lib | https://github.com/Folyd/robotstxt | Apache-2.0 | 102 | 0.3.0 (2021-02-13) | 0 / 2021-02-13 | **no** | robots.txt | C++ port | used as baseline by fast-robots | triaged |
| robotxt + sitemapo (spire-rs) | lib | https://github.com/spire-rs/robotxt , https://github.com/spire-rs/sitemapo | MIT | 2 / 2 | crates 0.7.0 / 0.3.0 (2026-10-21) | 6 / 5, 2026-10-28 | yes (tiny) | robots + sitemaps | sitemapo on quick-xml | none | triaged |
| fast-robots | lib | https://github.com/zeon256/fast-robots | MIT/Apache-2.0 | 2 | v0.1.5 (2026-06-25) | 43 / 2026-06-25 | yes (new) | robots.txt | zero-copy, memchr | **yes, with method** | deep |
| robotparser-rs | lib | https://github.com/messense/robotparser-rs | MIT | 18 | v0.11.0 (2022) | 0 / 2022-03-30 | **no** | robots.txt | — | none | triaged |
| sitemap (svmk) | lib | https://github.com/svmk/rust-sitemap | MIT | 29 | 0.4.1 (2020-11-03) | 0 / 2021-11-24 | **no** (Spider still uses it) | sitemap reader | xml-rs | none | triaged |
| sitemap-rs | lib | https://github.com/goddtriffin/sitemap-rs | MIT | 34 | v0.7.0 (2026-09-27) | 23 / 2026-09-27 | yes | sitemap **writer** only | — | none | triaged |
| quick-xml | lib | https://github.com/tafia/quick-xml | MIT | 1580 | v0.42.0 (2026-08-22) | 216 / 2026-11-02 | yes | streaming XML (sitemaps) | pull, borrowed events | crate benches | triaged |
| adblock-rust | lib | https://github.com/brave/adblock-rust | MPL-2.0 | 2919 | crate 0.13.5 (2026-10-28) | 180 / 2026-10-28 | yes | network + cosmetic filtering (Foxhound) | tokenised filters, FlatBuffers | **yes (Brave blog, CI)** | deep |
| url (rust-url) | lib | https://github.com/servo/rust-url | MIT/Apache-2.0 | 1585 | v2.5.7 GitHub; crate 2.5.8 (2026-01-05) | 14 / 2026-10-30 | yes | WHATWG URL | one String + u32 offsets | via ada | deep (layout) |
| ada-url (Rust bindings) | lib | https://github.com/ada-url/rust | MIT/Apache-2.0 | 123 (C++ core 1901) | v4.0.0 (2026-07-27) | 42 / 2026-11-01 | yes | WHATWG URL, C++ core via FFI | — | **yes (README + paper)** | deep (README) |
| fastbloom | lib | https://github.com/tomtomwombat/fastbloom | MIT/Apache-2.0 | 361 | v0.17.0 (2026-10-03) | 30 / 2026-10-14 | yes | Bloom filter | one hash, atomic variant | yes (author bench repo) | triaged |
| growable-bloom-filter | lib | https://github.com/dpbriggs/growable-bloom-filters | MIT | 30 | 2.1.1 (2024-10-06) | 0 / 2024-10-06 | dormant | growable Bloom | — | none | triaged |
| xorf | lib | https://github.com/ayazhafiz/xorf | MIT | 171 | 0.13.0 (2026-08-21) | 7 / 2026-11-06 | yes | static xor / binary fuse filters | static | papers | triaged |
| qfilter | lib | https://github.com/arthurprs/qfilter | MIT | 27 | v0.3.1 (2026-06-14) | 13 / 2026-06-14 | yes | quotient filter (growable, removable) | RSQF | README table | triaged |
| rust-cuckoofilter | lib | https://github.com/axiomhq/rust-cuckoofilter | MIT | 298 | 0.5.0 (2020) | 0 / 2025-10-27 | dormant | cuckoo filter | — | none | triaged |
| foldhash | lib | https://github.com/orlp/foldhash | Zlib | 388 | v0.2.0 tag (2025-08-23) | 1 / 2026-06-29 | yes (stable) | hasher (hashbrown default) | — | **yes, with method** | triaged |
| aHash | lib | https://github.com/tkaitchuck/aHash | MIT/Apache-2.0 | 1349 | v0.8.12 | 0 / 2025-11-21 | dormant | hasher (Spider) | AES | in foldhash/rapidhash suites | triaged |
| rapidhash | lib | https://github.com/hoxxep/rapidhash | MIT/Apache-2.0 | 289 | v4.5.1 (2026-07-03) | 64 / 2026-07-03 | yes | hasher | — | **yes (foldhash suite)** | triaged |
| fst | lib | https://github.com/BurntSushi/fst | Unlicense/MIT | 2123 | 0.4.7 (2021) | 0 / 2024-09-25 | stable/dormant | compact static sorted sets | FST | **yes (blog, Common Crawl URLs)** | triaged |
| lasso | lib | https://github.com/Kixiron/lasso | MIT/Apache-2.0 | 173 | 0.7.3 (2024) | 0 / 2024-08-19 | dormant | string interner | arena | none | triaged |
| mimalloc (Rust) / mimalloc (C) | lib | https://github.com/purpleprotocol/mimalloc_rust , https://github.com/microsoft/mimalloc | MIT | 842 / 13527 | 0.1.52 (2026-05-22) / v3.6.1 (2026-10-30) | 27 / 2026-06-30; C: 1658 / 2026-10-30 | yes | allocator (Foxhound) | per-thread heaps, v3 default | mimalloc-bench (not crawler) | deep (defaults) |
| tikv-jemallocator | lib | https://github.com/tikv/jemallocator | MIT/Apache-2.0 | 559 | 0.7.0 (2026-05-25) | 19 / 2026-10-10 | yes | allocator | decay-based purge | none for crawlers | triaged |
| snmalloc-rs | lib | https://github.com/microsoft/snmalloc (crate moved into it) | MIT | 1981 | 0.7.6 (2026-10-02) | 71 / 2026-11-02 | yes (old repo SchrodingerZhu/snmalloc-rs archived) | allocator | message-passing frees | none for crawlers | triaged |
| tokio | runtime | https://github.com/tokio-rs/tokio | MIT | 33718 | 1.54.1 (2026-10-21) | 527 / 2026-11-06 | yes | runtime (everyone) | work-stealing epoll | none crawler-specific | triaged (defaults read) |
| monoio | runtime | https://github.com/bytedance/monoio (→ monoio-rs/monoio) | MIT/Apache-2.0 | 5132 | 0.2.4 (2024-08-20) | 4 / 2026-10-15 | **low** | thread-per-core io_uring | — | vendor echo bench (2021) | triaged |
| glommio | runtime | https://github.com/DataDog/glommio | Apache-2.0/MIT | 3667 | v0.7.0 GitHub (2022); crate 0.9.0 (2024) | 0 / 2025-04-21 | **no** | thread-per-core io_uring | — | none | triaged |
| compio (+ cyper client) | runtime | https://github.com/compio-rs/compio | MIT | 1944 (cyper 90) | v0.19.2 (2026-08-18) | 371 / 2026-11-07 (cyper 55) | yes | thread-per-core, io_uring/IOCP | completion-based | none | triaged |

### In-depth sections

#### lol-html (streaming parse/rewrite)
- **What:** Cloudflare's Low Output Latency streaming HTML rewriter with a CSS-selector API; back-end of Workers' HTMLRewriter and of ROFL in Cloudflare's FL proxy ("running in production on millions of responses a second", https://blog.cloudflare.com/rust-nginx-module/). 100% Rust; C and JS APIs in repo. Both Foxhound (`lol_html = 3.0.2`, goodfetch `Cargo.toml:115`) and Spider (`lol_html = 2`, spider `spider/Cargo.toml:57`) use it.
- **Health:** 2093 stars, created 2019-09-09, v3.0.3 2026-10-28, 93 commits in 2026, last 2026-10-30; crate 7.25 M downloads (2.45 M recent).
- **Design that sets its cost** (Cloudflare, "A History of HTML Parsing at Cloudflare: Part 2", https://blog.cloudflare.com/html-parsing-2/):
  - Dual parser: a *tag scanner* reads only tag names (hashed to a 64-bit integer) and feeds the selector matcher; the full *lexer* runs only while input is inside the scope of a matching selector or when attributes are needed. "As ever the best optimization is not to do something." Consequence: a match-all `*` selector, or any selector needing attributes on every tag, keeps the lexer on for the whole document.
  - Tokens are "outlines" (integer ranges into the input), so a tag split across chunks is extended by adjusting indices, without copying.
  - Selector matching is a non-backtracking VM run left-to-right on the stack of open elements; the only per-element state is the tag-name hash plus the index of the first ancestor with a pending descendant jump; selectors sharing prefixes are compiled once into a radix-tree program of macro-instructions. Cost therefore scales with the number of start tags *in scope* times the active VM threads, not with document size times selector count as in right-to-left browser matching.
  - Encoding: selectors are pre-encoded to the page charset, so matching runs on raw bytes.
- **Published figures:** Cloudflare (2019, chart only): tag scanner "typically twice as fast as LazyHTML", lexer comparable, "a few times faster than the tokenizer from html5ever". Third-party, with method: y21/rust-html-parser-benchmark (criterion, 2021, `cargo bench`): lol_html 396 MiB/s on wikipedia.html vs html5ever 50 MiB/s and tl 497 MiB/s (https://github.com/y21/rust-html-parser-benchmark). astral-html's 2026 run (below): link extraction on the 418,744-byte PEP index takes 2,857.74 µs with lol_html (≈147 MB/s, computed), vs 1,116.75 µs astral-html, 1,766.79 µs tl, 2,914.33 µs html5gum, 11,346.75 µs scraper.
- **Fundamentals from source (@ `1d5ee9e88f23`, v3.0.3):** deps are cssparser, selectors, encoding_rs, hashbrown+foldhash, memchr (`Cargo.toml:31-43`); `memchr` used in the state machine (`src/parser/state_machine/mod.rs:369`) and attribute matcher. `MemorySettings` default: 1024-byte preallocated parsing buffer, unlimited max memory (`src/rewriter/settings.rs:848-849`). The `element!` macro parses its selector string on every expansion: `Cow::Owned($selector.parse::<Selector>().unwrap())` (`src/rewriter/settings.rs:431-437`); `Selector` is `Clone` (`src/selectors_vm/parser.rs:280-281`) and handlers accept `Cow<'selectors, Selector>` (`settings.rs:972`), so selectors can be parsed once and lent with `Cow::Borrowed`; the VM program is still compiled per rewriter (`src/selectors_vm/mod.rs:145`). Recent perf work: v3.0.1 "improved performance of selector matching on deeply nested elements and on stray end tags", v3.0.2 many-attribute tags across chunks, v3.0.3 deeply nested end-tag handlers (`CHANGELOG.md:1-25`). Repo benches: parsing, rewriting, selector matching (match-all, tag, class, attribute, multiple) on three files (`benches/cases/selector_matching.rs`), results not published.
- **Relevance:** it remains the right streaming parser for Foxhound's one pass (#2164). Measured gap in the prior note: 178 k instructions/page in lol_html for Foxhound vs 62 k for Spider, 14 k of it building the rewriter. Levers: feed chunks as they arrive; parse `Selector`s once per Run and pass `Cow::Borrowed`; never register `*` or attribute-only selectors (they keep the lexer on); register only the handlers the requested format needs.

#### astral-html (new, Oct 2026)
- **What:** Astral's (uv's) HTML tokenizer plus a light `Document` "designed for document traversal"; one runtime dependency (`memchr`), no unsafe beyond memchr (README). README states all code changes were authored by an LLM ("GPT-6 Astra in Codex"). Used by `uv-client` (uv `Cargo.toml:123`, `astral-html = 0.0.4`) and by xberg-io/html-to-markdown; crates.io reverse deps: uv-client, html-to-markdown-rs, vtcode-core. The 187 k downloads in its first month come from uv being built from crates.io; they look organic.
- **Health:** 43 stars, created 2026-10-04, v0.0.4 2026-10-20, 78 commits, last 2026-10-22.
- **Conformance (docs/conformance.md @ `55e7df63469b`):** tokenizer passes 7,045 state-expanded html5lib cases with no expected failures, plus 2,252 WPT tokenizer cases. `Document` is *not* a browser tree: no element insertion, no table repair, no formatting-element reconstruction, `<p>one<p>two` nests; scripting assumed off; MathML unsupported; no encoding detection, no CSS selectors, no serialization.
- **Published benchmark, with method** (benchmarks/results/README.md @ `7ea072ad`): link extraction (owned href/title/rel), median of three session means, criterion 100 samples, shared KVM VM on AMD EPYC-Milan, pinned vCPU, thin LTO, 1 codegen unit, system allocator, `tl` 0.7.8 without nightly SIMD; all six adapters produced identical link records. Times (µs): iniconfig 5.70 / tl 14.75 / html5gum 14.99 / lol_html 17.08 / scraper 66.81; PEP index (418,744 bytes): 1,116.75 / 1,766.79 / 2,914.33 / 2,857.74 / 11,346.75. Document construction: astral 1,024 µs vs tl 1,124 vs scraper 9,958 on PEP index. CodSpeed CI tracks CPU simulation and memory (docs/performance.md).
- **Relevance:** the fastest published link extractor in Rust (2.6× lol_html on the largest fixture). It is not streaming across chunks (`Reader::new(&str)` takes the whole input) and does not build a spec tree, so it cannot replace lol_html's chunked pass or the html5ever tree that readability needs. Watch it as a candidate for a raw-mode link pass on a fully buffered body. #2164/#1838.

#### HTTP: hyper 1.x, hyper-util, reqwest (defaults that cost per request)
- **reqwest 0.13.5 defaults** (@ `8e1c8061e5dd`, `src/async_impl/client.rs`): `pool_idle_timeout = 90 s` (:306), `pool_max_idle_per_host = usize::MAX` (:307), `http2_adaptive_window = false` (:349), TCP nodelay on (:378), DNS resolver `GaiResolver` unless the `hickory-dns` feature is on (:379, :436-438).
- **Per-request work in reqwest:** the request's `url::Url` is re-parsed into an `http::Uri` on every send: `try_uri(url) = url.as_str().parse()` (`src/into_url.rs:77-81`, called at `client.rs:2677`), and every default header is cloned into the request's `HeaderMap` (`client.rs:~2671-2674`). Calling hyper-util's client with a `Uri` built once avoids the second parse and the header clones.
- **hyper-util pool** (@ `885c965642d3`, `src/client/legacy/pool.rs:27,77-95`): one `Arc<Mutex<PoolInner>>` per client holding `HashMap<Key, Vec<Idle>>` idle lists, a `HashSet` of connecting keys and `HashMap<Key, VecDeque<oneshot::Sender>>` waiters; every checkout and return takes the mutex and hashes the (scheme, authority) key; an idle-interval task is spawned (`pool.rs:425`).
- **DNS:** hyper-util's `GaiResolver` runs blocking `getaddrinfo` via `tokio::task::spawn_blocking` for every resolution, without a cache (`src/client/legacy/connect/dns.rs:45,122-124`). Tokio's blocking pool spawns threads on demand up to 512 (`tokio/src/runtime/builder.rs:324`) and keeps idle ones 10 s (`runtime/blocking/pool.rs:231`), so bursts of new hosts create threads, each with its own stack and allocator thread cache. hickory's resolver is async and caches 8,192 responses by default (`crates/resolver/src/config.rs:589-591,770-771`); reqwest's hickory integration uses `LookupIpStrategy::Ipv6AndIpv4` (`src/dns/hickory.rs:68`). No published figure for the per-lookup cost of either was found.
- **Published benchmarks:** none from hyper/reqwest. The prior note measured Spider's whole fetch path at about 70 k instructions/page (hyper + reqwest).
- **Impersonation clients:** wreq (forked hyper/http2 + `btls` BoringSSL fork; already Foxhound's TLS-impersonation tier), impit (reqwest + Apify forks of rustls/h2/hyper-util), primp v2 (own rustls and h2 forks), koon (BoringSSL fork + forked quinn/h3 for HTTP/3). All fork the protocol stack to control ClientHello/SETTINGS order; none publishes Rust-side throughput or allocation figures (wreq-python has a Python-client benchmark suite only).
- **What a Spider-beating crawler should use:** hyper 1.x via hyper-util's client (or reqwest with these defaults overridden): build `Uri` once per frontier URL; cap idle connections per host to the per-host concurrency (#2194, #2165); a cached async resolver (hickory, or a per-site DNS cache owned by the Politeness actor) instead of `GaiResolver` to remove the blocking-pool hop and thread growth (#1838, #2165).

#### adblock-rust (Brave)
- **What:** Brave's network and cosmetic filter engine (ABP/uBO syntax). Foxhound uses it for cosmetic clean-up (`adblock = 0.13.3`, goodfetch `Cargo.toml:52`); Spider only through `chromey/adblock` in browser mode (`spider/Cargo.toml:249-250`).
- **Health:** 2919 stars, crate 0.13.5 2026-10-28, 180 commits in 2026, last 2026-10-28.
- **Published figures:** 2019 rewrite, 69× faster than the previous C++ engine; EasyList+EasyPrivacy at 5.7 µs per request on average, 4.6 µs with the browser-preparsed API; method: Cliqz dataset, 242,944 requests from the top 500 domains, 2018 MacBook Pro 2.6 GHz i7 (https://brave.com/blog/improved-ad-blocker-performance/). 2025–26: moving ~100,000 default filters into FlatBuffers cut engine memory by 75% (~45 MB); 19% fewer allocations from stack vectors, build time −15%, matching +13% by tokenising common regex filters (https://brave.com/privacy-updates/36-adblock-memory-reduction/, PR https://github.com/brave/adblock-rust/pull/554 with criterion: `rule-match-browserlike/brave-list` 1.57 s → 1.36 s). Its CI benchmark (PR #554 comment) reports for the Brave list: build 174.6 ms vs deserialize 24.5 ms (7.1×), 10.2 MB live after load (62.3 MB peak during build, 1.36 M allocations), 2.67 MB allocated for 1,000 requests, `url_cosmetic_resources` 190 µs, cosmetic class/id match 3.3 ms per bench iteration.
- **From source (@ `09d297c2c232`, `src/engine.rs`):** `serialize()` (:436) / `deserialize()` (:450) give a 7× faster start than building from rules; `hidden_class_id_selectors` (:367) answers only for the classes and ids present on the page; `url_cosmetic_resources` (:381) gives per-host rules.
- **Relevance:** serialise the engine once at build time and deserialize at start (#2165 start-up and peak build memory); query cosmetic rules per host once, and per page only for the class/id tokens seen in the streaming pass (#2164).

#### Main-content extraction: published accuracy
- **ScrapingHub article-extraction-benchmark (181 articles; official results updated after Rust backends were added on 2026-02-05, commit `62364afa17`; repo pushed 2026-05-29)** — F1 (± bootstrap), https://github.com/scrapinghub/article-extraction-benchmark: rs_trafilatura (git `9261e08`) **0.970**; trafilatura 2.0.0 (Python) 0.958; go_trafilatura 0.960; readability_js 0.947; go_readability_fork 0.947; readability-lxml 0.922; readable_readability 0.884; readability (kumabook) 0.3.0 0.873; **dom_smoothie 0.14.0 0.865**; readabilityrs 0.1.2 0.832; llm_readability 0.0.13 0.829; boilerpipe_rs 0.739; converters without extraction: html2text_rs 0.438, mdka 0.435, fast_html2md 0.515, htmd 0.184, html2md_rs 0.150. dom_smoothie was run with default `Readability::parse()` and its `content` converted to text (`extractors/rust_extractors/src/main.rs:66-70`). Foxhound vendors dom_smoothie 0.18.2; the benchmark has not re-run newer versions.
- **Bevendorff et al., "An Empirical Comparison of Web Content Extraction Algorithms", SIGIR 2023** (8 datasets combined, ROUGE-LSum; https://downloads.webis.de/publications/papers/bevendorff_2023c.pdf, Table 3): mean F1 Trafilatura 0.883, Readability 0.861 (best median 0.970, most robust), Resiliparse 0.859, DOM Distiller 0.858, Boilerpipe 0.834, justext 0.759. The authors' slides: Resiliparse is "the fastest by an order of magnitude" (https://downloads.webis.de/publications/slides/bevendorff_2023b.pdf); no absolute timing in the table.

#### rs-trafilatura
- **What:** pure-Rust Trafilatura-style extractor on dom_query, with an ML page-type classifier, per-type profiles (forums, docs, products) and an XGBoost extraction-quality predictor (README). Crate 0.2.2 (2026-04-03); no commit since 2026-04-03.
- **Published figures (author, README):** speed on 1,497 files (462 MB) on Linux x86_64: 21.8 ms/file overall (45.8 files/s), articles 14.1 ms (225 KB average); ScrapingHub F1 0.966 (README) / 0.970 (official table); WCXB (the author's own benchmark, webcontentextraction.org) F1 0.859 dev / 0.893 held-out. Single-thread or not, CPU model and allocator are not stated, so the speed figure is a claim.
- **Relevance:** the accuracy leader in Rust, but at ~14 ms per article it costs far more CPU than readability; good reference for a quality setting, not the default fast path (#2164).

#### trafilatura-rs (nchapman) and libreadability (nchapman/readability-rs)
- **trafilatura-rs** (port of go-trafilatura; scraper/html5ever): per-document Rust 606 µs (6 KB), 3.84 ms (85 KB), 2.90 ms (382 KB), 8.81 ms (906 KB) vs Go and Python; quality on a 960-entry presence/absence set: F-score 0.913 vs Python 0.920 vs Go 0.916; Apple M4 Max, criterion medians, reproduce commands in README "Benchmarks" (@ `1d2d12d252af`).
- **libreadability** (crate `libreadability` 0.3.0; scraper/html5ever): vs Go readeck/readability: ars-1 854 µs vs 1.85 ms; all 133 fixtures 320 ms vs 670 ms (2.1×); Apple M4 Max (README "Benchmarks" @ `ec8fad33be81`).
- **readabilityrs** for contrast: 36 ms on a 12 KB page, slower than Readability.js under JSDOM below ~150 KB (README "Benchmarks" @ `5959b20eb253`). Avoid.

#### dom_smoothie (Foxhound's readability)
- **What:** Readability.js port on dom_query (html5ever + tendril). 232 stars, 0.19.0 2026-10-27, 127 commits in 2026.
- **Design note (README):** `Readability::parse` retries with successively looser `ParsePolicy` flag sets and keeps the best attempt; `parse_with_policy` makes one attempt, "leading to significantly lower memory consumption", and `Strict` is the slowest. Part of the filtering is global across attempts, which differs from Readability.js and "may result in duplicate headings".
- **Figures:** ScrapingHub F1 0.865 for 0.14.0 (above), well below readability_js 0.947 on the same set. The prior Foxhound note measured 0.78 M instructions per page for it.
- **Relevance:** the accuracy gap (0.865 vs 0.947) and the multi-attempt memory are worth re-measuring on 0.19 before choosing a single-policy fast path (#2164, #2165).

#### Resiliparse 2.x
- **What:** Webis/ChatNoir web-archive toolkit. Since 2.0.0 (2026-10-31) the full processing API is in Rust with Python bindings on the same core ("Remove the legacy Cython implementations", CHANGELOG.md:9-25 @ `af6735f862ee`). HTML parsing still goes through vendored **Lexbor (C)** via FFI (`resiliparse-rs/src/third_party/lexbor.h`, `build.rs`); main-content extraction is `resiliparse-rs/src/extract/html2text.rs`. So: Rust core, C parser.
- **Health:** 153 stars, v2.0.1 2026-11-04, 611 commits in 2026; crate `resiliparse` 2.0.1 has 257 downloads (brand-new on crates.io).
- **Figures:** SIGIR 2023 mean F1 0.859 (3rd), fastest by an order of magnitude per the authors (above); FastWARC has a WARC parser benchmark suite (`benchmarks/warc/`).
- **Relevance:** a fast precision-oriented extractor with a C dependency; Lexbor is the fastest C HTML parser in common use, but Foxhound's single static binary would gain a C build. Candidate to benchmark against dom_smoothie for instructions/page (#2164).

#### HTML→Markdown: htmd, fast_html2md
- **htmd** (Foxhound's converter): turndown-like on html5ever rcdom. Author's benchmark: 1.37 MB Wikipedia page in 14.5 ms (0.5.0, criterion, Apple M4; `benches/README.md` @ `461ffa772cab`) ≈ 94 MB/s.
- **fast_html2md** (Spider's): default `rewriter` feature on lol_html, `stream` feature for async chunked input (`fast_html2md/Cargo.toml` features @ `93cbdc732272`); it registers `element!("*", …)` (`fast_html2md/src/rewriter/writer.rs:187`), so every tag goes through the lexer and the match-all path; criterion benches (`benches/throughput.rs`) exist but no published results. Spider's `spider_transformations` combines it with llm_readability.
- **html-to-markdown (xberg-io)**: claims 19–116 MB/s on a Wikipedia/mdream corpus with per-PR regression thresholds (README line 108); now on astral-html + html5ever. Claim only here.
- **Relevance:** Markdown after readability works on the small extracted tree, so converter speed matters less than not building a second tree; streaming Markdown straight from lol_html (fast_html2md's approach) only fits the no-readability path (#2164).

#### fast-robots (robots.txt)
- **What:** zero-copy RFC 9309 parser (rules borrow the input), memchr scanning, optional compiled matcher; 2 stars, created 2026-04-24, v0.1.5 2026-06-25.
- **Published benchmark, with method** (`BENCHMARK.md` @ `0bcd35c4636b`): Apple M1, `-C target-cpu=native`, bench-only mimalloc; parse 0.98–2.1 GiB/s; parse+match vs Google-port `robotstxt` 4.2–9.0× faster (common file 312 ns vs 2.30 µs); compiled matcher 332 ns per batch on rule-heavy files vs 90 µs uncompiled; real corpus of 6,863 robots.txt (9.49 MB).
- **Relevance:** robots.txt is parsed once per host, so parse speed is not a crawl bottleneck; the zero-copy layout (one buffer + range vectors per host) is the memory-relevant idea (#2138/#1838). texting_robots (Smerity; Common Crawl provenance, thorough tests) and Google's port are both unmaintained.

#### URL handling and dedup, bytes per URL
- **url crate:** `Url` = one `String` + u32 offsets + `HostInternal` + options (`url/src/lib.rs:227-249`); `size_of::<url::Url>() = 88` bytes on x86_64 (measured with url 2.5.8, a probe written for this note), plus the heap string (47 bytes for a typical URL, rounded up to the allocator size class).
- **ada-url (C++ via FFI):** README benchmark: parse 2.02 µs vs url 7.06 µs on the same input set (3.49×), `can_parse` 1.21 µs (`README.md:64-77` @ `fb173256e246`); method in Nizipli & Lemire, "Parsing Millions of URLs per Second" (Software: Practice and Experience, 2023). Adds a C++ build to the binary.
- **Bytes per URL each structure implies** (computed; formulas in brackets):
  - `HashSet<url::Url>`: ~88 + ~48–80 heap + 1 control byte, ÷ load factor (0.875 max, 0.44 after a resize) ≈ 160–390 B.
  - Interned string (Spider: string-interner `BufferBackend`/`StringBackend`/`BucketBackend` with `SymbolUsize`, `spider/src/utils/interner.rs:29-43`) + `HashSet<SymbolUsize>`: string bytes + 8-byte symbol + 1 control byte ÷ load ≈ 60–100 B.
  - `HashSet<u64>` fingerprint (hashbrown): 9 B/slot ÷ load = 10.3–20.6 B; 64-bit collision probability at 10 M URLs ≈ 2.7×10⁻⁶ (n²/2⁶⁵).
  - Bloom filter at a 1% false-positive rate: 9.59 bits ≈ 1.2 B. Spider's `MmapBloom` sizes for 1 M URLs at 1% and 7 hashes, rounds the bit count up to a power of two (16,777,216 bits = 2 MiB, 16.8 bits/URL, false-positive rate ≈ 0.05% at capacity), mmaps it (tries `MAP_HUGETLB` first), and uses it only as a negative pre-check before the exact set (`spider/src/utils/bloom.rs:16-25,77-81,115-130`; `interner.rs:287-328`).
  - Static filters (xorf): xor8 ≈ 9.84 bits/entry, binary fuse8 ≈ 9 bits/entry (Graf & Lemire, arXiv:1912.08258 and arXiv:2201.01174) but no inserts: only for sealed batches. qfilter: growable and supports removal (README).
  - FST (BurntSushi `fst`): 1,649,195,774 deduplicated Common Crawl URLs (134 GB raw) in a 27 GB FST set ≈ 16.4 B/URL; build 82 min from sorted input with 56 MB max memory (https://burntsushi.net/transducers/). Static and sorted-build only: fits a sealed on-disk dedup tier, not the live set.
  - fastbloom: one hash per item split into derived indices, "2–20 times faster" than other Rust Bloom crates (author's bench repo https://github.com/tomtomwombat/bench-bloom-filters), `AtomicBloomFilter` for lock-free sharing.
- **Hashers (foldhash suite: hashonly/lookupmiss/lookuphit/setbuild over many key types, ns/op):** AMD EPYC 9R14, no `target-cpu=native`: geometric mean rapidhash-f 4.67, foldhash-f 5.27, fxhash 6.13, ahash 6.50, siphash 23.66 (rapidhash `README.md:183-187` @ `7eb34028b77e`); Xeon 8481c: foldhash-f 6.21, foldhash-q 7.01, fxhash 7.56, ahash 8.74, siphash 28.70 (foldhash `README.md` "Performance" @ `77d8e3dae0d4`). foldhash is hashbrown's default; Spider uses ahash (`spider/Cargo.toml:29`).
- **Relevance:** the live dedup set can be 10–20 B/URL (u64 fingerprints in hashbrown) instead of 100+ B for strings, with the full URL only in the Run file; a Bloom/fuse tier only pays off as a pre-check before a disk lookup (#2138, #2165).

#### Allocators (mimalloc v2/v3, jemalloc, snmalloc, glibc)
- **What Foxhound runs:** `mimalloc = 0.1.48` with default features off (goodfetch `Cargo.toml:288`), locked at mimalloc 0.1.52 / libmimalloc-sys 0.1.49 (`Cargo.lock:4222-4223,4416-4417`). mimalloc_rust switched its default to **mimalloc v3** on 2026-04-19 ("Use v3 by default", libmimalloc-sys 0.1.46+; README: "By default this library uses mimalloc v3"), so Foxhound runs v3.3.x unless the `v2` feature is set.
- **v3 defaults that matter for peak RSS** (microsoft/mimalloc @ v3.6.1, `src/util/options.c`): `purge_delay = 1000 ms` (:148; v2.1.7 had 10 ms, `src/options.c:78`), `allow_thp = 2` on Linux, i.e. THP allowed and minimum purge size raised to 2 MiB so huge pages are not split (:102-107, :176-177; README changelog for v3.5.1: "may increase rss … Use -DMI_ALLOW_THP=OFF"). Foxhound's vendored v3.3 predates the THP default, but a libmimalloc-sys bump will bring it in. Runtime overrides: `MIMALLOC_ALLOW_THP=0`, `MIMALLOC_PURGE_DELAY=N`.
- **THP in general:** kernel docs: with THP enabled system-wide "application may end up allocating more memory resources"; a process can opt out with `prctl(PR_SET_THP_DISABLE, 1, 0, 0, 0)` (https://docs.kernel.org/admin-guide/mm/transhuge.html).
- **Published comparisons for async network servers** (none controlled and crawler-like):
  - rAMQP issue #23 (Tokio broker, 20-min connection-churn soak): glibc RSS +64% with throughput 719 k → 170 k msg/s; jemalloc RSS +12%, 1,033 k → 916 k msg/s; churn=0 control flat (https://github.com/ZerosAndOnesLLC/rAMQP/issues/23).
  - Write-up on a bursty Tokio service: RSS stayed at peak with glibc and with mimalloc v3, attributed to v3's cross-thread `xthread_free` lists reconciled only by later allocation on the owning page while work-stealing workers idle; jemalloc's decay-timer purge returned memory (https://pranitha.dev/posts/rust-and-memory-allocators/, 2026-06-17; anecdotal).
  - Mark Callaghan, MyRocks (not Rust): glibc peak RSS 36.2 GB vs jemalloc 12.2 GB; `MALLOC_ARENA_MAX=1` cuts RSS to 14.1 GB but performance is "lousy" (http://smalldatum.blogspot.com/2025/04/battle-of-mallocators-part-2.html).
  - mimalloc's own mimalloc-bench covers allocator microbenchmarks, not a crawler.
- **Tokio worker count:** no published RSS-per-worker figure was found. From the mechanisms above: every worker thread (and every blocking-pool thread from `GaiResolver`) gets its own allocator thread heap/arena (glibc: up to 8 × cores arenas), so fewer workers and no blocking-pool DNS directly bound the allocator's partially filled pages. Spider sets no global allocator (glibc) per the prior note.
- **Relevance (#2165):** one allocator (mimalloc v3 is fine) with `allow_thp=0` (or the process-wide prctl) and a short `purge_delay` to cap peak RSS; fewer runtime threads; jemalloc is the documented fallback when work-stealing plus idle workers keeps mimalloc pages pinned. Measure; do not assume.

#### Async runtimes
- **Tokio multi-thread** is what every Rust crawler in scope uses. Defaults read: blocking pool max 512 threads, 10 s keep-alive (above). No published crawler-like comparison of multi-thread vs current_thread-per-core found.
- **io_uring thread-per-core:** monoio's own benchmark (ByteDance, 2021, echo server, client and server on separate machines, Xeon Gold 5118): Monoio ≈2× Tokio peak at 4 cores, ≈3× at 16 cores, slower than Tokio at 1 core with few connections (https://github.com/bytedance/monoio/blob/master/docs/en/benchmark.md). Vendor benchmark on echo traffic, not an HTTP client. monoio is barely maintained (4 commits in 2026, last release 2024-08); glommio is unmaintained; compio is active (371 commits in 2026) with the `cyper` HTTP client (90 stars). No published HTTP-client throughput comparison for io_uring runtimes was found.
- **Relevance:** switching runtime would break the ecosystem (hyper-util, reqwest, wreq, tokio-tungstenite) for an unmeasured gain on a network-latency-bound workload; the syscall gap (Spider 12 vs Foxhound 61 per page) is better attacked inside Tokio first (#1838).

### Role summary: state of the art and what to use

| Role | State of the art (evidence) | Use to beat Spider on instructions/page and memory |
|---|---|---|
| HTTP client | hyper 1.x + hyper-util (reqwest wraps it; impersonation forks wreq/impit/primp/koon) — no published figures | hyper-util client with prebuilt `Uri`, no per-request header clones, idle cap per host, cached async DNS (hickory or own) instead of `GaiResolver` |
| Streaming parse | lol-html (Cloudflare design + third-party 396 MiB/s); astral-html is 2.6× faster for link extraction but not chunk-streaming nor a spec tree | lol-html on chunks as they arrive, selectors parsed once (`Cow::Borrowed`), no `*` selectors |
| Spec tree | html5ever (scraper/dom_query on top) | only when extraction is requested |
| Main content | accuracy: rs-trafilatura (F1 0.970, ~14 ms/article claim); speed: Resiliparse (SIGIR 2023, Lexbor C core); Rust readability ports trail Readability.js (dom_smoothie 0.865 vs 0.947) | keep dom_smoothie but single policy on the fast path, and benchmark libreadability (0.854 ms ars-1) and Resiliparse on `just compare`'s extraction cell |
| HTML→Markdown | htmd ≈94 MB/s (author); fast_html2md streams via lol_html but uses `*` | convert the readability output tree, not a second parse |
| robots/sitemaps | fast-robots (zero-copy, published method); quick-xml for streaming sitemaps (Spider still on unmaintained `sitemap` 0.4/xml-rs) | zero-copy per-host rules; quick-xml streaming sitemap reader |
| Ad/tracker | adblock-rust (5.7 µs/request 2019; FlatBuffers −75% memory 2026; deserialize 7× faster than build) | serialise once, per-host cosmetic resources, class/id-gated hiding selectors |
| URL/dedup | url (88 B + heap); ada 3.5× faster parse (C++); u64 fingerprints 10–20 B/URL; FST 16.4 B/URL sealed | fingerprints in memory, full URLs in the Run file; Bloom/fuse only before disk lookups |
| Allocator | mimalloc v3 / jemalloc; no crawler benchmark | one allocator, THP off, short purge delay, fewer threads |
| Runtime | Tokio; io_uring runtimes unproven for HTTP clients | stay on Tokio, fewer workers |

### Only triaged (no deep source read)
hyper internals beyond defaults, hickory internals, wreq, impit, primp, koon, ureq, isahc, cronet-rs, rama, html5ever, scraper, tl, html5gum, markupever, dom_query, kuchikiki, select.rs, scrape-rs, Selma, readabilityrs (README only), legible, llm-readability, kumabook readability, article_scraper, lectito, decruft, dom-content-extraction, trek, mdka, html2text, html2md-rs, html-to-markdown (xberg), texting_robots (only rule storage checked), robotstxt, robotxt/sitemapo, robotparser-rs, sitemap, sitemap-rs, quick-xml, growable-bloom-filter, xorf, qfilter, cuckoofilter, foldhash, aHash, rapidhash, fst, lasso, jemallocator, snmalloc, tokio internals, monoio, glommio, compio/cyper.

## Not verified, by part

### Part A1

- The bytes-per-URL figure is computed from data-structure layouts, not measured.
- That the idle broadcast receiver causes Spider's memory growth with site size is an inference from the source; it was not measured.
- Whether `tcp_nodelay` is on: reqwest's default was not checked at the resolved version.
- The `balance` monitor never starting rests on grep finding no caller of `get_global_cpu_state()`/`get_global_memory_state()` in `spider/src` at 2.53.9; a caller in another crate (e.g. spider_cli) was not searched.
- Stargazer timestamps (API refused the token with 403); organic-star judgment rests on forks, downloads and age.
- All Spider Cloud figures (100K+ pages/s, 74/182 pages/s, 99.9% success, the 1 M-page run, browser benchmark scores) are self-reported; the 1,000-URL corpus is not published, and the closed-source browser runtime could not be inspected.
- `benches/BENCHMARKS.md` figures (2024) could not be reproduced and look implausible as wall times.
- Changes on `main` after 2.53.9 were read only as commit subjects.

### Part A2

- Stargazer timelines for every project: the token gets 403 on REST and GraphQL stargazers. Star plausibility was judged from engagement and downloads only.
- pagesift's and crawlee-rs's benchmark numbers (nothing was run). pagesift's Spider arm may be memory-inflated by `subscribe(256)` per site; Spider defaults beyond `sync,encoding,time` were not checked.
- The webclaw.io and fastcrw.com cloud engines are assumed to be the open-source engines, based on vendor statements only.
- Ilmenite (pure-Rust static engine) and BeeCrawl (Rust core): first-party claims; source not read.
- Scrapfly, Exa, Parallel, BrowserCat and Cloudflare Browser Rendering: no Rust claim found by search; absence not proven.
- Deeper fundamentals of cortex-scout, stygian, crawlrs, netweir, AgenticCrawler and axon (triaged only).
- crw multi-job parallelism at server level (only the per-job loop was read); DonSeTch's HTTP connection-pool settings in v5core; crawlberg's runtime flavour inside each binding.
- Whether the Crawlee JS arm of `just compare` is on v4 beta, which uses Rust storage.

### Part A3

- crusty's "~10 gbit/s on c5.metal" and "<1 core at 100 mbit" (screenshot only, no method).
- millipede's spider/colly/crawlee figures (raw samples not committed; single macOS machine; I did not rerun).
- lychee proxy support (assumed reqwest env proxies; no explicit option found in `lychee-bin/src/config`).
- SiteOne 2026 commit count (API count not run for it) and its "25 % faster / 30 % less memory than PHP" claim.
- OpenSpider stargazer timeline (API 403 with the available token); its internal load-test numbers beyond what the source comments state.
- scrapix-v2 licence (no licence detected by the API; README says MIT).
- kumo and mycel: README/manual read only, source not read.
- cc-downloader resume behaviour (marked "?" in the table); jev-seo, crawlrs, spyweb, NEXUS-ENGINE feature cells come from keyword grep, not reading.
- Per-URL memory figures for crusty-core, Stract and SiteOne are my estimates from type layouts (url 2.x `Url` ≈ 80 B inline; `String` 24 B; hashbrown 1 control byte at ≤ 7/8 load), not measurements.
- Whether any Kagi/Teclis or Brave Search crawler code is public (none found in the `brave` org's Rust repos; Kagi not checked).

### Part B
- **Star timestamps** for obscura and moli: the token lacks stargazer access (403/401). The anomaly verdicts rest on ratios only.
- **chaser-oxide:** both `ccheshirecat/chaser-oxide` and `0xchasercat/chaser-oxide` return 404 via the API today. The details come from an Exa snapshot (~247★, last push 2026-04-28) and crates.io (0.2.4, 2026-04-20, 19.5 k downloads). Its "13,000+ V8 patches" and "300+ concurrent sessions per node" claims could not be checked.
- **Sources not read:** zendriver-rs, eoka, slicc-cdp, gsd-browser, chrome-devtools-cli, ferrous-browser, browser-rs-mcp, chrome-agent, dravr-browser, oxibrowser, browser_oxide, nokk, browser39, hwatu, plasmate, blitz, servo-fetch and bao were triaged from repo and crate metadata, plus a grep of Cargo.toml for the JS engine (oxibrowser: Boa + Blitz; browser_oxide and browser39: deno_core; nokk: v8 152; plasmate: v8 139; servo-fetch: servo 0.7; hwatu: WebKit). Their features are not checked from source.
- **Benchmark figures** for obscura, moli and Lightpanda (123 MB / 5 s vs Chrome 2 GB / 46 s for 100 pages on an m5.large, per the lightpanda README) are as published; not reproduced. obscura's numbers against Chrome use a cold process per page.
- **Pipe transport:** whether Playwright (behind playwright-rs) launches Chrome with `--remote-debugging-pipe` was not checked from source.
- **Chromey's handler:** its default (non-parallel) handler beyond the transport was left to the other reader.

### Part C
- Per-lookup instruction/syscall cost of `GaiResolver` vs hickory, and per-request allocation counts of reqwest vs bare hyper-util: no published figure; the sources show the work, not its size.
- Any RSS-per-Tokio-worker figure; the allocator/worker link above is reasoning from the documented mechanisms.
- rs-trafilatura's speed figures (CPU, threads, allocator not stated); scrape-rs and html-to-markdown throughput claims (no method).
- dom_smoothie accuracy at 0.18/0.19 (the official table ran 0.14.0).
- Bits/entry for xor8 and binary fuse8 are quoted from the papers' abstracts from memory of their stated results, not re-derived here.
- Why `htmd` has 7.7 M recent downloads with few visible reverse dependencies (largest listed dependents are small); not investigated.
- Which exact mimalloc v3 tag the `libmimalloc-sys` 0.1.49 submodule pins (changelog says v3.3.x; submodule SHA `30b2d9d890` at the crate's HEAD).
- ada-url's FFI overhead per call from Rust and its effect on binary size and static-PIE builds.
