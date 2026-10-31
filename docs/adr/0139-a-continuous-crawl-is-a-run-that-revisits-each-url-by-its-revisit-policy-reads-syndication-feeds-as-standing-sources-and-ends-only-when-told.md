# A continuous crawl is a Run that revisits each URL by its Revisit policy, reads syndication feeds as standing sources, and ends only when told

Foxhound gains a **Continuous crawl**: a crawl Run, under the setting `crawl.continuous`, that does not end when its Frontier empties. Each URL it visited carries the date it is due again, set by its Label's **Revisit policy**, and the Run waits idle until the next one is due. RSS and Atom feeds become a seed kind, `syndication(url, rules)`, read like sitemap seeds in any crawl, and a Continuous crawl reads its feeds and sitemaps again as standing sources of new URLs. One mechanism serves news and archive crawling, the profile of Common Crawl's CC-NEWS, and the monitoring of known pages. Decided with the owner in the continuous-crawl grilling of 2026-10-31 (notes and sources in `.claude/reports/grilling-continuous-crawl/`), for a milestone after M1; ADR 0138's out-of-scope section points here.

Foxhound already monitors in batches: Schedules with Watches (ADR 0036), the Response cache's `revalidate` mode (ADR 0046), Baseline change tracking with "only new or changed" Sinks (ADR 0026), live Record delivery (ADR 0103) and the Partitioned crawl (ADR 0054). What it lacked is low latency (minutes between a publication and its collection) and a memory per URL from one visit to the next.

How others do it, checked against their sources (`sources.md`, `sources-round3.md`):
- **StormCrawler** keeps a `nextFetchDate` per URL in its status index, and its topology "runs forever, or until you kill it". Its `DefaultScheduler` waits 1,440 minutes after a fetch, 120 after a fetch error and never after an error. Its `AdaptiveScheduler` takes a changed signature as a change and a 304 as none, between 60 minutes and 14 days. Intervals are set per metadata (`fetchInterval.FETCHED.isFeed=true`), and its `AggregationSpout` reads `nextFetchDate <= now` grouped into buckets by key. **URLFrontier** keeps queues by key with `refetchable_from_date`, 0 meaning never.
- **CC-NEWS** (`commoncrawl/news-crawl`, on StormCrawler) runs without end. It fetches each article once and never again, reads feeds every 12 hours and news sitemaps every 24 hours at first, then adapts both between 90 minutes and 90 days. It drops entries older than 30 days and rotates its WARC files at 1,024 MB or 24 hours.
- **Nutch** crawls in rounds (`bin/crawl <dir> <rounds>`), not without end. Its `AdaptiveFetchSchedule` raises the interval by 40 % on no change and lowers it by 20 % on a change, with a 30-day default interval and a 90-day maximum.
- **Heritrix 3** stays alive on an empty frontier only with `runWhileEmpty` (false by default) and has only a fixed `rescheduleDelaySeconds`. Its adaptive revisit was Heritrix 1's.
- **Crawlee** has `keepAlive` but no revisit, and fetching a URL again needs a new `uniqueKey`. **Scrapy** closes an idle spider unless `spider_idle` raises `DontCloseSpider`. Neither reads news sitemaps or RSS feeds as sources.
- **Feed readers** poll by entry rate. Miniflux's `entry_frequency` takes one week divided by the entries per week, clamped between 5 minutes and 24 hours, with `<ttl>`, `Cache-Control`, `Expires` and `Retry-After` as floors. FreshRSS defaults to 1 hour with a 15 to 20 minute floor. FreshRSS, NewsBlur, Inoreader and Feedbin subscribe through WebSub.
- **Google and Bing** ignore `<changefreq>` and `<priority>` and use `<lastmod>` only when it is "consistently and verifiably" accurate. A Google News sitemap holds only the last two days, at most 1,000 entries.
- **Cho and Garcia-Molina** proved that revisiting every page at one rate always gives fresher results than revisiting each in proportion to how often it changes (SIGMOD 2000). Their estimator of a page's change rate (ACM TOIT 2003) is the one Google's papers cite. No open-source crawler checked implements one.

## Decision

### A Run mode, not a Schedule feature

