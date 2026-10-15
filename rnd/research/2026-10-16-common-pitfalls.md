# Common pitfalls of browser automation and crawling: known failure modes from primary sources

State observed on **2026-10-16**. This is the research for [Common pitfalls catalog](https://github.com/goodfetch/goodfetch/issues/547) on the SDK API map [#461](https://github.com/goodfetch/goodfetch/issues/461). The [pain coverage matrix](https://github.com/goodfetch/goodfetch/blob/prototype/sdk-api/sdk-api-sketch/pain-coverage.md) lists the pains users reported in issues and forums. This note collects the failure modes that experienced teams handle without filing issues. It takes them from the leaders' documentation and source code, from crawler practice and from the RFCs and web standards. The [pitfall catalog](https://github.com/goodfetch/goodfetch/blob/prototype/sdk-api/sdk-api-sketch/pitfalls.md) on `prototype/sdk-api` classes each one against Goodfetch's adopted decisions as handled, a gap or declined.

## Method and limits

- **Sources.** Specifications: RFC 3986, 6066, 6585, 7725, 8305, 8996, 9110, 9111, 9112, 9113, 9309 and 9659, the WHATWG URL, Fetch, Encoding and HTML standards, the sitemaps.org protocol and the W3C WebDriver spec. Leaders' documentation and source:
  - Scrapy (docs at `latest`), Crawlee JS (docs, and `master` source where the docs give no default), Playwright (the docs' Markdown source), Puppeteer, Selenium.
  - Heritrix 3 (docs, bean reference and the default `profile-crawler-beans.cxml`), Apache Nutch (`nutch-default.xml`, `regex-normalize.xml.template`, `regex-urlfilter.txt.template`), StormCrawler (core and archetype source).
  - Google Search Central's crawling documentation.
  - Operations: Cloudflare's bot and challenge docs, the Apify platform docs, Kubernetes' pod lifecycle, Tokio's shutdown guide, the Linux man pages, the Rust `std::time` docs, Spidermon, dateparser, Babel and w3lib.
  - Common Crawl's FAQ.
- **How they were read.** Each page was fetched and read on 2026-10-15 or 2026-10-16 with WebFetch or `curl`. Four reading passes ran in parallel, one per group of domains. Exa had no credits left, so it was not used. Two claims were checked again by hand: Scrapy's `DOWNLOAD_MAXSIZE` applying before and after decompression, and RFC 9309's 5xx, redirect and 500 KiB rules. A value marked "(code)" was read in source code rather than documentation. A value read on Crawlee's `master` branch may be ahead of its latest release.
- **Not read.**
  - The Archive-It crawler-traps article (behind a Cloudflare challenge): its facts come from a search snippet only.
  - The Black Hat EU 2017 HTTP/2 fingerprinting slides (larger than the fetch limit): that entry rests on a search summary.
  - Google's soft-404 page (it answered 404): soft-404 facts come from two other Google pages.
  - IIPC trap guidance: not located.

  CDP `Runtime.enable` detection rests on one vendor's write-up (rebrowser), not on a standard.
- **Quotations** are at most ten words per source; everything else is paraphrase.
- **What this note does not do.** It does not judge Goodfetch. Which pitfall Goodfetch's ADRs, `TESTING.md` and ADR 0037 handle, and how, is the catalog's work. No code was run against a live site. The one observed fact is Scrapy's own `broad-crawls.html`, which answers 200 with a 645-byte JavaScript redirect stub (part 2, item 20; part 4, A2).

## Synthesis

The four parts below describe 110 failure modes. Grouped by domain, with the closures the catalog records:

| Domain | What goes wrong | Where the leaders converge | Where they split |
|---|---|---|---|
| URLs and crawl space (part 1) | Infinite URL spaces (calendars, facets, repeated path segments, session IDs), duplicate spellings of one URL, redirect loops, scope leaks, robots.txt and sitemap edge cases | Depth, hop and outlink caps exist everywhere, but most are off by default (Scrapy `DEPTH_LIMIT` 0, Crawlee `maxCrawlDepth` unset). The fragment is dropped from the duplicate key (Scrapy, Crawlee, StormCrawler, Nutch). RFC 9309 is the reference for robots.txt. | How aggressive canonicalization is (Crawlee strips a trailing slash, against RFC 3986 §6.2.4), and which session IDs are stripped (Nutch broadly, Heritrix only 32-character IDs). Redirect hop limits: 20 for Fetch and Scrapy, 10 for Google, 0 inline for Nutch. A 5xx robots.txt: the RFC disallows everything, Google relaxes it after 12 hours. |
| HTTP and network (part 2) | DNS, TLS, HTTP/2 and keep-alive failures. 429 and `Retry-After` in both forms. Truncated, compressed, huge or non-HTML bodies. Charset sniffing, soft 404s, redirects that change the method or leak credentials. | Retries limited to idempotent requests (RFC 9110 §9.2.2). Body size caps that apply after decompression (Scrapy 1 GiB, Google 15 MB). A truncated body is recorded as incomplete (RFC 9112 §8; Scrapy fails it). | Whether `Retry-After` is honoured (urllib3 and Crawlee 4 do, the Scrapy docs do not say). Certificate verification (Scrapy off by default). Charset order (the HTML standard's sniffing algorithm, or the simpler orders of the libraries). |
| Pages and the browser (part 3) | Hydration races, client-side routing, unreliable `networkidle`, fixed sleeps, stale elements, moving or covered targets. Overlays and consent banners, lazy loading, infinite scroll, virtualized lists. Frames, shadow DOM, canvas, popups and dialogs. Locale, viewport, headless differences, service workers, the back/forward cache. Downloads, crashes, memory bloat, zombie processes. | Lazy, strict locators with auto-waiting (Playwright, Puppeteer). `networkidle` discouraged. Dialogs dismissed by default. BFCache off (Playwright). Browsers retired after a page count (Crawlee: 100). Run with `--init` in containers. | Service workers: allowed by default (Playwright), blockable. Closed shadow roots are reached by none. Virtualized lists and canvas have no helper in any leader. |
| Sites and anti-bot (part 4, A) | Challenge pages taken for content, 200 stubs, honeypot links, rate bans, IP reputation, TLS, HTTP/2 and automation fingerprints, layout drift, geo-dependent content, logins expiring | Session pools that bind an IP to its cookies (Crawlee). Challenge pages are recognizable by status and header (Cloudflare's `cf-mitigated`). Fingerprints are judged by consistency. | Generated fingerprints by default (Crawlee) or not. Retry on block (Crawlee's `retryOnBlocked`) or escalate. |
| Long runs and operations (part 4, B) | Queue and duplicate-set memory growth, no memory guard, disk full, resume only after a clean stop, at-least-once duplicates, wall-clock timeouts, no back-pressure, hung fetches, file descriptors, log volume, SIGTERM | Disk-backed queues for broad crawls (Scrapy `JOBDIR`, Heritrix BDB). Monotonic clocks for elapsed time. A drain on SIGTERM within the grace period (Kubernetes 30 s). | A memory ceiling that closes the spider (Scrapy) or only scales down (Crawlee). Only Heritrix pauses on low disk (8,192 MiB free). Checkpoints off by default (Heritrix). Storage purged on start (Crawlee). |
| Data (part 4, C) | Schema drift, silent empty extraction, duplicate records, mojibake, locale-formatted dates and numbers, partial output files, CSV headers taken from the first item | Validation and coverage monitors after the fact (Spidermon). Temporary files renamed into place (POSIX `rename`). | Invalid items kept by default (Spidermon). Header from the first item (Scrapy) or a declared field list. |

The catalog finds most of these handled by an adopted decision, many already proved by a `TESTING.md` case or a Proving ground page. The rest close in six grilling tickets:
- redirect limits, truncated bodies and transport failures on the fetch tiers;
- decoding a response the same way on every tier;
- URL identity and crawl-space rules beyond ADR 0005's learned rules;
- page states the page API leaves open;
- a disk that fills under a long Run, and log volume;
- locale-dependent values under Grounding.

A few are declined with their reasons. The catalog gives the counts.

## Sources by part

Each part keeps its own source list per item, with full URLs. Items are cited from the catalog as part and number: "1.17" is part 1, item 17; "4.B6" is part 4, item B6.

## Part 1: URLs and crawl space, robots.txt and sitemaps

Research date: 2026-10-16. Every default below was read on the cited page or source file. Values marked "(code)" come from source files rather than docs. Quotes are kept to 10 words or fewer per source.

### A. Infinite and oversized crawl spaces

#### 1. Spider traps (infinite URL spaces in general)
- **Failure mode:** Pages generate an unbounded number of distinct URLs (calendars, filters, session tokens, broken relative links), so the frontier never drains and the crawl budget goes to near-worthless pages. You can't tell a URL is useless without fetching it first.
- **What leaders do:**
  - Google says complex URLs make Googlebot use more bandwidth than needed or leave content unindexed. It recommends robots.txt blocks for search results, calendars and sort/filter functions.
  - Heritrix has a status code for "chaff" detection of traps (-4000). It also has hop-limit status codes for link hops (-4001) and embed hops (-4002).
  - Crawlee documents that `maxRequestsPerCrawl` has no default and "should always be set" to prevent infinite loops.
  - Archive-It says to spot traps in the Hosts report (many queued documents on one host), then exclude them with scoping rules. This comes from the search snippet only (see sources).
- **Sources:**
  - https://developers.google.com/search/docs/crawling-indexing/url-structure
  - https://heritrix.readthedocs.io/en/latest/glossary.html
  - https://crawlee.dev/js/api/basic-crawler/interface/BasicCrawlerOptions
  - https://support.archive-it.org/hc/en-us/articles/208332943-Identify-and-avoid-crawler-traps- (not fetched: behind a Cloudflare challenge; facts from search-result snippet only)

#### 2. Repeating path segments (relative-link loops)
- **Failure mode:** A parent-relative or relative link on the wrong page, often with a server that returns 200 for any path, keeps growing the path (`/a/b/a/b/a/...`). Google names this case: a `../../category/stuff` link can generate bogus URLs. It recommends root-relative links and correct status codes for missing pages.
- **What leaders do:**
  - Nutch's default `regex-urlfilter.txt` rejects any URL whose slash-delimited segment repeats 3 or more times: `-.*(/[^/]+)/[^/]+\1/[^/]+\1/`.
  - Heritrix `PathologicalPathDecideRule` defaults to `maxRepetitions = 2` (code). It rejects 3 identical consecutive segments, such as `/a/a/a/`.
  - StormCrawler `BasicURLFilter` defaults to `maxPathRepetition = 3` (code). It rejects a URL once any non-empty segment appears a third time, consecutive or not. URLs with fewer than 5 `/`-split pieces are exempt.
- **Sources:**
  - https://developers.google.com/search/docs/crawling-indexing/url-structure
  - https://raw.githubusercontent.com/apache/nutch/master/conf/regex-urlfilter.txt.template
  - https://raw.githubusercontent.com/internetarchive/heritrix3/master/modules/src/main/java/org/archive/modules/deciderules/PathologicalPathDecideRule.java
  - https://raw.githubusercontent.com/apache/stormcrawler/main/core/src/main/java/org/apache/stormcrawler/filtering/basic/BasicURLFilter.java

#### 3. Excessive path depth and URL length
- **Failure mode:** Trap URLs often grow without bound in path depth or length. Very long URLs also break servers, logs and storage keys.
- **What leaders do:**
  - Heritrix `TooManyPathSegmentsDecideRule` defaults to `maxPathDepth = 20` (default profile).
  - Scrapy `URLLENGTH_LIMIT` defaults to 2083.
  - Nutch `db.max.outlink.length` defaults to 4096 characters, checked before normalizers and filters. It applies to outlinks and redirects.
  - StormCrawler `maxLength` defaults to -1 (off) in code; the archetype's `urlfilters.json` sets 1024.
  - Sitemaps.org requires each `<loc>` to be under 2,048 characters.
- **Sources:**
  - https://raw.githubusercontent.com/internetarchive/heritrix3/master/engine/src/main/resources/org/archive/crawler/restlet/profile-crawler-beans.cxml
  - https://docs.scrapy.org/en/latest/topics/settings.html
  - https://raw.githubusercontent.com/apache/nutch/master/conf/nutch-default.xml
  - https://raw.githubusercontent.com/apache/stormcrawler/main/archetype/src/main/resources/archetype-resources/src/main/resources/urlfilters.json
  - https://www.sitemaps.org/protocol.html#xmlTagDefinitions

#### 4. Calendars
- **Failure mode:** A dynamic calendar links forward and backward forever (for example `calendar.php?d=13&m=8&y=2011`), producing infinite date pages.
- **What leaders do:**
  - Google tells site owners to put `nofollow` on links to future calendar pages, or block calendars in robots.txt.
  - On the crawler side, nothing is calendar-specific. Defenses are the generic depth and hop limits (pitfall 20), per-host budgets, and operator-written regex exclusions. Heritrix ships a `MatchesListRegexDecideRule` (REJECT) whose regex list is empty by default.
- **Sources:**
  - https://developers.google.com/search/docs/crawling-indexing/url-structure
  - https://raw.githubusercontent.com/internetarchive/heritrix3/master/engine/src/main/resources/org/archive/crawler/restlet/profile-crawler-beans.cxml

#### 5. Faceted navigation and combinatorial query parameters
- **Failure mode:** Filters (color, size, price) and sorts combine multiplicatively, giving huge numbers of near-identical listing URLs. Crawlers usually fetch many of them before recognizing them as useless, which also slows discovery of new pages.
- **What Google recommends:**
  - If filtered pages are not needed: disallow facet parameters in robots.txt, or put filters in `#` fragments.
  - If they are needed:
    - Use `&` as the separator; commas, semicolons and brackets are hard to recognize.
    - Keep filter order consistent and prevent duplicate filters.
    - Return 404 for empty, duplicate or nonsensical filter combinations and for nonexistent pagination.
  - `rel=canonical` and `nofollow` are less effective long-term. `nofollow` works only if every link to the URL carries it.
  - The crawl-budget guide lists differently sorted versions and repetitive infinite-scroll pages as URLs to block, and says soft 404s keep being crawled.
- **Crawler implication:** Cap distinct query-parameter variants per path, or per-host URL counts, and treat empty-result pages as low value.
- **Sources:**
  - https://developers.google.com/search/docs/crawling-indexing/crawling-managing-faceted-navigation
  - https://developers.google.com/crawling/docs/crawl-budget
  - https://developers.google.com/search/docs/crawling-indexing/url-structure

### B. URL identity, normalization and duplicates

#### 6. Session IDs in URLs
- **Failure mode:** Each visit mints a new token (`;jsessionid=`, `PHPSESSID=`, `sid=`), so every page appears under endless distinct URLs. Google advises cookies instead of session IDs in URLs.
- **What leaders do:**
  - Nutch's default `regex-normalize.xml` rule 1 strips `sid`, `phpsessid`, `sessionid` (and `jsessionid`, `lsid`-style variants) case-insensitively: `(?i)(;?\b_?(l|j|bv_)?(sid|phpsessid|sessionid)=.*?)(\?|&|#|$)` becomes `$4`.
  - Heritrix `StripSessionIDs` (code) strips only `jsessionid`/`phpsessid`/`sid` followed by exactly 32 alphanumerics, and `ASPSESSIONID` plus 8 letters, `=`, 24 letters. Other lengths pass through.
  - StormCrawler has no dedicated session-ID rule. It moves `;params` from the last path segment into the query, then removes only names listed in `queryElementsToRemove` (empty by default).
- **Sources:**
  - https://developers.google.com/search/docs/crawling-indexing/url-structure
  - https://raw.githubusercontent.com/apache/nutch/master/conf/regex-normalize.xml.template
  - https://raw.githubusercontent.com/internetarchive/heritrix3/master/modules/src/main/java/org/archive/modules/canonicalize/StripSessionIDs.java
  - https://raw.githubusercontent.com/apache/stormcrawler/main/core/src/main/java/org/apache/stormcrawler/filtering/basic/BasicURLNormalizer.java

#### 7. Tracking and irrelevant parameters (utm_*, gclid, ref)
- **Failure mode:** Referral and analytics parameters (`gclid=`, `ref=`, `referrerid=`, `click=`) don't change content but create distinct URLs. Google lists `?gclid=` URLs as a duplicate source.
- **What leaders do:**
  - No surveyed crawler strips `utm_*` by default.
  - StormCrawler offers `queryElementsToRemove` (empty by default). Its `removeHashes` option drops parameters whose value is a 32-hex string: false in code, true in the archetype.
  - w3lib's `url_query_cleaner()` (used with Scrapy) keeps or removes named parameters.
  - Removal lists are operator configuration, never built in.
- **Sources:**
  - https://developers.google.com/search/docs/crawling-indexing/consolidate-duplicate-urls
  - https://developers.google.com/search/docs/crawling-indexing/url-structure
  - https://raw.githubusercontent.com/apache/stormcrawler/main/core/src/main/java/org/apache/stormcrawler/filtering/basic/BasicURLNormalizer.java
  - https://w3lib.readthedocs.io/en/latest/w3lib.html

#### 8. Query parameter ordering, empty `?`, stray `&`
- **Failure mode:** `?a=1&b=2` and `?b=2&a=1`, as well as a trailing `?`, `?&` or `&&`, are treated as different URLs, though most servers treat them the same. Collapsing them is not always safe: RFC 3986 §6.2.3 says empty delimiters like a bare `?` can't be removed unless the scheme allows it.
- **What leaders do:**
  - Scrapy's request fingerprint canonicalizes with w3lib `canonicalize_url`. It sorts query arguments by key then value, keeps blank values by default, normalizes percent-encoding case, resolves dot segments and drops the fragment.
  - Scrapy's `LinkExtractor` defaults to `canonicalize=False`. Its docs warn that canonicalization is for duplicate checking and can change the URL the server sees.
  - Nutch's default regex normalizer turns `?&` into `?`, collapses repeated `&`, and strips a trailing `?`, `&` or `.`.
  - StormCrawler sorts parameters only when query processing is on (`queryElementsToRemove` set or `removeHashes` true). Its `unmangleQueryString` (default true) turns the first `&` into `?` when a URL has `&` and `=` but no `?`.
- **Key design point:** Dedup on a canonical key, but fetch the original URL.
- **Sources:**
  - https://docs.scrapy.org/en/latest/topics/request-response.html
  - https://w3lib.readthedocs.io/en/latest/w3lib.html
  - https://docs.scrapy.org/en/latest/topics/link-extractors.html
  - https://raw.githubusercontent.com/apache/nutch/master/conf/regex-normalize.xml.template
  - https://raw.githubusercontent.com/apache/stormcrawler/main/core/src/main/java/org/apache/stormcrawler/filtering/basic/BasicURLNormalizer.java
  - https://www.rfc-editor.org/rfc/rfc3986.html#section-6.2.3

#### 9. Fragments (`#...`)
- **Failure mode:** Fragments never reach the server, so `page#a` and `page#b` are the same fetch. Keeping them in the dedup key multiplies URLs. The reverse problem: hash-routed SPAs (`/#/potatoes`) show different content per fragment, and dropping fragments loses that content. Google says it generally doesn't support fragments for content and recommends the History API.
- **What leaders do:**
  - Scrapy `keep_fragments` defaults to False in fingerprinting.
  - Crawlee `keepUrlFragment` defaults to false; the fragment is removed when computing `uniqueKey`.
  - StormCrawler `removeAnchorPart` defaults to true.
  - Nutch's default normalizer removes `#...` anchors.
  - RFC 3986 §6.2.3 notes fragments are never normalized by scheme-based rules.
- **Sources:**
  - https://docs.scrapy.org/en/latest/topics/request-response.html
  - https://crawlee.dev/js/api/core/interface/RequestOptions
  - https://raw.githubusercontent.com/apache/stormcrawler/main/core/src/main/java/org/apache/stormcrawler/filtering/basic/BasicURLNormalizer.java
  - https://raw.githubusercontent.com/apache/nutch/master/conf/regex-normalize.xml.template
  - https://developers.google.com/search/docs/crawling-indexing/url-structure
  - https://developers.google.com/search/docs/crawling-indexing/crawling-managing-faceted-navigation

#### 10. Case, trailing slash, default port, dot segments, `www.`
- **Failure mode:** One resource can be reached under many spellings. Over-normalizing merges URLs that differ (false positives); under-normalizing wastes fetches.
- **What the spec says:**
  - RFC 3986 §6.2.2 (safe for any URI): lowercase the scheme and host, uppercase percent-encoding hex, decode percent-encoded unreserved characters, remove dot segments.
  - RFC 3986 §6.2.3: for http, an empty path equals `/` and `:80` can be dropped.
  - RFC 3986 §6.2.4: treat `/data` and `/data/` as equal only when a redirect or protocol result shows equivalence.
  - Google: paths are case-sensitive (`/APPLE` and `/apple` are different URLs).
- **Where tools diverge:**
  - w3lib lowercases only the host; its doc lists no trailing-slash change.
  - Crawlee's documented `uniqueKey` example lowercases scheme and host and drops a trailing slash (`HTTP://www.EXAMPLE.com/something/` becomes `http://www.example.com/something`). That is more aggressive than RFC §6.2.4 and can merge distinct resources.
  - StormCrawler always lowercases the host and resolves dot segments (via `URI.normalize()`, when `checkValidURI` is true by default).
  - Nutch collapses duplicate slashes except after `:`.
  - Heritrix's default canonicalization rules (code) are LowercaseRule, StripUserinfoRule, StripWWWNRule, StripSessionIDs, StripSessionCFIDs and FixupQueryString. The default profile shows the bean commented out; its header says commented values reflect defaults.
  - Heritrix SURT form lowercases everything and maps https to http.
- **Sources:**
  - https://www.rfc-editor.org/rfc/rfc3986.html#section-6.2.2
  - https://www.rfc-editor.org/rfc/rfc3986.html#section-6.2.3
  - https://www.rfc-editor.org/rfc/rfc3986.html#section-6.2.4
  - https://developers.google.com/search/docs/crawling-indexing/url-structure
  - https://w3lib.readthedocs.io/en/latest/w3lib.html
  - https://crawlee.dev/js/api/core/interface/RequestOptions
  - https://raw.githubusercontent.com/apache/stormcrawler/main/core/src/main/java/org/apache/stormcrawler/filtering/basic/BasicURLNormalizer.java
  - https://raw.githubusercontent.com/apache/nutch/master/conf/regex-normalize.xml.template
  - https://raw.githubusercontent.com/internetarchive/heritrix3/master/modules/src/main/java/org/archive/modules/canonicalize/RulesCanonicalizationPolicy.java
  - https://raw.githubusercontent.com/internetarchive/heritrix3/master/engine/src/main/resources/org/archive/crawler/restlet/profile-crawler-beans.cxml
  - https://heritrix.readthedocs.io/en/latest/glossary.html

#### 11. Percent-encoding variants
- **Failure mode:** `%7e` vs `~`, `%2f` vs `%2F`, and raw non-ASCII vs UTF-8 percent-encoded paths produce duplicate keys. They also break robots.txt matching if the crawler and the rule are encoded differently. Decoding reserved characters (such as `%2F` becoming `/`) changes meaning.
- **What leaders do:**
  - RFC 3986 §6.2.2: decode only unreserved characters and uppercase the hex.
  - RFC 9309 §2.2.2: before comparing robots rules, percent-encode non-ASCII and reserved characters in rule paths, and decode percent-encoded unreserved ASCII in the URI.
  - StormCrawler always decodes unreserved escapes and `%uXXXX`, uppercases the remaining escapes, and UTF-8-encodes spaces, controls, `[ ] \ |` and non-ASCII.
  - Google: non-ASCII in links should be percent-encoded (`gemüse` becomes `gem%C3%BCse`).
  - WHATWG URL uses distinct percent-encode sets per component (path, query, special-query, fragment, userinfo).
- **Sources:**
  - https://www.rfc-editor.org/rfc/rfc3986.html#section-6.2.2
  - https://www.rfc-editor.org/rfc/rfc9309.html#section-2.2.2
  - https://raw.githubusercontent.com/apache/stormcrawler/main/core/src/main/java/org/apache/stormcrawler/filtering/basic/BasicURLNormalizer.java
  - https://developers.google.com/search/docs/crawling-indexing/url-structure
  - https://url.spec.whatwg.org/#path-percent-encode-set

#### 12. IDN / punycode hosts and odd IPv4 forms
- **Failure mode:** `bücher.example` and `xn--bcher-kva.example` are the same host. Hosts like `0x7f.1` are `127.0.0.1`. Without host canonicalization, politeness queues, scope checks, robots.txt caches and dedup all split one host into several, and SSRF filters can be bypassed.
- **What leaders do:**
  - The WHATWG host parser percent-decodes the host, runs UTS #46 domain-to-ASCII (punycode), lowercases ASCII, and parses hex and octal IPv4 parts.
  - StormCrawler `hostIDNtoASCII` defaults to false in code and is set to true in the archetype `urlfilters.json`.
- **Sources:**
  - https://url.spec.whatwg.org/#concept-host-parser
  - https://raw.githubusercontent.com/apache/stormcrawler/main/core/src/main/java/org/apache/stormcrawler/filtering/basic/BasicURLNormalizer.java
  - https://raw.githubusercontent.com/apache/stormcrawler/main/archetype/src/main/resources/archetype-resources/src/main/resources/urlfilters.json

#### 13. RFC parser vs browser (WHATWG) parser disagreement
- **Failure mode:** Real-world `href` values contain backslashes, tabs, newlines and leading or trailing spaces. A strict RFC 3986 parser rejects or mis-resolves them, while browsers (and so site authors' expectations) accept them, so the crawler sees different links than users do.
- **What leaders do:**
  - The WHATWG basic URL parser strips leading and trailing C0-control-or-space and removes all tabs and newlines (as validation errors, not failures). It treats `\` as `/` for special schemes, and matches `.`/`..` segments case-insensitively including `%2e` forms.
  - Scrapy's `LinkExtractor` `strip=True` (default) trims attribute whitespace per HTML5.
  - w3lib `safe_url_string()` parses the way browsers do (WHATWG) but does not block schemes like `javascript:`.
- **Sources:**
  - https://url.spec.whatwg.org/#concept-basic-url-parser
  - https://docs.scrapy.org/en/latest/topics/link-extractors.html
  - https://w3lib.readthedocs.io/en/latest/w3lib.html

#### 14. Relative URL resolution and `<base href>`
- **Failure mode:** Links are resolved against the wrong base. Typical causes: the pre-redirect URL instead of the final one, a `<base>` element ignored, or the wrong one of several `<base>` elements used. The result is broken or looping URLs (see pitfall 2).
- **What the specs say:**
  - RFC 3986 §5.1: base comes from the content first, then the encapsulating entity, then the retrieval URI. After redirects, the retrieval URI is the last one, which actually returned the content.
  - HTML: only the first `base` element with `href` (in tree order) counts. Its href is parsed against the fallback base URL. If parsing fails or the scheme is `data:` or `javascript:`, the fallback base is used.
- **What tools do:** Crawlee `enqueueLinks` `baseUrl` resolves relatives for Cheerio, but is ignored with Puppeteer, where the browser resolves.
- **Sources:**
  - https://www.rfc-editor.org/rfc/rfc3986.html#section-5.1
  - https://www.rfc-editor.org/rfc/rfc3986.html#section-5.2
  - https://html.spec.whatwg.org/multipage/semantics.html#the-base-element
  - https://crawlee.dev/js/api/core/interface/EnqueueLinksOptions

#### 15. Trusting or ignoring `rel=canonical`
- **Failure mode:** Treating `rel=canonical` as authoritative can silently drop distinct pages, such as a mis-templated site canonicalizing everything to the homepage. Ignoring it wastes fetches on parameter variants.
- **What Google does:**
  - Treats it as a strong signal, but "a hint, not a rule." It combines it with redirects (strong) and sitemap inclusion (weak).
  - Accepts it only in `<head>`; recommends absolute URLs.
  - Supports an HTTP `Link` header for non-HTML (PDF), and says using both the element and the header is error-prone.
  - Says don't use robots.txt for canonicalization; the canonical is crawled most, duplicates less often.
- **Sources:**
  - https://developers.google.com/search/docs/crawling-indexing/consolidate-duplicate-urls
  - https://developers.google.com/search/docs/crawling-indexing/canonicalization

#### 16. Near-duplicate content under distinct URLs
- **Failure mode:** URL normalization can't catch pages whose content is identical or nearly so: region variants, print views, sort orders, boilerplate-only differences. Exact-hash dedup misses near-duplicates.
- **What leaders do:**
  - Google clusters pages with the same or very similar primary content and picks one canonical. Language versions with the same primary language count as duplicates.
  - Google's crawl-budget guide says to eliminate duplicate content "rather than unique URLs."
  - Nutch `db.signature.class` defaults to `MD5Signature` (exact). The alternative `TextProfileSignature` has `min_token_len = 2` and `quant_rate = 0.01` by default.
- **Sources:**
  - https://developers.google.com/search/docs/crawling-indexing/canonicalization
  - https://developers.google.com/crawling/docs/crawl-budget
  - https://raw.githubusercontent.com/apache/nutch/master/conf/nutch-default.xml

### C. Redirects

#### 17. Redirect chains and loops
- **Failure mode:** A→B→A loops and long chains burn requests, and can hide off-scope targets. Google says long redirect chains hurt crawling.
- **What leaders do (hop limits):**
  - Google's crawlers follow up to 10 redirect hops by default (the "Redirects and Google Search" page does not state the number).
  - The WHATWG Fetch standard returns a network error when the redirect count is 20.
  - Scrapy `REDIRECT_MAX_TIMES` defaults to 20. It records `redirect_urls`, `redirect_reasons` and `redirect_times`, and supports a per-request `redirect_ttl`.
  - Nutch `http.redirect.max` defaults to 0: redirects aren't followed inline but recorded as new URLs for a later round. `http.robots.redirect.max` is 5 and `sitemap.redir.max` is 3.
  - Robots.txt redirects: see pitfall 22.
- **What Google treats as permanent vs temporary:**
  - Permanent: 301, 308, instant meta refresh, JS `location`.
  - Temporary: 302, 303, 307, delayed meta refresh.
- **Sources:**
  - https://developers.google.com/search/docs/crawling-indexing/http-network-errors
  - https://developers.google.com/search/docs/crawling-indexing/301-redirects
  - https://developers.google.com/crawling/docs/crawl-budget
  - https://fetch.spec.whatwg.org/#http-redirect-fetch (located by text search of the fetched spec)
  - https://docs.scrapy.org/en/latest/topics/downloader-middleware.html
  - https://raw.githubusercontent.com/apache/nutch/master/conf/nutch-default.xml

#### 18. Meta-refresh and JavaScript redirects
- **Failure mode:** Redirects that aren't HTTP 3xx are missed by HTTP-only crawlers, or followed into loops. A `<meta http-equiv=refresh content="300">` with no URL reloads the same page, so a naive follower loops forever. Refresh tags inside `<noscript>` trigger false redirects.
- **What the HTML spec says:** The refresh URL is optional; without one the target is the document itself. Parse failures and `javascript:` URLs abort. Only the first refresh directive per document applies.
- **What leaders do:**
  - Scrapy `METAREFRESH_ENABLED` is True, `METAREFRESH_MAXDELAY` is 100 s (longer delays not followed), and `METAREFRESH_IGNORE_TAGS` is `["noscript"]` since 2.11.2.
  - Google follows JS redirects only if rendering succeeds; it "might never see it" when rendering fails.
  - For robots.txt, Google does not follow logical redirects (JS, meta refresh, frames).
- **Sources:**
  - https://html.spec.whatwg.org/multipage/semantics.html (shared declarative refresh steps, attribute `http-equiv=refresh`)
  - https://docs.scrapy.org/en/latest/topics/downloader-middleware.html
  - https://developers.google.com/search/docs/crawling-indexing/301-redirects
  - https://developers.google.com/search/docs/crawling-indexing/robots/robots_txt

### D. Link discovery and scope

#### 19. Links hidden in JavaScript, onclick, forms
- **Failure mode:** Navigation through `onclick`, `javascript:` hrefs, `<span href>`, or router attributes without `href` is invisible to HTML link extraction. Speculative extraction of URL-like strings from scripts creates junk URLs.
- **What leaders do:**
  - Google reliably crawls only `<a>` with a real `href`. It "can't reliably extract URLs" from other elements, though it may try `javascript:` hrefs.
  - Scrapy `LinkExtractor` defaults to `tags=('a','area')` and `attrs=('href',)`. The docs show a `process_value` regex for `javascript:goToPage(...)`.
  - Heritrix `TransclusionDecideRule` caps speculative hops (`maxSpeculativeHops = 1`) and non-navlink hops (`maxTransHops = 2`).
  - Nutch extracts from `a, area, form, frame, iframe, script, link, img`, but `parser.html.form.use_action` is false because form actions cause empty form submissions.
- **Sources:**
  - https://developers.google.com/search/docs/crawling-indexing/links-crawlable
  - https://docs.scrapy.org/en/latest/topics/link-extractors.html
  - https://raw.githubusercontent.com/internetarchive/heritrix3/master/engine/src/main/resources/org/archive/crawler/restlet/profile-crawler-beans.cxml
  - https://raw.githubusercontent.com/apache/nutch/master/conf/nutch-default.xml

#### 20. Depth and hop limits (unbounded by default)
- **Failure mode:** With no depth cap, any trap is followed indefinitely. Crawl order matters too: depth-first order dives into traps before covering the site.
- **What leaders do:**
  - Scrapy `DEPTH_LIMIT` is 0 (no limit). The FAQ says DFO by default (LIFO scheduler queues); positive `DEPTH_PRIORITY` gives breadth-first.
  - Crawlee `maxCrawlDepth` has no default (unbounded).
  - StormCrawler `maxDepth` is -1 (disabled) in the archetype, with a per-seed metadata override.
  - Heritrix `TooManyHopsDecideRule` `maxHops` defaults to 20.
  - Nutch `scoring.depth.max` is 1000.
  - Outlink caps: Nutch `db.max.outlinks.per.page` is 100; Heritrix frontier `maxOutlinks` is 6000.
- **Sources:**
  - https://docs.scrapy.org/en/latest/topics/settings.html
  - https://docs.scrapy.org/en/latest/faq.html
  - https://crawlee.dev/js/api/basic-crawler/interface/BasicCrawlerOptions
  - https://raw.githubusercontent.com/apache/stormcrawler/main/core/src/main/java/org/apache/stormcrawler/filtering/depth/MaxDepthFilter.java
  - https://raw.githubusercontent.com/internetarchive/heritrix3/master/engine/src/main/resources/org/archive/crawler/restlet/profile-crawler-beans.cxml
  - https://raw.githubusercontent.com/apache/nutch/master/conf/nutch-default.xml

#### 21. Scope leaks: subdomains, scheme switches, off-site redirects
- **Failure mode:** "Same site" is ambiguous (host vs registrable domain, http vs https, `www`). Redirects and bypass flags let the crawl escape scope.
- **What leaders do:**
  - Scrapy `allowed_domains` also matches subdomains: `www.example.org` allows `bob.www.example.org` but not `example.org`. Empty or missing means everything is allowed. `dont_filter` or `allow_offsite` bypasses it.
  - Crawlee `enqueueLinks` defaults to `same-hostname`, which matches both http and https. `same-domain` includes subdomains; `same-origin` also requires the same protocol.
  - Nutch `db.ignore.external.links` defaults to false (mode `byHost`), and `db.ignore.also.redirects` is true, so redirects get the same check.
  - StormCrawler `HostURLFilter` in the archetype sets `ignoreOutsideDomain = true` and `ignoreOutsideHost = false`.
  - Heritrix scopes by seed-implied SURT prefixes; a trailing `/` on the seed decides whether subdomains are included. `seedsRedirectNewSeeds` is true by default, so a redirecting seed widens scope to its target.
- **Sources:**
  - https://docs.scrapy.org/en/latest/topics/downloader-middleware.html
  - https://crawlee.dev/js/api/core/enum/EnqueueStrategy
  - https://crawlee.dev/js/docs/examples/crawl-relative-links
  - https://raw.githubusercontent.com/apache/nutch/master/conf/nutch-default.xml
  - https://raw.githubusercontent.com/apache/stormcrawler/main/archetype/src/main/resources/archetype-resources/src/main/resources/urlfilters.json
  - https://heritrix.readthedocs.io/en/latest/glossary.html
  - https://raw.githubusercontent.com/internetarchive/heritrix3/master/engine/src/main/resources/org/archive/crawler/restlet/profile-crawler-beans.cxml

### E. robots.txt

#### 22. robots.txt fetch outcomes, redirects and caching
- **Failure mode:** Misreading the robots.txt fetch result either crawls a site that is temporarily down (5xx treated as "no file") or locks out a site forever.
- **RFC 9309:**
  - §2.3.1.3: 4xx means unavailable, so everything may be crawled.
  - §2.3.1.4: 5xx or network error means the crawler must assume complete disallow. If it stays unreachable for "a reasonably long period" (example: 30 days), the crawler may treat it as unavailable or keep a cached copy.
  - §2.3.1.2: follow at least 5 redirects, cross-host included; rules apply in the context of the initial authority.
  - §2.4: a cached copy should not be used beyond 24 hours unless the file is unreachable.
- **Divergences:**
  - Google treats 429 like 5xx. On 5xx it stops crawling for 12 hours, then uses the last good copy for up to 30 days; with no cached copy it assumes no restrictions. It caches for up to 24 hours (longer if it can't refresh), and an unresolved redirect chain counts as a 404.
  - Nutch: `http.robots.403.allow = true` (403 means allowed). `http.robots.503.defer.visits = true` defers the host for 300000 ms, 3 retries, on 5xx or 429.
  - CCBot follows up to 5 robots.txt redirects "in line with RFC 9309."
- **Sources:**
  - https://www.rfc-editor.org/rfc/rfc9309.html#section-2.3.1
  - https://www.rfc-editor.org/rfc/rfc9309.html#section-2.4
  - https://developers.google.com/search/docs/crawling-indexing/robots/robots_txt
  - https://raw.githubusercontent.com/apache/nutch/master/conf/nutch-default.xml
  - https://commoncrawl.org/faq

#### 23. robots.txt parsing and matching edge cases
- **Failure mode:** Naive prefix matching, first-match-wins, case-insensitive paths, ignoring `*` and `$`, or unbounded file sizes give wrong allow/deny decisions or memory blowups.
- **RFC 9309 rules:**
  - §2.2.2: the longest match (most octets) wins; allow wins a tie; no match means allowed; `/robots.txt` is always allowed. Path matching is case-sensitive.
  - §2.2.1: user-agent product tokens are matched case-insensitively. Several groups for the same token are merged; with no match the `*` group applies.
  - §2.2.3: `*` matches 0 or more characters and `$` anchors the end; literal `*` and `$` are written `%2A` and `%24`.
  - §2.2.4: unknown records like `Sitemap:` don't end a group.
  - §2.3.1.5: parse every line and use what parses.
  - §2.5: the parse limit must be at least 500 KiB.
- **Where implementations differ:**
  - Google reads 500 KiB and ignores the rest. Rules apply only to the exact host, protocol and port of the file. It supports only `user-agent`, `allow`, `disallow` and `sitemap`; `crawl-delay` is ignored.
  - CCBot obeys `Crawl-delay`.
  - Nutch caps it with `fetcher.max.crawl.delay = 30` s; pages behind a longer delay are skipped.
  - Scrapy `ROBOTSTXT_OBEY` is True in the project template (the fallback default is False); the parser is Protego.
- **Sources:**
  - https://www.rfc-editor.org/rfc/rfc9309.html#section-2.2.2
  - https://www.rfc-editor.org/rfc/rfc9309.html#section-2.5
  - https://developers.google.com/search/docs/crawling-indexing/robots/robots_txt
  - https://commoncrawl.org/faq
  - https://raw.githubusercontent.com/apache/nutch/master/conf/nutch-default.xml
  - https://docs.scrapy.org/en/latest/topics/settings.html

### F. Sitemaps

#### 24. Sitemap size, format and nesting limits
- **Failure mode:** Oversized or mis-encoded sitemaps (huge files, gzip bombs, unescaped `&`, relative `<loc>`) exhaust memory or yield invalid URLs. Index-of-index recursion can loop.
- **What the protocol says:**
  - At most 50,000 URLs and 50 MB (52,428,800 bytes) uncompressed per sitemap; gzip allowed.
  - A sitemap index lists at most 50,000 sitemaps under the same 50 MB limit, and may only reference sitemaps on the same site.
  - UTF-8 with entity-escaped `& ' " < >`; `<loc>` fully qualified and under 2,048 characters.
  - The protocol page does not address nesting indexes.
- **What leaders do:**
  - Google also accepts RSS, mRSS, Atom 1.0 and plain-text sitemaps.
  - Nutch `sitemap.size.max` is 52428800 bytes; it also tries `<host>/sitemap.xml` by default (`sitemap.url.default.sitemap.xml = true`).
  - Scrapy `SitemapSpider` follows all sitemaps in an index by default (`sitemap_follow`) and reads `Sitemap:` lines from robots.txt.
- **Sources:**
  - https://www.sitemaps.org/protocol.html#index
  - https://www.sitemaps.org/protocol.html#escaping
  - https://www.sitemaps.org/protocol.html#xmlTagDefinitions
  - https://developers.google.com/search/docs/crawling-indexing/sitemaps/build-sitemap
  - https://raw.githubusercontent.com/apache/nutch/master/conf/nutch-default.xml
  - https://docs.scrapy.org/en/latest/topics/spiders.html

#### 25. Sitemap scope (path and cross-host rules) and trust in lastmod, priority, changefreq
- **Failure mode:** A sitemap can list URLs outside its authority, letting a third party inject URLs or crawl targets. Its metadata is often unreliable: `lastmod` set to generation time, or `priority`/`changefreq` boilerplate.
- **Scope rules:**
  - Sitemaps.org: a sitemap may contain only URLs under its own directory, with the same protocol and host. Cross-host submission requires the other host's robots.txt to reference the sitemap with a `Sitemap:` line, which proves ownership.
  - Google: a root sitemap covers the whole site; elsewhere a sitemap covers its directory and below, unless submitted via Search Console.
  - Nutch `sitemap.strict.parsing = true` drops URLs outside the sitemap's path prefix.
- **Metadata trust:**
  - `priority` (0.0–1.0, default 0.5) and `changefreq` are hints only. Google ignores both.
  - Google uses `lastmod` only if it is "consistently and verifiably accurate." It should reflect significant changes, not edits like a copyright-year bump.
  - Nutch `sitemap.url.overwrite.existing = false`, so sitemap metadata doesn't overwrite CrawlDb values by default.
  - Scrapy `sitemap_filter` can select entries by `lastmod`.
- **Sources:**
  - https://www.sitemaps.org/protocol.html#location
  - https://www.sitemaps.org/protocol.html#submit_robots
  - https://developers.google.com/search/docs/crawling-indexing/sitemaps/build-sitemap
  - https://raw.githubusercontent.com/apache/nutch/master/conf/nutch-default.xml
  - https://docs.scrapy.org/en/latest/topics/spiders.html

### Notes on the sources

- **Could not fetch:**
  - The Archive-It crawler-traps help article is behind a Cloudflare challenge, and the Wayback copy was blocked for the fetch tool. Its facts come from search snippets only.
  - StormCrawler's `core/.../urlfilters.json` and `default-regex-filters.txt` returned 404 at the paths tried; the archetype `urlfilters.json` was used instead.
- **Not verified:**
  - IIPC trap guidance was not located.
  - The Common Crawl FAQ says CCBot is Nutch-based but does not document trap or dedup defaults.
- **Discrepancies worth recording:**
  - Google's 5xx robots.txt handling (12 h stop, then cache for up to 30 days, then assume allow if no cache) is more lenient than the RFC 9309 "complete disallow".
  - Nutch treats a 403 on robots.txt as allow, which matches the RFC's 4xx rule but is configurable.
  - Crawlee's uniqueKey trailing-slash stripping goes beyond RFC 3986 §6.2.4.
  - The Heritrix default `StripSessionIDs` only matches 32-character IDs.

## Part 2: HTTP and network

Researched 2026-10-16. Every default below was read on the cited page or source file on that date. RFCs were read in their rfc-editor.org plain-text form (`https://www.rfc-editor.org/rfc/rfcNNNN.txt`). They are cited by their HTML section anchors, which point to the same text. Crawlee's TypeScript source was read on the `master` branch, where `packages/basic-crawler/package.json` says 4.0.0, so it may be ahead of the latest release. Scrapy docs were read at `docs.scrapy.org/en/latest`.

Could not fetch:
- `https://developers.google.com/search/docs/crawling-indexing/soft-404-errors` returned HTTP 404. Soft-404 facts come from the two Google pages cited in pitfall 20.
- Exa tools failed because the account is out of credits. WebFetch and curl were used instead.

---

### 1. DNS resolution failure and resolver overload
**Failure mode.** A broad crawl resolves thousands of distinct hostnames. A small resolver pool or an upstream resolver that rate-limits shows up as slow starts and DNS timeouts. A timeout looks like a site failure even when the site is healthy.

**What specs and leaders do.**
- Scrapy:
  - `DNS_TIMEOUT` defaults to 60 s. The clock starts when the query is queued into the reactor thread pool, not when it is sent.
  - DNS runs on a thread pool of `REACTOR_THREADPOOL_MAXSIZE` threads, default 10.
  - For broad crawls, Scrapy recommends running your own DNS server, "with a local cache and upstream to a large" resolver. It also recommends raising `REACTOR_THREADPOOL_MAXSIZE` until DNS timeouts disappear.
  - DNS lookup errors (`CannotResolveHostError`) are retried by default.
- Google treats DNS errors "similarly to `5xx` server errors": crawling slows at once, and unreachable indexed URLs are dropped "within days". It tells site owners to allow both UDP and TCP for DNS, to keep their own DNS servers from being overloaded, and to allow up to 72 h for DNS changes to propagate.

**Sources.**
- https://docs.scrapy.org/en/latest/topics/settings.html#dns-timeout
- https://docs.scrapy.org/en/latest/topics/settings.html#reactor-threadpool-maxsize
- https://docs.scrapy.org/en/latest/topics/optimize.html (the old `broad-crawls.html` URL is now a meta-refresh to this page)
- https://docs.scrapy.org/en/latest/topics/downloader-middleware.html#retry-exceptions
- https://developers.google.com/crawling/docs/troubleshooting/dns-network-errors

### 2. Stale DNS cache entries
**Failure mode.** An in-process DNS cache keeps serving an old address after a site moves hosts or a CDN changes its records. Requests then go to a dead or wrong server and fail with connect errors or TLS name mismatches. The cache can also grow without bound in a broad crawl.

**What specs and leaders do.**
- Scrapy:
  - `DNSCACHE_ENABLED` defaults to `True` and `DNSCACHE_SIZE` to 10000 entries.
  - The cache applies only to the `CachingThreadedResolver` and `CachingHostnameResolver`.
  - The docs do not describe TTL handling.
- RFC 8305 §6: if new DNS answers arrive while connection attempts are still running, the client should add them to its list of addresses to try.
- RFC 8305 §9: applications "should not rely upon a stable hostname-to-address" mapping.
- RFC 9309 §2.4, for robots.txt: crawlers SHOULD NOT use a cached copy for more than 24 h unless the file is unreachable.

**Sources.**
- https://docs.scrapy.org/en/latest/topics/settings.html#dnscache-enabled
- https://www.rfc-editor.org/rfc/rfc8305.html#section-6
- https://www.rfc-editor.org/rfc/rfc8305.html#section-9
- https://www.rfc-editor.org/rfc/rfc9309.html#section-2.4

### 3. IPv6 and IPv4 fallback (Happy Eyeballs)
**Failure mode.** A host publishes an AAAA record, but the IPv6 path is broken. A client that tries addresses one after another and waits for each connect timeout stalls for a long time before it falls back to IPv4. A resolver that only handles IPv4 cannot reach IPv6-only hosts at all.

**What specs and leaders do.**
- RFC 8305 recommends these values (§8):
  - Send AAAA and A queries together, and do not wait for both answers.
  - Resolution Delay: 50 ms. After an A answer arrives, wait this long for the AAAA answer.
  - Connection Attempt Delay: 250 ms between attempts to different addresses.
  - Minimum Connection Attempt Delay: 100 ms, and never less than 10 ms.
  - Maximum Connection Attempt Delay: 2 s.
  - Do not cancel an earlier attempt when starting the next one.
- Scrapy:
  - The default resolver, `CachingThreadedResolver`, works only with IPv4 addresses.
  - `CachingHostnameResolver` supports IPv4 and IPv6 but ignores `DNS_TIMEOUT`.
  - The H2 handler needs `CachingHostnameResolver` for IPv6.

**Sources.**
- https://www.rfc-editor.org/rfc/rfc8305.html#section-3
- https://www.rfc-editor.org/rfc/rfc8305.html#section-5
- https://www.rfc-editor.org/rfc/rfc8305.html#section-8
- https://docs.scrapy.org/en/latest/topics/settings.html#twisted-dns-resolver
- https://docs.scrapy.org/en/latest/topics/download-handlers.html

### 4. TLS certificate errors (expired, self-signed, hostname mismatch)
**Failure mode.** Many real sites serve expired, self-signed or misissued certificates, or leave out intermediate certificates. A strict client fails these fetches, while a lax client silently accepts a man-in-the-middle. Crawlers differ in which way they fail by default, so the same URL succeeds in one tool and fails in another.

**What specs and leaders do.**
- Scrapy: `DOWNLOAD_VERIFY_CERTIFICATES` defaults to `False`, so certificates are not verified unless you opt in.
- Crawlee `HttpCrawlerOptions.ignoreSslErrors`: when `true`, certificate errors are ignored. No default is shown on the docs page.
- urllib3:
  - Strongly discourages turning verification off.
  - Warns with `InsecureRequestWarning` when verification is off.
  - Supports `assert_hostname` for servers whose certificate name differs from the host you connect to.
  - On macOS, Apple's Python/OpenSSL may also consult the system keychain, so a custom CA bundle can unexpectedly accept a certificate.

**Sources.**
- https://docs.scrapy.org/en/latest/topics/settings.html#download-verify-certificates
- https://crawlee.dev/js/api/http-crawler/interface/HttpCrawlerOptions
- https://urllib3.readthedocs.io/en/stable/advanced-usage.html

### 5. Old TLS versions, cipher mismatch and SNI
**Failure mode.**
- Legacy servers that only speak TLS 1.0 or 1.1 fail the handshake with modern clients.
- A client that does not send SNI, or that connects by IP address, gets the default certificate of a shared host, or a handshake alert.
- Some sites change their response based on the client's TLS fingerprint.

**What specs and leaders do.**
- RFC 8996 deprecates TLS 1.0 and 1.1.
- HTTP/2 over TLS requires TLS 1.2 or later. Clients MUST send `server_name` when the server is identified by a domain name (RFC 9113 §9.2).
- RFC 6066 §3: literal IPv4 and IPv6 addresses are "not permitted in HostName". A server that does not recognize the name may send `unrecognized_name` or continue the handshake.
- urllib3 2.x uses TLS 1.2 and later by default. Legacy servers need `ssl_minimum_version`. When connecting by IP, you set `server_hostname` and the `Host` header yourself.
- Scrapy:
  - `DOWNLOAD_TLS_MIN_VERSION` and `DOWNLOAD_TLS_MAX_VERSION` (added in 2.17.0) both default to `None`.
  - `DOWNLOADER_CLIENT_TLS_CIPHERS` defaults to `'DEFAULT'`.
  - Its anti-ban guidance notes that some sites respond differently depending on the client's TLS version.

**Sources.**
- https://www.rfc-editor.org/rfc/rfc8996.html
- https://www.rfc-editor.org/rfc/rfc9113.html#section-9.2
- https://www.rfc-editor.org/rfc/rfc6066.html#section-3
- https://urllib3.readthedocs.io/en/stable/advanced-usage.html
- https://docs.scrapy.org/en/latest/topics/settings.html#download-tls-min-version
- https://docs.scrapy.org/en/latest/topics/practices.html#avoiding-getting-banned

### 6. HTTP/2 GOAWAY and stream resets
**Failure mode.** A server sends GOAWAY to drain a connection, for example during a deploy, or resets single streams. A client that treats every in-flight request as failed either loses work or blindly retries non-idempotent requests. A client that keeps opening streams after GOAWAY breaks the protocol.

**What the spec says (RFC 9113).**
- After GOAWAY the receiver MUST NOT open new streams on that connection. It can open a new connection.
- GOAWAY carries a Last-Stream-ID:
  - Streams above it were not processed and can be safely retried on a new connection.
  - Streams at or below it may have been processed. Only idempotent requests (GET, PUT, DELETE) can be retried.
  - A connection that closes with no GOAWAY means any stream might have been processed.
- `REFUSED_STREAM` in RST_STREAM guarantees the stream was not processed. Clients MAY retry it even for non-idempotent methods (§8.7).
- `ENHANCE_YOUR_CALM` (0x0b) means the peer thinks the client is generating excessive load. Treat it as a back-off signal.
- PING lets a client test an idle connection that a NAT or load balancer may have silently dropped (§8.7).
- `SETTINGS_MAX_CONCURRENT_STREAMS` starts with no limit and is recommended to be no smaller than 100 (§6.5.2).

**Sources.**
- https://www.rfc-editor.org/rfc/rfc9113.html#section-6.8
- https://www.rfc-editor.org/rfc/rfc9113.html#section-8.7
- https://www.rfc-editor.org/rfc/rfc9113.html#section-7
- https://www.rfc-editor.org/rfc/rfc9113.html#section-6.5.2

### 7. Servers and stacks that misbehave on HTTP/2
**Failure mode.** Some origins or middleboxes break on HTTP/2:
- They return 421 after connection coalescing.
- They reject h2 with `HTTP_1_1_REQUIRED`, for example when they need TLS renegotiation.
- They simply perform worse on h2.

HTTP/2 client stacks in crawlers also often lack features that the HTTP/1.1 stack has.

**What specs and leaders do.**
- RFC 9110 §15.5.20: a client receiving 421 MAY retry over a different, fresh connection specific to the origin, even for non-idempotent methods.
- RFC 9113 defines `HTTP_1_1_REQUIRED` (0x0d) so a server can ask for a downgrade (§7, §9.2.1).
- Google's crawlers default to HTTP/1.1 and may switch to HTTP/2 based on past crawl statistics. A site opts out of h2 crawling by answering 421.
- Scrapy's `H2DownloadHandler`:
  - Is HTTPS only, with no HTTP/1.1 fallback and no h2c.
  - Has no proxy support. Server pushes are ignored.
  - Ignores `DOWNLOAD_FAIL_ON_DATALOSS`.
  - Is not confirmed to honor `DOWNLOAD_MAXSIZE`. The docs say that limit is honored by the default handler "but not necessarily" by others.
- Scrapy's httpx handler leaves HTTP/2 off by default (`HTTPX_HTTP2_ENABLED`) and is marked experimental.

**Sources.**
- https://www.rfc-editor.org/rfc/rfc9110.html#section-15.5.20
- https://www.rfc-editor.org/rfc/rfc9113.html#section-7
- https://developers.google.com/crawling/docs/crawlers-fetchers/overview-google-crawlers
- https://docs.scrapy.org/en/latest/topics/download-handlers.html
- https://docs.scrapy.org/en/latest/topics/settings.html#download-fail-on-dataloss

### 8. 429 Too Many Requests and Retry-After
**Failure mode.** A rate-limited client retries 429s on a fixed schedule, or right away, and makes the limit worse.

Retry-After has two forms: an HTTP-date or delta-seconds. A parser that handles only one form silently ignores the other. Rate limits also have no standard scope. They can be per resource, per server, per set of servers, per credential or per cookie, so per-URL backoff can miss the real limit.

**What specs and leaders do.**
- RFC 6585 §4:
  - 429 means "too many requests in a given amount of time".
  - It MAY include Retry-After.
  - It MUST NOT be stored by a cache.
  - The spec does not define how the server identifies the client or counts requests.
- RFC 6585 §7.2: servers under attack may just drop connections instead of sending 429.
- RFC 9110 §10.2.3: `Retry-After = HTTP-date / delay-seconds`. delay-seconds is a non-negative integer, as in `Retry-After: 120`.
- RFC 9110 §5.6.7:
  - Recipients MUST accept all three HTTP-date formats: IMF-fixdate, RFC 850 and asctime.
  - Dates are in UTC.
  - A two-digit RFC 850 year that appears more than 50 years in the future is read as being in the past.
- Google treats 429 as server overload, the same as a 5xx: it slows crawling. All other 4xx codes do not affect crawl rate.
- Scrapy:
  - 429 is in `RETRY_HTTP_CODES`.
  - The RetryMiddleware docs do not say that Retry-After is honored.
  - AutoThrottle adjusts by latency. Non-200 responses may raise the delay but never lower it.
- urllib3 `Retry`:
  - `respect_retry_after_header=True` for status codes 413, 429 and 503.
  - Retry-After is capped by `retry_after_max`, 21600 s.
  - `status_forcelist` is `None` by default, so status-based retries are off until configured.
- Crawlee:
  - The default `blockedStatusCodes` is `[401, 403, 429]`. A session that gets one of these is retired.
  - On `master`, a `ThrottlingRequestManager` honors Retry-After for 429 per domain. Without Retry-After it doubles the delay from `baseDelaySecs` (default 2 s) up to `maxDelaySecs` (default 60 s).
  - Requests held back this way do not count against `maxRequestRetries`.
  - The crawl stops with `PersistentRateLimitError` after `maxDomainStallSecs` (default 900 s) without one request getting through.
  - Its parser accepts digits-only delay-seconds or anything `Date.parse` accepts. It treats `Retry-After: 0` and past dates as no delay.

**Sources.**
- https://www.rfc-editor.org/rfc/rfc6585.html#section-4
- https://www.rfc-editor.org/rfc/rfc6585.html#section-7.2
- https://www.rfc-editor.org/rfc/rfc9110.html#section-10.2.3
- https://www.rfc-editor.org/rfc/rfc9110.html#section-5.6.7
- https://developers.google.com/search/docs/crawling-indexing/http-network-errors
- https://docs.scrapy.org/en/latest/topics/downloader-middleware.html#retry-http-codes
- https://docs.scrapy.org/en/latest/topics/autothrottle.html
- https://urllib3.readthedocs.io/en/stable/reference/urllib3.util.html
- https://crawlee.dev/js/api/core/interface/SessionPoolOptions
- https://github.com/apify/crawlee/blob/master/packages/basic-crawler/src/internals/throttling-request-manager.ts (read via raw.githubusercontent.com)
- https://github.com/apify/crawlee/blob/master/packages/basic-crawler/src/internals/http.ts (read via raw.githubusercontent.com)

### 9. 503 and other transient 5xx, including Retry-After on 503 and 3xx
**Failure mode.** Treating every 5xx as permanent drops pages that were only briefly unavailable. Retrying them aggressively piles load onto an origin that is already overloaded.

Retry-After on a 3xx is easy to miss. It sets a minimum wait before following the redirect. Overloaded servers may also refuse connections outright instead of sending 503.

**What specs and leaders do.**
- RFC 9110 §15.6.4: 503 is a temporary overload or maintenance, and the server MAY send Retry-After. The note says servers might "simply refuse the connection".
- RFC 9110 §10.2.3: on 503, Retry-After is the expected outage length. On any 3xx, it is the minimum wait before issuing the redirected request.
- Scrapy:
  - `RETRY_HTTP_CODES` defaults to `[500, 502, 503, 504, 522, 524, 408, 429]`. 522 and 524 are not IANA-registered status codes.
  - `RETRY_TIMES` defaults to 2 retries, in addition to the first download.
  - `RETRY_PRIORITY_ADJUST` defaults to -1, so retries go to the back of the queue.
- Crawlee:
  - Status codes of 500 and above count as errors by default.
  - `maxRequestRetries` defaults to 3.
  - On `master`, a non-HTML content type returned with a 5xx or a blocked status is treated as transient, so the request stays retryable.
- Google slows crawling on 5xx and on network errors, in proportion to how many URLs fail. URLs that keep failing are dropped. Crawl rate recovers gradually after 2xx responses return.
- robots.txt special case: RFC 9309 §2.3.1.4 says a 5xx or unreachable robots.txt means complete disallow. After a "reasonably long" time, such as 30 days, the crawler may treat it as unavailable or keep using its cache. Google:
  - First 12 h: stops crawling the site and keeps retrying robots.txt.
  - Up to 30 days: uses the last cached copy.
  - After 30 days: acts as if there were no robots.txt if the site is otherwise available.

**Sources.**
- https://www.rfc-editor.org/rfc/rfc9110.html#section-15.6.4
- https://www.rfc-editor.org/rfc/rfc9110.html#section-10.2.3
- https://docs.scrapy.org/en/latest/topics/downloader-middleware.html#module-scrapy.downloadermiddlewares.retry
- https://crawlee.dev/js/api/basic-crawler/interface/BasicCrawlerOptions
- https://github.com/apify/crawlee/blob/master/packages/http-crawler/src/internals/http-crawler.ts (read via raw.githubusercontent.com)
- https://developers.google.com/search/docs/crawling-indexing/http-network-errors
- https://www.rfc-editor.org/rfc/rfc9309.html#section-2.3.1.4
- https://developers.google.com/crawling/docs/robots-txt/robots-txt-spec

### 10. Retry storms, backoff without jitter, and unsafe retries
**Failure mode.**
- Retries at several layers multiply load on a struggling origin.
- Synchronized retries arrive in waves.
- Retrying a non-idempotent request can repeat a side effect.
- Retrying an automatic retry compounds both problems.

**What specs and leaders do.**
- RFC 9110 §9.2.2:
  - A client SHOULD NOT automatically retry a non-idempotent request unless it knows the request is safe to repeat.
  - A proxy MUST NOT retry non-idempotent requests.
  - A client "SHOULD NOT automatically retry a failed automatic retry".
- AWS Builders' Library:
  - Example: three retries at each of five layers raise database load 243x.
  - Retry at a single point in the stack.
  - Use capped exponential backoff with jitter.
  - Limit retries with a local token bucket (a retry budget) rather than a circuit breaker.
  - Retry side-effecting APIs only when they are idempotent.
- AWS Architecture Blog ("Exponential Backoff and Jitter"): un-jittered backoff is clearly the worst. Full and Decorrelated Jitter are best. Equal Jitter is the weakest of the jittered variants. The formulas are shown as images and were not read.
- urllib3 `Retry`:
  - `total=10`, `backoff_factor=0` (backoff off), `backoff_max=120` s, `backoff_jitter=0.0`.
  - The sleep is `backoff_factor × 2^(retries−1)` plus a uniform jitter up to `backoff_jitter`, capped at `backoff_max`.
  - By default it retries only idempotent methods: DELETE, GET, HEAD, OPTIONS, PUT and TRACE.
- Crawlee's BasicCrawlerOptions page documents no delay between ordinary retries. Only the 429 path on `master` backs off (pitfall 8).

**Sources.**
- https://www.rfc-editor.org/rfc/rfc9110.html#section-9.2.2
- https://builder.aws.com/content/3EumjoZascWd1oZiEgL8ORlv3qE/timeouts-retries-and-backoff-with-jitter (redirected from https://aws.amazon.com/builders-library/timeouts-retries-and-backoff-with-jitter/)
- https://aws.amazon.com/blogs/architecture/exponential-backoff-and-jitter/
- https://urllib3.readthedocs.io/en/stable/reference/urllib3.util.html
- https://crawlee.dev/js/api/basic-crawler/interface/BasicCrawlerOptions

### 11. Timeouts: connect, read and total are different
**Failure mode.**
- A read timeout limits the gap between socket reads, not the whole response, so a slow-drip server can hold a request open indefinitely.
- A connect timeout may or may not include DNS and the TLS handshake.
- With no total deadline, stuck requests tie up concurrency slots.
- A timeout set too low causes retry floods.

**What specs and leaders do.**
- urllib3 `Timeout`:
  - `connect` is the time allowed to establish the connection.
  - `read` is the maximum time between consecutive reads, and does not limit the full response.
  - `total` (default `None`) is one budget split across connect and read. When combined with the other two, the shorter applies.
- Scrapy:
  - `DOWNLOAD_TIMEOUT` defaults to 180 s and can be overridden per request with the `download_timeout` meta key.
  - For broad crawls, Scrapy suggests lowering it "to discard stuck requests more quickly".
  - `DNS_TIMEOUT` defaults to 60 s.
- Crawlee:
  - `HttpCrawler.navigationTimeoutSecs` defaults to 30. This is the source default; the docs page shows none.
  - `requestHandlerTimeoutSecs` defaults to 60.
  - Hooks that run around navigation share the navigation budget.
- AWS:
  - Set both a connection timeout and a request timeout.
  - Check whether each timer covers DNS and TLS.
  - Derive timeouts from a latency percentile matched to an acceptable false-timeout rate, such as p99.9 for 0.1%.

**Sources.**
- https://urllib3.readthedocs.io/en/stable/reference/urllib3.util.html
- https://docs.scrapy.org/en/latest/topics/settings.html#download-timeout
- https://docs.scrapy.org/en/latest/topics/optimize.html
- https://crawlee.dev/js/api/basic-crawler/interface/BasicCrawlerOptions
- https://github.com/apify/crawlee/blob/master/packages/http-crawler/src/internals/http-crawler.ts (read via raw.githubusercontent.com)
- https://builder.aws.com/content/3EumjoZascWd1oZiEgL8ORlv3qE/timeouts-retries-and-backoff-with-jitter

### 12. Keep-alive races and connection resets on idle connections
**Failure mode.** A client reuses a pooled persistent connection just as the server closes it for idleness. The request then fails with a reset or an empty reply even though the server is healthy. NATs and load balancers also drop idle connections silently.

**What specs and leaders do.**
- RFC 9112 §9.5: either side MAY close a connection at any time. A client may start a new request at the moment the server closes an "idle" connection. Servers SHOULD keep connections open and let flow control handle temporary overload, rather than closing them and expecting clients to retry.
- RFC 9112 §9.3: pipelined requests left unanswered when a connection closes SHOULD be retried, but the client must not pipeline again right after reconnecting.
- RFC 9110 §9.2.2: idempotent requests can be retried on a new connection after this kind of failure.
- RFC 9113 §8.7: HTTP/2 PING tests whether an idle connection is still alive.
- Scrapy retries `ConnectionLost`, `ConnectionDone` and `ConnectError` by default.

**Sources.**
- https://www.rfc-editor.org/rfc/rfc9112.html#section-9.5
- https://www.rfc-editor.org/rfc/rfc9112.html#section-9.3
- https://www.rfc-editor.org/rfc/rfc9110.html#section-9.2.2
- https://www.rfc-editor.org/rfc/rfc9113.html#section-8.7
- https://docs.scrapy.org/en/latest/topics/downloader-middleware.html#retry-exceptions

### 13. Connection pool exhaustion and too many connections per host
**Failure mode.** The pool is sized for one host but used for many, or concurrency is much larger than pool size. The client then either opens and throws away extra connections, losing reuse, or blocks waiting for a free one. Too many parallel connections to one host look like a denial-of-service attack and get refused.

**What specs and leaders do.**
- RFC 9112 §9.4:
  - Clients "ought to limit" simultaneous connections per server.
  - The spec sets no fixed maximum but encourages being conservative.
  - Servers may reject an excessive number of open connections from one client as abusive.
- urllib3:
  - `PoolManager` keeps up to 10 pools.
  - `maxsize` defaults to 1, so only one connection per host is kept for reuse. Extra connections are made, then discarded.
  - With `block=True`, at most `maxsize` connections are open per host, and further requests wait.
- Scrapy:
  - `CONCURRENT_REQUESTS` defaults to 16.
  - `CONCURRENT_REQUESTS_PER_DOMAIN` is listed as "1 (fallback: 8)".
  - Its broad-crawl guidance sizes global concurrency as per-domain concurrency × number of domains, for example 8 × 10 = 80.
  - Running several crawler instances multiplies these limits, so divide them by the number of crawlers.

**Sources.**
- https://www.rfc-editor.org/rfc/rfc9112.html#section-9.4
- https://urllib3.readthedocs.io/en/stable/advanced-usage.html
- https://docs.scrapy.org/en/latest/topics/settings.html#concurrent-requests-per-domain
- https://docs.scrapy.org/en/latest/topics/optimize.html
- https://docs.scrapy.org/en/latest/topics/practices.html

### 14. Proxy failures
**Failure mode.** Proxy errors are easily mistaken for target-site errors and charged to the site's retry budget. Examples:
- A proxy refuses the CONNECT tunnel or fails authentication.
- An exit IP is already banned by the target.
- A proxy injects its own error pages.

Credentials can also leak on redirect, and some HTTP stacks do not support proxies at all.

**What specs and leaders do.**
- Crawlee:
  - Proxy errors raise a `SessionError`, which rotates the session.
  - `maxSessionRotations` defaults to 10 per request and does not count against `maxRequestRetries`.
  - `proxyConfiguration` rotates proxy URLs.
- Scrapy:
  - `HttpProxyMiddleware` is on by default and reads the `http_proxy`, `https_proxy` and `no_proxy` environment variables.
  - The per-request `proxy` meta key overrides them and also ignores `no_proxy`.
  - `HTTPPROXY_AUTH_ENCODING` defaults to `latin-1`.
  - `TunnelError` is retried by default.
  - The H2 handler has no proxy support, and only the httpx handler supports SOCKS.
  - On redirect it drops `Proxy-Authorization` when the scheme changes.
- RFC 9110 §15.4 says to remove proxy-specific headers such as `Proxy-Authorization` when following redirects.

**Sources.**
- https://crawlee.dev/js/api/http-crawler/interface/HttpCrawlerOptions
- https://github.com/apify/crawlee/blob/master/packages/http-crawler/src/internals/http-crawler.ts (read via raw.githubusercontent.com)
- https://docs.scrapy.org/en/latest/topics/downloader-middleware.html#module-scrapy.downloadermiddlewares.httpproxy
- https://docs.scrapy.org/en/latest/topics/download-handlers.html
- https://github.com/scrapy/scrapy/blob/master/scrapy/downloadermiddlewares/redirect.py (read via raw.githubusercontent.com)
- https://www.rfc-editor.org/rfc/rfc9110.html#section-15.4

### 15. Compression: unsupported or wrong Content-Encoding
**Failure mode.** Each of these produces garbled bodies or decode errors:
- The client advertises an encoding it cannot decode, such as br or zstd without the library installed.
- The server labels a body gzip when it is actually plain or double-compressed.
- A Content-Type that already implies compression is confused with Content-Encoding.
- Different stacks configure the zstd window size differently.

**What specs and leaders do.**
- RFC 9110 §8.4:
  - Content-Encoding lists codings in the order they were applied.
  - "identity" SHOULD NOT appear in Content-Encoding.
  - A format that is inherently compressed is not restated in Content-Encoding.
  - Codings are case-insensitive (§8.4.1).
- RFC 9659 makes zstd window size a requirement in HTTP: decoders MUST support windows up to 8 MB, and encoders MUST NOT require more.
- Scrapy `HttpCompressionMiddleware`:
  - Supports gzip, deflate, brotli and zstd.
  - `COMPRESSION_ENABLED` defaults to `True`.
- Google's crawlers accept gzip, deflate and br, and advertise them in `Accept-Encoding`.

**Sources.**
- https://www.rfc-editor.org/rfc/rfc9110.html#section-8.4
- https://www.rfc-editor.org/rfc/rfc9659.html
- https://docs.scrapy.org/en/latest/topics/downloader-middleware.html#module-scrapy.downloadermiddlewares.httpcompression
- https://developers.google.com/crawling/docs/crawlers-fetchers/overview-google-crawlers

### 16. Decompression bombs and huge bodies (maximum response size)
**Failure mode.** A small compressed body can expand to gigabytes and exhaust memory. A plain huge body, such as a video or a log dump, does the same. A limit applied only to the bytes on the wire misses the bomb.

**What specs and leaders do.**
- Scrapy:
  - `DOWNLOAD_MAXSIZE` defaults to 1073741824 bytes (1 GiB). It applies "both before and after compression": decompression stops and the response is ignored once it would exceed the limit.
  - `DOWNLOAD_WARNSIZE` defaults to 33554432 bytes (32 MiB) and logs a warning.
  - Both can be set per request (`download_maxsize`, `download_warnsize`), and 0 disables them.
  - Checking the size before decompressing has to be implemented inside each download handler. Alternative handlers may not honor the limit.
- Google's crawlers fetch only the first 15 MB of a file by default, and ignore the rest.
- Googlebot for Search fetches the first 2 MB of supported file types and the first 64 MB of a PDF. Its limit is "applied on the uncompressed data".
- RFC 9309 §2.5: robots.txt parsers must handle at least 500 KiB. Google ignores content beyond 500 KiB.

**Sources.**
- https://docs.scrapy.org/en/latest/topics/settings.html#download-maxsize
- https://docs.scrapy.org/en/latest/topics/settings.html#download-warnsize
- https://docs.scrapy.org/en/latest/topics/download-handlers.html
- https://developers.google.com/crawling/docs/crawlers-fetchers/overview-google-crawlers
- https://developers.google.com/search/docs/crawling-indexing/googlebot
- https://www.rfc-editor.org/rfc/rfc9309.html#section-2.5
- https://developers.google.com/crawling/docs/robots-txt/robots-txt-spec

### 17. Truncated bodies and Content-Length mismatch
**Failure mode.** A connection closes early, a chunked body never ends, or fewer bytes arrive than Content-Length declares. The partial page parses "successfully" and is stored as if complete, which loses links and corrupts extracted data. Conflicting Content-Length values are also a sign of request smuggling.

**What specs and leaders do.**
- RFC 9112 §8: a client that receives an incomplete response "MUST record the message as incomplete". A body is incomplete when:
  - a chunked body has no terminating zero-size chunk, or
  - fewer octets arrive than a valid Content-Length declares, or
  - a close-delimited body ends with a TLS "incomplete close".
- RFC 9112 §6.3: an invalid Content-Length is an unrecoverable framing error, unless it is a list of identical valid values. Timing out before Content-Length bytes arrive means the message is incomplete.
- RFC 9111 §3.3: caches may store an incomplete 200 only if they mark it incomplete, and must not serve it as complete.
- Scrapy:
  - `DOWNLOAD_FAIL_ON_DATALOSS` defaults to `True`, so a Content-Length mismatch or unfinished chunked body raises `ResponseDataLossError`, which is retried by default.
  - If set to `False`, the response passes with a `dataloss` flag.
  - The H2 handler ignores this setting.

**Sources.**
- https://www.rfc-editor.org/rfc/rfc9112.html#section-8
- https://www.rfc-editor.org/rfc/rfc9112.html#section-6.3
- https://www.rfc-editor.org/rfc/rfc9111.html#section-3.3
- https://docs.scrapy.org/en/latest/topics/settings.html#download-fail-on-dataloss
- https://docs.scrapy.org/en/latest/topics/downloader-middleware.html#retry-exceptions

### 18. Binary or non-HTML content where HTML was expected
**Failure mode.** A link leads to a PDF, image, archive or video. The crawler downloads it in full and hands it to the HTML parser, wasting bandwidth and memory, or it misparses JSON error bodies.

**What specs and leaders do.**
- Crawlee `HttpCrawler`:
  - Supports only `text/html`, `text/xml`, `application/xhtml+xml`, `application/xml` and `application/json` by default. `additionalMimeTypes` extends the list, and `*/*` allows everything.
  - On `master`, an unsupported type throws with a message that says it is "Skipping resource". It also sets `noRetry`, unless the status is 5xx or a blocked code.
  - Error bodies sent as JSON are parsed to get their message.
- Scrapy `DOWNLOAD_MAXSIZE` and `DOWNLOAD_WARNSIZE` bound the cost of such downloads (pitfall 16).
- Google: crawlers stop at 15 MB by default, and Googlebot stops at 2 MB for supported types or 64 MB for PDFs.

**Sources.**
- https://crawlee.dev/js/api/http-crawler/interface/HttpCrawlerOptions
- https://github.com/apify/crawlee/blob/master/packages/http-crawler/src/internals/http-crawler.ts (read via raw.githubusercontent.com)
- https://developers.google.com/search/docs/crawling-indexing/googlebot

### 19. Character encoding: Content-Type charset, meta charset and BOM
**Failure mode.**
- A crawler that trusts only the `Content-Type` charset, or only `<meta charset>`, or defaults to UTF-8, produces mojibake on legacy pages.
- Libraries that implement "latin1" or "ascii" as true ISO-8859-1 or ASCII decode bytes 0x80–0x9F differently from browsers.
- A UTF-16 label in `<meta>` is a trap.

**What specs and leaders do.**
- HTML Living Standard, encoding sniffing algorithm, §13.2.3.2. In order:
  1. A BOM wins, with confidence certain.
  2. A user override.
  3. The transport-layer charset (Content-Type), if the encoding is supported, with confidence certain.
  4. A prescan for `<meta>`, which user agents are encouraged to limit to the first 1024 bytes. The result is tentative.
  5. The parent document's encoding, if same-origin.
  6. Encoding history, then optional frequency-based autodetection. The spec discourages autodetection for network resources, while noting that UTF-8 is very detectable.
  7. A locale-based default.
- A user agent may wait for more bytes before deciding, for example "500ms or 1024 bytes".
- A `<meta>` label of UTF-16BE/LE is treated as UTF-8, and x-user-defined as windows-1252.
- When a declaration found during parsing conflicts with the current encoding (§13.2.3.4), the user agent may switch decoders on the fly or re-parse from memory. If re-parsing would repeat a non-GET request, the new encoding is ignored.
- WHATWG Encoding Standard:
  - `decode` BOM-sniffs first. The BOM is "more authoritative than anything else", even though this violates the HTTP Content-Type semantics.
  - "latin1", "iso-8859-1" and "ascii" are labels for windows-1252. Byte 0x80 decodes to U+20AC (€).
  - Libraries exposing "Latin1" or "ASCII" APIs often disagree with this.
- Crawlee:
  - Uses the Content-Type charset.
  - On `master`, for HTML or XML with no charset, it reads the meta charset from the raw bytes, then falls back to `suggestResponseEncoding`, then UTF-8.
  - `forceResponseEncoding` overrides everything.
  - A source comment notes that error bodies are always decoded as UTF-8.

**Sources.**
- https://html.spec.whatwg.org/multipage/parsing.html#determining-the-character-encoding
- https://html.spec.whatwg.org/multipage/parsing.html#changing-the-encoding-while-parsing
- https://encoding.spec.whatwg.org/#decode
- https://encoding.spec.whatwg.org/#bom-sniff
- https://encoding.spec.whatwg.org/#names-and-labels
- https://crawlee.dev/js/api/http-crawler/interface/HttpCrawlerOptions
- https://github.com/apify/crawlee/blob/master/packages/http-crawler/src/internals/http-crawler.ts (read via raw.githubusercontent.com)

### 20. Soft 404s (200 OK for a missing page)
**Failure mode.** The server answers 200 for a page that does not exist, or for an empty or error page. A crawler that trusts the status code indexes junk and treats dead URLs as live. Logical redirects such as meta refresh or JavaScript are a related trap. Scrapy's own old `broad-crawls.html` is now a 200 response that meta-refreshes to `optimize.html`.

**What specs and leaders do.**
- Google:
  - A 2xx response whose content looks like an error, such as an empty page or an error message, is reported as a soft 404 in Search Console.
  - The Page indexing help page defines a soft 404 as a "not found" message without a 404 status. It recommends returning a real 404, or adding content if the page should exist, and checking with live URL inspection.
  - A 2xx "doesn't guarantee indexing".
  - For robots.txt, Google does not follow logical redirects (frames, JavaScript, meta refresh).

**Sources.**
- https://developers.google.com/search/docs/crawling-indexing/http-network-errors
- https://support.google.com/webmasters/answer/7440203
- https://developers.google.com/crawling/docs/robots-txt/robots-txt-spec
- https://docs.scrapy.org/en/latest/topics/broad-crawls.html (observed meta-refresh)
- Not fetched: https://developers.google.com/search/docs/crawling-indexing/soft-404-errors (HTTP 404)

### 21. 401, 403 and 451: blocked, forbidden, or legally unavailable
**Failure mode.**
- Treating 403 as "page gone" loses pages that were blocked only for this client. Many anti-bot systems answer 403, or 429.
- Retrying a 403 with the same credentials is pointless.
- 451 means legal blocking, which may be specific to one jurisdiction. It says nothing about whether the resource exists.
- Servers may also hide a forbidden resource behind a 404.

**What specs and leaders do.**
- RFC 9110 §15.5.4 (403):
  - The client SHOULD NOT repeat the request with the same credentials, but may try different ones.
  - The refusal may be unrelated to credentials.
  - A server MAY answer 404 to hide that the resource exists.
- RFC 7725 §3 (451):
  - The server is denying access "as a consequence of a legal demand", and may not be the origin.
  - The response SHOULD explain the demand, and may include a `Link` with `rel="blocked-by"`.
  - It implies neither existence nor non-existence of the resource.
  - It is cacheable by default.
- Crawlee: the default `blockedStatusCodes` is `[401, 403, 429]`. These retire the session. `retryOnBlocked` also checks HTML content for bot protection, currently Cloudflare and Google Search rate limiting.
- Google: 4xx codes other than 429 do not change crawl rate, and URLs that keep returning 4xx are dropped from the index. Google tells site owners not to use 401 or 403 to limit crawling.

**Sources.**
- https://www.rfc-editor.org/rfc/rfc9110.html#section-15.5.4
- https://www.rfc-editor.org/rfc/rfc7725.html#section-3
- https://crawlee.dev/js/api/core/interface/SessionPoolOptions
- https://crawlee.dev/js/api/http-crawler/interface/HttpCrawlerOptions
- https://developers.google.com/search/docs/crawling-indexing/http-network-errors

### 22. Conditional requests, 304 Not Modified and ETags
**Failure mode.**
- Re-crawls without validators waste bandwidth on unchanged pages.
- A client that sends validators but mishandles 304 replaces stored content with an empty body.
- Weak ETags compared with strong comparison never match.
- Some servers emit malformed Last-Modified dates.

**What specs and leaders do.**
- RFC 9110 §15.4.5: a 304 means the stored copy is valid and should be used "as if it were the content of a 200". A 304 MUST carry Content-Location, Date, ETag, Vary, Cache-Control and Expires if a 200 would.
- RFC 9110 §13.1.2: If-None-Match uses weak comparison.
- RFC 9111 §4.3.1: when validating, a cache MUST send entity tags if the stored response had them. It SHOULD send If-Modified-Since from Last-Modified when one stored response is being validated.
- RFC 9111 §4.2.2: heuristic freshness from Last-Modified, where "a typical setting" is 10% of the time since modification.
- Google's crawlers:
  - Support ETag/If-None-Match and Last-Modified/If-Modified-Since, and prefer ETag when both are present.
  - Treat `Cache-Control: max-age` as optional and ignore other directives.
  - Treat a 304 as content unchanged since the last crawl.
- Scrapy:
  - `HTTPCACHE_ENABLED` defaults to `False`, and the default policy is `DummyPolicy`.
  - The opt-in RFC2616 policy revalidates with Last-Modified and ETag.

**Sources.**
- https://www.rfc-editor.org/rfc/rfc9110.html#section-15.4.5
- https://www.rfc-editor.org/rfc/rfc9110.html#section-13.1.2
- https://www.rfc-editor.org/rfc/rfc9111.html#section-4.3.1
- https://www.rfc-editor.org/rfc/rfc9111.html#section-4.2.2
- https://developers.google.com/crawling/docs/crawlers-fetchers/overview-google-crawlers
- https://developers.google.com/search/docs/crawling-indexing/http-network-errors
- https://docs.scrapy.org/en/latest/topics/downloader-middleware.html#module-scrapy.downloadermiddlewares.httpcache

### 23. Vary and content negotiation (cache keys)
**Failure mode.**
- A crawler cache keyed only by URL serves a gzip, mobile or other-language variant to the wrong request.
- A response with `Vary: *` cannot be reused at all.
- Changing Accept-Language, User-Agent or cookies between fetches silently changes what is returned.

**What specs and leaders do.**
- RFC 9110 §12.5.5:
  - Vary names the request header fields that influenced the selected response.
  - `*` means other factors did, possibly including the client's network address.
  - Recipients MUST NOT reuse the response unless the listed fields match.
- RFC 9111 §4.1:
  - The cache key extends to the nominated headers, after whitespace and case normalization.
  - A header absent from the original request matches only another request where it is also absent.
  - `Vary: *` "always fails to match".
- Scrapy's RFC2616 cache policy lists Vary support as not implemented.

**Sources.**
- https://www.rfc-editor.org/rfc/rfc9110.html#section-12.5.5
- https://www.rfc-editor.org/rfc/rfc9111.html#section-4.1
- https://docs.scrapy.org/en/latest/topics/downloader-middleware.html#module-scrapy.downloadermiddlewares.httpcache

### 24. Cookies: tracking, limits, scope, and bans
**Failure mode.**
- Sites gate content behind cookie round-trips, such as consent cookies or bot-check tokens.
- Session cookies also link all of a crawler's requests together, which makes banning easier.
- Wrong scoping leaks cookies across sites. Examples: accepting a Domain attribute that is a public suffix, or ignoring the `__Host-` and `__Secure-` prefixes.
- Lenient parsing of oversized cookies or odd date formats diverges from browsers.

**What specs and leaders do.**
- RFC 6265bis (draft-22, December 2025, "In Final Review" at the RFC Editor, not yet an RFC):
  - Ignore a cookie whose name plus value exceeds 4096 octets.
  - Ignore attribute values longer than 1024 octets.
  - Cap Expires and Max-Age at 400 days.
  - User agents SHOULD support at least 50 cookies per domain and 3000 in total.
  - Many user agents reject Domain attributes that are public suffixes.
  - Two-digit years 70–99 become 19xx and 00–69 become 20xx.
- Scrapy's anti-ban advice: disable cookies (`COOKIES_ENABLED`) unless needed, so a session cookie does not link your requests.
- Crawlee: `persistCookiesPerSession` stores response cookies per session and resends them. It works only with the session pool, which holds up to `maxPoolSize`, default 1000.

**Sources.**
- https://www.ietf.org/archive/id/draft-ietf-httpbis-rfc6265bis-22.txt (sections 4.1.2.3, 4.1.3, 5.1.1, 5.5, 5.6, 6.1)
- https://datatracker.ietf.org/doc/draft-ietf-httpbis-rfc6265bis/
- https://docs.scrapy.org/en/latest/topics/practices.html#avoiding-getting-banned
- https://crawlee.dev/js/api/http-crawler/interface/HttpCrawlerOptions
- https://crawlee.dev/js/api/core/interface/SessionPoolOptions

### 25. Redirects: method changes, header leaks, loops and hop limits
**Failure mode.**
- POST becomes GET on 301 or 302, silently dropping the body.
- 307 and 308 must keep the method.
- Following a redirect to another host or scheme can leak `Authorization`, `Cookie` or `Proxy-Authorization`.
- Redirect loops or very long chains burn the request budget.
- Clients disagree on hop limits.

**What specs and leaders do.**
- RFC 9110:
  - On 301 and 302, a user agent MAY change POST to GET "for historical reasons" (§15.4.2, §15.4.3).
  - 303 leads to a GET or HEAD (§15.4.4).
  - 307 and 308 MUST NOT change the method (§15.4.8, §15.4.9). 308 is newer (2014) and "might not be recognized everywhere".
  - When following a redirect (§15.4), the client removes:
    - connection-specific, proxy and origin-specific headers;
    - cache validators;
    - resource-specific headers such as Referer, Origin, Authorization and Cookie.
  - If the method became GET or HEAD, the client also removes Content-Type, Content-Length and other content headers.
  - Clients SHOULD detect cyclical redirects. An older spec suggested a maximum of 5.
  - Retry-After on a 3xx sets a minimum wait (§10.2.3).
- Scrapy `RedirectMiddleware`:
  - `REDIRECT_MAX_TIMES` defaults to 20, and per-request `redirect_ttl` can only lower it.
  - Source: it switches to GET (dropping body, Content-Type and Content-Length) for POST on 301 or 302, and for non-GET/HEAD on 303.
  - It drops `Cookie` when the host changes or the scheme changes to something other than https.
  - It drops `Authorization` when the scheme, host or port changes.
  - It drops `Proxy-Authorization` when the scheme changes.
- urllib3 removes Authorization, Cookie and Proxy-Authorization on redirect by default.
- Google:
  - Googlebot follows up to 10 hops.
  - It treats 301 and 308 as strong signals, and 302, 303 and 307 as weak.
  - For robots.txt it follows at least 5 hops, then treats the file as a 404. RFC 9309 §2.3.1.2 says at least five.

**Sources.**
- https://www.rfc-editor.org/rfc/rfc9110.html#section-15.4
- https://www.rfc-editor.org/rfc/rfc9110.html#section-15.4.2
- https://www.rfc-editor.org/rfc/rfc9110.html#section-15.4.3
- https://www.rfc-editor.org/rfc/rfc9110.html#section-15.4.4
- https://www.rfc-editor.org/rfc/rfc9110.html#section-15.4.8
- https://www.rfc-editor.org/rfc/rfc9110.html#section-15.4.9
- https://docs.scrapy.org/en/latest/topics/downloader-middleware.html#redirect-max-times
- https://github.com/scrapy/scrapy/blob/master/scrapy/downloadermiddlewares/redirect.py (read via raw.githubusercontent.com)
- https://urllib3.readthedocs.io/en/stable/reference/urllib3.util.html
- https://developers.google.com/search/docs/crawling-indexing/http-network-errors
- https://www.rfc-editor.org/rfc/rfc9309.html#section-2.3.1.2
- https://developers.google.com/crawling/docs/robots-txt/robots-txt-spec

---

#### Points to check before relying on them
- Crawlee facts marked "`master`" come from source at package version 4.0.0 and may not be in a published release yet:
  - `ThrottlingRequestManager` and its Retry-After handling.
  - Meta-charset sniffing.
  - Treating non-HTML responses as transient on 5xx.
  - The `noRetry` flag on unsupported content types.
- Crawlee's docs pages show no default for `navigationTimeoutSecs`. The value 30 comes from the source schema.
- The Scrapy docs list `CONCURRENT_REQUESTS_PER_DOMAIN` as "1 (fallback: 8)" and `DOWNLOAD_DELAY` as "1 (fallback: 0)". The page does not explain what "fallback" means. It appears to separate the generated-project value from the built-in value, but this was not checked.
- The Scrapy docs do not say that RetryMiddleware honors Retry-After, and no statement either way was found.

## Part 3: pages and the browser

Researched 2026-10-16.

**How the sources were read.** Exa was out of credits, so pages were read with WebFetch or `curl`.

- Playwright: read from the docs' Markdown source at `https://raw.githubusercontent.com/microsoft/playwright/main/docs/src/...`, which is what `playwright.dev` renders. WebFetch was also used on `playwright.dev/docs/actionability`, `/docs/navigations`, `/docs/best-practices` and `/docs/api/class-page`. The URLs below are the published `playwright.dev` pages, with anchors derived from the headings.
- Puppeteer: read from `https://raw.githubusercontent.com/puppeteer/puppeteer/main/docs/...`, the source of `pptr.dev`.
- Selenium: read from `https://raw.githubusercontent.com/SeleniumHQ/seleniumhq.github.io/trunk/website_and_docs/content/documentation/...`, the source of `selenium.dev`.
- Crawlee: the `crawlee.dev` pages were read with WebFetch, and source files from the `apify/crawlee` `master` branch with `curl`.

Quotes are kept to 10 words or fewer.

---

### 1. SPA hydration: element visible before its handlers attach

- **Failure mode:** Server-rendered HTML shows up before the client JavaScript has attached listeners. A fast automation client clicks or types into a control that looks ready. The click does nothing, or the typed text is wiped when hydration finishes.
- **What the leaders do:**
  - Playwright's actionability checks are visible, stable, receives events, enabled and editable. They cannot detect missing listeners: Playwright says that if the button is enabled but listeners aren't attached, "the click won't have any effect".
  - Its fix belongs to the app: keep interactive controls disabled until hydration completes, so the Enabled check gates the action. To reproduce the problem, it suggests DevTools "Slow 3G" throttling.
  - For scrapers that can't change the site, the general Playwright practice is to wait for a web-first assertion on an outcome (for example, the expected content appears), not on the control.
- **Sources:**
  - https://playwright.dev/docs/navigations#hydration
  - https://playwright.dev/docs/actionability
  - https://playwright.dev/docs/best-practices

### 2. Client-side routing and BFCache: the URL changes without a normal navigation

- **Failure mode:** In SPAs, `history.pushState` or a hash change updates the URL with no new document. There is no response and no `load` or `domcontentloaded` for the new view, so "wait for navigation" logic either returns immediately with nothing or hangs. A single click can also trigger several navigations, such as redirect chains.
- **What the leaders do:**
  - Playwright treats a History API URL change as a navigation.
  - For a navigation to a different anchor or via the History API, `waitForNavigation` resolves with `null` instead of a response.
  - `waitForNavigation` is deprecated as "inherently racy". Use `page.waitForURL('**/target')`, which waits for the main frame to reach a given URL. It is recommended explicitly when a click "could trigger multiple navigations".
  - BFCache: Playwright disables BFCache by default in every browser. Even when enabled, testing BFCache restores is unsupported. A restore skips `commit`, `domcontentloaded` and `load`, so `goBack()` into BFCache leads to timeouts and a desynchronized Page object.
- **Sources:**
  - https://playwright.dev/docs/navigations#waiting-for-navigation
  - https://playwright.dev/docs/navigations#backforward-cache-bfcache
  - https://playwright.dev/docs/api/class-frame#frame-wait-for-navigation
  - https://playwright.dev/docs/api/class-page#page-wait-for-url

### 3. "Network idle" is an unreliable readiness signal

- **Failure mode:** Pages with long polling, WebSockets, analytics beacons or streaming may never reach zero connections, so network-idle waits time out. Pages whose content renders after an idle gap pass the wait too early.
- **What the leaders do:**
  - Playwright marks `waitUntil: 'networkidle'` as **DISCOURAGED**. It means no connections for at least 500 ms, and the docs add: "Don't use this method for testing". Rely on web assertions instead.
  - `page.goto` defaults to `waitUntil: 'load'`. The other options are `domcontentloaded`, `commit` (response received, document started loading) and `networkidle`.
  - Puppeteer still offers two thresholds:
    - `networkidle0`: no more than 0 connections for at least 500 ms.
    - `networkidle2`: no more than 2 connections for at least 500 ms. This tolerates a couple of persistent connections.
  - `page.waitForNetworkIdle` defaults to `idleTime` 500 ms and `concurrency` 0, and always waits at least `idleTime`.
  - Selenium navigation commands wait for `document.readyState` per the page load strategy, `"complete"` by default. Selenium notes that readyState covers only assets defined in the HTML, so JavaScript-added elements may not be present yet.
- **Sources:**
  - https://playwright.dev/docs/api/class-page#page-goto (shared `waitUntil` param text, from `docs/src/api/params.md`)
  - https://pptr.dev/api/puppeteer.puppeteerlifecycleevent
  - https://pptr.dev/api/puppeteer.page.waitfornetworkidle
  - https://pptr.dev/api/puppeteer.waitfornetworkidleoptions
  - https://www.selenium.dev/documentation/webdriver/waits/

### 4. Race conditions, fixed sleeps and mixed waiting strategies

- **Failure mode:** The script runs ahead of the page, so the element isn't there yet: a race. Fixed sleeps are either too short, which is flaky, or too long, which makes the session very slow. Mixing wait types makes timeouts unpredictable.
- **What the leaders do:**
  - Selenium calls races "one of the primary causes" of flaky tests and rejects sleeps for exactly the too-short/too-long reason. Its options:
    - Implicit wait: global and per session, default `0`.
    - Explicit wait: polls a condition. The polling interval, ignored exceptions, timeout and message are all configurable.
  - Selenium also warns: "Do not mix implicit and explicit waits." Its example is a 10 s implicit wait plus a 15 s explicit wait, which can time out only after 20 s.
  - Selenium session defaults: script timeout 30,000 ms, page load timeout 300,000 ms, implicit wait 0.
  - Playwright locators auto-wait and retry. Web-first assertions such as `toBeVisible()` wait for the condition, while one-shot checks like `isVisible()` don't wait at all. When an action's checks don't pass in time, Playwright throws `TimeoutError`.
  - Puppeteer locators wait automatically. The default `WaitTimeoutOptions.timeout` is `30_000` ms, can be changed with `page.setDefaultTimeout()` and can be overridden per locator with `.setTimeout()`.
  - Playwright Test labels a test "flaky" if it fails and then passes on retry. On any failure it discards the whole worker process and browser.
- **Sources:**
  - https://www.selenium.dev/documentation/webdriver/waits/
  - https://www.selenium.dev/documentation/webdriver/drivers/options/#timeouts
  - https://playwright.dev/docs/best-practices
  - https://playwright.dev/docs/actionability
  - https://pptr.dev/guides/page-interactions#locator-timeouts
  - https://pptr.dev/api/puppeteer.waittimeoutoptions
  - https://playwright.dev/docs/test-retries

### 5. Stale or detached element references after a re-render

- **Failure mode:** A handle to a DOM node becomes invalid once the framework re-renders that subtree, the page refreshes or navigates, or the context switches between frame and window. Actions on the old reference throw, or hit a node that is no longer shown.
- **What the leaders do:**
  - Selenium throws `StaleElementReferenceException` because element references are not relocated automatically. Its fixes:
    - Relocate before each use, or wrap the element with its locator and retry on staleness. Selenium warns the locator might then match a different element.
    - Switch back to the right frame or window when the context changed.
    - Navigate back and relocate when the page changed.
  - Selenium's shadow-root equivalent is `DetachedShadowRootException`.
  - Playwright locators are lazy: every action resolves an up-to-date element, so a re-rendered node is picked up automatically. Locators are also strict and throw if more than one element matches.
  - Playwright discourages APIs that don't wait for actionability, such as `page.$eval`.
- **Sources:**
  - https://www.selenium.dev/documentation/webdriver/troubleshooting/errors/#staleelementreferenceexception
  - https://www.selenium.dev/documentation/webdriver/troubleshooting/errors/#detachedshadowrootexception
  - https://playwright.dev/docs/locators#strictness (lazy re-resolution is in the locators intro)
  - https://playwright.dev/docs/api/class-page#page-eval-on-selector

### 6. Animations, transitions and late web fonts make targets move

- **Failure mode:** The element's box is still moving or resizing when the click is computed, so the click lands on the wrong spot or another element. Late-loading fonts reflow text and shift layout in the same way.
- **What the leaders do:**
  - Playwright's Stable check requires the "same bounding box for at least two consecutive animation frames". Click, check, hover, drag and screenshot all require it.
  - Puppeteer locators wait for a stable bounding box over two consecutive animation frames, check visibility and enabled state, and scroll the element into the viewport.
  - Playwright screenshots can use `animations: 'disabled'`, which fast-forwards finite animations and cancels infinite ones. The default is `allow` for `page.screenshot` and `disabled` for the visual-comparison variant. The caret is hidden by default.
  - Selenium recommends waiting for animations to cease. It lists animations as a cause of `ElementClickInterceptedException`.
  - Web fonts: `document.fonts.ready` resolves once font loading and layout are done and no further font loads are needed. That is a usable stability gate, though I found no leader that waits on it by default.
- **Sources:**
  - https://playwright.dev/docs/actionability
  - https://pptr.dev/guides/page-interactions#clicking-an-element-using-locators
  - https://playwright.dev/docs/api/class-page#page-screenshot (shared `animations`/`caret` param text)
  - https://www.selenium.dev/documentation/webdriver/troubleshooting/errors/#elementclickinterceptedexception
  - https://developer.mozilla.org/en-US/docs/Web/API/FontFaceSet/ready

### 7. Another element receives the click (sticky headers, modals, overlaps)

- **Failure mode:** The target is visible and enabled but covered by something else, such as a sticky navbar, a modal backdrop or a toast. The click goes to the covering element, or the driver refuses it.
- **What the leaders do:**
  - Playwright's Receives Events check hit-tests the action point, for example whether an overlay would catch a click at (10,10). It retries until timeout. `force: true` skips the non-essential checks.
  - Selenium checks visible, unobscured and enabled before clicking, and otherwise throws `ElementClickInterceptedException`. Its fixes are explicit waits (`elementToBeClickable`) and scrolling the element out from under fixed bars, with `scrollBy` or `Actions.moveToElement`.
- **Sources:**
  - https://playwright.dev/docs/actionability
  - https://www.selenium.dev/documentation/webdriver/troubleshooting/errors/#elementclickinterceptedexception

### 8. Unpredictable overlays: cookie-consent banners, sign-up modals

- **Failure mode:** Consent banners and marketing modals appear at random times, or only in some regions or sessions. They block clicks and hide content, so scripts written against a clean page fail intermittently.
- **What the leaders do:**
  - Playwright's `page.addLocatorHandler(locator, handler)` (v1.42) runs before every actionability-checked action or retry, and before every auto-waiting assertion:
    - If the overlay is visible, the handler runs first. Playwright then waits until the overlay is hidden unless `noWaitAfter` is set.
    - `times` is unlimited by default. Only one handler runs at a time, and handler time counts toward the action's timeout.
    - The handler changes focus and mouse position, so focus-then-`keyboard.press` sequences can break. Use `locator.press` instead.
    - For overlays that appear predictably, Playwright recommends handling them explicitly in the flow.
  - Crawlee's cookie-modal guide (docs/next) lists five approaches in pre- and post-navigation hooks:
    1. Set the consent cookie. This is site-specific and breaks silently.
    2. Click the button with a short timeout and swallow the error. Otherwise the timeout fails the request and consumes a retry.
    3. Inject `@duckduckgo/autoconsent` with `addInitScript`. It has rules for more than 300 consent platforms, adds 1–2 s per page and a bundle of about 400 kB, and opts out rather than accepting.
    4. Block the banner with `@ghostery/adblocker-playwright` (Fanboy's Cookie List). No consent is recorded, so consent-gated content stays gated.
    5. Ask an LLM through Stagehand. This is slowest and costs money per call.

    The guide recommends autoconsent when crawling many sites.
  - Crawlee also lists `playwrightUtils.closeCookieModals(page)` in its API index, with only a signature and no description. I did not find the function in the current `master` `playwright-utils.ts`, so its mechanism is unverified.
- **Sources:**
  - https://playwright.dev/docs/api/class-page#page-add-locator-handler
  - https://crawlee.dev/js/docs/next/guides/cookie-modals
  - https://crawlee.dev/js/api/playwright-crawler/namespace/playwrightUtils#closeCookieModals

### 9. Lazy loading and IntersectionObserver content

- **Failure mode:** Images, iframes and content blocks load only when they come near the viewport. A scraper that never scrolls, or uses a small viewport, captures placeholders or misses content.
- **What the leaders do:**
  - Chromium's native `loading="lazy"` uses fixed distance-from-viewport thresholds. They are 1250 px on fast connections (4G), reduced from 3000 px, and 2500 px on 3G or slower, reduced from 4000 px. They "can't be changed through the API".
  - Deferral happens only when JavaScript is enabled.
  - `display:none` images are not loaded, while `opacity:0` images still are.
  - Images without dimensions default to 0×0, so the browser may treat many of them as in-viewport and load them all.
  - `<iframe loading=lazy>` is standardized too.
  - IntersectionObserver is the JavaScript mechanism behind custom lazy loading and infinite scroll. Its callback fires when the target crosses a threshold of the root (the viewport by default). A positive `rootMargin` makes content start loading before it becomes visible.
  - Automation therefore has to really scroll, or position elements in view. Playwright auto-scrolls targets into view before actions.
- **Sources:**
  - https://web.dev/articles/browser-level-image-lazy-loading#distance-from-viewport
  - https://developer.mozilla.org/en-US/docs/Web/API/Intersection_Observer_API
  - https://playwright.dev/docs/input#scrolling

### 10. Infinite scroll

- **Failure mode:** More items load only as the user scrolls, often over XHR, fetch or WebSocket. Unless the page really scrolls and the script waits for new requests, the scraper gets only the first batch. Some feeds never end, and some need a "load more" button or a scroll back up.
- **What the leaders do:**
  - Crawlee's `infiniteScroll(page, options)` has these options:

    | Option | Default | Meaning |
    | --- | --- | --- |
    | `timeoutSecs` | 0 | No time limit. |
    | `maxScrollHeight` | 0 | No pixel limit. |
    | `waitForSecs` | 4 | Seconds without new requests before it stops. |
    | `scrollDownAndUp` | false | Scrolls back up a little after each scroll down. |
    | `buttonSelector` | none | A "load more" button to click while scrolling. |
    | `stopScrollCallback` | none | A custom function that ends scrolling. |

    In the source, it scrolls by `document.body.scrollHeight` with `mouse.wheel` (10000 px if the height is 0) and pauses 250 ms between scrolls. Each second it counts new `xhr`, `fetch`, `websocket` and `other` requests, and stops after `waitForSecs` consecutive seconds with no new ones.
  - Playwright recommends scrolling a bottom element such as the footer into view (`scrollIntoViewIfNeeded`) to trigger loading.
  - Puppeteer `locator.scroll()` uses mouse-wheel events.
- **Sources:**
  - https://crawlee.dev/js/api/playwright-crawler/namespace/playwrightUtils#infiniteScroll
  - https://github.com/apify/crawlee/blob/master/packages/playwright-crawler/src/internals/utils/playwright-utils.ts
  - https://playwright.dev/docs/input#scrolling
  - https://pptr.dev/guides/page-interactions#scroll-an-element

### 11. Virtualized (windowed) lists: only the visible rows exist in the DOM

- **Failure mode:** Large tables and lists render only the rows in view and recycle DOM nodes as the user scrolls. A DOM snapshot or `querySelectorAll` returns a small window of the data, and rows scrolled out of view are removed.
- **What the leaders do:**
  - TanStack Virtual, a widely used virtualizer, renders "only the visible items" and positions them by hand.
  - None of the automation docs I read has a dedicated virtual-list helper. The workable pattern combines pitfalls 10 and 5:
    - Scroll step by step and collect rows at each step, deduplicating by key.
    - Re-resolve locators after each scroll, because recycled nodes make old handles stale.

  This is my inference, not a documented default.
- **Sources:**
  - https://tanstack.com/virtual/latest/docs/introduction
  - https://playwright.dev/docs/input#scrolling

### 12. iframes, cross-origin and out-of-process iframes (OOPIF)

- **Failure mode:** Content in an `<iframe>` lives in a separate document. Page-level queries and injected scripts don't reach it. With Site Isolation, Chromium renders cross-site iframes in a different renderer process.
- **What the leaders do:**
  - Playwright gives every page a main frame. Page-level actions target the main frame, and child frames are reached with `page.frameLocator(...)`, `page.frame({name|url})` or `frame.childFrames()`. `Frame.isDetached()` reports whether a frame was removed.
  - Puppeteer exposes the frame tree (`mainFrame()`, `childFrames()`) and the `frameattached`, `framenavigated` and `framedetached` events.
  - Chromium design doc: OOPIFs exist for Site Isolation and dedicate a renderer to a single site, even when cross-site iframes are present. Features that iterated frames within one process had to change.
  - At the CDP level, `Target.setAutoAttach` auto-attaches to related targets such as iframes or workers. `waitForDebuggerOnStart` pauses each new target until `Runtime.runIfWaitingForDebugger`, so you can configure it before it runs. `flatten` enables `sessionId`-addressed sessions.
  - Selenium's stale-element guidance covers the frame side: an element reference is valid only while the driver is switched into its frame.
- **Sources:**
  - https://playwright.dev/docs/frames
  - https://playwright.dev/docs/api/class-frame#frame-is-detached
  - https://pptr.dev/api/puppeteer.frame
  - https://www.chromium.org/developers/design-documents/oop-iframes/
  - https://chromedevtools.github.io/devtools-protocol/tot/Target/#method-setAutoAttach (read via `https://raw.githubusercontent.com/ChromeDevTools/devtools-protocol/master/pdl/domains/Target.pdl`; the HTML page only redirected)
  - https://www.selenium.dev/documentation/webdriver/troubleshooting/errors/#staleelementreferenceexception

### 13. Shadow DOM (open vs closed) and `<template>` content

- **Failure mode:** Plain CSS selectors and XPath stop at shadow boundaries. Closed shadow roots are hidden from page JavaScript. `<template>` content is inert: it isn't rendered and isn't a child of the element (`childNodes` is always empty). Scripts that query the DOM therefore miss content or match the wrong nodes.
- **What the leaders do:**
  - Playwright locators pierce shadow DOM by default. The exceptions are XPath, which does not pierce shadow roots, and closed-mode shadow roots, which are unsupported.
  - Puppeteer adds the `>>>` (deep descendant) and `>>>>` (deep child) combinators and `pierce/`. These work only on open shadow roots and the first "depth" of a selector.
  - Selenium needs explicit shadow-root access. It throws `DetachedShadowRootException` when the host is replaced.
  - Template content lives in `template.content`, a `DocumentFragment`.
  - Declarative shadow DOM: a `<template shadowrootmode="open|closed">` is replaced at parse time by a ShadowRoot on its parent. `closed` hides that root from JavaScript.
- **Sources:**
  - https://playwright.dev/docs/locators#locate-in-shadow-dom
  - https://pptr.dev/guides/page-interactions#querying-elements-in-shadow-dom
  - https://www.selenium.dev/documentation/webdriver/troubleshooting/errors/#detachedshadowrootexception
  - https://developer.mozilla.org/en-US/docs/Web/HTML/Reference/Elements/template#declarative_shadow_dom

### 14. Canvas- and WebGL-rendered content is not in the DOM

- **Failure mode:** Charts, maps, games and some editors draw pixels to `<canvas>`. No text or element nodes exist to locate, read or click.
- **What the leaders do:**
  - MDN: a canvas "is just a bitmap" and is not exposed to accessibility tools. So role- and text-based locators, and accessibility-tree extraction, find nothing.
  - The only in-DOM fallback is alternative content inside `<canvas>`. It is shown only when canvas is unsupported or JavaScript is disabled.
  - None of the automation docs I read has a canvas helper. What's left is screenshots plus coordinate clicks, or reading the app's data from its network responses. This is my inference.
- **Sources:**
  - https://developer.mozilla.org/en-US/docs/Web/HTML/Reference/Elements/canvas#accessibility

### 15. Popups and new tabs

- **Failure mode:** `target="_blank"` links and `window.open` create a new page object. A script that keeps acting on the original page misses it. A listener attached after the click can miss the event.
- **What the leaders do:**
  - Playwright: start waiting before the click (`waitForEvent('popup')` or `context.waitForEvent('page')`), then click, then await.
  - The popup page is available only once it has navigated to its initial URL, after that response has started loading. To intercept the popup's first request, use `BrowserContext.route` or the context `request` event, not the page's.
  - Puppeteer emits `popup` with the new Page.
- **Sources:**
  - https://playwright.dev/docs/pages#handling-new-pages
  - https://playwright.dev/docs/api/class-page#page-event-popup
  - https://pptr.dev/api/puppeteer.pageevent

### 16. JavaScript dialogs (alert, confirm, prompt, beforeunload) block the page

- **Failure mode:** Dialogs are modal and block page execution, so the next action hangs or a command errors.
- **What the leaders do:**
  - Playwright auto-dismisses all dialogs when no `dialog` listener is registered. If a listener is registered, it **must** accept or dismiss, or the action "will stall". Logging the message alone hangs the click.
  - A `beforeunload` prompt appears only with `page.close({ runBeforeUnload: true })`, and that close call doesn't wait for the page to actually close.
  - Selenium's `unhandledPromptBehavior` defaults to "dismiss and notify". An open dialog blocks the next command with `UnhandledAlertException`. For `beforeunload`, ChromeDriver needs `ignore` plus BiDi to keep the prompt open.
  - Puppeteer emits `dialog` for alert, prompt, confirm and beforeunload, handled with `accept()` or `dismiss()`.
- **Sources:**
  - https://playwright.dev/docs/dialogs#alert-confirm-prompt-dialogs
  - https://playwright.dev/docs/dialogs#beforeunload-dialog
  - https://playwright.dev/docs/api/class-page#page-event-dialog
  - https://www.selenium.dev/documentation/webdriver/drivers/options/#unhandledpromptbehavior
  - https://www.selenium.dev/documentation/webdriver/troubleshooting/errors/#unhandledalertexception
  - https://pptr.dev/api/puppeteer.pageevent

### 17. Locale, time zone and geolocation change what the page shows

- **Failure mode:** Prices, dates, number formats, language, region-gated content and consent banners all depend on locale, time zone and location. A worker with a different system locale or time zone produces different, non-reproducible output.
- **What the leaders do:**
  - Playwright exposes these as context options:
    - `locale` (for example `de-DE`) sets `navigator.language`, the `Accept-Language` header and number/date formatting. It defaults to the system locale.
    - `timezoneId` (ICU IDs) defaults to the system time zone.
    - `geolocation` requires granting the `geolocation` permission.
  - These options affect only the browser, not the test runner's time zone. Pin them explicitly for reproducibility.
- **Sources:**
  - https://playwright.dev/docs/emulation#locale--timezone
  - https://playwright.dev/docs/emulation#geolocation
  - https://playwright.dev/docs/api/class-browser#browser-new-context-option-locale
  - https://playwright.dev/docs/api/class-browser#browser-new-context-option-timezone-id

### 18. Viewport and responsive-layout differences

- **Failure mode:** Breakpoints change markup: a hamburger menu instead of a nav bar, hidden columns, different lazy-load reach. Defaults differ between tools, so the same script sees different DOMs.
- **What the leaders do:**
  - Playwright's default viewport is 1280×720 for each page. `null` disables viewport emulation. Device descriptors set user agent, screen size, viewport and `hasTouch`, and `isMobile` controls whether the meta viewport tag is honoured.
  - Puppeteer's `defaultViewport` is 800×600.
  - Headless Chrome screens can be configured with `--screen-info`.
- **Sources:**
  - https://playwright.dev/docs/api/class-browser#browser-new-context-option-viewport
  - https://playwright.dev/docs/emulation#viewport
  - https://playwright.dev/docs/emulation#ismobile
  - https://pptr.dev/api/puppeteer.connectoptions
  - https://pptr.dev/guides/screen-configuration

### 19. Print and media emulation

- **Failure mode:** PDFs render with print CSS, so content differs from the screen version. Emulated `prefers-color-scheme` or reduced motion changes styles and animations.
- **What the leaders do:**
  - Playwright `page.pdf()` uses `print` media by default. Call `page.emulateMedia({ media: 'screen' })` first for a screen-styled PDF.
  - `emulateMedia` accepts `screen`, `print` or `null` (which disables emulation), plus `colorScheme` and `reducedMotion`.
- **Sources:**
  - https://playwright.dev/docs/api/class-page#page-pdf
  - https://playwright.dev/docs/api/class-page#page-emulate-media

### 20. Headless vs headful differences

- **Failure mode:** The old headless Chrome was a separate implementation. It behaved differently from real Chrome in features and fidelity, and in how sites detect it.
- **What the leaders do:**
  - Chrome now has "unified Headless and headful modes": `--headless` is real Chrome without visible windows. Since Chrome 132 the old mode ships only as the standalone `chrome-headless-shell`, which Chrome calls lighter and faster but less authentic.
  - Puppeteer defaults to new headless since v22. `headless: 'shell'` selects the old shell, which doesn't fully match Chrome's behaviour, and the shell needs `--enable-gpu` for GPU compositing.
  - Playwright's default headless Chromium is the headless shell. `channel: 'chromium'` opts into new headless.
- **Sources:**
  - https://developer.chrome.com/docs/chromium/headless
  - https://developer.chrome.com/blog/chrome-headless-shell
  - https://pptr.dev/guides/headless-modes
  - https://pptr.dev/troubleshooting#chrome-headless-shell-disables-gpu-compositing
  - https://playwright.dev/docs/browsers#chromium-new-headless-mode

### 21. Service workers cache or intercept requests

- **Failure mode:** A site's service worker answers fetches from its cache or rewrites them. The automation's request interception and network logging then miss traffic, and repeat visits serve stale content.
- **What the leaders do:**
  - Playwright's `serviceWorkers` context option defaults to `'allow'`. `'block'` blocks all service-worker registration.
  - The docs say to set `'block'` when network events seem to be missing under `route`. Tools like MSW install their own service worker, which hides requests from `page.route`.
- **Sources:**
  - https://playwright.dev/docs/api/class-browser#browser-new-context-option-service-workers
  - https://playwright.dev/docs/network#missing-network-events-and-service-workers

### 22. Downloads and file uploads

- **Failure mode:**
  - Downloads: they happen outside the DOM, may be cancelled, and their files may be cleaned up automatically. A script that doesn't wait for completion, or saves too late, loses the file.
  - Uploads: native OS file pickers can't be driven, and dynamically created inputs are hard to target.
- **What the leaders do:**
  - Playwright downloads:
    - `acceptDownloads` defaults to `true`, and files go to a temporary folder.
    - Downloaded files are deleted when the browser context that produced them closes.
    - Start waiting for the `download` event before the click. The event fires when the download starts, and the path is available only on completion. `download.path()` throws for failed or cancelled downloads and when connected remotely. `failure()` reports errors.
  - Selenium discourages testing downloads through the browser, because the API doesn't expose download progress. Fetch the link, with any cookies, over HTTP instead.
  - Uploads:
    - Playwright: `locator.setInputFiles()` sets an `<input type=file>` directly. For dynamically created inputs, wait for the `filechooser` event and call `setFiles`.
    - Selenium: `sendKeys(path)` on the file input, because it can't drive the OS dialog.
- **Sources:**
  - https://playwright.dev/docs/downloads
  - https://playwright.dev/docs/api/class-download
  - https://playwright.dev/docs/api/class-browser#browser-new-context-option-accept-downloads
  - https://playwright.dev/docs/input#upload-files
  - https://www.selenium.dev/documentation/test_practices/discouraged/file_downloads/
  - https://www.selenium.dev/documentation/webdriver/elements/file_upload/

### 23. Page crashes and "target closed" errors

- **Failure mode:** A renderer can crash, typically on memory exhaustion, or the browser can disconnect. In-flight and later operations on that page throw, and the work unit has to be retried in a fresh page.
- **What the leaders do:**
  - Playwright emits `page.on('crash')`. Pages crash when they try to allocate too much memory, and ongoing and later operations throw. Catch the exception: its message contains "crash".
  - `browser.on('disconnected')` fires when the browser closes or crashes, or `close()` is called.
  - Puppeteer emits `error` on page crash, which is different from `pageerror` for uncaught in-page exceptions. It also emits `close` and `framedetached`.
  - Playwright Test throws away the whole worker and browser after any failure, then retries in a new worker.
- **Sources:**
  - https://playwright.dev/docs/api/class-page#page-event-crash
  - https://playwright.dev/docs/api/class-browser#browser-event-disconnected
  - https://pptr.dev/api/puppeteer.pageevent
  - https://playwright.dev/docs/test-retries
  - Crawlee's crawler-level crash handling: not found in the pages I read (not fetched).

### 24. Memory bloat in long-running browser sessions

- **Failure mode:** A browser that has processed many pages grows in memory, slows down and eventually crashes pages (pitfall 23).
- **What the leaders do:**
  - Crawlee's `BrowserPool` retires browsers on these defaults:

    | Option | Default | Effect |
    | --- | --- | --- |
    | `retireBrowserAfterPageCount` | 100 | Retires a browser after this many pages. The docs say browsers "tend to get bloated". |
    | `maxOpenPagesPerBrowser` | 20 | Pages beyond this open in a new browser. |
    | `retireInactiveBrowserAfterSecs` | 10 | How often it checks for inactive browsers to retire. |
    | `closeInactiveBrowserAfterSecs` | 300 | Closes inactive browsers. |
    | `operationTimeoutSecs` | 15 | Timeout on launch and new-page operations, so they don't hang. |

    A retired session also retires its browser early. In the source, a sweep runs every 10 s, and pages that don't close within 5 s get a 1 s kill timeout.
  - Playwright's `page.requests()` lets old requests be garbage-collected to "prevent unbounded memory growth".
- **Sources:**
  - https://crawlee.dev/js/api/browser-pool/interface/BrowserPoolOptions
  - https://github.com/apify/crawlee/blob/master/packages/browser-pool/src/browser-pool.ts
  - https://playwright.dev/docs/api/class-page#page-requests

### 25. Zombie browser processes and container resource limits

- **Failure mode:** In containers, the Node or Python process often runs as PID 1. It doesn't reap exited Chrome children, so zombies pile up. Docker's small default shared memory makes Chromium run out of memory and crash.
- **What the leaders do:**
  - Playwright's Docker guidance:
    - Run with `--init` to avoid PID 1 special treatment, "a common reason for zombie processes".
    - Run with `--ipc=host` for Chromium, which can otherwise run out of memory and crash.
    - Use a seccomp profile (or `--cap-add=SYS_ADMIN` locally) for the sandbox.
  - Puppeteer's Docker guidance: `docker run --init` or `dumb-init` as PID 1 for zombie Chrome processes, a non-root user to avoid `--no-sandbox`, and `SYS_ADMIN` for its image.
  - The current Puppeteer troubleshooting page no longer mentions `/dev/shm` or `--disable-dev-shm-usage`. I did not find the advice there.
- **Sources:**
  - https://playwright.dev/docs/docker#recommended-docker-configuration
  - https://pptr.dev/troubleshooting#running-puppeteer-in-docker
  - https://pptr.dev/troubleshooting#tips
  - https://pptr.dev/guides/docker

---

#### Sources I tried but could not read, or that lacked the expected content

- Playwright `/docs/service-workers-experimental-network-events`: no doc-source file under that name (404). The service-worker guidance above comes from `network.md` and the `serviceWorkers` option.
- Crawlee `closeCookieModals`: the API page gives only a signature, and the function was not found in the current `master` source, so its mechanism is not verified.
- Crawlee page-crash handling: not fetched or not found.
- Puppeteer `/dev/shm` advice: absent from the current troubleshooting page.
- The CDP Target HTML page only redirected. Its content was read from the protocol PDL source instead.

## Part 4: sites and anti-bot (A), long runs and operations (B), data (C)

Read on 2026-10-15 and 2026-10-16. Every behaviour below was read on the cited page unless marked "observed" (checked directly during this research) or "not fetched". Quotes are at most 10 words per source; the rest is paraphrase. Defaults are those of the current "latest" docs at the time of reading (Scrapy docs mention 2.19.0 features).

Exa was out of credits, so pages were read with WebFetch (and one with curl).

---

### A. Sites and anti-bot

#### A1. A challenge or block page is taken for content
**Failure mode.** The anti-bot layer answers with an HTML interstitial in place of the requested resource. A crawler that only checks "got a response" stores the challenge page as data, or a JSON/XHR client fails to parse it.
**What leaders do.**
- Cloudflare puts the header `cf-mitigated: challenge` on every Challenge Page type, and "challenge" is its only valid value. A challenge response is always `text/html`, whatever resource type was requested. A client can detect it by reading that header on the fetch/XHR response.
- Cloudflare's error-page reference lists these status codes: Managed challenge / I'm Under Attack Mode 403, IP/country block 403, WAF block 403, rate-limiting block 429. So a challenge is not a 200, but its body is a full HTML page.
- Cloudflare says challenge pages fail when the browser expects non-HTML (AJAX/fetch). For API calls it offers Turnstile pre-clearance (a `cf_clearance` cookie issued on an HTML page first).
- Crawlee: with the browser crawlers' `retryOnBlocked` on, the crawler tries to get past detected bot protection. The documented detections are Cloudflare Bot Management and Google Search rate limiting. Crawlee's Camoufox guide adds a `handleCloudflareChallenge` helper in `postNavigationHooks` and warns it may not pass the challenge on its own.
- Scrapy: `HttpErrorMiddleware` sends non-2xx responses to the errback by default, not to the callback. A 403 challenge therefore never reaches parsing, but a 200 stub does (see A2).
**Sources.**
- https://developers.cloudflare.com/cloudflare-challenges/challenge-types/challenge-pages/detect-response/
- https://developers.cloudflare.com/cloudflare-challenges/challenge-types/challenge-pages/
- https://developers.cloudflare.com/rules/custom-errors/reference/error-page-types/
- https://crawlee.dev/js/api/browser-crawler/interface/BrowserCrawlerOptions (retryOnBlocked)
- https://crawlee.dev/js/docs/guides/avoid-blocking (Camoufox section)
- https://docs.scrapy.org/en/latest/topics/request-response.html#scrapy.Request.callback

#### A2. Soft 404s and HTTP-200 stubs
**Failure mode.** The server returns 200, but the body is an error message, an empty page, or a client-side redirect stub. Status-code checks call it success, and the stored record is junk.
**What leaders do.**
- Google defines a soft 404 as a page that returns `200 (success)` while its content looks like an error, such as an empty page or an error message. Google still processes the 200 content and flags it in Search Console as a soft 404. A 2xx status does not guarantee indexing.
- Observed during this research: `https://docs.scrapy.org/en/latest/topics/broad-crawls.html` returned HTTP/2 200 with a 645-byte body. The body held only a JS `location.replace` and a `<noscript>` meta refresh to `optimize.html#broad-crawls`. Scrapy's `METAREFRESH_IGNORE_TAGS` defaults to `["noscript"]` (changed from `[]` in 2.11.2), so Scrapy would not follow that meta refresh and would keep the stub. An HTTP fetcher without JS has the same problem.
- No crawler doc I read ships a generic soft-404 detector. The usual guards are the content checks in C2 (item counts, field coverage).
**Sources.**
- https://developers.google.com/search/docs/crawling-indexing/http-network-errors
- https://docs.scrapy.org/en/latest/topics/downloader-middleware.html#scrapy.downloadermiddlewares.redirect.MetaRefreshMiddleware
- Observed: `curl -sL https://docs.scrapy.org/en/latest/topics/broad-crawls.html`

#### A3. Honeypot links and labyrinths of fake content
**Failure mode.** The site injects links that humans never see. A crawler that follows them gets flagged, or it walks an endless maze of generated pages and wastes its budget on fake data.
**What leaders do.**
- Cloudflare AI Labyrinth adds invisible links with `Nofollow` tags that only bots see. A crawler that follows them is kept in "a maze of never-ending links". Cloudflare records the crawler's details and does not block or challenge those requests. Compliant bots that honour no-crawl instructions skip the honeypot, and the docs say SEO and page appearance are not affected.
- The implied defence: honour `nofollow` and robots rules, and cap depth and page budgets. Heritrix `CrawlLimitEnforcer` and Scrapy `DEPTH_LIMIT` and `CLOSESPIDER_PAGECOUNT` all default to 0 (no limit).
**Sources.**
- https://developers.cloudflare.com/bots/additional-configurations/ai-labyrinth/
- https://heritrix.readthedocs.io/en/stable/bean-reference.html (CrawlLimitEnforcer)
- https://docs.scrapy.org/en/latest/topics/settings.html (DEPTH_LIMIT)

#### A4. Rate-based bans and impolite pacing
**Failure mode.** Too many requests per host or per IP lead to 429/503 responses, then a ban. A crawler that retries immediately makes the ban worse.
**What leaders do.**
- Scrapy defaults: `DOWNLOAD_DELAY` 1 s per domain, randomised by `DOWNLOAD_DELAY_JITTER` 0.5; `CONCURRENT_REQUESTS_PER_DOMAIN` 1. RetryMiddleware retries `[500, 502, 503, 504, 522, 524, 408, 429]` `RETRY_TIMES` = 2 times, with priority adjusted by −1. Scrapy's ban advice is a delay of 2 s or more.
- Scrapy AutoThrottle (off by default): start delay 5 s, max 60 s, target concurrency 1.0. The latency of a non-200 response may raise the delay but never lower it.
- Scrapy's optimisation page says to raise concurrency gradually and to watch for 429/503, retries or rising latency.
- Heritrix `DispositionProcessor` waits `delayFactor` 5 × the last fetch duration, clamped between `minDelayMs` 3000 and `maxDelayMs` 30000. It honours robots `Crawl-Delay` up to `respectCrawlDelayUpToSeconds` 300. `BdbFrontier.retryDelaySeconds` is 900.
- Google treats 429 as a server error. 5xx and 429 make Google's crawlers slow down for a while, and the crawl rate rises again gradually once 2xx responses return.
- Spidermon's `UnwantedHTTPCodesMonitor` alerts after 10 responses with codes in `[400, 407, 429, 500, 502, 503, 504, 523, 540, 541]`.
**Sources.**
- https://docs.scrapy.org/en/latest/topics/settings.html (DOWNLOAD_DELAY, CONCURRENT_REQUESTS_PER_DOMAIN)
- https://docs.scrapy.org/en/latest/topics/downloader-middleware.html#scrapy.downloadermiddlewares.retry.RetryMiddleware
- https://docs.scrapy.org/en/latest/topics/autothrottle.html
- https://docs.scrapy.org/en/latest/topics/practices.html#avoiding-getting-banned
- https://docs.scrapy.org/en/latest/topics/optimize.html#optimize-concurrency
- https://heritrix.readthedocs.io/en/stable/bean-reference.html (DispositionProcessor, BdbFrontier)
- https://developers.google.com/search/docs/crawling-indexing/http-network-errors
- https://spidermon.readthedocs.io/en/latest/monitors.html

#### A5. IP reputation, and cookies not tied to the IP
**Failure mode.** One bad or flagged IP poisons every request it carries. Cookies earned on one IP and replayed from another look like session hijacking, and a small pool gets used up quickly.
**What leaders do.**
- Crawlee `SessionPool` (on by default) bundles one IP with its cookies, auth tokens and headers. It picks sessions at random so a small pool is not exhausted, and `persistCookiesPerSession` defaults to true.
- Session defaults: `maxPoolSize` 1000, `blockedStatusCodes` `[401, 403, 429]`. A session is discarded when its error score reaches `maxErrorScore` 3 (each `markBad` adds 1, each `markGood` removes 0.5), after `maxUsageCount` 50 uses, or after `maxAgeSecs` 3000.
- Crawlee rotates the session up to `maxSessionRotations` 10 times on blocking. These rotations are not counted against `maxRequestRetries` (3).
- The docs say to pass `session.id` to `proxyConfiguration.newUrl()` so a session keeps one proxy. They warn that early requests error a lot until the pool settles.
- Apify residential proxy: a `session` parameter pins one IP for about 30 minutes. The IP changes when the session expires or the proxy stops responding, and sessions can be destroyed at any time.
- Scrapy's ban advice: use an IP pool, and disable cookies so a session identifier does not link the requests.
**Sources.**
- https://crawlee.dev/js/docs/guides/session-management
- https://crawlee.dev/js/api/core/interface/SessionPoolOptions
- https://crawlee.dev/js/api/core/interface/SessionOptions
- https://crawlee.dev/js/api/basic-crawler/interface/BasicCrawlerOptions
- https://docs.apify.com/platform/proxy/residential-proxy
- https://docs.scrapy.org/en/latest/topics/practices.html#avoiding-getting-banned

#### A6. TLS fingerprint (JA3/JA4) gives the client away
**Failure mode.** The TLS ClientHello identifies the client library whatever the User-Agent says, so a "Chrome" UA on a Python or Rust TLS stack is easy to spot.
**What leaders do.**
- Cloudflare computes JA3/JA4 during the TLS handshake. The value is stable for one client type across destination IPs, ports and certificates.
- JA4 sorts ClientHello extensions, so modern browsers produce fewer unique fingerprints. JA4 Signals add per-fingerprint 1-hour ratios and ranks (for example `browser_ratio_1h`, `h2h3_ratio_1h`).
- Customers can block or challenge by JA3/JA4 in WAF custom rules, and an allowlisted fingerprint can bypass rules. A resumed TLS session is not fingerprinted again. This feature needs Enterprise Bot Management.
- Scrapy's ban advice includes setting `DOWNLOAD_TLS_MIN_VERSION` and `DOWNLOAD_TLS_MAX_VERSION` to match browser TLS behaviour.
**Sources.**
- https://developers.cloudflare.com/bots/concepts/ja3-ja4-fingerprint/
- https://docs.scrapy.org/en/latest/topics/practices.html#avoiding-getting-banned

#### A7. HTTP/2 fingerprint gives the client away
**Failure mode.** Even with a browser-like TLS stack, the HTTP/2 SETTINGS values, the WINDOW_UPDATE increment, PRIORITY frames and the order of pseudo-headers identify the client software.
**What leaders do.** The Black Hat EU 2017 talk on passive HTTP/2 fingerprinting proposes those four elements as a client fingerprint. Defence means matching a real browser's HTTP/2 framing, not only its headers.
**Sources.**
- https://www.blackhat.com/docs/eu-17/materials/eu-17-Shuster-Passive-Fingerprinting-Of-HTTP2-Clients.pdf. Not fetched: the PDF exceeded the 10 MB fetch limit, so this entry rests on the search-result summary of the slides only. Verify before relying on it.

#### A8. `navigator.webdriver` reveals remote control
**Failure mode.** Under WebDriver control, the page can read `navigator.webdriver === true`.
**What leaders do.** The W3C spec defines a webdriver-active flag, set to true when the user agent is under remote control and false at first. `navigator.webdriver` returns that flag. It is exposed on `Navigator` and should not be exposed on `WorkerNavigator`.
**Sources.**
- https://www.w3.org/TR/webdriver2/#interface

#### A9. Headless mode and an inconsistent browser fingerprint
**Failure mode.** The old headless implementation differs from real Chrome. A fingerprint that never changes, or does not match the IP or locale, links requests across IPs.
**What leaders do.**
- Chrome: old Headless was a separate browser implementation in the same binary. New Headless shares the code of headful Chrome. Since Chrome 132.0.6793.0, old Headless ships only as the standalone `chrome-headless-shell`. Puppeteer `headless: true` selects new Headless and `headless: 'shell'` the old one.
- Crawlee generates and injects browser fingerprints by default in `PlaywrightCrawler` and `PuppeteerCrawler` (`browserPoolOptions.useFingerprints`: true). `fingerprintGeneratorOptions` can constrain browsers, devices, operating systems and locales. The docs say fingerprints can persist across IPs, so they should change during scraping.
- For hard targets such as Cloudflare, Crawlee points to Camoufox (a stealth Firefox build), with `useFingerprints: false` to avoid conflicts.
**Sources.**
- https://developer.chrome.com/docs/chromium/headless
- https://crawlee.dev/js/docs/guides/avoid-blocking
- https://crawlee.dev/js/api/browser-pool/interface/BrowserPoolOptions

#### A10. CDP `Runtime.enable` is detectable
**Failure mode.** Puppeteer, Playwright and Selenium call `Runtime.Enable` over CDP to find execution contexts. That makes the browser emit `Runtime.consoleAPICalled` events, which a few lines of page JS can detect.
**What leaders do.** The rebrowser write-up says Cloudflare (Turnstile) and DataDome use this check. It describes two patches that stop the automatic `Runtime.Enable` per frame:
1. Use `Page.createIsolatedWorld`. Page scripts cannot see it, but it has no access to main-world objects.
2. Call `Runtime.Enable` and then `Runtime.Disable` at once to capture the context ID. This keeps main-world access but leaves a brief leak window.

It notes that console overrides can themselves be detected.
**Sources.**
- https://rebrowser.net/blog/how-to-fix-runtime-enable-cdp-detection-of-puppeteer-playwright-and-other-automation-libraries (vendor blog, not a standard)

#### A11. Markup changes mid-crawl (A/B tests, redesigns, generated class names)
**Failure mode.** Selectors tied to DOM structure or CSS classes stop matching. Fields come back empty without any error (see C1 and C2).
**What leaders do.**
- Playwright advises user-facing attributes and explicit contracts (role, text, test id) over XPath/CSS, because "Your DOM can easily change". Its bad example is a class-based selector broken by a designer's change.
- Spidermon field-coverage rules detect the breakage after the fact (C2).
**Sources.**
- https://playwright.dev/docs/best-practices#prefer-user-facing-attributes-to-xpath-or-css-selectors
- https://playwright.dev/docs/best-practices#use-locators

#### A12. Content depends on geography, locale or timezone
**Failure mode.** Prices, languages, availability and even page structure change with the exit IP's country and the browser's locale or timezone. A mismatch between IP and locale is also a bot signal.
**What leaders do.**
- Apify proxy selects the exit country with `country-XX` (`countryCode` in the SDK). US states use `country-US_XX`.
- Playwright emulates `locale`, `timezoneId`, `geolocation`, permissions, `userAgent` and viewport per context. These affect only the browser, not the test runner's timezone (set `TZ` for that).
- Crawlee fingerprint options include `locales`.
**Sources.**
- https://docs.apify.com/platform/proxy/residential-proxy
- https://playwright.dev/docs/emulation#locale--timezone
- https://crawlee.dev/js/docs/guides/avoid-blocking

#### A13. Login session expires mid-crawl
**Failure mode.** Auth cookies or tokens expire, or the server revokes them, during a long run or across a pause. The crawler goes on fetching login redirects or logged-out pages, often with status 200.
**What leaders do.**
- Scrapy: if a JOBDIR resume is delayed, cookies may expire and scheduled requests may fail.
- Playwright: you must delete a saved `storageState` yourself once it expires and re-run the auth setup. `sessionStorage` is not persisted, and Playwright has no API for it. The state file holds cookies that can impersonate the user, so never commit it.
- Crawlee caps a session's life with `maxAgeSecs` 3000 and `maxUsageCount` 50, and treats 401 as a blocked status by default (retires the session).
**Sources.**
- https://docs.scrapy.org/en/latest/topics/jobs.html
- https://playwright.dev/docs/auth#basic-shared-account-in-all-tests
- https://playwright.dev/docs/auth#session-storage
- https://playwright.dev/docs/auth#core-concepts
- https://crawlee.dev/js/api/core/interface/SessionOptions
- https://crawlee.dev/js/api/core/interface/SessionPoolOptions

---

### B. Long runs and operations

#### B1. Memory grows with the request queue
**Failure mode.** The scheduler keeps every pending request, with its `meta` and `cb_kwargs`, in RAM. Broad crawls grow until the process is killed for running out of memory.
**What leaders do.**
- Scrapy keeps the queue in memory by default. The leaks page says this is not necessarily a leak and suggests a persistent (disk) job queue, which `JOBDIR` provides.
- Scrapy `start()` is iterated as fast as it yields, so every start request reaches the scheduler early. To throttle this, the docs show pausing on `engine.needs_backout()` and `scheduler_empty`.
- For memory, Scrapy suggests giving higher priority to requests whose callbacks yield no more requests, so they leave the queue sooner, and trying BFO crawl order.
**Sources.**
- https://docs.scrapy.org/en/latest/topics/leaks.html (Too Many Requests?)
- https://docs.scrapy.org/en/latest/topics/spiders.html#start-requests
- https://docs.scrapy.org/en/latest/topics/optimize.html#optimize-memory

#### B2. Leaks through retained references, and memory that never goes back to the OS
**Failure mode.** Passing responses or large objects through request `meta`, `cb_kwargs` or callbacks ties their lifetime to queued requests. Even without a leak, the process RSS never shrinks.
**What leaders do.**
- Scrapy calls `meta`/`cb_kwargs`/callback references the most common leak cause. Its `trackref` tool (`prefs()`, `get_oldest()`) counts live Requests, Responses, Items and Selectors, and muppy covers other objects. Extensions should release on `spider_closed` what they allocate on `spider_opened`.
- "Leaks without leaks": Python may not return freed memory to the OS because of arena fragmentation. The fix is smaller jobs, a disk queue, and periodic restarts.
**Sources.**
- https://docs.scrapy.org/en/latest/topics/leaks.html

#### B3. No memory guard, or one that fires too late
**Failure mode.** Without a ceiling the kernel's OOM killer ends the process uncleanly, so there is no clean stop and no resumable state (B6).
**What leaders do.**
- Scrapy MemoryUsage extension: `MEMUSAGE_ENABLED` true, but `MEMUSAGE_LIMIT_MB` and `MEMUSAGE_WARNING_MB` default to 0 (no check). Memory is checked every 60 s. When the limit is exceeded the spider closes with reason `memusage_exceeded`. The extension does not work on Windows.
- Crawlee: `availableMemoryRatio` 0.25 (`CRAWLEE_MEMORY_MBYTES` overrides it; by default a quarter of system memory). The Snapshotter treats memory as overloaded above `maxUsedMemoryRatio` 0.9, and also watches the event loop (`maxBlockedMillis` 50) and client errors (`maxClientErrors` 3).
- Crawlee's AutoscaledPool scales concurrency up or down every 10 s (`autoscaleIntervalSecs`) in steps of 0.05, between `minConcurrency` 1 and `maxConcurrency` 200.
**Sources.**
- https://docs.scrapy.org/en/latest/topics/settings.html (MEMUSAGE_*)
- https://docs.scrapy.org/en/latest/topics/extensions.html#scrapy.extensions.memusage.MemoryUsage
- https://crawlee.dev/js/api/core/interface/ConfigurationOptions
- https://crawlee.dev/js/api/core/interface/SnapshotterOptions
- https://crawlee.dev/js/docs/guides/scaling-crawlers

#### B4. Browser leaks, hung browsers and zombie processes
**Failure mode.** Long-lived browsers grow in memory. A browser can hang after its last page, and child Chrome processes are not reaped in containers, so they pile up as zombies.
**What leaders do.**
- Crawlee BrowserPool recycles each browser after `retireBrowserAfterPageCount` 100 pages and caps it at `maxOpenPagesPerBrowser` 20. It closes inactive browsers after `closeInactiveBrowserAfterSecs` 300 because stuck browsers do not always close, and checks for inactivity every `retireInactiveBrowserAfterSecs` 10. Launch and new-page operations time out after `operationTimeoutSecs` 15.
- Puppeteer troubleshooting: use `docker run --init` (Docker 1.13.0 and later) to reap zombie processes, or `dumb-init` as PID 1. It notes that PID 1 is treated specially, which makes Chrome hard to terminate.
**Sources.**
- https://crawlee.dev/js/api/browser-pool/interface/BrowserPoolOptions
- https://pptr.dev/troubleshooting

#### B5. Unbounded seen-URL or dedup sets
**Failure mode.** The "already seen" set grows with every unique URL or item for the whole run. In RAM this is a slow memory leak, and if it is not persisted, a restart forgets it.
**What leaders do.**
- Scrapy `RFPDupeFilter` (the default `DUPEFILTER_CLASS`) filters on request fingerprints. With `JOBDIR` the visited set is saved as `requests.seen`.
- Heritrix uses a BerkeleyDB-backed already-seen filter (`BdbUriUniqFilter`), and its frontier is the BDB-backed `BdbFrontier`.
- Scrapy's item "Duplicates filter" example is a plain in-memory `set` of ids that grows with the crawl and is not persisted (see C3).
**Sources.**
- https://docs.scrapy.org/en/latest/topics/settings.html (DUPEFILTER_CLASS)
- https://docs.scrapy.org/en/latest/topics/jobs.html
- https://heritrix.readthedocs.io/en/stable/bean-reference.html (BdbUriUniqFilter, BdbFrontier)
- https://docs.scrapy.org/en/latest/topics/item-pipeline.html#duplicates-filter

#### B6. Disk fills up (queue, state, output, temporary files)
**Failure mode.** Disk queues, checkpoints, WARCs and output grow until writes fail mid-record. A full disk can corrupt state as well as stop the crawl.
**What leaders do.**
- Heritrix `DiskSpaceMonitor` pauses the crawl when free space on any path in `monitorPaths` drops below `pauseThresholdMiB` 8192. It checks on each stats snapshot.
- Heritrix uses hard links for the BDB files in checkpoints, and old checkpoint files can be deleted automatically (`forgetAllButLatest`).
- Scrapy caps response bodies at `DOWNLOAD_MAXSIZE` 1 GiB and warns at `DOWNLOAD_WARNSIZE` 32 MiB. The optimisation page notes the 1 GiB cap is multiplied by concurrency.
- For FTP/S3/GCS, Scrapy feeds go to a temporary local file until the end of the crawl (C8).
- Apify datasets accept items only under 9 MB of JSON.
**Sources.**
- https://heritrix.readthedocs.io/en/stable/bean-reference.html (DiskSpaceMonitor)
- https://github.com/internetarchive/heritrix3/wiki/Checkpointing
- https://heritrix.readthedocs.io/en/latest/operating.html
- https://docs.scrapy.org/en/latest/topics/settings.html (DOWNLOAD_MAXSIZE, DOWNLOAD_WARNSIZE)
- https://docs.scrapy.org/en/latest/topics/feed-exports.html#delayed-file-delivery
- https://docs.apify.com/platform/storage/dataset

#### B7. Crash recovery: resume works only after a clean stop, and some work is lost anyway
**Failure mode.** A kill -9, an OOM kill or power loss leaves on-disk state half-written. Resuming then fails, or work that was only in memory is silently lost.
**What leaders do.**
- Scrapy JOBDIR stores `requests.queue/`, `requests.seen` and `spider.state` for one job; do not share it.
  - Pause/resume works only after a clean stop, and an unclean shutdown can corrupt the directory.
  - Resume with the same Scrapy version.
  - Requests are pickled, so callbacks must be spider methods. Requests that cannot be serialised still run but live only in memory, so they are lost on pause (`SCHEDULER_DEBUG` logs them). `meta`/`cb_kwargs` come back as deep copies.
  - Newer releases add SQLite disk queues, which write more slowly but keep the queue "usable after an unclean shutdown".
- Heritrix:
  - Automatic checkpoints are off by default (`checkpointIntervalMinutes` −1). `checkpointOnShutdown` covers graceful JVM shutdown only. A kill, crash or power loss produces no checkpoint, so the docs advise enabling both interval and shutdown checkpoints.
  - Between checkpoints, the `frontier.recover.gz` journal (`recoveryLogEnabled` true) rebuilds the frontier approximately. Replaying it for about 100M URIs can take days.
  - `.warc.gz.open` files may be truncated and should be validated.
- Crawlee: `purgeOnStart` defaults to true (`CRAWLEE_PURGE_ON_START`), so a plain restart wipes the default storages unless you turn that off.
**Sources.**
- https://docs.scrapy.org/en/latest/topics/jobs.html
- https://docs.scrapy.org/en/latest/topics/settings.html (SCHEDULER_DISK_QUEUE)
- https://github.com/internetarchive/heritrix3/wiki/Checkpointing
- https://heritrix.readthedocs.io/en/latest/operating.html
- https://heritrix.readthedocs.io/en/stable/bean-reference.html (CheckpointService, BdbFrontier)
- https://crawlee.dev/js/api/core/interface/ConfigurationOptions
- https://crawlee.dev/js/docs/guides/request-storage

#### B8. Duplicated work after a crash (at-least-once delivery)
**Failure mode.** A request that was being processed when the worker died is handed out again. State saved on an interval rolls back to the last save, so side effects (items, writes) are repeated.
**What leaders do.**
- Apify request queue: locked requests are hidden from other clients until the lock expires. If the run fails, the lock lapses and the request "is processed again eventually".
  - Crawlee's default lock length equals `requestHandlerTimeoutSecs` (60 s).
  - Only the same client key or run can prolong (`prolongRequestLock`) or delete a lock.
- Crawlee emits `persistState` every `persistStateIntervalMillis` 60 000. On a platform migration the Actor gets only a few seconds to save, and restarts within a minute; `Actor.reboot()` after saving speeds this up.
- Heritrix re-emits a URI without final disposition up to `maxRetries` 30.
- Practical consequence: downstream writes must be idempotent or deduplicated (C3).
**Sources.**
- https://docs.apify.com/platform/storage/request-queue (Distributivity)
- https://crawlee.dev/js/api/core/interface/ConfigurationOptions
- https://crawlee.dev/js/api/basic-crawler/interface/BasicCrawlerOptions
- https://docs.apify.com/platform/actors/development/builds-and-runs/state-persistence
- https://heritrix.readthedocs.io/en/stable/bean-reference.html (BdbFrontier)

#### B9. Wall clock used for timeouts, backoff and rate limits
**Failure mode.** NTP steps, manual clock changes or VM suspend move the wall clock. Timeouts then fire early or never, and "elapsed" comes out negative.
**What leaders do.**
- Linux: `CLOCK_REALTIME` is subject to discontinuous jumps (settimeofday, NTP). `CLOCK_MONOTONIC` has no jumps but takes NTP frequency slewing, and does not count suspend. `CLOCK_BOOTTIME` is monotonic and includes suspend.
- Rust: `SystemTime` "is **not monotonic**", and `duration_since` returns `Err` when the clock went backwards. Use `Instant` for elapsed time.
  - `Instant` never goes backwards but "may jump forwards", barring platform bugs. On a violation, `duration_since` saturates to zero and `checked_duration_since` returns `None`.
  - Whether suspend counts as elapsed time is unspecified (Unix uses `CLOCK_MONOTONIC`, Darwin `CLOCK_UPTIME_RAW`).
  - Keep `SystemTime` for timestamps compared with other systems (for example Heritrix's ISO 8601 `crawl.log` timestamps).
**Sources.**
- https://man7.org/linux/man-pages/man2/clock_gettime.2.html
- https://doc.rust-lang.org/std/time/struct.SystemTime.html
- https://doc.rust-lang.org/std/time/struct.Instant.html
- https://heritrix.readthedocs.io/en/latest/operating.html (crawl.log format)

#### B10. No back-pressure between fetching and processing
**Failure mode.** Fetching outpaces parsing or the item pipeline (slow DB writes, LLM calls). Responses and items then pile up in memory, and so does the queue of discovered URLs.
**What leaders do.**
- Scrapy `SCRAPER_SLOT_MAX_ACTIVE_SIZE` 5 000 000 bytes: a soft cap on response data being processed. Above it, Scrapy stops processing new requests.
- `CONCURRENT_ITEMS` 100 limits parallel pipeline items per response.
- Start-request back-pressure is manual (B1).
- Crawlee AutoscaledPool scales up only when actual concurrency reaches `desiredConcurrencyRatio` 0.95 of the desired value and the system is not overloaded (B3). `maxRequestsPerMinute` is unlimited by default.
**Sources.**
- https://docs.scrapy.org/en/latest/topics/settings.html (SCRAPER_SLOT_MAX_ACTIVE_SIZE, CONCURRENT_ITEMS)
- https://docs.scrapy.org/en/latest/topics/optimize.html#broad-crawls
- https://crawlee.dev/js/docs/guides/scaling-crawlers

#### B11. Hung fetches and handlers with no timeout
**Failure mode.** A socket that stalls, a page that never finishes loading, or a handler waiting on a promise that never settles holds a worker forever. Throughput decays toward zero without any error.
**What leaders do.**
- Scrapy `DOWNLOAD_TIMEOUT` is 180 s (per request via `download_timeout`). The broad-crawl advice is to lower it.
- Heritrix FetchHTTP `timeoutSeconds` and `soTimeoutMs` default to 0. The docs warn that no socket timeout can hang a thread, and say to keep `soTimeoutMs` below `timeoutSeconds`.
- Crawlee `requestHandlerTimeoutSecs` is 60 and BrowserPool `operationTimeoutSecs` 15.
- Scrapy `CLOSESPIDER_TIMEOUT_NO_ITEM` can stop a run that has produced nothing for N seconds (off by default).
**Sources.**
- https://docs.scrapy.org/en/latest/topics/settings.html (DOWNLOAD_TIMEOUT)
- https://docs.scrapy.org/en/latest/topics/optimize.html#broad-crawls
- https://heritrix.readthedocs.io/en/stable/bean-reference.html (FetchHTTP)
- https://crawlee.dev/js/api/basic-crawler/interface/BasicCrawlerOptions
- https://crawlee.dev/js/api/browser-pool/interface/BrowserPoolOptions
- https://docs.scrapy.org/en/latest/topics/extensions.html#std-setting-CLOSESPIDER_TIMEOUT_NO_ITEM

#### B12. File descriptors run out
**Failure mode.** Sockets, browser pipes, open output files and per-host connections add up. Past the soft `RLIMIT_NOFILE`, every `open`, `socket` or `pipe` fails with `EMFILE`, often surfacing as odd network errors.
**What leaders do.**
- Linux: the soft limit is the one enforced. An unprivileged process may raise it up to the hard limit; raising the hard limit needs `CAP_SYS_RESOURCE` and is capped by `/proc/sys/fs/nr_open`.
- None of the crawler docs I read documents an FD guard. The levers are the concurrency caps (`CONCURRENT_REQUESTS`, `maxConcurrency` 200, `maxOpenPagesPerBrowser` 20).
**Sources.**
- https://man7.org/linux/man-pages/man2/getrlimit.2.html
- https://docs.scrapy.org/en/latest/topics/optimize.html#optimize-concurrency
- https://crawlee.dev/js/api/browser-pool/interface/BrowserPoolOptions

#### B13. Log volume
**Failure mode.** Logging every request at debug level costs CPU and fills disks on long runs. One huge unrotated file is also hard to use.
**What leaders do.**
- Scrapy `LOG_LEVEL` defaults to `'DEBUG'`. The optimisation page says to set `"INFO"` or higher to cut CPU.
- `LOG_FILE` does not rotate; for rotation, use Python's `logging.handlers` (for example `TimedRotatingFileHandler`). `LOG_FILE_APPEND` is true.
- Periodic statistics: Scrapy `LOGSTATS_INTERVAL` 60 s, Crawlee `loggingIntervalSecs` 60.
**Sources.**
- https://docs.scrapy.org/en/latest/topics/settings.html (LOG_LEVEL, LOG_FILE_APPEND, LOGSTATS_INTERVAL)
- https://docs.scrapy.org/en/latest/topics/optimize.html#optimize-cpu
- https://docs.scrapy.org/en/latest/topics/logging.html
- https://crawlee.dev/js/docs/guides/scaling-crawlers

#### B14. Shutdown on SIGTERM is not graceful
**Failure mode.** Orchestrators send SIGTERM and then SIGKILL after a grace period. A crawler that ignores SIGTERM, or needs longer than the grace period to drain, loses in-flight work and leaves state that cannot be resumed (B7).
**What leaders do.**
- Kubernetes: the default grace period is 30 s. The kubelet runs `preStop`, sends SIGTERM, and sends SIGKILL to anything still running when the period ends (with a one-off 2 s extension for a `preStop` that is still running).
- Tokio's guidance has three steps:
  1. Detect the signal (`signal::ctrl_c`, or a channel through `select!`).
  2. Broadcast cancellation with `CancellationToken`, which leaves tasks room to flush.
  3. Wait for the tasks with `TaskTracker` (`close()` then `wait()`).
- Scrapy: when a CLOSESPIDER condition triggers, requests already in the downloader (up to `CONCURRENT_REQUESTS`) are still processed. JOBDIR resume needs this clean stop.
- Heritrix `checkpointOnShutdown`; Apify `migrating` event (B8).
**Sources.**
- https://kubernetes.io/docs/concepts/workloads/pods/pod-lifecycle/#pod-termination
- https://tokio.rs/tokio/topics/shutdown
- https://docs.scrapy.org/en/latest/topics/extensions.html (CloseSpider)
- https://docs.scrapy.org/en/latest/topics/jobs.html
- https://heritrix.readthedocs.io/en/latest/operating.html

---

### C. Data

#### C1. Schema drift
**Failure mode.** After a site change, fields disappear, are renamed, or change type (a price becomes a string with a currency sign, a list becomes a scalar). Downstream consumers break, or they quietly accept bad rows.
**What leaders do.**
- Spidermon's `ItemValidationPipeline` (placed last in `ITEM_PIPELINES`) validates items against JSON Schema: required fields, types, regex patterns. Validation errors are recorded in stats under `spidermon/validation/...`.
- By default invalid items are kept and not annotated: `SPIDERMON_VALIDATION_DROP_ITEMS_WITH_ERRORS` and `SPIDERMON_VALIDATION_ADD_ERRORS_TO_ITEMS` are both false.
- `ItemValidationMonitor` fails the job past `SPIDERMON_MAX_ITEM_VALIDATION_ERRORS` (no default; not configured unless set).
**Sources.**
- https://spidermon.readthedocs.io/en/latest/item-validation.html
- https://spidermon.readthedocs.io/en/latest/monitors.html

#### C2. Extraction silently returns nothing
**Failure mode.** Selectors match nothing (A11), or pages are challenges or stubs (A1, A2). The run "succeeds" with zero or sparse items and an empty or near-empty output file.
**What leaders do.**
- Scrapy auto-close guards (all off by default): `CLOSESPIDER_PAGECOUNT_NO_ITEM` (consecutive responses with no items), `CLOSESPIDER_TIMEOUT_NO_ITEM`, `CLOSESPIDER_ERRORCOUNT`.
- Spidermon monitors:
  - `ItemCountMonitor`: `SPIDERMON_MIN_ITEMS`.
  - `FieldCoverageMonitor`: per-field coverage rules, `SPIDERMON_FIELD_COVERAGE_TOLERANCE` 0.
  - `FinishReasonMonitor`: expects `['finished']`.
  - `ZyteJobsComparisonMonitor`: compares item counts with previous jobs (off by default).
- Scrapy `FEED_STORE_EMPTY` is true by default, so a run with no items still writes an empty feed file. With false, no file is created or changed.
**Sources.**
- https://docs.scrapy.org/en/latest/topics/extensions.html#std-setting-CLOSESPIDER_PAGECOUNT_NO_ITEM
- https://spidermon.readthedocs.io/en/latest/monitors.html
- https://docs.scrapy.org/en/latest/topics/feed-exports.html#std-setting-FEED_STORE_EMPTY

#### C3. Duplicate records within a run and across runs
**Failure mode.** The same entity appears on several listing pages, under several URLs, or again after an at-least-once retry (B8). Re-runs append the same records again.
**What leaders do.**
- Scrapy's "Duplicates filter" pipeline example keeps an in-memory set of item `id`s and raises `DropItem` on a repeat. It assumes a unique `id` field, does not persist, and does not handle a missing id.
- Across runs, scrapy-deltafetch stores request fingerprints in `dbm` (`DELTAFETCH_DIR`) and skips pages seen in earlier crawls of the same spider. `deltafetch_key` can switch the key to an item id, and `DELTAFETCH_RESET` clears the state.
- Heritrix computes a SHA-1 content digest (`digestContent` true). `WARCWriterChainProcessor.skipIdenticalDigests` (default false) can skip writing a record whose digest matches the previous fetch.
- Crawlee moves RequestList requests into the queue first to avoid processing them twice.
**Sources.**
- https://docs.scrapy.org/en/latest/topics/item-pipeline.html#duplicates-filter
- https://github.com/scrapy-plugins/scrapy-deltafetch
- https://heritrix.readthedocs.io/en/stable/bean-reference.html (FetchHTTP, WARCWriterChainProcessor)
- https://crawlee.dev/js/docs/guides/request-storage

#### C4. Encoding errors and mojibake
**Failure mode.** The declared charset is wrong or missing, so bytes are decoded with the wrong encoding (for example UTF-8 read as Windows-1252 turns "é" into "Ã©"). Or the output is written in an encoding the consumer does not expect.
**What leaders do.**
- Scrapy `TextResponse` resolves the encoding in this order:
  1. The constructor argument.
  2. A BOM.
  3. The Content-Type header (unknown values are ignored).
  4. The in-body declaration (HTML/XML responses only).
  5. Inference from the body, which the docs call the most fragile.
- Scrapy `FEED_EXPORT_ENCODING` defaults to `"utf-8"`. If set to `None`, JSON output uses `\uXXXX` escapes for historic reasons.
- ftfy detects and reverses mojibake heuristically, including several stacked layers, and leaves correct text unchanged.
**Sources.**
- https://docs.scrapy.org/en/latest/topics/request-response.html#scrapy.http.TextResponse.encoding
- https://docs.scrapy.org/en/latest/topics/feed-exports.html#std-setting-FEED_EXPORT_ENCODING
- https://ftfy.readthedocs.io/en/latest/

#### C5. Dates and timezones misread
**Failure mode.** "01/02/2020" means 1 February or 2 January depending on locale. Relative dates ("3 hours ago") depend on when they were parsed. Naive datetimes silently take the crawler machine's timezone.
**What leaders do.**
- dateparser defaults:
  - `DATE_ORDER` is `MDY`, but `PREFER_LOCALE_DATE_ORDER` (true) lets the detected language's order win.
  - Impossible values are reassigned unless `STRICT_DATE_ORDER` is set: `13/12/10` becomes 13 December.
  - `TIMEZONE` defaults to the local timezone and `TO_TIMEZONE` to None, so results are naive unless `RETURN_AS_TIMEZONE_AWARE` is set.
  - `RELATIVE_BASE` defaults to now; `STRICT_PARSING` is false and `REQUIRE_PARTS` is `[]`.
- Playwright `timezoneId`/`locale` change what the page renders, not the runner's own timezone (A12).
**Sources.**
- https://dateparser.readthedocs.io/en/latest/settings.html
- https://playwright.dev/docs/emulation#locale--timezone

#### C6. Number formats depend on locale
**Failure mode.** "1.099,98" (German) and "1,099.98" (US) mean the same number. Naive parsing yields 1.099 or an error, and grouping separators can be non-breaking spaces.
**What leaders do.**
- Babel `parse_decimal` reads both as 1099.98 given the right locale (`de` vs `en_US`), and raises `NumberFormatError` on input that is invalid for the locale (for example `'2,109,998'` under `de`).
- `parse_number` accepts a locale's space-like separator (`'123\xa04567'` under `ru`) but rejects a plain space.
- Since 2.8.0 `parse_number` is limited and "may behave unexpectedly".
**Sources.**
- https://babel.pocoo.org/en/latest/numbers.html

#### C7. Partly written or overwritten output files
**Failure mode.** A crash mid-write leaves a truncated JSON array or WARC. Remote uploads that happen only at the end lose everything on a crash. Overwrite mode destroys the previous good output.
**What leaders do.**
- Scrapy delayed file delivery (FTP/FTPS/S3/GCS): items go to a temporary local file that is uploaded only when the crawl ends. `FEED_EXPORT_BATCH_ITEM_COUNT` (default 0) splits output into files of N items, each delivered when it fills, with `%(batch_id)d` or `%(batch_time)s` in the URI.
- `overwrite` defaults: local filesystem false (append), FTP true, S3 and GCS true (appending is not supported), stdout false. The docs warn that true loses the previous version of your data.
- Heritrix marks WARCs being written as `.warc.gz.open`. After a crash they may be truncated and need validating before the suffix is removed.
- POSIX `rename(2)` replaces an existing target atomically, so a reader never finds it missing: write to a temporary file, then rename. On NFS, a failed rename may still have happened.
**Sources.**
- https://docs.scrapy.org/en/latest/topics/feed-exports.html#delayed-file-delivery
- https://docs.scrapy.org/en/latest/topics/feed-exports.html#std-setting-FEED_EXPORT_BATCH_ITEM_COUNT
- https://docs.scrapy.org/en/latest/topics/feed-exports.html#std-setting-FEEDS
- https://heritrix.readthedocs.io/en/latest/operating.html
- https://man7.org/linux/man-pages/man2/rename.2.html

#### C8. CSV columns fixed by the first item
**Failure mode.** Items have different field sets (optional fields, schema drift). A tabular exporter that takes its header from the first item drops later fields, or misaligns them.
**What leaders do.**
- Scrapy exporters that cannot handle varying fields "will only export the fields found in the first item exported". The CSV header comes from `fields_to_export` or from the first item's fields.
- The fix is to set `FEED_EXPORT_FIELDS` / `fields_to_export` (a list, or a dict that also renames).
- Multi-valued fields are joined with `join_multivalued` `','`. `CsvItemExporter` ignores `export_empty_fields` and always exports empty fields.
- Apify: tabular exports (HTML, CSV, Excel) are limited to 2000 columns, and data past that "will not be retrieved". `fields`/`omit` select or exclude columns; `omit` accepts nested paths such as `address.city`, `fields` only top-level fields.
**Sources.**
- https://docs.scrapy.org/en/latest/topics/exporters.html#scrapy.exporters.BaseItemExporter.fields_to_export
- https://docs.scrapy.org/en/latest/topics/exporters.html#scrapy.exporters.CsvItemExporter
- https://docs.scrapy.org/en/latest/topics/feed-exports.html#std-setting-FEED_EXPORT_FIELDS
- https://docs.apify.com/platform/storage/dataset

---

### Notes on gaps and reliability
- **HTTP/2 fingerprinting (A7)** rests on a search summary, because the Black Hat PDF was too large to fetch.
- **CDP `Runtime.enable` detection (A10)** comes from one vendor blog (rebrowser), not from a standard or browser-vendor document.
- **Cloudflare challenge status codes:** the challenge-pages pages do not state a code. The 403 comes from the custom-error page-types reference. I found no Cloudflare page that documents 503 for challenges.
- **Crawlee:** I did not find docs stating whether a non-purged request queue reloads after a crash, nor default values for `retryOnBlocked` and `navigationTimeoutSecs`.
- **Scrapy:** the docs do not say how many Ctrl-C presses force an unclean stop.
- **Kubernetes grace period:** read from the second half of the pod-lifecycle page (the fetch offset past the first 100 000 characters).
