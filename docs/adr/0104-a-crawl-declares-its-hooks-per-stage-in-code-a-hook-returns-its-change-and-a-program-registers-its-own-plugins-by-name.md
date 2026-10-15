# A crawl declares its Hooks per stage in code, a Hook returns its change, and a program registers its own Plugins by name

A program plugs into a crawl in two ways.
- **Hooks in code**, declared on the `Crawl` beside its handlers, one method per stage whose answer changes the work, scoped by Label handles and run in the order declared. A Hook returns its change as a value and ends the work with a call that never returns.
- **Its own implementations of Plugin kinds**, ADR 0033's SDK handlers, registered on its connection by name in one option per kind and chosen by the same settings that choose an installed Plugin.

The leaders fail here in three ways ([crawl frameworks](https://github.com/goodfetch/automan/blob/research/consumer-api/rnd/research/research_notes/Consumer%20API%20of%20competitors/crawl-frameworks.md) §6.9):
- **Integers order the components.** Scrapy 2.19 keys its middlewares, pipelines and extensions by import path with integers ([`default_settings.py`](https://github.com/scrapy/scrapy/blob/2.19.0/scrapy/settings/default_settings.py#L327-L344): `OffsiteMiddleware` 50, `RetryMiddleware` 550), and its users ask how to pick them ([SO 6623470](https://stackoverflow.com/questions/6623470)) and what separates its two kinds of middleware ([SO 17872753](https://stackoverflow.com/questions/17872753)).
- **Hooks with no deadline of their own.** Crawlee 3.18's navigation hooks run only inside the outer handler timeout ([basic-crawler.ts](https://github.com/apify/crawlee/blob/v3.18.2/packages/basic-crawler/src/internals/basic-crawler.ts#L2025-L2034)); v4 bounds them ([upgrade guide](https://github.com/apify/crawlee/blob/8cb2eb61a0356a96205c08e02ed49cadc97081e5/docs/upgrading/upgrading-v4/upgrading-v4.md#L436-L442)).
- **Fetching tied to parsing.** Crawlee's crawler class picks both, and crawlee-python's [Design Inspiration From Scrapy](https://github.com/apify/crawlee-python/discussions/855) discussion asks to decouple them.

Decided in [Hooks and injectable components in the SDK API](https://github.com/goodfetch/automan/issues/481), on the map [The SDK API of Collie and Foxhound](https://github.com/goodfetch/automan/issues/461), every answer decided by the agent on the owner's delegation, with its [grilling record](https://github.com/goodfetch/automan/issues/481#issuecomment-6181739598) and [round 5](https://github.com/goodfetch/automan/issues/481#issuecomment-6181969007). Principles are cited by their IDs in [ADR 0077](0077-the-sdk-api-follows-eight-guarantees-and-four-ranked-shape-rules.md). It builds on decisions in review when it was made:
- [ADR 0093](0093-a-crawl-declares-its-labels-and-one-handler-each-and-a-handler-reads-collies-page-on-every-fetch-tier.md): the crawl, its Labels and handlers;
- ADR 0085 ([PR #501](https://github.com/goodfetch/automan/pull/501)): Collie's page API and its routes;
- ADR 0088 ([PR #504](https://github.com/goodfetch/automan/pull/504)): which Hook stages are Collie's;
- ADR 0092 ([PR #509](https://github.com/goodfetch/automan/pull/509)): Secret placeholders and `resolveSecret`;
- ADR 0101 ([PR #515](https://github.com/goodfetch/automan/pull/515)): the program's own Model provider;
- ADR 0102 ([The Run in code](https://github.com/goodfetch/automan/issues/478)): the Run's progress, outcomes and report;
- ADR 0103 ([PR #516](https://github.com/goodfetch/automan/pull/516)): Datasets and Sinks in code.

## Decision

### Which stages reach code

- **Hooks in code run at the six stages whose answer changes the work:** before request, after response, before navigation, after navigation, page ready, and before a Record's commit (S2).
- **The observe-only stages are not Hooks in code.** The run's start, Checkpoint, migration, Limit reached and stop, and a new **request ended** stage, reach a program through the Run's own values ([The Run in code](https://github.com/goodfetch/automan/issues/478)): its progress, its outcomes and its report. ADR 0081 already shapes what a program watches, and a second, Hook-shaped way to watch the same thing breaks S2. They stay Hook stages for process Plugins and Script Hooks, which have no program to listen.
- **Request ended** receives the request's outcome (ADR 0079), for a Plugin that, say, sends failed URLs to a queue. It is ADR 0037's failed-request handler (RT-04) for Plugins and Script Hooks; a program reads the outcome instead (ADR 0093).
- **Before the handler is page ready.** It runs on every tier, just before the handler, with the handler's page. A Label-scoped page-ready Hook is the router middleware that RT-07 says Label-scoped Hooks replace (Crawlee's `router.use`, S3).
- **After the handler is no stage.** Each of its jobs has one already:
  - a Record goes through the stage before its commit;
  - an enqueued request goes through before request, where a Hook may skip it;
  - the request's end is its outcome.
- **Navigation stages run only when a browser navigates.** A request on an HTTP tier has before request and after response, and page ready on every tier.
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
  - **Rust:** methods taking `AsyncFn` closures with a `HookOn` scope, answered inside `crawl.serve(&run)` as handlers are. They may borrow, and need neither `Send` nor `'static` (ADR 0081). On `goodfetch::blocking::Crawl` they are `Fn + Send + Sync + 'static`, as its handlers are.

  One method per stage gives each stage its own typed context and doc comment.
- **Stage names.** The Operation catalog names the stages `before_request`, `after_response`, `before_navigation`, `after_navigation`, `page_ready`, `before_record_commit` and `request_ended`. The lifecycle stages keep ADR 0033's names until area 7's spec writes them. The same names serve Plugin manifests, Script Hooks and the Run trace (S1).
- **Scope.**
  - `labels` takes typed handles (ADR 0082). A Hook scoped to one Label sees its Payload typed, and to several, their union. Python's types collapse a mixed list to `object`, and Rust's `HookOn::only` gives a JSON value.
  - A Label another crawl declared fails at once with `label_unknown`, and a Dataset another crawl declared with `dataset_unknown` (ADR 0103).
  - With no `labels`, a Hook runs for every Label and sees the Payload as `unknown`.
  - The stage before a Record's commit also takes `datasets`, ADR 0103's Dataset handles, so its Hook sees and returns the Dataset's Record type (G3).
- **A Hook that can never run is refused (G3).** An empty `labels` list fails at once with `hook_ineffective`. So does, when the Run starts, a navigation Hook all of whose Labels force a tier with no browser through `fetch.tier`.
- **Order.** Hooks run per stage in the order declared, scoped and unscoped alike. There are no integers and no priorities.
  - Hooks named in settings (process Plugins and Script Hooks under `hooks.*`, whose schema is area 7's spec) run first, layer by layer from Operator policy to the run layer.
  - The crawl's Hooks in code run last, so the program's own Hooks see what the others changed, as the nearest scope wins in ADR 0083.
  - A Hook's change is no lock: a later Hook may undo an earlier one's header. A value an operator must enforce is a locked setting (ADR 0028).
- **Name.** `name` defaults to the function's own name where the language has one, else to the stage and its position (`before_request#2`), which a Python `<lambda>` and a Rust closure also get. A second Hook of one name on one crawl fails at once with `hook_duplicate`, so a name identifies one Hook: in `hook_failed`, in the Run trace (G6), and for a test's `runHook` ([Testing a user's crawl or automation](https://github.com/goodfetch/automan/issues/482)).

### What a Hook receives and returns

Every context carries `request` (id, URL, Label, typed Payload, depth, attempt) and `run`, for its Key-value store (ADR 0103), which Script Hooks also have (ADR 0051).

| Stage | Also in the context | Returns | Ends the work with |
|---|---|---|---|
| before request | the tier about to fetch, the headers as built so far, `previousError` | `{ url?, headers? }`, or nothing | `skip(message)`: `skipped`, reason `hook` |
| after response | the final `response`, after redirects | `{ body? }`, or nothing | `block(cause)`: ends the attempt `blocked` with that Block cause; ADR 0021 escalates by new Session, and the request ends `blocked` once escalation runs out. `retry(message)`: `retry_requested` (ADR 0093) |
| before navigation, after navigation, page ready | `page`, Collie's page (ADR 0085) | nothing | page calls only |
| before a Record's commit | `dataset`, `record` | the Record to commit, changed or not, or nothing | `drop(reason)`: counted, never stored; `reject(reason)`: into the Dataset's rejection table, read with `run.rejectedRecords` (ADR 0103) |

- **Why a returned change.**
  - The type says exactly what each stage may change (G3).
  - A Hook stays a plain function that a test calls and checks.
  - Nothing half-changed crosses the connection.
  - Rust returns a value rather than holding a mutable borrow across an await.

  The calls that end the work never return, as a handler's `skip` and `retry` (ADR 0093). They are typed `never` in TypeScript and `NoReturn` in Python; Rust returns them as `Err`.
- **Hooks run in turn,** each seeing what those before it changed.
- **After response runs on every final response,** before ADR 0093's status rule. A soft block served with a 200 is marked there, and a 404 still fails unless `crawl.handle_statuses` lets it through.
- **Page stages follow the handler's tier rules (ADR 0093).** A call only a browser can serve escalates under `fetch.tier: auto`, or fails with `browser_required` under a forced tier.
- **At the Record stage, an answer never fails the request** (ADR 0103). A changed Record is checked against the Dataset's schema again, and rejected if it no longer fits.

### What a before-request Hook may change

- **The URL and headers, never the method or a body.** A crawl request is a GET (ADR 0093). A crawl that must POST does it from a handler with `page.request` (ADR 0085). Politeness counts that request (ADR 0086), and [Effects a program declares in code](https://github.com/goodfetch/automan/issues/500) decides whether it is an Effect. A Hook that turned a GET into a POST would change what the Frontier had deduplicated, what robots.txt and Politeness read, and whether the request is an Effect (ADR 0010), unseen (G5).
- **A header set to `null` is removed.**
- **The engine's own headers are refused.** ADR 0033's forbidden headers (`Host`, `Cookie`, `Authorization`, `Proxy-*`, method overrides) fail the Hook with `header_forbidden`, wrapped in `hook_failed`.
- **Secrets.** A header's value may be a Secret placeholder, bound by `secrets.*` and checked against the request's URL, as in `page.request` and a route's overrides (ADR 0092). The engine adds cookies, Secret values and the Session's credentials after the last Hook, so no Secret value passes through the program, and the headers a Hook reads never hold them.
- **A rewritten URL is checked against the crawl's scope and the run's domain policy,** whatever form the Hook takes. A rewrite that leaves them ends the request `skipped` (`out_of_scope`, `domain_policy`, ADR 0093). A Plugin's or a Script Hook's rewrite must also stay within the domains it declares (ADR 0033). An enqueue's other checks (`duplicate`, `max_depth`, `max_requests`) do not apply, since the request was already admitted. robots.txt and Site preferences read the URL as rewritten, so a rewrite cannot step around them (G5).

### Running and failing

- **Where a Hook in code runs** is ADR 0081's:
  - TypeScript: the program's event loop;
  - Python: the Run's task group (async), or the connection's thread pool (sync);
  - Rust: inside `crawl.serve(&run)`.

  It is the program's own code (ADR 0033): unconfined, accounted, and in the Run trace by name. Records it changed say so in their Provenance.
- **Every Hook in code is a round trip.** Each request of its Labels waits on the program, as under a `**/*` route (ADR 0085). A request is leased to the program from its first Hook in code to its commit, so `crawl.handler.concurrency`, which counts those leases (ADR 0081), bounds Hooks and handlers together. A Script Hook named in settings runs in the engine's Script sandbox with no round trip to the program (ADR 0051), and is the way down for a crawl that cannot afford one (G8).
- **Detaching.** A Run with a Hook in code cannot be detached (ADR 0078), as a Run with handlers cannot; nor can a Run whose settings choose an SDK handler, which lives in the program.
- **Timeout.** `timeout.hook` (10 s, ADR 0080) bounds each call. Past it, the call fails with `deadline_exceeded` naming that setting and the stage, and is retried as any `deadline_exceeded` is. A Label may set `timeout.hook` for the Hooks on its requests.
- **`hook_failed`.** A Hook that raises fails the work with `hook_failed`, a `ProgramError`. Its details are:
  - `stage`, `hook` (its name) and `label`;
  - the exception's type, message and stack, as `handler_failed` carries a handler's (ADR 0079), and, like it, always named in its message;
  - `wrappedCode`, the Goodfetch code raised inside, or `null` for the program's own exception.
- **Retries.**
  - The program's own exception is not retried unless `crawl.retry.handler_failed` is on. A bug in a Hook fails the same way on every attempt, and each attempt fetches the page again (G5).
  - A wrapped code is retried when its retryable flag says so (ADRs 0051, 0079).
- **A Hook that raises at the Record stage fails the attempt's commit.** Everything the attempt staged is discarded together, so no attempt half-commits (ADR 0003).

### What a program may replace

The research's injectable components map onto Plugin kinds (ADR 0033), or onto none:
- **Fetcher:** the Fetch backend kind, which a program may implement (below).
- **Parser: no component is needed.** Fetching is already decoupled from parsing:
  - the tier is a setting or learned, never a crawler class (ADR 0093);
  - the handler reads `ctx.page`, Collie's page on every tier, or parses the response's body with any library it imports, since it is the program's own code.

  crawlee-python documents custom parsers for the same reason (crawl frameworks §6.9). The Extractor kind replaces main-content extraction (ADR 0007), a different job.
- **Storage: declined,** as ADR 0033 declined storage backends: a replaceable store breaks portable, resumable runs (ADR 0002). Delivering Records anywhere is a Sink's job (ADR 0103), and a program may write one (below).
- **Session pool: declined.** ADR 0021's escalation by new Session, ADR 0053's shared Politeness and ADR 0035's stealth meet in the pool, and a replaced pool would step around all three (G5). Crawlee v4 accepts any `ISessionPool`. Goodfetch's pool is shaped by settings instead (`session.*`, `proxy.*`, ADR 0092), and a Proxy provider supplies its proxies.
- **Proxy:** the Proxy provider kind, one proxy per Session (ADRs 0021, 0092).
- **Tier predictor: declined.** Crawlee v4 accepts any `IRenderingTypePredictor`. ADR 0020 learns the tier by comparing a sample's results fetched both ways. A predictor in the program would put a round trip before every fetch with no result to check it against. `fetch.tier` per Label is the way down (ADR 0093, G8).

### The program's own Plugins: SDK handlers

- **Registered on `connect()`,** in one option per kind, each a map keyed by name, as ADR 0101 registers `modelProviders`. Every kind has one, except the Hook (declared on the `Crawl`, above), the Model provider (`modelProviders`, ADR 0101) and the Secret provider (`resolveSecret`, ADR 0092).
  - **TypeScript:** `fetchBackends`, `proxyProviders`, `sinks`, and so on for each kind, generated from the catalog's kinds.
  - **Python:** the same in snake_case: coroutine functions on the async client, run in the connection's task group; plain functions on the sync client, run on its thread pool (ADR 0081).
  - **Rust:** `ConnectOptions` declares each handler's name, one builder method per kind (`fetch_backend(name)`, `proxy_provider(name)`, `sink(name)`), and `gf.fetch_requests(name)`, `gf.proxy_requests(name)` and `gf.sink_requests(name)` read its calls as an `EventStream`, since the async API spawns nothing (ADR 0081). Reading a name not declared fails with `setting_invalid`. A call waits for its stream until its own deadline. A call dropped unanswered, or held by a stream dropped mid-Run, fails as an unreachable implementation of its kind would (ADR 0101).
  - **In every language, a program's own implementation of a kind is this SDK-handler form, with every engine.** On a linked engine (ADR 0078), the Rust-trait form of ADR 0033 is the engine's seam for its built-ins, not part of the SDK API, so a Rust program has one form, as the others have (S1, S2).

  A map keyed by name makes a duplicate name impossible. A handler serves every Run of its connection, as an installed Plugin serves every Run of its engine.
- **Chosen by name.** Settings choose a handler by its name wherever they choose an installed Plugin (ADR 0028's layers).
  - **The only handler of a kind that one setting chooses** (Fetch backend, Proxy provider, Browser backend, Extractor, Search provider, Challenge solver) stands in for that setting's built-in default on its connection. Any explicit value, at any layer or scope, replaces it, and `explainSettings()` shows it in the default's place, marked `registered` (G3). Without this, a proxy pool registered and never chosen would leave a crawl on the user's own address (G5).
  - Two handlers of such a kind are both available by name, and neither stands in for the default.
  - **Kinds that another setting or a reference names** take the handler's name there:
    - a Model reference `inhouse/<model>` (ADR 0101), and a Decision provider as a Model provider is;
    - a Secret reference `caller:` (ADR 0092), and a Code source in `secrets.<name>.codeSource` (ADR 0092);
    - a Notification channel where its settings name channels (ADR 0027);
    - a Sink through its `sinks.<name>` entry (below).

    Area 7's spec writes each kind's choosing key under ADR 0083's naming rule.
  - **A handler no setting chooses yet is not refused.** The connection's later Runs may choose it, as a Model provider no reference names yet may be (ADR 0101).
- **Names that clash or that nothing answers fail with `setting_invalid`, its `fix` naming both ways to supply one: install a Plugin, or register a handler.**
  - At `connect()`: a handler named as an installed Plugin of its kind.
  - When a scope opens: a setting naming a handler that is neither installed nor registered.
  - At a resume, before any work: a handler the Run used that the program no longer registers. Run state records the handler's name and the form `sdk_handler` in place of a digest (ADR 0033).
- **Failures.**
  - A `GoodfetchError` a handler raises keeps its code.
  - Any other exception is its kind's `<kind>_handler_failed`, a `ProgramError` carrying the handler's name and the exception's type, message and stack. Like `handler_failed`, its message always names that exception (ADR 0079), as ADR 0101's `model_handler_failed` does. `<kind>` is the kind's short name in the catalog: `fetch`, `proxy`, `sink`, and ADR 0101's `model`.
  - In Rust, `fail(error)` takes any error, a `goodfetch::Error` keeping its code and any other becoming `<kind>_handler_failed`, so a Rust handler's own failure is as diagnosable as a TypeScript one's (G6). ADR 0101's draft fails a Rust model call with a `goodfetch::Error` only; [Models, usage and cost in the SDK API](https://github.com/goodfetch/automan/issues/475) is asked to align.
  - ADR 0092's `resolveSecret` keeps `secret_unavailable`, as decided there.
- **Retries of a handler's failure.**
  - A Sink's is retried on ADR 0008's schedule, since its delivery ID makes a retry safe.
  - In a crawl, a crawl request that failed through a handler (its fetch, or the Session it needed) is retried only when `crawl.retry.handler_failed` is on for its own Label.
  - In a Collie Run, the call that needed the handler fails, and nothing retries it (ADR 0079).
- **A program's Fetch backend never carries a logged-in Session's requests,** as a third-party one never does (ADR 0033). The engine sends those itself, so no cookie or Secret value passes through the program (ADR 0092).
- **A program's Sink** (ADR 0026's SDK-handler form) serves the `sinks.<name>` entry of its name. That entry gives its Datasets, write mode and key as any Sink's does (ADR 0103), and a `type` beside a handler of that name is refused with `setting_invalid` when the Run starts. Each delivery carries ADR 0008's delivery ID and run epoch, and its Guarantee class is probed as any Sink's (ADR 0026). `run.records()` stays the reader with no delivery state (ADR 0103).
- **Rights.** A handler runs with the program's full rights, never confined, and a third-party package used as one does too, as ADR 0033 tells the docs to say.

### Error codes

| Group | Code | Retryable |
|---|---|---|
| `ProgramError` | `hook_failed` (its details above) | as its wrapped code; for the program's own exception, only under `crawl.retry.handler_failed` |
| `ProgramError` | `header_forbidden`, raised inside `hook_failed`; `hook_ineffective`; `hook_duplicate` | no |
| `ProgramError` | `fetch_handler_failed`, `proxy_handler_failed` | only under `crawl.retry.handler_failed`, in a crawl |
| `ProgramError` | `sink_handler_failed` | yes, on ADR 0008's schedule |
| `ProgramError` | the other kinds' `<kind>_handler_failed`, which area 7's spec names | no |

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
- **A before-request Hook that sets the method and a body,** as Scrapy's `FormRequest` and Crawlee's `method` and `payload` allow: rejected by G5. It would change the request unseen by deduplication, robots.txt and the Effect classing; a handler POSTs with `page.request`.
- **Retrying a Hook's own exception by default,** as Crawlee retries an error in a navigation hook: rejected by G5. **Never retrying it, with no switch:** rejected. A transient failure in the program's own service would fail its request for good, where the handler's switch already exists.
- **A replaced Session pool, a storage client and a tier predictor,** as Crawlee v4 accepts: rejected, above.
- **One generic `connect({ plugins: [...] })` list for every kind:** rejected. It was this ticket's first answer, replaced once ADR 0101 registered Model providers per kind: two forms for one concept break S1. **Rust traits run on the connection's runtime:** rejected for the same reason, beside ADR 0092's and ADR 0101's streams, and so is keeping the Rust-trait form public on a linked engine.
- **Rust handlers registered while their stream is open,** as ADR 0101's draft does: rejected. The connection's choice would change after its scope opened, against ADR 0083's "no setters" (G2), and `connect()` could not refuse a clash.
- **The registered handler as the connection's explicit value:** rejected by G3. It would beat `connect()`'s own `settings` naming an installed Plugin.
- **The kind's own failure code for an SDK handler's exception** (a Sink's unavailability, say): rejected. A program's exception is the program's to fix, a `ProgramError` by remedy (ADR 0079), as `handler_failed`, `hook_failed` and `model_handler_failed` already are.
- **Refusing a handler no setting chooses:** rejected. A connection serves many Runs, and a later one may choose it.
- **A service locator** (Crawlee for Python's): rejected by G2.

## Consequences

- **Amends ADR 0033.**
  - In the SDK API, Hooks in code are a crawl's, at the stages that change the work. Collie's page stages have no Hook in code. The observe-only stages reach a program as the Run's values.
  - A program's own implementation of a kind is the SDK-handler form with every engine; the Rust-trait form is the engine's seam for its built-ins.
  - Before request no longer changes the body, and every form's rewrite is checked against the crawl's scope.
  - The new observe-only stage `request_ended` is added.
  - A Hook before a Record's commit may be scoped by Dataset.
  - Hooks named in settings run before those in code, layer by layer.
  - A Hook's own exception follows `crawl.retry.handler_failed`.
  - SDK handlers are registered by name, one option per kind, and fail as `<kind>_handler_failed`.
- **Amends ADR 0079.** `hook_failed` names its stage, Hook and Label, and its wrapped code. An SDK handler's exception is its kind's `<kind>_handler_failed`, whose message, like `handler_failed`'s, always names the program's exception, in Rust's `Display` too.
- **Amends ADR 0083.** The only SDK handler of a kind that one setting chooses stands in for that setting's built-in default, marked `registered` in `explainSettings()`.
- **Amends ADR 0093** (its notes are added once it merges):
  - `crawl.retry.handler_failed` also covers `hook_failed` and a crawl request failed through an SDK handler;
  - headers and URL rewrites, not bodies, belong to a before-request Hook;
  - the scope is checked on a Hook's rewrite too, and `out_of_scope` covers a rewrite leaving it;
  - a Label takes `timeout.hook`;
  - Hooks are declared on the `Crawl`.
- **Amends ADR 0026.** A program's Sink is registered on its connection by name, serves the `sinks.<name>` entry of its name, and is the SDK-handler form in Rust too.
- **`GLOSSARY.md`.**
  - **SDK handler** is added.
  - **Hook** gains _Avoid_: pipeline (for a Hook before a Record's commit). Scrapy's item pipelines split here into that Hook and a Sink.
- **ADR 0037's matrix.**
  - RT-04 becomes `decided`.
  - RT-07, RT-08, RT-14, RT-21, SA-02, SA-17, SA-20 and CO-05 cite this ADR.
  - Areas 3, 6, 7 and 11 list it.
- **Areas.**
  - Area 7 (Plugin protocol) builds the stage names, the `hooks.*` schema, `request_ended`, each kind's generated handler, short name and `<kind>_handler_failed`, and each kind's choosing key under ADR 0083's naming rule.
  - Area 3 (Engine runtime) builds the order across forms, Hook timeouts and retries, a failed commit at the Record stage, and the `registered` default.
  - Area 6 (Collection) builds the scope check of a rewritten URL and an SDK-handler Sink's deliveries.
  - Area 11 (Client surfaces) builds the Hook methods, their contexts, the handler options and Rust's declared names and streams.
- **`TESTING.md`** gains the Hook conformance cases.
- **The `prototype/sdk-api` sketch** holds the `hooks` files in each language and snippets on golden scenario 05's crawl. The pain coverage matrix closes P46 and K12 as API.
- **Left to other tickets.**
  - [The Run in code](https://github.com/goodfetch/automan/issues/478): a discrete event for an observe-only stage its progress and report do not already show, such as Limit reached or migration.
  - [Models, usage and cost in the SDK API](https://github.com/goodfetch/automan/issues/475): Rust's declared names and `fail(error)` taking any error, for `model_requests`.
  - [Testing a user's crawl or automation](https://github.com/goodfetch/automan/issues/482): building a Hook's context to call a Hook in a test.
  - [API sketch: type-checked stubs and golden scenarios](https://github.com/goodfetch/automan/issues/483): placing a before-request Hook in golden scenario 05 or 08.
