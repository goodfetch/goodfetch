# V1 scope matrix

This matrix is the companion of [ADR 0037](0037-v1-scope-parity-and-pain-coverage.md). It gives one status, its source and its `/to-spec` area to every row of the [parity list of 2026-10-10](https://github.com/Jbmanllr/automan/blob/research/parity-list/rnd/research/2026-10-10-parity-list.md) (Crawlee JS 3.18.2 / 4.0.0-rc.1, Crawlee Python 1.10.4, Spider 2.53.9, Stagehand 4.2.0), to five Stagehand v3 capabilities reached through Crawlee, and to the 24 pains of `rnd/research/market/pains-ranking.md`. A ticket that resolves an `open` row changes that row in its pull request, and the map is done when no row reads `open` and the map has no open ticket. When a pull request changes an `open` row, it also updates the "Waits on" column of ADR 0037.

## Status counts

| Status | Parity rows | Stagehand v3 rows | Pains |
|---|---:|---:|---:|
| `decided` | 103 | 4 | 24 |
| `spec` | 38 | 1 | 0 |
| `excluded` | 1 | 0 | 0 |
| `open → #N` | 0 | 0 | 0 |
| Total | 142 | 5 | 24 |

## Parity rows

### RT Runtime & state

| ID | Capability | Status | Source | Spec area | Note |
|---|---|---|---|---|---|
| RT-01 | Concurrency autoscaled from CPU, memory and event-loop load | `decided` | ADR 0040, ADR 0004, ADR 0021 | 3 Engine runtime | One governor per engine scales on memory, CPU and Tokio saturation between a floor and the user's ceiling, shared fairly between runs; browser pages, HTTP fetches and model calls admitted separately. |
| RT-02 | Concurrency bounds and a requests-per-minute cap | `spec` | ADR 0004, ADR 0021, ADR 0028 | 3 Engine runtime | Global and per-domain caps decided. A global rate bound on the admission gate is spec. |
| RT-03 | Resource budget from container limits (cgroups, Kubernetes) | `decided` | ADR 0040 | 3 Engine runtime | The tightest of the process's own cgroup (v1 or v2), a Windows Job object, `memory_allowance` or half of physical memory, read again at every sample, plus the host's own memory pressure; engine plus children measured, PSS on Linux. |
| RT-04 | Retry count, handler before retry, failed-request handler | `spec` | ADR 0003, ADR 0028, ADR 0033 | 7 Plugin protocol | Retries are settings. A failed-request handler needs a new stage in ADR 0033's stage table. |
| RT-05 | Error types that steer retries or abort the run | `spec` | ADR 0030, ADR 0006, ADR 0021, ADR 0038, ADR 0079 | 11 Client surfaces | Typed Error codes are under semver, each with a retryable flag and one generated SDK class (ADR 0038), under seven groups by remedy; a handler's own exception is `handler_failed` (ADR 0079). How the classes map onto recovery, retirement and stop is spec. |
| RT-06 | Timeouts: navigation, handler, whole run, first byte | `decided` | ADR 0003, ADR 0020, ADR 0028, ADR 0080, [#69](https://github.com/Jbmanllr/automan/issues/69) item 7 | 3 Engine runtime | Every phase has a finite default timeout held as a setting, nested so no inner call outlives its outer one; a Run's own timeout has no built-in default and stops it with a Checkpoint (ADR 0080). The first-byte timeout's value is spec. |
| RT-07 | Routing to handlers by label, typed labels, middleware | `spec` | ADR 0033, [#71](https://github.com/Jbmanllr/automan/issues/71) | 11 Client surfaces | Label-scoped Hooks replace router middleware. The router and `userData` schema validation are SDK shape. |
| RT-08 | Pre- and post-navigation hooks | `decided` | ADR 0033 | 7 Plugin protocol | Hook stages before and after navigation. |
| RT-09 | User state persisted across restarts | `decided` | [#71](https://github.com/Jbmanllr/automan/issues/71), ADR 0002 | 3 Engine runtime | A Key-value store in the run file checkpoints with the run. The `useState` shape is spec. |
| RT-10 | Resume an interrupted crawl from persisted state | `decided` | ADR 0002, ADR 0003 | 3 Engine runtime | Frontier, Sessions and journal live in the per-run SQLite file. |
| RT-11 | Stop, pause and resume a running crawl | `decided` | ADR 0001, ADR 0002, ADR 0029 | 3 Engine runtime | Pause is checkpoint, stop and resume, not an in-memory pause. |
| RT-12 | Keep-alive run that waits for new work | `spec` | [#71](https://github.com/Jbmanllr/automan/issues/71), ADR 0005, ADR 0029 | 3 Engine runtime | A run flag plus CO-03's add-requests operation. How a keep-alive run finishes, and its Completeness, are spec under ADR 0029. |
| RT-13 | In-process scheduled (cron) runs | `decided` | ADR 0036 | 10 Scheduling | Daemon Schedules: cron, interval, Watch, after a run. |
| RT-14 | Lifecycle events: persist, system info, migrating, aborting | `decided` | ADR 0001, ADR 0024, ADR 0033 | 3 Engine runtime | Lifecycle Hooks, signals and the event stream. |
| RT-15 | Run statistics, status log, custom status message | `spec` | ADR 0024, ADR 0029 | 3 Engine runtime | Usage and progress events decided. A user status message is one more progress-event field. |
| RT-16 | Error grouping with HTML and screenshot snapshots | `spec` | ADR 0031, ADR 0029 | 12 Console | Traces are kept for failed runs. Grouping errors by code and stack is a report feature. |
| RT-17 | Structured logging: levels, JSON, custom sink | `spec` | [#70](https://github.com/Jbmanllr/automan/issues/70) item 9, ADR 0078 | 3 Engine runtime | The `tracing` substrate is decided. Levels and a JSON format are subscriber settings; SDKs relay engine logs to the host's logging (ADR 0078). |
| RT-18 | OpenTelemetry traces | `decided` | [#70](https://github.com/Jbmanllr/automan/issues/70) item 9 | 3 Engine runtime | `tracing` everywhere, opt-in OTLP export, GenAI conventions for model calls. Recorded only in the ticket. |
| RT-19 | Several crawlers in one process | `decided` | ADR 0002, ADR 0024, ADR 0078 | 3 Engine runtime | A Daemon hosts several runs, and an SDK opens many independent connections in one process (ADR 0078). |
| RT-20 | Hedged requests, coalescing duplicate in-flight requests | `decided` | ADR 0040 | 3 Engine runtime | Hedging opt-in for GET and HEAD from work items that declare no Effect, through the Session's own sticky exit, counted in politeness and Usage and capped at 5 %; coalescing on by default within one Session, never across Sessions or runs. |
| RT-21 | Pluggable storage, HTTP client, events and browser | `decided` | ADR 0033, ADR 0024, ADR 0002 | 7 Plugin protocol | Fetch, browser, Hook and Sink Plugin kinds decided; events reach code through Hooks and the protocol (ADR 0024). Storage excluded: a replaceable store would break portable, resumable runs (ADR 0002). |
| RT-22 | Typed config object and environment variables | `decided` | ADR 0028, ADR 0034 | 3 Engine runtime | Five-layer settings, generated schema, `GOODFETCH_` variables. |
| RT-23 | Run a script beside the browser to save round trips | `decided` | ADR 0011, ADR 0022, ADR 0051 | 9 AI and agent | Code mode in the Script sandbox; the Browser host keeps one round trip per action. |

### SDK SDK binding & distribution

| ID | Capability | Status | Source | Spec area | Note |
|---|---|---|---|---|---|
| SDK-01 | Primary language and package | `decided` | ADR 0024, ADR 0034 | 11 Client surfaces | `goodfetch` binary, Rust crate, npm and PyPI SDKs. |
| SDK-02 | Same API in several languages | `decided` | ADR 0011, ADR 0024, ADR 0030 | 11 Client surfaces | Every SDK is generated from one Operation catalog, in lockstep versions. |
| SDK-03 | Modular install | `decided` | ADR 0024, ADR 0033, ADR 0057 | 1 Repository, CI and release | One binary with every capability under a Size budget; browsers, sink processes and Plugins fetched on demand by digest. |
| SDK-04 | Project scaffolding CLI and templates | `decided` | ADR 0030, ADR 0049 | 11 Client surfaces | `goodfetch init` scaffolds TypeScript, Python and Rust from five starters (crawl, extract, browser automation, AI agent, Recipe), each with a Dockerfile, Compose file, CI workflow, AGENTS.md and skills, embedded in the binary; `--deploy <target>` adds a target's files. |
| SDK-05 | No-code CLI that crawls or drives a browser | `decided` | ADR 0011, ADR 0034, ADR 0036 | 11 Client surfaces | CLI generated from the catalog with `--json`; code-free catalog runs. |
| SDK-06 | Typed public API and schemas | `decided` | ADR 0011, ADR 0024, ADR 0028, ADR 0038, ADR 0081 | 11 Client surfaces | Typed catalog defined in Rust types, generated SDK types, sync and async Python: the async client on AnyIO, the sync one on a private thread per connection that works inside a running loop; Rust async on any executor plus `goodfetch::blocking` (ADR 0081). |
| SDK-07 | Docs for AI coding assistants | `decided` | ADR 0030 | 11 Client surfaces | `llms.txt`, `llms-full.txt`, embedded docs, skills, AGENTS.md. |
| SDK-08 | Browser provisioning | `decided` | ADR 0024 | 3 Engine runtime | Pinned builds in a digest-checked cache; `goodfetch browsers install`; offline path. |

### SA Site access

| ID | Capability | Status | Source | Spec area | Note |
|---|---|---|---|---|---|
| SA-01 | Plain HTTP fetching without a browser | `decided` | ADR 0020 | 5 Site access | HTTP tier on reqwest and wreq; the self-hosted equivalent of Stagehand's cloud fetch. |
| SA-02 | Replaceable HTTP transport | `decided` | ADR 0033 | 7 Plugin protocol | Fetch backend Plugin kind. |
| SA-03 | HTTP/2 | `decided` | ADR 0020, ADR 0035 | 5 Site access | wreq HTTP/2 settings matched to the build. |
| SA-04 | Handler HTTP request sharing Session cookies and proxy | `spec` | ADR 0019, ADR 0020 | 5 Site access | Maps to Playwright `page.request`; the browser-fetch tier keeps the identity. An Effect when the user declares it or an AI decides it (ADR 0010). |
| SA-05 | Chromium, headless or headful | `decided` | ADR 0019, ADR 0024, ADR 0035 | 5 Site access | Direct CDP driver with the guard. |
| SA-06 | Firefox, WebKit, Edge | `decided` | ADR 0019, ADR 0035, ADR 0030 | 5 Site access | Firefox and WebKit through Playwright's driver with the guard; Edge best effort. |
| SA-07 | Puppeteer backend | `decided` | ADR 0019, ADR 0017, ADR 0003 | 5 Site access | Puppeteer code attaches over raw CDP, unguarded. Its attach is bound to the work item's lease and cut when the lease ends (ADR 0003). |
| SA-08 | Attach to a running or remote browser; never close it | `decided` | ADR 0022, ADR 0033, ADR 0056, ADR 0078 | 5 Site access | Third-party CDP browsers and Browser hosts, the person's running browser through Chrome's consented remote debugging, and browsers the person's own code launched (`run.borrow()`, `cdp_url`); a Borrowed browser is never closed. |
| SA-09 | Browser pool: page caps, retirement, launch hooks | `decided` | ADR 0040, ADR 0006, ADR 0035, ADR 0033 | 3 Engine runtime | Sessions of the same run share a process as contexts unless their Fingerprint method is per process; browsers recycled after 100 pages, on memory growth or when idle, the Session carried over intact; launch hooks are spec (browser backend kind). |
| SA-10 | Isolated contexts per page or Session | `decided` | ADR 0006, ADR 0021 | 5 Site access | Each Session has its own cookies, identity and proxy. Context or browser isolation is spec. |
| SA-11 | Persistent browser profile (user data directory) | `decided` | ADR 0056, ADR 0035, ADR 0015 | 5 Site access | Saved identities persist cookies and storage. A Profile directory is launched in place by a browser of the family that wrote it (Chromium family, Firefox, WebKit), under its own lock and version checks, outside the reset gate. |
| SA-12 | Full native driver: frames, `route`, waits, assertions, `getBy*` | `spec` | ADR 0019, ADR 0003 | 5 Site access | `route`, assertions and `waitForResponse` follow ADR 0019's mapping. A raw CDP attach is cut when its lease ends (ADR 0003). |
| SA-13 | Click, fill, type, select, hover, scroll, drag, key, upload | `spec` | ADR 0019 | 5 Site access | Click, fill and human-like input decided. The rest follows ADR 0019's mapping; uploads land in the run directory. |
| SA-14 | Waits: selector, load state, network idle, delay | `decided` | ADR 0019, ADR 0080 | 5 Site access | The API waits through the guard until a deadline. App-state waits keep Playwright's names, add `waitForRequestFinished` and `waitForCount`, define `networkidle` and arm when called; a fixed delay is a typed tombstone that fails with `removed` in TypeScript and Python and never compiles in Rust (ADR 0080). |
| SA-15 | Targeting inside iframes and shadow DOM | `spec` | ADR 0019, ADR 0009 | 5 Site access | OOPIF tracking decided; the recorder spans frames and shadow DOM. Page API shadow-root targeting is spec. |
| SA-16 | Evaluate JS; init scripts before page scripts | `spec` | ADR 0019 | 5 Site access | `evaluate` decided. A user init script departs from `standard` mode's no-injection identity (ADR 0021); the spec marks it. |
| SA-17 | Request interception and resource or ad blocking | `decided` | ADR 0047, ADR 0014, ADR 0033, ADR 0019, ADR 0052 | 5 Site access | Ads and trackers blocked by default through `adblock-rust` in the policy interception, with a never-block list for challenge, anti-bot and payment scripts; `block_resources` by type; Hook resource blocking. User `route` is spec. |
| SA-18 | Network and console events | `decided` | [#71](https://github.com/Jbmanllr/automan/issues/71), ADR 0019, ADR 0081 | 5 Site access | Network and console capture decided. `page.on(event, listener)` keeps Playwright's event names and returns a Subscription, with `waitForEvent` for one event and a stream in Rust; URL patterns match in the engine (ADR 0081). |
| SA-19 | Main-document response: status, headers, body | `spec` | ADR 0019, ADR 0024 | 5 Site access | Body handles decided. Status, headers and final URL follow the Playwright `Response` mapping. |
| SA-20 | Custom headers, user agent, referer | `decided` | ADR 0033, ADR 0021, ADR 0035 | 5 Site access | Before-request Hooks change headers except `Host`, `Cookie` and `Authorization`. |
| SA-21 | Read and write cookies | `spec` | ADR 0019, ADR 0033, ADR 0056 | 5 Site access | Hooks set cookies. Reading follows `context.cookies`; ADR 0014 and ADR 0015 keep cookies from models and result exports; a person may export a Saved identity as `storageState` (ADR 0056). |
| SA-22 | Proxy: list, function, auth, SOCKS, per-request choice | `spec` | ADR 0021, ADR 0033, ADR 0015 | 5 Site access | Pools by kind decided; SOCKS5 auth relayed locally. Per-request choice excluded: it changes the IP under the same cookies (ADR 0021). |
| SA-23 | Device emulation: viewport, locale, timezone, geolocation, touch, scale | `spec` | ADR 0019, ADR 0035, ADR 0021 | 5 Site access | Locale, time zone and geolocation decided. User viewport, touch and scale stay consistent with the `standard` identity. |
| SA-24 | Redirect control and limits | `decided` | ADR 0020, ADR 0017, ADR 0028 | 5 Site access | The engine sets redirects; each is re-checked by the egress policy. |
| SA-25 | Bind to a local IP or network interface | `decided` | ADR 0044 | 4 Security and egress | A Session egress option on every OS: addresses, interface names, Linux IPv6 prefixes; DNS and WebRTC follow it; browsers through the egress relay; rotation after a block leaves the blocked /64 or /24. |
| SA-26 | Auto-dismiss JS dialogs | `spec` | ADR 0019, ADR 0014 | 5 Site access | The default dismisses dialogs, since accepting a `confirm()` may confirm an Effect (ADR 0014). |
| SA-27 | HTTP auth challenge in the browser | `spec` | ADR 0019, ADR 0015 | 5 Site access | Answered from a Secret reference bound to domains. |
| SA-28 | Close cookie-consent modals | `decided` | ADR 0048, ADR 0047, ADR 0052 | 5 Site access | Consent prompts refused by default through autoconsent, hidden when they offer no refusal and leave the content readable; a consent wall is the Block cause `consent_required`, accepted only on `consent_domains`, and read past without consenting only on a person-approved `wall_domains` (ADR 0052). |
| SA-29 | Several tabs with an active page | `spec` | ADR 0019, ADR 0031 | 5 Site access | The Live view follows popups. The tab API follows ADR 0019's mapping. |
| SA-30 | Clipboard read, write, copy, cut, paste | `spec` | ADR 0019 | 5 Site access | The page's clipboard, not the host's; mapped or excluded with a reason. |

### ST Stealth & challenge solving

| ID | Capability | Status | Source | Spec area | Note |
|---|---|---|---|---|---|
| ST-01 | Browser fingerprint generation and injection | `decided` | ADR 0035, ADR 0021 | 8 Stealth and Challenge solving | Fingerprint methods and generated identities. |
| ST-02 | Headless-Chrome evasion patches | `decided` | ADR 0021, ADR 0019, ADR 0035 | 8 Stealth and Challenge solving | Self-hosted equivalent of Verified browsers: Goodfetch-built Chromium, no `Runtime`, headful in `stealth`. |
| ST-03 | Browser-like headers for HTTP | `decided` | ADR 0035, ADR 0020 | 8 Stealth and Challenge solving | Header order and client hints captured from the build. |
| ST-04 | TLS and HTTP fingerprint impersonation | `decided` | ADR 0020, ADR 0035 | 8 Stealth and Challenge solving | wreq TLS and HTTP/2 matched to the Identity profile. |
| ST-05 | Hardened browser builds (Camoufox, CloakBrowser) | `decided` | ADR 0035, ADR 0033 | 8 Stealth and Challenge solving | Goodfetch build and Camoufox; CloakBrowser through a browser backend Plugin. |
| ST-06 | Cloudflare challenge handling | `decided` | ADR 0035, ADR 0021, ADR 0018 | 8 Stealth and Challenge solving | Self-hosted escalation ladder. Goodfetch's own solving logic is a licensed Private component (ADR 0018). |
| ST-07 | CAPTCHA solving | `decided` | ADR 0035, ADR 0033, ADR 0018 | 8 Stealth and Challenge solving | Self-hosted through the Challenge solver: the user's solver account or Plugin, or the licensed Private component. |
| ST-08 | Block detection with retry or rotation | `decided` | ADR 0035, ADR 0021 | 8 Stealth and Challenge solving | Typed block causes; the blocked Session is retired and reopened at the next tier. |
| ST-09 | Session-bound proxy rotation with tiered escalation | `decided` | ADR 0021, ADR 0006 | 8 Stealth and Challenge solving | Self-hosted with the user's own proxy providers. |
| ST-10 | Hand-off to a managed unblocker | `decided` | ADR 0033 | 8 Stealth and Challenge solving | A fetch backend Plugin reaches the user's unblocker account, with spend accounting. |

### CO Collection: frontier, sessions, extraction, datasets and sinks

| ID | Capability | Status | Source | Spec area | Note |
|---|---|---|---|---|---|
| CO-01 | Persistent request queue, dedup, forefront insertion | `decided` | ADR 0002, ADR 0005 | 6 Collection | Frontier in run state; learned and fixed URL rules. |
| CO-02 | Seed lists, list+queue tandem, iterable inputs | `spec` | [#71](https://github.com/Jbmanllr/automan/issues/71), ADR 0005, ADR 0024 | 6 Collection | Seeds decided. Streaming seed input uses ADR 0024's credit-based streams. |
| CO-03 | Add URLs while the crawl runs | `spec` | [#71](https://github.com/Jbmanllr/automan/issues/71), ADR 0005 | 6 Collection | Forefront insertion and priority decided. The add-requests operation is SDK shape. |
| CO-04 | Frontier ordered by priority | `decided` | ADR 0005, ADR 0004 | 6 Collection | Priority scores within a domain. |
| CO-05 | Link discovery: scope, include/exclude, selector, transform, limit | `spec` | [#71](https://github.com/Jbmanllr/automan/issues/71), ADR 0005, ADR 0033 | 6 Collection | URL rules and Hook rewrite decided. Scope strategies, patterns and link selectors are spec. |
| CO-06 | Enqueue by clicking JS-driven elements | `spec` | [#71](https://github.com/Jbmanllr/automan/issues/71) | 6 Collection | Clicks to discover links are not Effects; the spec decides how a discovery click is kept from causing one (ADR 0010, ADR 0014). |
| CO-07 | Crawl-depth limit | `spec` | [#71](https://github.com/Jbmanllr/automan/issues/71), ADR 0028 | 6 Collection | A depth cutoff reports the run `incomplete` (ADR 0005). |
| CO-08 | Page budgets, global and per path | `spec` | [#71](https://github.com/Jbmanllr/automan/issues/71), [#69](https://github.com/Jbmanllr/automan/issues/69) item 7 | 6 Collection | Global request Limits decided. Per-path budgets are spec. |
| CO-09 | Per-domain politeness incl. latency throttle and token bucket | `spec` | [#71](https://github.com/Jbmanllr/automan/issues/71), ADR 0021, ADR 0053 | 6 Collection | Delay, backoff and crawl-delay decided. Latency throttle and token bucket are opt-in; defaults stay. |
| CO-10 | Sitemap discovery and loading | `spec` | [#71](https://github.com/Jbmanllr/automan/issues/71), ADR 0016 | 6 Collection | robots.txt is already read on every run. Scope filtering is SC-02. |
| CO-11 | Callback for skipped or blocked links | `spec` | [#71](https://github.com/Jbmanllr/automan/issues/71), ADR 0005, ADR 0033 | 6 Collection | Learned rules are reported and the `skip` stage exists. This adds an event. |
| CO-12 | Session pool | `decided` | ADR 0006, ADR 0035, ADR 0002 | 6 Collection | Scoring, rotation and retirement, persisted in run state. |
| CO-13 | Authenticated crawling: log in once, reuse | `decided` | ADR 0006, ADR 0009, ADR 0015, ADR 0035, ADR 0056 | 5 Site access | Recovery chain, scripted login Plans, MFA, saved identities, and logins the user already has (ADR 0056). |
| CO-14 | Built-in HTML parsers | `decided` | ADR 0007 | 6 Collection | html5ever and scraper for fields; lol-html for streaming. |
| CO-15 | Per-page choice between HTTP and browser | `decided` | ADR 0020 | 6 Collection | Four fetch tiers chosen per route by sample comparison. |
| CO-16 | Parallel backends raced with a quality score | `decided` | ADR 0040, ADR 0020 | 6 Collection | Opt-in `tier_race` within one Session: browser-fetch first, a full navigation after a delay, the first result passing the Page check and the route's comparison wins; sample comparison stays the default. |
| CO-17 | Infinite-scroll helper | `spec` | [#71](https://github.com/Jbmanllr/automan/issues/71), ADR 0020, ADR 0033 | 6 Collection | Discovered API first; the scroll op and human-like scrolling are decided. |
| CO-18 | HTML to Markdown or text | `decided` | ADR 0007 | 6 Collection | dom-smoothie then htmd; the self-hosted equivalent of Stagehand's cloud fetch. |
| CO-19 | Contact, social and metadata extraction | `decided` | [#71](https://github.com/Jbmanllr/automan/issues/71) | 6 Collection | Formats: Open Graph, JSON-LD, microdata, contact and social. Recorded only in the ticket. |
| CO-20 | Screenshots: full page, clip, mask | `decided` | [#71](https://github.com/Jbmanllr/automan/issues/71) | 6 Collection | Formats include screenshots; full page, clip and mask are parameters. |
| CO-21 | Page to PDF | `decided` | [#71](https://github.com/Jbmanllr/automan/issues/71) | 6 Collection | Formats include PDF and MHTML. |
| CO-22 | File download and streaming | `decided` | [#71](https://github.com/Jbmanllr/automan/issues/71) | 6 Collection | Formats include downloaded files. |
| CO-23 | Collect every page resource | `decided` | [#71](https://github.com/Jbmanllr/automan/issues/71) | 6 Collection | A browser-tier WARC records every resource response. A standalone resource list would be spec. |
| CO-24 | Character-encoding handling | `spec` | [#71](https://github.com/Jbmanllr/automan/issues/71), ADR 0007, ADR 0025 | 6 Collection | Detection and forced encoding are spec; odd encodings are Proving ground pages. |
| CO-25 | Push records to a dataset and read them back | `decided` | ADR 0008, ADR 0026, ADR 0031 | 6 Collection | Dataset of Records, numbered results stream, paging and SQL tab. |
| CO-26 | Key-value store | `decided` | [#71](https://github.com/Jbmanllr/automan/issues/71) | 6 Collection | A Key-value store in the run file. Its operations are spec. |
| CO-27 | Export to CSV or JSON with all keys | `decided` | ADR 0008, ADR 0026 | 6 Collection | Files mode carries the union of all fields. |
| CO-28 | Storage backends: memory, filesystem, SQL, Redis, custom | `excluded` | ADR 0002, ADR 0033, ADR 0053 | 6 Collection | "Pluggable live storage … rejected" (ADR 0002): it brings back multiple writers. SQL and Redis are reached as Sinks (ADR 0026); a Redis-protocol server may hold Politeness, never run state (ADR 0053). |
| CO-29 | Stream of pages as they arrive | `decided` | ADR 0008, ADR 0024, ADR 0081 | 6 Collection | Ordered, lossless results stream with credit-based delivery, read with `for await`, `async for` or a Rust `EventStream` and closed by leaving the loop (ADR 0081). |
| CO-30 | WARC output | `decided` | [#71](https://github.com/Jbmanllr/automan/issues/71) | 6 Collection | WARC from both HTTP and browser tiers. |
| CO-31 | HTTP response cache and conditional re-crawl (ETag) | `decided` | ADR 0046, ADR 0036 | 6 Collection | Per-user Response cache, off by default: `revalidate` (RFC 9111) and `replay` on every tier; partitioned by Session, country, language and device; 10 GB and 30 days by default; never `replay` for Schedules. |

### AI AI & agents

| ID | Capability | Status | Source | Spec area | Note |
|---|---|---|---|---|---|
| AI-01 | Natural-language action on a page | `decided` | ADR 0009, ADR 0011, ADR 0013 | 9 AI and agent | `act` becomes a Plan step. |
| AI-02 | Observe, then replay without inference | `decided` | ADR 0009 | 9 AI and agent | `observe` and `act` compile into Plans replayed with no model call. |
| AI-03 | LLM extraction validated by a schema | `decided` | ADR 0013, ADR 0012 | 9 AI and agent | Grounding; per-page model extraction is an explicit mode. |
| AI-04 | Vision input | `decided` | ADR 0013 | 9 AI and agent | Opt-in, on the last rung of the ladder. |
| AI-05 | Multi-step autonomous agent loop | `decided` | ADR 0013 | 9 AI and agent | A thin, journaled built-in agent over the catalog. |
| AI-06 | Browser tools for external agent frameworks | `decided` | ADR 0011 | 11 Client surfaces | Generated adapters for Vercel AI SDK, LangChain/LangGraph and Mastra; MCP for the rest. |
| AI-07 | MCP server | `decided` | ADR 0011, ADR 0017 | 11 Client surfaces | stdio and Streamable HTTP. |
| AI-08 | Accessibility or DOM snapshot with element refs | `decided` | ADR 0011 | 9 AI and agent | Stable element references and diffs. |
| AI-09 | WebMCP: list, invoke, cancel, add/remove events | `spec` | ADR 0011, [#72](https://github.com/Jbmanllr/automan/issues/72) | 9 AI and agent | Site tools join the catalog as Effects. Cancelling follows ADR 0010's uncertain-Effect rules. |
| AI-10 | Model providers, own model, per-call override | `decided` | ADR 0012, ADR 0033 | 9 AI and agent | Three formats, gateways, local servers and the Model provider kind. |
| AI-11 | Managed model gateway, no provider keys | `decided` | ADR 0012 | 9 AI and agent | Local models need no key; the user's own gateway by URL. The managed, keyless gateway is a Goodfetch Cloud operation (ADR 0018). |
| AI-12 | Cache of AI results | `decided` | ADR 0009, ADR 0010, ADR 0026 | 9 AI and agent | Self-hosted equivalent: committed Plans replayed without a model; journaled answers reused on resume. |
| AI-13 | Self-healing after a failed action | `decided` | ADR 0009 | 9 AI and agent | One bounded Repair after Drift, verified before use. |
| AI-14 | Token and cost accounting | `decided` | ADR 0012, ADR 0033 | 9 AI and agent | Token, call and currency Limits; Usage. |
| AI-15 | Web search API for agents | `decided` | ADR 0011, ADR 0033 | 9 AI and agent | Interchangeable search providers, including self-hosted SearXNG. |
| AI-16 | Configure a crawl from a prompt | `decided` | ADR 0011 | 9 AI and agent | Compiles to a configuration reviewed before it runs. |
| AI-17 | Research synthesis: search, extract, summarize | `decided` | [#72](https://github.com/Jbmanllr/automan/issues/72) | 9 AI and agent | Q30: research synthesis as a cited built-in agent task. Recorded only in the ticket. |
| AI-18 | Code tools for the agent | `decided` | ADR 0011, ADR 0051 | 9 AI and agent | Code mode in the Script sandbox (V8 without JIT, in a confined worker per run) with only catalog calls. |
| AI-19 | Custom system prompt | `decided` | [#72](https://github.com/Jbmanllr/automan/issues/72) | 9 AI and agent | Q30: a custom system prompt on every AI operation. Recorded only in the ticket. |

### SC Security & compliance

| ID | Capability | Status | Source | Spec area | Note |
|---|---|---|---|---|---|
| SC-01 | Opt-in robots.txt with a custom user agent | `decided` | ADR 0016, ADR 0021 | 4 Security and egress | `respect_site_preferences`, off by default; product user agent in `identified`. |
| SC-02 | Same-hostname scope by default; host re-checked after redirects | `spec` | [#71](https://github.com/Jbmanllr/automan/issues/71), ADR 0017, ADR 0014 | 4 Security and egress | Re-checks decided. The default is same-hostname; filtering sitemap and robots URLs is a conformance case. |
| SC-03 | Only `http` and `https` schemes | `spec` | ADR 0014, ADR 0017 | 4 Security and egress | A fixed Frontier rule under the fail-closed policy. |
| SC-04 | Domain allow/deny on browser traffic | `decided` | ADR 0014, ADR 0017 | 4 Security and egress | Every request type, fail closed. |
| SC-05 | Guards against resource exhaustion | `spec` | ADR 0005, ADR 0020, ADR 0028, ADR 0025 | 4 Security and egress | Trap rules, Limits and redirects decided. Max page size and decompression-bomb caps are spec values. |
| SC-06 | TLS verification on by default, explicit opt-out | `decided` | ADR 0042, ADR 0033 | 4 Security and egress | Verified everywhere; per-domain trusted CAs, client certificates and accepted invalid certificates, pinned on first use and audited; browsers through the engine's TLS relay; never for operator endpoints. |
| SC-07 | Browser sandbox control | `decided` | ADR 0017, ADR 0016 | 4 Security and egress | On by default, switchable per run, recorded. |
| SC-08 | Secrets kept out of model prompts | `decided` | ADR 0014, ADR 0015 | 4 Security and egress | `%var%` placeholders typed by the engine. |
| SC-09 | Isolation of model-written code | `decided` | ADR 0011, ADR 0051 | 4 Security and egress | The Script sandbox: a worker confined as a Plugin at network level `none`, with no files or Secrets. |
| SC-10 | Telemetry off unless configured | `decided` | ADR 0043 | 4 Security and egress | Locked: nothing leaves unless configured and no telemetry at all; only digest-pinned downloads, which `offline` turns off; browsers' background traffic off; checked in CI on three OSes. |

### CN Console

| ID | Capability | Status | Source | Spec area | Note |
|---|---|---|---|---|---|
| CN-01 | Watch a running browser live | `decided` | ADR 0031, ADR 0006 | 12 Console | Live view with three tiers and take control. |
| CN-02 | Session recording and replay | `decided` | ADR 0031 | 12 Console | Self-hosted: Run traces replayed on the Timeline. |
| CN-03 | Run status and progress in a UI | `decided` | ADR 0031, ADR 0029, ADR 0024 | 12 Console | Console over the Run index, on three axes. |
| CN-04 | Hosted dashboard of runs, filterable by metadata | `decided` | ADR 0031, ADR 0036 | 12 Console | Self-hosted: the Console over the Run index, with run labels. |

### CL Cloud readiness

| ID | Capability | Status | Source | Spec area | Note |
|---|---|---|---|---|---|
| CL-01 | Docker images and Dockerfiles in templates | `decided` | ADR 0024, ADR 0017, ADR 0049 | 13 Self-hosted deployment and operations | Five non-root, multi-arch image tags (default, `-slim`, `-full`, `-node`, `-python`) on Debian 13 slim with SBOM and attestations; every template has a Dockerfile (SDK-04). |
| CL-02 | Deployment guides (AWS Lambda, Cloud Run) | `decided` | ADR 0031, ADR 0049 | 13 Self-hosted deployment and operations | Supported means a tested guide plus a CI deploy: Docker and Compose, Kubernetes (Helm), systemd, Cloud Run, Lambda as a container image, and edge runtimes as Clients; ECS and Fargate, Azure Container Apps, Fly.io, Railway and Render are documented. |
| CL-03 | Crawler served as an HTTP service | `decided` | ADR 0027, ADR 0011, ADR 0017 | 11 Client surfaces | Daemon HTTP JSON-RPC listener; MCP Streamable HTTP. |
| CL-04 | Several processes or machines sharing one queue | `decided` | ADR 0024, ADR 0003, ADR 0002, ADR 0054 | 3 Engine runtime | `workers=N` processes lease from one run. Machines sharing a live frontier excluded: multiple writers (ADR 0002). Past one engine, a Partitioned crawl splits a crawl across Group members, each Partition owning its domains and receiving their forwarded links (ADR 0054). |
| CL-05 | Remote browser sessions with regions and keep-alive | `decided` | ADR 0022, [#69](https://github.com/Jbmanllr/automan/issues/69) item 8 | 14 Cloud-readiness contracts | Self-hostable Browser host; Sessions can be kept alive. Regions are a Cloud matter. |
| CL-06 | Move from local to the vendor cloud by configuration | `decided` | ADR 0001, ADR 0031, ADR 0036 | 14 Cloud-readiness contracts | The cloud is an ordinary protocol client; the Console takes its endpoint as configuration. |
| CL-07 | Persistent browser contexts stored in the cloud | `decided` | ADR 0035, ADR 0015 | 14 Cloud-readiness contracts | Self-hosted: saved identities and session export. |

## Stagehand v3 capabilities reached through Crawlee

| ID | Capability | Status | Source | Spec area | Note |
|---|---|---|---|---|---|
| V3-01 | `act`, `extract`, `observe` on the crawler's page mid-crawl | `spec` | ADR 0011, ADR 0013, [#72](https://github.com/Jbmanllr/automan/issues/72) | 9 AI and agent | AI operations run on any Session page; per-page model extraction is an explicit mode. The handler-page method shape is spec. |
| V3-02 | Local cache of AI results (v3 `cacheDir`) | `decided` | ADR 0009, ADR 0010 | 9 AI and agent | Plans are local, versioned files replayed without a model; a service-held cache was rejected (ADR 0009). |
| V3-03 | Custom model client (v3 `llmClient`) | `decided` | ADR 0012, ADR 0033 | 9 AI and agent | The SDK model callback, the SDK-handler form of the Model provider kind, still journaled, counted and capped. |
| V3-04 | Built-in agent loop (v3 `agent().execute()`) | `decided` | ADR 0013 | 9 AI and agent | A thin, journaled built-in agent; dropping it, as Stagehand v4 did, was rejected. |
| V3-05 | Local or remote browser chosen by configuration (v3 `env`) | `decided` | ADR 0022, ADR 0001, ADR 0028 | 5 Site access | Local browser, Browser host, or a third-party CDP browser such as Browserbase; the cloud is an ordinary protocol client. |

No v3 API compatibility layer is offered (ADR 0037). A Crawlee+Stagehand migration guide is part of the docs (spec area 11 Client surfaces).

## Pains

| Rank | Pain | Status | Source | Spec areas | Note |
|---:|---|---|---|---|---|
| 1 | `access-antibot-ip` | `decided` | ADR 0019, ADR 0020, ADR 0021, ADR 0035 | 8 Stealth and Challenge solving; 5 Site access | Goodfetch cannot promise access to any site: whether a site admits automated traffic is the site owner's decision and an ongoing arms race. |
| 2 | `agent-token-latency-cost` | `decided` | ADR 0009, ADR 0011, ADR 0013, ADR 0025 | 9 AI and agent | Plans replay with no model call; MCP profiles trim tools; observations send diffs; cost per verified result is benchmarked. |
| 3 | `integ-mcp-agent-tooling` | `decided` | ADR 0011, ADR 0017, ADR 0030 | 11 Client surfaces; 9 AI and agent | One catalog generates MCP, CLI, skills and adapters; OAuth 2.1 for remote MCP; docs served as MCP resources. |
| 4 | `access-auth-session` | `decided` | ADR 0006, ADR 0015, ADR 0035, ADR 0056 | 5 Site access | Page check, recovery ladder, MFA, saved identities. A login the user already has comes in through `session.source`: a state file, a cookie header, an installed browser, a Profile directory or the user's running browser, read by the browser that owns it, with re-import from source on expiry. |
| 5 | `cost-unpredictable-runaway` | `decided` | ADR 0041, ADR 0012, ADR 0036, ADR 0010 | 3 Engine runtime; 10 Scheduling | Limits on the install or Daemon, a person, a named token and a Client connection, held by a per-user Usage ledger every engine of every major reserves against; uncapped runs marked; only a person raises a Limit. Credit billing is Goodfetch Cloud's. |
| 6 | `compliance-site-owner-politeness` | `decided` | ADR 0045, ADR 0021, ADR 0004, ADR 0016 | 5 Site access; 4 Security and egress | 402 is the Block cause `payment_required`, never evaded; opt-in Paid fetches through Cloudflare's headers and x402, capped and journaled; RSL read as a Site preference. Goodfetch cannot reduce the bot traffic other tools send or the blocks site owners place on shared IP ranges. |
| 7 | `compliance-security-privacy` | `decided` | ADR 0014, ADR 0015, ADR 0016, ADR 0017 | 4 Security and egress | Certifications such as SOC 2 attach to an organization operating a service, so they belong to Goodfetch Cloud, not to the framework. |
| 8 | `dx-setup-docs` | `decided` | ADR 0024, ADR 0028, ADR 0030 | 11 Client surfaces; 1 Repository, CI and release | Support channels and response times are a project-governance commitment, not an engine feature. |
| 9 | `agent-deterministic-replay` | `decided` | ADR 0009, ADR 0010, ADR 0013 | 9 AI and agent | Plans are versioned files replayed without a model; the recorder also emits TypeScript or Python code. |
| 10 | `silent-false-success` | `decided` | ADR 0006, ADR 0013, ADR 0026, ADR 0029 | 6 Collection; 9 AI and agent | Page check, Success checks, Grounding, typed Challenge causes and the three-axis report. |
| 11 | `agent-unreliable-drift` | `decided` | ADR 0009, ADR 0013, ADR 0019 | 9 AI and agent | Goodfetch does not make a model succeed at an arbitrary task; it makes an agent's failure visible (Success checks, Verdict) and its success repeatable (Plans). |
| 12 | `integ-sdk-llm-runtime` | `decided` | ADR 0012, ADR 0022, ADR 0024, ADR 0049, ADR 0056, ADR 0078 | 11 Client surfaces; 5 Site access; 13 Self-hosted deployment and operations | Three SDKs, model formats, third-party CDP browsers. Edge and serverless Clients over WebSocket (ADR 0049). `run.borrow()` relays the user's own Playwright or Puppeteer browser over the SDK's JSON-RPC connection; `cdp_url` covers Selenium and other tools. |
| 13 | `cost-infra-proxy-spend` | `decided` | ADR 0020, ADR 0021, ADR 0025, ADR 0035 | 8 Stealth and Challenge solving; 5 Site access | Proxy and CAPTCHA-service prices are set by their vendors; Goodfetch minimizes how much of them a run uses and reports the spend per verified result. |
| 14 | `data-extraction-quality` | `decided` | ADR 0007, ADR 0013, ADR 0025 | 6 Collection; 2 Proving ground and conformance | Grounding, an extractor chosen on an independent corpus, and an extraction-quality benchmark family. |
| 15 | `ops-scale-ratelimit-scheduling` | `decided` | ADR 0004, ADR 0021, ADR 0036, ADR 0049, ADR 0053, ADR 0054, ADR 0055 | 3 Engine runtime; 10 Scheduling; 13 Self-hosted deployment and operations | Backoff, fair admission and the Daemon scheduler; Politeness (per domain and per proxy) shared by every run of a user and across machines through an optional Politeness store (ADR 0053); Partitioned crawls past one engine (ADR 0054); a Daemon group with a Lead Daemon, so the Helm chart runs any number of pods (ADR 0055). |
| 16 | `rel-hang-crash` | `decided` | ADR 0003, ADR 0019, ADR 0024, ADR 0033 | 3 Engine runtime | Leases, CDP command timeouts, process groups, Plugin heartbeat and resume from Checkpoint. |
| 17 | `selfhost-parity-lockin` | `decided` | ADR 0001, ADR 0018, ADR 0031, ADR 0049 | 13 Self-hosted deployment and operations; 14 Cloud-readiness contracts | Nothing held back to sell; Goodfetch's own solving logic is Private (ADR 0018). A signed Helm chart passing the `restricted` Pod Security level, tested on `kind`, and six supported targets (ADR 0049). |
| 18 | `agent-human-in-loop` | `decided` | ADR 0006, ADR 0014, ADR 0027, ADR 0031 | 12 Console; 9 AI and agent | Human pause, confirmation categories, Live view takeover and the "Needs you" list. |
| 19 | `dx-observability` | `decided` | [#70](https://github.com/Jbmanllr/automan/issues/70) item 9, ADR 0031, ADR 0050 | 3 Engine runtime; 13 Self-hosted deployment and operations | OTLP export decided in [#70](https://github.com/Jbmanllr/automan/issues/70) item 9; run traces on the Timeline. Run and engine metrics through OTLP and a token-protected Prometheus endpoint, with versioned names; token-free `/livez` and `/readyz` (ADR 0050). |
| 20 | `maint-site-drift` | `decided` | ADR 0007, ADR 0009, ADR 0013 | 9 AI and agent | Fingerprinted Targets; Drift caught before the first Effect; one verified Repair. |
| 21 | `maint-upgrade-regression` | `decided` | ADR 0025, ADR 0030, ADR 0032 | 1 Repository, CI and release; 2 Proving ground and conformance | One lockstep version, semver contracts, and a benchmark regression gate on every minor. |
| 22 | `perf-memory-concurrency` | `decided` | ADR 0040, ADR 0004, ADR 0020, ADR 0024 | 3 Engine runtime | Resource-aware concurrency from container limits, browser recycling on memory growth, and a shedding ladder ending in a resumable `resources_exhausted` stop. |
| 23 | `rel-incorrect-incomplete` | `decided` | ADR 0003, ADR 0005, ADR 0008, ADR 0026 | 6 Collection | Outputs visible exactly once, delivery IDs, upsert on declared keys, and the Completeness axis. |
| 24 | `durability-resume` | `decided` | ADR 0002, ADR 0003, ADR 0010 | 3 Engine runtime | Per-run SQLite, portable Checkpoints, epochs, and Effects never repeated blindly. |
