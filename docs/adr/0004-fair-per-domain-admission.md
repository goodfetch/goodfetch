# Fair per-domain admission by default, with fairness before priority

The engine admits work through bounded per-domain queues served round-robin under a global cap, rather than through one global queue: with one heavy domain under overload, small domains got 2.6 times more work in the lab, and Scrapy made a similar change its default in 2.14 after one domain took every global slot. A domain is the registrable domain under the full Public Suffix List, private entries included, and a recipe can override the key. Decided in [Adopt runtime and state decisions](https://github.com/Jbmanllr/automan/issues/70).

## Considered Options

- **Global FIFO queue** (Crawlee v3): rejected as the default. It is simpler, but one dominant domain starves the rest, and with a single site fair admission collapses to the same order anyway.
- **Priority first across domains** (Mercator): rejected. Round-robin picks the domain and priority orders requests within it, so a high priority on one domain never starves another, and cannot jump ahead of it either.
- **Hostname as the key** (Scrapy): rejected, because subdomains of one site usually share one server.

## Consequences

- A domain waiting out a politeness delay or a backoff is not ready and holds no global capacity; the cap counts only requests in flight.
- Per-domain queues are journaled and survive a resume; drained queues are released, so per-domain state stays bounded.
- Each turn admits one request per domain: no weights or larger turns for some domains in V1. Politeness defaults (8 requests in flight per domain, backoff on 429, 503 and `Retry-After`) and the optional per-IP limit are set in ADR 0021.
- The decision rests on the mechanism, head-of-line blocking by one domain, not on the size of the lab effect, whose verdict is still `insufficient-n` (5 of 22 blocks). The V1 benchmark measures the missing blocks; only a measurable slowdown on single-domain crawls would reopen it.
