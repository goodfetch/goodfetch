# A page is analysed once, a costly feature is a setting that removes its work, and run state commits in groups at a Durability level the crawl chooses

Every page a crawl fetches is parsed once, in one **Page analysis**, whose readers run in a fixed order on one tree and leave one immutable result that every later module reads. A reader runs only when the request or the Run's settings demand it. Work that depends only on a Run's inputs is built when the Run starts. Every costly per-page feature is a setting, and switching it off removes its work as well as its effect. Run state commits each attempt in two transactions, which the single writer groups across pages under one sync, at one of four **Durability levels** (`full`, the default, `fast`, `periodic` and `off`). Structural counts become ceilings that fail a change. This is the spec of the optimization pass ([#1577](https://github.com/goodfetch/goodfetch/issues/1577)), decided with the owner in a grilling session on 2026-10-31.

> Amended by [ADR 0142](0142-a-runs-state-lives-in-memory-its-run-file-is-written-in-batches-and-a-clean-stop-writes-everything.md), proposed and pending the owner's reading: memory holds a Run's state and the run file is written in Run-state batches. Each Durability level becomes a preset of the batch settings: `full`, `fast` and `periodic` keep their meaning, `off` writes nothing during the crawl, and `batch` is the new default.

On 2026-10-30 the owner found Foxhound's comparison with Spider unacceptable. They decided that every comparison runs each tool with the same features and the same settings, in a five-scenario matrix against Spider, Crawlee and Scrapy, with every metric (the comments on #1577). The profile of the code-free 1,000-page crawl at 16 requests at once (tip `0fd15a3a7`, summarized on #1577 on 2026-10-31, its key figures checked against the raw outputs and the code) shows that the gap is structural. Each layer redoes the work another layer already did:

| Setting | Foxhound | Spider 2.53.9 |
|---|---|---|
| Default (`run_state.durability: full`, robots.txt obeyed) | 11.6 s | 0.09 s |
| Like for like (`fast`, robots.txt not obeyed) | 3.0–3.5 s | 0.09 s |

Foxhound runs 32 times Spider's instructions and makes 329 system calls per page against 12. The measured causes, in order:

1. **The cosmetic clean-up runs on every page, even with `formats: {}`.** It compiles about 614 CSS selectors per page, twice: 48.9 % of instructions, and `Matcher::new` alone is 33 %. `measured_formats` calls `clean_up.remove` before it reads the request.
2. **About 8 run-state commits per page, about 1.1 writes each,** each awaited before the page goes on. In `full` that is 7.5 fsyncs per page and 71 % of the wall time. In `fast` the single writer is the serial resource, and only about 1.7 of 16 cores are used. `RunState::read` goes through `write`.
3. **Write amplification:** 142 KB of WAL, 34 KB of savepoint journal spilled to `$TMPDIR` and 8 KB of checkpoints per page, for 462 bytes served. The `run` row's `journal_position`, rewritten on every commit, is 22.8 % of WAL frames.
4. **SQL is prepared again on every call:** 135 times per page, 14.9 %.
5. **Datasets parse their JSON again and build a JSON Schema validator for every page,** inside the writer: about 6 %.
6. **Readability runs for the Page signature** on the untouched document, while formats run it on the cleaned one: 8.6 %.
7. **8 HTML parses per page.** Modules pass HTML as strings: the clean-up serializes its result and the next step parses it again.
8. **RSS is mostly allocator retention:** a live heap of 14.7 MB against an RSS of about 208 MiB, of which 110 MiB is pages mimalloc keeps and 57–67 MiB is the executable's own pages. The workspace builds mimalloc v3, whose purge delay is 1,000 ms (10 ms in v2).

ADR 0002 already promised "many operations grouped per commit by the single writer". The code does not keep that promise, because each page awaits its writes one after another.

The owner asked for « évidemment les meilleures pratiques », taking inspiration from the best crawl frameworks to arrange the architecture. They also asked that the features that slow a crawl, such as persistence after a crash, Politeness and reading robots.txt, « doivent être sous flag (l'user doit pouvoir ne pas les utiliser pour maximum performances) », and that Foxhound offer checkpoint or batch protection against crashes, as competitors do.

How proven crawlers shape this work. Each was checked against its primary source on 2026-10-31.

- **One parse, shared by every reader.**
  - Nutch's `HtmlParser` builds one `DocumentFragment` and hands it to every `HtmlParseFilter` ([HtmlParser](https://github.com/apache/nutch/blob/master/src/plugin/parse-html/src/java/org/apache/nutch/parse/html/HtmlParser.java), [HtmlParseFilter](https://github.com/apache/nutch/blob/master/src/java/org/apache/nutch/parse/HtmlParseFilter.java)).
  - StormCrawler's `JSoupParserBolt` parses once ([2.x](https://github.com/DigitalPebble/storm-crawler/blob/2.10/core/src/main/java/com/digitalpebble/stormcrawler/bolt/JSoupParserBolt.java)).
  - Scrapy caches `response.selector` in `_cached_selector` ([text.py](https://github.com/scrapy/scrapy/blob/master/scrapy/http/response/text.py)).
  - Heritrix passes one `CrawlURI` through its processor chains, but finds links with regular expressions and builds no tree ([ExtractorHTML](https://github.com/internetarchive/heritrix3/blob/master/modules/src/main/java/org/archive/modules/extractor/ExtractorHTML.java)).
  - Crawlee 3.18.2 parses each body twice, once with htmlparser2 and once with `cheerio.load`; only its development branch parses once ([3.18.2](https://github.com/apify/crawlee/blob/96ed398b26954f0e7818cc862f33ff4bb706c84b/packages/cheerio-crawler/src/internals/cheerio-crawler.ts)).
  - Spider reads each body in one streaming lol_html pass with precompiled selectors.
- **The shape of the pipeline.**
  - Heritrix's `ToeThread`, Scrapy, Crawlee and Spider carry a page from start to end in one worker or task.
  - StormCrawler splits the work into Storm bolts that can run on different machines. It serves continuous low-latency crawling, such as Common Crawl's [CC-NEWS](https://commoncrawl.org/news-crawl). Its fetcher's internal queues are unbounded by default (`fetcher.max.queue.size: -1`, [crawler-default.yaml](https://github.com/apache/stormcrawler/blob/main/core/src/main/resources/crawler-default.yaml)).
- **Durability.** No surveyed crawler syncs run state.
  - Scrapy's `JOBDIR` pickles each request when it is queued, writes its queue index and `spider.state` only on a clean close, never syncs, and its documentation warns that an unclean shutdown can corrupt it ([jobs](https://docs.scrapy.org/en/latest/topics/jobs.html), [scheduler.py](https://github.com/scrapy/scrapy/blob/master/scrapy/core/scheduler.py)).
  - Crawlee persists its state every 60 s with a plain `writeFile` and purges its storage on start by default ([Configuration](https://crawlee.dev/js/api/core/class/Configuration)).
  - Heritrix's checkpoints are off by default, and its BDB JE databases are non-transactional and deferred-write ([CheckpointService](https://github.com/internetarchive/heritrix3/blob/master/engine/src/main/java/org/archive/crawler/framework/CheckpointService.java), [BdbModule](https://github.com/internetarchive/heritrix3/blob/master/commons/src/main/java/org/archive/bdb/BdbModule.java)).
  - Databases group syncs instead: in LevelDB's writer queue and RocksDB's write group, the writer at the head of the queue writes everything queued behind it ([db_impl.cc](https://github.com/google/leveldb/blob/main/db/db_impl.cc), [Pipelined Write](https://github.com/facebook/rocksdb/wiki/Pipelined-Write)), and PostgreSQL's leading backend flushes for its group.
- **SQLite and the parsers.**
  - With `temp_store=MEMORY`, statement journals, the ones savepoints use, never touch the disk ([tempfiles](https://www.sqlite.org/tempfiles.html), [global.c](https://github.com/sqlite/sqlite/blob/master/src/global.c)). Automatic checkpoints are `PASSIVE` ([pragma](https://www.sqlite.org/pragma.html#pragma_wal_autocheckpoint)).
  - rusqlite's `prepare_cached` keeps an LRU cache of 16 statements by default ([docs](https://docs.rs/rusqlite/latest/rusqlite/struct.Connection.html)).
  - A `dom_query` `Document` is never `Sync`, and its `atomic` feature makes it `Send` ([docs](https://docs.rs/dom_query/latest/dom_query/struct.Document.html)): one tree has one owner at a time.

## Decision

### The Page analysis

- **A fetched page is parsed once, in one Page analysis.** The body, read once into `Bytes` (ADR 0134), is parsed on the CPU pool (below). Every module that reads a page is a reader of the Page analysis, and the readers run in this order on the one tree:
  1. the readers that see the page untouched, as ADR 0007 requires: links, `rel=canonical`, `meta refresh`, `<base href>`, the program's own selectors and XPath, and `html.raw`;
  2. the cosmetic clean-up, which removes its elements **in place on the same tree**, never on a copy or a re-serialized string;
  3. the readers of the cleaned page: the main content, the Page signature and the formats.

  > Note (agent decision, pending owner review), 2026-11-01, from reading the code at `85083c57b` for [#1577](https://github.com/goodfetch/goodfetch/issues/1577): the tree passes four states, and one step consumes it. It starts untouched, the clean-up removes in place, then composing the shadow roots rewrites it (only when the page holds one), and dom-smoothie takes the `Document` by value and leaves only its article. So the readers of the whole cleaned page (`html.cleaned`, tables, metadata, contacts, images, media, full Markdown) read before the main content, and the tree is copied (`Document::clone`, no parse) only when the demand plan has a reader after a step that consumes it: the main content's fallback that removes landmarks, or `fit`. At most 1 copy per page, a new register row `F1.tree_copies`. A format's string (`html.raw`, `html.cleaned`, `html.main`) is serialized once, only when asked. Markdown reads the tree too: htmd's only entry that does not parse takes an rcdom tree, so the main content's tree is walked into one with no parse, and `scraper`'s readers move to `dom_query`. F1's crate (ADR 0140, open question 1) is `goodfetch-extractor`, which Collie and Foxhound both use and which stays pure: its caller runs it on the CPU pool, and Foxhound passes its own readers of the untouched page in, built once per Run.
- **Its result is immutable and shared.** The Page analysis returns one `Send + Sync` value shared by `Arc`: links, canonical, title, main content's text, Page signature, formats and fields. The Frontier, Datasets, learning and the writer read only that value. The tree ends with the Page analysis, unless a handler on a document tier needs it; no module receives a page's HTML as a string to parse.

  > Amended on 2026-11-01 ([the sweep of ADR 0140](https://github.com/goodfetch/goodfetch/blob/claude/festive-curie-bc4545/rnd/research/2026-11-01-shared-paths-ticket-sweep/proposal.md), D3, decided by the orchestrator under the owner's « fait tout ce que tu peux faire seul », open to their review; #767, #1097, #1473, #1474): **a document tier keeps the tree for its handler's life.** A handler's calls (`content`, `formats`, Locator reads, questions and waits) arrive after the analysis, one task per call, with selectors nobody knew in advance, so they read the attempt's kept tree, with its one owner on the CPU pool, and count in F1: no parse per call, selectors from the engine's cache. The kept tree stays untouched, since a program's selectors see the page untouched (ADR 0007); the clean-up and the main content run once, on a copy, and their result is kept for the attempt. Only a Label that declares a handler keeps one, and `dom_query`'s `atomic` feature, off today, makes it `Send`.

- **Extractors read the tree.** ([the sweep of ADR 0140](https://github.com/goodfetch/goodfetch/blob/claude/festive-curie-bc4545/rnd/research/2026-11-01-shared-paths-ticket-sweep/proposal.md), D4, same authority; #788, #814.) Neither built-in Extractor costs a second parse: dom-smoothie through `Readability::with_document`, rs-trafilatura through a document entry patched into its vendored fork (#1634, which also moves it to this workspace's `dom_query`). An Extractor Plugin crosses a process boundary, so it gets a counted allowance, only when chosen: 1 serialization and 1 parse of its answer (`F1.plugin_serializations`, `F1.plugin_answer_parses`).
- **The browser tier takes the same path,** from the DOM's serialization.

  > Note (agent decision, pending owner review), 2026-11-01: on the browser tier and in Collie, this covers formats, each read from a new `GetOuterHTML` because the DOM moves, so the unit there is one serialization read, and each counts one parse. Links, fields and Block markers stay read in the live page, as a person sees it, and the browser tier learns nothing, as today (`browser_run` never calls the Page signature).

### Work runs only when something asks for it

- **A demand plan, computed before the Page analysis.** A pure function of the request (formats, fields, Datasets) and the Run's settings (`follow`, `frontier.learning`, the tier) gives the set of readers to run. Each reader declares what it produces and needs. A result several readers share, such as the main content, is computed once. A table test holds, for each combination, that no reader outside the plan runs. For example, `formats: {}` with learning off runs neither the clean-up nor readability.
- **No tree when none is needed.** When no reader in the plan needs a tree (no format, no field, learning off), the Page analysis reads links, canonical, refresh and `<base>` in one streaming lol_html pass with precompiled selectors, as ADR 0007 already adopted lol-html for and as Spider does.

  > Note (agent decision, pending owner review), 2026-11-01: the streaming path reads the same links as the tree does, trap-link judgement included, which needs each link's ancestors and the page's `aria-controls`: the pass keeps a stack of open elements and judges at its end. A reader it cannot serve puts the page on the tree, by the demand plan, never with a weaker answer. A test holds that both paths give the same links on the corpus. lol_html parses its selectors once (`Cow::Borrowed`), and builds its program per rewriter inside the library, which the instructions figure measures. A tree reader uses a kept `Matcher`, never `select("…")`, which compiles on each call.
- **The Page signature reads the cleaned page's main content,** the same main content the formats read: one readability run per page, and a signature that a consent prompt or a paywall overlay cannot make the same on every page of a site. This amends ADR 0005's note of #851.

  > Note (agent decision, pending owner review), 2026-11-01: it is the main content of the Run's chosen Extractor with its fallback, as formats read it; today it hard-codes dom-smoothie with no fallback. **ADR 0047's share** measures the main content before the rules, a second readability run on every page they touch, so by default the share becomes an estimate with no second run: the text removed over that text plus the main content left. It is an upper bound of the true share, so it can flag more and never misses one. `extraction.removal_share: exact` restores the measure at the cost of 1 copy and 1 readability run, a counted allowance when chosen.

### Run invariants are built once

- **Nothing that depends only on a Run's inputs is computed per page:**
  - Compiled selectors live in a bounded cache of the engine, keyed by the selector's text and the Clean-up rules' digest (a Run holds to one version, ADR 0047).
  - A crawl's Dataset declarations are parsed, and their validators compiled, when the crawl starts.
  - Settings, the Operator policy and user configuration are resolved once per Run.
  - Provenance's file and the Run index are opened once per Run.
  - Every SQLite connection keeps its prepared statements: `prepare_cached`, with a capacity that holds every statement the connection runs.

    > Note (agent decision, pending owner review), 2026-11-01: this turns on rusqlite's `cache` feature in the workspace and lifts `hashlink`'s ban in `ci/deny/collie.toml`, since Collie's engine runs the same connections. A setting cannot do it: a statement cache is not a capability.

### Every costly per-page feature is a setting

- **Each per-page step is either needed for the result or a feature.**
  - **Needed:** fetching, decoding, reading links when the crawl follows them, the exact duplicate check and the attempt's outcome.
  - **Features:** durability, robots.txt, Politeness, learning and the Page signature, the clean-up, formats, Provenance and Datasets.
- **Each feature is one setting per value** (ADR 0086's rule), and switching it off is recorded in the Audit log and marked on the results it touches, as ADR 0023 already decides for every costly control (and ADR 0028 for a weakened guarantee). When a feature is off, the demand plan schedules none of its readers, admission does not consult it, and nothing of it is written.
- **The spec's deliverables:**
  - the inventory of every per-page step, with its class, the setting that switches it off and its measured cost;
  - the documented **maximum-throughput** combination, which is scenario 1 of the comparison matrix.
- **`frontier.learning`,** true by default, is new. False switches off learned rules, the Page signature and readability; the fixed rules and the exact duplicate check stay. The run report says no rule was learned.
- **Politeness and robots.txt switched off cost nothing.**
  - With `site_preferences.respect_robots_txt: false`, robots.txt is never fetched, read or stored.

    > Amended on 2026-11-01 ([the sweep of ADR 0140](https://github.com/goodfetch/goodfetch/blob/claude/festive-curie-bc4545/rnd/research/2026-11-01-shared-paths-ticket-sweep/proposal.md), D8, same authority as D3; #684, #1649): the Provenance mark `robots_txt: "ignored"` needs robots.txt read, so it becomes its own setting, `site_preferences.mark_robots_txt_ignored`, false by default. With it on, each origin's one copy is read and each page costs one match. The Audit log and the run report still record `respect_robots_txt: false`. `follow: sitemap` reads robots.txt for its `Sitemap` lines because the crawl asked for that, and matches no rule in it.
  - With a delay of 0 and no per-domain limit, admission does not consult the Politeness store.
  - The backoff on 429, 503 and `Retry-After` stays a separate feature with its own setting.
  - An Operator policy can still lock any of these values (ADR 0086).

### Durability levels

- **`run_state.durability` takes four values.** Each is a Durability level:

| Level | When run state commits | After a process crash | After a power cut | SQLite |
|---|---|---|---|---|
| `full` (default) | each attempt's outcome, grouped across pages, synced before anything is told outside | only attempts in flight are lost | only attempts in flight are lost | WAL; the claim commits without a sync of its own, the outcome with one |
| `fast` | as `full`, without a sync | only attempts in flight are lost | recent commits may roll back, without corruption | WAL, `synchronous=NORMAL` |
| `periodic` | as `fast`, with one synced commit every `run_state.commit_interval` (5 s by default) | only attempts in flight are lost | the Run resumes at the last synced commit and redoes at most one interval, without corruption | WAL, `synchronous=NORMAL`, `FULL` for the interval's group |
| `off` | grouped, never synced | the Run starts again from nothing | as after a process crash | WAL, `synchronous=OFF`, checkpoints never synced either |

> Note (agent decision, pending owner review), 2026-11-01: `periodic` was first decided as one long open transaction, which reading `run_state` rules out. It acknowledges a write before its commit, which the writer's contract, its model test and ADR 0122 forbid; a read-only connection misses acknowledged writes, so the idle check could spin; a stale writer's transaction blocks a takeover's `BEGIN IMMEDIATE` past SQLite's busy timeout; and a failed commit would roll back an interval of acknowledged work. A commit that syncs covers every frame before it, so one synced commit every interval gives the same bound with none of these, and a process crash loses less. It writes more WAL than one long transaction; scenario 2 measures it.

- **Two transactions per attempt.** The **claim**, before the fetch, admits the request and takes its Lease. The **outcome**, after it, commits learning, enqueued links, the Lease's commit, the settlement, the charge and the Records in one atomic transaction, as ADR 0103 already wants of a Record ("staged with the attempt and commits once with it"). No page is ever half written.

  > Note (agent decision, pending owner review), 2026-11-01: a transaction here is one job on the writer, whatever group it commits in. The claim is one job that takes the charge's journal, the Lease and the Session draw, whose Label comes from the Lease, from the delivery loop that holds the pool. Three things stay jobs of their own, which their outcome divides by (ADR 0140): a followed redirect's check-and-set, whose answer can skip the page; a handler's `learn`, which must commit before the handler runs (ADR 0005); and a program's enqueue, whose IDs go back to it. A code-free attempt absorbs `learn` in its outcome, in a nested savepoint that keeps today's tolerance of its failure, and an attempt that fails still commits what it learned. A new link is keyed at admission, so `set_keys` leaves. A domain's hold or backoff journal joins the next group.
- **The single writer groups transactions across pages, without a timer.** While one sync runs, the writer takes everything queued and commits it in one transaction with one sync, as LevelDB's writer queue does. Each page keeps its own savepoint, so one page's failing write rolls back alone.
- **What `full` guarantees.** Everything the engine tells the outside has been synced before it is told: a readable Record, an outcome, `run.finish`'s answer and a Checkpoint. A claim lost to a power cut costs one more fetch, never an acknowledgement. In SQLite's WAL, the outcome's sync covers every frame written before it, so a claim committed without its own sync becomes durable with the next outcome.

  > Note (agent decision, pending owner review), 2026-11-01: what is told is what ADR 0002 acknowledges. `run.progress`'s in-flight counts, a handler's request, its enqueue answers and a Provenance line go out earlier, as today, and promise nothing. **A claim's lost charge:** a power cut can lose the journal of a charge for a fetch that happened, and the ledger's reclaim would then under-charge. So after an unclean end, the reclaim counts the journal plus the requests that were in flight at `full`, and the whole Slice at any lower level: it can over-charge a Limit, never under-charge one, and the run report says so.
- **What holds at every level:**
  - An Effect's journal entry commits before the Effect at every level, synced except at `off` (ADRs 0003 and 0010). An Effect never happens twice after a resume.
  - A clean stop (`run.pause`, an abort, a migration) writes a consistent Checkpoint. A paused Run resumes even at `off`.
  - Run state stays in a file at `off`, so ADR 0134's 8 heap bytes per URL in the Frontier still hold.
  - An `off` run file carries an "unclean" mark. Opened after a crash, it is discarded and the Run starts again.
- **The default stays `full`** (ADRs 0002 and 0077's G5). The pass measures `full` against `fast` after grouping; if the gap stays large, the write-up says so, and the owner reopens the default at the owner test.
- **`fast`, `periodic` and `off` weaken a guarantee.** The run report's `weakened` and `config explain` label them (ADR 0028), as `fast` is labelled today.

### Run-file settings

- **`temp_store=MEMORY`.** Savepoint journals never spill to `$TMPDIR`.
- **One savepoint per transaction of a page** (its claim, its outcome), not one per write.
- **`journal_position` is written once per group commit,** and counts group commits. That still matches ADR 0002's note of #218: "the number of commits the run file holds".
- **No checkpoint runs on the writer.** Automatic checkpoints are off on the writer's connection. A checkpointer on its own connection, on a `tasks` thread, runs a `PASSIVE` checkpoint when the WAL passes 16 MB, and a `TRUNCATE` one on a clean stop.
- `page_size` stays 4,096, and the writer's page cache 512 KiB (ADR 0134's note of #1571).

### Reads leave the writer

- **The writer only writes.** It stays the single actor of ADR 0002 and `STYLE.md`.
- **Reads go through a small, bounded group of kept read-only connections** on the owner's blocking threads, as ADRs 0031 and 0103 already read, though today each read opens its own.
- **A read that must see a given write** waits for the writer's commit count to reach that write's commit, then opens its read transaction.
- **The Frontier's hot state is not read back:** its per-domain queues, already bounded at 64, stay in memory.

### The pipeline

- **One task per page carries the page from claim to outcome.** It holds the Lease, the deadline, cancellation (ADR 0038) and the outcome, as Heritrix, Scrapy, Crawlee and Spider do.

  > Note (agent decision, pending owner review), 2026-11-01: the delivery loop issues the claims, which batch naturally; the page's task then carries it from its fetch to its outcome.
- **Two shared, bounded resources** take the staged model's strengths (sizing each step on its own, and batching) without splitting an attempt across queues:
  - **A CPU pool** sized to the cores runs every Page analysis. It is a policy of `tasks` (ADR 0060), not Tokio's blocking pool, which grows to 512 threads.

    > Note (agent decision, pending owner review), 2026-11-01: a method of `TaskOwner` with permits sized to the cores in front of the owner's blocking threads, so at most that many analyses run. `BlockingPolicy` stays a `Copy` value, and `Inline` runs the analysis inline, so the paused benchmark's counts stay exact.
  - **The writer** groups commits (above).
- **A page being analysed holds no network slot.** The slot among the requests in flight is given back once the fetch ends (ADR 0004 counts only requests in flight), and the analysis waits for the CPU pool.
- **Every channel is bounded** (`STYLE.md`).

### Memory

- **Allocation churn is removed first:** the clean-up's compilations, the savepoint journal, Datasets, the 8 parses, `Resolved` clones and progress serialization.
- **mimalloc's purge delay is set to 0 in code**, against v3's 1,000 ms.
- **RSS and CPU are measured again on the shipped static musl binary** (ADR 0024), and the allocator changes only if that measurement justifies it. The allocator was compared with glibc's, which no Linux build ships.
- **The executable's resident pages** (57–67 MiB in the profile, above the whole 32 MiB target) are measured and written up as their own part of any gap to the target.

### Structural counts fail a change

- **Seven counts, read from the in-memory crawl of #1572** (`crawl::in_memory_pages`, on the paused runtime) in steady state, with caches warm:

| Count, per page | Ceiling |
|---|---|
| HTML parses | at most 1; 0 on the streaming path |
| Selector compilations | 0 |
| SQL statement preparations | 0 |
| Run-file transactions (`full`, `fast`) | at most 2 |
| Syncs at `full` | at most 1, at any concurrency |
| File opens | 0 |
| Configuration reads | 1 per Run |

- **The engine exposes each count as an ops listener metric** (ADR 0050), so a user sees the same figures. The test fails past a ceiling, from the pass's pull request on, beside `GATING` set back to `Enforced`.
- **System calls are not counted:** futex and epoll waits depend on scheduling, so the figure is not exact.

### Ceilings, targets and the matrix after the pass

- **Every ADR 0134 ceiling gates,** with the counts above.
  - The pass must bring **bytes allocated per crawled page** (234.45× today) under its 2× ceiling on the in-memory crawl. If it cannot, its pull request proposes a value with its measured cause, and the owner decides before `GATING` changes.
- **"At least Spider's" pages per second is read on scenario 1** of the matrix, the like-for-like scenario. The 32 MiB peak RSS stays read on the default code-free crawl, and is also reported for scenario 1.
- **The matrix's Foxhound cells** (the owner's comments of 2026-10-30 on #1577):
  - **Scenario 1 (raw):** `run_state.durability: off`, `frontier.learning: false`, no format, robots.txt not obeyed, `politeness.domain` delay 0 and concurrency 16.
  - **Scenario 2 (durable):** two Foxhound cells, `full` (its default) and `periodic` at 60 s. The latter is the like-for-like cell against Crawlee's `persistStorage` (60 s, no sync) and Scrapy's `JOBDIR`.
  - Scenarios 3 to 5 are unchanged.

### A `STYLE.md` rule, lints and step 7

- **`STYLE.md` gains, under Memory and hot paths:**
  > **A page is parsed once, in the Page analysis; every module that reads a page is a reader of that analysis, declares what it needs, and reads its immutable result. Work that depends only on a Run's inputs is built when the Run starts, never per page. A costly per-page feature is a setting, and switching it off removes its work.**
  > _Why:_ the M1 profile found 8 parses, 614 selectors compiled and 135 statements prepared per page, each layer redoing what another had done.
- **`disallowed-methods` lints,** each exception under an `#[expect]` that names this ADR:
  - `dom_query::Document::from`, `scraper::Html::parse_*` and `lol_html::rewrite_str` outside the Page analysis;
  - `rusqlite::Connection::prepare` outside the stores' modules;
  - building a JSON Schema validator outside a crawl's start;
  - compiling a selector outside the engine's cache.

  > Note (agent decision of #1840, pending owner review), 2026-11-02: Clippy cannot list `dom_query::Document::from`: the call resolves to the trait's `From::from`, whatever the type (clippy.toml answers "does not refer to a reachable function"). `disallowed-types` lists `dom_query::Document` instead, which catches that call, `Document::fragment` and a `use` of it; a signature names the tree `goodfetch_counted_work::ParsedHtml`. The other two stay in `disallowed-methods`.
- **Step 7's review (`code-review`, Standards) reports:**
  - a module that takes a page's HTML as a string on a page's path;
  - per-page work that depends only on the Run.

## Considered Options

- **Cache each module's own result and keep the modules as they are:** rejected. These are local patches over the model that produced 8 parses, and the next spec's module would parse again.
- **Share the tree itself between asynchronous modules:** rejected. A `dom_query` `Document` is never `Sync`, so sharing it needs a `Mutex`, which `STYLE.md` bars around shared state.
- **One streaming lol_html pass for everything:** rejected. Readability needs a tree (ADR 0007 rejected lol-html for main content), and lol_html matches no sibling combinators. It stays the path when nothing needs a tree.
- **Laziness on first access only, with no demand plan:** rejected. What runs then depends on call order, and one stray call brings a cost back unseen.
- **The Page signature on the untouched page, as today:** rejected. It costs a second readability run per page whenever a format is asked, and an overlay rich in text can sign every page of a site alike and merge distinct pages.
- **A cheaper signature without readability:** rejected. It is weaker evidence, and it would redo #851's thresholds.
- **Grouping commits alone, keeping about 8 awaited writes per page:** rejected. Each page's latency would stay 8 round trips through the writer.
- **Asynchronous durability, with writes acknowledged before their sync and only outside acknowledgements gated on a durability watermark:** rejected for now. It gives the outside the same guarantee as two transactions with grouping, but its code and its Simulation tests (ADR 0060) are much heavier.
- **`periodic` as one long open transaction:** rejected, for the reasons in the note under the table of Durability levels.
- **Weakening `full` to periodic checkpoints,** as Heritrix and Scrapy do: rejected. It breaks ADR 0002's guarantee. `periodic` offers it as a choice instead.
- **`periodic` as the default:** rejected. It would align with competitors at the price of up to 5 s of work lost on a power cut, against ADR 0077's G5.
- **`off` with an in-memory journal or no journal:** rejected. Without WAL, readers cannot read while the writer writes, and without a journal a savepoint cannot roll back.
- **Syncing the claim at `full`:** rejected. It protects nothing the outside is ever told, and it costs one more sync per page at low concurrency.
- **One `performance: max` switch:** rejected. It contradicts ADR 0086's one setting per value, hides what is weakened, and cannot switch off a single feature.
- **A staged pipeline, in the manner of StormCrawler or SEDA:** rejected. Inside one process it splits each attempt, with its Lease and its cancellation, across queues, and adds queues to bound and handoffs to pay for. Scaling past one engine is already ADR 0054's partition by domain, which also keeps a domain's Politeness on one engine.
- **Changing the allocator now:** rejected. The measurement used glibc's allocator, which no Linux build ships, and most of the retention comes from churn this ADR removes.
- **Only ADR 0134's byte and instruction ceilings, without structural counts:** rejected. A second parse costs about 5 %, under a 1.10× ceiling, so it would pass unseen twice.
- **Several ADRs, one each for the analysis, durability and settings:** rejected. The decisions depend on one another: the signature on learning, the claim's sync on the levels and on grouping.

## Consequences

- **The optimization pass ([#1577](https://github.com/goodfetch/goodfetch/issues/1577)) takes this ADR as its one spec,** through `/to-spec`, run as one whole spec (ADR 0136) once [#1583](https://github.com/goodfetch/goodfetch/pull/1583) has merged. Its pull request records before-and-after figures for every metric of the owner's matrix and sets the ceilings (ADR 0134). Fixes 1–6 of the profile were estimated to bring instructions to 30–35 % of today; no figure is promised here, and a missed target is written up as ADR 0134 says.
- **Amends ADR 0002:** the four Durability levels, two transactions per attempt, commits grouped across pages, `journal_position` per group commit and checkpoints off the writer.
- **Amends ADR 0005** (the note of #851): the Page signature reads the cleaned page's main content, and `frontier.learning` switches learning off.
- **Amends ADR 0007:** one Page analysis per page, the clean-up in place on the same tree, and the streaming path when nothing needs a tree.
- **Amends ADR 0134:** the structural counts as ceilings, the scenario the Spider target is read on, the matrix's Foxhound cells and the rule for the 2× ceiling.
- **Notes on ADRs 0023** (its table gains the Durability levels, `frontier.learning` and the Page signature), **0028** (`periodic`, `off` and `frontier.learning: false` join `weakened_by`), **0060** (a CPU pool policy in `tasks`), **0086** (switched off means no work; `site_preferences.mark_robots_txt_ignored`) **and 0103** (at `periodic`, a Record a reader saw can be lost to a power cut, which `weakened` labels).
- **`STYLE.md`** gains the rule above. **`TESTING.md`** gains the structural counts beside the absolute ceilings.
- **`GLOSSARY.md` gains two terms,** added with this ADR:
  - **Page analysis:** the one step in which a fetched page is parsed and read by every reader its crawl needs, leaving one result that every later step shares. _Avoid_: parse step, pipeline stage, page processing.
  - **Durability level:** what a run's state survives: a process crash, a power cut, both or neither, chosen per run as `full`, `fast`, `periodic` or `off`. _Avoid_: persistence mode, sync mode, fsync mode.
- **Contracts change in place.** The new values of `run_state.durability`, and the settings `run_state.commit_interval`, `frontier.learning`, `extraction.removal_share` and `site_preferences.mark_robots_txt_ignored`, are added to `goodfetch-protocol`'s types and regenerated with `xtask codegen`. Nothing is published, so no format changes version (ADR 0066).
- **Out of scope: continuous crawling.** A Run that never ends, per-URL revisit dates, a revisit policy and feed discovery, which StormCrawler serves, are decided in ADR 0139, from their own grilling session. This ADR's one task per page with shared pools does not block them.
