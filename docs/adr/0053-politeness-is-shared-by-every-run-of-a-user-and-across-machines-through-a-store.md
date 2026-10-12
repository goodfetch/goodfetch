# Politeness is shared by every run of a user, and across machines through an optional Politeness store

ADR 0021's per-domain limits (8 requests in flight, `Retry-After`, backoff from 2 s to 60 s, a 429 halving concurrency) lived in each run's own state (ADR 0004). Three runs crawling one site therefore sent it 24 requests at once, and a 429 seen by one run slowed none of the others. Pain #15 (`ops-scale-ratelimit-scheduling`) is this: crawlee#4039 kept hammering a domain despite its backoff, crawlee-python#1437 fell into a "death spiral" of 429s, and a potential user ruled out an engine "because it lacks built-in distributed rate limiting" (hn-102). **Politeness**, the per-domain and per-proxy limits, is now shared by every run of a user on a machine, whichever engine runs it, and across machines through an optional **Politeness store** that speaks the Redis protocol. Decided in [Scaling past one engine](https://github.com/Jbmanllr/automan/issues/116).

How comparable tools share their limits:
- **Inside one process only:** Crawlee's `sameDomainDelaySecs` and its newer `ThrottlingRequestManager` keep their clocks in an in-memory `Map` ([source](https://github.com/apify/crawlee/blob/master/packages/basic-crawler/src/internals/throttling-request-manager.ts)); Spider, Katana and Colly's `LimitRule` likewise. Every extra crawler multiplies the load on a site.
- **A shared queue but no shared limit:** scrapy-redis, Crawlee's `RequestQueueV2` and Firecrawl's workers share work, not pacing.
- **A shared limit in Redis:** Scrapy Cluster's sliding-window throttle per domain, scoped by IP or spider type ([docs](https://scrapy-cluster.readthedocs.io/en/latest/topics/crawler/controlling.html)), the only off-the-shelf one found. Browserbase enforces account-wide concurrency on its own servers and answers 429 with `retry-after` ([limits](https://docs.browserbase.com/guides/concurrency-rate-limits)).

## Considered Options

