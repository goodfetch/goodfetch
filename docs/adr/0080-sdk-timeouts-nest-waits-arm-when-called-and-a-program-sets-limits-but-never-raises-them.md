# SDK timeouts nest in one duration type per language, waits arm when called, and a program sets Limits but never raises them

A program says how long things may take with one `timeout`, a duration in its language's own type, which nests: a call made inside another never outlasts it. Every phase has a finite default held as a setting, and a timeout's error names the setting that set it. App-state waits arm when they are called, so a response cannot slip past its waiter. Cancellation is each language's own (an `AbortSignal`, task cancellation, a dropped future) and always reaches the engine through `$/cancel`. Limits are set in code on a connection and a Run, and only lowered there. Timing is the research's first pain: 53 of the 375 top Playwright and Puppeteer Stack Overflow questions (1,686 votes) are waits, races and sleeps, and "element ready is not app ready" survives auto-waiting ([synthesis §4](https://github.com/goodfetch/automan/blob/research/consumer-api/rnd/research/2026-10-14-consumer-api.md#4-pain-points-merged-ranked-by-evidence-weight), pain 1). The others are close behind: Crawlee's silent hangs, which took years to fix (pain 6); Playwright Python's `timeout` in milliseconds where httpx and asyncio use seconds; Skyvern's minutes, seconds and milliseconds depending on the call; Python cancellation that Playwright calls undefined behaviour (pain 15); and agents whose only cap is a step count (AI pain 13). Decided in [Time, waits, deadlines, cancellation and budgets](https://github.com/goodfetch/automan/issues/467), on the map [The SDK API of Collie and Foxhound](https://github.com/goodfetch/automan/issues/461), under the principles of ADR 0077 and the error model of ADR 0079.

## Decision

### Timeouts

- **One concept, a duration that nests.** `timeout` is the longest a call, a wait or a phase of a Run may take, counted from its start: a click, a navigation, a crawl handler, a sink write. The engine turns it into an end time that the calls made inside inherit. An action started with 10 s left in its handler gets 10 s, not its own 30 s. A child may end sooner than its parent but never later, which is Azure's rule for cancellation trees. There is no `extendTimeout`: a handler that needs longer gets a longer timeout through its route's setting (S2, G2).
- **A Human pause stops the clock.** Time spent waiting in a Human pause does not count toward any enclosing timeout. How long a pause itself may last is [Sessions, logins, Human pause and secrets in code](https://github.com/goodfetch/automan/issues/476)'s.
- **One duration type per language (S1, G3):**
  - TypeScript: a `number` of milliseconds, the one unit throughout the TypeScript SDK, named `timeout` without a unit suffix (Playwright's name and `AbortSignal.timeout(ms)`'s unit, S3).
  - Python: `datetime.timedelta` only. A pasted Playwright `timeout=30_000` is then a type error, where a number would silently mean 30,000 seconds or 30 milliseconds.
  - Rust: `std::time::Duration`. Every call is a builder that implements `IntoFuture`: `.timeout(d)` or `.no_timeout()`, then `.await`.
- **Three states.**
  - Omitted, the setting's default applies. Python's omitted value is a `not_given` sentinel, as in openai-python.
  - An explicit null means no timeout: `null` in TypeScript, `None` in Python, `.no_timeout()` in Rust. An enclosing timeout still applies.
  - `0`, a negative value or a non-finite number fails with a `ProgramError` whose fix names the null form, because `0` reads as "at once".
- **Every phase has a finite default**, held as a setting (ADR 0028) that can be set where the Run, Session or page is opened, overridden per call and shown by `goodfetch config explain`:
  - connect: 30 s
  - action, the guard's window (ADR 0019): 30 s
  - navigation: 30 s
  - app-state wait: 30 s
  - crawl handler: 60 s
  - hook: 10 s
  - sink write: 30 s
  - `close()`: 30 s

  There is no mutable setter such as Playwright's `page.setDefaultTimeout`, which changes calls elsewhere in the program (G2). The setting names, and whether Foxhound's defaults differ from Collie's, are [Settings in code](https://github.com/goodfetch/automan/issues/470)'s.
- **A timeout's error** is in ADR 0079's `deadline_exceeded` group. It carries the timeout's value and the elapsed time, what was waited for, the last state observed, and in `limit` its source: the call's argument, or the setting with its layer. When an enclosing timeout ended the call, the error names that one (G6).
- **A timeout fails only its call.** In a crawl, it fails that work item, which is then retried under the crawl's retry policy ([Crawl definition and handler context](https://github.com/goodfetch/automan/issues/477)). The Run goes on. Only the Run's own timeout, a Limit or an abort stops a Run.
- **The Run's own timeout.** Starting a Run takes a `timeout` for the whole Run, with no default, because crawls rightly run for hours and Limits cap their cost. On expiry the Run stops with a Checkpoint, `stopped` and `incomplete` (ADR 0029), with the stop reason `timeout`, and it can be resumed; the resumed Run's clock starts afresh. It is not a Limit: elapsed time is not Usage, and "only a person raises it" has no meaning for a clock.

### Waits

- **App state, with Playwright's names where the meaning matches (S3).** The waits are:
  - `waitForURL`, `waitForResponse`, `waitForRequest`, `waitForLoadState`, `waitForFunction` and `locator.waitFor({ state })`, as in Playwright.
  - `waitForRequestFinished`, which waits until a matching request has finished and its body is read.
  - `locator.waitForCount(n | { min, max })`, which returns the count. It has a new name because no Playwright call outside `expect` does this.

  The `networkidle` load state is kept and defined: no request in flight for 500 ms, WebSockets and event streams excluded. Each wait takes `timeout`, and in TypeScript `signal`. Python spells them in `snake_case`. Where a host-language predicate runs is [Concurrency model per language](https://github.com/goodfetch/automan/issues/468)'s.
- **Waiters arm when called.** The SDK sends the arming request when the method is called, not when it is awaited, and the engine keeps each page's calls in order. So the response a click triggers can never arrive before its waiter. Each language writes it in its own form:
  - TypeScript, Playwright's promise-first form: `const r = page.waitForResponse(u); await click(); await r`. It is equally safe under `Promise.all`.
  - Python, Playwright's blocks in the async and sync APIs: `async with page.expect_response(u) as info:`, then `await info.value`. The same exists for `expect_request` and `expect_request_finished`.
  - Rust: `page.wait_for_response(u).during(action).await?` arms the waiter, then drives the action, and returns both results. Rust futures do nothing until polled, so a waiter merely joined with its trigger could arm too late. A waiter can also be awaited alone.
- **No fixed sleep.** The page has no sleep verb. A tombstone `waitForTimeout` / `wait_for_timeout` keeps the familiar name, but it cannot type-check, because its parameter is `never`, `Never` or `Infallible`. Its type-level deprecation (TSDoc `@deprecated`, PEP 702, `#[deprecated]`) names the app-state waits and the host's own timer, and at run time it fails with the new Error code `removed`, a `ProgramError`. It does nothing, so it is not an alias (S2). It gives people and coding agents the pointer at the moment they write the call (T). Puppeteer removed its sleep in 22.0 without one.

### Cancellation

- **Each language's own mechanism, always reaching the engine.**
  - TypeScript: `signal?: AbortSignal` on every cancellable call. An abort rejects with `signal.reason`, as `fetch` does, so `AbortSignal.timeout()` and `AbortSignal.any()` compose natively.
  - Python: cancelling the task (`Task.cancel`, `asyncio.timeout`, anyio's `fail_after`) sends `$/cancel` without waiting for the engine's answer, and `CancelledError` propagates unchanged.
  - Rust: dropping a call's future sends `$/cancel` without blocking, under ADR 0078's `Drop` rule.

  A signal and a timeout both apply, and whichever comes first ends the call. The Run trace records a cancelled call as cancelled. An Effect already sent is never treated as undone (ADR 0010). How the blocking Python client cancels is Concurrency model per language's.

### Limits in code

- **Set on a connection and on a Run.** `connect()` takes `limits` for its Client connection, and starting a Run takes `limits` for that Run (ADR 0041's scopes). They are typed per kind of Usage: model tokens, model calls, browser time, proxy bytes, paid solves, and a cost as a decimal amount with an ISO 4217 currency (`Money`), so no float rounding reaches a cap.
- **Lowered, never raised.** A program may set or lower the Limits that cover its own work; no SDK has an operation that raises one, because only a person raises a Limit (ADR 0041).
- **Reaching a Limit.** The call that reaches a Limit fails with `limit_reached`, a `RefusedError` (ADR 0079), naming the Limit's kind, scope, value and where it was set. The Run ends `stopped` and `incomplete`, with the stop reason `limit`, and keeps its partial results (ADR 0029). How Usage and cost are reported on results and Runs is [Models, usage and cost in the SDK API](https://github.com/goodfetch/automan/issues/475)'s.
- **No step Limit, and no per-call Limit.** A model call already counts every step, and an Agent's own step cap, if it has one, is [The Agent in Collie's SDK API](https://github.com/goodfetch/automan/issues/474)'s. A call is bounded by its timeout and its Run's Limits; to cap one step alone, a program gives it a Run of its own, which is the unit of counting (ADR 0078). No call carries a hidden cap, as HyperAgent's `extract` did with `maxSteps: 2`.

## Considered Options

- **An absolute `deadline` parameter**, as gRPC and Trio take: rejected. It is exact, but no scraping leader uses it, and a duration nests just as well once the engine holds the end time (S3, T). **Both a timeout and a deadline**: rejected by S2.
- **`context.extendTimeout()`**, as on Crawlee's master branch for v4: rejected. A handler changing its own limit is hidden state, and the route's setting already gives a long handler its time.
- **Mutable default setters** (`setDefaultTimeout`, `setDefaultNavigationTimeout`), as in Playwright: rejected by G2. A setter on a page changes calls written elsewhere.
- **`0` meaning no timeout**, as in Playwright and Scrapy's `CLOSESPIDER_TIMEOUT`: rejected. It reads as "at once", and an explicit null says what it means.
- **Python taking a number**, as Playwright Python takes `float` milliseconds or a `timedelta` and httpx takes `float` seconds: rejected. Either unit silently misreads code written for the other; `timedelta` only is what Crawlee for Python takes (`request_handler_timeout=timedelta(minutes=1)`).
- **`Temporal.Duration` or a `timeoutInMs` suffix in TypeScript**: rejected. Temporal is unflagged only from Node 26, and Azure's suffix is unfamiliar where one unit holds throughout the SDK.
- **Clocks running through a Human pause**: rejected. Every human step would need a hand-raised timeout, which AI pain 11 reports.
- **One generic `page.waitFor(condition)`**: rejected. It is smaller, but unfamiliar to Playwright users and to models (S3, T).
- **Keeping `waitForTimeout`**, as Playwright does while marking it discouraged, or **removing it silently**, as Puppeteer 22.0 did: rejected. The first keeps the flaky habit; the second leaves people with no replacement (P37).
- **One callback form everywhere** (`waitForResponse(u, () => click())`, Java's form): rejected by S1. It is uniform but unidiomatic in TypeScript and Python. **The promise-first form in Rust**: rejected, because lazy futures make it unsafe.
- **Azure's `abortSignal` name**: rejected for `signal`, the name of `fetch` and Playwright (S3).
- **A wall-clock Limit kind in ADR 0041**: rejected. Elapsed time is not Usage, and the Usage ledger has nothing to hold. **Reporting a Run's timeout as an abort**: rejected by G6, because the Run trace would not say what stopped it.
- **A "call" scope for Limits**: rejected for now. It amends ADR 0041 and adds a second layer of caps to explain, and a Run of its own already caps one step. **A step Limit**: rejected, because model calls count steps.
- **A timeout failing its whole Run**: rejected. One slow page would end a crawl.

## Consequences

- **Amends ADR 0029.** A Run's own timeout is a fifth cause of `stopped`, beside a Limit, an abort, the loss of the engine and exhausted resources: such a Run is never `complete`, and it resumes with a fresh clock.
- **Error codes.** `removed` joins `goodfetch-protocol`'s list in the `program` group, and `timeout` joins the stop reasons of ADR 0079's AI work results and of a Run.
- **Area 3 (Engine runtime)** builds the phase timeouts, their nesting and the Run's timeout. **Area 5 (Site access)** builds the waits, the `networkidle` definition and arming in call order. **Area 11 (Client surfaces)** builds each language's duration type, the three states, cancellation and Limits in code. Each lists this ADR.
- **ADR 0037's matrix**: whole-run and phase timeouts (RT-06) and waits (SA-14) are decided here.
- **The `prototype/sdk-api` sketch** holds this ADR's signatures in its `time` files for each language. If its golden scenarios show that a Run of its own is a clumsy way to cap one AI step, a call scope for Limits returns to the map as a ticket of its own.
