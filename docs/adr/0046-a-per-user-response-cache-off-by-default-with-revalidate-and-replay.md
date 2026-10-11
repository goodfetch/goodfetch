# A per-user response cache, off by default, with revalidate and replay modes on every tier

Responses can be reused across runs through a response cache kept per user on the machine, outside any run's file. It is off by default. In `revalidate` mode it follows HTTP caching rules (RFC 9111): a response is reused while fresh and otherwise checked with a conditional request. In `replay` mode, meant for development, everything is stored and reused without expiry. Both modes work on HTTP tiers and in browsers. The key never lets a response cross Sessions, countries, languages or devices, the cache never holds blocks, Effects, payments or secrets, and Schedules never use `replay`. ADR 0002 keeps a run's state in its own file; the cache is an optimization outside it, so a run resumed on another machine simply misses it (parity row CO-31). Decided in [Network defaults](https://github.com/Jbmanllr/automan/issues/118).

Scrapy's `HttpCacheMiddleware` stores everything and never revalidates by default (`DummyPolicy`), keys on a request fingerprint, and skips expired entries without ever deleting them, so its cache grows without bound ([docs](https://docs.scrapy.org/en/latest/topics/downloader-middleware.html), [httpcache.py](https://github.com/scrapy/scrapy/blob/master/scrapy/extensions/httpcache.py)). Spider's cache is on by default, through `http-cache-reqwest`, keyed on method, URL, the `Authorization` token and an optional namespace ([Cargo.toml](https://github.com/spider-rs/spider/blob/main/spider/Cargo.toml)). Firecrawl serves pages up to two days old by default (`maxAge`; [docs](https://docs.firecrawl.dev/features/fast-scraping)). Crawlee has no response cache ([crawlee#1669](https://github.com/apify/crawlee/issues/1669)). Playwright replays HAR files matched on URL, method and POST body ([docs](https://playwright.dev/docs/mock#replaying-from-har)). None separates responses by proxy country or language.

## Considered Options

- **Watches and the Baseline only** (ADR 0036): rejected. Users writing handlers re-fetch every page on every try, which costs time and proxy bytes and burdens the site.
- **A cache on by default** (Spider, Firecrawl): rejected. Stale pages would appear in production runs with no request made.
- **`http-cache-reqwest` as the cache**: rejected. It covers reqwest only, not wreq or the browser tiers, and its stores document no eviction ([docs](https://docs.rs/http-cache-reqwest/latest/http_cache_reqwest/)).
- **A key of method and URL with an optional namespace** (Spider): rejected. Geo-dependent or logged-in content would leak between runs.
- **Keying on the Frontier's "same page" rules**: rejected. They decide what to visit and may merge URLs whose content differs.
- **Unbounded storage** (Scrapy): rejected. It fills disks and keeps personal data with no end.

## Consequences

**Modes.**
- `cache.mode` is `off` (the default), `revalidate` or `replay`, a setting through ADR 0028's layers.
- `revalidate` decides freshness with `http-cache-semantics` (BSD-2-Clause, maintained) and revalidates with `ETag` and `Last-Modified`. It stores GET and HEAD responses only.
- `replay` stores and reuses every response it sees, without expiry, including POST, PUT and PATCH, keyed by a hash of the canonicalized body (JSON key order, form field order), so the development loop also covers discovered APIs (ADR 0020).
- In browsers, the engine's request interception serves cached responses and sends the conditional requests: CDP `Fetch.fulfillRequest` in Chromium, Playwright's `route.fulfill` in Firefox and WebKit.

**Key.**
- The request part is the method, the exact URL without its fragment, the body hash in `replay`, and the request headers the response names in `Vary`.
- The partition part is public or Session. A request sent with no cookies and no credentials, whose response sets no cookie and is not `private`, goes to the public partition. Everything else goes to the partition of the Session, when it is saved for reuse across runs (ADR 0017); the entries of a Session that is not saved die with it.
- Every partition also carries the person on a shared Daemon (ADR 0031), the proxy exit country, the Identity profile's language and its device class (desktop or mobile).

**Never cached.**
- Responses the rules class as a block, a challenge, a soft block, `payment_required` (ADR 0045) or a decoy (ADR 0035), and responses whose Page check fails.
- Requests the Effect journal records as Effects (ADR 0010), paid fetches (ADR 0045), and requests carrying a resolved secret (ADR 0016).

**Storage and retention.**
- A SQLite index, using the SQLite the engine already bundles, over content-addressed files, kept per user beside the Usage ledger (ADR 0041).
- `cache.max_size` defaults to 10 GB per user, with least-recently-used eviction. `cache.max_age_ceiling` defaults to 30 days and deletes older entries in every mode. Operator policy can cap or lock both.
- `goodfetch cache stats`, `goodfetch cache clear` (everything, or by domain, run or partition) and `goodfetch cache verify`, with the same information in the Console under Engine & settings.
- ADR 0016's erasure operation also removes matching cache entries.

**Results.**
- A cache hit counts as a fetch for Completeness (ADR 0005). Provenance records that the Record came from the cache, with the original fetch time and the run that fetched it.
- A run that used `replay` is marked so in its result, in the Console and in the CLI.
- Usage counts cache hits apart from fetches, so request counts stay honest.

**Schedules.**
- Watches (ADR 0036) never use the cache; they exist to see changes and send their own conditional requests.
- A Schedule's target runs may use `revalidate`. A Schedule whose target sets `replay` is refused with the Error code `replay_not_allowed`, because replayed pages would hide changes from the Baseline.