- **Politeness per run** (the previous state, and Crawlee's): rejected. A user's own runs gang up on a site.
- **The Daemon as the only authority**: rejected, as ADR 0041 rejected it for Limits. An Attached engine or an engine embedded through the Rust crate never reaches the Daemon and would escape it.
- **Across machines, Goodfetch Cloud only** (as ADR 0041 first placed Limits): rejected. Pain #17 (`selfhost-parity-lockin`) is exactly a capability kept for the hosted service, and hn-103 names distributed rate limiting as a problem people pay for.
- **Daemons electing an authority among themselves**: rejected. It means building an election and failover for state whose loss only resets pacing.
- **Partitioning alone** (ADR 0054): kept for one crawl, where it makes Politeness exact with no store, but it says nothing about independent crawls that reach the same site.
- **On store loss, failing closed at once or failing open**: rejected. Failing closed at once turns a cache restart into a halt of every run; failing open breaks the very promise the store exists to keep. The degraded mode below keeps runs moving through ordinary outages, and stops admission to shared keys only when an outage outlasts `politeness_degraded_max`, because past that point a crashed engine cannot be told from a cut-off one.

## Consequences

**What is shared** (changes ADRs 0004 and 0021).
- **Per domain:** the count of requests in flight, the time before which `Retry-After` or a backoff forbids a request, and the halved concurrency after a 429 with its gradual recovery. The key is the run's domain key (ADR 0004): the registrable domain unless a Recipe overrides it, so runs with the same key share one entry.
- **Per proxy** (adds to ADR 0021). `proxy_concurrency` and `proxy_rate` limit the requests sent through one proxy endpoint and credential, unset by default.
  - A 429 or 502 from the proxy itself, not from the site, is a proxy refusal. It is classified as a proxy error, never as a Block cause, so it never triggers ADR 0021's escalation. The engine learns a ceiling from those refusals, halving as on a site's 429, and raises it again gradually after a quiet period; the ceiling can be set by hand.
  - A 407 from the proxy is a credential error: it fails the request with the Error code `proxy_auth_failed` and does not change the ceiling.
- **Per cloud-browser provider** (changes ADR 0040). The ceiling learned per provider from a vendor's refusals is shared the same way, since a vendor's plan limits the whole account.
- **What stays per run:** the per-domain queues and their order (ADR 0004), learned escalation and tier choice (ADRs 0020 and 0021), and `per_ip_concurrency`, which is keyed by the target's resolved IP.

**How settings combine.**
- Each run keeps its own values (ADR 0021, settings per run). A run admits a request to a domain only while the combined count in flight for that key, across every run sharing it, is below the run's own cap. A stricter run therefore waits while a looser one may still go up to its own cap.
- A `Retry-After`, a backoff or a halving seen by any run applies to every run sharing the key.
- Runs waiting on the same busy domain take turns, as ADR 0040 serves runs before domains.
- `politeness_sharing` is a setting per run: `shared` by default (through the Politeness store when one is configured, else on the machine); `per_daemon`, which shares on the machine only and ignores a store; or `run`, which restores per-run Politeness. Anything but `shared` is marked in the run's result (ADR 0023).

**On one machine: a per-user Politeness file.**
- Every engine of the user (Attached engines, the Daemon, engines embedded through the Rust crate) takes and returns its *permits*, one per request in flight, in one SQLite transaction on a per-user Politeness file. It lives outside the per-major folders like the Usage ledger (ADR 0041), so every major shares it.
- **No polling.** An engine waiting for a permit registers a wake endpoint (a Unix domain socket on Linux and macOS, a named pipe on Windows); the engine that returns a permit wakes the first waiter in turn. A wait for a `Retry-After` or a backoff is a timer on a known deadline.
- **Nothing to keep.** The file holds no Usage and nothing to resume, so it skips `fsync`. A permit records its run and epoch; an engine reclaims the permits of a run whose writer lock is free or held at a later epoch, as ADR 0041 reclaims slices.

**Across machines: the Politeness store.**
- **What it is.** A Valkey or Redis server the user provides, set in user configuration or Operator policy (ADR 0028) with a namespace, so every engine pointing at the same namespace shares Politeness. It holds no run state, so CO-28's exclusion of pluggable live storage (ADR 0037) still stands: nothing is resumed from it, and losing it resets only pacing.
- **Algorithms.** Rates use GCRA, run as a Lua script, so no server module is needed (redis-cell's `CL.THROTTLE` is the reference, [repo](https://github.com/brandur/redis-cell)). Permits are leases with an expiry, renewed while the request lasts. Every key a script touches for one domain or proxy carries the same hash tag, so the script runs on one Redis Cluster slot. A returned permit wakes waiters through pub/sub; a rate wait sleeps until the deadline GCRA returns.
- **Client.** The `redis` crate (BSD-3), async on Tokio with TLS through rustls, Sentinel and Cluster. TLS is required off loopback, and the password is a Secret reference (ADR 0015).
- **Precedence.** With a store configured, the store is the shared state for runs set to `shared`, and the per-user file serves only runs set to `per_daemon`.
- **Goodfetch Cloud** runs the same code against its own Valkey.

**When the store is unreachable.**
- **Heartbeats.** Every engine writes a heartbeat to the store, reporting its longest request timeout, so the live engines sharing a namespace are known. A heartbeat goes stale no later than a permit's lease expires, its record is kept until the store frees its reservation (below), and an engine that exits cleanly deletes its own.
- **Granted shares.** Each heartbeat's acknowledgement carries the engine's degraded share for every key it uses, and the longest request timeout any engine reported for that key. Only the store computes the share, so both sides always hold the same one. The cap shared out is the largest cap any engine's runs reported for that key. The store never lets the shares it has granted add up to more than the cap, except by the minimum of 1 per engine below: a share is the smaller of the cap divided by the live engines and what the cap leaves after every share currently granted to other engines, live or stale, rounded down and at least 1. A share that has to shrink counts at its old size until the engine acknowledges the new one, so a new or growing share waits for the others' next heartbeats.
- **A cut-off engine** keeps the permits it holds until their leases expire, then waits the longest request timeout of its last acknowledgement, so requests other engines admitted before it went stale have ended. It then admits within its last granted share: a run admits a request only while the engine's combined count in flight for that key, across its runs, is below that share scaled by the run's own cap over the largest one. Per-proxy limits are shared out the same way.
- **Engines still connected.** While an engine's heartbeat is stale, the store's script keeps reserved the larger of the share it last granted that engine and the permits it held, so engines that still reach the store admit only the rest of the cap. Requests in flight therefore exceed the cap only by the minimum of 1 per cut-off engine whose share would otherwise have been 0.
- **For a bounded time.** The store cannot tell a crashed engine from a cut-off one, so degraded admission lasts at most `politeness_degraded_max` (10 minutes by default, settable), counted on each side's own clock from the moment the engine sent its last acknowledged heartbeat. The cut-off engine then admits nothing more to shared keys, and the store frees its reservation only once `politeness_degraded_max` plus the longest request timeout the engine reported has passed, so requests it admitted have ended first. From then on, the cut-off engine still admits nothing to shared keys until it reconnects; its runs stay live and resumable. An outage of the whole store longer than that therefore stops admission to shared keys everywhere, which the degraded-period report says.
- `Retry-After`, backoff and halving go on locally.
- The degraded period is reported as an event on each affected run's event stream (ADR 0024), a metric on the ops listener (ADR 0050) and a note in each affected run's result. Reconnection is automatic, with backoff.

**Formats** (changes ADR 0030). The per-user Politeness file and the Politeness store's key and script layout join ADR 0030's contract formats. Both follow the Usage ledger's exception: their major stays frozen across product majors, and newer versions only add, so every supported major shares them.

**Several Daemons in a Daemon group** (ADR 0055) share Politeness through a store. A group of more than one Group member, formed by the Helm chart or by `goodfetch group join`, is refused unless a store is set or its Operator policy sets `politeness_sharing: per_daemon`, which every run's result and the Console then show.
