# A crawl declares its Labels and one handler each, and a handler reads Collie's page on every fetch tier

A program defines a crawl by declaring its Labels, each with exactly one handler, on a value that needs no connection; a Run then carries the crawl out. Every handler receives the same context on every fetch tier: Collie's page over the crawl request's document, which the engine serves without a browser on the non-browser tiers and moves to the browser only when a call needs one. A crawl request ends in one of ADR 0079's outcomes, and a handler skips or retries it with a call. Competitors fail here in three ways:
- **Switching tiers is a rewrite.** Crawlee's context changes shape with its crawler class, Scrapy reaches a browser through a plugin, a reactor and per-request meta, and Spider-rs through a recompile ([crawl frameworks](https://github.com/goodfetch/automan/blob/research/consumer-api/rnd/research/research_notes/Consumer%20API%20of%20competitors/crawl-frameworks.md) §3.2, §6.5; [synthesis](https://github.com/goodfetch/automan/blob/research/consumer-api/rnd/research/2026-10-14-consumer-api.md) pain 8).
- **Skipping is faked as a failure.** Crawlee v3 skipped a request by setting `noRetry`, throwing, and lowering `requestsFailed` by hand (§6.15).
- **Retry defaults disagree.** Crawlee 3.18.2 retries any handler error three times, and Scrapy 2.19.0 never retries a callback's exception, both read in their source for this decision.

