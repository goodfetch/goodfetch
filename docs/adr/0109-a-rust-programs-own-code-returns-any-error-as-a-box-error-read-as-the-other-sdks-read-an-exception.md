# A Rust program's own code returns any error as a `BoxError`, read as the other SDKs read an exception

Every function of a Rust program that the SDK calls and that can fail (a crawl handler, a crawl Hook, a login step, an Agent tool, the Agent's Success check, an SDK handler's `fail`) returns its error as one type, `BoxError`: std's `Box<dyn std::error::Error + Send + Sync + 'static>`, the name tower and AWS Lambda's Rust runtime give it. `?` turns any error the program meets into it, so a `ParseIntError` or a `serde_json::Error` in a handler ends the attempt `failed` with `handler_failed` naming it, as a `throw` does in TypeScript and Python, while the SDK's own `Error` returned as the box keeps its code. Before this, a Rust handler returned the SDK's opaque `Error`, which nothing of the program converts into, so `handler_failed` was reachable only by a panic, and golden scenario 06 turned a wrong JSON shape into a skip, which changes the outcome. Decided in [A Rust crawl handler's own error](https://github.com/goodfetch/goodfetch/issues/528), on the map [The SDK API of Collie and Foxhound](https://github.com/goodfetch/goodfetch/issues/461).

## Decision

### One error for the program's own code

- **`BoxError`** is `Box<dyn std::error::Error + Send + Sync + 'static>`, defined in the Collie client crate and re-exported by `goodfetch_collie` and `goodfetch_foxhound` (ADR 0095). std converts into it every `Error + Send + Sync + 'static`, a `&str` and a `String`; the program's helpers that return a `Box<dyn Error + Send + Sync>` pass it through; `anyhow::Error` and `eyre::Report` convert into it; and so does the SDK's own `Error`. It is the language's own "any error" (tower's `BoxError`, Lambda's `Error`, a `main` returning `Box<dyn Error>`), so it needs no third-party type (G7), no Goodfetch type to learn and no conversion call (S1, S3, T).
- **Where it is the error.** In every language a function of the program fails by raising its own exception; in Rust each of these returns `Result<_, BoxError>`, or takes one:
  - a crawl handler (`crawl.handle`), async and blocking, and the `finish` of a `CrawlCall` answered through the way down (ADR 0093);
  - a crawl Hook at every stage, async and blocking, and the `finish` of a `HookCall` (ADR 0104);
  - a login step: a crawl's, a Label's and a Session's (ADRs 0092, 0093);
  - an Agent tool and the Agent's Success check (ADR 0100);
  - an SDK handler call's `fail`: a Model provider's, a Proxy provider's, a Fetch backend's and a Sink's (ADRs 0101, 0104);
  - the function a crawl context's `for_each` calls with each item of a page's stream, async and blocking ([ADR 0111](0111-a-rust-crawl-hands-what-it-subscribes-on-its-page-to-serve-which-reads-it-for-the-rest-of-the-attempt.md)), whose own error its item kind's rule handles (`page_handler_failed` for a route, `listener_failed` for a listener);
  - any later function of the program the SDK calls with a crawl's context, under the same rule.
- **One name.** The sketch's two names for this box, `BoxError` and `HandlerError`, become `BoxError` (S1, S2). An Effect body keeps ADR 0106's generic error, which `BoxError` satisfies, so a body inside a handler may return it.
- **The cost.** A body whose last expression is an SDK call that returns the SDK's `Result` (`ctx.push(…).await`) ends `ctx.push(…).await?; Ok(())`, as every function returning `anyhow::Result` does; rustc's error names that fix.

### How the SDK reads it

- **The SDK's own error keeps its code.** A box that holds the SDK's `Error` (it downcasts) keeps its code and retryable flag, as a `GoodfetchError` raised inside a handler does (ADR 0079), except that a Hook's stays wrapped in `hook_failed` naming it (ADR 0104): a navigation timeout returned with `?` is retried as its flag says.
- **Anything else is the program's own,** an error that wraps a Goodfetch error in its `source()` chain included, as Python's `raise MyError() from error` and TypeScript's `new Error(…, { cause })` raise the program's error. The outermost error decides, in every language (S1, G1). An `anyhow::Error` that holds a Goodfetch error is converted by `?` into a box of anyhow's own wrapper, which no longer downcasts, so it is the program's own too; a program that wants the code kept returns the SDK's `Error` itself.
- **Its code** is the one each place already gives a program's own exception: `handler_failed` for a handler (ADR 0079); `hook_failed` with no wrapped code for a Hook (ADR 0104), where a Goodfetch code returned by a Hook stays wrapped in `hook_failed` naming it, as ADR 0104 has it; `page_handler_failed` or `listener_failed` for a `for_each` function (ADR 0111); `logged_out`'s cause for a login step (ADR 0092); `agent_tool_failed` and `success_check_threw` (ADR 0100); each kind's `<kind>_handler_failed` (ADRs 0101, 0104). Retries follow those ADRs: `crawl.retry.handler_failed` is still off by default (ADR 0093).

### What `handler_failed` carries from Rust

ADR 0079's details carry the exception's type name, its message and the stack frames of the program's code, and its message always names that exception. A Rust program's error fills them as follows, for `handler_failed`, `hook_failed`, `agent_tool_failed`, `success_check_threw` and each `<kind>_handler_failed` alike:

- **Type:** `BoxError` for an error and `panic` for a panic. The box erases the concrete type, and Rust has no stable way to read it back; a made-up or guessed name, from `std::any::type_name` of the box (which Lambda's own docs call unreliable) or from the `Debug` form (`std::io::Error`'s starts with `Os` or `Custom`), would mislead (G1).
- **Message:** the error's `Display`, followed by each `source()` in turn, joined by `": "`, as anyhow's and eyre's reports print an error; a panic's payload when it is a `&str` or a `String`. Outside the process the chain is gone, so the message carries it: that is how a Rust error is diagnosed (G6).
- **Stack:** empty. A Rust error carries no stable backtrace, and the SDK installs no panic hook (G4).
- **In the process,** the boxed error is the Goodfetch error's `source()`, and a panic's payload is kept as its message.

### The calls that end the work

- **`ctx.skip()` and `ctx.retry()`** on a crawl context, async and blocking, and a Hook's `skip`, `block`, `retry`, `drop_record` and `reject`, return a `BoxError` holding the SDK's `Error` with its code, so ADR 0093's and ADR 0104's form `return Err(ctx.skip("…"))` compiles as written (T). Their only use is to be returned, and the engine ends the attempt from the call itself, so one wrapped or swallowed still ends it (ADR 0105).

### Panics

- **A handler's panic** stays `handler_failed`, caught by `serve` and the blocking `serve` (ADR 0093). **A crawl Hook's or a login step's panic** inside `serve` is caught the same way and is that function's own error: `hook_failed`, or `logged_out`'s cause, as their exceptions are in TypeScript and Python (S1). A panic is a bug, not a stop.

### Golden scenario 06

The Rust program deserializes each JSON page with `serde_json::from_value(…)?`: a wrong shape ends the request `failed` with `handler_failed` naming the deserializer's error, as TypeScript's Zod and Python's Pydantic errors do, where the program used to skip it.

## Considered Options

- **An anyhow-shaped `HandlerError`** with `impl<E: Error + Send + Sync + 'static> From<E>`, which could record `type_name::<E>()` and a backtrace: rejected. By coherence it cannot also take a `&str`, a `String`, a `Box<dyn Error>` or an `anyhow::Error` (anyhow's own `from_boxed` exists for this), so `?` on a program's helper fails to compile, and it cannot itself implement `std::error::Error`. A trap for programs and for coding agents (S1, T) to buy a type name.
- **A constructor `Error::handler(source)`:** rejected. `.map_err(Error::handler)?` on every fallible line is a form no other SDK has (S1) and no Rust library asks of its users.
- **A generic error per handler** (`E: Into<BoxError>`): rejected. A closure that uses `?` on two error types and ends with `Ok(())` leaves `E` uninferred, the common handler shape (T).
- **Panics only:** rejected. A panic for an expected failure breaks Rust's error idiom, and TypeScript's and Python's `throw` keeps no Rust twin (S1).
- **Searching the `source()` chain for a Goodfetch error:** rejected. Rust would then read a program's deliberate wrapping differently from Python and TypeScript (S1).
- **`std::any::type_name` of the box, or the `Debug` form, as the type name:** rejected by G1; the first names the box and Lambda calls it unreliable, the second is wrong for common types.
- **Keeping `skip()` and `retry()` returning `Error`** (`return Err(ctx.skip("…").into())`), or `Err(ctx.skip("…"))?`: rejected. The documented form would not compile, and clippy's `try_err` flags the second (T).
- **Keeping `HandlerError` beside `BoxError`:** rejected; two names for one type (S2).

