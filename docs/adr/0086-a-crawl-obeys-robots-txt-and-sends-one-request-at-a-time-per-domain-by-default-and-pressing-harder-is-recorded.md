# A crawl obeys robots.txt and sends one request at a time per domain by default, and pressing harder is recorded

ADRs 0016 and 0021 chose parity with Crawlee and Spider. A crawl read robots.txt but did not obey it unless a switch was on, and it kept up to 8 requests in flight per domain with no delay. The research on competitors' APIs shows what that parity costs. Site owners call Scrapy "a malicious denial-of-service tool in its default configuration" ([scrapy#6597](https://github.com/scrapy/scrapy/issues/6597)), and the library defaults they mean are the same 8 per site with no delay. "Please respect robots.txt" is Crawl4AI's top issue ([#107](https://github.com/unclecode/crawl4ai/issues/107)). Scrapy answered with safer values in its generated project template only (2.13.3, whose release notes cite #6597), and those never reach a program that calls the library from a script ([crawl frameworks §6.2](https://github.com/goodfetch/goodfetch/blob/research/consumer-api/rnd/research/research_notes/Consumer%20API%20of%20competitors/crawl-frameworks.md), [synthesis](https://github.com/goodfetch/goodfetch/blob/research/consumer-api/rnd/research/2026-10-14-consumer-api.md) pain 9). ADR 0077's G5 asks that, with nothing set, Goodfetch is never a burden on the sites it reaches, and that anything less safe is an explicit opt-in, visible and recorded. So a crawl now obeys robots.txt and sends one request at a time per domain, one second apart, unless the program says otherwise, and saying otherwise is recorded. Decided in [Crawl politeness defaults](https://github.com/goodfetch/goodfetch/issues/480), on the map [The SDK API of Collie and Foxhound](https://github.com/goodfetch/goodfetch/issues/461).

## Decision

### What the defaults govern

- **Crawl requests**: the requests a crawl's Frontier admits (ADR 0004), start URLs, enqueued links, sitemap entries and retries included, and every navigation or fetch a crawl handler makes from the page or context it was given (a `goto`, a click that navigates, a fetch, an AI verb acting on that page). The crawl, not a person, decides how many handlers run at once, so their navigation is the crawl's. The per-domain limits, the backoff and robots.txt govern them. How a navigation that robots.txt refuses surfaces inside a handler is [Crawl definition and handler context](https://github.com/goodfetch/goodfetch/issues/477)'s. The seam through which Foxhound's Politeness admits navigation on Collie's page API is [How the products are built](https://github.com/goodfetch/goodfetch/issues/454)'s.
  > Amended by [ADR 0087](0087-each-product-is-an-executable-crate-that-links-only-its-crates-and-has-its-own-size-budget.md): the seam is a navigation gate on Collie's page. The engine consults it at its request interception for each main-frame navigation, whatever starts it, and each fetch made through the page's `request`, holding the permit until the response arrives. Foxhound's handler context sets it, backed by the per-domain limits and delay, the backoff and robots.txt, on the page it hands over and on every page opened from that page, or in its Session by the handler, while the handler runs; a page a program opens has none.
  > Amended by [ADR 0121](0121-service-workers-run-under-the-engines-interception-the-back-forward-cache-is-off-locators-reach-closed-shadow-roots-and-a-read-never-scrolls-a-list.md): the gate also takes a navigation's permit for a service worker's request that carries that navigation to the network, and none for a navigation a service worker answers from its own cache, which sends nothing to the site.
  > Amended by [ADR 0093](0093-a-crawl-declares-its-labels-and-one-handler-each-and-a-handler-reads-collies-page-on-every-fetch-tier.md): in the glossary's terms, a Crawl request is what the Frontier admits; a handler's own navigation and fetches are governed as crawl requests are. Such a navigation that robots.txt, a Crawl-delay or, while `site_preferences.respect_use_signals` is on, an AI-use signal refuses fails with `site_preferences_refused` and its reason, before it is sent or, for a signal the response carries, when it arrives, and if that escapes the handler, the request ends `skipped` with the same reason.
- **In flight** means from a request's start until its response has arrived, a browser-tier page until its load event, not while its handler runs.
  > Amended by [ADR 0105](0105-a-programs-tests-replay-a-response-recording-in-a-test-engine-that-reaches-no-site.md): a request a Response recording answers reaches no site, so it takes no slot, delay or backoff.
- **A program's own navigation is outside Politeness**: outside a crawl handler, a `page.goto` or a fetch the program calls, an AI verb acting on its page, an Agent working in its own Session. That is Collie's work, directed by the program as a browser's is by its user, and ADR 0084 places Politeness in Foxhound.
  - robots.txt is still read for it and recorded in Provenance. Nothing of Politeness gates it, counts it or learns from it.
  - Whether Collie carries ADR 0053's per-proxy limits is [Collie's exact capability set](https://github.com/goodfetch/goodfetch/issues/455)'s. Whether a Collie page's 429 should slow a Foxhound crawl on the same machine is [Collie and Foxhound installed on one machine](https://github.com/goodfetch/goodfetch/issues/486)'s (ADR 0098).
    > Amended by [ADR 0088](0088-collie-carries-every-capability-except-those-only-many-pages-need.md): Collie carries them, with the ceilings learned per proxy and per cloud-browser provider. They protect the user's own accounts with the vendors rather than the sites, so they hold a program's own navigation through that proxy or provider.
  - RFC 9309 addresses crawlers, which it calls automatic clients. Google's [user-triggered fetchers](https://developers.google.com/crawling/docs/crawlers-fetchers/google-user-triggered-fetchers) generally ignore robots.txt because a person asked for the fetch. Playwright and Stagehand, the leaders for one page, have no politeness at all (S3).
  - One page at a time is not a burden; a program that opens many pages at once on one site wrote that parallelism itself.
  - An Agent that puts work into a Frontier is crawling, and that crawl is governed.

### With nothing set

| | robots.txt | In flight per site | Delay |
|---|---|---|---|
| Foxhound until now (ADRs 0016, 0021) | read, not obeyed | 8 per domain | none |
| **Foxhound now** | **obeyed** | **1 per domain** | **1 s** |
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
  - The groups that name the product token `Foxhound`, the Product that crawls (ADR 0084), apply, and so do those that name `site_preferences.robots_txt_user_agent` when it is set. They are merged into one group, as RFC 9309 merges the groups that match one token. Only when neither token has a group does the `*` group apply. This holds in every Access mode. RFC 9309 §2.2.1 says a token should appear in the user agent the crawler sends; a `standard` Session sends neither, and matching them anyway only adds the rules a site wrote for Foxhound or for the operator's name. The token names the crawler, as `Googlebot` does, never the organization: Goodfetch is not the name of anything installed.
    > Amended by [ADR 0090](0090-products-are-named-goodfetch-collie-and-goodfetch-foxhound-and-share-one-product-version.md): the product token is `goodfetch-foxhound`, qualified as every Product name is in a namespace open to everyone, since a Go scraping framework is also named Foxhound. It still names the crawler, and the rejected option below reads "Matching only `goodfetch-foxhound`".
  - `site_preferences.robots_txt_user_agent` is the operator's own product token (letters, underscores and hyphens), as Crawlee's `respectRobotsTxtFile: { userAgent }`, Scrapy's `ROBOTSTXT_USER_AGENT` and Firecrawl's `robotsUserAgent` let users set one (S3). It belongs in the user agent the crawl sends, which `identified` or a Hook sets (ADR 0033). It can select a group more generous than `*`, so setting it is recorded like a value that presses harder.
  - A robots.txt answered with a 4xx other than 429 restricts nothing, as Google reads RFC 9309 §2.3.1.3.
  - A robots.txt answered with a 429 or a 5xx, or unreachable, is never cached as "no rules". A copy fetched earlier keeps applying meanwhile, however old, as RFC 9309 §2.4 allows; with no copy, everything is disallowed. robots.txt is the whole origin's file, so a 500 here backs off the domain like a 503, and robots.txt is fetched again once the backoff allows. Each failed fetch counts as an attempt of every crawl request waiting on it, so those requests end skipped with `robots_txt` after their retries, like a request whose own fetch keeps failing.
  - Each origin's robots.txt (scheme, host and port, as RFC 9309 scopes it) is used for at most 24 hours per user, shared by the user's runs, then fetched again. The last good copy is kept past that only to keep applying while robots.txt answers a 429 or a 5xx or is unreachable; losing it only means fetching robots.txt again. Fetching it counts in the domain's Politeness.
    > Amended by [ADR 0105](0105-a-programs-tests-replay-a-response-recording-in-a-test-engine-that-reaches-no-site.md): a Run replaying a Response recording reads robots.txt from the recording alone, never from or into this shared copy, and one the recording lacks reads as no rules, since nothing reaches the site.
  - It applies to start URLs too, so a crawl that does nothing says why (G1).
- **Crawl-delay is applied with robots.txt.** A host's Crawl-delay raises the delay of its whole domain key, as ADR 0004 keys every per-domain limit; the largest one applies. Crawlee's unreleased 4.0 keys it by hostname unless `throttleBy: 'registrableDomain'` is set, and ignores it with a warning when no throttling manager covers the domain ([source](https://github.com/apify/crawlee/blob/8cb2eb6/packages/basic-crawler/src/internals/throttling-request-manager.ts)).
  - It is honoured up to `site_preferences.max_crawl_delay`, 30 s by default as in Nutch and StormCrawler.
  - A host asking for more has its crawl requests skipped with `crawl_delay`, whose fix names that setting, and does not raise the delay.
  - `null` waits out any value; `0` skips every host whose Crawl-delay is above zero. RFC 9309 leaves Crawl-delay out and Google ignores it, but Bing, Crawlee's unreleased 4.0 (through its throttling manager), Nutch, StormCrawler and Heritrix (up to 300 s) honour it. Scrapy has never applied it ([scrapy#892](https://github.com/scrapy/scrapy/issues/892)).
- **One request in flight per domain, at least one second between two starts** (`politeness.domain.concurrency` 1, `politeness.domain.delay` 1 s). These are Scrapy's template values, chosen in answer to #6597, and StormCrawler's; Nutch waits 5 s and Heritrix at least 3 s.
  - The delay counts between starts, so a domain gets at most one request a second, and fewer as it slows down: a site that takes 3 s to answer gets one request every 3 s.
  - A browser-tier fetch counts as one request, whatever its page loads.
    > Amended by [ADR 0118](0118-every-fetch-tier-follows-redirects-as-the-browser-does-fails-a-cut-body-and-resends-only-what-a-site-never-processed.md): each redirect or refresh hop of that fetch is a request start of its own, with its own permit; what its page loads besides still counts as nothing.
  - A crawl of many sites stays fast, because ADR 0004 serves domains in turn. One large site is slower: books.toscrape.com's 1,100 pages take about 18 minutes.
  - A `politeness.domain.delay` of 0 means no delay.
- **A failing site slows the crawl.** A 502, a 504, and a connection that is refused or times out back off the domain as ADR 0021's 503 does. A 500 does not, because it usually belongs to one page. A 429 still halves the domain's concurrency.
  > Amended by [ADR 0118](0118-every-fetch-tier-follows-redirects-as-the-browser-does-fails-a-cut-body-and-resends-only-what-a-site-never-processed.md): an HTTP/2 `ENHANCE_YOUR_CALM` from the site backs off the domain too. The backoff's waits are jittered, and a `Retry-After` is honoured up to `politeness.retry_after.max`. Each hop of a redirect is a request start under its domain's limits and delay, with its own permit.
- **The delay is shared** like the count in flight (ADR 0053): a run admits a request to a domain only once its own delay has passed since the last start any run sharing the key made there. Two runs on one site therefore do not double its rate.
  - The last start lives where ADR 0053 keeps the count in flight: the per-user Politeness file, or the Politeness store, whose script stamps it with the store's own clock, as it does its rates.
  - In ADR 0053's degraded mode each cut-off engine may still admit one request at a time. An outage can therefore multiply a domain's rate by the number of cut-off engines, for at most `politeness.degraded_max`.
- **Redundant copies need room.** In a crawl, a hedged request's second copy and `tier_race`'s second fetch (ADR 0040) are crawl requests like any other: each needs a free slot and the delay. With `politeness.domain.concurrency` at 1 neither could ever start. So a crawl's Run that switches either on while it is 1 is refused with `setting_not_applicable` when it opens, naming `politeness.domain.concurrency` as the fix (G3).
  > Amended by [ADR 0093](0093-a-crawl-declares-its-labels-and-one-handler-each-and-a-handler-reads-collies-page-on-every-fetch-tier.md): `tier_race` is the setting `fetch.tier_race`; its second fetch, like a hedge's copy, an escalation's fetch in the browser and a sample comparison's second fetch, is governed as a crawl request is, though none of them is a retry.

### Pressing harder is explicit and recorded

- **What counts.** A value that lets a crawl press a site harder than its built-in default: `politeness.domain.concurrency` above 1, `politeness.domain.delay` below 1 s, `politeness.backoff.enabled` off, `politeness.backoff.initial` or `politeness.backoff.max` shortened, `politeness.sharing` other than `shared`, or `site_preferences.respect_robots_txt` off. A `site_preferences.robots_txt_user_agent` is recorded the same way.
- **Where it can be set.** From any explicit source: code, a CLI flag, an Integration's form, configuration, a Recipe or an Operator policy. Never through an environment variable (ADR 0083). It is one setting per value, as Crawlee's `respectRobotsTxtFile` and Scrapy's `ROBOTSTXT_OBEY` are: no confirmation, acknowledgement value or allow list besides (S2, S3).
- **How it is recorded**, as ADR 0028 requires of any weakening:
  - the Audit log records each such value once per run, with the layer that set it;
  - the run's result lists it (ADR 0023);
  - the Provenance of a Record fetched against its host's robots.txt marks it;

    > Amended on 2026-11-01 (ADR 0138; D8 of [the sweep of ADR 0140](https://github.com/goodfetch/goodfetch/blob/claude/festive-curie-bc4545/rnd/research/2026-11-01-shared-paths-ticket-sweep/proposal.md), decided by the orchestrator under the owner's « fait tout ce que tu peux faire seul », open to their review; #684, #1649): this mark needs robots.txt read while `site_preferences.respect_robots_txt` is off, against ADR 0138's "switched off costs nothing". So it is its own setting, `site_preferences.mark_robots_txt_ignored`, off by default. The other four records above stay on.

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
| `politeness.domain.concurrency` | 1 | a crawl |
| `politeness.domain.delay` | 1 s | a crawl |
| `politeness.backoff.enabled` | on | a crawl |
| `politeness.backoff.initial` | 2 s | a crawl |
| `politeness.backoff.max` | 60 s | a crawl |
| `politeness.ip.concurrency` (was `per_ip_concurrency`) | none | a crawl |
| `politeness.proxy.concurrency` (was `proxy_concurrency`) | none | a crawl |
| `politeness.proxy.rate` (was `proxy_rate`), in requests per second | none | a crawl |
| `politeness.sharing` (was `politeness_sharing`) | `shared` | a crawl |
| `politeness.degraded_max` (was `politeness_degraded_max`) | 10 minutes | the engine |
| `site_preferences.respect_robots_txt` | on | a crawl |
| `site_preferences.mark_robots_txt_ignored` | off | a crawl |
| `site_preferences.max_crawl_delay` | 30 s | a crawl |
| `site_preferences.robots_txt_user_agent` | none | a crawl |
| `site_preferences.respect_use_signals` (was `respect_site_preferences`) | off | every run |

> Amended by [ADR 0088](0088-collie-carries-every-capability-except-those-only-many-pages-need.md): `politeness.proxy.concurrency`, `politeness.proxy.rate` and `politeness.sharing` also apply to a program's own navigation through that proxy, in both Products, since they protect the user's account with the proxy's vendor.

> Amended by [ADR 0118](0118-every-fetch-tier-follows-redirects-as-the-browser-does-fails-a-cut-body-and-resends-only-what-a-site-never-processed.md): `politeness.retry_after.max` (15 minutes, a crawl) bounds how long a domain waits on a `Retry-After`. Whatever its value, nothing is sent sooner than the site asked, so it is not among the values that press a site harder.

- **Scopes.** These keys are set on a connection or a Run, never on a Session or a page, since ADR 0053 keeps Politeness settings per run. `politeness.degraded_max` is the engine's own, set in user configuration or an Operator policy only; a program's `settings` refuses it with `setting_not_applicable`, and `explainSettings` still shows it.
- **Collie Runs.** Politeness is Foxhound's (ADR 0084), so of these keys a Collie Run takes only `site_preferences.respect_use_signals`; any other fails with `setting_not_applicable` (ADR 0083). In TypeScript and Python the types leave them out of a Collie Run's settings, so the type checker catches the mistake first (G3). In Rust, whose `Settings` takes any key (ADR 0083), the Run refuses it when it opens. Whether Collie carries ADR 0053's per-proxy limits is [Collie's exact capability set](https://github.com/goodfetch/goodfetch/issues/455)'s.
  > Amended by [ADR 0087](0087-each-product-is-an-executable-crate-that-links-only-its-crates-and-has-its-own-size-budget.md): on a Collie engine, a key of this table that only Foxhound honours fails with `not_in_product`, naming Foxhound, before the scope check; `setting_not_applicable` stays for a Collie Run on a Foxhound engine. Which of these keys Collie honours is Collie's exact capability set's.
  > Amended by [ADR 0088](0088-collie-carries-every-capability-except-those-only-many-pages-need.md): a Collie Run also takes `politeness.proxy.concurrency`, `politeness.proxy.rate` and `politeness.sharing`, since Collie carries the per-proxy limits and their sharing.
- **Durations** take each language's duration type (ADR 0080).

### Skip reasons

A request skipped because of Site preferences ends `skipped` (ADR 0079), with one of three reasons, each with its own fix (G1, G6):

- `robots_txt`: a Disallow line of the origin's robots.txt, or a robots.txt answered with a 429 or a 5xx, or unreachable, with no earlier copy, until the request's retries ran out.
- `crawl_delay`: a Crawl-delay above `site_preferences.max_crawl_delay`.
- `use_signals`: an AI-use signal that refuses the run's Purpose while `site_preferences.respect_use_signals` is on.

## Considered Options

- **Keeping ADRs 0016 and 0021 as they were**, for parity with Crawlee and Spider: rejected by G5. Eight in flight with no delay is Scrapy's library default, the one site owners reported (#6597 and [#6755](https://github.com/scrapy/scrapy/issues/6755)). The cost of that default falls on the tool's name, and a program that needs Crawlee's pace sets two values.
- **Safer values in a project template only** (Scrapy 2.13.3): rejected. A program that calls the SDK never sees a template.
- **Obeying robots.txt and the per-domain limits on every navigation, Collie's included**: rejected. It would break the Playwright-shaped page API (ADR 0019): a Collie program could not open two tabs on one site. RFC 9309 addresses crawlers, ADR 0083 rules out a default that differs by product, so one setting cannot be on for crawls and off for pages, and ADR 0084 places Politeness in Foxhound.
- **Defaults per Purpose** (robots.txt obeyed only for `ai-train`, say): rejected above.
- **Two in flight per domain, or a shorter delay** (Crawl4AI's 0.1 s; the research's "1–2 per host"): rejected. With the delay counted between starts, a second request in flight only helps once the site answers more slowly than the delay, which is when it most needs relief.
- **A longer delay** (Nutch's 5 s, Heritrix's 3 s minimum): rejected. One second is the value a leader chose against this very evidence (S3), and it keeps a single site's crawl practical.
- **Latency-adaptive pacing by default** (Scrapy's AutoThrottle, Heritrix's delay factor): rejected as the default and kept opt-in (ADR 0037's CO-09). One request in flight with a fixed gap already slows down with the site, and it is predictable.
- **Leaving a crawl handler's own navigation ungated**: rejected. A crawl running eight handlers, each calling `goto` on one site, would press it eight times over its limit with nothing set.
- **A Crawl-delay per host inside a domain key**: rejected. It needs a second clock per host for little gain: ADR 0004 keys every per-domain limit by the registrable domain because subdomains usually share one server. Crawlee 4.0 keys Crawl-delay by hostname unless told otherwise, which lets a site spread over subdomains be paced host by host.
- **Reading a robots.txt answered with 429 as "no rules"**, RFC 9309's literal rule for any 4xx: rejected, as Google rejects it. A 429 asks for less load, and caching it as "no rules" would lift every restriction for a day.
- **Exempting a hedge or a race's second copy from the limits**: rejected. A second request in flight is exactly what the default rules out; a crawl that wants hedges raises its concurrency, which is recorded.
- **Waiting out any Crawl-delay, or capping it silently**: rejected. Waiting stalls a crawl for hours on a host that asks for one request a minute. Capping ignores what the host asked, with no record.
- **Backing off on every 5xx**: rejected. A 500 is usually one broken page, not an overloaded server.
- **Matching only `Foxhound`, with no operator token**: rejected. A crawl whose user agent names `AcmeBot`, set through `identified` or a Hook, would ignore the group a site wrote for `AcmeBot`.
- **Requiring an operator contact, or `identified` by default** (Nutch, StormCrawler, Heritrix): rejected above. It reverses ADR 0021's main decision, which this ticket does not reopen.
- **A contact in the `standard` user agent, or a `From` header**: rejected above.
- **Honouring AI-use signals by default**: rejected above. ADR 0016 is reopened for crawl politeness only.
- **An allow list of domains whose robots.txt is ignored**, or a person's approval as for ADR 0052's Wall allowance: rejected. A Wall allowance guards against evading a condition of access; this is pacing and stated preferences, which one recorded setting covers (S2). Scoping a setting to a part of a crawl is [Crawl definition and handler context](https://github.com/goodfetch/goodfetch/issues/477)'s per-Label question.
  > Amended by [ADR 0093](0093-a-crawl-declares-its-labels-and-one-handler-each-and-a-handler-reads-collies-page-on-every-fetch-tier.md): Politeness and Site preferences are not settable per Label: two Labels on one domain share its slot and delay.

## Consequences

- **Amends ADR 0016.** robots.txt and its Crawl-delay leave the compliance controls that are off by default: a crawl obeys them unless a recorded setting turns them off. `respect_site_preferences` becomes `site_preferences.respect_use_signals` and covers only the AI-use signals and RSL.
- **Amends ADR 0021.** The defaults are 1 request in flight and a 1 s delay per domain. 502, 504 and refused or timed-out connections back off like a 503. Crawl-delay follows `site_preferences.respect_robots_txt` and `site_preferences.max_crawl_delay`. The settings take the keys above.
- **Amends ADR 0053.** The per-domain state shared by every run gains the time of the last request start, and its settings take the keys above.
- **Amends ADR 0004**, whose pointer to ADR 0021's defaults now reads 1 in flight and a 1 s delay, and whose list of shared state gains the last start.
- **Amends ADR 0023**: the backoff it lets a run switch off also covers 502, 504 and refused or timed-out connections.
- **Amends ADR 0040**: a hedge or `tier_race` switched on while `politeness.domain.concurrency` is 1 is refused.
- **Amends ADR 0054**, whose single-site Partition is capped by the new defaults.
- **Amends ADR 0083**, whose list of older keys still to get an area loses `respect_site_preferences`, renamed here.
- **Amends ADRs 0044, 0045, 0049, 0055 and 0060** only to rename the keys they mention: `per_ip_concurrency`, `respect_site_preferences`, `politeness_sharing` and `politeness_degraded_max`. `TESTING.md` takes `politeness.sharing` too.
- **Areas.**
  - Area 6 (Collection) builds robots.txt fetching and caching, Crawl-delay, the skip reasons and the counting of a handler's navigation.
  - Area 3 (Engine runtime) builds the shared delay, the added backoff signals and the refusal of hedges and races.
  - Area 5 (Site access) builds the product-token matching in every Access mode, the operator's token included.
  - Area 4 (Security and egress) builds the Audit log entries and the Provenance mark.
  - Area 11 (Client surfaces) builds the settings types and the skip reasons.
- **ADR 0037's matrix.** SC-01, CO-09 and pain 6 cite this ADR, and areas 3, 4, 5, 6 and 11 list it.
- **The V1 benchmark** reports crawl throughput at these defaults and with politeness raised, as ADR 0016 has it publish what each control costs.
- **Golden scenario 05** sets no politeness value. books.toscrape.com's robots.txt answers 404, so nothing is skipped.
- **Left to other tickets.** [Collie's exact capability set](https://github.com/goodfetch/goodfetch/issues/455) (ADR 0088) decides whether Collie carries ADR 0053's per-proxy limits; [Collie and Foxhound installed on one machine](https://github.com/goodfetch/goodfetch/issues/486) (ADR 0098) decides whether a Collie page's 429 slows a Foxhound crawl on the same machine; [How the products are built](https://github.com/goodfetch/goodfetch/issues/454) owns the seam through which Foxhound's Politeness admits a handler's navigation on Collie's page API. [Collie's page API](https://github.com/goodfetch/goodfetch/issues/472) decides how a use signal that refuses a Collie Run's Purpose surfaces on its page, while `site_preferences.respect_use_signals` is on. [Crawl definition and handler context](https://github.com/goodfetch/goodfetch/issues/477) (ADR 0093) decides how a navigation robots.txt refuses surfaces inside a handler, and whether these settings can be set per Label. A page a handler lends to another library through the raw CDP endpoint (ADRs 0019 and 0084) is no longer driven through the engine's page API; whether its navigation stays counted is [Third-party automation on Foxhound pages, and Collie inside other crawlers](https://github.com/goodfetch/goodfetch/issues/456)'s. [The Run in code](https://github.com/goodfetch/goodfetch/issues/478) (ADR 0102) places the list of values that pressed harder in the Run's result. [The Agent in Collie's SDK API](https://github.com/goodfetch/goodfetch/issues/474) keeps an Agent's own navigation program-directed and decides any opt-in to obey robots.txt for it. Cloudflare credited ChatGPT-User with stopping when robots.txt disallowed it ([report](https://blog.cloudflare.com/perplexity-is-using-stealth-undeclared-crawlers-to-evade-website-no-crawl-directives/)), while OpenAI's [bots page](https://developers.openai.com/api/docs/bots) now says robots.txt may not apply to user-initiated actions.
  > Amended by [ADR 0087](0087-each-product-is-an-executable-crate-that-links-only-its-crates-and-has-its-own-size-budget.md): the seam through which Foxhound's Politeness admits a handler's navigation is decided there: a navigation gate on Collie's page.
  >
  > Amended by [ADR 0089](0089-a-page-is-lent-through-an-endpoint-that-shows-only-its-session-and-collie-joins-other-crawlers-by-borrowing.md): a Lent page's navigation stays the crawl's in a crawl handler: ADR 0087's navigation gate keeps gating the lent page, and the pages the library creates carry it too, so each navigation is admitted once and robots.txt refuses a disallowed one; where interception no longer sees the library's requests, they go ungated and the Run trace says so. A library's own HTTP client, such as Playwright's `request`, sends from the program's process and is the one fetch from the page this cannot count.
  >
  > Amended by [ADR 0100](0100-collies-agent-is-one-bounded-call-on-a-page-judged-by-the-programs-success-check-and-replayed-whole-from-the-plan.md): an Agent's own navigation offers no opt-in to obey robots.txt: it works for one person toward one task, robots.txt is still read and recorded in Provenance, and `agent.domains` bounds where it goes. On a crawl handler's page its navigation is the handler's, governed here.
- **The `prototype/sdk-api` sketch** holds the `politeness.*` and `site_preferences.*` settings and the skip reasons in each language, with a snippet on golden scenario 05. The pain coverage matrix closes P12 as API.
