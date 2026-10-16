# The Frontier parses URLs as browsers do, fixes only safe rules, learns the rest on evidence and caps a URL's shape

The engine parses every URL a crawl meets once, with the WHATWG URL Standard, whatever tier served it. A crawl request's identity is a key made from that parsed URL by fixed rules RFC 3986 calls safe, plus ADR 0005's parameter rules. The request is still fetched at its own URL. Firecrawl's `www`, scheme, trailing-slash and index-file merges, which ADR 0005 applied to every site, become rules learned per host on evidence, as no-effect parameters already are. Fragments are dropped unless a Label keeps them. `rel=canonical` is evidence, never a merge. Three caps on a URL's shape are on by default. A sitemap's entries stay inside the crawl's scope. Facets get no default rule.

**What this answers.** Rows U02, U05, U06, U07, U09, U10 and U20 of the [pitfall catalog](https://github.com/goodfetch/goodfetch/blob/prototype/sdk-api/sdk-api-sketch/pitfalls.md), from the [common pitfalls research](https://github.com/goodfetch/goodfetch/blob/research/common-pitfalls/rnd/research/2026-10-16-common-pitfalls.md) (items 1.1 to 1.13, 1.15, 1.16, 1.21, 1.24 and 1.25). ADR 0005 named the competitors' fixed rules without writing them out. It also merged forms that RFC 3986 §6.2.4 merges only on evidence, and it chose no parser and no fragment rule. ADR 0093 measured scope against every seed without saying which host a sitemap seed widens it by.

**Competitors and standards, re-read on 2026-10-16.**
- **Crawlee.** [`normalizeUrl`](https://github.com/apify/apify-shared-js/blob/master/packages/utilities/src/utilities.client.ts) parses with the WHATWG `URL`. It drops `utm_*`, sorts the parameters, strips one trailing slash and drops the fragment unless `keepUrlFragment` is set. The request is fetched at its own `url`, and `uniqueKey` is only the key.
- **Firecrawl.** [`generateURLPermutations`](https://github.com/firecrawl/firecrawl/blob/main/apps/api/src/lib/crawl-redis.ts) treats `www`, `http`/`https`, `index.html`, `index.php` and a trailing slash as one URL, under `deduplicateSimilarURLs`, which is on by default. Its `normalizeURL` keeps a fragment that starts `#/` or `#!/`.
- **Scrapy, StormCrawler, Nutch.**
  - All three drop the fragment from the key (R 1.9).
  - Scrapy's `LinkExtractor` warns that canonicalizing can change the URL the server sees (R 1.8).
  - Scrapy's `URLLENGTH_LIMIT` is 2,083 (R 1.3).
  - Nutch rejects a segment repeated three times, Heritrix three identical segments in a row, and Heritrix's `maxPathDepth` is 20 (R 1.2, 1.3).
  - Nutch strips session IDs broadly, Heritrix only 32-character ones (R 1.6).
- **Standards.**
  - RFC 3986 §6.2.2 and §6.2.3 list the safe normalizations. §6.2.4 allows protocol-based merges only when "equivalence is clearly indicated by both the result of accessing the resources" and the scheme's conventions: a spider that sees `/data` redirect to `/data/` will "likely regard the two as equivalent in the future" (R 1.10).
  - The [WHATWG URL Standard](https://url.spec.whatwg.org/) is what browsers parse, hosts included: UTS #46 punycode, hex and octal IPv4 (R 1.12, 1.13). The [`url` crate](https://docs.rs/url/2.5.8/url/) 2.5.8 implements it.
  - [Jakarta Servlet 6.0](https://jakarta.ee/specifications/servlet/6.0/jakarta-servlet-spec-6.0.html) §7.1.3 names the URL-rewriting session parameter `jsessionid`.
  - [sitemaps.org](https://www.sitemaps.org/protocol.html#submit_robots) lets a sitemap list another host's URLs when that host's robots.txt names it (R 1.25).
  - Google treats `rel=canonical` as "a hint, not a rule" (R 1.15). It tells sites to disallow facet parameters in robots.txt (R 1.5).

Decided in [URL identity and crawl-space rules beyond ADR 0005's learned rules](https://github.com/goodfetch/goodfetch/issues/551), raised by the [Common pitfalls catalog](https://github.com/goodfetch/goodfetch/issues/547) on the map [The SDK API of Collie and Foxhound](https://github.com/goodfetch/goodfetch/issues/461). Every answer was decided by the agent on the owner's delegation, with its [grilling record](https://github.com/goodfetch/goodfetch/issues/551#issuecomment-6204435654) and its [first addendum](https://github.com/goodfetch/goodfetch/issues/551#issuecomment-6204745613). It extends ADR 0093, which the owner confirmed, and reverses none of it. Principles are cited by their IDs in [ADR 0077](0077-the-sdk-api-follows-eight-guarantees-and-four-ranked-shape-rules.md).

## Decision

### One parser, in the engine

- **Every URL a crawl meets is parsed by the WHATWG URL Standard's basic URL parser, as the `url` crate implements it.** That covers a seed, an `addSeed`, an `enqueue`, an `enqueueLinks` `href`, a sitemap entry, a redirect's `Location` and a Hook's rewrite (ADR 0104).
  - **Why browsers' parser.** The parser strips leading and trailing spaces and controls and removes tabs and newlines. It reads `\` as `/`, lowercases the scheme and host, and drops the default port. It reads an empty path as `/`, resolves dot segments (`%2e` forms too) and percent-encodes what each component's set requires. So the crawl sees the links a user's browser sees (R 1.13).
  - **Hosts.** A host is percent-decoded, mapped to ASCII by UTS #46 (punycode), and an IPv4 address in hex, octal or shortened form becomes dotted decimal (R 1.12). Politeness's domain key (ADR 0004), robots.txt's origin (ADR 0086), the scope (ADR 0093) and the domain policy (ADR 0017) all read this parsed host. `bücher.example` and `xn--bcher-kva.example` are then one host everywhere.
  - **On every tier.** The engine resolves the `href` attribute's own value, not the browser's resolved link, so a link's identity never depends on the tier that served the page (G1, and ADR 0093's one handler on every tier).
- **The base.**
  - An `href` and a handler's `enqueue` resolve against the document's base URL, as ADR 0093's `enqueueLinks` does, so a relative URL in `enqueue` works as Scrapy's `response.follow` does.
  - A sitemap entry resolves against its sitemap's URL.
  - A `Location` resolves against its request's URL.
  - A seed and `addSeed` take an absolute URL.
- **What is refused.** A URL that fails to parse is not added, with the reason `invalid_url` (G1). A scheme other than `http` or `https` is ADR 0093's `unsupported_scheme`.

### A crawl request's identity

- **The key, not the fetched URL.** ADR 0093's identity is the method, the URL's key and a hash of the body.
  - The key is made by the rules below. The request is fetched at its URL as parsed, and its outcomes name that URL: parameters are not sorted, `utm_*` is not stripped, and no fragment is sent, since a fragment never reaches a server.
  - When two URLs share a key, the first one admitted is fetched.
  - A declared `key` still replaces the identity (ADR 0093).
- **The fixed rules, in order.** None merges URLs that RFC 3986 or HTTP says may differ. The path keeps its case, since paths are case-sensitive (R 1.10).
  1. **The parser's own work**, above.
  2. **Percent-encoding** (RFC 3986 §6.2.2). In the path and the query, hexadecimal digits are uppercased, and percent-encoded unreserved characters (`A`–`Z`, `a`–`z`, `0`–`9`, `-`, `.`, `_`, `~`) are decoded. A reserved character such as `%2F` stays encoded, and `+` stays apart from `%20`.
  3. **The fragment is dropped**, unless the request's Label keeps it (below).
  4. **ADR 0005's parameter rules:**
     - Crawlee's `utm_*` and Crawl4AI's `gclid` and `fbclid` are dropped;
     - the remaining pairs are sorted by name, a stable sort that keeps the order of values within one name, so `?a=2&a=1` stays apart from `?a=1&a=2`;
     - empty pairs (`&&`, a leading or trailing `&`) and an empty query are dropped, as Crawlee's sort and Nutch's normalizer drop them.
  5. **The Servlet session parameter.** A `;jsessionid=…` path parameter, its name matched in any case, is dropped, since the Servlet specification reserves that name for session IDs. Every other `;` parameter stays part of the path. Query-string session IDs are ADR 0005's learned no-effect parameters, so `sid=`, which names a story on some sites, is merged only on evidence.
- **Firecrawl's merges are learned per host, on evidence.** One leading `www.` label against the bare host, `http` against `https`, a trailing slash, and a final `index.html` or `index.php` against the folder: each is merged once evidence shows the two forms are one resource, as ADR 0005 learns no-effect parameters.
  - **The evidence:**
    - a redirect from one form to the other, the evidence RFC 3986 §6.2.4 names;
    - matching page signatures for the two forms;
    - `rel=canonical` (below).
  - **What a learned merge does.** Like every learned rule, it is kept in run state with its evidence and the number of URLs it merged, it is reported (`equivalent_form` in the Run report's learned rules), and it never crosses a registrable domain (ADR 0005).
    - A pending request it merges ends `skipped` with ADR 0093's `duplicate`.
    - Whether the slash and index-file merges hold per host or per path, and how much evidence each kind needs, are area 6's spec, like ADR 0005's other thresholds.
  - **Why.** A fixed merge that is wrong is silent: the second page is never fetched, and nothing says so (G1). The cost is the first fetch of each other form on a host. On most sites that fetch answers with a redirect, which is itself the evidence.
- **The scope ignores one leading `www.` label on its own.** ADR 0093 compares scope hosts "after the Frontier's fixed URL rules, so `www.` is no other host". That stays true: comparing hosts for scope ignores one leading `www.` label as a scope rule. Under `same_origin`, which also compares the scheme and port, a seed's `http` origin also admits the `https` origin of its host, since sites upgrade and the old fixed scheme merge admitted it; never the reverse, a downgrade. The scope therefore does not change while identity waits for evidence.
- **`rel=canonical` is evidence, never a merge.** A page's canonical is the first `<link rel="canonical">` in its `<head>`, or a `Link` header with `rel="canonical"`. It counts as evidence for a learned rule when it differs from the page's URL only by what such a rule covers: a parameter, `www.`, the scheme, a trailing slash or an index file. It never merges anything by itself. A canonical to another path or another registrable domain is ignored for identity, since a mis-templated site may point every page at its home page (R 1.15). The canonical URL is not enqueued for the program: a handler that wants it reads the tag and enqueues it.
- **A redirect to a URL the Frontier already holds.** When a request's redirect reaches a URL whose key the Frontier already admitted, the request ends `skipped` with `duplicate` and its handler does not run, as Scrapy's duplicate filter drops it. The redirect still counts as evidence for a learned merge. A redirect's final URL is recorded under its key as visited, so a later enqueue of it is a `duplicate` too. A `Location` that fails to parse, hop limits and loops are [#549](https://github.com/goodfetch/goodfetch/issues/549)'s.

### Fragments

- **`crawl.keep_url_fragment`**, false by default, and settable at connection, Run and Label scope, keeps the fragment in the key of that scope's requests. The name is Crawlee's `keepUrlFragment` (S3). Crawlee and Scrapy agree on dropping fragments by default.
  - The key of a request is made by the rules its Label sets, so one URL with two fragments is one request on a Label that drops them, and two on a Label that keeps them.
  - The browser navigates to the URL with its fragment. The fragment is still never sent to a server.
- **Only tiers that run the page's scripts serve such a Label:** `light_browser` and `browser`. On any other tier, every fragment would fetch the same document again (G5).
  - Under `fetch.tier: auto`, the sample comparison chooses between those two.
  - Forcing `http`, `browser_fetch` or `discovered_api` on that Label fails the Run's start with `setting_not_applicable`, as ADR 0110 refuses kept responses under a forced `http` or `browser_fetch`. `discovered_api` is refused here too, though ADR 0110 admits it: its replayed calls cannot tell which call a fragment's view makes without running the page's scripts.
- **Fixed for a Run's life.** `crawl.keep_url_fragment` is one of ADR 0102's keys fixed for a Run's life, since the keys of the requests already admitted would no longer match the new ones. A resume that changes it raises `setting_not_applicable`.

### Caps on a URL's shape

- **Three settings, on by default (G5),** at connection and Run scope, as `crawl.max_depth` is (ADR 0093):

  | Key | Built-in | Refuses |
  |---|---|---|
  | `crawl.max_url_length` | 2,083 | a URL longer than this many characters, as parsed and without its fragment (Scrapy's `URLLENGTH_LIMIT`) |
  | `crawl.max_path_segments` | 20 | a path of more segments, as the parser splits it, an empty last segment (a trailing slash) not counted (Heritrix's `maxPathDepth`) |
  | `crawl.max_segment_repeats` | 2 | a segment, or a run of segments, that appears more than this many times back to back, so `/a/a` passes and `/a/a/a` does not (Heritrix's `maxRepetitions`) |

  - `crawl.max_segment_repeats` counts `/a/a/a` as Heritrix does. It also catches the `/a/b/a/b/a/b` loops that relative links make on a site answering 200 for any path, Google's `../../category/stuff`. Numbers that recur apart, as in `/page/1/tag/1/sort/1`, are not caught, though StormCrawler's any-third-occurrence rule would refuse them.
  - `null` turns a cap off.
- **Where they apply.** They are checked on what enters the Frontier (seeds, enqueues, sitemap entries), after the scope and the domain policy, so only a URL the crawl would otherwise admit is refused by a cap, and an over-long link to another host stays `out_of_scope`. Redirects are not checked, since their limits are [#549](https://github.com/goodfetch/goodfetch/issues/549)'s.
- **The answer.** A URL past a cap is not added, with the setting's name as its reason (`max_url_length`, `max_path_segments`, `max_segment_repeats`), as ADR 0093 names `max_depth`.
- **Completeness.** A finished Run that refused a URL by a cap is `unverified`, since a heuristic cut its crawl space (ADR 0005). Its report's Completeness reasons name the setting and how many URLs it refused.

### Facets get no default rule

- **No default rule merges or caps faceted navigation.** Filter and sort parameters change the page, so ADR 0005's no-effect rule cannot merge them. Every default rule has a cost:
  - a cap on query variants per path also cuts a catalogue served as `product.php?id=…` on one path;
  - a learned trap that closes paths whose pages bring no new links also closes product pages, whose links are mostly known already;
  - no per-path request bound fits every site.
- **What bounds facets:**
  - robots.txt, which Google tells sites to use for facet parameters and a crawl obeys by default (ADR 0086);
  - a `crawl.scope.exclude` pattern naming the facet parameters, since ADR 0093's patterns match the whole URL, its query included;
  - `crawl.max_requests`.
- **Why not more.** At ADR 0086's default rate, a facet explosion is ordinary load on the site, so it costs the program's requests and is no burden on the site (G5). ADR 0037's CO-08 per-path bounds (Spider's `with_budget` shape) stay area 6's spec.

### Sitemap entries stay inside the crawl's scope

- **An entry is never a seed.** ADR 0093 checks each entry "as an enqueue is", at depth 0, so an entry outside the crawl's scope is not added with `out_of_scope` and is counted in its sitemap's totals.
- **A sitemap seed widens the scope by the host of the URL the program named.** That URL is a sitemap or a site's robots.txt (ADR 0093). A seed naming `https://www.example/robots.txt`, whose `Sitemap:` line points at a sitemap on `cdn.example`, admits that sitemap's `www.example` entries. This is sitemaps.org's cross-submit rule: the robots.txt proves the site owns the sitemap. A seed naming the `cdn.example` sitemap itself widens the scope by `cdn.example` only.
- **A sitemap's own fetches**, nested indexes included, go through the domain policy (ADR 0017), Politeness and robots.txt (ADR 0093), not the scope. They run no handler.
- **sitemaps.org's directory rule is not enforced.** Google ignores it for a submitted sitemap, and the scope already bounds what enters.

### Answers

The not-added reasons of ADR 0093 gain `invalid_url`, `max_url_length`, `max_path_segments` and `max_segment_repeats`. Each is a value, never an error (ADR 0079), and an SDK keeps a reason it does not know (G3). No skip reason and no Error code is added.

## Considered Options

- **An RFC 3986 parser:** rejected. It rejects or resolves differently the `href`s browsers accept (R 1.13).
- **Each tier's own parser** (Chromium's in the browser): rejected by S1. One link would get two identities across tiers.
- **Fetching the rewritten URL:** rejected. Sorting or stripping parameters can change what the server returns (R 1.8), and Crawlee and Scrapy fetch the original.
- **Firecrawl's merges kept fixed** (ADR 0005's first form): rejected by G1. A site where they differ loses pages and says nothing, which RFC 3986 §6.2.4 warns against.
- **Firecrawl's merges dropped:** rejected. The www and scheme spellings of one site would double a crawl that a redirect already shows to be one.
- **Fragments kept when spelled `#/` or `#!/`** (Firecrawl): rejected. A hash route spelled `#products` is still lost, and a static anchor spelled `#/` is fetched again.
- **Fragments kept when the browser sees them change the document:** rejected. Every fragment needs a browser fetch to find out.
- **Stripping `sid`, `phpsessid` and `sessionid` as a fixed list** (Nutch): rejected. `sid` names a story on some sites, and ADR 0005's learned rule merges real session IDs on evidence.
- **Honouring `rel=canonical` as a merge:** rejected. Google calls it a hint, and a mis-templated site drops every page but one.
- **Ignoring `rel=canonical`:** rejected. It is free evidence for rules the Frontier learns anyway.
- **No caps by default,** as Scrapy's `DEPTH_LIMIT` and Crawlee's `maxCrawlDepth`: rejected by G5. A trap the learned rule misses runs until a Limit.
- **StormCrawler's repeat rule** (any third occurrence of a segment): rejected. It refuses numeric paths such as `/page/1/tag/1/sort/1`.
- **A URL refused by a cap leaving the Run `complete`:** rejected by G1. A heuristic cut the crawl space, which is what `unverified` says (ADR 0005).
- **A default facet rule** (a per-path variant cap, a learned no-new-links trap or a default per-path bound): rejected. Each one cuts real crawls, as shown above.
- **Each sitemap entry a seed:** rejected by G5. One sitemap would carry a `same_hostname` crawl to any host it names.
- **Admitting an entry when its own host's robots.txt lists the sitemap:** not needed. A robots.txt seed already gives that proof, and checking every entry's host would fetch other hosts' robots.txt files the crawl may never need.
- **Enforcing sitemaps.org's directory rule** (Nutch's strict parsing): rejected. It drops entries Google admits, and the scope already bounds what enters.

## Consequences

- **Amends ADR 0005.**
  - Its fixed rules are written out.
  - Firecrawl's `www`, scheme, trailing-slash and index-file merges become learned rules on evidence.
  - `rel=canonical` and redirects count as evidence.
  - A URL refused by a cap makes a finished Run `unverified`.
  - Notes sit under its summary and its "Fixed rules only" option.
- **Amends ADR 0093.**
  - The engine parses every URL with the WHATWG URL Standard, and a handler's `enqueue` resolves against the document's base URL.
  - The key is made by this ADR's rules and the URL fetched is the URL as parsed.
  - `crawl.keep_url_fragment` is a Label setting, the caps are Run settings, and the not-added reasons gain four values.
  - A redirect to an admitted key ends `skipped` with `duplicate`.
  - The scope ignores one leading `www.` label on its own, and a sitemap seed widens it by the host the program named.
  - Under `same_origin`, a seed's `http` origin also admits its host's `https` origin.
  - The `duplicate` skip reason also covers a redirect reaching a key already admitted.
  - Notes sit under its sitemap seed, its `duplicate` skip reason, its enqueue answers, its Frontier bullet, its scope bullet and its settings table.
- **Adds to ADR 0102.** `crawl.keep_url_fragment` is a key fixed for a Run's life. A note sits under its resume's `settings` bullet.
- **ADR 0037's matrix.**
  - CO-01, CO-05, CO-07, CO-10 and SC-02 cite this ADR. SC-02's sitemap filtering is now this ADR's rule.
  - CO-08, SC-03 and SC-05 keep `spec` and cite it:
    - CO-08: facets get no default rule;
    - SC-03: the Frontier's half of the scheme rule;
    - SC-05: the URL caps.
  - No row changes status, so the counts stay.
  - Areas 4, 6 and 11 list it.
- **`TESTING.md`** gains the URL identity conformance cases.
- **Who builds what.**
  - **Area 6 (Collection)** builds the parser's use, the key, the learned merges and their evidence, the fragment setting, the caps and the sitemap scope.
  - **Area 4 (Security and egress)** builds the parsed host's use by the domain policy and the scope.
  - **Area 11 (Client surfaces)** builds the settings and reasons in each SDK.
- **The Proving ground** gains the pages these cases need, beside ADR 0025's crawler traps and duplicate URLs: a hash-routed app, a faceted listing whose robots.txt disallows its facet parameters, a relative-link loop on a host answering 200 for any path, a host where `/a` and `/a/` and `www.` and the bare host serve different pages, and a sitemap cross-submitted from another host's robots.txt.
- **The pitfall catalog.** Rows U02, U05, U06, U07, U09, U10 and U20 close as Handled, each citing this ADR and its `TESTING.md` cases.
- **The `prototype/sdk-api` sketch** holds the four reasons, the four settings and the `equivalent_form` learned rule in each language.
- **[#549](https://github.com/goodfetch/goodfetch/issues/549)** keeps redirect hop limits and loops; a redirect reaching an admitted key is this ADR's `duplicate`.