## Consequences

- **Amends ADR 0093.** A Rust handler, a login step and the way down's `CrawlCall::finish` take `BoxError`; `ctx.skip()` and `ctx.retry()` return it; a crawl Hook's or a login step's panic inside `serve` is caught as a handler's is. Notes sit under "How a request ends".
- **Amends ADR 0079.** A Rust handler's own error is any error returned as a `BoxError`; the outermost error decides whether a Goodfetch code is kept; in Rust `handler_failed`'s type name reads `BoxError` or `panic`, its message carries the `source()` chain and its stack is empty. Notes sit under "A handler's own exception" and its Rust bullet.
- **Amends ADR 0104.** A Rust Hook returns `BoxError`, its ending calls return it, `HookCall::finish` and the SDK handler calls' `fail` take it, and a Hook's panic is `hook_failed`. Notes sit under its Rust form, its ending calls and its failures.
- **Adds to ADRs 0092, 0100 and 0101.** A Rust login step, Agent tool, Success check and Model provider call's `fail` take `BoxError`, read by the same rule. Notes sit under the login step, what a tool raises and the program's own provider.
- **Amends ADR 0111.** `for_each`'s function returns `Result<(), BoxError>`, async and blocking. Notes sit under its bounds and its blocking form.
- **Adds to ADR 0105.** In Rust, `runHandler`'s `handler_failed` carries the details above. A note sits under its attempt's endings.
- **`TESTING.md`** gains the Rust program-error conformance cases.
- **Area 11 (Client surfaces)** builds `BoxError`, its downcast and the details it fills in the Rust SDK, and lists this ADR. **Area 6 (Collection)** and **area 7 (Plugin protocol)** read `handler_failed`, `hook_failed` and `<kind>_handler_failed` from Rust as above.
- **ADR 0037's matrix:** error types that steer retries (RT-05) cites this ADR.
- **Other tickets of the map.** [A route a Rust before-navigation Hook installs](https://github.com/goodfetch/goodfetch/issues/530) (ADR 0111) left `for_each`'s error type to this ticket: it is `BoxError`, above. [Send bounds on a Rust crawl's handlers](https://github.com/goodfetch/goodfetch/issues/531) decides the bounds of these functions; their futures' error stays `BoxError`. [Close the pain coverage matrix](https://github.com/goodfetch/goodfetch/issues/485) gets pain row P48's Rust part filled.
- **The `prototype/sdk-api` sketch** replaces `HandlerError` with `BoxError` in its `results`, `agent`, `plugins`, `models` and `sessions` files and Foxhound's `crawl`, `hooks` and `blocking`; scenario 06 returns its JSON error with `?`, the `crawl` snippet shows a `ParseFloatError` ending `handler_failed`, and P48 names this ADR.
