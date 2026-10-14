# Code style

How code is shaped in Goodfetch: the engine, its crates, the SDKs, the Console and the tools around them. Every rule comes with its reason, so that an exception can be judged against it. The hard-to-reverse rules (contracts, errors on the wire, cancellation, casing, concurrency, Cargo features) are decided in [ADR 0038](docs/adr/0038-contracts-from-rust-types-one-error-cancellation-and-concurrency-model.md); this file restates them only as far as code needs them. Rules a tool can check also live in the tool's configuration, which wins when the two disagree, and this file is then corrected.

Where this file is silent, the references are the [Rust API Guidelines](https://rust-lang.github.io/api-guidelines/checklist.html) and Microsoft's [Pragmatic Rust Guidelines](https://microsoft.github.io/rust-guidelines/).

## Design

The design vocabulary is the one of `codebase-design`: a **module** has an **interface** (everything a caller must know, not only its types), it sits at a **seam**, and it is **deep** when a lot of behaviour hides behind a small interface.

- **A crate's public API is its interface.** Everything else is `pub(crate)` or private, and the `unreachable_pub` lint warns on a `pub` that nothing outside can reach.
  _Why:_ what callers can't see, maintainers can change freely.
- **Concrete types first; a trait only where two adapters exist.** A test fake counts as the second adapter only when the dependency has no local stand-in: SQLite in a temporary file is a stand-in, so it gets no trait; a remote model provider has none, so a mock adapter is justified. Every external dependency is still injected, never reached through a global: as a value (the Goodfetch home, an address, the environment map) when a stand-in exists, and as a handle only for time (`Clock`) and random choices (`Entropy`), whose test versions are the second adapter (ADR 0059, rules in [`TESTING.md`](TESTING.md)). The Plugin kinds of ADR 0033 are real seams, since built-ins, process Plugins and SDK handlers all implement them.
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

- **The root `Cargo.toml` is a virtual manifest.** Members live under `crates/` and `crates/sinks/`, plus `proving-ground/` (ADR 0032) and an `xtask/` folder at the root for `xtask codegen` and `xtask release`. Each folder name equals its crate name, and internal crates are named `goodfetch-<glossary term>`, apart from the test-only `goodfetch-test-support`. Siblings and shared dependencies are referenced with `.workspace = true`.
- **A concern becomes its own crate only for one of these reasons:**
  1. it is a separate executable (the binary, a sink process, the sandbox helper, `xtask`, the Proving ground);
  2. the compiler must enforce a dependency direction (`goodfetch-protocol` never depends on the engine; the Proving ground never depends on engine crates; `goodfetch-conformance` depends only on `goodfetch-protocol`; `goodfetch-test-support` depends on no Goodfetch crate and is only ever a dev-dependency (ADR 0062); a Private component's interface with its no-op implementation);
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
- **No task starts outside an owner's tracker.** Spawn through it (`TaskTracker::spawn`, `TaskTracker::spawn_blocking`); a Clippy `disallowed-methods` rule bans `tokio::spawn`, `tokio::task::spawn_blocking`, `tokio::task::spawn_local`, `Handle::spawn`, `Handle::spawn_blocking`, `Runtime::spawn`, `Runtime::spawn_blocking`, `LocalSet::spawn_local`, `std::thread::spawn`, `std::thread::Builder::spawn` and `std::thread::scope` outside the module that implements this (`tasks`). A tracked task may own a `JoinSet` for a fan-out whose results it awaits before it returns.
  _Why:_ an orphan task outlives its cancelled run and keeps writing.
- **Never block an async worker.** Blocking I/O and CPU-heavy work go through the `tasks` module's blocking policy, a value: the owner's tracked blocking threads, or a dedicated thread (an actor's, such as the run's SQLite writer) that `tasks` starts and its owner joins before it stops. In the Simulation the same policy runs blocking calls inline and a dedicated thread's body as a task on the host's runtime (ADR 0060); `std::thread::sleep`, blocking `std::process::Command` and blocking `std::fs` calls are disallowed in async crates.
- **Sockets and runtimes have one place each** (ADR 0060).
  - Connections the Simulation carries (stdio of an Attached engine, the Daemon's socket and named pipe, loopback TCP, the WebSocket listener, the private endpoint, the ops listener, the peer link, wake endpoints) are opened only in `goodfetch-engine`'s `transport` module, which takes the network as a value; `rpc` takes its connections from it. `disallowed-types` lists the socket types of `std::net`, `std::os::unix::net`, `tokio::net` and Tokio's named pipes, and `disallowed-methods` lists `tokio::io::stdin`, `tokio::io::stdout` and `std::io::stdin`. Fetch sockets behind the egress checks, the CDP connection, plugin-protocol pipes and third-party clients open their own, each module with an `#[expect]` naming ADR 0060. So does the test-only `goodfetch-scripted-engine`, which the Simulation never runs, with an `#[expect]` naming ADRs 0060 and 0069.
  - Only `main`, test helpers and bench targets (`benches/`, ADR 0071) build a Tokio runtime (`Runtime::new`, `Builder::new_current_thread`, `Builder::new_multi_thread`, `#[tokio::main]`), and `tokio::task::block_in_place` is in `disallowed-methods`.
  _Why:_ the same engine code then runs on the OS, the in-memory transport pair and the Simulation, and replays from a seed.
- **Never hold a lock across `.await`.**
- **Every public async function has a `# Cancel safety` section** saying what is lost if its future is dropped.
- **Public futures are `Send`**, checked by a compile-time assertion on the main entry points.

### Lints, formatting and toolchain

- **Edition 2024, a pinned toolchain in `rust-toolchain.toml`, and the MSRV of ADR 0030 tested in CI.**
- **Warnings fail CI.** Clippy runs with `--all-targets --all-features`.
- **Clippy `pedantic` is on as warnings**, with a short allow-list kept in `[workspace.lints]`.
- **Time and order come from their handles:** `SystemTime::now`, `std::time::Instant::now`, `tokio::time::Instant::now` and library constructors that read their own clock or randomness (`Ulid::new`) are in `disallowed-methods` outside the `Clock` and `Entropy` modules, `std::env::var`, `var_os` and `vars` outside `main`, and `iter_over_hash_type` is denied (ADR 0059). The handles' test constructors `Clock::paused` and `Entropy::seeded` are in `disallowed-methods` too, opted into only by test modules (ADR 0062) and bench target roots (ADR 0071).
  _Why:_ a test can then drive every clock the code reads, and hash order never reaches output.
- **Denied:** `print_stdout` and `print_stderr`, `dbg_macro`, `todo`, `unimplemented`, `unwrap_used` (allowed in tests), `allow_attributes`, `allow_attributes_without_reason`, `clone_on_ref_ptr`, `exit` (outside `main`). `std::io::stdout` and `std::io::stderr` are in `disallowed-methods`.
- **Only three writers touch stdout or stderr:** the protocol transport, the CLI's output writer (its human output, its `--json` output and its hints on stderr) and the `tracing` subscriber's writer (stderr or a file). Everything else logs through `tracing`. The one exception is the test-only `goodfetch-scripted-engine`, which writes its script's messages to stdout under an `#[expect]` naming ADRs 0060 and 0069.
  _Why:_ when the binary runs as an Attached engine started by an SDK, or as the stdio MCP server, stdout is the JSON-RPC channel, so one stray write corrupts the protocol; and the CLI's `--json` output is under semver (ADR 0030).
- **Exceptions use `#[expect(lint, reason = "…")]`, never `#[allow]`.**
  _Why:_ `expect` warns when the exception is no longer needed, so stale exceptions disappear.
- **rustfmt runs on its default 2024 style.**
  _Why:_ the options teams most often change (import granularity and grouping, comment wrapping) need a nightly toolchain, and defaults end style debates.
- **CI also runs `cargo-deny`, `cargo-shear` (unused dependencies), `cargo-nextest`, `typos` and `zizmor` (workflow security).**

### `unsafe`

- **`unsafe_code` is denied across the workspace.** A module that needs it (the in-house `sandbox_init` binding of ADR 0033, the pipe file descriptors of ADR 0017) opts in with `#![expect(unsafe_code, reason = "…")]`.
- **Every `unsafe` block carries a `// SAFETY:` comment** (`undocumented_unsafe_blocks`), and `unsafe_op_in_unsafe_fn` is denied.
- **Miri runs in CI on the opted-in code it can execute (pure-Rust `unsafe`).** FFI and system-call modules, which Miri can't run, are covered by tests on each real OS, and by the Release suite's AddressSanitizer run on Linux (ADR 0067).
- **`unsafe` is never used to get past `Send` or `Sync`.**

### Logging and secrets

- **`tracing` everywhere, never `println!` or `log`.** Spans cover each RPC request, run, page and CDP command, with glossary IDs as fields (`run_id`, `session_id`).
- **`#[instrument(skip_all, fields(…))]`**, so arguments are only logged when named.
  _Why:_ one `?config` in a log line can leak a proxy password, and secrets never reach logs (ADRs 0014, 0015).
- **Events are structured:** a fixed message plus fields, no `format!` inside a log call.
- **Logs go to stderr or a file, never stdout**, through the `tracing` subscriber.
- **Levels:** `error` needs someone to act; `warn` is a degraded but handled state; `info` is a lifecycle step of a run, Session or Daemon; `debug` explains a decision; `trace` is per-message detail.
- **Credentials, cookies, tokens and keys are `secrecy::SecretString`** (or a wrapper with a hand-written `Debug`). Secret canaries and redaction property tests prove the secret reaches no output ([`TESTING.md`](TESTING.md#secrets), ADR 0067).

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
- **Platform differences use `cfg(target_os = …)`, never features.** Custom `cfg`s are declared for `unexpected_cfgs`: `goodfetch_loom` for loom tests (ADR 0060), and `tokio_unstable` only because the Simulation's scenarios compile under it, in a job of their own. Under `goodfetch_loom`, a crate's `sync` module maps its atomics and locks to loom, and `disallowed-types` refuses the std atomic, `Mutex`, `RwLock` and `Condvar` types outside it.

### Tests

The test rules live in [`TESTING.md`](TESTING.md) (where tests live, how they are written and named, shared helpers, fixtures, snapshots, injection and determinism, simulation, model checking and crash testing, flaky tests and Live checks, the test-first process, suites, AI features and security tests; ADRs 0059, 0060, 0061, 0062, 0064, 0065 and 0067), apart from Design's "Tests go through the interface" and each language's runner choices below.

### Documentation in code

- **`missing_docs` warns on `goodfetch` and `goodfetch-protocol`.**
- **Each public function of `goodfetch` and `goodfetch-protocol` documents `# Errors`** (with the Error codes it can return), **`# Cancel safety`** if it is async and **`# Panics`** if it can panic; on `goodfetch` it also has **an example** that runs as a doctest.
- **Internal crates document what a caller outside the module must know**: their `//!` headers, `# Errors` where a caller would otherwise guess, and `# Cancel safety` on every public async function (ADR 0038).
- **The catalog's description of an operation is its only text source:** MCP tool descriptions, SDK docstrings and reference pages are generated from it.
  _Why:_ what a model reads in a tool description directly affects agent accuracy, and the surfaces then never disagree.
- **Comments say why, never what the code already says.**

## Protocol and SDKs

- **The SDK API follows the principles of [ADR 0077](docs/adr/0077-the-sdk-api-follows-eight-guarantees-and-four-ranked-shape-rules.md)**: eight guarantees (G1–G8), four ranked shape rules (S1–S4) and a tie-breaker (T), cited by ID.
- **Contracts are Rust types in `goodfetch-protocol`**, emitted as a committed JSON Schema bundle (ADR 0038). To add or change an operation, edit the types, run `xtask codegen`, and commit what it produces.
- **Generated code is never edited by hand.** It lives in a `generated/` folder in each SDK, and CI fails when it differs from what `xtask codegen` produces.
- **Hand-written code in each SDK is the runtime and the ergonomic layer:** engine start and discovery, framing, `hello`, sockets, request correlation, flow-control credits, event resumption, file-backed bodies, cancellation, and the objects users hold.
- **Settings reach code through one `settings` option per scope, generated from the settings schema** ([ADR 0083](docs/adr/0083-a-program-sets-settings-in-one-typed-option-explicit-values-beat-the-environment-and-no-value-is-silently-ignored.md)): a nested object literal in TypeScript, a nested `TypedDict` in Python, typed key constants on `Settings` in Rust. No SDK reads the environment outside `connect()`'s environment map.
- **The wire is snake_case.** Python uses wire names as they are; TypeScript presents camelCase through the generated mapping, which never renames user data (extracted records, headers, metadata maps).

## TypeScript

Applies to `sdks/typescript`, `console/`, `adapters/`, `integrations/`, `bench/` and the docs site's `website/` in the pnpm workspace.

- **TypeScript 7 with `strict`, `noUncheckedIndexedAccess`, `exactOptionalPropertyTypes`, `verbatimModuleSyntax`, `isolatedDeclarations` and `erasableSyntaxOnly`.** A TypeScript 6 alias is kept only for tools that need the compiler API (API Extractor, TypeDoc, Twoslash) until TypeScript 7 provides one.
- **Optional fields are written `?: T | undefined`.**
  _Why:_ the published types then work whether or not the consumer enables `exactOptionalPropertyTypes`.
- **oxlint with type-aware rules, and oxfmt for formatting.** `no-floating-promises` is an error.
  _Why:_ an unawaited promise loses its error silently; oxlint's type-aware rules run on the real TypeScript 7 checker.
- **Libraries build with tsdown. Outside `console/` and `website/`, tests run with Vitest on every supported Node line (ADR 0024), plus smoke scripts of the installed package on Bun, Deno 2 and the edge runtimes (ADR 0069).**
- **The Console and the docs site are tested in browsers, on one Node line:** Vitest Browser Mode and Playwright Test, against the binary for the served Console (ADR 0068, rules in [`TESTING.md`](TESTING.md)).
- **Packages are ESM-only** (ADR 0024) and pass `publint` and `@arethetypeswrong/cli` before publishing. npm is the only registry.
- **Named exports only; no TypeScript `enum`s** (string-literal unions instead).
- **SDK idioms:**
  - every operation that can be cancelled takes an `AbortSignal` option;
  - engines and Sessions are disposed with `await using`;
  - event streams are `AsyncIterable`;
  - engine responses are not validated at runtime beyond the protocol version check;
  - schemas users pass in (for extraction, Datasets and Payloads) are accepted as Standard Schema with Standard JSON Schema (the interfaces Zod and ArkType implement, and Valibot through its adapter), whose types the SDK copies rather than depending on a schema library, and converted to JSON Schema (ADR 0082).
- **Errors are one class per Error code, generated.**
- **Doc comments are TSDoc.**

## Python

Applies to `sdks/python`, the Python adapters and the uv workspace. Supported versions follow ADR 0024: every CPython not past end of life, including free-threaded builds.

- **pyright in strict mode gates CI**, and mypy `--strict` checks the generated code and the type tests that exercise the public API (`TESTING.md`, ADR 0069).
  _Why:_ pyright is what most editors show; the mypy check keeps the types right for mypy users too.
- **ruff formats and lints**, with the rule sets `E F I UP B C4 ARG T20 TID251 PERF RUF ANN PYI TC ASYNC D`, ignoring `D206` and `D300`, which conflict with ruff's formatter.
  _Why:_ `ASYNC` catches blocking calls inside async code, and `T20` bans `print`.
- **Pure-Python packages build with `uv_build`; every package ships `py.typed`.** The SDK's `py3-none` platform wheels, which carry the engine binary (ADR 0024), are built by the release job of ADR 0032.
- **The async core uses anyio**, so the async API runs on asyncio and trio. The sync API is generated and runs that core on its own event-loop thread (ADR 0038).
- **Protocol data are pydantic v2 models, generated; errors are one exception class per Error code, generated.**
- **Tests use pytest with the anyio plugin** (every async test runs on asyncio and trio), `inline-snapshot` for protocol payloads and `hypothesis` for encode and decode round-trips. CI also runs the free-threaded build.
- **Docstrings follow the Google style**, enforced by ruff's `D` rules, for Griffe (ADR 0030).
