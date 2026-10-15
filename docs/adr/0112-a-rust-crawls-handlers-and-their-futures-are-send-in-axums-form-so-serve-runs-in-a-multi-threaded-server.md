# A Rust crawl's handlers and their futures are `Send`, in axum's form, so `serve` runs in a multi-threaded server

A Rust crawl's handlers, its Hooks and login steps, what it hands to `ctx.for_each`, and every Effect body are `Send`, and so are the futures they return. Each closure the SDK calls again and again is written in axum's form, `FnOnce(…) -> Fut + Clone + Send + Sync`, cloned for each call, and an Effect body, called once, is `FnOnce(Effect) -> Fut + Send`. So `crawl.serve(&run)` returns a `Send` future that runs inside an axum handler or a spawned task, and a program whose state cannot be `Send` answers `run.crawl_requests()` itself.

`Send` is Rust's marker for a value that may move to another thread. A multi-threaded runtime, Tokio's default and the one axum uses, may resume a task on any thread, so everything a task holds across an `.await` must be `Send`.

[ADR 0081](0081-each-sdk-follows-its-languages-concurrency-model-and-shapes-events-by-meaning.md) runs a crawl's handlers inside the future `serve` returns, and [ADR 0093](0093-a-crawl-declares-its-labels-and-one-handler-each-and-a-handler-reads-collies-page-on-every-fetch-tier.md) let them "need neither `Send` nor `'static`". That future holds the `Crawl`, which holds the handlers, so it was not `Send` either. axum requires a handler's future to be `Send + 'static` (axum 0.8.9's `Handler::Future`), and so does `tokio::spawn`.

The sketch compiled only because its `Crawl` stub held nothing. Holding the handlers it would really hold, golden scenario 12 fails at `get(get_books)` with E0277. That breaks ADR 0081's promise of spawnable futures (C-SEND-SYNC) and the scenario's point, a crawl inside a web service (pain rows P05 and P22; G4).

