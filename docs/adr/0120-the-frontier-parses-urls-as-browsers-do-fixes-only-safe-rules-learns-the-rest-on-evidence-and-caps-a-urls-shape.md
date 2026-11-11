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

Decided in [URL identity and crawl-space rules beyond ADR 0005's learned rules](https://github.com/goodfetch/goodfetch/issues/551), raised by the [Common pitfalls catalog](https://github.com/goodfetch/goodfetch/issues/547) on the map [The SDK API of Collie and Foxhound](https://github.com/goodfetch/goodfetch/issues/461). Every answer was decided by the agent on the owner's delegation, with its [grilling record](https://github.com/goodfetch/goodfetch/issues/551#issuecomment-6204435654), its [first addendum](https://github.com/goodfetch/goodfetch/issues/551#issuecomment-6204745613) and its [second addendum](https://github.com/goodfetch/goodfetch/issues/551#issuecomment-6204877870). It extends ADR 0093, which the owner confirmed, and reverses none of it. Principles are cited by their IDs in [ADR 0077](0077-the-sdk-api-follows-eight-guarantees-and-four-ranked-shape-rules.md).

## Decision

### One parser, in the engine

- **Every URL a crawl meets is parsed by the WHATWG URL Standard's basic URL parser, as the `url` crate implements it.** That covers a seed, an `addSeed`, an `enqueue`, an `enqueueLinks` `href`, a sitemap entry, a redirect's `Location` and a Hook's rewrite (ADR 0104).
  - **Why browsers' parser.** The parser strips leading and trailing spaces and controls and removes tabs and newlines. It reads `\` as `/`, lowercases the scheme and host, and drops the default port. It reads an empty path as `/`, resolves dot segments (`%2e` forms too) and percent-encodes what each component's set requires. So the crawl sees the links a user's browser sees (R 1.13).
  - **Hosts.** A host is percent-decoded, mapped to ASCII by UTS #46 (punycode), and an IPv4 address in hex, octal or shortened form becomes dotted decimal (R 1.12). Politeness's domain key (ADR 0004), robots.txt's origin (ADR 0086), the scope (ADR 0093) and the domain policy (ADR 0017) all read this parsed host. `bücher.example` and `xn--bcher-kva.example` are then one host everywhere.
  - **On every tier.** The engine resolves the `href` attribute's own value, not the browser's resolved link, so a link's identity never depends on the tier that served the page (G1, and ADR 0093's one handler on every tier).
- **The base.**
  - An `href` and a handler's `enqueue` resolve against the document's base URL, as ADR 0093's `enqueueLinks` does, so a relative URL in `enqueue` works as Scrapy's `response.follow` does.
  > Note (agent decision, accepted by the owner on 2026-10-21), 2026-10-21, [#828](https://github.com/goodfetch/goodfetch/issues/828): until the engine serves a handler's page (area 5), `crawl_request.enqueue` carries the document it resolves against, `document`: the URL it was reached at after its redirects and its first `<base>` element's `href` as written; the engine resolves the `href` against that URL as HTML sets a document's base URL (a `href` that does not parse, or a `data:` or `javascript:` one, leaves the document's URL), and without a `document` the crawl request's own URL is the base. Until [#869](https://github.com/goodfetch/goodfetch/issues/869) stages an attempt's enqueues, an enqueue commits at once while its attempt's lease holds, and one from an attempt that ended fails with `attempt_ended`; until [#832](https://github.com/goodfetch/goodfetch/issues/832), a request's identity is its URL as parsed, kept exact by a unique index in the run file.
  - An `href`'s query is percent-encoded in its document's encoding, as the HTML standard's URL parsing does, and that encoding is the one [ADR 0119](0119-every-fetch-tier-decodes-a-crawls-document-as-the-browser-does-a-label-names-the-content-types-its-handler-takes-and-exports-are-utf-8.md) decodes the document with. A handler's `enqueue`, a seed and a sitemap entry use UTF-8.
  - A sitemap entry resolves against its sitemap's URL.
  - A `Location` resolves against its request's URL.
  - A seed and `addSeed` take an absolute URL.
- **What is refused.** A URL that fails to parse is not added, with the reason `invalid_url` (G1). A scheme other than `http` or `https` is ADR 0093's `unsupported_scheme`.

### A crawl request's identity

- **The key, not the fetched URL.** ADR 0093's identity is the method, the URL's key and a hash of the body.
  - The key is made by the rules below. The request is fetched at its URL as parsed, and its outcomes name that URL: parameters are not sorted, `utm_*` is not stripped, and no fragment is sent, since a fragment never reaches a server.
  - When two URLs share a key, the first one admitted is fetched.
  - A declared `key` still replaces the identity (ADR 0093).

  > Note (agent decision, accepted by the owner on 2026-10-22), 2026-10-21, [#832](https://github.com/goodfetch/goodfetch/issues/832): a seed and an enqueue carry `method` (an RFC 9110 token, `GET` by default, normalized as Fetch normalizes one, `CONNECT`, `TRACE` and `TRACK` refused), `body` (its bytes in Base64 with padding, as a page's body chunks are) and `key`, and a crawl request reaches its handler with its method and body. The body's hash is its SHA-256, and an empty body counts as none. A declared `key` is kept apart from every identity the Frontier makes, so a key that spells a method and a URL never merges with that request. The tracking parameter names are matched as written (`utm_` as a prefix, `gclid` and `fbclid` whole), never in another case, so a parameter that only resembles one stays. The Frontier keeps its visited set in the run file, each identity with the crawl request it was admitted for, so the duplicate check holds nothing in memory. Until the fetch tiers follow redirects ([#759](https://github.com/goodfetch/goodfetch/issues/759), [#762](https://github.com/goodfetch/goodfetch/issues/762)), the Frontier's redirect check is an interface no tier calls yet: each hop asks it whether its key was admitted for another request, and a request whose hop it answers `duplicate` ends `skipped` with `duplicate` before its handler runs; the final URL is then recorded under its key.

  > Note (owner decision D10 of the deep review, 2026-10-28; recorded 2026-10-30 by G23): the final URL's record checks again, in the same write, whether another request was admitted for its key or reached it since the hop was checked, and the request then ends `skipped` with `duplicate`, its handler not run. So of two chains that reach one key at once, only the one whose document arrives first is handled. The key is not claimed at the hop, so a chain that fails holds nothing.
- **The fixed rules, in order.** None merges URLs that RFC 3986 or HTTP says may differ. The path keeps its case, since paths are case-sensitive (R 1.10).
  1. **The parser's own work**, above.
  2. **Percent-encoding** (RFC 3986 §6.2.2). In the path and the query, hexadecimal digits are uppercased, and percent-encoded unreserved characters (`A`–`Z`, `a`–`z`, `0`–`9`, `-`, `.`, `_`, `~`) are decoded. A reserved character such as `%2F` stays encoded, and `+` stays apart from `%20`.
     > Note (agent decision, accepted by the owner on 2026-10-22), 2026-10-22: a `%` not followed by two hexadecimal digits stays as written, as the browsers' parser keeps it, and an encoded hexadecimal digit right after such a `%`, or after it and one digit, stays encoded, so decoding never makes a percent-encoding the URL did not hold. `?%3%30` then keeps apart from `?%30`, as the WHATWG URL Standard's one-pass percent-decoding keeps them (`%30` against `0`), and the key of a key is itself. Writing such a `%` as `%25` instead was declined: it would merge `?%3%30` with `?%2530`, which a server reading the request target as sent may tell apart.
  3. **The fragment is dropped**, unless the request's Label keeps it (below).
  4. **ADR 0005's parameter rules:**
     - Crawlee's `utm_*` and Crawl4AI's `gclid` and `fbclid` are dropped;
     - the remaining pairs are sorted by name, a stable sort that keeps the order of values within one name, so `?a=2&a=1` stays apart from `?a=1&a=2`;
     - empty pairs (`&&`, a leading or trailing `&`) and an empty query are dropped, as Crawlee's sort and Nutch's normalizer drop them.
  5. **The Servlet session parameter.** A `;jsessionid=…` path parameter, its name matched in any case, is dropped, since the Servlet specification reserves that name for session IDs. Every other `;` parameter stays part of the path. Query-string session IDs are ADR 0005's learned no-effect parameters, so `sid=`, which names a story on some sites, is merged only on evidence.
     > Note (agent decision, accepted by the owner on 2026-10-22), 2026-10-22: a segment that dropping the parameter would leave as `.` or `..` keeps it, so `a/.;jsessionid=1/b` stays apart from `a/b` rather than becoming a dot segment the parser resolves, and the key of a key is itself.
- **Firecrawl's merges are learned per host, on evidence.** One leading `www.` label against the bare host, `http` against `https`, a trailing slash, and a final `index.html` or `index.php` against the folder: each is merged once evidence shows the two forms are one resource, as ADR 0005 learns no-effect parameters.
  - **The evidence:**
    - a redirect from one form to the other, the evidence RFC 3986 §6.2.4 names;
    - matching page signatures for the two forms;
    - `rel=canonical` (below).
  - **What a learned merge does.** Like every learned rule, it is kept in run state with its evidence and the number of URLs it merged, it is reported (`equivalent_form` in the Run report's learned rules), and it never crosses a registrable domain (ADR 0005).
    - A pending request it merges ends `skipped` with ADR 0093's `duplicate`.
    - Whether the slash and index-file merges hold per host or per path, and how much evidence each kind needs, are area 6's spec, like ADR 0005's other thresholds.
    > Note (agent decision, on a guarantee, not read by the owner), 2026-10-24, [#856](https://github.com/goodfetch/goodfetch/issues/856): spec #581's thresholds are built as written, each form per host (the `www` form on the bare host, and never when the bare host is a public suffix), learned only from a `GET` with no body. A form or a parameter is never learned once a pair of its pages differs, whatever evidence follows, and a learned one that meets such a pair stops for the rest of the Run: the report gives the pair as `contradiction`, keeps the URLs it had merged, and requests it had already ended stay ended. `rel=canonical` counts toward the forms alone, as the spec's threshold names; a canonical that drops a parameter teaches nothing yet. A pending request a rule learned later merges with another is ended in the same commit as the learning, keeping the request already fetched or being fetched, or else the first in the crawl's order, and leaves its domain's queue. The report's `form` names the form, and a URL two rules merged together is listed under each. Until the fetch tiers follow redirects ([#762](https://github.com/goodfetch/goodfetch/issues/762)), the redirect evidence is an interface no tier calls yet, `learn_redirect`, which each hop is to call.
    > Note (agent decision, on a guarantee, not read by the owner), 2026-11-11, [#2310](https://github.com/goodfetch/goodfetch/issues/2310): the requests a stopped rule had ended no longer stay ended. When a contradicting pair stops a rule, each request that a merge relying on it had ended `skipped` with `duplicate`, and that is still so, is pending again in the same commit, with its ID, position and domain key, its outcome out of the ended requests' counts until it ends again, as a re-admitted request's is (ADR 0102); a merge relies on every rule that changed the identity of one of the requests it showed to be one page, so a request two rules merged together is given back when either stops. A URL the rule had refused at admission as a `duplicate` belongs to no request and cannot be given back: it counts as lost until the crawl meets it again and adds it. What a stopped rule had merged and the crawl did not get back, such a URL or a request the crawl could not make pending again, is the Completeness reason `learned_rule_stopped`, one for each such rule, naming it with its count, which leaves a finished Run `unverified`. The report still lists every URL the rule had merged.
    > Accepted by the owner on 2026-11-10 ([#2253](https://github.com/goodfetch/goodfetch/issues/2253), Q6): a form or a parameter is never learned once a pair of its pages differs, and once it is refused or stopped it takes no more evidence ([#2287](https://github.com/goodfetch/goodfetch/issues/2287)).
  - **Why.** A fixed merge that is wrong is silent: the second page is never fetched, and nothing says so (G1). The cost is the first fetch of each other form on a host. On most sites that fetch answers with a redirect, which is itself the evidence.
- **The scope ignores one leading `www.` label on its own.** ADR 0093 compares scope hosts "after the Frontier's fixed URL rules, so `www.` is no other host". That stays true: comparing hosts for scope ignores one leading `www.` label as a scope rule. Under `same_origin`, which also compares the scheme and port, a seed's `http` origin also admits the `https` origin of its host, since sites upgrade and the old fixed scheme merge admitted it; never the reverse, a downgrade. The scope therefore does not change while identity waits for evidence.
- **`rel=canonical` is evidence, never a merge.** A page's canonical is the first `<link rel="canonical">` in its `<head>`, or a `Link` header with `rel="canonical"`. It counts as evidence for a learned rule when it differs from the page's URL only by what such a rule covers: a parameter, `www.`, the scheme, a trailing slash or an index file. It never merges anything by itself. A canonical to another path or another registrable domain is ignored for identity, since a mis-templated site may point every page at its home page (R 1.15). The canonical URL is not enqueued for the program: a handler that wants it reads the tag and enqueues it.
- **A redirect to a URL the Frontier already holds.** When a request's redirect reaches a URL whose key the Frontier already admitted for another request, the request ends `skipped` with `duplicate` and its handler does not run, as Scrapy's duplicate filter drops it. A hop back to the request's own key, or to a URL earlier in its own chain, is followed, since a site may set a cookie and redirect back by design, and `fetch.redirect.max` counts it. The redirect still counts as evidence for a learned merge. A redirect's final URL is recorded under its key as visited, so a later enqueue of it is a `duplicate` too. A `Location` that does not parse fails at once with `response_not_ok`, and `fetch.redirect.max` bounds hops and loops ([ADR 0118](0118-every-fetch-tier-follows-redirects-as-the-browser-does-fails-a-cut-body-and-resends-only-what-a-site-never-processed.md)).

### Fragments

- **`crawl.keep_url_fragment`**, false by default, and settable at connection, Run and Label scope, keeps the fragment in the key of that scope's requests. The name is Crawlee's `keepUrlFragment` (S3). Crawlee and Scrapy agree on dropping fragments by default.
  - The key of a request is made by the rules its Label sets, so one URL with two fragments is one request on a Label that drops them, and two on a Label that keeps them.
  - The browser navigates to the URL with its fragment. The fragment is still never sent to a server.
- **Only tiers that run the page's scripts serve such a Label:** `light_browser` and `browser`. On any other tier, every fragment would fetch the same document again (G5).
  - Under `fetch.tier: auto`, the sample comparison chooses between those two.
  - Forcing `http`, `browser_fetch` or `discovered_api` on that Label fails the Run's start with `setting_not_applicable`, as ADR 0110 refuses kept responses under a forced `http` or `browser_fetch`. `discovered_api` is refused here too, though ADR 0110 admits it: its replayed calls cannot tell which call a fragment's view makes without running the page's scripts.
- **Fixed for a Run's life.** `crawl.keep_url_fragment` is one of ADR 0102's keys fixed for a Run's life, since the keys of the requests already admitted would no longer match the new ones. A resume that changes it raises `setting_not_applicable`.
  > Note (agent decision, on a default, not read by the owner), 2026-10-25, [#939](https://github.com/goodfetch/goodfetch/issues/939): a request keeping its fragment has the key of its other rules followed by `#` and the fragment. Which requests keep it, the Run's value and each Label's own, is recorded in the run file when the Run begins and read back on every admission and resume; a `crawl.start` under the Run's ID whose values differ is refused with `setting_not_applicable`. Under `fetch.tier: auto`, and at the built-in default, such a request runs on `browser` (no `http` sample) until `light_browser` is built. A sitemap's entries and a redirect hop's key drop the fragment, as an HTTP redirect's fragment never reaches the server.

### Caps on a URL's shape

- **Three settings, on by default (G5),** at connection and Run scope, as `crawl.max_depth` is (ADR 0093):

  | Key | Built-in | Refuses |
  |---|---|---|
  | `crawl.max_url_length` | 2,083 | a URL longer than this many characters, as parsed and without its fragment (Scrapy's `URLLENGTH_LIMIT`) |
  | `crawl.max_path_segments` | 20 | a path of more segments, as the parser splits it, an empty last segment (a trailing slash) not counted (Heritrix's `maxPathDepth`) |
  | `crawl.max_segment_repeats` | 2 | a segment, or a run of segments, that appears more than this many times back to back, so `/a/a` passes and `/a/a/a` does not (Heritrix's `maxRepetitions`) |

  - `crawl.max_segment_repeats` counts as Heritrix's `maxRepetitions` pattern does, a run of segments as well as one segment: `/a/a/a`, and the `/a/b/a/b/a/b` loops that relative links make on a site answering 200 for any path, Google's `../../category/stuff`. Numbers that recur apart, as in `/page/1/tag/1/sort/1`, are not caught, though StormCrawler's any-third-occurrence rule would refuse them.
  - `null` turns a cap off.
- **Where they apply.** They are checked on what enters the Frontier (seeds, enqueues, sitemap entries), after the scope and the domain policy, so only a URL the crawl would otherwise admit is refused by a cap, and an over-long link to another host stays `out_of_scope`. Redirects are not checked, since `fetch.redirect.max` bounds them ([ADR 0118](0118-every-fetch-tier-follows-redirects-as-the-browser-does-fails-a-cut-body-and-resends-only-what-a-site-never-processed.md)).
- **The answer.** A URL past a cap is not added, with the setting's name as its reason (`max_url_length`, `max_path_segments`, `max_segment_repeats`), as ADR 0093 names `max_depth`.
- **Completeness.** A finished Run that refused a URL by a cap is `unverified`, since a heuristic cut its crawl space (ADR 0005). Its report's Completeness reasons name the setting and how many URLs it refused.
  > Note (agent decision, on a default, not read by the owner), 2026-10-22, [#843](https://github.com/goodfetch/goodfetch/issues/843): the caps are checked after the scope, the identity check and ADR 0093's bounds, so a URL refused by a cap is one the crawl would otherwise have admitted, and a duplicate is never counted. A URL's length is its characters as the parser serializes it, its fragment aside. `crawl.max_segment_repeats` refuses a run of `w` segments repeated more than the cap back to back, found as `w × cap` segments in a row each equal to the one `w` before; a cap of 0 refuses any path with a segment. Each refusal is counted by its setting in the run file, and a finished Run is `unverified` and lists `{code: "crawl.max_segment_repeats", message}` among its `completeness_reasons` (ADR 0102's reason shape), the message giving the count.

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
  > Note (agent decision, on a guarantee, not read by the owner), 2026-10-24, [#847](https://github.com/goodfetch/goodfetch/issues/847): `crawl.path_budgets` takes each entry's `pattern` as a glob, as `crawl.scope.*` takes theirs (a regular expression form waits for the SDKs' `LinkPattern` mapping, area 11), matched against the URL's path and query as the WHATWG parser serializes them (`/shop?*`). It is checked after `crawl.max_requests` and before the caps, on what the crawl would otherwise admit, seeds included; each entry counts what it admitted and refused by its pattern in the run file, so a resume that changes an entry's count keeps what it admitted. Each entry that refused a URL is a Completeness reason of its own, `{code: "crawl.path_budgets", message}`, the message naming the pattern, its count and how many it refused. An unreadable pattern fails the start with `pattern_unsupported`, and a resume with one ends the Run `failed`.

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
- **No URL-shape caps by default,** as Scrapy's `DEPTH_LIMIT` and Crawlee's `maxCrawlDepth` leave depth open (Scrapy's `URLLENGTH_LIMIT` is on): rejected by G5. A trap the learned rule misses runs until a Limit.
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
  - A redirect to a key admitted for another request ends `skipped` with `duplicate`.
  - The scope ignores one leading `www.` label on its own, and a sitemap seed widens it by the host the program named.
  - Under `same_origin`, a seed's `http` origin also admits its host's `https` origin.
  - The `duplicate` skip reason also covers a redirect reaching a key already admitted for another request.
  - Notes sit under its sitemap seed, its `duplicate` skip reason, its enqueue answers, its Frontier bullet, its scope bullet and its settings table.
- **Adds to ADR 0102.** `crawl.keep_url_fragment` is a key fixed for a Run's life. A note sits under its resume's `settings` bullet.
- **Amends [ADR 0118](0118-every-fetch-tier-follows-redirects-as-the-browser-does-fails-a-cut-body-and-resends-only-what-a-site-never-processed.md).** ADR 0118 keeps redirect hop limits and loops, a hop back to the request's own URL included, and a `Location` that does not parse; a redirect reaching a key admitted for another request is this ADR's `duplicate`, and the identity a redirect keeps uses the URL's key. A note sits under its "keeps its Frontier identity" bullet.
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
