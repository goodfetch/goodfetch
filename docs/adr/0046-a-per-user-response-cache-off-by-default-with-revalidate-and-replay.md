# A per-user Response cache, off by default, with revalidate and replay modes on every tier

Responses can be reused across runs through a Response cache kept per user on the machine, outside any run's file. It is off by default. In `revalidate` mode it follows HTTP caching rules (RFC 9111): a response is reused while fresh and otherwise checked with a conditional request. In `replay` mode, meant for development, everything is stored and reused without expiry. Both modes work on HTTP tiers and in browsers. A response sent with or setting cookies or credentials, or marked `private`, never leaves its saved identity, or its Session when it has none, and no response crosses persons, countries, languages or devices. The cache never holds blocks, Effects, payments or secrets, discarding a run removes what it cached, and Schedules never use `replay`. ADR 0002 keeps a run's state in its own file; the cache is an optimization outside it, so a run resumed on another machine simply misses it (parity row CO-31). Decided in [Network defaults](https://github.com/goodfetch/goodfetch/issues/118).

Scrapy's `HttpCacheMiddleware` stores everything and never revalidates by default (`DummyPolicy`), keys on a request fingerprint, and skips expired entries without ever deleting them, so its cache grows without bound ([docs](https://docs.scrapy.org/en/latest/topics/downloader-middleware.html), [httpcache.py](https://github.com/scrapy/scrapy/blob/master/scrapy/extensions/httpcache.py)). Spider's cache is on by default, through `http-cache-reqwest`, keyed on method, URL, the `Authorization` token and an optional namespace ([Cargo.toml](https://github.com/spider-rs/spider/blob/main/spider/Cargo.toml)). Firecrawl serves pages up to two days old by default (`maxAge`; [docs](https://docs.firecrawl.dev/features/fast-scraping)). Crawlee has no response cache ([crawlee#1669](https://github.com/apify/crawlee/issues/1669)). Playwright replays HAR files matched on URL, method and POST body ([docs](https://playwright.dev/docs/mock#replaying-from-har)). None separates responses by proxy country or language.

## Considered Options

- **Watches and the Baseline only** (ADR 0036): rejected. Users writing handlers re-fetch every page on every try, which costs time and proxy bytes and burdens the site.
- **A cache on by default** (Spider, Firecrawl): rejected. Stale pages would appear in production runs with no request made.
- **`http-cache-reqwest` as the cache**: rejected. It covers reqwest only, not wreq or the browser tiers, and its stores document no eviction ([docs](https://docs.rs/http-cache-reqwest/latest/http_cache_reqwest/)).
- **A key of method and URL with an optional namespace** (Spider): rejected. Geo-dependent or logged-in content would leak between runs.
- **Keying on the Frontier's "same page" rules**: rejected. They decide what to visit and may merge URLs whose content differs.
- **Unbounded storage** (Scrapy): rejected. It fills disks and keeps personal data with no end.
- **Entries that outlive a discarded run**: rejected. ADR 0001 lets only saved identities outlive a reset or a discard.

## Consequences

**Modes.**
- `cache.mode` is `off` (the default), `revalidate` or `replay`, a setting through ADR 0028's layers.
- `revalidate` decides freshness with `http-cache-semantics` (BSD-2-Clause, maintained) and revalidates with `ETag` and `Last-Modified`. It stores GET and HEAD responses only.
- `replay` stores and reuses every response it sees, without expiry, including POST, PUT and PATCH, keyed by a hash of the canonicalized body (JSON key order, form field order), so the development loop also covers discovered APIs (ADR 0020).
  > Amended by [ADR 0105](0105-a-programs-tests-replay-a-response-recording-in-a-test-engine-that-reaches-no-site.md): `replay` stays the development loop. A program's tests replay a Response recording, a WARC file its Run's start names, which answers exactly, never expires and never fetches a missing response.
- In browsers, the engine's request interception serves cached responses and sends the conditional requests: CDP `Fetch.fulfillRequest` in Chromium, Playwright's `route.fulfill` in Firefox and WebKit.

**Key.**
- The request part is the method, the exact URL without its fragment, the body hash in `replay`, and the request headers the response names in `Vary`.
- The partition part is public or Session. A request sent with no cookies and no credentials, whose response sets no cookie and is not `private`, goes to the public partition. Everything else goes to the partition of the Session's saved identity (ADR 0035); the entries of a Session with no saved identity die with the Session.
- Every partition also carries the person on a shared Daemon (ADR 0031), the proxy exit country, the Identity profile's language and its device class (desktop or mobile).

**Never cached.**
- Responses with any Block cause (`challenge`, `captcha`, `rate_limit`, `ip_block`, `decoy`, `payment_required`, `consent_required`; ADRs 0035, 0045 and 0048), responses the Decision provider classes as a soft block (ADR 0021), and responses whose Page check fails.
  > Amended by [ADR 0118](0118-every-fetch-tier-follows-redirects-as-the-browser-does-fails-a-cut-body-and-resends-only-what-a-site-never-processed.md): nor a cut response, whose body ended before its length, its last chunk or its stream's end.
- Requests the Effect journal records as Effects (ADR 0010), Paid fetches (ADR 0045), and requests carrying a resolved secret (ADR 0016).
- Responses fetched under a certificate exception (`trusted_cas`, `client_certificates` or `accept_invalid_certs`; ADR 0042), so a run that verifies a host normally is never served what another run accepted without verification.

**Storage and lifetime.**
- **Changes ADR 0002:** reusable responses live outside run state, in a SQLite index, using the SQLite the engine already bundles, over content-addressed files, kept per user and per product major, as saved identities are (ADR 0035), so an upgrade starts with an empty cache. **Changes ADR 0030:** within a major, an engine that finds a cache format newer than it reads runs without the cache, neither reading nor writing it, so two minors never wipe each other's entries.
- Each entry records the run that fetched it. Resetting or discarding a run deletes the entries it fetched, so the cache keeps ADR 0001's rule. Deleting a saved identity deletes its partition. In a Daemon group, the Lead Daemon broadcasts these deletions to every Group member's cache (ADR 0055).
- `cache.max_size` defaults to 10 GB per user, with least-recently-used eviction. `cache.max_age_ceiling` defaults to 30 days and deletes older entries in every mode. Operator policy can cap or lock both.
  > Amended by [ADR 0122](0122-a-run-stops-resumably-before-its-disk-fills-and-a-daemons-logs-rotate-at-a-bounded-size.md): while its filesystem is at the `low` or `critical` storage level, the cache stores no new entry and evicts nothing beyond its own cap.
- `goodfetch cache stats`, `goodfetch cache clear` (everything, or by domain, run or partition) and `goodfetch cache verify`, with the same information in the Console under Engine & settings.
  > Amended by [ADR 0137](0137-the-console-leaves-both-products-and-the-engine-and-the-owners-monitoring-tool-is-designed-apart.md): no Product carries a Console. The engine behaviour this ADR describes stands and the Console's part does not; a Client reaches it.
- **Changes ADR 0016:** its erasure operation also removes matching cache entries.

**Results.**
- A cache hit counts as a fetch for Completeness (ADR 0005). Provenance records that the Record came from the cache, with the original fetch time and the run that fetched it.
- A run that used `replay` is marked so in its result, in the Console and in the CLI.
  > Amended by [ADR 0137](0137-the-console-leaves-both-products-and-the-engine-and-the-owners-monitoring-tool-is-designed-apart.md): no Product carries a Console, so what this says of the Console ends; the rest stands.
- Usage counts cache hits apart from fetches, so request counts stay honest.

**Schedules.**
- **Changes ADR 0036:** Watches never use the cache; they exist to see changes and send their own conditional requests. A Schedule's target runs may use `revalidate`. A Schedule whose target sets `replay` is refused with the Error code `replay_not_allowed`, which is not retryable (ADR 0038), because replayed pages would hide changes from the Baseline.
- Saving an identity drops, from its partition, the entries fetched under a Consent prompt acceptance that the saving run's `consent_domains` does not cover (ADR 0048).
