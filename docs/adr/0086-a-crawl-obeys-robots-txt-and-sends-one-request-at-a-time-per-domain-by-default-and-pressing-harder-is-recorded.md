# A crawl obeys robots.txt and sends one request at a time per domain by default, and pressing harder is recorded

ADRs 0016 and 0021 chose parity with Crawlee and Spider. A crawl read robots.txt but did not obey it unless a switch was on, and it kept up to 8 requests in flight per domain with no delay. The research on competitors' APIs shows what that parity costs. Site owners call Scrapy "a malicious denial-of-service tool in its default configuration" ([scrapy#6597](https://github.com/scrapy/scrapy/issues/6597)), and the library defaults they mean are the same 8 per site with no delay. "Please respect robots.txt" is Crawl4AI's top issue ([#107](https://github.com/unclecode/crawl4ai/issues/107)). Scrapy answered with safer values in its generated project template only (2.13.3, whose release notes cite #6597), and those never reach a program that calls the library from a script ([crawl frameworks §6.2](https://github.com/goodfetch/automan/blob/research/consumer-api/rnd/research/research_notes/Consumer%20API%20of%20competitors/crawl-frameworks.md), [synthesis](https://github.com/goodfetch/automan/blob/research/consumer-api/rnd/research/2026-10-14-consumer-api.md) pain 9). ADR 0077's G5 asks that, with nothing set, Goodfetch is never a burden on the sites it reaches, and that anything less safe is an explicit opt-in, visible and recorded. So a crawl now obeys robots.txt and sends one request at a time per domain, one second apart, unless the program says otherwise, and saying otherwise is recorded. Decided in [Crawl politeness defaults](https://github.com/goodfetch/automan/issues/480), on the map [The SDK API of Collie and Foxhound](https://github.com/goodfetch/automan/issues/461).

## Decision

### What the defaults govern

- **Crawl requests**: the requests a crawl's Frontier admits (ADR 0004), start URLs, enqueued links, sitemap entries and retries included. The per-domain limits, the backoff and robots.txt govern them.
- **A program's own navigation is directed by the program**, as a browser's is by its user: a `page.goto` or a fetch it calls, an AI verb acting on its page, an Agent working in its own Session. robots.txt is still read for it and recorded in Provenance, but neither robots.txt nor the per-domain limits gate it. The per-proxy limits and `politeness.sharing` still apply (ADR 0053).
  - RFC 9309 addresses crawlers, which it calls automatic clients. Google's [user-triggered fetchers](https://developers.google.com/crawling/docs/crawlers-fetchers/google-user-triggered-fetchers) generally ignore robots.txt because a person asked for the fetch. Playwright and Stagehand, the leaders for one page, have no politeness at all (S3).
  - One page at a time is not a burden; a program that opens many pages at once on one site wrote that parallelism itself.
  - An Agent that puts work into a Frontier is crawling, and that crawl is governed.

### With nothing set

| | robots.txt | In flight per site | Delay |
|---|---|---|---|
| Goodfetch until now (ADRs 0016, 0021) | read, not obeyed | 8 per domain | none |
| **Goodfetch now** | **obeyed** | **1 per domain** | **1 s** |
| Scrapy 2.19, library | ignored | 8 per hostname | none |
| Scrapy 2.19, project template (since 2.13.3) | obeyed | 1 | 1 s |
| Crawlee 3.18 (unreleased 4.0: the same) | ignored | no per-domain limit | none |
| Crawl4AI 0.9.5 | ignored | 5 sessions | mean 0.1 s |
| Firecrawl | obeyed; turning it off is Enterprise-only | managed by the server | managed by the server |
| Nutch 1.23 | obeyed | 1 per queue | 5 s |
| StormCrawler 3.7 | obeyed | 1 per queue | 1 s |
| Heritrix 3.18 | obeyed | 1 per queue | 5 × the last fetch's time, 3 to 30 s |

Sources, read at those tags: Scrapy's [`default_settings.py`](https://github.com/scrapy/scrapy/blob/2.19.0/scrapy/settings/default_settings.py) and [release notes](https://github.com/scrapy/scrapy/blob/2.19.0/docs/news.rst), Crawlee's [`basic-crawler.ts`](https://github.com/apify/crawlee/blob/v3.18.2/packages/basic-crawler/src/internals/basic-crawler.ts), Crawl4AI's [`async_configs.py`](https://github.com/unclecode/crawl4ai/blob/v0.9.5/crawl4ai/async_configs.py), Firecrawl's [v2 types](https://github.com/firecrawl/firecrawl/blob/f4fdf7f300415180f45272e893918dbfa3649073/apps/api/src/controllers/v2/types.ts), Nutch's [`nutch-default.xml`](https://github.com/apache/nutch/blob/release-1.23/conf/nutch-default.xml), StormCrawler's [`crawler-default.yaml`](https://github.com/apache/stormcrawler/blob/stormcrawler-3.7.0/core/src/main/resources/crawler-default.yaml) and Heritrix's [`DispositionProcessor`](https://github.com/internetarchive/heritrix3/blob/3.18.0/engine/src/main/java/org/archive/crawler/postprocessor/DispositionProcessor.java).

- **robots.txt is obeyed** (`site_preferences.respect_robots_txt`), following [RFC 9309](https://www.rfc-editor.org/rfc/rfc9309):
  - The groups that name the product token `Goodfetch` apply, merged, or else the `*` group, in every Access mode. A `standard` Session never names Goodfetch in its user agent, but a group a site writes for Goodfetch still binds it.
  - A robots.txt answered with a 4xx restricts nothing. One answered with a 5xx, or unreachable, disallows everything: the host's crawl requests wait under the domain's backoff, and those still waiting when their attempts run out are skipped with `robots_txt`.
  - Each host's robots.txt is fetched once and kept per user for at most 24 hours, shared by the user's runs. Fetching it counts in the domain's Politeness.
  - It applies to start URLs too, so a crawl that does nothing says why (G1).
- **Crawl-delay is applied with robots.txt.** It raises the delay of its host's crawl requests up to `site_preferences.max_crawl_delay`, 30 s by default as in Nutch and StormCrawler. A host asking for more has its crawl requests skipped with `crawl_delay`, whose fix names that setting, and `null` waits out any value. RFC 9309 leaves Crawl-delay out and Google ignores it, but Bing, Crawlee's unreleased 4.0, Nutch, StormCrawler and Heritrix (up to 300 s) honour it. Scrapy has never applied it ([scrapy#892](https://github.com/scrapy/scrapy/issues/892)).
- **One request in flight per domain, at least one second between two starts** (`politeness.domain.concurrency` 1, `politeness.domain.delay` 1 s). These are Scrapy's template values, chosen in answer to #6597, and StormCrawler's; Nutch waits 5 s and Heritrix at least 3 s.
  - The delay counts between starts, so a domain gets at most one request a second, and fewer as it slows down: a site that takes 3 s to answer gets one request every 3 s.
  - A browser-tier fetch counts as one request, whatever its page loads.
  - A crawl of many sites stays fast, because ADR 0004 serves domains in turn. One large site is slower: books.toscrape.com's 1,100 pages take about 18 minutes.
- **A failing site slows the crawl.** A 502, a 504, and a connection that is refused or times out back off the domain as ADR 0021's 503 does. A 500 does not, because it usually belongs to one page. A 429 still halves the domain's concurrency.
- **The delay is shared** like the count in flight (ADR 0053): a run admits a request to a domain only once its own delay has passed since the last start any run sharing the key made there. Two runs on one site therefore do not double its rate.

### Pressing harder is explicit and recorded

- **What counts.** A value that lets a crawl press a site harder than its built-in default: `politeness.domain.concurrency` above 1, `politeness.domain.delay` below 1 s, `politeness.backoff.enabled` off or its waits shortened, `politeness.sharing` other than `shared`, or `site_preferences.respect_robots_txt` off.
- **Where it can be set.** Only explicitly: in code, configuration, a Recipe or an Operator policy, never through an environment variable (ADR 0083). It is one setting per value, as Crawlee's `respectRobotsTxtFile` and Scrapy's `ROBOTSTXT_OBEY` are: no confirmation, acknowledgement value or allow list besides (S2, S3).
- **How it is recorded**, as ADR 0028 requires of any weakening:
  - the Audit log records each such value once per run, with the layer that set it;
  - the run's result lists it (ADR 0023);
  - the Provenance of a Record fetched against its host's robots.txt marks it;
  - the run logs it once as a warning through the host's logging (G4);
  - `explainSettings` shows the layer (ADR 0083).
- **Who can stop it.** An Operator policy or a Recipe can cap or lock any of these values (ADR 0028), so a shared Daemon can forbid a run from turning robots.txt off.

### The Access mode and the user agent stay

- **`standard` stays the default Access mode** (ADR 0021): a crawl presents a real browser's identity and no operator contact.
  - The harm in the evidence is load and ignored robots.txt, which the defaults above answer, and a declared bot is blocked more often.
  - A contact in a browser's user agent, or a `From` header that no browser sends, would break `standard`'s consistent identity.
- **A program that wants to be identifiable** uses `identified`. Its Web Bot Auth `Signature-Agent` header points to the key directory the user hosts on a domain they control (ADR 0045).
- **An operator contact is not required**, although Nutch and StormCrawler refuse to fetch without `http.agent.name` and Heritrix refuses to launch a job without an operator contact URL. The research's lesson asked for one ([firecrawl#1169](https://github.com/firecrawl/firecrawl/issues/1169): "Any legit bot crawler should set its own UA"); it is declined for the reasons above, so a later ticket reopens it only with new evidence.

### AI-use signals stay as ADR 0016 set them

Content Signals, IETF aipref, TDMRep and RSL (ADR 0045) say how results may be used, not how hard a site may be pressed. They are still read and recorded with the Purpose in Provenance on every run, and applied only with `site_preferences.respect_use_signals`, off by default. That is ADR 0016's `respect_site_preferences`, renamed now that robots.txt has a setting of its own. A Purpose left at its default `search` is not a declaration to judge them against. The defaults above do not depend on the Purpose: the load a site bears does not depend on what the results are for, and per-Purpose defaults would make a Purpose the way out of politeness.

### Settings

The keys follow ADR 0083's naming rule: two families, so `goodfetch config explain politeness` shows every limit (S2, G6). ADR 0083 leaves other older keys without an area to area 3's spec; the keys of ADRs 0016, 0021 and 0053 below are renamed here, in place, because this ADR changes what they do.

| Key | Built-in | Applies to |
|---|---|---|
| `politeness.domain.concurrency` | 1 | crawl requests |
| `politeness.domain.delay` | 1 s | crawl requests |
| `politeness.backoff.enabled` | on | crawl requests |
| `politeness.backoff.initial` | 2 s | crawl requests |
| `politeness.backoff.max` | 60 s | crawl requests |
| `politeness.ip.concurrency` (was `per_ip_concurrency`) | none | crawl requests |
| `politeness.proxy.concurrency` (was `proxy_concurrency`) | none | every run |
| `politeness.proxy.rate` (was `proxy_rate`), in requests per second | none | every run |
| `politeness.sharing` (was `politeness_sharing`) | `shared` | every run |
| `politeness.degraded_max` (was `politeness_degraded_max`) | 10 minutes | the engine |
| `site_preferences.respect_robots_txt` | on | crawl requests |
| `site_preferences.max_crawl_delay` | 30 s | crawl requests |
| `site_preferences.respect_use_signals` (was `respect_site_preferences`) | off | every run |

A key that applies only to crawl requests, set on a Collie Run, fails with `setting_not_applicable` (ADR 0083). The SDK types leave those keys out of a Collie Run's settings, so a type checker catches the mistake first (G3). Durations take each language's duration type (ADR 0080).

### Skip reasons

A request skipped because of Site preferences ends `skipped` (ADR 0079), with one of three reasons, each with its own fix (G1, G6):

- `robots_txt`: a Disallow line of the host's robots.txt, or a robots.txt that stayed unreachable.
- `crawl_delay`: a Crawl-delay above `site_preferences.max_crawl_delay`.
- `use_signals`: an AI-use signal that refuses the run's Purpose while `site_preferences.respect_use_signals` is on.

## Considered Options

- **Keeping ADRs 0016 and 0021 as they were**, for parity with Crawlee and Spider: rejected by G5. Eight in flight with no delay is Scrapy's library default, the one site owners reported (#6597 and [#6755](https://github.com/scrapy/scrapy/issues/6755)). The cost of that default falls on the tool's name, and a program that needs Crawlee's pace sets two values.
- **Safer values in a project template only** (Scrapy 2.13.3): rejected. A program that calls the SDK never sees a template.
- **Obeying robots.txt and the per-domain limits on every navigation, Collie's included**: rejected. It would break the Playwright-shaped page API (ADR 0019): a Collie program could not open two tabs on one site. RFC 9309 addresses crawlers, and ADR 0083 rules out a default that differs by product, so one setting cannot be on for crawls and off for pages.
- **Defaults per Purpose** (robots.txt obeyed only for `ai-train`, say): rejected above.
- **Two in flight per domain, or a shorter delay** (Crawl4AI's 0.1 s; the research's "1–2 per host"): rejected. With the delay counted between starts, a second request in flight only helps once the site answers more slowly than the delay, which is when it most needs relief.
- **A longer delay** (Nutch's 5 s, Heritrix's 3 s minimum): rejected. One second is the value a leader chose against this very evidence (S3), and it keeps a single site's crawl practical.
- **Latency-adaptive pacing by default** (Scrapy's AutoThrottle, Heritrix's delay factor): rejected as the default and kept opt-in (ADR 0037's CO-09). One request in flight with a fixed gap already slows down with the site, and it is predictable.
- **Waiting out any Crawl-delay, or capping it silently**: rejected. Waiting stalls a crawl for hours on a host that asks for one request a minute. Capping ignores what the host asked, with no record.
- **Backing off on every 5xx**: rejected. A 500 is usually one broken page, not an overloaded server.
- **Requiring an operator contact, or `identified` by default** (Nutch, StormCrawler, Heritrix): rejected above. It reverses ADR 0021's main decision, which this ticket does not reopen.
- **A contact in the `standard` user agent, or a `From` header**: rejected above.
- **Honouring AI-use signals by default**: rejected above. ADR 0016 is reopened for crawl politeness only.
- **An allow list of domains whose robots.txt is ignored**, or a person's approval as for ADR 0052's Wall allowance: rejected. A Wall allowance guards against evading a condition of access; this is pacing and stated preferences, which one recorded setting covers (S2). Scoping a setting to a part of a crawl is [Crawl definition and handler context](https://github.com/goodfetch/automan/issues/477)'s per-Label question.

## Consequences

- **Amends ADR 0016.** robots.txt and its Crawl-delay leave the compliance controls that are off by default: a crawl obeys them unless a recorded setting turns them off. `respect_site_preferences` becomes `site_preferences.respect_use_signals` and covers only the AI-use signals and RSL.
- **Amends ADR 0021.** The defaults are 1 request in flight and a 1 s delay per domain. 502, 504 and refused or timed-out connections back off like a 503. Crawl-delay follows `site_preferences.respect_robots_txt` and `site_preferences.max_crawl_delay`. The settings take the keys above.
- **Amends ADR 0053.** The per-domain state shared by every run gains the time of the last request start, and its settings take the keys above.
- **Amends ADR 0004**, whose pointer to ADR 0021's defaults now reads 1 in flight and a 1 s delay.
- **Amends ADRs 0044, 0045, 0049, 0055 and 0060** only to rename the keys they mention: `per_ip_concurrency`, `respect_site_preferences`, `politeness_sharing` and `politeness_degraded_max`. `TESTING.md` takes `politeness.sharing` too.
- **Areas.** Area 6 (Collection) builds robots.txt matching and caching, Crawl-delay and the skip reasons. Area 3 (Engine runtime) builds the shared delay and the added backoff signals. Area 4 (Security and egress) builds the Audit log entries and the Provenance mark. Area 11 (Client surfaces) builds the settings types and the skip reasons.
- **ADR 0037's matrix.** SC-01, CO-09 and pain 6 cite this ADR, and areas 3, 4, 5, 6 and 11 list it.
- **The V1 benchmark** reports crawl throughput at these defaults and with politeness raised, as ADR 0016 has it publish what each control costs.
- **Golden scenario 05** sets no politeness value. books.toscrape.com's robots.txt answers 404, so nothing is skipped.
- **Left to other tickets.** [Crawl definition and handler context](https://github.com/goodfetch/automan/issues/477) decides whether these settings can be set per Label. [The Run in code](https://github.com/goodfetch/automan/issues/478) places the list of values that pressed harder in the Run's result. [The Agent in Collie's SDK API](https://github.com/goodfetch/automan/issues/474) keeps an Agent's own navigation program-directed and decides any opt-in to obey robots.txt for it. Cloudflare credited ChatGPT-User with stopping when robots.txt disallowed it ([report](https://blog.cloudflare.com/perplexity-is-using-stealth-undeclared-crawlers-to-evade-website-no-crawl-directives/)), while OpenAI's [bots page](https://developers.openai.com/api/docs/bots) now says robots.txt may not apply to user-initiated actions.
- **The `prototype/sdk-api` sketch** holds the `politeness.*` and `site_preferences.*` settings and the skip reasons in each language, with a snippet on golden scenario 05. The pain coverage matrix closes P12 as API.
