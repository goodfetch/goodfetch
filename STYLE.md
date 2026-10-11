# Code style

How code is shaped in Goodfetch: the engine, its crates, the SDKs, the Console and the tools around them. Every rule comes with its reason, so that an exception can be judged against it. The hard-to-reverse rules (contracts, errors on the wire, cancellation, casing, concurrency, Cargo features) are decided in [ADR 0038](docs/adr/0038-contracts-from-rust-types-one-error-cancellation-and-concurrency-model.md); this file restates them only as far as code needs them. Rules a tool can check also live in the tool's configuration, which wins when the two disagree, and this file is then corrected.

Where this file is silent, the references are the [Rust API Guidelines](https://rust-lang.github.io/api-guidelines/checklist.html) and Microsoft's [Pragmatic Rust Guidelines](https://microsoft.github.io/rust-guidelines/).

## Design

The design vocabulary is the one of `codebase-design`: a **module** has an **interface** (everything a caller must know, not only its types), it sits at a **seam**, and it is **deep** when a lot of behaviour hides behind a small interface.

- **A crate's public API is its interface.** Everything else is `pub(crate)` or private, and the `unreachable_pub` lint warns on a `pub` that nothing outside can reach.
  _Why:_ what callers can't see, maintainers can change freely.
- **Concrete types first; a trait only where two adapters exist.** A test fake counts as the second adapter only when the dependency has no local stand-in: SQLite in a temporary file is a stand-in, so it gets no trait; a remote model provider has none, so a mock adapter is justified. The Plugin kinds of ADR 0033 are real seams, since built-ins, process Plugins and SDK handlers all implement them.
  _Why:_ a trait with one implementation is indirection with no payoff, and it spreads generics and slows compiles.
- **Prefer types over generics, and generics over `dyn`.** When `dyn` is needed, hide it in a newtype of our own (`struct Transport(Arc<dyn TransportImpl>)`) instead of showing `Arc<dyn …>` in an interface.
  _Why:_ the interface stays stable if the dispatch strategy changes.
- **State with one owner is an actor.** A task owns it and receives messages through a channel; no `Arc<Mutex<…>>` around a connection, a CDP session or a run's writer.
  _Why:_ one owner removes lock ordering, lock-held-across-await bugs and shared mutation.
- **Tests go through the interface.** If a test needs to reach past it, the module is the wrong shape.
  _Why:_ such tests survive internal refactors, and they test what callers rely on.

## Names

- **Identifiers use the terms of [`GLOSSARY.md`](GLOSSARY.md), never its `_Avoid_` terms.** It is `Session`, not `Login` or `Profile`; `Sink`, not `Output` or `Connector`. A concept the code needs that has no entry gets one in the glossary first.
  _Why:_ specs, docs, code and conversations then use one language.
- **Full words, no abbreviations** except those everyone writes (`id`, `url`, `http`, `cdp`).
- **Rust naming follows the Rust API Guidelines:** casing, `as_`/`to_`/`into_` for conversions by cost and ownership, no `get_` prefix on getters, `iter`/`iter_mut`/`into_iter`.

## Rust

### Workspace and crates

- **The root `Cargo.toml` is a virtual manifest.** Members live under `crates/` and `crates/sinks/`, plus `proving-ground/` (ADR 0032) and an `xtask/` folder at the root for `xtask codegen` and `xtask release`. Each folder name equals its crate name, and internal crates are named `goodfetch-<glossary term>`. Siblings and shared dependencies are referenced with `.workspace = true`.
- **A concern becomes its own crate only for one of these reasons:**
  1. it is a separate executable (the binary, a sink process, the sandbox helper, `xtask`, the Proving ground);
  2. the compiler must enforce a dependency direction (`goodfetch-protocol` never depends on the engine; the Proving ground never depends on engine crates; a Private component's interface with its no-op implementation);
  3. it is heavy and rarely changes, so isolating it speeds up incremental builds (generated CDP bindings);
  4. it is a Plugin-kind seam whose built-in implementation pulls dependencies nothing else needs.

  Otherwise it is a module. Each area's spec names its crates by these reasons.
  _Why:_ crates build in parallel and enforce directions, but each one adds visibility plumbing; a stated reason keeps them meaningful.
- **Module files are `foo.rs` beside a `foo/` folder, never `mod.rs`.** `lib.rs` only declares modules and re-exports.
  _Why:_ editor tabs full of `mod.rs` are unreadable, and a short `lib.rs` shows the crate's shape at a glance.
- **Each crate and each module opens with a `//!` comment stating its role.**

### Errors

- **Each crate has its own typed errors, written with `thiserror`.** Public error types are `#[non_exhaustive]`.
- **`anyhow` only in `main`, `xtask` and tests.**
- **Every failure a Client can see maps to an Error code** from the list in `goodfetch-protocol` (snake_case string, retryable flag, JSON-RPC class). Only the RPC layer converts errors for the wire.
  _Why:_ Error codes are under semver (ADR 0030) and every SDK generates a class per code, so a code is matched on, never a message.
- **No `unwrap()` outside tests.** `expect("invariant: …")` states an invariant the type system can't express; anything input can trigger returns an error.
- **Panics are for bugs only, never for input or control flow.**

### Async and concurrency

- **Tokio is the runtime, and concurrency is structured** (ADR 0038): child cancellation tokens from the engine process (an Attached engine or the Daemon) to each run, Session and page; each owner waits for its own tasks before it stops.
- **No task starts outside an owner's tracker.** Spawn through it (`TaskTracker::spawn`, `TaskTracker::spawn_blocking`); a Clippy `disallowed-methods` rule bans `tokio::spawn`, `tokio::task::spawn_blocking`, `tokio::task::spawn_local`, `Handle::spawn`, `Handle::spawn_blocking`, `Runtime::spawn` and `Runtime::spawn_blocking` outside the module that implements this. A tracked task may own a `JoinSet` for a fan-out whose results it awaits before it returns.
  _Why:_ an orphan task outlives its cancelled run and keeps writing.
- **Never block an async worker.** Blocking I/O and CPU-heavy work go to the owner's tracked `spawn_blocking`, or to a dedicated thread (an actor's, for example) that its owner joins before it stops; `std::thread::sleep`, blocking `std::process::Command` and blocking `std::fs` calls are disallowed in async crates.
- **Never hold a lock across `.await`.**
- **Every public async function has a `# Cancel safety` section** saying what is lost if its future is dropped.
- **Public futures are `Send`**, checked by a compile-time assertion on the main entry points.

### Lints, formatting and toolchain

- **Edition 2024, a pinned toolchain in `rust-toolchain.toml`, and the MSRV of ADR 0030 tested in CI.**
- **Warnings fail CI.** Clippy runs with `--all-targets --all-features`.
- **Clippy `pedantic` is on as warnings**, with a short allow-list kept in `[workspace.lints]`.
- **Denied:** `print_stdout` and `print_stderr`, `dbg_macro`, `todo`, `unimplemented`, `unwrap_used` (allowed in tests), `allow_attributes`, `allow_attributes_without_reason`, `clone_on_ref_ptr`, `exit` (outside `main`). `std::io::stdout` and `std::io::stderr` are in `disallowed-methods`.
- **Only three writers touch stdout or stderr:** the protocol transport, the CLI's output writer (its human output, its `--json` output and its hints on stderr) and the `tracing` subscriber's writer (stderr or a file). Everything else logs through `tracing`.
  _Why:_ when the binary runs as an Attached engine started by an SDK, or as the stdio MCP server, stdout is the JSON-RPC channel, so one stray write corrupts the protocol; and the CLI's `--json` output is under semver (ADR 0030).
- **Exceptions use `#[expect(lint, reason = "…")]`, never `#[allow]`.**
  _Why:_ `expect` warns when the exception is no longer needed, so stale exceptions disappear.
- **rustfmt runs on its default 2024 style.**
  _Why:_ the options teams most often change (import granularity and grouping, comment wrapping) need a nightly toolchain, and defaults end style debates.
- **CI also runs `cargo-deny`, `cargo-shear` (unused dependencies), `cargo-nextest`, `typos` and `zizmor` (workflow security).**

### `unsafe`

- **`unsafe_code` is denied across the workspace.** A module that needs it (the in-house `sandbox_init` binding of ADR 0033, the pipe file descriptors of ADR 0017) opts in with `#![expect(unsafe_code, reason = "…")]`.
- **Every `unsafe` block carries a `// SAFETY:` comment** (`undocumented_unsafe_blocks`), and `unsafe_op_in_unsafe_fn` is denied.
- **Miri runs in CI on the opted-in code it can execute (pure-Rust `unsafe`).** FFI and system-call modules, which Miri can't run, are covered by tests on each real OS.
- **`unsafe` is never used to get past `Send` or `Sync`.**

### Logging and secrets

- **`tracing` everywhere, never `println!` or `log`.** Spans cover each RPC request, run, page and CDP command, with glossary IDs as fields (`run_id`, `session_id`).
- **`#[instrument(skip_all, fields(…))]`**, so arguments are only logged when named.
  _Why:_ one `?config` in a log line can leak a proxy password, and secrets never reach logs (ADRs 0014, 0015).
- **Events are structured:** a fixed message plus fields, no `format!` inside a log call.
- **Logs go to stderr or a file, never stdout**, through the `tracing` subscriber.
- **Levels:** `error` needs someone to act; `warn` is a degraded but handled state; `info` is a lifecycle step of a run, Session or Daemon; `debug` explains a decision; `trace` is per-message detail.
- **Credentials, cookies, tokens and keys are `secrecy::SecretString`** (or a wrapper with a hand-written `Debug`), and a unit test proves the secret is not printed.

### Dependencies

A crate or package becomes a dependency only when it meets this bar:

- its license is on the allowlist of ADR 0018;
- it builds on every target: static musl (ADR 0024), macOS and Windows;
- it is maintained, or deliberately stable;
- it has no open security advisory;
- it supports our MSRV;
- its build script downloads nothing.

For anything non-trivial, a known and widely used project is preferred to writing it in-house. Versions live in `[workspace.dependencies]` with `default-features = false`, and each crate turns on what it uses. `cargo-deny` and the npm and Python equivalents (ADR 0032) enforce what they can.

### Cargo features

- **A capability is never a Cargo feature** (ADR 0038): stealth, the Challenge solver, AI and costly controls are settings.
- **Features exist only on the embeddable `goodfetch` crate**, only for heavy optional dependencies and `test-util`, and they are additive. The binary builds one fixed set, and `cargo-hack` checks every combination of `goodfetch`'s features.
- **Platform differences use `cfg(target_os = …)`, never features.** Custom `cfg`s are declared for `unexpected_cfgs`.

### Tests

A module is tested according to the kind of dependency it has, using the four categories of `codebase-design`:

- **In-process** (pure logic: URL normalization, robots, frontier rules, extraction, run-state rules): through the module's interface; `proptest` for parsers.
- **Local-substitutable** (a real local stand-in exists): SQLite in a temporary file; a real Chromium (the Goodfetch build of ADR 0035) against Proving ground pages served locally. CDP is never mocked.
  _Why:_ a fake browser lets tests pass where a real one fails, which is the false success ADR 0025 measures.
- **Remote but owned** (Goodfetch's own processes across a boundary: sink processes, process Plugins, the Daemon): the logic is tested through an in-memory adapter of the protocol, and the real processes through the conformance suites.
- **True external** (model providers, CAPTCHA services): a mock adapter replaying recorded exchanges.

Untrusted input gets a `cargo-fuzz` target per decoder: CDP messages, JSON-RPC and plugin-protocol framing, HTML and Markdown, sitemaps, run-state import.
- **Snapshots** use `insta`, with redactions for what varies, for JSON-RPC responses, generated surfaces and extraction output. Snapshot files are never edited by hand.
- **Layout:** one integration-test binary per crate (`tests/it/main.rs` with modules), run with `cargo-nextest`.
  _Why:_ each file in `tests/` links its own binary, which multiplies build time.
- **Doctests only on `goodfetch`**, where they are the examples of ADR 0030.
- **Test names describe the behaviour** (`resumed_run_never_repeats_an_effect`), with no `test_` prefix.
- **A flaky test is quarantined with an issue, never retried silently.**
- The conformance suites of the ADRs stay the interface tests for every contract with several implementations.

### Documentation in code

- **`missing_docs` warns on `goodfetch` and `goodfetch-protocol`.**
- **Each public function of `goodfetch` and `goodfetch-protocol` documents `# Errors`** (with the Error codes it can return), **`# Cancel safety`** if it is async and **`# Panics`** if it can panic; on `goodfetch` it also has **an example** that runs as a doctest.
- **Internal crates document what a caller outside the module must know**: their `//!` headers, `# Errors` where a caller would otherwise guess, and `# Cancel safety` on every public async function (ADR 0038).
- **The catalog's description of an operation is its only text source:** MCP tool descriptions, SDK docstrings and reference pages are generated from it.
  _Why:_ what a model reads in a tool description directly affects agent accuracy, and the surfaces then never disagree.
- **Comments say why, never what the code already says.**

## Protocol and SDKs

- **Contracts are Rust types in `goodfetch-protocol`**, emitted as a committed JSON Schema bundle (ADR 0038). To add or change an operation, edit the types, run `xtask codegen`, and commit what it produces.
- **Generated code is never edited by hand.** It lives in a `generated/` folder in each SDK, and CI fails when it differs from what `xtask codegen` produces.
- **Hand-written code in each SDK is the runtime and the ergonomic layer:** engine start and discovery, framing, `hello`, sockets, request correlation, flow-control credits, event resumption, file-backed bodies, cancellation, and the objects users hold.
- **The wire is snake_case.** Python uses wire names as they are; TypeScript presents camelCase through the generated mapping, which never renames user data (extracted records, headers, metadata maps).

## TypeScript

Applies to `sdks/typescript`, `console/`, `adapters/`, `integrations/` and `bench/` in the pnpm workspace.

- **TypeScript 7 with `strict`, `noUncheckedIndexedAccess`, `exactOptionalPropertyTypes`, `verbatimModuleSyntax`, `isolatedDeclarations` and `erasableSyntaxOnly`.** A TypeScript 6 alias is kept only for tools that need the compiler API (API Extractor, TypeDoc) until TypeScript 7 provides one.
- **Optional fields are written `?: T | undefined`.**
  _Why:_ the published types then work whether or not the consumer enables `exactOptionalPropertyTypes`.
- **oxlint with type-aware rules, and oxfmt for formatting.** `no-floating-promises` is an error.
  _Why:_ an unawaited promise loses its error silently; oxlint's type-aware rules run on the real TypeScript 7 checker.
- **Libraries build with tsdown; tests run with Vitest on every supported Node line (ADR 0024), plus a smoke suite of the built package on Bun and Deno 2.**
- **Packages are ESM-only** (ADR 0024) and pass `publint` and `@arethetypeswrong/cli` before publishing. npm is the only registry.
- **Named exports only; no TypeScript `enum`s** (string-literal unions instead).
- **SDK idioms:**
  - every operation that can be cancelled takes an `AbortSignal` option;
  - engines and Sessions are disposed with `await using`;
  - event streams are `AsyncIterable`;
  - engine responses are not validated at runtime beyond the protocol version check;
  - schemas users pass in (for extraction) are accepted as Standard Schema (the interface Zod, Valibot and ArkType share) and converted to JSON Schema.
- **Errors are one class per Error code, generated.**
- **Doc comments are TSDoc.**

## Python

Applies to `sdks/python`, the Python adapters and the uv workspace. Supported versions follow ADR 0024: every CPython not past end of life, including free-threaded builds.

- **pyright in strict mode gates CI**, and mypy checks a small file that exercises the public API.
  _Why:_ pyright is what most editors show; the mypy check keeps the types right for mypy users too.
- **ruff formats and lints**, with the rule sets `E F I UP B C4 ARG T20 TID251 PERF RUF ANN PYI TC ASYNC D`, ignoring `D206` and `D300`, which conflict with ruff's formatter.
  _Why:_ `ASYNC` catches blocking calls inside async code, and `T20` bans `print`.
- **Pure-Python packages build with `uv_build`; every package ships `py.typed`.** The SDK's `py3-none` platform wheels, which carry the engine binary (ADR 0024), are built by the release job of ADR 0032.
- **The async core uses anyio**, so the async API runs on asyncio and trio. The sync API is generated and runs that core on its own event-loop thread (ADR 0038).
- **Protocol data are pydantic v2 models, generated; errors are one exception class per Error code, generated.**
- **Tests use pytest with the anyio plugin** (every async test runs on asyncio and trio), `inline-snapshot` for protocol payloads and `hypothesis` for encode and decode round-trips. CI also runs the free-threaded build.
- **Docstrings follow the Google style**, enforced by ruff's `D` rules, for Griffe (ADR 0030).