- **`crawl.continuous`**, `false` by default, makes a crawl's Run a Continuous crawl. It implies `crawl.keep_alive` (RT-12) and is fixed for the Run's life, so a resume cannot change it, as `crawl.scope.*`.
- **It stays `running`.** While nothing is due, its progress says `idle` and gives `next_due_at`. It ends only by `finish()`, `stop()`, an abort, a Limit or `timeout.run`, which has no default (ADR 0080) and counts idle time as keep-alive does. `finish()` schedules no more revisits, finishes the first visits still waiting, and ends the Run `finished`.
- **A Schedule cannot target a Continuous crawl:** `schedules.create` and `schedules.apply` refuse one with the Error code `continuous_not_schedulable`, which is not retryable. Schedules stay the tool for batch collection, and a Continuous crawl does a Watch's work for each URL itself.
- **Who keeps it running.**
  - A code-free catalog crawl runs on the Daemon, which resumes it after its own restart and never stops idle while it holds a live Continuous crawl.
  - A program's Continuous crawl lives as long as its program. A program restarted by systemd, a container or any supervisor resumes the same Run by its Run ID (ADR 0102), and the documentation gives example units.

### Revisits

- **A revisit re-admits the visited identity** as its existing crawl request, as `retryFailed` does: the same ID and a new attempt whose outcome replaces the previous one. A revisit is never an enqueue, which would be a `duplicate`.
- **Revisit state lives on disk,** in a `revisits` table of the run file written only by a Continuous crawl. Each row holds the identity, the domain key, `next_visit_at`, the current interval, `last_fetched_at`, `last_changed_at`, the Page signature, the `ETag` and `Last-Modified` validators, and the counts of visits and changes seen. The run keeps its own validators, independent of the per-user Response cache and its 30-day ceiling. An index on `(domain_key, next_visit_at)` serves the due URLs: each domain's queue (ADR 0004) is filled with a bounded window of due rows, read through the index as StormCrawler's spout reads its status index. No per-URL value is held in memory, so ADR 0134's 8 heap bytes per URL hold. A batch crawl writes no row and maintains no index.
- **The Revisit policy is a Label setting,** `revisit.policy`, beside `fetch.kept_responses.*` (ADR 0110). It takes one of five forms:
  - `never`, for pages read once, such as articles;
  - `every <duration>`, a fixed interval;
  - `adaptive { initial, min, max, rate_changed, rate_unchanged }`, Nutch's and StormCrawler's multiplicative schedule: the interval shrinks by `rate_changed` (0.5) after a change and grows by `rate_unchanged` (0.5) after none, within `min` and `max`. It is the default for pages: initial 1 day, min 1 hour, max 30 days;
  - `estimated { initial, min, max, target_freshness }`. It estimates each URL's change rate from its visits and changes with Cho and Garcia-Molina's estimator, r̂ = −log((n − X + 0.5)/(n + 0.5)), and starts a new URL from a prior of two intervals of `initial`, as Google's papers do. It schedules the URL so that its expected freshness reaches `target_freshness` (0.9), never in proportion to its change rate. A URL changing faster than `min` allows gets `min`, and the Run report lists it;
  - `entry_rate { min, max }`, the feed readers' schedule and the default for syndication sources: one week divided by the source's new entries per week. It is clamped between 15 minutes and 24 hours when the source returns validators, and between 1 hour and 24 hours when it does not.
- **A change is checked by `revisit.change_check`,** a Label setting:
  - `conditional+signature`, the default: the revisit sends `If-None-Match` and `If-Modified-Since`. A 304 means unchanged; otherwise the Page signature decides;
  - `conditional`: every 200 counts as changed;
  - `signature`: no validators are sent;
  - `none`: every revisit counts as changed.

  For a syndication source or a sitemap, a change is a new entry, or an entry updated since it was read.
