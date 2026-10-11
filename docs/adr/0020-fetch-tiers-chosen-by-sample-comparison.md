# Fetch tiers are chosen per route by comparing a sample with the browser

The engine fetches each page through one of four tiers: HTTP; a discovered API, which replays the JSON calls captured while the browser loaded a sample; the browser; and browser-fetch, HTTP sent from inside the Session's browser so that its identity does not change. It chooses per route by fetching a sample both ways and comparing the results with the browser's, and a declared total that no longer matches invalidates the choice. In the lab, the compared sample was exact on 120 of 120 runs and cost 0.004 of the browser's time on a static page, and the discovered API was exact on 120 of 120 and cost 0.008 of the browser's time on GraphQL and infinite scroll ([portal report](https://github.com/Jbmanllr/automan/blob/main/rnd/lab/reports/portal-2026-10-09.md), [quiet-host report](https://github.com/Jbmanllr/automan/blob/main/rnd/lab/reports/quiet-host-2026-10-10.md)). Decided in [Adopt site-access decisions](https://github.com/Jbmanllr/automan/issues/76).

## Considered Options

- **Browser only**: rejected as the default. It is exact but the most expensive tier, by two orders of magnitude on static pages and discoverable APIs.
- **Falling back to the browser when the HTTP page looks empty**: rejected. In the lab this rule declared stale prices and partial pages complete.
- **A light engine as the default** (Lightpanda): rejected. It fails on shadow DOM ([paired light engine](https://github.com/Jbmanllr/automan/blob/main/rnd/lab/reports/light-engine-paired-2026-10-09.md)).

## Consequences

- HTTP transports: reqwest in `standard` and `identified` Sessions; wreq in HTTP-only `stealth` Sessions, with its TLS and HTTP/2 fingerprint matched to the Session's Identity profile (ADR 0035 picks the fingerprints); browser-fetch in any Session that may move to the browser, because a separate client changes the TLS fingerprint at that moment ([identity report](https://github.com/Jbmanllr/automan/blob/main/rnd/lab/reports/identity-2026-10-08.md)). Impit is not shipped: at equal throughput it used about ten times the memory of reqwest, and 1.6 to ten times its CPU per task ([transports](https://github.com/Jbmanllr/automan/blob/main/rnd/lab/reports/http-transport-2026-10-09.md)).
- The engine sets decompression, redirects and timeouts itself, and a conformance gate checks every transport: the lab found reqwest built without gzip and wreq following no redirects by default.
- Discovered APIs are limited to page-number and cursor plans in V1; signed requests and single-use tokens stay in the browser.
- Lightpanda is an optional tier, run out of process and downloaded on demand, never bundled, so its AGPL licence stays outside Goodfetch's distribution. It is chosen only when the run's declared capabilities allow it and its sample matches Chrome's; it used 1/8 to 1/20 of Chrome's CPU in the lab.
- A run can force a tier, which skips the comparison (ADR 0023); Provenance records the tier and whether it was compared or forced.
- A run can instead race browser-fetch against a full navigation, page by page, with the opt-in `tier_race`, judged by the Page check (ADR 0039).
- A block can also move a domain to another tier (ADR 0021).
