# The frontier learns which URLs are the same page, and every run reports its completeness

The frontier applies the fixed URL rules competitors use, then learns three more from what it fetches: query parameters that do not change the page (by page signature), trap paths to close, and cursor pagination preferred over offset. Every run reports a completeness of complete, incomplete or unverified, and a run stopped by a Limit is never complete. In the lab, the naive frontier ran out of budget on a trap site where the learned one finished in 75 requests, and offset pagination over a table modified during the crawl lost rows while reporting success 15 times out of 15. Decided in [Adopt collection decisions](https://github.com/Jbmanllr/automan/issues/71).

> Amended by [ADR 0093](0093-a-crawl-declares-its-labels-and-one-handler-each-and-a-handler-reads-collies-page-on-every-fetch-tier.md): a crawl request's identity in the Frontier is its method, its URL after these rules and a hash of its body, as Crawlee's `useExtendedUniqueKey`, so a POST to one URL with two bodies is two requests; a declared `key` still wins.

Competitor evidence: [frontier and sessions research](https://github.com/Jbmanllr/automan/blob/research/frontier-sessions-competitors/rnd/research/2026-10-10-frontier-sessions-competitors.md).

## Considered Options

- **Fixed rules only**: rejected. None of the nine competitors surveyed (Crawlee, Scrapy, Spider, Firecrawl, Crawl4AI, Stagehand, Browser Use, Skyvern, Apify) learns traps or no-effect parameters. Their fixed rules differ (Crawlee strips `utm_*` and sorts parameters, Firecrawl merges `www`, scheme, trailing slash, `index.html` and `index.php`, Crawl4AI also strips `gclid` and `fbclid`); Goodfetch applies all of them first, and learns on top.
- **Canonicalization declared only by the run's code**: kept as an override, not as the only mechanism. It suffices for a known site, not for discovery.
- **Success or failure only, as competitors report**: rejected. Crawlee's `run()` returns statistics without a stop reason and only logs a limit stop, and Firecrawl reports `completed` at its page limit; only Scrapy's `finish_reason` names a limit, and it still reports a depth cutoff as finished.

## Consequences

- Each learned rule is kept in run state with its evidence and the number of URLs it merged, and is reported; rules never cross registrable domains. Declared canonicalization always wins.
- A wrongly learned no-effect parameter merges distinct pages. The report makes that visible, and a conformance fixture covers it.
- Unverified covers a frontier exhausted only through offset pagination or a heuristic.
- Unverified also covers a finished run that left pages in scope at the Block cause `payment_required`, which it lists (ADR 0045).
- The same holds for pages left at the Block cause `consent_required` (ADR 0048).
- The frontier also takes priority scores and forefront insertion, for parity with Spider and Crawlee.
