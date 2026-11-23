# A crawl declares its Hooks per stage in code, a Hook returns its change, and a program registers its own Plugins by name

A program plugs into a crawl in two ways.
- **Hooks in code**, declared on the `Crawl` beside its handlers, one method per stage whose answer changes the work, scoped by Label handles and run in the order declared. A Hook returns its change as a value and ends the work with a call that never returns.
- **Its own implementations of Plugin kinds**, ADR 0033's SDK handlers, registered on its connection by name in one option per kind and chosen by the same settings that choose an installed Plugin.

The leaders fail here in three ways ([crawl frameworks](https://github.com/goodfetch/goodfetch/blob/research/consumer-api/rnd/research/research_notes/Consumer%20API%20of%20competitors/crawl-frameworks.md) §6.9):
- **Integers order the components.** Scrapy 2.19 keys its middlewares, pipelines and extensions by import path with integers ([`default_settings.py`](https://github.com/scrapy/scrapy/blob/2.19.0/scrapy/settings/default_settings.py#L327-L344): `OffsiteMiddleware` 50, `RetryMiddleware` 550), and its users ask how to pick them ([SO 6623470](https://stackoverflow.com/questions/6623470)) and what separates its two kinds of middleware ([SO 17872753](https://stackoverflow.com/questions/17872753)).
- **Hooks with no deadline of their own.** Crawlee 3.18's navigation hooks run only inside the outer handler timeout ([basic-crawler.ts](https://github.com/apify/crawlee/blob/v3.18.2/packages/basic-crawler/src/internals/basic-crawler.ts#L2025-L2034)); v4 bounds them ([upgrade guide](https://github.com/apify/crawlee/blob/8cb2eb61a0356a96205c08e02ed49cadc97081e5/docs/upgrading/upgrading-v4/upgrading-v4.md#L436-L442)).
- **Fetching tied to parsing.** Crawlee's crawler class picks both, and crawlee-python's [Design Inspiration From Scrapy](https://github.com/apify/crawlee-python/discussions/855) discussion asks to decouple them.

Decided in [Hooks and injectable components in the SDK API](https://github.com/goodfetch/goodfetch/issues/481), on the map [The SDK API of Collie and Foxhound](https://github.com/goodfetch/goodfetch/issues/461), every answer decided by the agent on the owner's delegation, with its [grilling record](https://github.com/goodfetch/goodfetch/issues/481#issuecomment-6181739598), [round 5](https://github.com/goodfetch/goodfetch/issues/481#issuecomment-6181969007), [round 6](https://github.com/goodfetch/goodfetch/issues/481#issuecomment-6187768415), [round 7](https://github.com/goodfetch/goodfetch/issues/481#issuecomment-6188086570), [round 8](https://github.com/goodfetch/goodfetch/issues/481#issuecomment-6189635610), [round 9](https://github.com/goodfetch/goodfetch/issues/481#issuecomment-6189837014), [round 10](https://github.com/goodfetch/goodfetch/issues/481#issuecomment-6190018867) and [round 11](https://github.com/goodfetch/goodfetch/issues/481#issuecomment-6190163609). Principles are cited by their IDs in [ADR 0077](0077-the-sdk-api-follows-eight-guarantees-and-four-ranked-shape-rules.md). It builds on these adopted decisions:
- [ADR 0085](0085-collies-page-api-keeps-playwrights-names-strictness-waits-values-cross-as-json-and-unanswered-requests-fall-through.md): Collie's page API and its routes;
- [ADR 0088](0088-collie-carries-every-capability-except-those-only-many-pages-need.md): which Hook stages are Collie's;
- [ADR 0090](0090-products-are-named-goodfetch-collie-and-goodfetch-foxhound-and-share-one-product-version.md): the Products' crate and connection names;
- [ADR 0092](0092-sessions-open-from-their-source-page-pause-waits-for-a-person-and-a-secret-is-a-placeholder-bound-to-domains.md): Secret placeholders and `resolveSecret`;
- [ADR 0093](0093-a-crawl-declares-its-labels-and-one-handler-each-and-a-handler-reads-collies-page-on-every-fetch-tier.md): the crawl, its Labels, handlers and crawl requests, each with its method, headers and body;
- [ADR 0103](0103-a-handler-pushes-records-staged-with-its-attempt-into-datasets-its-crawl-declares-and-a-reader-never-slows-the-crawl.md): Datasets, the rejection table and Sinks in code.

It also builds on decisions in review when it was made:
- ADR 0101 ([PR #515](https://github.com/goodfetch/goodfetch/pull/515)): the program's own Model provider;
- ADR 0102 ([PR #521](https://github.com/goodfetch/goodfetch/pull/521)): the Run's progress, outcomes and report.

## Decision

### Which stages reach code

- **Hooks in code run at the six stages whose answer changes the work:** before request, after response, before navigation, after navigation, page ready, and before a Record's commit (S2).
- **The observe-only stages are not Hooks in code.** The run's start, Checkpoint, migration, Limit reached and stop, and a new **request ended** stage, reach a program through the Run's own values ([The Run in code](https://github.com/goodfetch/goodfetch/issues/478)): its progress, its outcomes and its report. ADR 0081 already shapes what a program watches, and a second, Hook-shaped way to watch the same thing breaks S2. They stay Hook stages for process Plugins and Script Hooks, which have no program to listen.
- **Request ended** receives a crawl request's outcome (ADR 0079), for a Plugin that, say, sends failed URLs to a queue. It is Foxhound's, as are before request, after response and before a Record's commit (ADR 0088). It is ADR 0037's failed-request handler (RT-04) for Plugins and Script Hooks; a program reads the outcome instead (ADR 0093).
- **Before the handler is page ready.** It runs on every tier, just before the handler, with the handler's page. A Label-scoped page-ready Hook is the router middleware that RT-07 says Label-scoped Hooks replace (Crawlee's `router.use`, S3).
- **After the handler is no stage.** Each of its jobs has one already:
  - a Record goes through the stage before its commit;
  - an enqueued request goes through before request, where a Hook may skip it;
  - the request's end is its outcome.
- **Navigation stages run only on the tiers that navigate to the request's URL:** `browser`, and `light_browser`, ADR 0020's light engine, which loads the page and runs its scripts (ADR 0093). On the others (`http`, `discovered_api` and `browser_fetch`, which fetches from inside the browser without navigating) a request has before request and after response, and page ready on every tier.
- **Before request is also the stage before a retry.** It runs on every fetch of a request: each retry, escalation and sample-comparison run. Its context carries the attempt, the tier about to fetch and `previousError`, the error of the attempt before (`null` on the first). A Hook may change the next attempt's URL or headers. It may stop the retries with `skip(message)`: the request then ends `skipped` with the reason `hook`, and its outcome still lists every failed attempt's error. RT-04's handler before a retry needs no stage of its own (S2).
- **A Collie program has no Hooks in code.** It navigates its own page, so each stage is already a line of its own code or a page call (ADR 0085):
  - before a navigation, the lines before `goto`, or `session.route` for a navigation it did not start, such as the Agent's;
  - before each document, `addInitScript`;
  - an overlay before an action, `addLocatorHandler`.

  Collie's page stages (ADR 0088) still reach a Collie Run as Plugins and Script Hooks through settings (S2, S3).

### Declaring a Hook

- **One method per stage on the `Crawl`,** beside its Labels and handlers (ADR 0093). Hooks, like handlers, are never persisted, so the `Crawl` a Run resumes with brings them back.
  - **TypeScript:** `beforeRequest`, `afterResponse`, `beforeNavigation`, `afterNavigation`, `pageReady`, `beforeRecordCommit`, each taking the Hook and `{ labels, name }`.
  - **Python:** the same names in snake_case, as decorators on `Crawl` (plain functions) and `AsyncCrawl` (`async def`), each returning the function unchanged so a test calls it as it is, as `@crawl.handle(label)` does.
  - **Rust:** methods taking `AsyncFn` closures with a `HookOn` scope, answered inside `crawl.serve(&run)` as handlers are. They may borrow, and need neither `Send` nor `'static` (ADR 0081). On `goodfetch_foxhound::blocking::Crawl` they are `Fn + Send + Sync + 'static`, as its handlers are. Beside `run.crawl_requests()`, the way down beneath `serve` for handlers (ADR 0093), `run.hook_requests()` streams the Hook calls, so a program that answers its requests itself answers its Hooks too (G8). It has one reader, so a second stream while one is open fails with `setting_invalid`, and a call dropped unanswered fails its work as a Hook past `timeout.hook` does.
    > Amended by [ADR 0109](0109-a-rust-programs-own-code-returns-any-error-as-a-box-error-read-as-the-other-sdks-read-an-exception.md): a Rust Hook returns its change or a `BoxError`, so `?` on any error of the program is the Hook's own exception (`hook_failed` with no wrapped code), and `HookCall::finish` takes a `BoxError`; a Hook's panic inside `serve` is `hook_failed` too.
  > Amended by [ADR 0111](0111-a-rust-crawl-hands-what-it-subscribes-on-its-page-to-serve-which-reads-it-for-the-rest-of-the-attempt.md): a Rust Hook at a page stage installs what must outlive it by handing the page's stream to `ctx.for_each(stream, handler)`, which `serve` reads for the rest of the attempt, and keeps an init script with `Registration::keep()`; on a context read from `run.hook_requests()`, `for_each` fails with `setting_invalid`.
    > Amended by [ADR 0112](0112-a-rust-crawls-handlers-and-their-futures-are-send-in-axums-form-so-serve-runs-in-a-multi-threaded-server.md): a Rust Hook takes a handler's form, `FnOnce(…) -> Fut + Clone + Send + Sync + 'a` with `Fut: Send`, so it may borrow but is `Send`, and `serve`'s future stays `Send`.
  > Note (agent decision, as built on 2026-11-23, [#2912](https://github.com/goodfetch/goodfetch/issues/2912)): a Run that `run.retry_failed` or `run.resume` reopens with no start under its ID has no `Crawl` to bring its Hooks back, so its Hooks in code would be skipped and a Record could commit without its `before_record_commit` Hook. The run file of a crawl therefore records its Hooks in code as declared, each one's name, stage, Labels and Datasets, never its code: at the Run's start, and again when a start under its ID resumes it or `run.retry_failed` reopens it after one, so a start that declares none clears them. A reopen of a Run whose run file records a Hook in code, with no start under its ID in this engine to declare them again, is refused before the Run reopens, nothing done, with `hook_undeclared`, whose details carry the Run ID and the Hooks recorded. Its fix: resume the Run through `crawl.start` with its Run ID, from the program that declares them; for `run.retry_failed` on an ended Run, call `crawl.start` with its Run ID first, which answers the Run as it ended and keeps the declarations, then `run.retry_failed`. The check reads a Checkpoint the resume names as it reads a run file, once the Run's lock is taken, so a Run another writer holds is refused as in use first; as `crawl.start` takes no Checkpoint yet, a Checkpoint of such a Run cannot be resumed, which a start taking one would end. A live crawl's `run.retry_failed` re-admits its requests into the crawl, which holds its Hooks, and a Run whose crawl declared no Hook in code reopens as before. Having the two calls take the Hooks again was rejected, as Hooks as start options were (Considered Options): a Hook is code the program runs, and these calls often come from the CLI or the Console, which hold none.

  One method per stage gives each stage its own typed context and doc comment.
- **Stage names.** The Operation catalog names the stages `before_request`, `after_response`, `before_navigation`, `after_navigation`, `page_ready`, `before_record_commit` and `request_ended`. The lifecycle stages keep ADR 0033's names until area 7's spec writes them. The same names serve Plugin manifests, Script Hooks and the Run trace (S1).
- **Scope.**
  - `labels` takes typed handles (ADR 0082). A Hook scoped to one Label sees its Payload typed, and to several, their union. Python's types collapse a mixed list to `object`, and Rust's `HookOn::only` gives a JSON value.
  - A Label another crawl declared fails at once with `label_unknown`, and a Dataset another crawl declared with `dataset_unknown` (ADR 0103).
  - With no `labels`, a Hook runs for every Label and sees the Payload as `unknown`.
  - The stage before a Record's commit also takes `datasets`, ADR 0103's Dataset handles, so its Hook sees and returns the Dataset's Record type (G3).
- **A Hook that can never run is refused (G3).** An empty `labels` or `datasets` list fails at once with `hook_ineffective`. So does, when the Run starts, a navigation Hook all of whose Labels force a tier that does not navigate (`http`, `discovered_api`, `browser_fetch`) through `fetch.tier`.
- **Order.** Hooks run per stage in the order declared, scoped and unscoped alike. There are no integers and no priorities.
  - Hooks named in settings (process Plugins and Script Hooks under `hooks.*`, whose schema is area 7's spec) run first, layer by layer from Operator policy to the run layer.
  - The crawl's Hooks in code run last, so the program's own Hooks see what the others changed, as the nearest scope wins in ADR 0083.
  - A Hook's change is no lock: a later Hook may undo an earlier one's header. A value an operator must enforce is a locked setting (ADR 0028).
- **Name.** `name` defaults to the function's own name where the language has one, else to the stage and its position (`before_request#2`), which a Python `<lambda>` and a Rust closure also get. A second Hook of one name on one crawl fails at once with `hook_duplicate`, so a name identifies one Hook: in `hook_failed`, in the Run trace (G6), and for a test's `runHook` ([Testing a user's crawl or automation](https://github.com/goodfetch/goodfetch/issues/482) (ADR 0105)).

### What a Hook receives and returns

Every context carries `request` (id, URL, Label, typed Payload, depth, attempt) and `run`, for its Key-value store (ADR 0103), which Script Hooks also have (ADR 0051). A context belongs to one call: once the Hook has returned or ended the work, a call on it fails with `attempt_ended` (ADR 0093).

> Amended by [ADR 0106](0106-a-program-declares-an-effect-as-a-keyed-block-whose-output-the-effect-journal-keeps-judged-by-what-the-engine-sent.md): a context's `run` also declares Effects with `run.effect`, in its crawl request's namespace; a Hook call belongs to its request's attempt for the window, `effect_key_reused` and `effect_staging_refused`.

| Stage | Also in the context | Returns | Ends the work with |
|---|---|---|---|
| before request | the tier about to fetch, the request's `method` and `body` as enqueued, the headers as built so far (the enqueue's own included), `previousError` | `{ url?, headers? }`, or nothing | `skip(message)`: `skipped`, reason `hook` |
| after response | the final `response`, after redirects | `{ body? }`, or nothing | `block(cause)`: ends the attempt `blocked` with that Block cause; ADR 0021 escalates by new Session, and the request ends `blocked` once escalation runs out. `retry(message)`: `retry_requested` (ADR 0093) |
| before navigation, after navigation, page ready | `page`, Collie's page (ADR 0085) | nothing | page calls only |
| before a Record's commit | `dataset`, `record` | the Record to commit, changed or not, or nothing | `drop(reason)`: counted, never stored; `reject(reason)`: into the Dataset's rejection table, read with `run.rejectedRecords` (ADR 0103) |

- **Why a returned change.**
  - The type says exactly what each stage may change (G3).
  - A Hook stays a plain function that a test calls and checks.
  - Nothing half-changed crosses the connection.
  - Rust returns a value rather than holding a mutable borrow across an await.

  The calls that end the work never return, as a handler's `skip` and `retry` (ADR 0093). They are typed `never` in TypeScript and `NoReturn` in Python; Rust returns them as `Err`.
  > Amended by [ADR 0109](0109-a-rust-programs-own-code-returns-any-error-as-a-box-error-read-as-the-other-sdks-read-an-exception.md): Rust's `skip`, `block`, `retry`, `drop_record` and `reject` return a `BoxError` holding their `Error`, so `return Err(ctx.skip(…))` is the form.
- **Hooks run in turn,** each seeing what those before it changed.
- **After response runs on every final response,** before ADR 0093's status rule. A soft block served with a 200 is marked there, and a 404 still fails unless `crawl.handle_statuses` lets it through.
- **Page stages follow the handler's tier rules (ADR 0093).** A call only a browser can serve escalates under `fetch.tier: auto`, or fails with `browser_required` under a forced tier.
- **At the Record stage, an answer never fails the request** (ADR 0103). A changed Record is checked against the Dataset's schema again, and rejected if it no longer fits.

### What a before-request Hook may change

- **The URL and headers, never the method or the body.** A crawl request carries its method, headers and body from its enqueue or seed (ADR 0093), so a crawl of a POST or GraphQL API states each request where it knows it, and a Hook reads them (to sign a request, say) without rebuilding them. The method and the body are part of the request's identity in the Frontier (ADR 0093), and whether a non-GET request is an Effect is [Effects a program declares in code](https://github.com/goodfetch/goodfetch/issues/500)'s ([PR #523](https://github.com/goodfetch/goodfetch/pull/523)). A Hook that changed them would change what the Frontier had deduplicated, what robots.txt and Politeness read, and whether the request is an Effect (ADR 0010), unseen (G5).
  > Amended by [ADR 0106](0106-a-program-declares-an-effect-as-a-keyed-block-whose-output-the-effect-journal-keeps-judged-by-what-the-engine-sent.md): a crawl request is a read whatever its method; the method and the body still stay the enqueue's, for the Frontier, robots.txt and Politeness.
- **A header set to `null` is removed.**
- **Bearer tokens are area 5's.** Whether a crawl request may carry `Authorization` built from a Secret placeholder, set by its enqueue or by a Hook in code, for an API behind a Bearer token, and what the engine's interception does with a route's `Authorization` or `Cookie` override on a crawl request's own fetch, are area 5's spec, under ADR 0092's rule on where a placeholder goes. Until then, a handler sends such a call with `page.request`, which takes a placeholder in any header (ADR 0092).
- **The engine's own headers are refused.** ADR 0033's forbidden headers (`Host`, `Cookie`, `Authorization`, `Proxy-*`, method overrides) fail the Hook with `header_forbidden`, wrapped in `hook_failed`.
- **Secrets.** A header's value may be a Secret placeholder, bound by `secrets.*` and checked against the request's URL, as in `page.request` and a route's overrides (ADR 0092). The engine adds cookies, Secret values and the Session's credentials after the last Hook, so no Secret value passes through the program, and the headers a Hook reads never hold them.
- **A rewritten URL is checked against the crawl's scope and the run's domain policy,** whatever form the Hook takes. A rewrite that leaves them ends the request `skipped` (`out_of_scope`, `domain_policy`, ADR 0093).
  - **A Hook in code** may rewrite to any host that check admits, since it is the program's own code and the scope is the program's own statement of where its crawl may go.
  - **A Plugin or a Script Hook** must also keep ADR 0033's rule: the original request's registrable domain, or else a domain it declares.
  - An enqueue's other checks (`duplicate`, `max_depth`, `max_requests`) do not apply, since the request was already admitted.
  - robots.txt and Site preferences read the URL as rewritten, so a rewrite cannot step around them (G5).
- **Hooks come before routes.** A crawl request's fetch passes, in this order:
  1. its before-request Hooks, those named in settings, then the crawl's in code;
  2. on a tier where the page's routes see the request (`page.route`, `session.route`, ADR 0085), those routes, which see the Hook's URL and headers, and whose overrides win, since they come later;
  3. the run's domain policy and Filter lists, on what the last of them sends on (ADRs 0014, 0047, 0085), and ADR 0087's navigation gate, with its Politeness permit and robots.txt, for the host it goes to;
  4. the engine's own interception, which adds cookies, Secret values and the Session's credentials, so neither a Hook nor a route reads them.

  An after-response Hook sees the response the page received, one a route fulfilled included.
- **A route holds a crawl request's own fetch to a Hook's rules.** On the request's own document fetch, the original request a route reads (ADR 0085) is the one its last before-request Hook left. A route's override of the method or the body fails the route with `page_value_invalid`, and the request is aborted as when a route throws (ADR 0085), so its attempt fails with that code, since the Frontier, robots.txt, Politeness and the Effect classing would not see it (G5). A route's new URL is checked as a Hook's rewrite is (above), so one that leaves the scope, the domain policy, robots.txt or Site preferences ends the request `skipped` with that reason, and nothing is sent. The page's other requests (its scripts, images and calls) keep ADR 0085's rules.
  > Amended by [ADR 0106](0106-a-program-declares-an-effect-as-a-keyed-block-whose-output-the-effect-journal-keeps-judged-by-what-the-engine-sent.md): a crawl request is a read whatever its method, so no Effect classing judges it; a route's change of the method or the body still fails, since the Frontier and robots.txt would not see it.

### Running and failing

- **Where a Hook in code runs** is ADR 0081's:
  - TypeScript: the program's event loop;
  - Python: the Run's task group (async), or the connection's thread pool (sync);
  - Rust: inside `crawl.serve(&run)`.

  It is the program's own code (ADR 0033): unconfined, accounted, and in the Run trace by name. Records it changed say so in their Provenance.
- **Every Hook in code is a round trip.** Each request of its Labels waits on the program, as under a `**/*` route (ADR 0085). A request is leased to the program from its first Hook in code to its commit, so `crawl.handler.concurrency`, which counts those leases (ADR 0081), bounds Hooks and handlers together. A before-request Hook runs before its request takes its Politeness permit, since a rewrite may change the host. So that one slow domain cannot hold every lease, the engine hands a request to its Hooks only once its own domain's permit is free. A request rewritten to another host waits for that host's permit holding its lease, a cost the rewrite brings. At most `crawl.handler.concurrency` requests of the Hook's Labels, 16 by default, are leased at once. A Script Hook named in settings runs in the engine's Script sandbox with no round trip to the program (ADR 0051), and is the way down for a crawl that cannot afford one (G8).
- **Detaching.** A Run with a Hook in code cannot be detached (ADR 0078), as a Run with handlers cannot; nor can a Run that uses an SDK handler, chosen by its settings or by `registered`, since the handler lives in the program. The refusal's `fix` names the key that chose it.
- **Timeout.** `timeout.hook` (10 s, ADR 0080) bounds each call. Past it, the call fails with `deadline_exceeded` naming that setting and the stage, and is retried as any `deadline_exceeded` is. A Label may set `timeout.hook` for the Hooks on its requests.
- **`hook_failed`.** A Hook that raises fails the work with `hook_failed`, a `ProgramError`. Its details are:
  - `stage`, `hook` (its name) and `label` (`null` at a lifecycle stage, which has no request);
  - the exception's type, message and stack, as `handler_failed` carries a handler's (ADR 0079), and, like it, always named in its message;
  - `wrappedCode`, the Goodfetch code raised inside, or `null` for the program's own exception.
- **Retries.**
  - The program's own exception is not retried unless `crawl.retry.handler_failed` is on. A bug in a Hook fails the same way on every attempt, and each attempt fetches the page again (G5).
  - A wrapped code is retried when its retryable flag says so (ADRs 0051, 0079).
  - Either retry draws on the request's `crawl.retry.max`, which its Hooks share with its handler (ADR 0093).
- **A Hook that raises at the Record stage fails the attempt's commit.** Everything the attempt staged is discarded together, so no attempt half-commits (ADR 0003).

### What a program may replace

The research's injectable components map onto Plugin kinds (ADR 0033), or onto none:
- **Fetcher:** the Fetch backend kind, which a program may implement (below).
- **Parser: no component is needed.** Fetching is already decoupled from parsing:
  - the tier is a setting or learned, never a crawler class (ADR 0093);
  - the handler reads `ctx.page`, Collie's page on every tier, or parses the response's body with any library it imports, since it is the program's own code.

  crawlee-python's [extending guide](https://github.com/apify/crawlee-python/blob/v1.10.4/docs/guides/extending_crawlee.mdx) still pairs each custom parser with a crawler class of its own, the coupling this avoids. The Extractor kind replaces main-content extraction (ADR 0007), a different job.
- **Storage: declined,** as ADR 0033 declined storage backends: a replaceable store breaks portable, resumable runs (ADR 0002). Delivering Records anywhere is a Sink's job (ADR 0103), and a program may write one (below).
- **Session pool: declined.** ADR 0021's escalation by new Session, ADR 0053's shared Politeness and ADR 0035's stealth meet in the pool, and a replaced pool would step around all three (G5). Crawlee v4 accepts any `ISessionPool`. Goodfetch's pool is shaped by settings instead (`session.*`, `proxy.*`, ADR 0092), and a Proxy provider supplies its proxies.
- **Proxy:** the Proxy provider kind, one proxy per Session (ADRs 0021, 0092).
- **Tier predictor: declined.** Crawlee v4 accepts any `IRenderingTypePredictor`. ADR 0020 learns the tier by comparing a sample's results fetched both ways. A predictor in the program would put a round trip before every fetch with no result to check it against. `fetch.tier` per Label is the way down (ADR 0093, G8).

### The program's own Plugins: SDK handlers

- **Registered on `connect()`,** in one option per kind, each a map keyed by name, as ADR 0101 registers `modelProviders`. Every kind has one, except the Hook (declared on the `Crawl`, above), the Model provider (`modelProviders`, ADR 0101) and the Secret provider (`resolveSecret`, ADR 0092).
  - **TypeScript:** `fetchBackends`, `proxyProviders`, `sinks`, and so on for each kind, generated from the catalog's kinds.
  - **Python:** the same in snake_case: coroutine functions on the async client, run in the connection's task group; plain functions on the sync client, run on its thread pool (ADR 0081).
  - **Rust:** `ConnectOptions` declares each handler's name, one builder method per kind (`fetch_backend(name)`, `proxy_provider(name)`, `sink(name)`), and `gf.fetch_requests(name)`, `gf.proxy_requests(name)` and `gf.sink_requests(name)` read its calls as an `EventStream`, since the async API spawns nothing (ADR 0081). Reading a name not declared fails with `setting_invalid`, and so does a second stream for a name while one is open, since calls that need an answer have one reader. A call waits for its stream until its own deadline. A call dropped unanswered, or held by a stream dropped mid-Run, fails with its kind's code for an unreachable implementation, which area 7's spec names for each kind, as a dropped model call is `model_unavailable` (ADR 0101), and is retried as that code is.
  - **Rust's Secret provider is declared the same way.** `ConnectOptions::resolve_secret()` declares that the program answers `caller:` references, and ADR 0092's `gf.secret_requests()` reads them under the rules above. Without the declaration a `caller:` reference raises `secret_unavailable`, its `fix` naming the option, as in TypeScript and Python, whether or not a stream happens to be open (S1, G2). A request dropped unanswered is `secret_unavailable` too (ADR 0092).
  - **In every language, a program's own implementation of a kind is this SDK-handler form, with every engine.** On a linked engine (ADR 0078), the Rust-trait form of ADR 0033 is the engine's seam for its built-ins, not part of the SDK API, so a Rust program has one form, as the others have (S1, S2).

  A map keyed by name makes a duplicate name impossible. A handler serves every Run of its connection, as an installed Plugin serves every Run of its engine. A Product's SDK has only the options of its own kinds: Collie's has no `fetchBackends` or `sinks`, since the Fetch backend and Sink kinds are Foxhound's (ADR 0088).
- **Chosen by name.** Settings choose a handler by its name wherever they choose an installed Plugin (ADR 0028's layers).
  - **The only handler of a kind that one setting chooses** (Fetch backend, Proxy provider, Browser backend, Extractor, Search provider, Challenge solver) becomes that setting's value at the connection scope, in the run layer with the origin `registered`, the lowest of the run layer's origins. Every other run-layer value replaces it: `connect()`'s own `settings`, a Run's, a Label's, a call's argument. Configuration does not, since the program's own code beats configuration files (ADR 0083). A lock on the key (ADR 0028), from Operator policy or a Recipe, wins over it with no refusal, since the program never wrote the key. `explainSettings()` shows the origin, or the lock (G6). Without this, a proxy pool registered and never chosen would leave a crawl on the user's own address (G5).
  - **Two handlers of such a kind fail `connect()` with `setting_invalid`** when nothing else sets that key for the connection (its `settings`, configuration or a lock), for the same reason (G5). The `fix` says to set the key, to one of them or to another implementation.
  - **Kinds that another setting or a reference names** take the handler's name there:
    - a Model reference `inhouse/<model>` (ADR 0101), and a Decision provider as a Model provider is;
    - a Secret reference `caller:` (ADR 0092), and a Code source in `secrets.<name>.code_source` (ADR 0092);
    - a Notification channel where its settings name channels (ADR 0027);
    - a Sink through its `sinks.<name>` entry (below).

    Area 7's spec writes each kind's choosing key under ADR 0083's naming rule.
  - **A handler no setting chooses yet is not refused.** The connection's later Runs may choose it, as a Model provider no reference names yet may be (ADR 0101).
- **Names that clash or that nothing answers fail with `setting_invalid`, its `fix` naming both ways to supply one: install a Plugin, or register a handler.**
  - At `connect()`: a handler named as a built-in or an installed Plugin of its kind, since they share one namespace, as ADR 0101's provider names do.
  - When a scope opens: a setting naming a handler that is neither installed nor registered.
  - At a resume, before any work: a handler the Run used that the program no longer registers. Run state records the handler's name and the form `sdk_handler` in place of a digest (ADR 0033).
- **Failures.**
  - A `GoodfetchError` a handler raises keeps its code.
  - Any other exception is its kind's `<kind>_handler_failed`, a `ProgramError` carrying the handler's name and the exception's type, message and stack. Like `handler_failed`, its message always names that exception (ADR 0079), as ADR 0101's `model_handler_failed` does. `<kind>` is the kind's short name in the catalog: `fetch`, `proxy`, `sink`, and ADR 0101's `model`.
  - In Rust, `fail(error)` takes any error, the SDK's own `Error` (`goodfetch_foxhound::Error`, `goodfetch_collie::Error`, ADR 0090) keeping its code and any other becoming `<kind>_handler_failed`, so a Rust handler's own failure is as diagnosable as a TypeScript one's (G6). ADR 0101's draft fails a Rust model call with a `goodfetch::Error` only; [Models, usage and cost in the SDK API](https://github.com/goodfetch/goodfetch/issues/475) is asked to align.
    > Amended by [ADR 0109](0109-a-rust-programs-own-code-returns-any-error-as-a-box-error-read-as-the-other-sdks-read-an-exception.md): "any error" is `BoxError`, one type for every function of the program, read by the outermost error, with the details ADR 0109 gives a Rust error.
  - ADR 0092's `resolveSecret` keeps `secret_unavailable`, as decided there.
- **Retries of a handler's failure.**
  - A Sink's is retried on ADR 0008's schedule, since its delivery ID makes a retry safe.
  - Any other kind's `<kind>_handler_failed`: in a crawl, the crawl request it failed (its fetch, the Session it needed) is retried only when `crawl.retry.handler_failed` is on for that request's Label; in a Collie Run, the call that needed the handler fails, and nothing retries it (ADR 0079).
  - Every other code a handler's call ends with (a `GoodfetchError` the handler raised, an unreachable implementation's code) follows its own retryable flag.
- **A program's Fetch backend never carries a request that uses a Secret or a logged-in identity,** as a third-party one never does (ADR 0033). Such requests include a logged-in Session's, one whose before-request Hook set a Secret placeholder, one whose Session has `session.http_credentials`, and one whose Session's proxy has a password (`proxy.password` is a Secret reference, ADR 0092, as is a Proxy provider's). The engine sends those itself, so no logged-in Session's cookie and no Secret value passes through the program (ADR 0092), even when the backend was chosen only by `registered`. Every other request reaches the backend with its method, headers and body (ADR 0093), an anonymous Session's ordinary cookies (ADR 0033), and its Session's proxy server when it has one, so a proxied crawl does not leave from the user's own address. A response's `Set-Cookie` updates the Session, as a third-party backend's does. The Run trace marks each request the engine sent itself in place of the backend, with the reason, so a backend that receives nothing says why (G6).
- **A program's Sink** (ADR 0026's SDK-handler form) serves the `sinks.<name>` entry of its name. That entry gives its Datasets, write mode and key as any Sink's does (ADR 0103), and a `type` beside a handler of that name is refused with `setting_invalid` when the Run starts. Each delivery carries ADR 0008's delivery ID and run epoch, and its Guarantee class is probed as any Sink's (ADR 0026). `run.records()` stays the reader with no delivery state (ADR 0103).
- **Rights.** A handler runs with the program's full rights, never confined, and a third-party package used as one does too, as ADR 0033 tells the docs to say.

### Error codes

They join `goodfetch-protocol`'s list in the `program` group (ADR 0079).

| Code | Retryable |
|---|---|
| `hook_failed` (its details above) | as its wrapped code; for the program's own exception, only under `crawl.retry.handler_failed` |
| `header_forbidden`, raised inside `hook_failed`; `hook_ineffective`; `hook_duplicate` | no |
| `sink_handler_failed` | yes, on ADR 0008's schedule |
| every other `<kind>_handler_failed`: `fetch_handler_failed`, `proxy_handler_failed` and those area 7's spec names | in a crawl, only under `crawl.retry.handler_failed`; in a Collie Run, no |

## Considered Options

- **Every stage as a Hook in code, lifecycle included:** rejected by S2. An observe-only Hook is a listener under another name, beside the Run's own values.
- **Before request and after response only,** as Scrapy's downloader middleware: rejected. The handler starts after the navigation, so no code could block resources or set a cookie before it.
- **A "before retry" stage:** rejected by S2. Before request already runs on every attempt and sees the previous one's error.
- **An "after the handler" stage,** as Scrapy's spider middleware: rejected. Its jobs are each another stage's, and the split between Scrapy's two kinds of middleware is what its users ask about.
- **Hooks on a Collie Run or Session:** rejected by S2. `goto`, routes, init scripts and locator handlers already answer each stage, under Playwright's names (S3).
- **Hooks in each Label's options:** rejected. A Hook shared by several Labels would be repeated, its order across Labels would be unclear, and unscoped Hooks would still need a home.
- **Arrays in the crawl's options,** as Crawlee's `preNavigationHooks`: rejected. A Label handle exists only after its declaration, so a scoped Hook could not be built in the same expression.
- **Hooks as options of the Run's start:** rejected. Hooks are never persisted, and the `Crawl` already carries them through a resume.
- **Import paths with integer orders,** as Scrapy: rejected. Declaration order says the same with nothing to pick.
- **One `hook(stage, fn)` method:** rejected. Per-stage methods each carry their own typed context and documentation, and Python's decorators read as Crawlee for Python's.
- **Mutating a request object in the context** (Crawlee's hooks): rejected. The SDK would diff what changed, a refused header would be found only after the Hook returned, and Rust would hold a mutable borrow across awaits. **Scrapy's union of return types:** rejected by S2, and returning a response would let a Hook fake a page.
- **A before-request Hook that sets the method and a body,** as Scrapy's `FormRequest` and Crawlee's `method` and `payload` allow on a request: rejected by G5. It would change the request unseen by deduplication, robots.txt and the Effect classing. A crawl request takes its method and body at its enqueue (ADR 0093), where Scrapy and Crawlee take them too.
  > Amended by [ADR 0106](0106-a-program-declares-an-effect-as-a-keyed-block-whose-output-the-effect-journal-keeps-judged-by-what-the-engine-sent.md): a crawl request is a read whatever its method, so the Effect classing no longer judges it; the rejection stands for deduplication and robots.txt.
- **Retrying a Hook's own exception by default,** as Crawlee retries an error in a navigation hook: rejected by G5. **Never retrying it, with no switch:** rejected. A transient failure in the program's own service would fail its request for good, where the handler's switch already exists.
- **A replaced Session pool, a storage client and a tier predictor,** as Crawlee v4 accepts: rejected, above.
- **One generic `connect({ plugins: [...] })` list for every kind:** rejected. It was this ticket's first answer, replaced once ADR 0101 registered Model providers per kind: two forms for one concept break S1. **Rust traits run on the connection's runtime:** rejected for the same reason, beside ADR 0092's and ADR 0101's streams, and so is keeping the Rust-trait form public on a linked engine.
- **Rust handlers registered while their stream is open,** as ADR 0101's draft does: rejected. The connection's choice would change after its scope opened, against ADR 0083's "no setters" (G2), and `connect()` could not refuse a clash.
- **The registered handler as the connection's explicit value, above `settings`:** rejected by G3. It would beat `connect()`'s own `settings` naming an installed Plugin. **The registered handler as a built-in default, below configuration:** rejected by G2. A configuration file would then beat the program's own code (ADR 0083). **Two handlers of a kind, neither chosen:** rejected by G5. Runs would go out from the user's own address.
- **The kind's own failure code for an SDK handler's exception** (a Sink's unavailability, say): rejected. A program's exception is the program's to fix, a `ProgramError` by remedy (ADR 0079), as `handler_failed`, `hook_failed` and `model_handler_failed` already are.
- **Refusing a handler no setting chooses:** rejected. A connection serves many Runs, and a later one may choose it.
- **A service locator** (Crawlee for Python's): rejected by G2.

## Consequences

- **Amends ADR 0033.**
  - The Rust-trait form is the engine's seam for its built-ins, not a form of the user's own code.
  - In the SDK API, Hooks in code are a crawl's, at the stages that change the work. Collie's page stages have no Hook in code. The observe-only stages reach a program as the Run's values.
  - A program's own implementation of a kind is the SDK-handler form with every engine; the Rust-trait form is the engine's seam for its built-ins.
  - Before request no longer changes the body, and every form's rewrite is checked against the crawl's scope.
  - The new observe-only stage `request_ended` is added.
  - A Hook before a Record's commit may be scoped by Dataset.
  - Hooks named in settings run before those in code, layer by layer.
  - A Hook's own exception follows `crawl.retry.handler_failed`.
  - SDK handlers are registered by name, one option per kind, and fail as `<kind>_handler_failed`.
- **Amends ADR 0079.** `hook_failed` names its stage, Hook and Label, and its wrapped code. An SDK handler's exception is its kind's `<kind>_handler_failed`, whose message, like `handler_failed`'s, always names the program's exception, in Rust's `Display` too.
- **Amends ADR 0083.** The only SDK handler of a kind that one setting chooses becomes that setting's connection value, in the run layer with the origin `registered`, below every other run-layer origin.
- **Amends ADR 0093:**
  - `crawl.retry.handler_failed` also covers `hook_failed` and a crawl request failed through an SDK handler;
  - a before-request Hook changes a crawl request's URL and headers, never the method or body it was enqueued with, and its context shows them;
  - the scope is checked on a Hook's rewrite too, and `out_of_scope` covers a rewrite leaving it;
  - a Label takes `timeout.hook`;
  - Hooks are declared on the `Crawl`;
  - RT-04 becomes `decided`, since before request is the stage before a retry.
- **Amends ADR 0092.** A before-request Hook's header is among the places a Secret placeholder may go, checked against the request's URL at every hop. Rust declares its Secret provider when connecting (`ConnectOptions::resolve_secret()`), and `gf.secret_requests()` reads what it declared.
- **Amends ADR 0085.** On a crawl request's own document fetch, a route reads the request as its last before-request Hook left it, may not change its method or body, and has a new URL checked as a Hook's rewrite.
- **Amends ADR 0103.** A Hook's own exception at the Record stage is retried only under `crawl.retry.handler_failed`, and a Sink written in the program is registered on its connection by name.
- **Amends ADR 0088.** The new stage `request_ended` is Foxhound's, beside before request, after response and before a Record's commit.
- **Amends ADR 0026.** A program's Sink is registered on its connection by name, serves the `sinks.<name>` entry of its name, and is the SDK-handler form in Rust too; a Rust-trait Sink is no third-party form.
- **`goodfetch-protocol`** gains `header_forbidden`, `hook_ineffective`, `hook_duplicate` and each kind's `<kind>_handler_failed`, through `xtask codegen`'s naming checks (ADR 0079).
- **`GLOSSARY.md`.**
  - **SDK handler** is added.
  - **Hook** gains _Avoid_: pipeline (for a Hook before a Record's commit). Scrapy's item pipelines split here into that Hook and a Sink.
- **ADR 0037's matrix.**
  - RT-04 becomes `decided`.
  - RT-07, RT-08, RT-14, RT-21, SA-02, SA-17, SA-20, SA-22 and CO-05 cite this ADR.
  - Areas 3, 5, 6, 7 and 11 list it.
- **Areas.**
  - Area 7 (Plugin protocol) builds the stage names, the `hooks.*` schema, `request_ended`, each kind's generated handler, short name and `<kind>_handler_failed`, and each kind's choosing key under ADR 0083's naming rule.
  - Area 3 (Engine runtime) builds the order across forms, Hook timeouts and retries, a failed commit at the Record stage, and the `registered` origin.
  - Area 5 (Site access) builds the order of Hooks, routes and the engine's interception, and a route's limits on a crawl request's own fetch.
  - Area 6 (Collection) builds the scope check of a rewritten URL and an SDK-handler Sink's deliveries.
  - Area 11 (Client surfaces) builds the Hook methods, their contexts, the handler options and Rust's declared names and streams.
- **`TESTING.md`** gains the Hook conformance cases.
- **The `prototype/sdk-api` sketch** holds the `hooks` files in each language and snippets on golden scenario 05's crawl. The pain coverage matrix closes P46 and K12 as API.
- **Left to other tickets.**
  - [The Run in code](https://github.com/goodfetch/goodfetch/issues/478): a discrete event for an observe-only stage its progress and report do not already show, such as Limit reached or migration.
  - [Models, usage and cost in the SDK API](https://github.com/goodfetch/goodfetch/issues/475): Rust's declared names and `fail(error)` taking any error, for `model_requests`; and no Rust-trait Model provider in the SDK API.
  - [Testing a user's crawl or automation](https://github.com/goodfetch/goodfetch/issues/482) ([PR #522](https://github.com/goodfetch/goodfetch/pull/522)): building a Hook's context to call a Hook in a test.
    > Amended by [ADR 0105](0105-a-programs-tests-replay-a-response-recording-in-a-test-engine-that-reaches-no-site.md): a test builds no Hook context; the testing kit's `runHook(crawl, name, input)` builds the stage's context from the input, runs that one Hook, and returns its change, the call that ended the work, or `hook_failed`.
  - [API sketch: type-checked stubs and golden scenarios](https://github.com/goodfetch/goodfetch/issues/483): placing a before-request Hook in golden scenario 05 or 08.
