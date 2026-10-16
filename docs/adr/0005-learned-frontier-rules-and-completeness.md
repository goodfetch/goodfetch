# The frontier learns which URLs are the same page, and every run reports its completeness

The frontier applies the fixed URL rules competitors use, then learns three more from what it fetches: query parameters that do not change the page (by page signature), trap paths to close, and cursor pagination preferred over offset. Every run reports a completeness of complete, incomplete or unverified, and a run stopped by a Limit is never complete. In the lab, the naive frontier ran out of budget on a trap site where the learned one finished in 75 requests, and offset pagination over a table modified during the crawl lost rows while reporting success 15 times out of 15. Decided in [Adopt collection decisions](https://github.com/goodfetch/goodfetch/issues/71).

> Amended by [ADR 0093](0093-a-crawl-declares-its-labels-and-one-handler-each-and-a-handler-reads-collies-page-on-every-fetch-tier.md): a crawl request's identity in the Frontier is its method, its URL after these rules and a hash of its body, as Crawlee's `useExtendedUniqueKey`, so a POST to one URL with two bodies is two requests; a declared `key` still wins.

> Amended by [ADR 0120](0120-the-frontier-parses-urls-as-browsers-do-fixes-only-safe-rules-learns-the-rest-on-evidence-and-caps-a-urls-shape.md): the fixed rules are written out (the WHATWG parser's own work, RFC 3986's percent-encoding normalization, the fragment dropped, `utm_*`, `gclid` and `fbclid` dropped and the parameters sorted, `;jsessionid=` dropped), and they make a key while the request is fetched at its own URL. Firecrawl's `www`, scheme, trailing-slash, `index.html` and `index.php` merges are no longer fixed: each is learned per host on evidence (a redirect between the forms, matching page signatures, `rel=canonical`) and reported as `equivalent_form`. `rel=canonical` is evidence for a learned rule, never a merge. A finished run that refused a URL by one of the caps on a URL's shape is `unverified`.

Competitor evidence: [frontier and sessions research](https://github.com/goodfetch/goodfetch/blob/research/frontier-sessions-competitors/rnd/research/2026-10-10-frontier-sessions-competitors.md).

## Considered Options

- **Fixed rules only**: rejected. None of the nine competitors surveyed (Crawlee, Scrapy, Spider, Firecrawl, Crawl4AI, Stagehand, Browser Use, Skyvern, Apify) learns traps or no-effect parameters. Their fixed rules differ (Crawlee strips `utm_*` and sorts parameters, Firecrawl merges `www`, scheme, trailing slash, `index.html` and `index.php`, Crawl4AI also strips `gclid` and `fbclid`); Goodfetch applies all of them first, and learns on top.
  > Amended by [ADR 0120](0120-the-frontier-parses-urls-as-browsers-do-fixes-only-safe-rules-learns-the-rest-on-evidence-and-caps-a-urls-shape.md): only RFC 3986's safe rules and the parameter rules stay fixed; Firecrawl's `www`, scheme, trailing-slash, `index.html` and `index.php` merges are learned on evidence.
- **Canonicalization declared only by the run's code**: kept as an override, not as the only mechanism. It suffices for a known site, not for discovery.
- **Success or failure only, as competitors report**: rejected. Crawlee's `run()` returns statistics without a stop reason and only logs a limit stop, and Firecrawl reports `completed` at its page limit; only Scrapy's `finish_reason` names a limit, and it still reports a depth cutoff as finished.

## Consequences

- Each learned rule is kept in run state with its evidence and the number of URLs it merged, and is reported; rules never cross registrable domains. Declared canonicalization always wins.
- A wrongly learned no-effect parameter merges distinct pages. The report makes that visible, and a conformance fixture covers it.
- Unverified covers a frontier exhausted only through offset pagination or a heuristic.
- Unverified also covers a finished run that left pages in scope at the Block cause `payment_required`, which it lists (ADR 0045).
- The same holds for pages left at the Block cause `consent_required` (ADR 0048).
- The frontier also takes priority scores and forefront insertion, for parity with Spider and Crawlee.