- **What an unchanged revisit does** is `revisit.on_unchanged`. Under `skip`, the default, it runs no handler, pushes no Record and ends `unchanged`. Under `handler`, the handler runs anyway.
- **Errors.** A revisit whose fetch fails retryably is due again after `revisit.on_error` (2 hours, StormCrawler's `fetchInterval.fetch.error`). A URL that answers 404 or 410 on `revisit.gone_after` revisits in a row (1 by default) ends `gone` and is not revisited.
- **Site hints** are switched one by one under `revisit.site_hints.*`:
  - `Cache-Control: max-age`, `Expires`, `Retry-After`, RSS `<ttl>` and `sy:updatePeriod` are floors: a revisit never comes sooner than the site asks. They are on by default.
  - A sitemap's `<lastmod>` and `news:publication_date`, and an entry's update date in a feed, bring a visited URL's revisit forward to now when they are later than `last_fetched_at`. They are on by default. A host's `<lastmod>` is trusted only once its changes have matched Page signature changes, learned on evidence as ADR 0120's rules are, and the Run report lists the hosts whose `<lastmod>` it trusts.
  - `<changefreq>` and `<priority>` are off by default, since Google and Bing ignore them. On, `<changefreq>` gives the initial interval.
- **The schedule is a pure function** of the policy, the previous state, whether the URL changed and the site's hints, so it is tested alone.

### Sources of new URLs

- **`syndication(url, rules)`** is a seed kind beside `sitemap(url, rules)` (ADR 0093). It reads RSS 2.0, RSS 1.0 and Atom 1.0, whose types ADR 0119's content types for it name. Like a sitemap seed, it is a work item that runs no handler, and it passes Politeness and robots.txt as a page does. Its entries' links are given the Label of the first matching rule and checked as an enqueue is, at depth 0 and with no Payload. In a batch crawl, a source is read once.
- **Sitemaps** also read `<lastmod>` and the `news:` extension in any crawl, under `crawl.sitemap.read_lastmod` (`true`).
- **In a Continuous crawl, sources are read again** under their own Revisit policy:
  - syndication sources: `entry_rate`;
  - news sitemaps: `adaptive`, initial 1 hour, between 15 minutes and 24 hours;
  - other sitemaps: `adaptive`, initial 24 hours, between 1 hour and 30 days.

  An entry dated more than `crawl.continuous.max_entry_age` ago (30 days, CC-NEWS's rule) is not added.
- **Discovery.** Under `crawl.syndication.discover`, a page in scope that links an RSS or Atom feed with `<link rel="alternate" type="application/rss+xml">` or `type="application/atom+xml"` adds it as a `syndication` seed. The setting is on by default in a Continuous crawl and off elsewhere. robots.txt `Sitemap:` lines are read as they already are.
- **WebSub.** `crawl.syndication.websub` is `off` (the default), `auto` or `on`. When on, a source that names a hub (`Link: rel="hub"` in its headers before its document, then `<link rel="hub">`) is subscribed through ADR 0027's HTTP listener: the subscription is verified by its `hub.challenge`, carries one secret per subscription whose `X-Hub-Signature` is checked on every delivery, and its lease is renewed before it expires. `auto` subscribes only when the listener's public URL is configured and a hub's verification reaches it, as FreshRSS does. A hub on a private address is refused, as Feedbin does. A push only brings the source's next read forward to now, since some hubs push a stub (YouTube). Polling goes on while pushes arrive, its interval multiplied by `crawl.syndication.websub.poll_factor` (12, NewsBlur's factor).

### Order within a domain and Politeness

- **Each turn of a domain** (ADR 0004) picks by `crawl.revisit.order`. The default is `sources,first_visits,revisits`: due source reads first, which is where latency is won, then first visits, then due page revisits. Within each kind, ADR 0004's priority and `forefront` order holds.
- **Revisits take at most `crawl.revisit.domain_share`** (0.5) of a domain's starts while first visits wait. A domain whose Politeness cannot keep up falls behind, and the Run report says so. The site is never pressed harder than ADR 0086 allows.
- **Politeness and robots.txt are unchanged.** robots.txt is already fetched again after 24 hours per user (ADR 0086).
- **Evidence.** Only first visits are evidence for Learned rules and trap paths (ADR 0005), so a run of unchanged revisits never closes a path as `trap_path`.

### Records, Sinks and change

- **A changed revisit runs its handler, and its Records go to their Datasets** as any Record does. Datasets stay append-only (ADR 0103).
- **"Only new or changed" in a Continuous crawl** compares each Record with the latest Record of the same Dataset key in the same Run, not with a Baseline. The `history` and `upsert` write modes (ADR 0026) apply as they are. A Dataset with no key delivers every Record of a changed revisit.
- **A `gone` URL soft-deletes the Records it produced:** the proof is per URL, so no complete Run is needed (ADR 0036).
- **Sinks are probed again** every `sinks.reprobe_interval` (24 hours) in a Continuous crawl, not only at its start. A Files Sink rolls its parts at `sink.files.rotate_size` (1 GiB) or `sink.files.rotate_after` (24 hours), as CC-NEWS rotates its WARC files.

### Report, Completeness and Limits

- **`report()` can be read while the Run goes on.** It is the same Run report, read early: Lifecycle `running`, no Verdict and no Completeness yet, Usage, Learned rules, Sinks, warnings and a **Freshness** block. The block gives the revisits overdue, the median and the largest lag, the counts of `unchanged`, `gone` and changed revisits, and the trusted `<lastmod>` hosts, overall and per domain. It is one channel, not a second, so ADR 0102's refusal of a periodic status line stands.
- **Completeness keeps its meaning of coverage.** At the end, a Continuous crawl is `complete` when every URL it discovered was visited at least once, with ADR 0005's `unverified` causes unchanged. Late revisits are Freshness, never Completeness.
- **A Run's Limits may carry a period** (day, week or month) in a Continuous crawl, as other scopes' Limits do (ADR 0041), and so may its Model allowance (ADR 0107). Under `crawl.continuous.idle_when_limit_spent` (`true`), a spent period makes the Run `idle` until the next period. When it is `false`, the Run stops `stopped` as today.

### Growth and retention

- **Records** leave the run file once every Sink has acknowledged them and `crawl.continuous.retention` (7 days) has passed. A Record no Sink acknowledged is never removed, so a Continuous crawl with no Sink keeps every Record, and its report warns about the run file's growth. Exports read what the run file holds.
- **Outcomes** keep the latest per URL and counters; their history leaves with the retention. Learned rules keep the last `crawl.continuous.merged_kept` (1,000) merged URLs each, beside a total count.
- **Visited identities** are kept for the Run's life by default. Under `crawl.continuous.forget_after`, an identity not seen for that long is forgotten, and a source that announces its URL again then adds it as new. The Run trace keeps its 1 GB cap.
- **Clean-up rules.** Under `clean_up.follow_latest`, on by default in a Continuous crawl and off elsewhere, the Run adopts a newly published Clean-up rules version at its next Safe point. It records the version and date in its report, and change tracking says when a Record's comparison crosses that switch, as ADR 0036 already does across a Baseline.

### Presets

- **`crawl.continuous.preset`** is `news`, `monitoring`, `archive` or `custom` (the default). It is a named overlay of default values on ADR 0028's layers: every key it sets can be overridden, and `config explain` names the preset as a value's origin. No crawler checked ships named presets; their norm is templates (Scrapy, Crawlee, StormCrawler's archetypes) and overlays (Heritrix's sheets, StormCrawler's layered YAML). Each preset therefore also comes with a template Recipe.
  - **`news`**, CC-NEWS's profile: page Labels `never`, syndication sources `entry_rate`, news sitemaps re-read as above, `crawl.continuous.max_entry_age` 30 days, `crawl.syndication.discover` on.
  - **`monitoring`**: page Labels `adaptive`, from 1 hour to 30 days, with `revisit.on_unchanged: skip` and `crawl.syndication.discover` off.
  - **`archive`**: page Labels `adaptive`, from 1 day to 90 days (Nutch's maximum), and `crawl.continuous.retention` 30 days.

### A Partitioned Continuous crawl

- **Each Partition is a Continuous crawl,** and the crawl's Lifecycle is `running` while any Partition runs (ADR 0054).
- **A domain handed to another Partition** carries its `revisits` rows beside its pending URLs and the fingerprints of URLs already seen, so the new owner keeps the schedule instead of revisiting everything.

## Considered Options

- **Close Schedules with a Watch** (every 5 minutes, say): rejected. Each Run starts from an empty Frontier, so it fetches every article again or filters them afterwards against a Baseline. Latency never drops below the period, under the default `skip` overlap. Every Run pays its start, ADR 0036's 10-run retention cannot hold 288 Runs a day, and no URL is remembered from one Run to the next.
- **A standing object above Runs that keeps a durable Frontier and starts bounded Runs on due URLs** (Nutch's crawldb and segments): rejected. The Frontier would leave run state, against ADR 0002 and the glossary, with two stores, a handoff between them and one more writer. Nutch's rounds are batches and do not reach low latency.
- **Numbered cycles inside the Run, each with its own report** (Nutch's segments): rejected. The cut is arbitrary and rebuilds batch Runs inside one Run. `report()` read early gives the same insight with one channel.
- **Revisit dates in an in-memory priority heap:** rejected. The date alone is 8 bytes, which spends ADR 0134's whole ceiling, while a disk index costs about 100 bytes of disk per URL.
- **Revisits in proportion to change rate:** rejected. Cho and Garcia-Molina proved uniform revisiting fresher; `estimated` targets freshness instead.
- **Trusting `<changefreq>`, `<priority>` and every `<lastmod>`:** rejected, as Google and Bing reject them. `<lastmod>` is trusted per host on evidence.
- **A 5-minute floor for feeds:** rejected as a default. FreshRSS floors at 15 to 20 minutes, and Rachel Kroll's guidance for feed readers is at most hourly with conditional requests and daily without. A user may still set `min` lower.
- **Deduplicating Records by key inside the Dataset:** rejected, as ADR 0103 rejected it. Change is decided before the handler runs, and by key in the Sink.
- **WebSub out of scope:** rejected. It is the only standard way to seconds of latency, and Google's hub, WordPress's plugin (100,000+ installs) and four major readers use it. It is `off` by default because a local Daemon is usually not reachable.
- **Keeping Clean-up rules pinned for a Continuous crawl's whole life** and turning ADR 0047's 14-day warning into a "Needs you" item: rejected as the default. A six-month news crawl would run on stale rules. It remains a setting: `clean_up.follow_latest: false`.

## Consequences

- **Amends ADR 0102.** A Continuous crawl does not finish when its Frontier is empty. `report()` can be read while it runs. "Recurring Runs are ADR 0036's Schedules" holds for batch Runs only. A revisit is a re-admission as `retryFailed` is.
- **Amends ADR 0036.** A Schedule refuses a Continuous crawl as its target with `continuous_not_schedulable`. A Daemon holding a live Continuous crawl never stops idle.
- **Amends ADRs 0026 and 0036.** In a Continuous crawl, "only new or changed" compares by key within the Run, a `gone` URL soft-deletes its Records, Sinks are probed again periodically, and Files Sinks roll by size and age.
- **Amends ADRs 0103 and 0122.** A Continuous crawl removes acknowledged Records past its retention, outcomes past the latest per URL and merged URLs past `crawl.continuous.merged_kept`.
- **Amends ADR 0118.** A 304 answering a revisit's conditional request is `unchanged`, not `response_not_ok`.
- **Amends ADRs 0041 and 0107.** A Run's Limit and Model allowance may carry a period in a Continuous crawl.
- **Amends ADR 0047.** A Continuous crawl may adopt a new Clean-up rules version under `clean_up.follow_latest`.
- **Amends ADR 0005.** Only first visits are evidence for Learned rules and trap paths.
- **Amends ADR 0004.** A domain's turn orders by `crawl.revisit.order` and caps revisits at `crawl.revisit.domain_share`.
- **Amends ADR 0054.** A domain's handoff carries its revisit state.
- **Amends ADRs 0093 and 0119.** The `syndication(url, rules)` seed kind and the content types it reads. Sitemaps read `<lastmod>` and `news:`.
- **Amends ADR 0134.** The Frontier-at-scale check gains a Continuous scenario: 100,000 URLs, half of them due, then 1,000,000, under the same 8-byte and 1.5× ceilings. The Daemon soak measures its leak slope per revisit cycle as well as per crawl.
- **Verification.** End-to-end tests run Continuous crawls under a simulated clock the engine exposes to tests, as `tokio::time::pause` does, so a month of revisits takes seconds. The spec confirms the clock seam the tests already have. An unchanged revisit costs one conditional request and one grouped write (ADR 0138's group commit), with no parse.
- **Matrix (ADR 0037).**
  - RT-12 covers Continuous crawls.
  - New rows: Continuous crawl, Revisit policy, the `syndication` seed kind and WebSub.
  - CO-10 gains `<lastmod>` and `news:`.
  - CO-31 notes that a Continuous crawl keeps its own validators.
- **Glossary.**
  - New terms: Continuous crawl, Revisit, Revisit policy, Syndication feed, Freshness and Preset.
  - "recrawl" and "refetch" are _Avoid_ terms for Revisit.
  - Run, Run report and Frontier say what a Continuous crawl changes in them.
- **Not decided here:** the milestone (by `/to-spec`, after M1), the `<lastmod>` trust thresholds, and the spec's Error codes beyond `continuous_not_schedulable`.