On stable Rust (1.93), an `AsyncFn` bound cannot require its future to be `Send`. That needs return-type notation ([rust-lang/rust#109417](https://github.com/rust-lang/rust/issues/109417)), and [RFC 3668](https://github.com/rust-lang/rfcs/blob/master/text/3668-async-closures.md) keeps the future's type unnameable. An `Fn(…) -> Fut` bound refuses every `async move` closure that owns its captures. axum's `FnOnce + Clone` form accepts both kinds of async closure, since RFC 3668 makes every async closure `FnOnce`.

Decided in [Send bounds on a Rust crawl's handlers](https://github.com/goodfetch/goodfetch/issues/531), on the map [The SDK API of Collie and Foxhound](https://github.com/goodfetch/goodfetch/issues/461), under the principles of [ADR 0077](0077-the-sdk-api-follows-eight-guarantees-and-four-ranked-shape-rules.md). It was found while writing golden scenario 12 in Rust ([#483](https://github.com/goodfetch/goodfetch/issues/483)). TypeScript and Python are unchanged: their handlers run on the program's event loop or the connection's thread pool (ADR 0081), and neither language has a `Send` marker.

## Decision

### Handlers are `Send`, always

- **One `Crawl`, with `Send` handlers and `Send` futures** (G4, S1, S2):
  - `serve`'s future runs in axum, tonic, a task spawned on a multi-threaded runtime, or a `#[tokio::test]`.
  - A `Send` future also runs on a single-threaded executor, actix-web's per-worker runtimes included, so no program loses a place it could run.
  - This follows the Rust ecosystem's default. axum's handlers and `tokio::spawn` need `Send` futures, async-trait makes futures `Send` unless a trait opts out with `?Send`, and the Rust team's [guidance on public async traits](https://blog.rust-lang.org/2023/12/21/async-fn-rpit-in-traits/) has authors provide the `Send` form.
- **State that cannot be `Send` stays with the program** (G8). A program whose handler state is `Rc` or `RefCell` reads `run.crawl_requests()`, the `EventStream` beneath `serve` (ADR 0093). It takes no closure and imposes no bound, so the program answers each `CrawlCall` in its own code, on its own executor. Shared mutable state in a handler is an `Arc` of a lock or an atomic, as in any handler that runs concurrently.

### The form: axum's

The form was chosen on S1 and S3 (Rust's idiom, under the name and shape axum users already write), T (one form that every async closure fits) and G3 (a closure that is not `Send` fails to compile, with rustc's message).

- **A crawl's handler** is `F: FnOnce(CrawlContext<P>) -> Fut + Clone + Send + Sync + 'a`, with `Fut: Future<Output = …> + Send + 'a`. `serve` clones the handler for each crawl request and calls the clone once, as axum does for each HTTP request. The output type is ADR 0093's, as [A Rust crawl handler's own error](https://github.com/goodfetch/goodfetch/issues/528) settles it. Whatever error type it gives, a `Send` future needs that error to be `Send`.
- **Every async closure fits.** RFC 3668 makes every async closure `FnOnce`, and a closure is `Clone` when its captures are. Two forms work unchanged:
  - `async |ctx| { … }` borrowing the declaring function's locals, as scenario 12 borrows its Dataset;
  - `async move |ctx| { … }` owning clones of the Labels and Datasets it uses, as scenarios 05, 08 and 10 return a `Crawl<'static>`.

  The `Fn(…) -> Fut` form would refuse the second ("async closure does not implement `Fn` because it captures state from its environment"). A closure that mutates a value it captured by move mutates a fresh clone each time; rustc warns that the value "is never read".
- **The same form, wherever the SDK stores a closure and calls it later.** It holds the closure behind a trait object, which cannot carry its future's `Send`-ness, so the bound is written in the signature:
  - **On a `Crawl`:** the six Hook stages ([ADR 0104](0104-a-crawl-declares-its-hooks-per-stage-in-code-a-hook-returns-its-change-and-a-program-registers-its-own-plugins-by-name.md)), and the crawl's and a Label's login steps (ADR 0093). These are `Send + Sync + 'a`, with `Send + 'a` futures.
  - **The handler a crawl's code hands to `ctx.for_each`** ([ADR 0111](0111-a-rust-crawl-hands-what-it-subscribes-on-its-page-to-serve-which-reads-it-for-the-rest-of-the-attempt.md)), which `serve` reads for the rest of the attempt. It is `Send + Sync + 'static`, with a `Send + 'static` future, and the stream's items are `Send + 'static`, answering what ADR 0111 left here.
  - **A Session's login step** ([ADR 0092](0092-sessions-open-from-their-source-page-pause-waits-for-a-person-and-a-secret-is-a-placeholder-bound-to-domains.md)). It runs inside a call's future, which ADR 0081 makes `Send + 'static`, so the step is `Send + Sync + 'static`, with a `Send + 'static` future.
  - **An Agent tool and an Agent task's Success check** ([ADR 0100](0100-collies-agent-is-one-bounded-call-on-a-page-judged-by-the-programs-success-check-and-replayed-whole-from-the-plan.md)). They were already `Send + 'static` as `Fn(…) -> Fut`, and take the `FnOnce + Clone` form so that `async move` tools fit too (S2).
- **An Effect body is called once:** `F: FnOnce(Effect) -> Fut + Send + 'a`, with `Fut: Future<Output = Result<T, E>> + Send + 'a`, and `T` and `E` `Send` ([ADR 0106](0106-a-program-declares-an-effect-as-a-keyed-block-whose-output-the-effect-journal-keeps-judged-by-what-the-engine-sent.md)). The declaration's future is `Send`, so a handler that declares an Effect, or a Collie program in a server that declares one, stays `Send`.
  - The body gets `Effect` by value, a cheap clone holding its keys, because a `FnOnce(&Effect) -> Fut` future cannot borrow its argument.
  - Bodies are still written `async |effect| { … }`.
- **A synchronous closure inside a call's future** is `Fn(…) + Send + Sync + 'static`, as ADR 0081's waiter predicates are. This covers `enqueue_links_with`'s Payload function.
- **A closure that a generic `async fn` calls at once needs nothing stated.** `Collie::scope` and `Foxhound::scope` take `impl AsyncFnOnce(&…)`, and their futures are `Send` whenever the body's is, through Rust's auto traits.
- **When return-type notation stabilizes**, the bounds may relax to `AsyncFnOnce(…) + Clone + Send + Sync` with `F(..): Send` as an addition. RFC 3668 makes every callable that returns a future implement `async FnOnce`, so the relaxed bound accepts every closure this one does. `AsyncFn` would not do: a closure that moves an owned capture out in its body, such as `async move |_| { consume(state) }`, fits `FnOnce + Clone` but not `AsyncFn` (E0525).

### Borrowing is kept, and it is checked

- **Not `'static`.** Handlers, Hooks, a crawl's login steps and Effect bodies keep the `'a` that lets them borrow the declaring function's locals (ADR 0081). axum's handler future is `'static` because scenario 12 declares its crawl inside it. A program that spawns `serve` builds a `Crawl<'static>` from handlers that own clones, and spawns `async move { crawl.serve(&run).await }`.
- **The blocking API is unchanged.** `goodfetch_foxhound::blocking`'s handlers, Hooks and login steps are already `Fn + Send + Sync + 'static` (ADR 0081).
- **Checked at compile time** (C-SEND-SYNC, ADR 0081): `Crawl<'static>` and `LabelDeclaration<'static, P>` are `Send + Sync`, and `serve`'s future is `Send`. A handler that holds an `Rc` across an `.await` fails to compile, with rustc naming the value that is not `Send` (G3).

## Considered Options

- **Handlers that are not `Send`** (ADRs 0081 and 0093 as written): rejected. A crawl could not run inside axum or a spawned task, scenario 12 does not compile once the `Crawl` holds its handlers, and ADR 0081's promise of `Send` futures breaks (G4).
- **A `Crawl` and a `LocalCrawl` pair**, `trait-variant`'s pattern: rejected. Every declaration method, Hook stage, login step and the testing kit would exist twice (S2, T). A crawl declared once could not move from a test into a service. The way down already serves state that cannot be `Send`.
- **`tokio::task::LocalSet` in the docs:** rejected. axum does not run its handlers in one, so the program would pin `serve` to a thread of its own and send results back (G4).
- **`Send` as an opt-in variant:** rejected. It is the pair with the wrong default, against async-trait's default and the Rust team's guidance.
- **`AsyncFn` with return-type notation:** rejected. It is unstable, and it refuses a closure that moves an owned capture out of its body (E0525), which `FnOnce + Clone` accepts. The later relaxation above is to `AsyncFnOnce(…) + Clone` with return-type notation instead.
- **`Fn(…) -> Fut`:** rejected. On rustc 1.93 it refuses every `async move` handler that owns its captures, the form three golden scenarios use, and leaves `move |ctx| { let x = x.clone(); async move { … } }` in its place (T).
- **Handlers returning a boxed future:** rejected. Every handler would need `Box::pin(async move { … })`, a form none of Rust's async closures has.
- **`'static` handlers**, as `blocking` has them: rejected. `Send` alone is what a multi-threaded server needs, and borrowing the declaring function's locals is what keeps scenario 12 short.
- **`AsyncFnOnce(&Effect)` kept for Effect bodies:** rejected. Its future cannot be `Send` on stable Rust, so any handler or server future that declares an Effect would lose `Send`.

## Consequences

- **Amends ADR 0081.** A crawl's handlers, its Hooks and login steps and a Session's login step are `Send`, with `Send` futures, in axum's `FnOnce + Clone` form; an Effect body is `FnOnce(Effect) -> Fut + Send`, with a `Send` future. They still run inside the future the program awaits and may borrow, but are no longer free of `Send`. A note sits under "Where user code runs".
- **Amends ADR 0093.** A Rust crawl's handlers and login steps are `FnOnce(…) -> Fut + Clone + Send + Sync + 'a` with `Send` futures, and `serve`'s future is `Send`. A note sits under "Where handlers run".
- **Amends ADR 0104.** A Rust crawl's Hooks take the handlers' form, and are `Send` with `Send` futures. A note sits under "Declaring a Hook".
- **Amends ADR 0111.** `ctx.for_each`'s handler is `FnOnce(T) -> Fut + Clone + Send + Sync + 'static`, with `Fut: Send + 'static` and `T: Send + 'static`, so `serve`'s future stays `Send`. A note sits under its bounds.
- **Amends ADR 0106.** A Rust Effect body is `FnOnce(Effect) -> Fut + Send` with a `Send` future, given `Effect` by value, and its declaration's future is `Send`. A note sits under "Each language's form".
- **Leaves to [A Rust crawl handler's own error](https://github.com/goodfetch/goodfetch/issues/528)** the error type a handler returns, which this ADR requires to be `Send`.
- **Area 11 (Client surfaces)** builds the bounds, the per-request clone, the compile-time checks, and a compile-fail test for a handler that is not `Send`. ADR 0037's SDK-06 and RT-07 list this ADR.
- **The `prototype/sdk-api` sketch** has these bounds in `crawl.rs` (`for_each` included), `hooks.rs`, `effects.rs`, `run.rs`, `sessions.rs` and `agent.rs`, the compile-time checks in `crawl.rs`, and a `serve` spawned on Tokio beside a program answering `crawl_requests()` with `Rc` state in `snippet_concurrency.rs`.
