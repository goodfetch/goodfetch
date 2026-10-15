# A Rust crawl hands what it subscribes on its page to `serve`, which reads it for the rest of the attempt

A Rust crawl's code that subscribes something on its page and then returns, such as a before-navigation Hook that installs a route, hands the subscription's stream to `ctx.for_each(stream, handler)`. `crawl.serve(&run)` then reads that stream with the handler until the attempt ends. An init script it adds is kept with `Registration::keep()`. In TypeScript and Python a `beforeNavigation` Hook calls `page.route(url, handler)` and returns, and the handler stays until the page closes ([ADR 0104](0104-a-crawl-declares-its-hooks-per-stage-in-code-a-hook-returns-its-change-and-a-program-registers-its-own-plugins-by-name.md)). In Rust, `page.route(url)` is an `EventStream` the program must keep reading ([ADR 0081](0081-each-sdk-follows-its-languages-concurrency-model-and-shapes-events-by-meaning.md), [ADR 0085](0085-collies-page-api-keeps-playwrights-names-strictness-waits-values-cross-as-json-and-unanswered-requests-fall-through.md)). The Hook must return before the page navigates, so golden scenario 06 had to read the stream on a `tokio::spawn`ed task ([sketch at `eb1e35b`](https://github.com/goodfetch/goodfetch/blob/eb1e35b4d75779f2abcb3bdb6a9b31e353567514/sdk-api-sketch/rust/examples/s06_mixed_fetch_crawl.rs#L59-L78)). That task is outside the attempt's lease, needs Tokio, and drops its errors. Tokio says a dropped `JoinHandle` "*detaches* the associated task" ([docs](https://docs.rs/tokio/latest/tokio/task/struct.JoinHandle.html)). The same trap holds for dialogs, other page events and locator-handler turns, all `EventStream`s in Rust, and for an init script, whose `Registration` is removed when dropped (ADR 0085).

The Rust drivers either spawn or require spawnable closures:
- chromiumoxide's own interception example reads its paused-request stream on a spawned task and prints the errors it cannot return ([`interception.rs`](https://github.com/mattsse/chromiumoxide/blob/afcc3a4313f2087249b4490d94e54bf8e3bfaccf/examples/interception.rs#L37-L60)).
- padamson's playwright-rs 0.20 takes `F: Fn(Route) -> Fut + Send + Sync + 'static` ([`network.rs`](https://github.com/padamson/playwright-rust/blob/7de3b1c443fde1232c0a9df0988aea0320f55577/crates/playwright/src/protocol/page/network.rs#L44-L47)) and runs each call on a Tokio task ([`dispatch.rs`](https://github.com/padamson/playwright-rust/blob/7de3b1c443fde1232c0a9df0988aea0320f55577/crates/playwright/src/protocol/page/dispatch.rs#L195-L207)).

Playwright keeps a route until `unroute` or the page closes ([`page.route`](https://playwright.dev/docs/api/class-page#page-route)), and Puppeteer answers in a `page.on('request')` listener ([network interception](https://pptr.dev/guides/network-interception)), both on the event loop. Crawlee's `blockRequests` ([`playwright-utils.ts`](https://github.com/apify/crawlee/blob/v3.18.2/packages/playwright-crawler/src/internals/utils/playwright-utils.ts#L292-L310)) and scrapy-playwright's `PLAYWRIGHT_ABORT_REQUEST` ([README](https://github.com/scrapy-plugins/scrapy-playwright/blob/v0.0.48/README.md#playwright_abort_request)) are declarative (checked 2026-10-15). It answers the Rust part of pain rows P30 and P46. Decided in [A route a Rust before-navigation Hook installs](https://github.com/goodfetch/goodfetch/issues/530), every answer by the agent on the owner's delegation, on the map [The SDK API of Collie and Foxhound](https://github.com/goodfetch/goodfetch/issues/461), under the principles of [ADR 0077](0077-the-sdk-api-follows-eight-guarantees-and-four-ranked-shape-rules.md), building on [ADR 0093](0093-a-crawl-declares-its-labels-and-one-handler-each-and-a-handler-reads-collies-page-on-every-fetch-tier.md) and [ADR 0105](0105-a-programs-tests-replay-a-response-recording-in-a-test-engine-that-reaches-no-site.md).

## Decision

### `serve` reads what the crawl's code subscribed

- **`ctx.for_each(stream, handler)`** is a method of every Rust crawl context that holds a page: the page-stage context of the before-navigation, after-navigation and page-ready Hooks, and the handler's `CrawlContext`. `stream` is one of that page's own subscriptions: its routes (`page.route(url)`), its events (`page.events(…)`, dialogs included) or a locator handler's turns (`page.add_locator_handler(…)`). `crawl.serve(&run)`, the future already answering the attempt, reads it from then on and calls `handler` with each item, until the attempt ends and its page closes (ADR 0093). Nothing is spawned on the program's executor, so it runs on any executor (ADR 0081, G4), and it is tracked with the attempt, so its failures reach the attempt (G1, G6).
- **It follows ADR 0081's rule for crawl code** (handlers run "inside the future the program awaits") and ADR 0092's ("a closure given where its scope opens and run inside that call's future"). The streams are subscribed when their method is called, before navigation in a before-navigation Hook, so nothing slips through (ADR 0081).
- **The page API is unchanged.** A route is still `page.route(url)`, with ADR 0085's semantics. `for_each` only says who reads the stream, so each subscription keeps one form in Rust (S1), and one method serves every kind (S2), under the name of `futures`' combinator.
- **Order.** A route's requests are answered concurrently, as TypeScript's and Python's route handlers run. Events, dialogs and locator-handler turns come one at a time, in order (ADRs 0081, 0085).
- **An init script** a Hook adds outlives it through **`Registration::keep()`**, which keeps the script until its page or Session closes, as TypeScript's stays until disposed. It works in a Collie program too.

### Bounds and failures

- **Bounds.** `handler` is `AsyncFn(T) -> Result<(), Error> + 'static`: it outlives the code that installs it, and the contexts have no lifetime parameter (ADR 0081). It is `Send` exactly when the crawl's handlers are, which [Send bounds on a Rust crawl's handlers](https://github.com/goodfetch/goodfetch/issues/531) settles. Its error type is the Hooks' and handlers', which [A Rust crawl handler's own error](https://github.com/goodfetch/goodfetch/issues/528) settles.
  > Amended by [ADR 0112](0112-a-rust-crawls-handlers-and-their-futures-are-send-in-axums-form-so-serve-runs-in-a-multi-threaded-server.md): `handler` takes the handlers' form, `FnOnce(T) -> Fut + Clone + Send + Sync + 'static` with `Fut: Future<Output = Result<(), Error>> + Send + 'static` and `T: Send + 'static`, cloned for each item, so `serve`'s future stays `Send`.
- **A failing handler follows its kind's rule, as in TypeScript and Python** (S1). A panic counts as an error, as `serve` already catches a handler's (ADR 0093).
  - A route's request is aborted, the route removed, and `page_handler_failed` raised under ADR 0085's rule: from the attempt's call waiting on that request, or else from its next page call.
  - A listener is removed and `listener_failed` raised under ADR 0081's rule.
  - A dialog gets its default, and `page_handler_failed` follows the routes' rule.
  - A locator handler fails the action it interrupted with `page_handler_failed`, and stays registered.

  So the Rust SDK raises `page_handler_failed` and `listener_failed` for what `for_each` reads. `route_already_handled` still cannot happen in Rust, since an answer takes the route.
- **An error the stream itself yields** ends the attempt with that error's code.
- **An item the handler drops unanswered** gives its default, as ADR 0085 has it: a route falls back, a dialog gets its default, and a locator handler's turn lets the action go on.

### The way down, blocking and tests

- **The way down.** A context read from `run.crawl_requests()` or `run.hook_requests()` (ADRs 0093, 0104) has no `serve` answering its attempt. There `for_each` fails at once with `setting_invalid`, the code ADR 0104 gives a misused stream, and its `fix` names the stream to read. The program then reads the page's stream itself, as a Collie program does (G8).
- **Blocking.** The contexts of `goodfetch_foxhound::blocking` take an `EventIter` and an `Fn(T) -> Result<(), Error> + Send + Sync + 'static`. The blocking `serve` runs that function on the `tasks` module's tracked blocking threads, as it runs handlers (ADR 0081).
- **Tests.** `runHandler` serves its one attempt (ADR 0105), so what its handler hands to `for_each` answers that attempt's later requests. `runHook` ends with the Hook, so a Hook's route is exercised by a crawl started on a Response recording (ADR 0105).
- **What does not change:** TypeScript and Python, a Collie program's streams, which it reads with `during`, `select` or a task of its own, and ADR 0104's Hook answers.

### Golden scenario 06

The Rust program's before-navigation Hook subscribes `page.route("**/*")` and hands it to `ctx.for_each`, whose handler aborts images and fonts with `blockedbyclient` and falls back for the rest. The spawned task and its obligation are gone. Whether the scenario should block images and fonts with ADR 0047's `block_resources` setting instead of a Hook stays with the [Close the pain coverage matrix](https://github.com/goodfetch/goodfetch/issues/485), which holds that item from #483's review; either works in Rust now.

## Considered Options

- **The program's own spawned task**, as the sketch had it: rejected as the documented form. The task is outside the attempt's lease, its errors vanish (G1, G6), and it needs Tokio and `Send + 'static`, against ADR 0081's "any executor". A program may still do it, outside the attempt's guarantees.
- **A declarative route answer** (`page.route(pattern).abort()` or `.fulfill(…)` as a value the engine applies with no stream): rejected. It is a second form of route in every language (S2), or a Rust-only one (S1). It covers only answers that need no code, and resource types are not URLs. Blocking whole types is already a setting, ADR 0047's `block_resources`.
- **A before-navigation Hook answer that carries routes:** rejected. A handler, a page-ready Hook, a dialog or a locator handler could not use it, and Rust's Hook would install routes differently from TypeScript's, which installs them on the page (S1).
- **A task the SDK spawns on the connection's runtime:** rejected. It runs program code on the engine's runtime, which ADR 0081 rejected because a blocking handler would stall the engine.
- **One context method per kind** (`ctx.route(url, handler)`, `ctx.on(event, handler)`, `ctx.add_locator_handler(…, handler)`), Playwright's and playwright-rs's `route(url, handler)` shape: rejected. Each subscription would get a second spelling in one SDK (S1), three methods would do one thing (S2), and S2 outranks the familiarity S3 gives.
- **`ctx.add_init_script` on the context:** rejected for `Registration::keep()`, which adds no context method and serves a Collie program too.
- **Handlers that may borrow (`+ 'a`):** rejected. It needs a lifetime on the contexts, against ADR 0081's rule that no public type has one.

## Consequences

- **Amends ADR 0081.** In a Rust crawl, a stream the crawl's code subscribes on its page is read by `crawl.serve` through `ctx.for_each` for the rest of the attempt, with no spawn. The Rust SDK raises `listener_failed` for a listener read that way. Notes sit under "Where user code runs" and its error codes.
- **Amends ADR 0085.** The Rust SDK raises `page_handler_failed` for a route, dialog or locator handler read through `ctx.for_each`. `Registration::keep()` keeps an init script until its page or Session closes. Notes sit under its error table and its forms per language.
- **Adds to ADR 0104.** A Rust Hook at a page stage installs what must outlive it through `ctx.for_each` and `Registration::keep()`. On a context read from `run.hook_requests()`, `for_each` fails with `setting_invalid`. A note sits under its Rust form of Hooks.
- **Adds to ADR 0093.** `crawl.serve(&run)`, and the blocking `serve`, also read the streams the crawl's code hands to `ctx.for_each`. A note sits under where handlers run.
- **`TESTING.md`** gains the Rust crawl's `for_each` conformance cases.
- **Area 11 (Client surfaces)** builds `for_each` in the async and blocking crawl contexts, its refusal on the way down and `Registration::keep()`. It lists this ADR.
- **ADR 0037's matrix:** request interception (SA-17) cites this ADR.
- **Other tickets of the map.** [Send bounds on a Rust crawl's handlers](https://github.com/goodfetch/goodfetch/issues/531) and [A Rust crawl handler's own error](https://github.com/goodfetch/goodfetch/issues/528) apply their answers to `for_each`'s handler. [Close the pain coverage matrix](https://github.com/goodfetch/goodfetch/issues/485) keeps scenario 06's choice between the Hook and `block_resources`.
- **The `prototype/sdk-api` sketch**, at [`5b424f0`](https://github.com/goodfetch/goodfetch/commit/5b424f0aff6abd6e1cef5deec60a5878f518e1df), holds `for_each` on Foxhound's async and blocking contexts and `Registration::keep()` in Collie's client package, and golden scenario 06's Rust program uses them. Pain rows P30 and P46 cite this ADR.