Decided in [Crawl definition and handler context](https://github.com/goodfetch/automan/issues/477), on the map [The SDK API of Collie and Foxhound](https://github.com/goodfetch/automan/issues/461), every answer decided by the agent on the owner's delegation, with its [grilling record](https://github.com/goodfetch/automan/issues/477#issuecomment-6181133439), [round 7](https://github.com/goodfetch/automan/issues/477#issuecomment-6181341593) and [round 8](https://github.com/goodfetch/automan/issues/477#issuecomment-6181741958). Principles are cited by their IDs in [ADR 0077](0077-the-sdk-api-follows-eight-guarantees-and-four-ranked-shape-rules.md). The page is [ADR 0085](https://github.com/goodfetch/automan/pull/501)'s and Sessions are [ADR 0092](https://github.com/goodfetch/automan/pull/509)'s, both in review when this was decided.

## Decision

### Declaring a crawl

- **A Label for each kind of request, with exactly one handler.** Behind JSON-RPC a callback cannot cross the connection or a Checkpoint, so a request names its handler by a name the engine persists: its Label (ADR 0082). There is no default handler: Crawlee's catches a misspelled label silently, which typed handles make a type error instead (S2, G3).
- **A `Crawl` value holds the declaration,** made with no connection and no I/O, so a module may declare it when it loads (ADR 0078):
  - `new Crawl()` in TypeScript;
  - `AsyncCrawl()` (`async def` handlers) and `Crawl()` (plain functions) in Python, as the client pair;
  - `Crawl::new()` in Rust, and `goodfetch::blocking::Crawl` for `Fn + Send + Sync + 'static` handlers (ADR 0081).

  A Run is started from it and resumed with it, since handlers are never persisted.
- **Two steps per Label: declare it, then attach its handler.**
  - `crawl.label(name, { payload, settings, login })` returns the typed handle `Label<P>`.
  - `crawl.handle(label, handler)` attaches the handler. Python spells it as the decorator `@crawl.handle(label)`, which returns the function unchanged, so a test calls it as it is ([Testing a user's crawl or automation](https://github.com/goodfetch/automan/issues/482)).
  - **A login step** (ADRs 0006, 0092) is the `login` option of a Label's declaration (`login=` in Python), or `login` of `new Crawl({ login })` (`AsyncCrawl(login=...)`) for the whole crawl; a Label's own wins for its Sessions. Rust spells them `crawl.login_for(&label, …)` and `crawl.login(…)`.
  - **Why not one step.** One step was tried with tsc 7.0.2: an expression-bodied handler that enqueues to its own Label fails with TS7022, a trap a coding agent writes for pagination, and Rust cannot name a binding inside its own initializer (S1, T).
  - **Rust** declares `crawl.label::<P>(name)`, with the schema `schemars` derives, or `crawl.label_with(name, schema, settings)`.
- **Mistakes are typed and early (G3, G6).**
  - A second Label of one name, or a second handler for one Label, fails at once with `label_duplicate`.
  - A Label left without a handler fails the Run's start with `label_unhandled`.
  - A Label the crawl does not declare fails an enqueue with `label_unknown`. A resume whose pending requests name a Label no longer declared is refused with `label_unknown` before any work, beside ADR 0082's schema check.
- **Where handlers run** follows ADR 0081: on the event loop in TypeScript, on the connection's thread pool in Python's sync API, and, in its async API, as tasks in the task group of the Run started from the `Crawl`, since the `Crawl` itself is no value to close. Closing the Run cancels and awaits them. In Rust, `crawl.serve(&run).await` answers the Run's crawl requests with the crawl's handlers inside that future until the Run ends, so handlers may borrow local state and need neither `Send` nor `'static`. Dropping that future stops answering: its leased requests are re-run, each counting a `handler_lost` attempt (below). `run.crawl_requests()` is the `EventStream` beneath it, for a program that answers requests itself (G8).

### Seeds and the noun

- **A crawl request** is the noun ADR 0079 left open: one URL a crawl's Frontier admitted, with its Label and Payload, the work item one handler processes (ADR 0003), which ends in one outcome. "Request" is every crawler's word, and prose may keep it. The SDK already has a `Request` type, the page's network request (ADR 0085), so the types are `CrawlRequest<P>` and `CrawlRequestOutcome`, which replaces the placeholder `RequestOutcome` (S3).
- **Seeds are crawl requests:** a URL, a Label and a Payload, checked as an enqueue is, so a seed the run's domain policy refuses is not added (`domain_policy`).
  - **The start verb.** Its spelling, and resuming, are [The Run in code](https://github.com/goodfetch/automan/issues/478)'s. The sketch uses `gf.foxhound.start(crawl, { seeds: [seed(url, home)] })` provisionally.
  - **`run.addSeed(url, label, payload?, { priority, forefront, key })`** adds a seed to a running crawl, committed at once (ADR 0037's CO-03), and answers as an enqueue does. The start reports its refused seeds as The Run in code decides. `ctx.run` is bound to its attempt, so `addSeed` called through it inside a handler is staged with the attempt, as an enqueue is.
  - **Seeds define the scope,** so a seed widens it by its host, while a handler's `enqueue` is checked against it. The two get two names because they mean two things, and widening the scope from a handler is the explicit call (S3, G5).

### What a handler receives

- **One `CrawlContext` on every fetch tier** (S2; the pain coverage matrix's K03):
  - `request`: id, the URL the Frontier admitted, the request's own `label` (the handle for pagination), the `payload` typed by its Label's schema, `depth` and `attempt`;
  - `page`: a Collie `Page` (ADR 0085), whose `url` is the final URL and whose `session()` reaches cookies (ADR 0092);
  - `response`: the main document's final `Response`, after redirects, with its status, headers, redirect chain and a body always kept;
  - `tier`: the fetch tier of this attempt;
  - `run`: the crawl's Run, through which [Crawl output in code](https://github.com/goodfetch/automan/issues/479) reaches Datasets and the Key-value store;
  - `enqueue`, `enqueueLinks`, `skip` and `retry`.
- **No second way to reach the same thing** (S2, G4): no `log` (the program's own logger), no `session` (`page.session()`), no `sendRequest` (`page.request`, ADR 0085), no selector API beside the page (Parsel- or Cheerio-like), and no `extendTimeout()` (a Label's `timeout.handler`).
- **A context belongs to one attempt.** An attempt ends at `skip`, at `retry`, at an escalation, at the handler's timeout, at a lost lease, or when the program closes the Run. From then on, every call on its context fails with `attempt_ended`, naming why, and what the attempt staged is discarded (ADR 0003). Catching it changes nothing, since the decision stands. In Python its class derives from `BaseException`, as `asyncio.CancelledError` does, so `except Exception:` lets it through; it still carries its code and group.

### Fetch tiers, per Label, and escalation

- **Collie's page on every tier.** ADR 0020 compares a sample's results fetched both ways, so one handler must run unchanged on every tier. Three tiers are non-browser and one is the browser:
  - `http` and `browser_fetch` fetch the document without rendering it (`browser_fetch` sends the request from inside the Session's browser, ADR 0020); the engine serves `page` from that document: locators and reads answer from it, and so do reading AI verbs;
  - `discovered_api` fetches the document over HTTP as `http` does, and serves the page's network waits (`waitForResponse`, `waitForRequestFinished`) from the JSON calls it replays;
  - `browser` renders the page.

  `page.request` goes through the Session's own HTTP transport on every tier and never escalates.
- **What only a browser can serve escalates.** Such calls include a gesture, `evaluate`, a network wait no replayed call answers, a screenshot, an acting AI verb or `page.lend()`. Under `fetch.tier: auto`, the first one ends the attempt and runs the request again in the browser. That run is neither a retry nor a failure, and it teaches the tier choice.
- **A static document never changes,** so a wait for something it lacks, or a read of one element that would wait for it, escalates too. Waiting for content is thus how a handler states what its page must hold. A JavaScript page fetched over HTTP whose content is missing escalates instead of passing as an empty success. A question (`count()`, `isVisible()`) answers at once and never escalates.
- **Under a forced tier, nothing escalates:** the call fails with `browser_required`, a `ProgramError` naming the Label's `fetch.tier`.
- **Which calls each tier serves exactly** is area 6's spec, under one rule: a call is served only when its result is exact; otherwise it escalates.
- **The unit of the tier choice.** ADR 0020's "route" is a **Label on one domain** (ADR 0004's key), which area 6's spec may split by URL path where a Label's samples disagree; the name avoids a clash with `page.route`. The setting `fetch.tier` takes `auto` (the default) or forces `http`, `discovered_api`, `browser_fetch` or `browser` at any scope down to a Label. There is no per-request override: requests needing another tier are another kind of request, so another Label, which may share a handler (S2). Provenance records the tier and whether it was forced (ADR 0023).
- **A rename.** ADR 0040's `tier_race` joins the family as `fetch.tier_race` (ADR 0083's naming rule).

### Collie inside a handler

- **`ctx.page` is a Collie Page** with its AI verbs, whose shapes are [Collie's AI verbs and Plans](https://github.com/goodfetch/automan/issues/473)'s. Each call is bound to the work item's lease (ADR 0003), journaled (ADR 0010) and counted in the crawl's own Run (Usage, Limits). There is no Collie Run inside a crawl (ADR 0078).
- **What a crawl learns per Label,** such as the tier choice or a Plan, is keyed by the Label on a domain.
- **Lending.** `ctx.page.lend()` ([Third-party automation on Foxhound pages, and Collie inside other crawlers](https://github.com/goodfetch/automan/issues/456)) is bound to the lease.
- **Model calls of a discarded attempt** still count in Usage: they were made.

### A handler's own navigation and Site preferences

- **A handler's navigation that Site preferences refuse** fails before it is sent, with `site_preferences_refused`, a `RefusedError` whose details name the URL and the reason (`robots_txt` or `crawl_delay`), and whose `fix` names the setting. That navigation is a `goto`, a click or an acting AI verb that navigates, or a `page.request` (ADR 0086). AI-use signals concern results, never fetching, so they refuse no navigation.
- **If it escapes the handler,** the request ends `skipped` with that same reason, not `failed`, since the site asked not to be crawled there. A handler that catches it carries on (G1, G5).

### How a request ends

- **`ctx.skip(message)`** ends the request `skipped`, with the reason `handler` and the message.
- **`ctx.retry(message)`** ends the attempt with `retry_requested`, a `PageError` that is retryable, counted against `crawl.retry.max`. When retries run out, the request ends `failed` with that error.
- **Both unwind the handler.** TypeScript types them `never` and Python `NoReturn`. A Rust handler returns `Err(ctx.skip(…))`, since its handlers return `Result<(), goodfetch::Error>`. Skipping is a call, not a thrown failure the program fakes (G1).
- **Retries.**
  - **`crawl.retry.max` is 2,** so three attempts, as Scrapy's `RETRY_TIMES`, the gentler of the two leaders (G5).
  - **What is retried:** an attempt whose error is retryable, such as a navigation timeout raised inside the handler (a `GoodfetchError` keeps its code and retryable flag, ADR 0079), and a handler past `timeout.handler` (ADR 0080).
  - **`handler_failed` is not retried** unless `crawl.retry.handler_failed` is on. A bug fails the same way every time, and each retry fetches the page again (G5).
  - **A lost lease.** An attempt whose lease ends because the program stopped answering (its process ended, its connection dropped, or Rust's `serve` future was dropped) ends with `handler_lost`, an `UnavailableError` that is retryable, and counts against `crawl.retry.max`, so a request that crashes the program does not re-run forever. An attempt the program's own `close()` stops counts nothing: the request returns to pending in the Checkpoint (ADR 0078).
  - **Not retries:** escalations and sample-comparison runs. A block's escalation by new Session follows ADR 0021.
- **Which responses reach a handler**, as Scrapy's `HttpErrorMiddleware` decides it:
  - a 2xx after redirects reaches the handler;
  - a Block cause ends `blocked` (ADRs 0021, 0045, 0048);
  - a 5xx or a 408 is retried, and fails as `response_unavailable` (an `UnavailableError`, retryable: the site is failing for now) when retries run out; its 502, 503 and 504 also back off the domain (ADR 0086);
  - any other 4xx fails at once with `response_not_ok` (a `PageError`, not retryable);
  - `crawl.handle_statuses` (none by default, settable per Label) lists other statuses whose responses go to the handler, so a link checker reads its 404s, and the error's `fix` names it.
- **The skip reasons** (ADR 0079's full list):
  - `out_of_scope`: a redirect leaving the crawl's scope;
  - `domain_policy`: a redirect or a Hook's rewrite to a host the run's domain policy refuses (ADRs 0014, 0017);
  - `duplicate`: a pending request merged by a URL rule learned after its admission (ADR 0005);
  - `robots_txt`, `crawl_delay` and `use_signals` (ADR 0086);
  - `hook`: a before-request Hook (ADR 0033);
  - `handler`: `ctx.skip()`.

  The `skipped` outcome carries `message`, the handler's or the Hook's own text. Each outcome names its request (id, URL, Label, depth); how a program reads outcomes is The Run in code's.

### Enqueueing, scope and the Frontier

- **`enqueue(url, label, payload?, { priority, forefront, key })`** adds one URL. In TypeScript and Python its Payload argument exists only when the Label has one, and Rust passes `()`, so a missing or wrong Payload is a type error.
- **`enqueueLinks(locator, label, payload?, { include, exclude, limit, priority })`** adds every link a Collie locator matches, each `href` resolved against the document's base URL. The locator is the page API's one way to target elements (S2). Two methods, because they return two types.
- **What they return.**
  - An enqueue's result is a value (ADR 0079): added, with its id, or not added, with `duplicate`, `out_of_scope`, `domain_policy`, `max_depth`, `max_requests` or `unsupported_scheme`.
  - `enqueueLinks` returns the added and not-added URLs, and a matched element with no link is reported as `no_link`.
  - So "enqueued nothing" says why (G1). A Payload failing its schema still raises `payload_invalid` (ADR 0082).
- **Staging.** A handler's enqueues are outputs staged with its attempt and committed with it (ADR 0003), so a failed or escalated attempt leaves no stray links.
- **The Frontier.** It deduplicates by URL after its rules, whatever the Label (ADR 0005). `key` replaces that identity, a declared canonicalization that always wins. `priority` and `forefront` are ADR 0005's.
- **Only GET.** Headers, bodies and URL rewrites belong to a before-request Hook (ADR 0033). A POST is `page.request`, and may be an Effect.
- **Scope is crawl-wide, checked on every enqueue and every redirect.**
  - `crawl.scope.strategy`: `same_hostname` by default (Crawlee's default; Scrapy allows every host when `allowed_domains` is unset), or `same_domain`, `same_origin` or `all`.
  - `crawl.scope.include` and `crawl.scope.exclude`: URL patterns matched in the engine (ADR 0081). Besides the strategy, a URL must match an `include` pattern when there are any, and no `exclude` pattern; they never replace the strategy, as Crawlee v4 learned.
  - **Measured against every seed,** with hosts compared after the Frontier's fixed URL rules, so `www.` is no other host (ADR 0005). `include` and `exclude` on `enqueueLinks` only narrow.
  - **Bounds.** `crawl.max_depth` (seeds at 0) and `crawl.max_requests` (requests admitted) have no default. Reaching either refuses further enqueues with its reason and ends the Run `incomplete` (ADR 0005).
  - **No transform function:** rewriting is a Hook's.

### Settings and the Label scope

- **A Label is a settings scope** between the Run and its requests. It takes:
  - every setting a page takes (ADRs 0083, 0085, 0091), such as `timeout.action`, `timeout.navigation`, `timeout.wait` and `locator.test_id_attribute`;
  - `timeout.handler`, `fetch.tier`, `crawl.retry.*`, `crawl.handle_statuses` and `crawl.handler.concurrency`;
  - every `session.*` key and `secrets.*` (ADR 0092), with a `login` step as an option of its declaration.
- **Sessions per Label.** A Label that sets what a Session holds (`session.source`, `session.http_credentials`, `secrets.*` or a `login` step) gets Sessions from a pool of its own; Labels that set none share the Run's pool. So one crawl mixes anonymous and logged-in pages, as Scrapling routes requests to declared sessions with `sid=`, without a second Run that would split its Limits and Completeness. `session.page_check` and `session.recovery.human_pause` judge the Label's pages in whichever pool serves them, so a Page check per Label splits nothing.
- **What a Label refuses**, with `setting_not_applicable`: the Session's identity (`browser.*`, `proxy.*`, `network.*`), which takes the Run's values in every pool, and Politeness, Site preferences, scope and Run-wide keys. The TypeScript and Python types leave them out, and Rust refuses them when the Run starts (G3).
- **Politeness per Label is declined:** two Labels on one domain share its slot and delay with every run of the user (ADR 0053). A per-domain override, as Scrapy's `DOWNLOAD_SLOTS`, would be keyed by domain, and none is added.
- **A page failing its `expectedPage`** (`session.page_check`) on a non-browser tier escalates under `fetch.tier: auto`, as a wait for missing content does. On the browser tier, or under a forced tier, it fails with `page_check_failed` (ADR 0006). A login page is a lost login on every tier.
- **`crawl.handler.concurrency` is 16 by default** (Scrapy's `CONCURRENT_REQUESTS`): the engine never leases more requests to the program at once (ADR 0081). Set on a Label, it also caps that Label's handlers, within the Run's.

| Key | Built-in | Scopes |
|---|---|---|
| `crawl.scope.strategy` | `same_hostname` | connection, Run |
| `crawl.scope.include`, `crawl.scope.exclude` | none | connection, Run |
| `crawl.max_depth`, `crawl.max_requests` | none | connection, Run |
| `crawl.retry.max` | 2 | down to a Label |
| `crawl.retry.handler_failed` | off | down to a Label |
| `crawl.handle_statuses` | none | down to a Label |
| `crawl.handler.concurrency` | 16 | down to a Label |
| `fetch.tier` | `auto` | down to a Label |
| `fetch.tier_race` (was `tier_race`) | off | connection, Run |

`fetch.*` is a crawl's: on a Collie Run it fails with `setting_not_applicable`, as `crawl.*` does, and the TypeScript and Python types leave it out.

### Error codes

| Group | Code | Retryable |
|---|---|---|
| `ProgramError` | `label_duplicate`, `label_unhandled`, `label_unknown`, `attempt_ended`, `browser_required` | no |
| `PageError` | `retry_requested` | yes |
| `PageError` | `response_not_ok` | no |
| `UnavailableError` | `response_unavailable`, `handler_lost` | yes |
| `RefusedError` | `site_preferences_refused` | no |

## Considered Options

- **Per-request callbacks** (Scrapy, Scrapling): rejected. A function cannot cross the connection or a Checkpoint, so it needs a registry of names, which a Label already is. **One handler that switches on the Label** (Spider-rs): rejected; every program writes its own router, with no Payload typed per kind.
- **A default handler** (Crawlee's `addDefaultHandler`): rejected. It catches unlabelled seeds and misspelled labels alike; seeds name their Label, and typed handles catch misspellings.
- **Declaring a Label and its handler in one call:** rejected for the TS7022 trap and Rust's ordering, above. **Handlers registered on the Run after it starts:** rejected; a request can arrive before its handler. **A crawl class to subclass** (Scrapy's `Spider`): rejected; inheritance as configuration, and a method name doubling as a persisted name.
- **A context whose shape changes with the tier** (classic Crawlee): rejected; it is the rewrite users complain of (§3.2). **A selector API beside the page:** rejected by S2. **A restricted context type** (Crawlee's adaptive crawler): rejected; a third shape to learn, when the page API already reads a static document.
- **Failing instead of escalating** when an HTTP-tier handler touches the browser: rejected while the engine chose the tier, since its guess is no bug of the program's; kept when the program forced the tier. **Loading the browser mid-handler and carrying on:** rejected; one attempt would read two documents.
- **A tier per request** (Scrapling's `sid=`, scrapy-playwright's meta): rejected by S2; a Label is a kind of request. **Tiers chosen at build time** (Spider-rs's Cargo features): rejected by ADR 0038's rule that a capability is a setting.
- **Retrying `handler_failed` by default** (Crawlee): rejected by G5. **Never retrying any handler error** (Scrapy): rejected; a retryable code raised inside, such as a navigation timeout, deserves the retry it signals. **`RetryRequestError` with no cap** (Crawlee): rejected; `ctx.retry()` counts against `crawl.retry.max`.
- **Skip and retry as return values:** rejected; a helper deep inside a handler could not use them. **A flag set, then a return** (Crawlee v3): rejected; forgetting either half is silent.
- **Every non-2xx reaching the handler:** rejected; a 404 would pass as a handled page unless every handler checked. **Retrying 4xx:** rejected by G5.
- **A selector string for `enqueueLinks`** (Crawlee): rejected by S2; the locator is the page API's one way to target elements. **Patterns replacing the scope strategy** (Crawlee v3): rejected; Crawlee v4 reversed it. **Scope relative to the current page** (Crawlee): rejected; a redirect to another host would carry the crawl there. **Scrapy's open default** (every host): rejected by G5.
- **Queue additions written through at once** (Crawlee v4's default): rejected for a handler's enqueues; staging them with the attempt keeps a failed attempt's links out, as ADR 0003 stages every output.
- **Politeness per Label:** rejected; it belongs to the site, and per-Label values would be ignored (G3) or a way to press a site harder (G5).
- **`Request` in a crawl namespace** for the noun: rejected; one name with two meanings in one SDK (S3). **Visit, task or work item:** rejected as unfamiliar.
- **Run-wide Sessions only:** rejected; a crawl mixing anonymous and logged-in pages would need two Runs, splitting its Limits, Usage and Completeness. **A Session named per enqueue** (Scrapling's `sid=`): rejected by S2; the Label already names the kind of request. **Every Session setting splitting the pool:** rejected; a Page check per Label would give each Label its own logins.
- **Failing a failed expected page on every tier:** rejected; a JavaScript page fetched over HTTP would fail where the browser passes.
- **One code, retryable or not by status:** rejected; ADR 0079 gives each code one flag.
- **A failed-request handler** (Crawlee's `failedRequestHandler`): rejected; a failed request is an outcome value (ADR 0079), and a callback would be a second channel for the same fact (S2). A Hook stage before a retry stays a Hook question.
- **A re-run after a lost lease that never counts:** rejected; a request that crashes the program would re-run forever. **One that always counts:** rejected; a clean stop would use up retries.

## Consequences

- **Amends ADR 0020.** Its route is a Label on one domain; a program forces a tier per Label with `fetch.tier`; a call only a browser serves, a wait for missing content and a failed `expectedPage` on a non-browser tier escalate the request; `tier_race` is `fetch.tier_race`.
- **Amends ADR 0040**, whose `tier_race` is `fetch.tier_race`.
- **Amends ADR 0006.** A page failing its expected page on a non-browser tier, under `fetch.tier: auto`, escalates to the browser instead of stopping the work item.
- **Amends ADR 0023**, whose per-run or per-call switch of the sample comparison is also set per Label: `fetch.tier` forces a tier at any scope down to a Label.
- **Amends ADR 0079.**
  - The noun is crawl request, and the type is `CrawlRequestOutcome`, which names its request.
  - The skip reasons are listed, and `skipped` carries a message.
  - `handler_failed` is not retried unless `crawl.retry.handler_failed` is on.
  - `skip()` and `retry()` end a handler's attempt.
  - In Python, `AttemptEndedError` derives from `BaseException` rather than `GoodfetchError`, the one class outside the hierarchy, as `asyncio.CancelledError` is outside `Exception`.
- **Amends ADR 0081.** Its Rust crawl helper is `crawl.serve(&run)`; its handler concurrency setting is `crawl.handler.concurrency`, settable per Label; and Python's async crawl handlers run in the task group of the Run started from the `Crawl`, which registers them but has no lifecycle.
- **Amends ADR 0082.** Labels are declared on a `Crawl`, and a resume also refuses pending requests of an undeclared Label with `label_unknown`.
- **Amends ADR 0083.** A Label is a settings scope, and its keys are those above. `fetch.*` is refused on a Collie Run.
- **Amends ADR 0086.**
  - In this ADR's terms, a crawl request is what the Frontier admits. A handler's own navigation, and a hedge's or `fetch.tier_race`'s second copy, are governed as crawl requests are.
  - A navigation that Site preferences refuse inside a handler raises `site_preferences_refused`.
  - Politeness is not settable per Label.
- **`GLOSSARY.md`** gains **Crawl request**.
- **Error codes.** The codes in the table join `goodfetch-protocol` with their groups and retryable flags, and `xtask codegen` generates one class per code under ADR 0079's naming rule.
- **ADR 0037's matrix.**
  - RT-07, CO-03, CO-05, CO-07 and SC-02 become `decided` by this ADR.
  - RT-04 keeps `spec`, since a stage before a retry is [Hooks and injectable components in the SDK API](https://github.com/goodfetch/automan/issues/481)'s; its note names the retry settings and the failed-request handler this ADR declines.
  - CO-08, CO-11, CO-15, CO-16 and V3-01 cite it.
  - Areas 3, 4, 5, 6 and 11 list it.
- **Areas.**
  - Area 6 (Collection) builds the Frontier's answers to enqueues, the scope, the bounds, which calls each tier serves on a static document, and the per-Label tier choice.
  - Area 3 (Engine runtime) builds attempts, staging of enqueues, retries, escalation, and the Label checks at start and resume.
  - Area 5 (Site access) builds Session pools per Label and the expected-page escalation.
  - Area 4 (Security and egress) builds the scope's host checks after redirects and the domain policy's refusal of seeds.
  - Area 11 (Client surfaces) builds the `Crawl` value, the context, the signals that end an attempt and the Rust helper in each language.
- **`TESTING.md`** gains the crawl conformance cases, including `handler_failed`'s.
- **The `prototype/sdk-api` sketch** holds the `crawl` files in each language and snippets on golden scenarios 05, 06 and 08. The pain coverage matrix closes P11, P49 and K03 as API, and Crawl's parts of P20 and P48.
- **Left to other tickets.**
  - The Run in code: the start and resume verbs, and reading outcomes.
  - Crawl output in code: pushing Records from `ctx.run`, staged with the attempt.
  - Hooks and injectable components in the SDK API: Hooks scoped by a `Label` handle, request headers and bodies, and a stage before a retry.
  - Testing a user's crawl or automation: building a `CrawlContext` from a saved response.
  - [Effects a program declares in code](https://github.com/goodfetch/automan/issues/500): a declared Effect must be checked rather than repeated, since a handler re-runs on a retry, an escalation, a sample comparison and a lost lease.
  - Once ADRs 0085 and 0092 are on `main`, the links to them become relative; if ADR 0092 merges first, it gains a note: `secrets.*` and `session.page_check` are also set on a Label.
  - Collie's AI verbs and Plans: Plans keyed by the Label on a domain.
